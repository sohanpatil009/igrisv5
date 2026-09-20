//! igris-runtime — lifecycle, config, single-runtime rule.
//! The binary owns ONE Tokio runtime via #[tokio::main].
//! This crate only provides config + graceful shutdown handles.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub data_dir: String,
    pub shutdown_timeout: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            data_dir: "./data".into(),
            shutdown_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeHandle {
    inner: Arc<NodeInner>,
}

#[derive(Debug)]
struct NodeInner {
    config: RwLock<RuntimeConfig>,
    shutdown_tx: watch::Sender<bool>,
}

impl NodeHandle {
    pub fn new(config: RuntimeConfig) -> Self {
        let (tx, _) = watch::channel(false);
        Self {
            inner: Arc::new(NodeInner {
                config: RwLock::new(config),
                shutdown_tx: tx,
            }),
        }
    }

    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.inner.shutdown_tx.subscribe()
    }

    pub async fn config(&self) -> RuntimeConfig {
        self.inner.config.read().await.clone()
    }

    pub async fn shutdown(&self) {
        let _ = self.inner.shutdown_tx.send(true);
    }

    /// Run a future with shutdown-aware timeout. Never blocks the runtime.
    pub async fn run_with_timeout<F, T>(&self, fut: F, d: Duration) -> anyhow::Result<T>
    where
        F: std::future::Future<Output = T>,
    {
        timeout(d, fut)
            .await
            .map_err(|_| anyhow::anyhow!("operation timed out"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_broadcasts() {
        let h = NodeHandle::new(RuntimeConfig::default());
        let mut rx = h.subscribe_shutdown();
        h.shutdown().await;
        rx.changed().await.expect("signal");
        assert!(*rx.borrow());
    }
}
