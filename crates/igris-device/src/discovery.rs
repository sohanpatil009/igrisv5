//! LAN discovery: enumerate local /24s, probe device ports concurrently.
//! Probes are plain HTTP GETs against `/api/ecosystem/v1/info` with a short
//! timeout; responders are merged into a registry with last-seen timestamps
//! and pruned after the device timeout. No mDNS yet — scan only.

use crate::{DEVICE_TIMEOUT_SECS, ECO_HTTP_PORT};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
pub const MAX_CONCURRENT_PROBES: usize = 32;

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub ip: String,
    pub port: u16,
    pub last_seen: Instant,
}

#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub port: u16,
    pub timeout: Duration,
    pub max_concurrent: usize,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            port: ECO_HTTP_PORT,
            timeout: PROBE_TIMEOUT,
            max_concurrent: MAX_CONCURRENT_PROBES,
        }
    }
}

#[derive(Debug, Default)]
pub struct Discovery {
    config: DiscoveryConfig,
    client: reqwest::Client,
    devices: HashMap<String, DiscoveredDevice>,
}

impl Discovery {
    pub fn new(config: DiscoveryConfig) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self {
            config,
            client,
            devices: HashMap::new(),
        })
    }

    /// All local IPv4 interface addresses (self is skipped during scans).
    pub fn local_ipv4s() -> Vec<Ipv4Addr> {
        local_ip_address::list_afinet_netifas()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, ip)| match ip {
                std::net::IpAddr::V4(v4) => Some(v4),
                _ => None,
            })
            .collect()
    }

    /// /24 host addresses for an interface IP, excluding network/broadcast/self.
    pub fn subnet_targets(ip: Ipv4Addr) -> Vec<Ipv4Addr> {
        let o = ip.octets();
        (1u16..=254)
            .filter_map(|last| {
                let cand = Ipv4Addr::new(o[0], o[1], o[2], last as u8);
                (cand != ip).then_some(cand)
            })
            .collect()
    }

    fn info_url(ip: &Ipv4Addr, port: u16) -> String {
        format!("http://{ip}:{port}/api/ecosystem/v1/info")
    }

    /// Probe one host:port. Returns the device on success, None on any failure.
    pub async fn probe(&self, ip: Ipv4Addr, port: u16) -> Option<DiscoveredDevice> {
        #[derive(Deserialize)]
        struct InfoReply {
            #[serde(default)]
            device_id: String,
            #[serde(default)]
            name: String,
            #[serde(default)]
            platform: String,
        }
        let reply: InfoReply = self
            .client
            .get(Self::info_url(&ip, port))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        if reply.device_id.is_empty() {
            return None;
        }
        Some(DiscoveredDevice {
            id: reply.device_id,
            name: reply.name,
            platform: reply.platform,
            ip: ip.to_string(),
            port,
            last_seen: Instant::now(),
        })
    }

    /// Full sweep over every local /24. Bounded concurrency via semaphore.
    /// Returns the currently known devices after pruning stale entries.
    pub async fn scan(&mut self) -> Vec<DiscoveredDevice> {
        use tokio::sync::Semaphore;
        let sem = std::sync::Arc::new(Semaphore::new(self.config.max_concurrent));
        let mut targets = Vec::new();
        for ip in Self::local_ipv4s() {
            // Skip loopback / link-local / self-only scans of odd adapters.
            if ip.is_loopback() || ip.is_link_local() {
                continue;
            }
            targets.extend(Self::subnet_targets(ip));
        }
        let mut set = tokio::task::JoinSet::new();
        for ip in targets {
            let permit = sem.clone().acquire_owned().await;
            let client = self.client.clone();
            let port = self.config.port;
            let timeout = self.config.timeout;
            set.spawn(async move {
                let _permit = permit;
                #[derive(Deserialize)]
                struct InfoReply {
                    #[serde(default)]
                    device_id: String,
                    #[serde(default)]
                    name: String,
                    #[serde(default)]
                    platform: String,
                }
                let url = Self::info_url(&ip, port);
                let reply: Option<InfoReply> = client
                    .get(&url)
                    .timeout(timeout)
                    .send()
                    .await
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                let reply = reply?;
                if reply.device_id.is_empty() {
                    return None;
                }
                Some(DiscoveredDevice {
                    id: reply.device_id,
                    name: reply.name,
                    platform: reply.platform,
                    ip: ip.to_string(),
                    port,
                    last_seen: Instant::now(),
                })
            });
        }
        while let Some(res) = set.join_next().await {
            if let Ok(Some(d)) = res {
                self.devices.insert(d.id.clone(), d);
            }
        }
        self.prune()
    }

    pub fn merge(&mut self, device: DiscoveredDevice) {
        self.devices.insert(device.id.clone(), device);
    }

    /// Bulk merge (e.g. a throwaway sweep's results). Returns live count.
    pub fn merge_many(&mut self, fresh: Vec<DiscoveredDevice>) -> usize {
        for d in fresh {
            self.devices.insert(d.id.clone(), d);
        }
        self.prune().len()
    }

    /// Drop entries silent longer than the device timeout. Returns survivors.
    pub fn prune(&mut self) -> Vec<DiscoveredDevice> {
        let cutoff = Duration::from_secs(DEVICE_TIMEOUT_SECS);
        self.devices.retain(|_, d| d.last_seen.elapsed() < cutoff);
        let mut out: Vec<DiscoveredDevice> = self.devices.values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn known(&self) -> Vec<DiscoveredDevice> {
        let mut out: Vec<DiscoveredDevice> = self.devices.values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_targets_skip_self_and_reserved() {
        let targets = Discovery::subnet_targets(Ipv4Addr::new(192, 168, 1, 10));
        assert_eq!(targets.len(), 253); // 254 hosts minus self
        assert!(!targets.contains(&Ipv4Addr::new(192, 168, 1, 10)));
        assert!(!targets.contains(&Ipv4Addr::new(192, 168, 1, 0)));
        assert!(!targets.contains(&Ipv4Addr::new(192, 168, 1, 255)));
    }

    #[test]
    fn prune_drops_stale() {
        let mut d = Discovery {
            config: DiscoveryConfig::default(),
            client: reqwest::Client::new(),
            devices: HashMap::new(),
        };
        d.merge(DiscoveredDevice {
            id: "old".into(),
            name: "o".into(),
            platform: "x".into(),
            ip: "1.2.3.4".into(),
            port: 1,
            last_seen: Instant::now() - Duration::from_secs(DEVICE_TIMEOUT_SECS + 1),
        });
        d.merge(DiscoveredDevice {
            id: "new".into(),
            name: "n".into(),
            platform: "x".into(),
            ip: "1.2.3.5".into(),
            port: 1,
            last_seen: Instant::now(),
        });
        let kept = d.prune();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "new");
    }

    #[test]
    fn merge_many_bulk_loads() {
        let mut d = Discovery {
            config: DiscoveryConfig::default(),
            client: reqwest::Client::new(),
            devices: HashMap::new(),
        };
        let fresh = vec![
            DiscoveredDevice {
                id: "a".into(),
                name: "a".into(),
                platform: "x".into(),
                ip: "1.1.1.1".into(),
                port: 1,
                last_seen: Instant::now(),
            },
            DiscoveredDevice {
                id: "b".into(),
                name: "b".into(),
                platform: "x".into(),
                ip: "1.1.1.2".into(),
                port: 1,
                last_seen: Instant::now(),
            },
        ];
        assert_eq!(d.merge_many(fresh), 2);
        assert_eq!(d.known().len(), 2);
    }
}
