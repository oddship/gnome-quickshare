use anyhow::{bail, ensure};
use prost::Message;

use crate::sharing_nearby::{WifiCredentials, wifi_credentials_metadata::SecurityType};

#[derive(Clone)]
pub struct WifiPayload {
    pub ssid: String,
    pub password: String,
    pub security_type: SecurityType,
    pub hidden_ssid: bool,
}

impl std::fmt::Debug for WifiPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WifiPayload")
            .field("ssid", &self.ssid)
            .field("password", &"[redacted]")
            .field("security_type", &self.security_type)
            .field("hidden_ssid", &self.hidden_ssid)
            .finish()
    }
}

impl WifiPayload {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            !self.ssid.is_empty() && self.ssid.len() <= 32,
            "Wi-Fi network names must contain 1 to 32 bytes"
        );
        match self.security_type {
            SecurityType::Open => ensure!(
                self.password.is_empty(),
                "Open networks cannot have a password"
            ),
            SecurityType::WpaPsk => ensure!(
                (8..=63).contains(&self.password.len())
                    || (self.password.len() == 64
                        && self.password.bytes().all(|b| b.is_ascii_hexdigit())),
                "WPA passwords must contain 8 to 63 bytes, or 64 hexadecimal digits"
            ),
            SecurityType::Wep => ensure!(
                ((self.password.len() == 5 || self.password.len() == 13)
                    && self.password.is_ascii())
                    || ((self.password.len() == 10 || self.password.len() == 26)
                        && self.password.bytes().all(|b| b.is_ascii_hexdigit())),
                "WEP keys must contain 5 or 13 ASCII characters, or 10 or 26 hexadecimal digits"
            ),
            SecurityType::UnknownSecurityType => bail!("Unsupported Wi-Fi security type"),
        }
        Ok(())
    }

    pub(crate) fn encode(&self) -> anyhow::Result<Vec<u8>> {
        self.validate()?;
        Ok(WifiCredentials {
            password: Some(self.password.clone()),
            hidden_ssid: Some(self.hidden_ssid),
        }
        .encode_to_vec())
    }

    pub(crate) fn decode(
        ssid: String,
        security_type: SecurityType,
        bytes: &[u8],
    ) -> anyhow::Result<Self> {
        // Never put credential bytes or protobuf error context into diagnostics.
        let credentials = WifiCredentials::decode(bytes)
            .map_err(|_| anyhow::anyhow!("Invalid Wi-Fi credential payload"))?;
        let payload = Self {
            ssid,
            password: credentials.password.unwrap_or_default(),
            security_type,
            hidden_ssid: credentials.hidden_ssid.unwrap_or_default(),
        };
        payload.validate()?;
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_preserve_hidden_networks_and_reject_invalid_wire_data() {
        let wifi = WifiPayload {
            ssid: "नमस्ते".into(),
            password: "private-password".into(),
            security_type: SecurityType::WpaPsk,
            hidden_ssid: true,
        };
        let bytes = wifi.encode().unwrap();
        let received = WifiPayload::decode(wifi.ssid.clone(), wifi.security_type, &bytes).unwrap();
        assert_eq!(received.password, wifi.password);
        assert!(received.hidden_ssid);
        assert!(!format!("{wifi:?}").contains(&wifi.password));
        for bytes in [
            &[0x0a, 0x80][..],
            &[0x0a, 0x03, 0xff, 0xff, 0xff][..],
            &[0x0a, 0x08, b'a'][..],
        ] {
            let error =
                WifiPayload::decode("Test".into(), SecurityType::WpaPsk, bytes).unwrap_err();
            assert_eq!(error.to_string(), "Invalid Wi-Fi credential payload");
        }
    }

    #[test]
    fn security_types_validate_without_disclosing_passwords() {
        let mut wifi = WifiPayload {
            ssid: "Test".into(),
            password: String::new(),
            security_type: SecurityType::Open,
            hidden_ssid: false,
        };
        assert!(wifi.encode().is_ok());
        wifi.security_type = SecurityType::WpaPsk;
        assert!(wifi.validate().is_err());
        wifi.password = "a".repeat(64);
        assert!(wifi.validate().is_ok());
        wifi.password = "z".repeat(64);
        assert!(wifi.validate().is_err());
        wifi.security_type = SecurityType::Wep;
        wifi.password = "abcde".into();
        assert!(wifi.validate().is_ok());
        wifi.password = "01020304ff".into();
        assert!(wifi.validate().is_ok());
        wifi.ssid = "a".repeat(33);
        assert!(wifi.validate().is_err());
    }
}
