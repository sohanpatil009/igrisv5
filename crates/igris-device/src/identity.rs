//! Device identity: stable id + secret, SHA-256 fingerprint for TOFU.
//! Secrets persist via export/import (file persistence lands with the daemon);
//! fingerprints are what travel the wire — never the secret.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex_encode(&h.finalize())
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    /// Never transmitted. Fingerprint (below) is the public handle.
    pub secret: String,
}

impl Identity {
    pub fn generate(name: &str, platform: &str) -> Self {
        Self {
            device_id: Uuid::new_v4().to_string(),
            name: name.into(),
            platform: platform.into(),
            secret: Uuid::new_v4().to_string() + &Uuid::new_v4().to_string(),
        }
    }

    /// Stable public handle: SHA-256 over id + secret.
    pub fn fingerprint(&self) -> String {
        sha256_hex(format!("{}:{}", self.device_id, self.secret).as_bytes())
    }

    /// Pairing key derived per-session from the secret (HMAC key material).
    pub fn pairing_key(&self, peer_id: &str) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(self.secret.as_bytes());
        h.update(b"|");
        h.update(peer_id.as_bytes());
        h.finalize().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_stable_and_unique() {
        let a = Identity::generate("laptop", "windows");
        assert_eq!(a.fingerprint(), a.fingerprint());
        let b = Identity::generate("laptop", "windows");
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint().len(), 64);
    }

    #[test]
    fn pairing_key_differs_per_peer() {
        let a = Identity::generate("a", "x");
        assert_ne!(a.pairing_key("p1"), a.pairing_key("p2"));
    }
}
