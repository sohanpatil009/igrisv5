//! Signed message envelope: versioned, timestamped, HMAC-SHA256.
//! Receivers reject bad signatures, stale timestamps (>5min skew), and
//! version mismatches before touching the payload.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub const PROTOCOL_VERSION: &str = "1.0.0";
pub const MAX_SKEW_SECS: i64 = 300;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    Announcement,
    ClipboardSync,
    PairRequest,
    Heartbeat,
    TaskDispatch,
    TaskResult,
    FileChunkOffer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcoMessage {
    pub version: String,
    pub msg_type: MessageType,
    pub sender_id: String,
    pub sender_name: String,
    pub payload: Vec<u8>,
    pub timestamp_ms: i64,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum MessageError {
    #[error("bad signature")]
    BadSignature,
    #[error("stale timestamp")]
    Stale,
    #[error("version mismatch: {0}")]
    Version(String),
}

impl EcoMessage {
    pub fn new(
        msg_type: MessageType,
        sender_id: &str,
        sender_name: &str,
        payload: Vec<u8>,
        now_ms: i64,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION.into(),
            msg_type,
            sender_id: sender_id.into(),
            sender_name: sender_name.into(),
            payload,
            timestamp_ms: now_ms,
            signature: None,
        }
    }

    fn mac_input(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(self.sender_id.as_bytes());
        v.push(0);
        v.extend_from_slice(self.timestamp_ms.to_le_bytes().as_slice());
        v.push(0);
        v.extend_from_slice(&self.payload);
        v
    }

    pub fn sign(&mut self, key: &[u8]) {
        let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
        mac.update(&self.mac_input());
        self.signature = Some(crate::identity::hex_encode(&mac.finalize().into_bytes()));
    }

    pub fn verify(&self, key: &[u8], now_ms: i64) -> Result<(), MessageError> {
        if self.version != PROTOCOL_VERSION {
            return Err(MessageError::Version(self.version.clone()));
        }
        if (now_ms - self.timestamp_ms).abs() > MAX_SKEW_SECS * 1000 {
            return Err(MessageError::Stale);
        }
        let Some(sig) = &self.signature else {
            return Err(MessageError::BadSignature);
        };
        let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
        mac.update(&self.mac_input());
        let expect = crate::identity::hex_encode(&mac.finalize().into_bytes());
        if &expect == sig {
            Ok(())
        } else {
            Err(MessageError::BadSignature)
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let mut m = EcoMessage::new(
            MessageType::Heartbeat,
            "d1",
            "laptop",
            b"ping".to_vec(),
            1_000_000,
        );
        m.sign(b"key-1");
        assert!(m.verify(b"key-1", 1_000_000).is_ok());
    }

    #[test]
    fn tamper_and_wrong_key_fail() {
        let mut m = EcoMessage::new(
            MessageType::Heartbeat,
            "d1",
            "laptop",
            b"ping".to_vec(),
            1_000_000,
        );
        m.sign(b"key-1");
        assert!(matches!(
            m.verify(b"key-2", 1_000_000),
            Err(MessageError::BadSignature)
        ));
        let mut tampered = m.clone();
        tampered.payload = b"pong".to_vec();
        assert!(matches!(
            tampered.verify(b"key-1", 1_000_000),
            Err(MessageError::BadSignature)
        ));
        let unsigned = EcoMessage::new(
            MessageType::Heartbeat,
            "d1",
            "laptop",
            b"ping".to_vec(),
            1_000_000,
        );
        assert!(matches!(
            unsigned.verify(b"key-1", 1_000_000),
            Err(MessageError::BadSignature)
        ));
    }

    #[test]
    fn stale_and_version_rejected() {
        let mut m = EcoMessage::new(
            MessageType::Heartbeat,
            "d1",
            "laptop",
            b"ping".to_vec(),
            1_000_000,
        );
        m.sign(b"k");
        assert!(matches!(
            m.verify(b"k", 1_000_000 + 301_000),
            Err(MessageError::Stale)
        ));
        m.version = "9.9.9".into();
        assert!(matches!(
            m.verify(b"k", 1_000_000),
            Err(MessageError::Version(_))
        ));
    }
}
