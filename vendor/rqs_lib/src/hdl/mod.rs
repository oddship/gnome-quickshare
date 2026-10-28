use std::collections::HashMap;

use info::{InternalFileInfo, TransferMetadata};
use p256::{PublicKey, SecretKey};

use crate::securegcm::ukey2_client_init::CipherCommitment;
use crate::sharing_nearby::wifi_credentials_metadata::SecurityType;
use crate::utils::RemoteDeviceInfo;

#[cfg(feature = "experimental")]
mod ble;
#[cfg(feature = "experimental")]
pub use ble::*;
#[cfg(all(feature = "experimental", target_os = "linux"))]
mod blea;
#[cfg(all(feature = "experimental", target_os = "linux"))]
pub use blea::*;
mod wifi;
pub use wifi::WifiPayload;
mod crypto;
pub(crate) mod deadline;
mod frame;
mod inbound;
pub use inbound::*;
pub mod info;
mod mdns_daemon;
mod mdns_discovery;
pub use mdns_discovery::*;
mod mdns;
pub use mdns::*;
mod outbound;
pub use outbound::*;

#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq)]
pub enum TransferState {
    #[default]
    Initial,
    ReceivedConnectionRequest,
    SentUkeyServerInit,
    SentUkeyClientInit,
    SentUkeyClientFinish,
    SentPairedKeyEncryption,
    ReceivedUkeyClientFinish,
    SentConnectionResponse,
    SentPairedKeyResult,
    SentIntroduction,
    ReceivedPairedKeyResult,
    WaitingForUserConsent,
    ReceivingFiles,
    SendingFiles,
    Disconnected,
    Rejected,
    Cancelled,
    Finished,
}

#[derive(Debug, Default)]
pub struct InnerState {
    pub id: String,
    pub server_seq: i32,
    pub client_seq: i32,
    pub encryption_done: bool,

    // Subject to be used-facing for progress, ...
    pub state: TransferState,
    pub remote_device_info: Option<RemoteDeviceInfo>,
    pub pin_code: Option<String>,
    pub transfer_metadata: Option<TransferMetadata>,
    pub transferred_files: HashMap<i64, InternalFileInfo>,

    // Everything needed for encryption/decryption/verif
    pub cipher_commitment: Option<CipherCommitment>,
    pub private_key: Option<SecretKey>,
    pub public_key: Option<PublicKey>,
    pub server_init_data: Option<Vec<u8>>,
    pub client_init_msg_data: Option<Vec<u8>>,
    pub ukey_client_finish_msg_data: Option<Vec<u8>>,
    pub decrypt_key: Option<Vec<u8>>,
    pub recv_hmac_key: Option<Vec<u8>>,
    pub encrypt_key: Option<Vec<u8>>,
    pub send_hmac_key: Option<Vec<u8>>,

    // Used to handle/track ingress transfer
    pub text_payload: Option<TextPayloadInfo>,
    // pub text_payload_id: i64,
    // pub text_is_url: bool,
    // pub wifi_ssid: Option<String>,
    pub payload_buffers: HashMap<i64, Vec<u8>>,
}

#[derive(Debug, Clone)]
pub enum TextPayloadInfo {
    Url(i64),
    Text(i64),
    Wifi((i64, String, SecurityType)), // id, ssid, security type
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPayloadType {
    Url,
    Text,
    Wifi,
}

impl TextPayloadInfo {
    pub fn get_i64_value(&self) -> i64 {
        match self {
            TextPayloadInfo::Url(value)
            | TextPayloadInfo::Text(value)
            | TextPayloadInfo::Wifi((value, _, _)) => value.to_owned(),
        }
    }

    pub fn from_type(text_type: &TextPayloadType, payload_id: i64) -> Self {
        match text_type {
            TextPayloadType::Url => TextPayloadInfo::Url(payload_id),
            TextPayloadType::Text => TextPayloadInfo::Text(payload_id),
            TextPayloadType::Wifi => TextPayloadInfo::Wifi((
                payload_id,
                String::new(),
                SecurityType::UnknownSecurityType,
            )),
        }
    }
}

impl TextPayloadType {
    pub fn to_proto_type(&self) -> crate::sharing_nearby::text_metadata::Type {
        match self {
            TextPayloadType::Url => crate::sharing_nearby::text_metadata::Type::Url,
            TextPayloadType::Text => crate::sharing_nearby::text_metadata::Type::Text,
            TextPayloadType::Wifi => crate::sharing_nearby::text_metadata::Type::Unknown,
        }
    }
}

#[cfg(test)]
mod lifecycle_regressions {
    use super::*;
    use crate::channel::{ChannelMessage, Message, TransferAction};
    use crate::location_nearby_connections::{OfflineFrame, v1_frame};
    use crate::utils::{DeviceType, RemoteDeviceInfo};
    use prost::Message as _;
    use tokio::io::AsyncReadExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::broadcast;
    use tokio::time::{Duration, timeout};

    async fn sockets() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        (socket, peer)
    }

    async fn assert_plain_disconnect(peer: &mut TcpStream) {
        let mut length = [0; 4];
        timeout(Duration::from_secs(2), peer.read_exact(&mut length))
            .await
            .unwrap()
            .unwrap();
        let mut body = vec![0; u32::from_be_bytes(length) as usize];
        timeout(Duration::from_secs(2), peer.read_exact(&mut body))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            OfflineFrame::decode(body.as_slice())
                .unwrap()
                .v1
                .unwrap()
                .r#type(),
            v1_frame::FrameType::Disconnection
        );
    }

    #[tokio::test]
    async fn cancelling_before_key_exchange_is_safe_in_both_directions() {
        let (socket, mut peer) = sockets().await;
        let (sender, _) = broadcast::channel(8);
        let mut inbound = InboundRequest::new(socket, "inbound".into(), sender.clone());
        assert!(!inbound.state.encryption_done);
        assert!(inbound.state.encrypt_key.is_none());
        sender
            .send(ChannelMessage {
                id: "inbound".into(),
                msg: Message::Lib {
                    action: TransferAction::TransferCancel,
                },
            })
            .unwrap();
        let error = timeout(Duration::from_secs(2), inbound.handle())
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.downcast_ref::<crate::errors::AppError>().is_some());
        assert_eq!(inbound.state.state, TransferState::Cancelled);
        assert_plain_disconnect(&mut peer).await;

        let (socket, mut peer) = sockets().await;
        let mut outbound = OutboundRequest::new(
            *b"test",
            socket,
            "outbound".into(),
            sender.clone(),
            OutboundPayload::Text {
                text: "hello".into(),
                text_type: TextPayloadType::Text,
            },
            RemoteDeviceInfo {
                name: "peer".into(),
                device_type: DeviceType::Phone,
            },
        );
        // Model cancelling an actual connection while waiting for ServerInit.
        outbound.send_connection_request().await.unwrap();
        outbound.send_ukey2_client_init().await.unwrap();
        let mut reader = frame::FrameReader::default();
        for _ in 0..2 {
            timeout(Duration::from_secs(2), reader.read(&mut peer))
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(outbound.state.state, TransferState::SentUkeyClientInit);
        assert!(!outbound.state.encryption_done);
        assert!(outbound.state.encrypt_key.is_none());
        sender
            .send(ChannelMessage {
                id: "outbound".into(),
                msg: Message::Lib {
                    action: TransferAction::TransferCancel,
                },
            })
            .unwrap();
        let error = timeout(Duration::from_secs(2), outbound.handle())
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.downcast_ref::<crate::errors::AppError>().is_some());
        assert_eq!(outbound.state.state, TransferState::Cancelled);
        assert_plain_disconnect(&mut peer).await;
    }
}
