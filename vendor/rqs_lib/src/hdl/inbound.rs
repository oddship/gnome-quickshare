use std::fs::File;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::PathBuf;
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
#[cfg(test)]
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::broadcast::{Receiver, Sender};

use super::{InnerState, TransferState};
use crate::channel::{self, ChannelMessage, MessageClient, TransferAction, TransferKind};
use crate::hdl::TextPayloadInfo;
use crate::hdl::info::{InternalFileInfo, TransferMetadata, TransferPayload, TransferPayloadKind};
use crate::location_nearby_connections::payload_transfer_frame::{
    PacketType, PayloadChunk, PayloadHeader, payload_header,
};
use crate::location_nearby_connections::{KeepAliveFrame, OfflineFrame, PayloadTransferFrame};
use crate::securegcm::ukey2_alert::AlertType;
use crate::securegcm::{
    DeviceToDeviceMessage, GcmMetadata, Type, Ukey2Alert, Ukey2ClientFinished, Ukey2ClientInit,
    Ukey2HandshakeCipher, Ukey2Message, Ukey2ServerInit, ukey2_message,
};
use crate::securemessage::{
    EcP256PublicKey, EncScheme, GenericPublicKey, Header, HeaderAndBody, PublicKeyType,
    SecureMessage, SigScheme,
};
use crate::sharing_nearby::wifi_credentials_metadata::SecurityType;
use crate::sharing_nearby::{paired_key_result_frame, text_metadata};
use crate::utils::{
    DeviceType, RemoteDeviceInfo, encode_point, gen_ecdsa_keypair, gen_random, get_download_dir,
    hkdf_extract_expand, to_four_digit_string,
};
use crate::{location_nearby_connections, sharing_nearby};

type HmacSha256 = Hmac<Sha256>;

const SANITY_DURATION: Duration = Duration::from_micros(10);

#[derive(Debug)]
pub struct InboundRequest {
    socket: TcpStream,
    frame_reader: super::frame::FrameReader,
    pub state: InnerState,
    sender: Sender<ChannelMessage>,
    receiver: Receiver<ChannelMessage>,
}

// A request owns only files it successfully created after consent. Completed
// files leave transferred_files; everything still tracked is incomplete, even
// when all bytes arrived but the peer never sent the completion marker.
impl Drop for InboundRequest {
    fn drop(&mut self) {
        for pending in self.state.transferred_files.values() {
            let Some(file) = pending.file.as_ref() else {
                continue;
            };
            let Ok(opened) = file.metadata() else {
                continue;
            };
            let Ok(current) = std::fs::symlink_metadata(&pending.file_url) else {
                continue;
            };
            // Do not delete a replacement file or follow a replacement symlink.
            if current.is_file()
                && current.dev() == opened.dev()
                && current.ino() == opened.ino()
                && let Err(error) = std::fs::remove_file(&pending.file_url)
            {
                warn!("Could not remove incomplete receive: {error}");
            }
        }
    }
}

impl InboundRequest {
    pub fn new(socket: TcpStream, id: String, sender: Sender<ChannelMessage>) -> Self {
        let receiver = sender.subscribe();

        Self {
            socket,
            frame_reader: Default::default(),
            state: InnerState {
                id,
                server_seq: 0,
                client_seq: 0,
                state: TransferState::Initial,
                // Encryption is enabled only after key derivation succeeds.
                encryption_done: false,
                ..Default::default()
            },
            sender,
            receiver,
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

                        if let channel::Message::Lib { action } = &channel_msg.msg {
                            debug!("inbound: got: {:?}", channel_msg);
                            match action {
                                TransferAction::ConsentAccept => {
                                    self.accept_transfer().await?;
                                },
                                TransferAction::ConsentDecline => {
                                    self.update_state(
                                        |e| {
                                            e.state = TransferState::Rejected;
                                        },
                                        true,
                                    ).await;

                                    self.reject_transfer(Some(
                                        sharing_nearby::connection_response_frame::Status::Reject
                                    )).await?;
                                    return Err(anyhow!(crate::errors::AppError::NotAnError));
                                },
                                TransferAction::TransferCancel => {
                                    self.update_state(
                                        |e| {
                                            e.state = TransferState::Cancelled;
                                        },
                                        true,
                                    ).await;
                                    self.disconnection().await?;
                                    return Err(anyhow!(crate::errors::AppError::NotAnError));
                                },
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
            TransferState::Initial => {
                debug!("Handling State::Initial frame");
                let frame = location_nearby_connections::OfflineFrame::decode(&*frame_data)?;
                let rdi = self.process_connection_request(&frame)?;
                info!("RemoteDeviceInfo: {:?}", rdi);

                // Advance current state
                self.update_state(
                    |e: &mut InnerState| {
                        e.state = TransferState::ReceivedConnectionRequest;
                        e.remote_device_info = Some(rdi);
                    },
                    false,
                )
                .await;
            }
            TransferState::ReceivedConnectionRequest => {
                debug!("Handling State::ReceivedConnectionRequest frame");
                let msg = Ukey2Message::decode(&*frame_data)?;
                self.process_ukey2_client_init(&msg).await?;

                self.update_state(
                    |e: &mut InnerState| {
                        e.state = TransferState::SentUkeyServerInit;
                        e.client_init_msg_data = Some(frame_data);
                    },
                    false,
                )
                .await;
            }
            TransferState::SentUkeyServerInit => {
                debug!("Handling State::SentUkeyServerInit frame");
                let msg = Ukey2Message::decode(&*frame_data)?;
                self.process_ukey2_client_finish(&msg, &frame_data).await?;

                self.update_state(
                    |e: &mut InnerState| {
                        e.state = TransferState::ReceivedUkeyClientFinish;
                    },
                    false,
                )
                .await;
            }
            TransferState::ReceivedUkeyClientFinish => {
                debug!("Handling State::ReceivedUkeyClientFinish frame");
                let frame = location_nearby_connections::OfflineFrame::decode(&*frame_data)?;
                self.process_connection_response(&frame).await?;

                self.update_state(
                    |e: &mut InnerState| {
                        e.state = TransferState::SentConnectionResponse;
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

    fn process_connection_request(
        &self,
        frame: &location_nearby_connections::OfflineFrame,
    ) -> Result<RemoteDeviceInfo, anyhow::Error> {
        let v1_frame = frame
            .v1
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        if v1_frame.r#type() != location_nearby_connections::v1_frame::FrameType::ConnectionRequest
        {
            return Err(anyhow!(format!(
                "Unexpected frame type: {:?}",
                v1_frame.r#type()
            )));
        }

        let connection_request = v1_frame
            .connection_request
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        let endpoint_info = connection_request
            .endpoint_info
            .as_ref()
            .ok_or_else(|| anyhow!("Missing endpoint info"))?;

        // Check if endpoint info length is greater than 17
        if endpoint_info.len() <= 17 {
            return Err(anyhow!("Endpoint info too short"));
        }

        let device_name_length = endpoint_info[17] as usize;
        // Validate length including device name
        if endpoint_info.len() < device_name_length + 18 {
            return Err(anyhow!(
                "Endpoint info too short to contain the device name"
            ));
        }

        // Extract and validate device name based on length
        let device_name = std::str::from_utf8(&endpoint_info[18..(18 + device_name_length)])
            .map_err(|_| anyhow!("Device name is not valid UTF-8"))?;

        // Parsing the device type
        let raw_device_type = (endpoint_info[0] & 7) >> 1_usize;

        Ok(RemoteDeviceInfo {
            name: device_name.to_string(),
            device_type: DeviceType::from_raw_value(raw_device_type),
        })
    }

    async fn process_ukey2_client_init(&mut self, msg: &Ukey2Message) -> Result<(), anyhow::Error> {
        if msg.message_type() != ukey2_message::Type::ClientInit {
            self.send_ukey2_alert(AlertType::BadMessageType).await?;
            return Err(anyhow!(
                "UKey2: message_type({:?}) != ClientInit",
                msg.message_type
            ));
        }

        let client_init = match Ukey2ClientInit::decode(msg.message_data()) {
            Ok(uk2ci) => uk2ci,
            Err(e) => {
                self.send_ukey2_alert(AlertType::BadMessageData).await?;
                return Err(anyhow!("UKey2: Ukey2ClientInit::decode: {}", e));
            }
        };

        if client_init.version() != 1 {
            self.send_ukey2_alert(AlertType::BadVersion).await?;
            return Err(anyhow!("UKey2: client_init.version != 1"));
        }

        if client_init.random().len() != 32 {
            self.send_ukey2_alert(AlertType::BadRandom).await?;
            return Err(anyhow!("UKey2: client_init.random.len != 32"));
        }

        // Searching for preferred cipher commitment
        let mut found = false;
        for commitment in &client_init.cipher_commitments {
            trace!("CipherCommitment: {:?}", commitment.handshake_cipher());
            if Ukey2HandshakeCipher::P256Sha512 == commitment.handshake_cipher() {
                found = true;
                self.update_state(
                    |e| {
                        e.cipher_commitment = Some(commitment.clone());
                    },
                    false,
                )
                .await;
                break;
            }
        }

        if !found {
            self.send_ukey2_alert(AlertType::BadHandshakeCipher).await?;
            return Err(anyhow!("UKey2: badHandshakeCipher"));
        }

        if client_init.next_protocol() != "AES_256_CBC-HMAC_SHA256" {
            self.send_ukey2_alert(AlertType::BadNextProtocol).await?;
            return Err(anyhow!(
                "UKey2: badNextProtocol: {}",
                client_init.next_protocol()
            ));
        }

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

        let server_init = Ukey2ServerInit {
            version: Some(1),
            random: Some(rand::rng().random::<[u8; 32]>().to_vec()),
            handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512.into()),
            public_key: Some(pkey.encode_to_vec()),
        };

        let server_init_msg = Ukey2Message {
            message_type: Some(ukey2_message::Type::ServerInit.into()),
            message_data: Some(server_init.encode_to_vec()),
        };

        let server_init_data = server_init_msg.encode_to_vec();
        self.update_state(
            |e| {
                e.private_key = Some(secret_key);
                e.public_key = Some(public_key);
                e.server_init_data = Some(server_init_data.clone());
            },
            false,
        )
        .await;

        self.send_frame(server_init_data).await?;

        Ok(())
    }

    async fn process_ukey2_client_finish(
        &mut self,
        msg: &Ukey2Message,
        frame_data: &Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        if msg.message_type() != ukey2_message::Type::ClientFinish {
            self.send_ukey2_alert(AlertType::BadMessageType).await?;
            return Err(anyhow!(
                "UKey2: message_type({:?}) != ClientFinish",
                msg.message_type
            ));
        }

        let sha512 = Sha512::digest(frame_data);
        if self.state.cipher_commitment.as_ref().unwrap().commitment() != &sha512[..] {
            error!("cipher_commitment isn't equals to sha512(frame_data)");
            return Err(anyhow!("UKey2: cipher_commitment != sha512"));
        }

        let client_finish = match Ukey2ClientFinished::decode(msg.message_data()) {
            Ok(uk2cf) => uk2cf,
            Err(e) => {
                return Err(anyhow!("UKey2: Ukey2ClientFinished::decode: {}", e));
            }
        };

        if client_finish.public_key.is_none() {
            return Err(anyhow!("UKey2: client_finish.public_key None"));
        }

        let client_public_key = match GenericPublicKey::decode(client_finish.public_key()) {
            Ok(cpk) => cpk,
            Err(e) => {
                return Err(anyhow!("UKey2: GenericPublicKey::decode: {}", e));
            }
        };

        self.finalize_key_exchange(client_public_key).await?;

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

        let response = location_nearby_connections::OfflineFrame {
			version: Some(location_nearby_connections::offline_frame::Version::V1.into()),
			v1: Some(location_nearby_connections::V1Frame {
				r#type: Some(location_nearby_connections::v1_frame::FrameType::ConnectionResponse.into()),
				connection_response: Some(location_nearby_connections::ConnectionResponseFrame {
					response: Some(location_nearby_connections::connection_response_frame::ResponseStatus::Accept.into()),
					os_info: Some(location_nearby_connections::OsInfo {
						r#type: Some(location_nearby_connections::os_info::OsType::Linux.into())
					}),
					..Default::default()
				}),
				..Default::default()
			})
		};

        self.send_frame(response.encode_to_vec()).await?;

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
                        if self
                            .state
                            .text_payload
                            .as_ref()
                            .is_some_and(|text| text.get_i64_value() == payload_id)
                            && self.state.state != TransferState::ReceivingFiles
                        {
                            return Err(anyhow!("Text payload received before consent"));
                        }

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
                            if buffer.len() != header.total_size() as usize {
                                return Err(anyhow!("Payload completed before its declared size"));
                            }
                            debug!("Received complete byte payload");

                            if self.state.text_payload.is_some()
                                && self.state.text_payload.as_ref().unwrap().get_i64_value()
                                    == payload_id
                            {
                                info!("Transfer finished");

                                match self.state.text_payload.clone().unwrap() {
                                    TextPayloadInfo::Url(_) => {
                                        let payload = std::str::from_utf8(buffer)?.to_owned();
                                        self.update_state(
                                            |e| {
                                                if let Some(tmd) = e.transfer_metadata.as_mut() {
                                                    tmd.payload =
                                                        Some(TransferPayload::Url(payload));
                                                }
                                            },
                                            false,
                                        )
                                        .await;
                                    }
                                    TextPayloadInfo::Text(_) => {
                                        let payload = std::str::from_utf8(buffer)?.to_owned();
                                        self.update_state(
                                            |e| {
                                                if let Some(tmd) = e.transfer_metadata.as_mut() {
                                                    tmd.payload =
                                                        Some(TransferPayload::Text(payload));
                                                }
                                            },
                                            false,
                                        )
                                        .await;
                                    }
                                    // FIXME: ChannelMessage's structure needs to be redone as well
                                    // It needs to have TextPayloadInfo instead of TextPayloadType
                                    TextPayloadInfo::Wifi((_, ssid, security_type)) => {
                                        let wifi = super::WifiPayload::decode(
                                            ssid.clone(),
                                            security_type,
                                            buffer,
                                        )?;

                                        self.update_state(
                                            |e| {
                                                if let Some(tmd) = e.transfer_metadata.as_mut() {
                                                    tmd.payload = Some(TransferPayload::Wifi {
                                                        ssid,
                                                        password: wifi.password,
                                                        security_type,
                                                        hidden_ssid: wifi.hidden_ssid,
                                                    });
                                                }
                                            },
                                            false,
                                        )
                                        .await;
                                    }
                                }

                                self.update_state(
                                    |e| {
                                        e.state = TransferState::Finished;
                                        if let Some(metadata) = &mut e.transfer_metadata {
                                            metadata.total_bytes = header.total_size() as u64;
                                            metadata.ack_bytes = header.total_size() as u64;
                                        }
                                    },
                                    true,
                                )
                                .await;
                                self.disconnection().await?;
                                return Err(anyhow!(crate::errors::AppError::NotAnError));
                            } else {
                                let inner_frame = sharing_nearby::Frame::decode(buffer.as_slice())?;
                                self.process_transfer_setup(&inner_frame).await?;
                            }
                        }
                    }
                    payload_header::PayloadType::File => {
                        info!("Processing PayloadType::File");
                        let payload_id = header.id();
                        if self.state.state != TransferState::ReceivingFiles {
                            return Err(anyhow!("File payload received before consent"));
                        }

                        let file_internal = self
                            .state
                            .transferred_files
                            .get_mut(&payload_id)
                            .ok_or_else(|| {
                                anyhow!("File payload ID ({}) is not known", payload_id)
                            })?;

                        let current_offset = file_internal.bytes_transferred;
                        if chunk.offset() != current_offset {
                            return Err(anyhow!(
                                "Invalid offset into file {}, expected {}",
                                chunk.offset(),
                                current_offset
                            ));
                        }

                        let chunk_size = chunk.body().len();
                        if current_offset + chunk_size as i64 > file_internal.total_size {
                            return Err(anyhow!(
                                "Transferred file size exceeds previously specified value: {} vs {}",
                                current_offset + chunk_size as i64,
                                file_internal.total_size
                            ));
                        }

                        if !chunk.body().is_empty() {
                            file_internal
                                .file
                                .as_ref()
                                .ok_or_else(|| anyhow!("File was not approved"))?
                                .write_all_at(chunk.body(), current_offset as u64)?;
                            file_internal.bytes_transferred += chunk_size as i64;

                            self.update_state(
                                |e| {
                                    if let Some(tmd) = e.transfer_metadata.as_mut() {
                                        tmd.ack_bytes += chunk_size as u64;
                                    }
                                },
                                true,
                            )
                            .await;
                        } else if (chunk.flags() & 1) == 1 {
                            if file_internal.bytes_transferred != file_internal.total_size {
                                return Err(anyhow!("File ended before its declared size"));
                            }
                            self.state.transferred_files.remove(&payload_id);
                            if self.state.transferred_files.is_empty() {
                                info!("Transfer finished");
                                self.update_state(
                                    |e| {
                                        e.state = TransferState::Finished;
                                    },
                                    true,
                                )
                                .await;
                                self.disconnection().await?;
                                return Err(anyhow!(crate::errors::AppError::NotAnError));
                            }
                        }
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
            TransferState::SentConnectionResponse => {
                debug!("Processing State::SentConnectionResponse");
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
                        e.state = TransferState::ReceivedPairedKeyResult;
                    },
                    false,
                )
                .await;
            }
            TransferState::ReceivedPairedKeyResult => {
                debug!("Processing State::ReceivedPairedKeyResult");
                self.process_introduction(v1_frame).await?;
            }
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
        &self,
        v1_frame: &sharing_nearby::V1Frame,
    ) -> Result<(), anyhow::Error> {
        if v1_frame.paired_key_result.is_none() {
            return Err(anyhow!("Missing required fields"));
        }

        Ok(())
    }

    async fn process_introduction(
        &mut self,
        v1_frame: &sharing_nearby::V1Frame,
    ) -> Result<(), anyhow::Error> {
        let introduction = v1_frame
            .introduction
            .as_ref()
            .ok_or_else(|| anyhow!("Missing required fields"))?;

        // No need to inform the channel here, we'll do it anyway with files info
        self.update_state(
            |e| {
                e.state = TransferState::WaitingForUserConsent;
            },
            false,
        )
        .await;

        if introduction.file_metadata.len() > 1024 {
            return Err(anyhow!("Too many files in offer"));
        }
        let mut payload_ids = std::collections::HashSet::new();
        for file in &introduction.file_metadata {
            crate::utils::validate_filename(file.name())?;
            if file.size() < 0 || !payload_ids.insert(file.payload_id()) {
                return Err(anyhow!("Invalid file metadata"));
            }
        }
        if !introduction.file_metadata.is_empty() && introduction.text_metadata.is_empty() {
            trace!("process_introduction: handling file_metadata");
            let mut files_name = Vec::with_capacity(introduction.file_metadata.len());
            let mut total_bytes: u64 = 0;

            for file in &introduction.file_metadata {
                info!("File name: {}", file.name());

                let mut dest = get_download_dir();
                dest.push(file.name());

                info!("Destination: {:?}", dest);
                if dest.exists()
                    || self
                        .state
                        .transferred_files
                        .values()
                        .any(|pending| pending.file_url == dest)
                {
                    let mut counter = 1;
                    dest.pop();

                    loop {
                        // How name conflict resolution happens in most software,
                        // lorem.txt
                        // lorem (1).txt
                        // ...
                        let incremented_file_path = {
                            let file_path = PathBuf::from(file.name());

                            let incremented_file_stem = format!(
                                "{} ({counter})",
                                file_path
                                    .file_stem()
                                    .and_then(|it| it.to_str())
                                    .expect("Must be UTF-8 since Path is derived from String"),
                            );

                            let incremented_file_path =
                                file_path.with_file_name(incremented_file_stem);

                            file_path
                                .extension()
                                .map(|it| incremented_file_path.with_extension(it))
                                .unwrap_or_else(|| incremented_file_path)
                        };

                        dest.push(incremented_file_path);
                        if !dest.exists()
                            && !self
                                .state
                                .transferred_files
                                .values()
                                .any(|pending| pending.file_url == dest)
                        {
                            break;
                        }
                        dest.pop();
                        counter += 1;
                    }

                    info!("New destination: {:?}", dest);
                }

                let info = InternalFileInfo {
                    payload_id: file.payload_id(),
                    file_url: dest,
                    bytes_transferred: 0,
                    total_size: file.size(),
                    file: None,
                };
                total_bytes = total_bytes
                    .checked_add(info.total_size as u64)
                    .ok_or_else(|| anyhow!("Total file size overflow"))?;
                self.state.transferred_files.insert(file.payload_id(), info);
                files_name.push(file.name().to_owned());
            }

            let metadata = TransferMetadata {
                id: self.state.id.clone(),
                source: self.state.remote_device_info.clone(),
                payload_kind: TransferPayloadKind::Files,
                payload_preview: Default::default(),
                payload: Some(TransferPayload::Files(files_name)),
                pin_code: self.state.pin_code.clone(),
                total_bytes,
                ack_bytes: Default::default(),
            };

            info!("Asking for user consent: {:?}", metadata);
            self.update_state(
                |e| {
                    e.transfer_metadata = Some(metadata);
                },
                true,
            )
            .await;
        } else if introduction.text_metadata.len() == 1 {
            trace!("process_introduction: handling text_metadata");
            let meta = introduction.text_metadata.first().unwrap();

            match meta.r#type() {
                text_metadata::Type::Url => {
                    let metadata = TransferMetadata {
                        id: self.state.id.clone(),
                        source: self.state.remote_device_info.clone(),
                        payload_kind: TransferPayloadKind::Url,
                        payload_preview: Some(meta.text_title.clone().unwrap_or_default()),
                        pin_code: self.state.pin_code.clone(),
                        payload: Default::default(),
                        total_bytes: Default::default(),
                        ack_bytes: Default::default(),
                    };

                    info!("Asking for user consent: {:?}", metadata);
                    self.update_state(
                        |e| {
                            e.text_payload = Some(TextPayloadInfo::Url(meta.payload_id()));
                            e.transfer_metadata = Some(metadata);
                        },
                        true,
                    )
                    .await;
                }
                text_metadata::Type::PhoneNumber
                | text_metadata::Type::Address
                | text_metadata::Type::Text => {
                    let metadata = TransferMetadata {
                        id: self.state.id.clone(),
                        source: self.state.remote_device_info.clone(),
                        payload_kind: TransferPayloadKind::Text,
                        payload_preview: Some(meta.text_title.clone().unwrap_or_default()),
                        pin_code: self.state.pin_code.clone(),
                        payload: Default::default(),
                        total_bytes: Default::default(),
                        ack_bytes: Default::default(),
                    };

                    info!("Asking for user consent: {:?}", metadata);
                    self.update_state(
                        |e| {
                            e.text_payload = Some(TextPayloadInfo::Text(meta.payload_id()));
                            e.transfer_metadata = Some(metadata);
                        },
                        true,
                    )
                    .await;
                }
                text_metadata::Type::Unknown => {
                    // Reject transfer
                    self.reject_transfer(Some(
						sharing_nearby::connection_response_frame::Status::UnsupportedAttachmentType,
					))
					.await?;
                }
            }
        } else if introduction.wifi_credentials_metadata.len() == 1 {
            trace!("process_introduction: handling wifi_credentials_metadata");
            let meta = introduction.wifi_credentials_metadata.first().unwrap();
            if meta.ssid().is_empty()
                || meta.ssid().len() > 32
                || meta.security_type() == SecurityType::UnknownSecurityType
            {
                self.reject_transfer(Some(
                    sharing_nearby::connection_response_frame::Status::UnsupportedAttachmentType,
                ))
                .await?;
                return Ok(());
            }

            let metadata = TransferMetadata {
                id: self.state.id.clone(),
                source: self.state.remote_device_info.clone(),
                payload_kind: TransferPayloadKind::WiFi,
                payload_preview: Some(meta.ssid.clone().unwrap_or_default()),
                pin_code: self.state.pin_code.clone(),
                payload: Default::default(),
                total_bytes: Default::default(),
                ack_bytes: Default::default(),
            };

            self.update_state(
                |e| {
                    e.text_payload = Some(TextPayloadInfo::Wifi((
                        meta.payload_id(),
                        meta.ssid().to_owned(),
                        meta.security_type(),
                    )));
                    e.transfer_metadata = Some(metadata);
                },
                true,
            )
            .await;
        } else {
            // Reject transfer
            self.reject_transfer(Some(
                sharing_nearby::connection_response_frame::Status::UnsupportedAttachmentType,
            ))
            .await?;
        }

        Ok(())
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

    async fn accept_transfer(&mut self) -> Result<(), anyhow::Error> {
        if self.state.state != TransferState::WaitingForUserConsent {
            return Err(anyhow!("Consent is not pending"));
        }
        let ids: Vec<i64> = self.state.transferred_files.keys().cloned().collect();

        for id in ids {
            let mfi = self.state.transferred_files.get_mut(&id).unwrap();

            // Refuse a collision or symlink introduced after the offer was shown.
            let file = File::options()
                .write(true)
                .create_new(true)
                .open(&mfi.file_url)?;
            info!("Created file: {:?}", file);
            mfi.file = Some(file);
        }

        let frame = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::Response.into()),
                connection_response: Some(sharing_nearby::ConnectionResponseFrame {
                    status: Some(sharing_nearby::connection_response_frame::Status::Accept.into()),
                }),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&frame).await?;

        self.update_state(
            |e| {
                e.state = TransferState::ReceivingFiles;
            },
            true,
        )
        .await;

        Ok(())
    }

    async fn reject_transfer(
        &mut self,
        reason: Option<sharing_nearby::connection_response_frame::Status>,
    ) -> Result<(), anyhow::Error> {
        let sreason = if let Some(r) = reason {
            r
        } else {
            sharing_nearby::connection_response_frame::Status::Reject
        };

        let frame = sharing_nearby::Frame {
            version: Some(sharing_nearby::frame::Version::V1.into()),
            v1: Some(sharing_nearby::V1Frame {
                r#type: Some(sharing_nearby::v1_frame::FrameType::Response.into()),
                connection_response: Some(sharing_nearby::ConnectionResponseFrame {
                    status: Some(sreason.into()),
                }),
                ..Default::default()
            }),
        };

        self.send_encrypted_frame(&frame).await?;

        Ok(())
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
                e.decrypt_key = Some(client_key);
                e.recv_hmac_key = Some(client_hmac_key);
                e.encrypt_key = Some(server_key);
                e.send_hmac_key = Some(server_hmac_key);
                e.pin_code = Some(to_four_digit_string(&auth_string));
                e.encryption_done = true;
            },
            false,
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

        trace!("Sending msg into the channel");
        let _ = self.sender.send(ChannelMessage {
            id: self.state.id.clone(),
            msg: channel::Message::Client(MessageClient {
                kind: TransferKind::Inbound,
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

#[cfg(test)]
mod consent_regressions {
    use super::*;

    async fn request() -> (InboundRequest, TcpStream) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let (sender, _) = tokio::sync::broadcast::channel(8);
        let mut request = InboundRequest::new(socket, "test".into(), sender);
        request.state.recv_hmac_key = Some(vec![0x11; 32]);
        request.state.decrypt_key = Some(vec![0x22; 32]);
        request.state.encrypt_key = Some(vec![0x22; 32]);
        request.state.send_hmac_key = Some(vec![0x11; 32]);
        request.state.encryption_done = true;
        (request, peer)
    }
    fn encrypted_payload(
        kind: payload_header::PayloadType,
        declared: i64,
        body: Vec<u8>,
    ) -> SecureMessage {
        encrypted_payload_at(kind, declared, body, 0)
    }
    fn encrypted_payload_at(
        kind: payload_header::PayloadType,
        declared: i64,
        body: Vec<u8>,
        offset: i64,
    ) -> SecureMessage {
        let offline = OfflineFrame {
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(
                    location_nearby_connections::v1_frame::FrameType::PayloadTransfer.into(),
                ),
                payload_transfer: Some(PayloadTransferFrame {
                    payload_header: Some(PayloadHeader {
                        id: Some(42),
                        r#type: Some(kind.into()),
                        total_size: Some(declared),
                        ..Default::default()
                    }),
                    payload_chunk: Some(PayloadChunk {
                        offset: Some(offset),
                        flags: Some(1),
                        body: Some(body),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let plaintext = DeviceToDeviceMessage {
            message: Some(offline.encode_to_vec()),
            sequence_number: Some(1),
        }
        .encode_to_vec();
        let mut cipher = Cipher::new_256(&[0x22; 32]);
        cipher.set_auto_padding(true);
        let envelope = HeaderAndBody {
            header: Header {
                encryption_scheme: EncScheme::Aes256Cbc.into(),
                signature_scheme: SigScheme::HmacSha256.into(),
                iv: Some(vec![0; 16]),
                ..Default::default()
            },
            body: cipher.cbc_encrypt(&[0; 16], &plaintext),
        }
        .encode_to_vec();
        let mut mac = HmacSha256::new_from_slice(&[0x11; 32]).unwrap();
        mac.update(&envelope);
        SecureMessage {
            header_and_body: envelope,
            signature: mac.finalize().into_bytes().to_vec(),
        }
    }
    fn pending_file(request: &mut InboundRequest, path: &std::path::Path) {
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.write_all_at(b"partial", 0).unwrap();
        request.state.transferred_files.insert(
            42,
            InternalFileInfo {
                payload_id: 42,
                file_url: path.to_owned(),
                bytes_transferred: 7,
                total_size: 10,
                file: Some(file),
            },
        );
        request.state.state = TransferState::ReceivingFiles;
    }

    #[tokio::test]
    async fn incomplete_files_are_removed_when_receive_ends() {
        let directory =
            std::env::temp_dir().join(format!("quickshare-cleanup-{}", rand::random::<u64>()));
        std::fs::create_dir(&directory).unwrap();
        for mode in ["cancel", "disconnect", "abort"] {
            let (mut request, mut peer) = request().await;
            let path = directory.join("partial.txt");
            pending_file(&mut request, &path);
            match mode {
                "cancel" => {
                    // A peer that sends a prefix then stalls must not block Cancel.
                    peer.write_all(&[0, 0, 0, 100, 1]).await.unwrap();
                    let sender = request.sender.clone();
                    let id = request.state.id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        sender
                            .send(ChannelMessage {
                                id,
                                msg: channel::Message::Lib {
                                    action: TransferAction::TransferCancel,
                                },
                            })
                            .unwrap();
                    });
                    assert!(
                        tokio::time::timeout(Duration::from_secs(2), request.handle())
                            .await
                            .unwrap()
                            .is_err()
                    );
                    assert_eq!(request.state.state, TransferState::Cancelled);
                    drop(request);
                }
                "disconnect" => {
                    drop(peer);
                    assert!(
                        tokio::time::timeout(Duration::from_secs(2), request.handle())
                            .await
                            .unwrap()
                            .is_err()
                    );
                    drop(request);
                }
                _ => {
                    let (ready, started) = tokio::sync::oneshot::channel();
                    let task = tokio::spawn(async move {
                        let _ = ready.send(());
                        request.handle().await
                    });
                    started.await.unwrap();
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                }
            }
            assert!(!path.exists(), "incomplete receive survived {mode}");
        }
        // A malformed final marker must not turn a partial file into a success.
        let (mut request, _peer) = request().await;
        let path = directory.join("premature.txt");
        pending_file(&mut request, &path);
        assert!(
            request
                .decrypt_and_process_secure_message(&encrypted_payload_at(
                    payload_header::PayloadType::File,
                    10,
                    vec![],
                    7
                ))
                .await
                .unwrap_err()
                .to_string()
                .contains("before its declared size")
        );
        drop(request);
        assert!(!path.exists());
        std::fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn cleanup_preserves_completed_and_replaced_destinations() {
        let directory =
            std::env::temp_dir().join(format!("quickshare-cleanup-{}", rand::random::<u64>()));
        std::fs::create_dir(&directory).unwrap();
        let (mut request, _peer) = request().await;
        let completed = directory.join("complete.txt");
        pending_file(&mut request, &completed);
        request
            .state
            .transferred_files
            .get_mut(&42)
            .unwrap()
            .total_size = 7;
        // Completion removes the file from pending ownership before disconnect.
        let _ = request
            .decrypt_and_process_secure_message(&encrypted_payload_at(
                payload_header::PayloadType::File,
                7,
                vec![],
                7,
            ))
            .await;
        assert_eq!(request.state.state, TransferState::Finished);
        drop(request);
        assert_eq!(std::fs::read(&completed).unwrap(), b"partial");
        for symlink in [false, true] {
            let (mut request, _peer) = self::request().await;
            let path = directory.join("replaced.txt");
            pending_file(&mut request, &path);
            std::fs::remove_file(&path).unwrap();
            if symlink {
                std::os::unix::fs::symlink(&completed, &path).unwrap();
            } else {
                std::fs::write(&path, b"replacement").unwrap();
            }
            drop(request);
            assert!(path.exists());
            assert_eq!(
                std::fs::read(&path).unwrap(),
                if symlink {
                    b"partial".as_slice()
                } else {
                    b"replacement".as_slice()
                }
            );
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn encrypted_text_and_file_data_require_consent() {
        for kind in [
            payload_header::PayloadType::Bytes,
            payload_header::PayloadType::File,
        ] {
            let (mut request, _peer) = request().await;
            request.state.state = TransferState::WaitingForUserConsent;
            request.state.text_payload = Some(TextPayloadInfo::Text(42));
            let result = request
                .decrypt_and_process_secure_message(&encrypted_payload(kind, 1, vec![b'x']))
                .await;
            assert!(result.unwrap_err().to_string().contains("before consent"));
            assert_eq!(request.state.state, TransferState::WaitingForUserConsent);
            assert!(request.state.payload_buffers.is_empty());
        }
    }
    #[tokio::test]
    async fn wifi_credentials_require_consent_and_complete_protobuf_payload() {
        let wifi = super::super::WifiPayload {
            ssid: "Test".into(),
            password: "private-password".into(),
            security_type: SecurityType::WpaPsk,
            hidden_ssid: true,
        };
        let bytes = wifi.encode().unwrap();
        for (state, declared, body, expected) in [
            (
                TransferState::WaitingForUserConsent,
                bytes.len() as i64,
                bytes.clone(),
                "before consent",
            ),
            (
                TransferState::ReceivingFiles,
                bytes.len() as i64 + 1,
                bytes,
                "before its declared size",
            ),
            (
                TransferState::ReceivingFiles,
                3,
                vec![0x0a, 0x01, 0xff],
                "Invalid Wi-Fi credential payload",
            ),
        ] {
            let (mut request, _peer) = request().await;
            request.state.state = state.clone();
            request.state.text_payload = Some(TextPayloadInfo::Wifi((
                42,
                wifi.ssid.clone(),
                wifi.security_type,
            )));
            let error = request
                .decrypt_and_process_secure_message(&encrypted_payload(
                    payload_header::PayloadType::Bytes,
                    declared,
                    body,
                ))
                .await
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            assert_eq!(request.state.state, state);
            assert!(!error.to_string().contains(&wifi.password));
        }
    }

    #[tokio::test]
    async fn negative_sizes_and_oversized_chunks_are_rejected() {
        for (declared, body, expected) in
            [(-1, vec![], "too large"), (1, vec![1, 2], "declared size")]
        {
            let (mut request, _peer) = request().await;
            request.state.state = TransferState::SentConnectionResponse;
            let result = request
                .decrypt_and_process_secure_message(&encrypted_payload(
                    payload_header::PayloadType::Bytes,
                    declared,
                    body,
                ))
                .await;
            assert!(result.unwrap_err().to_string().contains(expected));
        }
    }
    #[tokio::test]
    async fn duplicate_accept_cannot_reopen_files() {
        let (mut request, _peer) = request().await;
        request.state.state = TransferState::ReceivingFiles;
        assert!(
            request
                .accept_transfer()
                .await
                .unwrap_err()
                .to_string()
                .contains("not pending")
        );
    }
    #[tokio::test]
    async fn unsafe_file_metadata_is_rejected_before_destination_creation() {
        for (name, size) in [("../escape", 1), ("/absolute", 1), ("okay.txt", -1)] {
            let (mut request, _peer) = request().await;
            let frame = sharing_nearby::V1Frame {
                introduction: Some(sharing_nearby::IntroductionFrame {
                    file_metadata: vec![sharing_nearby::FileMetadata {
                        name: Some(name.into()),
                        size: Some(size),
                        payload_id: Some(1),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert!(request.process_introduction(&frame).await.is_err());
            assert!(request.state.transferred_files.is_empty());
        }
    }
}
