//! On-disk device state: identity + trust pins as JSON under a data dir.
//! Atomic writes (temp + rename) so a crash never leaves half a file.
//! Secrets in identity.json are file-mode protected by OS convention;
//! keyring migration is tracked hardening work, not ignored.

use crate::identity::Identity;
use crate::trust::TrustStore;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(String),
    #[error("corrupt file {file}: {reason}")]
    Corrupt { file: String, reason: String },
}

#[derive(Debug, Clone)]
pub struct DeviceStore {
    dir: PathBuf,
}

impl DeviceStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Default location: `<exe-dir>/pkg/ecosystem`, falling back to cwd.
    pub fn default_dir() -> PathBuf {
        let base = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("pkg").join("ecosystem")
    }

    fn write_atomic(&self, name: &str, bytes: &[u8]) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| StoreError::Io(e.to_string()))?;
        let dest = self.dir.join(name);
        let tmp = self.dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, bytes).map_err(|e| StoreError::Io(e.to_string()))?;
        std::fs::rename(&tmp, &dest).map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let dest = self.dir.join(name);
        match std::fs::read(&dest) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StoreError::Io(e.to_string())),
        }
    }

    pub fn save_identity(&self, identity: &Identity) -> Result<(), StoreError> {
        let json = serde_json::to_string_pretty(identity).map_err(|e| StoreError::Corrupt {
            file: "identity.json".into(),
            reason: e.to_string(),
        })?;
        self.write_atomic("identity.json", json.as_bytes())
    }

    pub fn load_identity(&self) -> Result<Option<Identity>, StoreError> {
        match self.read("identity.json")? {
            None => Ok(None),
            Some(b) => serde_json::from_slice(&b)
                .map(Some)
                .map_err(|e| StoreError::Corrupt {
                    file: "identity.json".into(),
                    reason: e.to_string(),
                }),
        }
    }

    /// Load-or-create: stable identity across restarts, fresh on first boot.
    pub fn identity_or_create(
        &self,
        name: &str,
        platform: &str,
    ) -> Result<(Identity, bool), StoreError> {
        if let Some(id) = self.load_identity()? {
            return Ok((id, false));
        }
        let id = Identity::generate(name, platform);
        self.save_identity(&id)?;
        Ok((id, true))
    }

    pub fn save_trust(&self, trust: &TrustStore) -> Result<(), StoreError> {
        self.write_atomic("trusted_devices.json", trust.export_json().as_bytes())
    }

    pub fn load_trust(&self) -> Result<TrustStore, StoreError> {
        let mut trust = TrustStore::new();
        if let Some(b) = self.read("trusted_devices.json")? {
            let s = String::from_utf8(b).map_err(|e| StoreError::Corrupt {
                file: "trusted_devices.json".into(),
                reason: e.to_string(),
            })?;
            trust.import_json(&s).map_err(|e| StoreError::Corrupt {
                file: "trusted_devices.json".into(),
                reason: e.to_string(),
            })?;
        }
        Ok(trust)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> DeviceStore {
        DeviceStore::new(std::env::temp_dir().join(format!(
            "igris-store-{}-{}",
            std::process::id(),
            rand_suffix()
        )))
    }

    fn rand_suffix() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos().to_string())
            .unwrap_or_else(|_| "0".into())
    }

    #[test]
    fn identity_stable_across_restarts() {
        let s = tmp_store();
        let (first, created) = s.identity_or_create("laptop", "windows").unwrap();
        assert!(created);
        let (second, created_again) = s.identity_or_create("other-name", "linux").unwrap();
        assert!(!created_again, "stored identity wins over new args");
        assert_eq!(first.device_id, second.device_id);
        assert_eq!(first.fingerprint(), second.fingerprint());
    }

    #[test]
    fn trust_pins_survive_restart() {
        let s = tmp_store();
        let mut t = s.load_trust().unwrap();
        assert_eq!(t.trusted_count(), 0);
        t.verify_or_pin("peer-1", "fp-aaa").unwrap();
        s.save_trust(&t).unwrap();
        let reloaded = s.load_trust().unwrap();
        assert!(reloaded.is_trusted("peer-1", "fp-aaa"));
    }

    #[test]
    fn corrupt_files_reported_not_panicked() {
        let s = tmp_store();
        std::fs::create_dir_all(s.dir()).unwrap();
        std::fs::write(s.dir().join("identity.json"), b"{oops").unwrap();
        assert!(matches!(s.load_identity(), Err(StoreError::Corrupt { .. })));
    }
}
