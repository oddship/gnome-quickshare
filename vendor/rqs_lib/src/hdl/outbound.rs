use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

use anyhow::anyhow;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use libaes::{AES_256_KEY_LEN, Cipher};
use p256::ecdh::diffie_hellman;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use prost::Message;
use rand::Rng;
use sha2::{Digest, Sha256, Sha512};
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::broadcast::{Receiver, Sender};

use super::info::{InternalFileInfo, TransferMetadata, TransferPayload, TransferPayloadKind};
use super::{InnerState, TextPayloadInfo, TextPayloadType, TransferState, WifiPayload};
use crate::channel::{self, ChannelMessage, MessageClient, TransferAction, TransferKind};
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::Medium;
use crate::location_nearby_connections::connection_response_frame::ResponseStatus;
use crate::location_nearby_connections::payload_transfer_frame::{
    PacketType, PayloadChunk, PayloadHeader, payload_header,
};
use crate::location_nearby_connections::{KeepAliveFrame, OfflineFrame, PayloadTransferFrame};
use crate::securegcm::ukey2_alert::AlertType;
use crate::securegcm::ukey2_client_init::CipherCommitment;
use crate::securegcm::{
    DeviceToDeviceMessage, GcmMetadata, Type, Ukey2Alert, Ukey2ClientFinished, Ukey2ClientInit,
    Ukey2HandshakeCipher, Ukey2Message, Ukey2ServerInit, ukey2_message,
};
use crate::securemessage::{
    EcP256PublicKey, EncScheme, GenericPublicKey, Header, HeaderAndBody, PublicKeyType,
    SecureMessage, SigScheme,
};
use crate::sharing_nearby::{
    FileMetadata, IntroductionFrame, TextMetadata, file_metadata, paired_key_result_frame,
};
use crate::utils::{
    DeviceType, RemoteDeviceInfo, create_text_preview, encode_point, gen_ecdsa_keypair, gen_random,
    hkdf_extract_expand, to_four_digit_string,
};
use crate::{DEVICE_NAME, location_nearby_connections, sharing_nearby};

type HmacSha256 = Hmac<Sha256>;

const SANITY_DURATION: Duration = Duration::from_micros(10);

#[derive(Debug, Clone)]
pub enum OutboundPayload {
    Files(Vec<String>),
    Wifi(WifiPayload),
    Text {
        text: String,
        text_type: TextPayloadType,
    },
}

#[derive(Debug)]
pub struct OutboundRequest {
    endpoint_id: [u8; 4],
    socket: TcpStream,
    frame_reader: super::frame::FrameReader,
    pub state: InnerState,
    sender: Sender<ChannelMessage>,
    receiver: Receiver<ChannelMessage>,
    payload: OutboundPayload,
}

impl OutboundRequest {
    pub fn new(
        endpoint_id: [u8; 4],
        socket: TcpStream,
        id: String,
        sender: Sender<ChannelMessage>,
        payload: OutboundPayload,
        rdi: RemoteDeviceInfo,
    ) -> Self {
        let receiver = sender.subscribe();

        let transfer_metadata = match &payload {
            OutboundPayload::Files(files) => TransferMetadata {
                source: Some(rdi),
                payload_kind: TransferPayloadKind::Files,
                payload: Some(TransferPayload::Files(files.to_vec())),
                id: Default::default(),
                pin_code: Default::default(),
                payload_preview: Default::default(),
                total_bytes: Default::default(),
                ack_bytes: Default::default(),
            },
            OutboundPayload::Wifi(wifi) => TransferMetadata {
                source: Some(rdi),
                payload_kind: TransferPayloadKind::WiFi,
                payload: Some(TransferPayload::Wifi {
                    ssid: wifi.ssid.clone(),
                    password: wifi.password.clone(),
                    security_type: wifi.security_type,
                    hidden_ssid: wifi.hidden_ssid,
                }),
                id: Default::default(),
                pin_code: Default::default(),
                payload_preview: Some(wifi.ssid.clone()),
                total_bytes: Default::default(),
                ack_bytes: Default::default(),
            },
            OutboundPayload::Text { text, text_type } => {
                let (payload_kind, transfer_payload) = match text_type {
                    TextPayloadType::Url => {
                        (TransferPayloadKind::Url, TransferPayload::Url(text.clone()))
                    }
                    TextPayloadType::Text => (
                        TransferPayloadKind::Text,
                        TransferPayload::Text(text.clone()),
                    ),
                    TextPayloadType::Wifi => (
                        TransferPayloadKind::Text,
                        TransferPayload::Text(text.clone()),
                    ),
                };
                TransferMetadata {
                    source: Some(rdi),
                    payload_kind,
                    payload: Some(transfer_payload),
                    id: Default::default(),
                    pin_code: Default::default(),
                    payload_preview: Some(create_text_preview(text, 32).clone()),
                    total_bytes: text.len() as u64,
                    ack_bytes: Default::default(),
                }
            }
        };

        Self {
            endpoint_id,
            socket,
            frame_reader: Default::default(),
            state: InnerState {
                id,
                server_seq: 0,
                client_seq: 0,
                state: TransferState::Initial,
                // Encryption is enabled only after key derivation succeeds.
                encryption_done: false,
                transfer_metadata: Some(transfer_metadata),
                ..Default::default()
            },
            sender,
            receiver,
            payload,
        }
    }

    pub async fn handle(&mut self) -> Result<(), anyhow::Error> {
        tokio::select! {
            i = self.receiver.recv() => {
                match i {
                    Ok(channel_msg) => {
                        if channel_msg.id != self.state.id {
                            return Ok(());
                        }

                        if let channel::Message::Lib { action }  = &channel_msg.msg {
                            debug!("outbound: got: {:?}", channel_msg);
                            if action == &TransferAction::TransferCancel {
                                self.update_state(
                                    |e| {
                                        e.state = TransferState::Cancelled;
                                    },
                                    true,
                                ).await;
                                self.disconnection().await?;
                                return Err(anyhow!(crate::errors::AppError::NotAnError));
                            }
                        }
                    }
                    Err(e) => {
                        error!("inbound: channel error: {}", e);
                    }
                }
            },
            frame = self.frame_reader.read(&mut self.socket) => {
                self.process_frame(frame?).await?
            }
        }

        Ok(())
    }

    async fn process_frame(&mut self, frame_data: Vec<u8>) -> Result<(), anyhow::Error> {
        let current_state = &self.state;
        // Now determine what will be the request type based on current state
        match current_state.state {
            TransferState::SentUkeyClientInit => {
                debug!("Handling State::SentUkeyClientInit frame");
                let msg = Ukey2Message::decode(&*frame_data)?;
                self.update_state(
                    |e| {
                        e.server_init_data = Some(frame_data);
                    },
                    false,
                )
                .await;
                self.process_ukey2_server_init(&msg).await?;

                // Advance current state
                self.update_state(
                    |e: &mut InnerState| {
                        e.state = TransferState::SentUkeyClientFinish;
                        e.encryption_done = true;
                    },
                    false,
                )
                .await;
            }
            TransferState::SentUkeyClientFinish => {
                debug!("Handling State::SentUkeyClientFinish frame");
                let frame = location_nearby_connections::OfflineFrame::decode(&*frame_data)?;
                self.process_connection_response(&frame).await?;

                // Advance current state
                self.update_state(
                    |e: &mut InnerState| {
                        e.state = TransferState::SentPairedKeyEncryption;
                        e.server_init_data = Some(frame_data);
                        e.encryption_done = true;
                    },
                    false,
                )
                .await;
            }
            _ => {
                debug!("Handling SecureMessage frame");
                let smsg = SecureMessage::decode(&*frame_data)?;
                self.decrypt_and_process_secure_message(&smsg).await?;
            }
        }

        Ok(())
    }

    pub async fn send_connection_request(&mut self) -> Result<(), anyhow::Error> {
        let device_name = DEVICE_NAME.read().unwrap().clone();
        let request = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::ConnectionRequest.into(),
                ),
                connection_request: Some(location_nearby_connections::ConnectionRequestFrame {
                    endpoint_id: Some(String::from_utf8_lossy(&self.endpoint_id).to_string()),
                    endpoint_name: Some(device_name.clone().into()),
                    endpoint_info: Some(
                        RemoteDeviceInfo {
                            name: device_name.clone(),
                            device_type: DeviceType::Laptop,
                        }
                        .serialize(),
                    ),
                    mediums: vec![Medium::WifiLan.into()],
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };

        self.send_frame(request.encode_to_vec()).await?;

        Ok(())
    }

    pub async fn send_ukey2_client_init(&mut self) -> Result<(), anyhow::Error> {
        let (secret_key, public_key) = gen_ecdsa_keypair();

        let encoded_point = public_key.to_encoded_point(false);
        let x = encoded_point.x().unwrap();
        let y = encoded_point.y().unwrap();

        let pkey = GenericPublicKey {
            r#type: PublicKeyType::EcP256.into(),
            ec_p256_public_key: Some(EcP256PublicKey {
                x: encode_point(Bytes::from(x.to_vec()))?,
                y: encode_point(Bytes::from(y.to_vec()))?,
            }),
            ..Default::default()
        };

        let finish_frame = Ukey2Message {
            message_type: Some(ukey2_message::Type::ClientFinish.into()),
            message_data: Some(
                Ukey2ClientFinished {
                    public_key: Some(pkey.encode_to_vec()),
                }
                .encode_to_vec(),
            ),
        };

        let sha512 = Sha512::digest(finish_frame.encode_to_vec());
        let frame = Ukey2Message {
            message_type: Some(ukey2_message::Type::ClientInit.into()),
            message_data: Some(
                Ukey2ClientInit {
                    version: Some(1),
                    random: Some(gen_random(32)),
                    next_protocol: Some(String::from("AES_256_CBC-HMAC_SHA256")),
                    cipher_commitments: vec![CipherCommitment {
                        handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512.into()),
                        commitment: Some(sha512.to_vec()),
                    }],
                }
                .encode_to_vec(),
            ),
        };

        self.send_frame(frame.encode_to_vec()).await?;

        self.update_state(
            |e| {
                e.state = TransferState::SentUkeyClientInit;
                e.private_key = Some(secret_key);
                e.public_key = Some(public_key);
                e.client_init_msg_data = Some(frame.encode_to_vec());
                e.ukey_client_finish_msg_data = Some(finish_frame.encode_to_vec());
            },
            false,
        )
        .await;

        Ok(())
    }

    async fn process_ukey2_server_init(&mut self, msg: &Ukey2Message) -> Result<(), anyhow::Error> {
        if msg.message_type() != ukey2_message::Type::ServerInit {
            self.send_ukey2_alert(AlertType::BadMessageType).await?;
            return Err(anyhow!(
                "UKey2: message_type({:?}) != ServerInit",
                msg.message_type
            ));
        }

        let server_init = match Ukey2ServerInit::decode(msg.message_data()) {
            Ok(uk2si) => uk2si,
            Err(e) => {
                return Err(anyhow!("UKey2: Ukey2ClientFinished::decode: {}", e));
            }
        };

        if server_init.version() != 1 {
            self.send_ukey2_alert(AlertType::BadVersion).await?;
            return Err(anyhow!("UKey2: server_init.version != 1"));
        }

        if server_init.random().len() != 32 {
            self.send_ukey2_alert(AlertType::BadRandom).await?;
            return Err(anyhow!("UKey2: server_init.random.len != 32"));
        }

        if server_init.handshake_cipher() != Ukey2HandshakeCipher::P256Sha512 {
            self.send_ukey2_alert(AlertType::BadHandshakeCipher).await?;
            return Err(anyhow!("UKey2: handshake_cipher != P256Sha512"));
        }

        let server_public_key = match GenericPublicKey::decode(server_init.public_key()) {
            Ok(spk) => spk,
            Err(e) => {
                return Err(anyhow!("UKey2: GenericPublicKey::decode: {}", e));
            }
        };

        self.finalize_key_exchange(server_public_key).await?;
        self.send_frame(self.state.ukey_client_finish_msg_data.clone().unwrap())
            .await?;

        let frame = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::ConnectionResponse.into(),
                ),
                connection_response: Some(location_nearby_connections::ConnectionResponseFrame {
					response: Some(location_nearby_connections::connection_response_frame::ResponseStatus::Accept.into()),
					os_info: Some(location_nearby_connections::OsInfo {
						r#type: Some(location_nearby_connections::os_info::OsType::Linux.into())
					}),
					..Default::default()
				}),
                ..Default::default()
            }),
        };

        self.send_frame(frame.encode_to_vec()).await?;

        Ok(())
    }

    async fn process_connection_response(
        &mut self,
        frame: &location_nearby_connections::OfflineFrame,
    ) -> Result<(), anyhow::Error> {
        let v1_frame = frame
            .v1
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        if v1_frame.r#type() != location_nearby_connections::v1_frame::FrameType::ConnectionResponse
        {
            return Err(anyhow!(format!(
                "Unexpected frame type: {:?}",
                v1_frame.r#type()
            )));
        }

        if v1_frame.connection_response.is_none() {
            return Err(anyhow!("Unexpected None connection_response".to_string()));
        }

        if v1_frame.connection_response.as_ref().unwrap().response() != ResponseStatus::Accept {
            return Err(anyhow!("Connection rejected by third party".to_string()));
        }

        let paired_encryption = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::PairedKeyEncryption.into()),
                paired_key_encryption: Some(sharing_nearby::PairedKeyEncryptionFrame {
                    secret_id_hash: Some(gen_random(6)),
                    signed_data: Some(gen_random(72)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&paired_encryption).await?;

        Ok(())
    }

    async fn decrypt_and_process_secure_message(
        &mut self,
        smsg: &SecureMessage,
    ) -> Result<(), anyhow::Error> {
        let mac_key = self
            .state
            .recv_hmac_key
            .as_deref()
            .ok_or_else(|| anyhow!("Missing receive authentication key"))?;
        let key = self
            .state
            .decrypt_key
            .as_deref()
            .ok_or_else(|| anyhow!("Missing receive encryption key"))?;
        let d2d_msg = super::crypto::decrypt_message(smsg, mac_key, key)?;

        let seq = self.get_client_seq_inc().await;
        if d2d_msg.sequence_number() != seq {
            return Err(anyhow!(
                "Error d2d_msg.sequence_number invalid ({} vs {})",
                d2d_msg.sequence_number(),
                seq
            ));
        }

        let offline = location_nearby_connections::OfflineFrame::decode(d2d_msg.message())?;
        let v1_frame = offline
            .v1
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;
        match v1_frame.r#type() {
            location_nearby_connections::v1_frame::FrameType::PayloadTransfer => {
                trace!("Received FrameType::PayloadTransfer");
                let payload_transfer = v1_frame
                    .payload_transfer
                    .as_ref()
                    .ok_or_else(|| anyhow!("Missing required fields"))?;

                let header = payload_transfer
                    .payload_header
                    .as_ref()
                    .ok_or_else(|| anyhow!("Missing required fields"))?;
                let chunk = payload_transfer
                    .payload_chunk
                    .as_ref()
                    .ok_or_else(|| anyhow!("Missing required fields"))?;

                match header.r#type() {
                    payload_header::PayloadType::Bytes => {
                        info!("Processing PayloadType::Bytes");
                        let payload_id = header.id();

                        if header.total_size() < 0
                            || header.total_size() > super::frame::MAX_FRAME_LENGTH as i64
                        {
                            self.state.payload_buffers.remove(&payload_id);
                            return Err(anyhow!(
                                "Payload too large: {} bytes",
                                header.total_size()
                            ));
                        }

                        if self.state.payload_buffers.len() >= 16
                            && !self.state.payload_buffers.contains_key(&payload_id)
                        {
                            return Err(anyhow!("Too many buffered payloads"));
                        }
                        let buffered: usize =
                            self.state.payload_buffers.values().map(Vec::len).sum();
                        if buffered.saturating_add(chunk.body().len()) > 10 * 1024 * 1024 {
                            return Err(anyhow!("Buffered payload limit exceeded"));
                        }
                        self.state
                            .payload_buffers
                            .entry(payload_id)
                            .or_insert_with(|| Vec::with_capacity(header.total_size() as usize));

                        // Get the current length of the buffer, if it exists, without holding a mutable borrow.
                        let buffer_len = self.state.payload_buffers.get(&payload_id).unwrap().len();
                        if chunk.offset() != buffer_len as i64 {
                            self.state.payload_buffers.remove(&payload_id);
                            return Err(anyhow!(
                                "Unexpected chunk offset: {}, expected: {}",
                                chunk.offset(),
                                buffer_len
                            ));
                        }

                        let buffer = self.state.payload_buffers.get_mut(&payload_id).unwrap();
                        if buffer.len().saturating_add(chunk.body().len())
                            > header.total_size() as usize
                        {
                            return Err(anyhow!("Payload exceeds declared size"));
                        }
                        if let Some(body) = &chunk.body {
                            buffer.extend(body);
                        }

                        if (chunk.flags() & 1) == 1 {
                            debug!("Chunk flags & 1 == 1 ?? End of data ??");

                            let inner_frame = sharing_nearby::Frame::decode(buffer.as_slice())?;
                            self.process_transfer_setup(&inner_frame).await?;
                        }
                    }
                    payload_header::PayloadType::File => {
                        error!("Unhandled PayloadType::File: {:?}", header.r#type())
                    }
                    payload_header::PayloadType::Stream => {
                        error!("Unhandled PayloadType::Stream: {:?}", header.r#type())
                    }
                    payload_header::PayloadType::UnknownPayloadType => {
                        error!(
                            "Invalid PayloadType::UnknownPayloadType: {:?}",
                            header.r#type()
                        )
                    }
                }
            }
            location_nearby_connections::v1_frame::FrameType::KeepAlive => {
                trace!("Sending keepalive");
                self.send_keepalive(true).await?;
            }
            _ => {
                error!("Unhandled offline frame encrypted: {:?}", offline);
            }
        }

        Ok(())
    }

    async fn process_transfer_setup(
        &mut self,
        frame: &sharing_nearby::Frame,
    ) -> Result<(), anyhow::Error> {
        let v1_frame = frame
            .v1
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        if v1_frame.r#type() == sharing_nearby::v1_frame::FrameType::Cancel {
            info!("Transfer canceled");
            self.update_state(
                |e| {
                    e.state = TransferState::Cancelled;
                },
                true,
            )
            .await;
            self.disconnection().await?;
            return Err(anyhow!(crate::errors::AppError::NotAnError));
        }

        match self.state.state {
            TransferState::SentPairedKeyEncryption => {
                debug!("Processing State::SentPairedKeyEncryption");
                self.process_paired_key_encryption_frame(v1_frame).await?;
                self.update_state(
                    |e| {
                        e.state = TransferState::SentPairedKeyResult;
                    },
                    false,
                )
                .await;
            }
            TransferState::SentPairedKeyResult => {
                debug!("Processing State::SentPairedKeyResult");
                self.process_paired_key_result(v1_frame).await?;
                self.update_state(
                    |e| {
                        e.state = TransferState::SentIntroduction;
                    },
                    true,
                )
                .await;
            }
            TransferState::SentIntroduction => {
                debug!("Processing State::SentIntroduction");
                self.process_consent(v1_frame).await?;
            }
            TransferState::SendingFiles => {}
            _ => {
                info!(
                    "Unhandled connection state in process_transfer_setup: {:?}",
                    self.state.state
                );
            }
        }

        Ok(())
    }

    async fn process_paired_key_encryption_frame(
        &mut self,
        v1_frame: &sharing_nearby::V1Frame,
    ) -> Result<(), anyhow::Error> {
        if v1_frame.paired_key_encryption.is_none() {
            return Err(anyhow!("Missing required fields"));
        }

        let paired_result = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::PairedKeyResult.into()),
                paired_key_result: Some(sharing_nearby::PairedKeyResultFrame {
                    status: Some(paired_key_result_frame::Status::Unable.into()),
                }),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&paired_result).await?;

        Ok(())
    }

    async fn process_paired_key_result(
        &mut self,
        v1_frame: &sharing_nearby::V1Frame,
    ) -> Result<(), anyhow::Error> {
        if v1_frame.paired_key_result.is_none() {
            return Err(anyhow!("Missing required fields"));
        }

        let mut file_metadata: Vec<FileMetadata> = vec![];
        let mut text_metadata: Vec<TextMetadata> = vec![];
        let mut wifi_credentials_metadata = vec![];
        let mut transferred_files: HashMap<i64, InternalFileInfo> = HashMap::new();
        let mut total_to_send: u64 = 0;
        let mut text_payload_info: Option<TextPayloadInfo> = None;

        match &self.payload {
            OutboundPayload::Files(files) => {
                for f in files {
                    let path = Path::new(f);
                    if !path.is_file() {
                        warn!("Path is not a file: {}", f);
                        continue;
                    }

                    let file = match File::open(f) {
                        Ok(_f) => _f,
                        Err(e) => {
                            error!("Failed to open file: {f}: {:?}", e);
                            continue;
                        }
                    };
                    let fmetadata = match file.metadata() {
                        Ok(_fm) => _fm,
                        Err(e) => {
                            error!("Failed to get metadata for: {f}: {:?}", e);
                            continue;
                        }
                    };

                    let ftype = mime_guess::from_path(path)
                        .first_or_octet_stream()
                        .to_string();

                    let meta_type = if ftype.starts_with("image/") {
                        file_metadata::Type::Image
                    } else if ftype.starts_with("video/") {
                        file_metadata::Type::Video
                    } else if ftype.starts_with("audio/") {
                        file_metadata::Type::Audio
                    } else if path.extension().unwrap_or_default() == "apk" {
                        file_metadata::Type::App
                    } else {
                        file_metadata::Type::Unknown
                    };

                    info!("File type to send: {}", ftype);
                    let fname = path
                        .file_name()
                        .ok_or_else(|| anyhow!("Failed to get file_name for {f}"))?;
                    let fmeta = FileMetadata {
                        payload_id: Some(rand::rng().random::<i64>()),
                        name: Some(fname.to_os_string().into_string().unwrap()),
                        size: Some(fmetadata.size() as i64),
                        mime_type: Some(ftype),
                        r#type: Some(meta_type.into()),
                        ..Default::default()
                    };
                    transferred_files.insert(
                        fmeta.payload_id(),
                        InternalFileInfo {
                            payload_id: fmeta.payload_id(),
                            file_url: path.to_path_buf(),
                            bytes_transferred: 0,
                            total_size: fmeta.size(),
                            file: Some(file),
                        },
                    );
                    file_metadata.push(fmeta);
                    total_to_send += fmetadata.size();
                }
            }
            OutboundPayload::Wifi(wifi) => {
                let bytes = wifi.encode()?;
                let payload_id = rand::rng().random::<i64>();
                total_to_send = bytes.len() as u64;
                text_payload_info = Some(TextPayloadInfo::Wifi((
                    payload_id,
                    wifi.ssid.clone(),
                    wifi.security_type,
                )));
                wifi_credentials_metadata.push(sharing_nearby::WifiCredentialsMetadata {
                    ssid: Some(wifi.ssid.clone()),
                    security_type: Some(wifi.security_type.into()),
                    payload_id: Some(payload_id),
                    id: Some(payload_id),
                });
            }
            OutboundPayload::Text { text, text_type } => {
                anyhow::ensure!(
                    *text_type != TextPayloadType::Wifi,
                    "Use a Wi-Fi credential payload instead of text"
                );
                let payload_id = rand::rng().random::<i64>();
                let text_bytes = text.as_bytes();
                total_to_send = text_bytes.len() as u64;

                text_payload_info = Some(TextPayloadInfo::from_type(text_type, payload_id));

                let tmeta = TextMetadata {
                    text_title: None,
                    r#type: Some(text_type.to_proto_type().into()),
                    payload_id: Some(payload_id),
                    size: Some(text_bytes.len() as i64),
                    id: Some(payload_id),
                };
                text_metadata.push(tmeta);

                info!(
                    "Text type to send: {:?}, size: {}",
                    text_type,
                    text_bytes.len()
                );
            }
        }

        self.update_state(
            |e| {
                if let Some(tmd) = e.transfer_metadata.as_mut() {
                    tmd.total_bytes = total_to_send;
                }
                e.transferred_files = transferred_files;
                if text_payload_info.is_some() {
                    e.text_payload = text_payload_info;
                }
            },
            false,
        )
        .await;

        let intro_frame = IntroductionFrame {
            file_metadata,
            text_metadata,
            wifi_credentials_metadata,
            ..Default::default()
        };

        let introduction = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::Introduction.into()),
                introduction: Some(intro_frame),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&introduction).await?;

        Ok(())
    }

    async fn process_consent(
        &mut self,
        v1_frame: &sharing_nearby::V1Frame,
    ) -> Result<(), anyhow::Error> {
        if v1_frame.r#type() != sharing_nearby::v1_frame::FrameType::Response
            || v1_frame.connection_response.is_none()
        {
            return Err(anyhow!("Missing required fields"));
        }

        match v1_frame.connection_response.as_ref().unwrap().status() {
            sharing_nearby::connection_response_frame::Status::Accept => {
                info!("State is now State::SendingFiles");
                self.update_state(
                    |e| {
                        e.state = TransferState::SendingFiles;
                    },
                    true,
                )
                .await;

                // Handle sending Text
                if let Some(ref text_payload_info) = self.state.text_payload {
                    let payload_id = text_payload_info.get_i64_value();

                    let bytes = match &self.payload {
                        OutboundPayload::Text { text, .. } => {
                            Some((text.as_bytes().to_vec(), false))
                        }
                        OutboundPayload::Wifi(wifi) => Some((wifi.encode()?, true)),
                        OutboundPayload::Files(_) => None,
                    };
                    if let Some((text_bytes, sensitive)) = bytes {
                        let text_len = text_bytes.len() as i64;

                        info!("Sending text payload, size: {}", text_len);

                        let payload_header = PayloadHeader {
                            id: Some(payload_id),
                            r#type: Some(payload_header::PayloadType::Bytes.into()),
                            total_size: Some(text_len),
                            is_sensitive: Some(sensitive),
                            ..Default::default()
                        };

                        // Send the text content
                        let wrapper = location_nearby_connections::OfflineFrame {
                            version: Some(
                                location_nearby_connections::offline_frame::Version::V1.into(),
                            ),
                            v1: Some(location_nearby_connections::V1Frame {
                                r#type: Some(
                                    location_nearby_connections::v1_frame::FrameType::PayloadTransfer
                                        .into(),
                                ),
                                payload_transfer: Some(PayloadTransferFrame {
                                    packet_type: Some(PacketType::Data.into()),
                                    payload_chunk: Some(PayloadChunk {
                                        offset: Some(0),
                                        flags: Some(0),
                                        body: Some(text_bytes.clone()),
                                    }),
                                    payload_header: Some(payload_header.clone()),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }),
                        };

                        self.encrypt_and_send(&wrapper).await?;

                        // Send lastChunk marker
                        let wrapper = location_nearby_connections::OfflineFrame {
                            version: Some(
                                location_nearby_connections::offline_frame::Version::V1.into(),
                            ),
                            v1: Some(location_nearby_connections::V1Frame {
                                r#type: Some(
                                    location_nearby_connections::v1_frame::FrameType::PayloadTransfer
                                        .into(),
                                ),
                                payload_transfer: Some(PayloadTransferFrame {
                                    packet_type: Some(PacketType::Data.into()),
                                    payload_chunk: Some(PayloadChunk {
                                        offset: Some(text_len),
                                        flags: Some(1), // lastChunk
                                        body: Some(vec![]),
                                    }),
                                    payload_header: Some(payload_header),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }),
                        };

                        self.encrypt_and_send(&wrapper).await?;

                        // Update progress
                        self.update_state(
                            |e| {
                                if let Some(tmd) = e.transfer_metadata.as_mut() {
                                    tmd.ack_bytes = text_len as u64;
                                }
                            },
                            true,
                        )
                        .await;

                        // Mark as finished
                        info!("Text transfer finished");
                        self.update_state(
                            |e| {
                                e.state = TransferState::Finished;
                            },
                            true,
                        )
                        .await;
                        self.disconnection().await?;
                        return Ok(());
                    }
                }

                // Handle sending Files
                let ids: Vec<i64> = self.state.transferred_files.keys().cloned().collect();
                info!("We are sending: {:?}", ids);
                let mut ids_iter = ids.into_iter();

                // Loop through all files
                'send_all_files: loop {
                    let current = match ids_iter.next() {
                        Some(i) => i,
                        None => {
                            info!("All files have been transferred");
                            self.update_state(
                                |e| {
                                    e.state = TransferState::Finished;
                                },
                                true,
                            )
                            .await;
                            self.disconnection().await?;
                            // Breaking instead of NotAnError to allow peacefull termination
                            break;
                        }
                    };

                    // Loop until we reached end of file
                    loop {
                        // Since this task's runtime is blocked with the outer loop,
                        // OutboundRequest::handle() will not be called again.
                        // Thus, we need to check for cancellation here.
                        match self.receiver.try_recv() {
                            Ok(channel_msg) => {
                                if channel_msg.id == self.state.id {
                                    // TODO: if-let chains will be available in 1.88
                                    if let channel::Message::Lib { action } = &channel_msg.msg {
                                        debug!("outbound: got: {:?}", channel_msg);
                                        if action == &TransferAction::TransferCancel {
                                            self.update_state(
                                                |e| {
                                                    e.state = TransferState::Cancelled;
                                                },
                                                true,
                                            )
                                            .await;
                                            self.disconnection().await?;
                                            break 'send_all_files;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                match e {
                                    TryRecvError::Empty => {}
                                    e => {
                                        error!("inbound: channel error: {}", e)
                                    }
                                };
                            }
                        };

                        // Workaround to limit scope of the immutable borrow on self
                        let (curr_state, buffer, bytes_read) = {
                            let curr_state = match self.state.transferred_files.get(&current) {
                                Some(s) => s,
                                None => break,
                            };

                            info!("> Currently sending {:?}", curr_state.file_url);
                            if curr_state.bytes_transferred == curr_state.total_size {
                                // An empty file still needs its FILE/LAST_CHUNK
                                // frame; merely dropping it leaves the receiver
                                // waiting for a payload that will never arrive.
                                if curr_state.total_size == 0 {
                                    let header = PayloadHeader {
                                        id: Some(current),
                                        r#type: Some(payload_header::PayloadType::File.into()),
                                        total_size: Some(0),
                                        is_sensitive: Some(false),
                                        file_name: curr_state
                                            .file_url
                                            .file_name()
                                            .map(|name| name.to_string_lossy().into_owned()),
                                        ..Default::default()
                                    };
                                    self.send_file_completion(header).await?;
                                }
                                debug!("File {current} finished");
                                self.update_state(
                                    |e| {
                                        e.transferred_files.remove(&current);
                                    },
                                    false,
                                )
                                .await;
                                break;
                            }

                            if curr_state.file.is_none() {
                                warn!("File {current} is none");
                                break;
                            }

                            let buffer = read_file_chunk(
                                curr_state.file.as_ref().unwrap(),
                                curr_state.total_size - curr_state.bytes_transferred,
                            )?;
                            let bytes_read = buffer.len();

                            (
                                InternalFileInfo {
                                    payload_id: curr_state.payload_id,
                                    file_url: curr_state.file_url.clone(),
                                    bytes_transferred: curr_state.bytes_transferred,
                                    total_size: curr_state.total_size,
                                    file: None,
                                },
                                buffer,
                                bytes_read,
                            )
                        };

                        let sending_buffer = buffer[..bytes_read].to_vec();
                        info!(
                            "> File ready: {bytes_read} bytes && {} && left to send: {} with current offset: {}",
                            sending_buffer.len(),
                            curr_state.total_size - curr_state.bytes_transferred,
                            curr_state.bytes_transferred
                        );

                        let payload_header = PayloadHeader {
                            id: Some(current),
                            r#type: Some(payload_header::PayloadType::File.into()),
                            total_size: Some(curr_state.total_size),
                            is_sensitive: Some(false),
                            file_name: curr_state
                                .file_url
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned()),
                            ..Default::default()
                        };

                        let wrapper = location_nearby_connections::OfflineFrame {
							version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
							v1: Some(location_nearby_connections::V1Frame {
								r#type: Some(
									location_nearby_connections::v1_frame::FrameType::PayloadTransfer.into(),
								),
								payload_transfer: Some(PayloadTransferFrame {
									packet_type: Some(PacketType::Data.into()),
									payload_chunk: Some(PayloadChunk {
										offset: Some(curr_state.bytes_transferred),
										flags: Some(0),
										body: Some(buffer[..bytes_read].to_vec()),
									}),
									payload_header: Some(payload_header.clone()),
									..Default::default()
								}),
								..Default::default()
							}),
						};

                        self.encrypt_and_send(&wrapper).await?;
                        self.update_state(
                            |e| {
                                if let Some(mu) = e.transferred_files.get_mut(&current) {
                                    mu.bytes_transferred += bytes_read as i64;
                                }

                                if let Some(tmd) = e.transfer_metadata.as_mut() {
                                    tmd.ack_bytes += bytes_read as u64;
                                }
                            },
                            true,
                        )
                        .await;

                        // If we just sent the last bytes of the file, mark it as finished
                        if curr_state.bytes_transferred + bytes_read as i64 == curr_state.total_size
                        {
                            debug!(
                                "File {current} finished, curr offset: {} over total: {}",
                                curr_state.bytes_transferred + bytes_read as i64,
                                curr_state.total_size
                            );

                            self.send_file_completion(payload_header).await?;
                            break;
                        }
                    }
                }
            }
            sharing_nearby::connection_response_frame::Status::Reject
            | sharing_nearby::connection_response_frame::Status::NotEnoughSpace
            | sharing_nearby::connection_response_frame::Status::UnsupportedAttachmentType
            | sharing_nearby::connection_response_frame::Status::TimedOut => {
                warn!(
                    "Cannot process: consent denied: {:?}",
                    v1_frame.connection_response.as_ref().unwrap().status()
                );
                self.update_state(
                    |e| {
                        e.state = TransferState::Disconnected;
                    },
                    true,
                )
                .await;
                self.disconnection().await?;
                return Err(anyhow!(crate::errors::AppError::NotAnError));
            }
            sharing_nearby::connection_response_frame::Status::Unknown => {
                error!("Unknown consent type: aborting");
                self.update_state(
                    |e| {
                        e.state = TransferState::Disconnected;
                    },
                    true,
                )
                .await;
                self.disconnection().await?;
                return Err(anyhow!(crate::errors::AppError::NotAnError));
            }
        }

        Ok(())
    }

    async fn send_file_completion(&mut self, header: PayloadHeader) -> Result<(), anyhow::Error> {
        let frame = OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::PayloadTransfer.into(),
                ),
                payload_transfer: Some(PayloadTransferFrame {
                    packet_type: Some(PacketType::Data.into()),
                    payload_chunk: Some(PayloadChunk {
                        offset: Some(header.total_size()),
                        flags: Some(1),
                        body: Some(vec![]),
                    }),
                    payload_header: Some(header),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        self.encrypt_and_send(&frame).await
    }

    async fn disconnection(&mut self) -> Result<(), anyhow::Error> {
        let frame = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::Disconnection.into(),
                ),
                disconnection: Some(location_nearby_connections::DisconnectionFrame {
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };

        if self.state.encryption_done {
            self.encrypt_and_send(&frame).await
        } else {
            self.send_frame(frame.encode_to_vec()).await
        }
    }

    async fn finalize_key_exchange(
        &mut self,
        raw_peer_key: GenericPublicKey,
    ) -> Result<(), anyhow::Error> {
        let peer_key = super::crypto::peer_public_key(raw_peer_key)?;
        let priv_key = self
            .state
            .private_key
            .as_ref()
            .ok_or_else(|| anyhow!("Missing local handshake key"))?;

        let dhs = diffie_hellman(priv_key.to_nonzero_scalar(), peer_key.as_affine());
        let derived_secret = Sha256::digest(dhs.raw_secret_bytes());

        let mut ukey_info: Vec<u8> = vec![];
        ukey_info.extend_from_slice(
            self.state
                .client_init_msg_data
                .as_ref()
                .ok_or_else(|| anyhow!("Missing client handshake transcript"))?,
        );
        ukey_info.extend_from_slice(
            self.state
                .server_init_data
                .as_ref()
                .ok_or_else(|| anyhow!("Missing server handshake transcript"))?,
        );

        let auth_label = "UKEY2 v1 auth".as_bytes();
        let next_label = "UKEY2 v1 next".as_bytes();

        let auth_string = hkdf_extract_expand(auth_label, &derived_secret, &ukey_info, 32)?;
        let next_secret = hkdf_extract_expand(next_label, &derived_secret, &ukey_info, 32)?;

        let salt_hex = "82AA55A0D397F88346CA1CEE8D3909B95F13FA7DEB1D4AB38376B8256DA85510";
        let salt =
            hex::decode(salt_hex).map_err(|e| anyhow!("Failed to decode salt_hex: {}", e))?;

        let d2d_client = hkdf_extract_expand(&salt, &next_secret, "client".as_bytes(), 32)?;
        let d2d_server = hkdf_extract_expand(&salt, &next_secret, "server".as_bytes(), 32)?;

        let key_salt_hex = "BF9D2A53C63616D75DB0A7165B91C1EF73E537F2427405FA23610A4BE657642E";
        let key_salt = hex::decode(key_salt_hex)
            .map_err(|e| anyhow!("Failed to decode key_salt_hex: {}", e))?;

        let client_key = hkdf_extract_expand(&key_salt, &d2d_client, "ENC:2".as_bytes(), 32)?;
        let client_hmac_key = hkdf_extract_expand(&key_salt, &d2d_client, "SIG:1".as_bytes(), 32)?;
        let server_key = hkdf_extract_expand(&key_salt, &d2d_server, "ENC:2".as_bytes(), 32)?;
        let server_hmac_key = hkdf_extract_expand(&key_salt, &d2d_server, "SIG:1".as_bytes(), 32)?;

        self.update_state(
            |e| {
                e.decrypt_key = Some(server_key);
                e.recv_hmac_key = Some(server_hmac_key);
                e.encrypt_key = Some(client_key);
                e.send_hmac_key = Some(client_hmac_key);
                e.pin_code = Some(to_four_digit_string(&auth_string));
                e.encryption_done = true;

                if let Some(ref mut tm) = e.transfer_metadata {
                    tm.pin_code = Some(to_four_digit_string(&auth_string));
                }
            },
            true,
        )
        .await;

        info!("Pin code: {:?}", self.state.pin_code);

        Ok(())
    }

    async fn send_ukey2_alert(&mut self, atype: AlertType) -> Result<(), anyhow::Error> {
        let alert = Ukey2Alert {
            r#type: Some(atype.into()),
            error_message: None,
        };

        let data = Ukey2Message {
            message_type: Some(atype.into()),
            message_data: Some(alert.encode_to_vec()),
        };

        self.send_frame(data.encode_to_vec()).await
    }

    async fn send_encrypted_frame(
        &mut self,
        frame: &sharing_nearby::Frame,
    ) -> Result<(), anyhow::Error> {
        let frame_data = frame.encode_to_vec();
        let body_size = frame_data.len();

        let payload_header = PayloadHeader {
            id: Some(rand::rng().random_range(i64::MIN..i64::MAX)),
            r#type: Some(payload_header::PayloadType::Bytes.into()),
            total_size: Some(body_size as i64),
            is_sensitive: Some(false),
            ..Default::default()
        };

        let transfer = PayloadTransferFrame {
            packet_type: Some(PacketType::Data.into()),
            payload_chunk: Some(PayloadChunk {
                offset: Some(0),
                flags: Some(0),
                body: Some(frame_data),
            }),
            payload_header: Some(payload_header.clone()),
            ..Default::default()
        };

        let wrapper = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::PayloadTransfer.into(),
                ),
                payload_transfer: Some(transfer),
                ..Default::default()
            }),
        };

        // Encrypt and send offline
        self.encrypt_and_send(&wrapper).await?;

        // Send lastChunk
        let transfer = PayloadTransferFrame {
            packet_type: Some(PacketType::Data.into()),
            payload_chunk: Some(PayloadChunk {
                offset: Some(body_size as i64),
                flags: Some(1), // lastChunk
                body: Some(vec![]),
            }),
            payload_header: Some(payload_header),
            ..Default::default()
        };

        let wrapper = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::PayloadTransfer.into(),
                ),
                payload_transfer: Some(transfer),
                ..Default::default()
            }),
        };

        // Encrypt and send offline
        self.encrypt_and_send(&wrapper).await?;

        Ok(())
    }

    async fn encrypt_and_send(&mut self, frame: &OfflineFrame) -> Result<(), anyhow::Error> {
        let d2d_msg = DeviceToDeviceMessage {
            sequence_number: Some(self.get_server_seq_inc().await),
            message: Some(frame.encode_to_vec()),
        };

        let key = self.state.encrypt_key.as_ref().unwrap();
        let msg_data = d2d_msg.encode_to_vec();
        let iv = gen_random(16);

        let mut cipher = Cipher::new_256(&key[..AES_256_KEY_LEN].try_into().unwrap());
        cipher.set_auto_padding(true);
        let encrypted = cipher.cbc_encrypt(&iv, &msg_data);

        let hb = HeaderAndBody {
            body: encrypted,
            header: Header {
                encryption_scheme: EncScheme::Aes256Cbc.into(),
                signature_scheme: SigScheme::HmacSha256.into(),
                iv: Some(iv),
                public_metadata: Some(
                    GcmMetadata {
                        r#type: Type::DeviceToDeviceMessage.into(),
                        version: Some(1),
                    }
                    .encode_to_vec(),
                ),
                ..Default::default()
            },
        };

        let mut hmac = HmacSha256::new_from_slice(self.state.send_hmac_key.as_ref().unwrap())?;
        hmac.update(&hb.encode_to_vec());
        let result = hmac.finalize();

        let smsg = SecureMessage {
            header_and_body: hb.encode_to_vec(),
            signature: result.into_bytes().to_vec(),
        };

        self.send_frame(smsg.encode_to_vec()).await?;

        Ok(())
    }

    async fn send_keepalive(&mut self, ack: bool) -> Result<(), anyhow::Error> {
        let ack_frame = location_nearby_connections::OfflineFrame {
            version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(location_nearby_connections::v1_frame::FrameType::KeepAlive.into()),
                keep_alive: Some(KeepAliveFrame { ack: Some(ack) }),
                ..Default::default()
            }),
        };

        if self.state.encryption_done {
            self.encrypt_and_send(&ack_frame).await
        } else {
            self.send_frame(ack_frame.encode_to_vec()).await
        }
    }

    async fn send_frame(&mut self, data: Vec<u8>) -> Result<(), anyhow::Error> {
        super::frame::write(&mut self.socket, &data).await?;
        Ok(())
    }

    async fn get_server_seq_inc(&mut self) -> i32 {
        self.update_state(
            |e| {
                e.server_seq += 1;
            },
            false,
        )
        .await;

        self.state.server_seq
    }

    async fn get_client_seq_inc(&mut self) -> i32 {
        self.update_state(
            |e| {
                e.client_seq += 1;
            },
            false,
        )
        .await;

        self.state.client_seq
    }

    async fn update_state<F>(&mut self, f: F, inform: bool)
    where
        F: FnOnce(&mut InnerState),
    {
        f(&mut self.state);

        if !inform {
            return;
        }

        let _ = self.sender.send(ChannelMessage {
            id: self.state.id.clone(),
            msg: channel::Message::Client(MessageClient {
                kind: TransferKind::Outbound,
                state: Some(self.state.state.clone()),
                metadata: self.state.transfer_metadata.clone(),
            }),
        });
        // Add a small sleep timer to allow the Tokio runtime to have
        // some spare time to process channel's message. Otherwise it
        // get spammed by new requests. Currently set to 10 micro secs.
        tokio::time::sleep(SANITY_DURATION).await;
    }
}

// Read no more than the approved offer size even if the source grows. A source
// truncated after introduction must fail, rather than emitting endless empty
// non-final chunks while the peer waits for the advertised bytes.
fn read_file_chunk(mut file: &File, remaining: i64) -> std::io::Result<Vec<u8>> {
    if remaining <= 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "No file bytes remain",
        ));
    }
    let mut buffer = vec![0; remaining.min(512 * 1024) as usize];
    let count = file.read(&mut buffer)?;
    if count == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "Source file ended before its offered size",
        ));
    }
    buffer.truncate(count);
    Ok(buffer)
}

#[cfg(test)]
mod file_regressions {
    use super::*;
    #[test]
    fn reads_respect_offer_size_and_reject_truncated_sources() {
        let directory =
            std::env::temp_dir().join(format!("quickshare-source-{}", rand::random::<u64>()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("growing.txt");
        std::fs::write(&path, b"offered appended").unwrap();
        let file = File::open(&path).unwrap();
        assert_eq!(read_file_chunk(&file, 7).unwrap(), b"offered");
        let shortened = directory.join("shortened.txt");
        std::fs::write(&shortened, b"original").unwrap();
        let file = File::open(&shortened).unwrap();
        File::options()
            .write(true)
            .open(&shortened)
            .unwrap()
            .set_len(0)
            .unwrap();
        assert_eq!(
            read_file_chunk(&file, 8).unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
