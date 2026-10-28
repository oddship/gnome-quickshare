use crate::securegcm::DeviceToDeviceMessage;
use anyhow::{Result, anyhow, ensure};
use hmac::{Hmac, Mac};
use libaes::Cipher;
use p256::PublicKey;
use prost::Message;
use sha2::Sha256;

use crate::securemessage::{
    EncScheme, GenericPublicKey, HeaderAndBody, PublicKeyType, SecureMessage, SigScheme,
};

/// UKEY2 coordinates are positive big-endian two's-complement integers, not
/// fixed-width SEC1 byte strings. Preserve short values and validate sign/width.
fn coordinate(encoded: &[u8]) -> Result<[u8; 32]> {
    ensure!(
        !encoded.is_empty() && encoded.len() <= 33,
        "Invalid P-256 coordinate length"
    );
    ensure!(encoded[0] & 0x80 == 0, "Negative P-256 coordinate");
    let magnitude = if encoded.len() == 33 {
        ensure!(encoded[0] == 0, "P-256 coordinate exceeds 256 bits");
        &encoded[1..]
    } else {
        encoded
    };
    let mut padded = [0; 32];
    padded[32 - magnitude.len()..].copy_from_slice(magnitude);
    Ok(padded)
}

pub(super) fn peer_public_key(raw: GenericPublicKey) -> Result<PublicKey> {
    ensure!(
        raw.r#type == PublicKeyType::EcP256 as i32,
        "Unsupported peer key type"
    );
    let point = raw
        .ec_p256_public_key
        .ok_or_else(|| anyhow!("Missing peer P-256 key"))?;
    let mut encoded = [0; 65];
    encoded[0] = 4;
    encoded[1..33].copy_from_slice(&coordinate(&point.x)?);
    encoded[33..].copy_from_slice(&coordinate(&point.y)?);
    PublicKey::from_sec1_bytes(&encoded).map_err(|_| anyhow!("Peer key is not a valid P-256 point"))
}

/// Authenticate before parsing/decrypting peer-controlled envelope parameters.
pub(super) fn decrypt_message(
    message: &SecureMessage,
    mac_key: &[u8],
    key: &[u8],
) -> Result<DeviceToDeviceMessage> {
    let mut mac = Hmac::<Sha256>::new_from_slice(mac_key)?;
    mac.update(&message.header_and_body);
    mac.verify_slice(&message.signature)
        .map_err(|_| anyhow!("Invalid message authentication code"))?;
    let envelope = HeaderAndBody::decode(message.header_and_body.as_slice())?;
    ensure!(
        envelope.header.signature_scheme == SigScheme::HmacSha256 as i32
            && envelope.header.encryption_scheme == EncScheme::Aes256Cbc as i32,
        "Unsupported secure message schemes"
    );
    ensure!(
        envelope.header.associated_data_length() == 0,
        "Unsupported associated data"
    );
    ensure!(envelope.header.iv().len() == 16, "Invalid AES IV length");
    ensure!(
        !envelope.body.is_empty() && envelope.body.len().is_multiple_of(16),
        "Invalid AES ciphertext length"
    );
    let key: &[u8; 32] = key
        .try_into()
        .map_err(|_| anyhow!("Invalid local decryption key"))?;
    let mut cipher = Cipher::new_256(key);
    // libaes's automatic unpadding only truncates the final byte count. Check
    // all PKCS#7 bytes ourselves before decoding the plaintext protobuf.
    cipher.set_auto_padding(false);
    let mut plaintext = cipher.cbc_decrypt(envelope.header.iv(), &envelope.body);
    let padding = *plaintext
        .last()
        .ok_or_else(|| anyhow!("Empty AES plaintext"))? as usize;
    ensure!(
        (1..=16).contains(&padding) && padding <= plaintext.len(),
        "Invalid AES padding"
    );
    let end = plaintext.len() - padding;
    ensure!(
        plaintext[end..]
            .iter()
            .all(|byte| *byte as usize == padding),
        "Invalid AES padding"
    );
    plaintext.truncate(end);
    Ok(DeviceToDeviceMessage::decode(plaintext.as_slice())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::securemessage::EcP256PublicKey;
    use crate::utils::encode_point;
    use p256::SecretKey;
    use p256::elliptic_curve::sec1::ToEncodedPoint;

    fn wire_key(x: Vec<u8>, y: Vec<u8>) -> GenericPublicKey {
        GenericPublicKey {
            r#type: PublicKeyType::EcP256 as i32,
            ec_p256_public_key: Some(EcP256PublicKey { x, y }),
            ..Default::default()
        }
    }

    #[test]
    fn signed_coordinate_width_and_padding_are_checked() {
        let short = coordinate(&[1]).unwrap();
        assert_eq!(short[31], 1);
        assert!(short[..31].iter().all(|x| *x == 0));
        assert_eq!(coordinate(&[0x7f; 32]).unwrap(), [0x7f; 32]);
        let mut signed = vec![0];
        signed.extend([0x80; 32]);
        assert_eq!(coordinate(&signed).unwrap(), [0x80; 32]);
        for invalid in [vec![], vec![0; 34], vec![0x80], vec![0xff; 32], vec![1; 33]] {
            assert!(coordinate(&invalid).is_err());
        }
    }

    #[test]
    fn malformed_peer_points_and_key_types_return_errors() {
        assert!(peer_public_key(wire_key(vec![0; 32], vec![0; 32])).is_err());
        assert!(peer_public_key(GenericPublicKey::default()).is_err());
        for kind in [
            PublicKeyType::Rsa2048 as i32,
            PublicKeyType::Dh2048Modp as i32,
            999,
        ] {
            let mut raw = wire_key(vec![1], vec![1]);
            raw.r#type = kind;
            assert!(peer_public_key(raw).is_err());
        }
    }

    #[test]
    fn valid_short_coordinate_round_trips() {
        // Scalar 43 has a coordinate whose signed encoding is shorter than
        // 32 bytes; the previous decoder incorrectly rejected this valid key.
        let mut secret = [0; 32];
        secret[31] = 43;
        let key = SecretKey::from_slice(&secret).unwrap().public_key();
        let point = key.to_encoded_point(false);
        let x = encode_point(bytes::Bytes::copy_from_slice(point.x().unwrap())).unwrap();
        let y = encode_point(bytes::Bytes::copy_from_slice(point.y().unwrap())).unwrap();
        assert!(x.len() < 32 || y.len() < 32);
        assert_eq!(peer_public_key(wire_key(x, y)).unwrap(), key);
    }
    fn signed_envelope(header: crate::securemessage::Header, body: Vec<u8>) -> SecureMessage {
        let encoded = HeaderAndBody { header, body }.encode_to_vec();
        let mut mac = Hmac::<Sha256>::new_from_slice(&[0x11; 32]).unwrap();
        mac.update(&encoded);
        SecureMessage {
            header_and_body: encoded,
            signature: mac.finalize().into_bytes().to_vec(),
        }
    }
    fn aes_header() -> crate::securemessage::Header {
        crate::securemessage::Header {
            signature_scheme: SigScheme::HmacSha256 as i32,
            encryption_scheme: EncScheme::Aes256Cbc as i32,
            iv: Some(vec![0; 16]),
            ..Default::default()
        }
    }
    #[test]
    fn malformed_authenticated_envelopes_return_errors() {
        for length in [0, 1, 15, 17, 32] {
            let mut header = aes_header();
            header.iv = Some(vec![0; length]);
            assert!(
                decrypt_message(
                    &signed_envelope(header, vec![0; 16]),
                    &[0x11; 32],
                    &[0x22; 32]
                )
                .unwrap_err()
                .to_string()
                .contains("IV length")
            );
        }
        for length in [0, 1, 15, 17] {
            assert!(
                decrypt_message(
                    &signed_envelope(aes_header(), vec![0; length]),
                    &[0x11; 32],
                    &[0x22; 32]
                )
                .unwrap_err()
                .to_string()
                .contains("ciphertext length")
            );
        }
        let mut header = aes_header();
        header.encryption_scheme = EncScheme::None as i32;
        assert!(
            decrypt_message(
                &signed_envelope(header, vec![0; 16]),
                &[0x11; 32],
                &[0x22; 32]
            )
            .unwrap_err()
            .to_string()
            .contains("schemes")
        );
        let mut message = signed_envelope(aes_header(), vec![0; 16]);
        message.signature[0] ^= 1;
        assert!(
            decrypt_message(&message, &[0x11; 32], &[0x22; 32])
                .unwrap_err()
                .to_string()
                .contains("authentication code")
        );
    }
    #[test]
    fn padding_is_validated_and_valid_messages_decode() {
        let mut cipher = Cipher::new_256(&[0x22; 32]);
        cipher.set_auto_padding(false);
        for padding in [0, 17, 2] {
            let mut invalid = [0; 16];
            invalid[15] = padding;
            let body = cipher.cbc_encrypt(&[0; 16], &invalid);
            assert!(
                decrypt_message(
                    &signed_envelope(aes_header(), body),
                    &[0x11; 32],
                    &[0x22; 32]
                )
                .unwrap_err()
                .to_string()
                .contains("padding")
            );
        }
        cipher.set_auto_padding(true);
        let plaintext = DeviceToDeviceMessage {
            message: Some(b"hello".to_vec()),
            sequence_number: Some(1),
        };
        let body = cipher.cbc_encrypt(&[0; 16], &plaintext.encode_to_vec());
        assert_eq!(
            decrypt_message(
                &signed_envelope(aes_header(), body),
                &[0x11; 32],
                &[0x22; 32]
            )
            .unwrap(),
            plaintext
        );
    }
}
