//! TOFU trust store: pin a fingerprint on first pairing, verify after.
//! A changed fingerprint for a known device is a hard error (possible MITM),
//! never an auto-update. Export/import moves trust to a new install.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, thiserror::Error)]
pub enum TrustError {
    #[error("fingerprint mismatch for {device}: possible MITM, re-pair required")]
    Mismatch { device: String },
    #[error("unknown device: {0}")]
    Unknown(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    pins: HashMap<String, String>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// First sighting pins; later sightings must match.
    /// Returns true when newly pinned, false when matched.
    pub fn verify_or_pin(
        &mut self,
        device_id: &str,
        fingerprint: &str,
    ) -> Result<bool, TrustError> {
        match self.pins.get(device_id) {
            None => {
                self.pins.insert(device_id.into(), fingerprint.into());
                Ok(true)
            }
            Some(pinned) if pinned == fingerprint => Ok(false),
            Some(_) => Err(TrustError::Mismatch {
                device: device_id.into(),
            }),
        }
    }

    pub fn is_trusted(&self, device_id: &str, fingerprint: &str) -> bool {
        self.pins
            .get(device_id)
            .map(|p| p == fingerprint)
            .unwrap_or(false)
    }

    pub fn revoke(&mut self, device_id: &str) -> bool {
        self.pins.remove(device_id).is_some()
    }

    pub fn trusted_count(&self) -> usize {
        self.pins.len()
    }

    pub fn export_json(&self) -> String {
        serde_json::to_string(&self.pins).unwrap_or_else(|_| "{}".into())
    }

    pub fn import_json(&mut self, json: &str) -> Result<usize, TrustError> {
        let pins: HashMap<String, String> = serde_json::from_str(json)
            .map_err(|_| TrustError::Unknown("bad trust export".into()))?;
        let n = pins.len();
        self.pins = pins;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_then_verify() {
        let mut t = TrustStore::new();
        assert!(t.verify_or_pin("d1", "fp1").unwrap());
        assert!(!t.verify_or_pin("d1", "fp1").unwrap());
        assert!(t.is_trusted("d1", "fp1"));
        assert!(!t.is_trusted("d1", "other"));
    }

    #[test]
    fn mismatch_is_hard_error() {
        let mut t = TrustStore::new();
        t.verify_or_pin("d1", "fp1").unwrap();
        let err = t.verify_or_pin("d1", "fp2").unwrap_err();
        assert!(matches!(err, TrustError::Mismatch { .. }));
        // The old pin survives the attack attempt.
        assert!(t.is_trusted("d1", "fp1"));
    }

    #[test]
    fn revoke_and_export_roundtrip() {
        let mut t = TrustStore::new();
        t.verify_or_pin("d1", "fp1").unwrap();
        assert!(t.revoke("d1"));
        assert!(!t.revoke("d1"));
        t.verify_or_pin("d2", "fp2").unwrap();
        let json = t.export_json();
        let mut t2 = TrustStore::new();
        assert_eq!(t2.import_json(&json).unwrap(), 1);
        assert!(t2.is_trusted("d2", "fp2"));
    }
}
