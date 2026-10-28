use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::sync::broadcast::Sender;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;

use crate::channel::{self, ChannelMessage, MessageClient, TransferKind};
use crate::errors::AppError;
use crate::hdl::deadline::{self, HANDSHAKE_TIMEOUT, TransferDeadline};
use crate::hdl::{InboundRequest, OutboundPayload, OutboundRequest, TransferState};
use crate::utils::RemoteDeviceInfo;

const INNER_NAME: &str = "TcpServer";
const MAX_INCOMING_CONNECTIONS: usize = 16;

#[derive(Debug, Clone)]
pub struct SendInfo {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub ob: OutboundPayload,
}

pub struct TcpServer {
    endpoint_id: [u8; 4],
    handshake_timeout: std::time::Duration,
    incoming_slots: Arc<Semaphore>,
    tcp_listener: TcpListener,
    sender: Sender<ChannelMessage>,
    connect_receiver: Receiver<SendInfo>,
}

impl TcpServer {
    pub fn new(
        endpoint_id: [u8; 4],
        tcp_listener: TcpListener,
        sender: Sender<ChannelMessage>,
        connect_receiver: Receiver<SendInfo>,
    ) -> Result<Self, anyhow::Error> {
        Ok(Self {
            endpoint_id,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            incoming_slots: Arc::new(Semaphore::new(MAX_INCOMING_CONNECTIONS)),
            tcp_listener,
            sender,
            connect_receiver,
        })
    }

    pub async fn run(&mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        info!("{INNER_NAME}: service starting");
        let mut handlers = tokio::task::JoinSet::new();

        loop {
            let cctk = ctk.clone();

            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                }
                Some(result) = handlers.join_next(), if !handlers.is_empty() => {
                    if let Err(error) = result {
                        error!("{INNER_NAME}: transfer task failed: {error}");
                    }
                }
                Some(i) = self.connect_receiver.recv() => {
                    info!("{INNER_NAME}: connect_receiver: got {:?}", i);
                    let endpoint_id = self.endpoint_id;
                    let sender = self.sender.clone();
                    let handshake_timeout = self.handshake_timeout;
                    handlers.spawn(async move {
                        let id = i.id.clone();
                        if let Err(e) = Self::connect(endpoint_id, sender.clone(), cctk, i, handshake_timeout).await {
                            let _ = sender.send(ChannelMessage {
                                id,
                                msg: channel::Message::Client(MessageClient {
                                    kind: TransferKind::Outbound,
                                    state: Some(TransferState::Disconnected),
                                    metadata: None,
                                }),
                            });
                            error!("{INNER_NAME}: error sending: {}", e);
                        }
                    });
                }
                r = self.tcp_listener.accept() => {
                    match r {
                        Ok((socket, remote_addr)) => {
                            let Ok(slot) = self.incoming_slots.clone().try_acquire_owned() else {
                                debug!("{INNER_NAME}: incoming connection limit reached");
                                continue;
                            };
                            trace!("{INNER_NAME}: new client: {remote_addr}");
                            let esender = self.sender.clone();
                            let csender = self.sender.clone();
                            let handshake_timeout = self.handshake_timeout;

                            handlers.spawn(async move {
                                let _slot = slot;
                                let mut ir = InboundRequest::new(socket, remote_addr.to_string(), csender);

                                let mut budget = TransferDeadline::new(handshake_timeout);
                                loop {
                                    let expires = budget.next(&ir.state);
                                    let result = tokio::select! {
                                        _ = cctk.cancelled() => break,
                                        _ = deadline::wait(expires) => Err(anyhow::anyhow!("Peer made no transfer progress before its deadline")),
                                        result = ir.handle() => result,
                                    };
                                    match result {
                                        Ok(_) => {},
                                        Err(e) => match e.downcast_ref() {
                                            Some(AppError::NotAnError) => break,
                                            None => {
                                                if ir.state.state == TransferState::Initial {
                                                    break;
                                                }

                                                if ir.state.state != TransferState::Finished {
                                                    let _ = esender.send(ChannelMessage {
                                                        id: remote_addr.to_string(),
                                                        msg: channel::Message::Client(MessageClient {
                                                            kind: TransferKind::Inbound,
                                                            state: Some(TransferState::Disconnected),
                                                            metadata: Default::default()
                                                        }),
                                                    });
                                                }
                                                error!("{INNER_NAME}: error while handling client: {e} ({:?})", ir.state.state);
                                                break;
                                            }
                                        },
                                    }
                                }
                            });
                        },
                        Err(err) => {
                            error!("{INNER_NAME}: error accepting: {}", err);
                            break;
                        }
                    }
                }
            }
        }

        // Wait for request destructors (including partial-file cleanup), rather
        // than leaving detached sockets alive after the service reports stopped.
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
        Ok(())
    }

    async fn connect(
        endpoint_id: [u8; 4],
        sender: Sender<ChannelMessage>,
        ctk: CancellationToken,
        si: SendInfo,
        handshake_timeout: std::time::Duration,
    ) -> Result<(), anyhow::Error> {
        debug!("{INNER_NAME}: Connecting to: {}", si.addr);
        let socket = tokio::select! {
            _ = ctk.cancelled() => return Ok(()),
            socket = tokio::time::timeout(std::time::Duration::from_secs(8), TcpStream::connect(&si.addr)) => socket??,
        };

        let mut or = OutboundRequest::new(
            endpoint_id,
            socket,
            si.id.clone(),
            sender.clone(),
            si.ob,
            RemoteDeviceInfo {
                device_type: crate::DeviceType::Unknown,
                name: si.name,
            },
        );

        // Send connection request
        or.send_connection_request().await?;
        // Send UKEY init
        or.send_ukey2_client_init().await?;

        let mut budget = TransferDeadline::new(handshake_timeout);
        loop {
            let expires = budget.next(&or.state);
            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                },
                _ = deadline::wait(expires) => return Err(anyhow::anyhow!("Peer made no transfer progress before its deadline")),
                r = or.handle() => {
                    if let Err(e) = r {
                        match e.downcast_ref() {
                            Some(AppError::NotAnError) => break,
                            None => {
                                if or.state.state == TransferState::Initial {
                                    break;
                                }

                                if or.state.state != TransferState::Finished && or.state.state != TransferState::Cancelled {
                                    let _ = sender.send(ChannelMessage {
                                        id: si.id.clone(),
                                        msg: channel::Message::Client(MessageClient {
                                            kind: TransferKind::Outbound,
                                            state: Some(TransferState::Disconnected),
                                            metadata: Default::default()
                                        }),
                                    });
                                }
                                error!("{INNER_NAME}: error while handling client: {e} ({:?})", or.state.state);
                                break;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod lifecycle_regressions {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn outgoing_handshake_does_not_block_receives_and_shutdown_closes_both() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, _) = tokio::sync::broadcast::channel(8);
        let (connect, receiver) = tokio::sync::mpsc::channel(1);
        let mut server = TcpServer::new(*b"test", listener, sender.clone(), receiver).unwrap();
        let token = CancellationToken::new();
        let server_token = token.clone();
        let task = tokio::spawn(async move { server.run(server_token).await });
        let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
        connect
            .send(SendInfo {
                id: "pending-send".into(),
                addr: remote.local_addr().unwrap().to_string(),
                name: "remote".into(),
                ob: OutboundPayload::Text {
                    text: "hello".into(),
                    text_type: crate::hdl::TextPayloadType::Text,
                },
            })
            .await
            .unwrap();
        let (mut outgoing_peer, _) = timeout(Duration::from_secs(2), remote.accept())
            .await
            .unwrap()
            .unwrap();
        let mut peer = TcpStream::connect(address).await.unwrap();
        // Observe the actual request subscription, rather than assuming accept
        // has run after a fixed sleep. Then leave the prefix unfinished.
        timeout(Duration::from_secs(2), async {
            while sender.receiver_count() < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        peer.write_all(&[0]).await.unwrap();
        token.cancel();
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut byte = [0; 1];
        let closed = timeout(Duration::from_secs(2), peer.read(&mut byte))
            .await
            .unwrap();
        assert!(
            matches!(closed, Ok(0)) || closed.is_err(),
            "incoming task kept its socket alive"
        );
        let mut handshake = Vec::new();
        let closed = timeout(
            Duration::from_secs(2),
            outgoing_peer.read_to_end(&mut handshake),
        )
        .await
        .unwrap();
        assert!(
            closed.is_ok() || closed.unwrap_err().kind() == std::io::ErrorKind::ConnectionReset
        );
        assert!(!handshake.is_empty());
        assert_eq!(sender.receiver_count(), 0);
    }

    #[tokio::test]
    async fn connection_failure_reports_the_transfer_id() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (sender, mut events) = tokio::sync::broadcast::channel(8);
        let (connect, receiver) = tokio::sync::mpsc::channel(1);
        let mut server = TcpServer::new(*b"test", listener, sender, receiver).unwrap();
        let token = CancellationToken::new();
        let server_token = token.clone();
        let task = tokio::spawn(async move { server.run(server_token).await });
        connect
            .send(SendInfo {
                id: "unique-transfer-id".into(),
                // Invalid address gives a deterministic connection error without
                // relying on another process leaving a particular TCP port closed.
                addr: "invalid socket address".into(),
                name: "unreachable".into(),
                ob: OutboundPayload::Text {
                    text: "hello".into(),
                    text_type: crate::hdl::TextPayloadType::Text,
                },
            })
            .await
            .unwrap();
        let event = timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.id, "unique-transfer-id");
        let client = event.msg.as_client().unwrap();
        assert_eq!(client.kind, TransferKind::Outbound);
        assert_eq!(client.state, Some(TransferState::Disconnected));
        token.cancel();
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn malformed_server_key_reports_failure_instead_of_panicking() {
        use crate::securegcm::{
            Ukey2HandshakeCipher, Ukey2Message, Ukey2ServerInit, ukey2_message,
        };
        use crate::securemessage::{EcP256PublicKey, GenericPublicKey, PublicKeyType};
        use prost::Message as _;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (sender, mut events) = tokio::sync::broadcast::channel(8);
        let (connect, receiver) = tokio::sync::mpsc::channel(1);
        let mut server = TcpServer::new(*b"test", listener, sender, receiver).unwrap();
        let token = CancellationToken::new();
        let server_token = token.clone();
        let task = tokio::spawn(async move { server.run(server_token).await });
        connect
            .send(SendInfo {
                id: "invalid-peer-key".into(),
                addr: remote.local_addr().unwrap().to_string(),
                name: "peer".into(),
                ob: OutboundPayload::Text {
                    text: "hello".into(),
                    text_type: crate::hdl::TextPayloadType::Text,
                },
            })
            .await
            .unwrap();
        let (mut peer, _) = timeout(Duration::from_secs(2), remote.accept())
            .await
            .unwrap()
            .unwrap();
        // Receive ConnectionRequest and ClientInit before replying as a peer.
        timeout(Duration::from_secs(2), async {
            for _ in 0..2 {
                let length = peer.read_u32().await.unwrap();
                assert!(length < 1024 * 1024);
                let mut body = vec![0; length as usize];
                peer.read_exact(&mut body).await.unwrap();
            }
        })
        .await
        .unwrap();
        let invalid_key = GenericPublicKey {
            r#type: PublicKeyType::EcP256 as i32,
            ec_p256_public_key: Some(EcP256PublicKey {
                x: vec![0; 32],
                y: vec![0; 32],
            }),
            ..Default::default()
        };
        let message = Ukey2Message {
            message_type: Some(ukey2_message::Type::ServerInit as i32),
            message_data: Some(
                Ukey2ServerInit {
                    version: Some(1),
                    random: Some(vec![1; 32]),
                    handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512 as i32),
                    public_key: Some(invalid_key.encode_to_vec()),
                }
                .encode_to_vec(),
            ),
        }
        .encode_to_vec();
        peer.write_u32(message.len() as u32).await.unwrap();
        peer.write_all(&message).await.unwrap();
        let event = timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.id, "invalid-peer-key");
        assert_eq!(
            event.msg.as_client().unwrap().state,
            Some(TransferState::Disconnected)
        );
        token.cancel();
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn silent_handshakes_expire_and_report_outgoing_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let incoming_address = listener.local_addr().unwrap();
        let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (sender, mut events) = tokio::sync::broadcast::channel(8);
        let (connect, receiver) = tokio::sync::mpsc::channel(1);
        let mut server = TcpServer::new(*b"test", listener, sender.clone(), receiver).unwrap();
        server.handshake_timeout = Duration::from_millis(200);
        let token = CancellationToken::new();
        let server_token = token.clone();
        let task = tokio::spawn(async move { server.run(server_token).await });
        connect
            .send(SendInfo {
                id: "silent-peer".into(),
                addr: remote.local_addr().unwrap().to_string(),
                name: "peer".into(),
                ob: OutboundPayload::Text {
                    text: "hello".into(),
                    text_type: crate::hdl::TextPayloadType::Text,
                },
            })
            .await
            .unwrap();
        let (mut peer, _) = timeout(Duration::from_secs(2), remote.accept())
            .await
            .unwrap()
            .unwrap();
        let event = timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.id, "silent-peer");
        assert_eq!(
            event.msg.as_client().unwrap().state,
            Some(TransferState::Disconnected)
        );
        let mut handshake = Vec::new();
        timeout(Duration::from_secs(2), peer.read_to_end(&mut handshake))
            .await
            .unwrap()
            .unwrap();
        assert!(!handshake.is_empty());
        // The listener remains available and also expires an incoming peer that
        // never sends a ConnectionRequest. No empty Activity entry is emitted.
        let mut incoming = TcpStream::connect(incoming_address).await.unwrap();
        let mut byte = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(events.try_recv().is_err());
        token.cancel();
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn incoming_limit_releases_slots_and_shutdown_joins_handlers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, _) = tokio::sync::broadcast::channel(8);
        let (_connect, receiver) = tokio::sync::mpsc::channel(1);
        let mut server = TcpServer::new(*b"test", listener, sender.clone(), receiver).unwrap();
        let slots = Arc::new(Semaphore::new(1));
        server.incoming_slots = slots.clone();
        let token = CancellationToken::new();
        let server_token = token.clone();
        let task = tokio::spawn(async move { server.run(server_token).await });
        let first = TcpStream::connect(address).await.unwrap();
        timeout(Duration::from_secs(2), async {
            while slots.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut second = TcpStream::connect(address).await.unwrap();
        let mut byte = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), second.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        drop(first);
        timeout(Duration::from_secs(2), async {
            while slots.available_permits() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut third = TcpStream::connect(address).await.unwrap();
        timeout(Duration::from_secs(2), async {
            while slots.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        token.cancel();
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(sender.receiver_count(), 0);
        assert_eq!(slots.available_permits(), 1);
        assert_eq!(
            timeout(Duration::from_secs(2), third.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}
