//! igris-device — mesh identity, trust, pairing, envelopes, transfers.
//!
//! Phase 9a: clean-room protocol core + real HTTP discovery/service.
//! Ports kept as UX contract: 53317/53318 FastSwap, 53327/53328 Eco.
//! TLS proxy + cert pinning ride on top in 9b without route changes.

pub mod discovery;
pub mod identity;
pub mod message;
pub mod pairing;
pub mod server;
pub mod transfer;
pub mod trust;

use serde::{Deserialize, Serialize};

pub const FASTSWAP_HTTP_PORT: u16 = 53317;
pub const FASTSWAP_TLS_PORT: u16 = 53318;
pub const ECO_HTTP_PORT: u16 = 53327;
pub const ECO_TLS_PORT: u16 = 53328;
pub const HEARTBEAT_SECS: u64 = 30;
pub const DEVICE_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCaps {
    pub cpu_cores: u32,
    pub ram_mb: u64,
    pub has_gpu: bool,
    pub storage_mb: u64,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub caps: DeviceCaps,
    pub trusted: bool,
}

impl DeviceInfo {
    pub fn compute_score(&self) -> u64 {
        self.caps.cpu_cores as u64 * 10
            + self.caps.ram_mb / 512
            + if self.caps.has_gpu { 50 } else { 0 }
    }

    /// Public announcement payload for discovery replies.
    pub fn announcement(&self) -> serde_json::Value {
        serde_json::json!({
            "device_id": self.id,
            "name": self.name,
            "platform": self.platform,
            "score": self.compute_score(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_scores_higher() {
        let base = DeviceCaps {
            cpu_cores: 8,
            ram_mb: 16000,
            has_gpu: false,
            storage_mb: 512000,
            models: vec![],
        };
        let a = DeviceInfo {
            id: "a".into(),
            name: "laptop".into(),
            platform: "windows".into(),
            caps: base.clone(),
            trusted: true,
        };
        let b = DeviceInfo {
            id: "b".into(),
            name: "server".into(),
            platform: "linux".into(),
            caps: DeviceCaps {
                has_gpu: true,
                ..base
            },
            trusted: true,
        };
        assert!(b.compute_score() > a.compute_score());
    }
}
