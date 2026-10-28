use crate::i18n::{tr, trf};
use crate::model::Settings;
use rqs_lib::{
    EndpointInfo, SendInfo, Visibility,
    channel::{ChannelMessage, Message, MessageClient, TransferAction, TransferKind},
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug)]
pub enum Command {
    Send(SendInfo),
    Action(String, TransferAction),
    Visible(bool),
    Restart(Settings),
    Stop,
}
#[derive(Debug)]
pub enum Event {
    Starting,
    Ready(u32),
    Device(EndpointInfo),
    Transfer(ChannelMessage),
    Error(String),
}
#[derive(Clone)]
pub struct Backend {
    tx: tokio::sync::mpsc::UnboundedSender<Command>,
    worker: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}
struct Session {
    service: rqs_lib::RQS,
    sender: tokio::sync::mpsc::Sender<SendInfo>,
    transfers: tokio::sync::broadcast::Receiver<ChannelMessage>,
    discovered: tokio::sync::broadcast::Receiver<EndpointInfo>,
    _devices: tokio::sync::broadcast::Sender<EndpointInfo>,
}
impl Session {
    async fn start(settings: &Settings) -> Result<Self, String> {
        settings.validate()?;
        std::fs::create_dir_all(&settings.downloads).map_err(|e| {
            trf(
                "Cannot open receive folder {path}: {error}",
                &[
                    ("path", &settings.downloads.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )
        })?;
        let mut service = rqs_lib::RQS::new(
            Visibility::Invisible,
            settings.port,
            Some(settings.downloads.clone()),
            Some(settings.name.clone()),
        );
        let transfers = service.message_sender.subscribe();
        let (devices, discovered) = tokio::sync::broadcast::channel(64);
        let sender = service
            .run()
            .await
            .map_err(|e| {
                trf(
                    "Could not start Quick Share: {error}",
                    &[("error", &e.to_string())],
                )
            })?
            .0;
        if !cfg!(test)
            && let Err(error) = service.discovery(devices.clone())
        {
            service.stop().await;
            return Err(trf(
                "Discovery could not start: {error}",
                &[("error", &error.to_string())],
            ));
        }
        Ok(Self {
            service,
            sender,
            transfers,
            discovered,
            _devices: devices,
        })
    }
}
fn terminal_event(id: String, state: rqs_lib::TransferState) -> Event {
    Event::Transfer(ChannelMessage {
        id,
        msg: Message::Client(MessageClient {
            kind: TransferKind::Outbound,
            state: Some(state),
            metadata: None,
        }),
    })
}
impl Backend {
    pub fn start(settings: Settings) -> (Self, async_channel::Receiver<Event>) {
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, rx) = async_channel::unbounded();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let mut settings = settings;
                'service: loop {
                    if events.send(Event::Starting).await.is_err() { break; }
                    let mut session = match Session::start(&settings).await {
                        Ok(session) => session,
                        Err(error) => {
                            let _ = events.send(Event::Error(error)).await;
                            while let Some(command) = commands.recv().await {
                                match command { Command::Restart(next) => { settings = next; continue 'service; }, Command::Stop => return, _ => {} }
                            }
                            return;
                        }
                    };
                    let _ = events.send(Event::Ready(session.service.port_number.unwrap_or_default())).await;
                    let mut restart = false;
                    let mut probes: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
                    loop {
                        tokio::select! {
                            command = commands.recv() => match command {
                                Some(Command::Send(info)) => {
                                    let events = events.clone(); let sender = session.sender.clone(); let id = info.id.clone();
                                    let task = tokio::spawn(async move {
                                        match tokio::time::timeout(Duration::from_secs(8), tokio::net::TcpStream::connect(&info.addr)).await {
                                            Ok(Ok(stream)) => {
                                                drop(stream);
                                                if sender.send(info.clone()).await.is_ok() { return; }
                                            },
                                            _ => { let _ = events.send(Event::Error(trf("Could not reach {device}. Check the other device and your network.", &[("device", &info.name)]))).await; }
                                        }
                                        let _ = events.send(terminal_event(info.id, rqs_lib::TransferState::Disconnected)).await;
                                    });
                                    if let Some(previous) = probes.insert(id, task) { previous.abort(); }
                                },
                                Some(Command::Action(id, action)) => {
                                    if action == TransferAction::TransferCancel && let Some(probe) = probes.remove(&id) && !probe.is_finished() {
                                        probe.abort(); let _ = events.send(terminal_event(id.clone(), rqs_lib::TransferState::Cancelled)).await;
                                    }
                                    let _ = session.service.message_sender.send(ChannelMessage { id, msg: Message::Lib { action } });
                                },
                                Some(Command::Visible(value)) => session.service.change_visibility(if value { Visibility::Visible } else { Visibility::Invisible }),
                                Some(Command::Restart(next)) => { settings = next; restart = true; break; },
                                Some(Command::Stop) | None => break,
                            },
                            result = session.transfers.recv() => match result {
                                Ok(message) if message.msg.as_client().is_some() => {
                                    probes.remove(&message.id);
                                    if events.send(Event::Transfer(message)).await.is_err() { break; }
                                },
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => { let _ = events.send(Event::Error(tr("Some transfer updates were missed. Check the other device before retrying."))).await; },
                                _ => {},
                            },
                            result = session.discovered.recv() => { if let Ok(device) = result && events.send(Event::Device(device)).await.is_err() { break; } },
                        }
                    }
                    for task in probes.values() { task.abort(); }
                    session.service.change_visibility(Visibility::Invisible);
                    // A stalled peer must not trap service recovery indefinitely.
                    let _ = tokio::time::timeout(Duration::from_secs(5), session.service.stop()).await;
                    if !restart { break; }
                }
            });
        });
        (
            Self {
                tx,
                worker: Arc::new(Mutex::new(Some(worker))),
            },
            rx,
        )
    }
    /// Called after the GTK main loop exits, so process termination cannot
    /// interrupt request teardown and incomplete-file cleanup.
    pub fn shutdown(&self) -> std::thread::Result<()> {
        self.send(Command::Stop);
        let mut worker = self.worker.lock().unwrap();
        match worker.take() {
            Some(worker) => worker.join(),
            None => Ok(()),
        }
    }
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rqs_lib::{
        OutboundPayload, TextPayloadType, TransferState, channel::TransferKind,
        hdl::info::TransferPayload,
    };
    use std::time::Duration;

    async fn next(rx: &async_channel::Receiver<Event>) -> Event {
        tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("protocol event timed out")
            .expect("backend closed")
    }
    async fn exchange(
        backend: &Backend,
        receiver: &Backend,
        rx: &async_channel::Receiver<Event>,
        port: u32,
        payload: OutboundPayload,
        accept: bool,
    ) -> Option<TransferPayload> {
        backend.send(Command::Send(SendInfo {
            id: format!("127.0.0.1:{port}"),
            name: "Protocol test".into(),
            addr: format!("127.0.0.1:{port}"),
            ob: payload,
        }));
        let mut inbound_done = false;
        let mut outbound_done = false;
        let mut result = None;
        let mut offered = false;
        while !inbound_done || !outbound_done {
            match next(rx).await {
                Event::Error(e) => panic!("backend error: {e}"),
                Event::Transfer(message) => {
                    let Some(client) = message.msg.as_client() else {
                        continue;
                    };
                    let incoming = client.kind == TransferKind::Inbound;
                    if incoming
                        && client.state == Some(TransferState::WaitingForUserConsent)
                        && !offered
                    {
                        offered = true;
                        let meta = client.metadata.as_ref().expect("offer metadata");
                        assert_eq!(meta.pin_code.as_ref().expect("matching code").len(), 4);
                        if matches!(
                            meta.payload_kind,
                            rqs_lib::hdl::info::TransferPayloadKind::WiFi
                        ) {
                            assert!(
                                meta.payload.is_none(),
                                "password must not arrive before consent"
                            );
                        }
                        if let Some(TransferPayload::Files(files)) = &meta.payload {
                            assert!(crate::model::safe_offer(files));
                        }
                        receiver.send(Command::Action(
                            message.id.clone(),
                            if accept {
                                TransferAction::ConsentAccept
                            } else {
                                TransferAction::ConsentDecline
                            },
                        ));
                    }
                    let terminal = if accept {
                        client.state == Some(TransferState::Finished)
                    } else {
                        matches!(
                            client.state,
                            Some(TransferState::Rejected | TransferState::Disconnected)
                        )
                    };
                    if terminal {
                        if incoming {
                            inbound_done = true;
                            if let Some(meta) = &client.metadata {
                                if accept
                                    && matches!(
                                        meta.payload_kind,
                                        rqs_lib::hdl::info::TransferPayloadKind::WiFi
                                    )
                                {
                                    assert!(meta.total_bytes >= 4);
                                    assert_eq!(meta.ack_bytes, meta.total_bytes);
                                    assert!(!format!("{meta:?}").contains("private-password"));
                                }
                                result = meta.payload.clone();
                            }
                        } else {
                            outbound_done = true;
                        }
                    }
                    assert!(
                        !matches!(client.state, Some(TransferState::Cancelled)),
                        "unexpected cancellation"
                    );
                }
                _ => {}
            }
        }
        assert!(offered, "receiver must request consent");
        result
    }
    #[tokio::test]
    #[cfg_attr(
        feature = "bluetooth",
        ignore = "Run without Bluetooth hardware access"
    )]
    async fn shutdown_joins_worker_and_closes_stalled_connections() {
        use tokio::io::AsyncReadExt;
        let root = std::env::temp_dir().join(format!("quickshare-shutdown-{}", std::process::id()));
        let (backend, events) = Backend::start(Settings {
            name: "Shutdown test".into(),
            downloads: root.clone(),
            ..Settings::default()
        });
        let port = loop {
            match next(&events).await {
                Event::Ready(port) => break port,
                Event::Error(error) => panic!("{error}"),
                _ => {}
            }
        };
        let mut stalled = tokio::net::TcpStream::connect(("127.0.0.1", port as u16))
            .await
            .unwrap();
        // The peer never starts its handshake; quitting must still close it.
        let copy = backend.clone();
        tokio::time::timeout(
            Duration::from_secs(8),
            tokio::task::spawn_blocking(move || copy.shutdown().unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        let mut byte = [0];
        let read = tokio::time::timeout(Duration::from_secs(1), stalled.read(&mut byte))
            .await
            .unwrap();
        assert!(
            matches!(read, Ok(0)) || read.is_err(),
            "connection survived shutdown"
        );
        assert!(events.is_closed(), "worker still owns its event sender");
        let _listener = std::net::TcpListener::bind(("0.0.0.0", port as u16))
            .expect("listener survived shutdown");
        backend.shutdown().unwrap(); // Idempotent through a second clone.
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn shutdown_after_startup_failure_joins_worker() {
        let (backend, events) = Backend::start(Settings {
            port: Some(65536),
            ..Settings::default()
        });
        while !matches!(next(&events).await, Event::Error(_)) {}
        backend.shutdown().unwrap();
        assert!(events.is_closed());
    }

    #[tokio::test]
    #[cfg_attr(
        feature = "bluetooth",
        ignore = "Run with --no-default-features to avoid Bluetooth hardware access"
    )]
    async fn encrypted_loopback_files_text_url_and_decline() {
        let root = std::env::temp_dir().join(format!(
            "quickshare-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let downloads = root.join("received");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("hello.txt");
        let content = "Hello from GNOME Quickshare.\n".repeat(8192);
        std::fs::write(&source, &content).unwrap();
        let good = Settings {
            name: "Protocol test".into(),
            downloads: downloads.clone(),
            ..Settings::default()
        };
        let (backend, rx) = Backend::start(Settings {
            port: Some(65536),
            ..good.clone()
        });
        loop {
            if let Event::Error(error) = next(&rx).await {
                assert!(error.contains("port"));
                break;
            }
        }
        backend.send(Command::Restart(good));
        let _sender_port = loop {
            match next(&rx).await {
                Event::Ready(port) => break port,
                Event::Error(e) => panic!("{e}"),
                _ => {}
            }
        };
        let (receiver, receiver_events) = Backend::start(Settings {
            name: "Receiver test".into(),
            downloads: downloads.clone(),
            ..Settings::default()
        });
        let port = loop {
            match next(&receiver_events).await {
                Event::Ready(port) => break port,
                Event::Error(e) => panic!("{e}"),
                _ => {}
            }
        };
        let (merged, rx_merged) = async_channel::unbounded();
        for stream in [rx, receiver_events] {
            let merged = merged.clone();
            tokio::spawn(async move {
                while let Ok(event) = stream.recv().await {
                    if merged.send(event).await.is_err() {
                        break;
                    }
                }
            });
        }
        let rx = rx_merged;
        exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Files(vec![source.to_string_lossy().into_owned()]),
            true,
        )
        .await;
        assert_eq!(
            std::fs::read_to_string(downloads.join("hello.txt")).unwrap(),
            content
        );
        exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Files(vec![source.to_string_lossy().into_owned()]),
            true,
        )
        .await;
        assert_eq!(
            std::fs::read_to_string(downloads.join("hello (1).txt")).unwrap(),
            content
        );
        let text = "A little closer. नमस्ते 🌍";
        let result = exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Text {
                text: text.into(),
                text_type: TextPayloadType::Text,
            },
            true,
        )
        .await;
        assert!(matches!(result, Some(TransferPayload::Text(value)) if value == text));
        let url = "https://example.org/hello?from=gnome";
        let result = exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Text {
                text: url.into(),
                text_type: TextPayloadType::Url,
            },
            true,
        )
        .await;
        assert!(matches!(result, Some(TransferPayload::Url(value)) if value == url));
        // Real encrypted Wi-Fi credential round trips. Credentials are not in
        // the introduction; only explicit consent releases their BYTES payload.
        use rqs_lib::{WifiPayload, sharing_nearby::wifi_credentials_metadata::SecurityType};
        for wifi in [
            WifiPayload {
                ssid: "Test WPA".into(),
                password: "private-password".into(),
                security_type: SecurityType::WpaPsk,
                hidden_ssid: true,
            },
            WifiPayload {
                ssid: "Open Café".into(),
                password: String::new(),
                security_type: SecurityType::Open,
                hidden_ssid: false,
            },
            WifiPayload {
                ssid: "Test WEP".into(),
                password: "0102030405".into(),
                security_type: SecurityType::Wep,
                hidden_ssid: false,
            },
        ] {
            let result = exchange(
                &backend,
                &receiver,
                &rx,
                port,
                OutboundPayload::Wifi(wifi.clone()),
                true,
            )
            .await;
            assert!(
                matches!(result, Some(TransferPayload::Wifi { ssid, password, security_type, hidden_ssid })
                if ssid == wifi.ssid && password == wifi.password && security_type == wifi.security_type && hidden_ssid == wifi.hidden_ssid)
            );
        }
        exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Wifi(WifiPayload {
                ssid: "Decline test".into(),
                password: "secret-password".into(),
                security_type: SecurityType::WpaPsk,
                hidden_ssid: false,
            }),
            false,
        )
        .await;
        exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Files(vec![source.to_string_lossy().into_owned()]),
            false,
        )
        .await;
        assert!(
            !downloads.join("hello (2).txt").exists(),
            "declined offers must not write files"
        );
        // Cancellation while the receiving peer is waiting for consent.
        let outbound_id = "cancel-test".to_string();
        backend.send(Command::Send(SendInfo {
            id: outbound_id.clone(),
            addr: format!("127.0.0.1:{port}"),
            name: "Cancel test".into(),
            ob: OutboundPayload::Files(vec![source.to_string_lossy().into_owned()]),
        }));
        let mut inbound_stopped = false;
        let mut outbound_stopped = false;
        let mut cancelled = false;
        let mut inbound_id = None;
        while !inbound_stopped || !outbound_stopped {
            if let Event::Transfer(message) = next(&rx).await {
                let client = message.msg.as_client().unwrap();
                if client.kind == TransferKind::Inbound
                    && client.state == Some(TransferState::WaitingForUserConsent)
                    && !cancelled
                {
                    cancelled = true;
                    inbound_id = Some(message.id.clone());
                    backend.send(Command::Action(
                        outbound_id.clone(),
                        TransferAction::TransferCancel,
                    ));
                }
                if client.kind == TransferKind::Inbound
                    && inbound_id.as_ref() == Some(&message.id)
                    && matches!(
                        client.state,
                        Some(TransferState::Cancelled | TransferState::Disconnected)
                    )
                {
                    inbound_stopped = true;
                }
                if client.kind == TransferKind::Outbound
                    && message.id == outbound_id
                    && client.state == Some(TransferState::Cancelled)
                {
                    outbound_stopped = true;
                }
            }
        }
        assert!(cancelled);
        assert!(!downloads.join("hello (2).txt").exists());
        // Duplicate basenames in one offer must receive distinct destinations.
        exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Files(vec![
                source.to_string_lossy().into_owned(),
                source.to_string_lossy().into_owned(),
            ]),
            true,
        )
        .await;
        assert_eq!(
            std::fs::read_to_string(downloads.join("hello (2).txt")).unwrap(),
            content
        );
        assert_eq!(
            std::fs::read_to_string(downloads.join("hello (3).txt")).unwrap(),
            content
        );
        // Empty files need a real final FILE chunk, both alone and alongside
        // nonempty payloads. Rejection must not create even an empty destination.
        let empty = root.join("empty.txt");
        std::fs::write(&empty, []).unwrap();
        exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Files(vec![empty.to_string_lossy().into_owned()]),
            false,
        )
        .await;
        assert!(!downloads.join("empty.txt").exists());
        exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Files(vec![empty.to_string_lossy().into_owned()]),
            true,
        )
        .await;
        assert_eq!(
            std::fs::metadata(downloads.join("empty.txt"))
                .unwrap()
                .len(),
            0
        );
        exchange(
            &backend,
            &receiver,
            &rx,
            port,
            OutboundPayload::Files(vec![
                empty.to_string_lossy().into_owned(),
                source.to_string_lossy().into_owned(),
            ]),
            true,
        )
        .await;
        assert_eq!(
            std::fs::metadata(downloads.join("empty (1).txt"))
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            std::fs::read_to_string(downloads.join("hello (4).txt")).unwrap(),
            content
        );
        // A failed TCP connection must reach the UI as a terminal outcome.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let unused = listener.local_addr().unwrap();
        drop(listener);
        backend.send(Command::Send(SendInfo {
            id: unused.to_string(),
            addr: unused.to_string(),
            name: "Offline test".into(),
            ob: OutboundPayload::Text {
                text: "hello".into(),
                text_type: TextPayloadType::Text,
            },
        }));
        loop {
            if let Event::Transfer(message) = next(&rx).await
                && message.id == unused.to_string()
                && message.msg.as_client().unwrap().state == Some(TransferState::Disconnected)
            {
                break;
            }
        }
        backend.shutdown().unwrap();
        receiver.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
