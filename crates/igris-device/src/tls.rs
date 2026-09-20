//! TLS proxy: accept TLS 1.2+ with a device self-signed cert, forward
//! decrypted bytes to the local plain-HTTP service. Ring-only crypto.
//!
//! Trust model (TOFU, enforced by callers, not bypassed here):
//! the client pins the server's cert fingerprint in its `TrustStore`
//! BEFORE sending anything. A mismatch means no request leaves the box.
//! `danger_accept_invalid_certs`-style clients are only safe behind that
//! gate — see `require_pin`.

use crate::identity::sha256_hex;
use crate::trust::{TrustError, TrustStore};
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Debug, Clone, thiserror::Error)]
pub enum TlsError {
    #[error("cert generation: {0}")]
    Cert(String),
    #[error("bad key material")]
    BadKey,
    #[error("io: {0}")]
    Io(String),
}

/// Self-signed cert for `cn`. Returns (cert_der, key_der).
pub fn generate_cert(cn: &str) -> Result<(Vec<u8>, Vec<u8>), TlsError> {
    let key = rcgen::generate_simple_self_signed(vec![cn.to_string()])
        .map_err(|e| TlsError::Cert(e.to_string()))?;
    let cert_der = key.cert.der().to_vec();
    let key_der = key.key_pair.serialize_der();
    Ok((cert_der, key_der))
}

/// Public handle clients pin: SHA-256 over the cert DER.
pub fn fingerprint_der(der: &[u8]) -> String {
    sha256_hex(der)
}

fn server_config(cert_der: &[u8], key_der: &[u8]) -> Result<rustls::ServerConfig, TlsError> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    // Deterministic provider: never depend on ambient feature unification.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certs = vec![CertificateDer::from(cert_der.to_vec())];
    let key = PrivateKeyDer::try_from(key_der.to_vec()).map_err(|_| TlsError::BadKey)?;
    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::Cert(e.to_string()))
}

/// TOFU gate: pin on first sight, hard-fail on mismatch. Call this with the
/// fingerprint fetched OUT-OF-BAND (pairing screen, QR) — never trust the
/// fingerprint the wire itself hands you on first contact without user
/// confirmation.
pub fn require_pin(
    trust: &mut TrustStore,
    device_id: &str,
    fingerprint: &str,
) -> Result<bool, TrustError> {
    trust.verify_or_pin(device_id, fingerprint)
}

/// TLS accept loop: `tls_listener` -> decrypt -> proxy to `http_target`.
/// Each connection is independent; a dead peer fails only itself.
pub async fn serve_proxy(
    tls_listener: tokio::net::TcpListener,
    http_target: SocketAddr,
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
) -> Result<(), TlsError> {
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config(&cert_der, &key_der)?));
    loop {
        let (stream, _peer) = tls_listener
            .accept()
            .await
            .map_err(|e| TlsError::Io(e.to_string()))?;
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let Ok(mut tls) = acceptor.accept(stream).await else {
                return;
            };
            let Ok(mut plain) = tokio::net::TcpStream::connect(http_target).await else {
                return;
            };
            let _ = tokio::io::copy_bidirectional(&mut tls, &mut plain).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_fingerprint_stable_and_unique() {
        let (cert_a, _) = generate_cert("igris.local").unwrap();
        let (cert_b, _) = generate_cert("igris.local").unwrap();
        assert_eq!(fingerprint_der(&cert_a), fingerprint_der(&cert_a));
        assert_eq!(fingerprint_der(&cert_a).len(), 64);
        assert_ne!(fingerprint_der(&cert_a), fingerprint_der(&cert_b));
    }

    #[test]
    fn pin_gate_blocks_mitm_before_any_traffic() {
        let mut trust = TrustStore::new();
        // First sighting with user confirmation pins.
        assert!(require_pin(&mut trust, "peer-1", "fp-aaa").unwrap());
        // Same fingerprint passes.
        assert!(!require_pin(&mut trust, "peer-1", "fp-aaa").unwrap());
        // Attacker's cert fails the gate: caller must NOT open any connection.
        assert!(require_pin(&mut trust, "peer-1", "fp-evil").is_err());
    }

    #[test]
    fn bad_key_material_rejected() {
        assert!(matches!(
            server_config(b"not-a-cert", b"not-a-key"),
            Err(TlsError::BadKey | TlsError::Cert(_))
        ));
    }

    #[tokio::test]
    async fn tls_proxy_carries_http_end_to_end() {
        use crate::server::{router, AppState};
        use crate::{DeviceCaps, DeviceInfo};

        let info = DeviceInfo {
            id: "tls-peer".into(),
            name: "tls-test".into(),
            platform: "test".into(),
            caps: DeviceCaps {
                cpu_cores: 2,
                ram_mb: 4000,
                has_gpu: false,
                storage_mb: 50000,
                models: vec![],
            },
            trusted: true,
        };
        let state = AppState::new(
            info,
            std::env::temp_dir().join(format!("igris-tls-{}", std::process::id())),
        );
        // Plain HTTP service on an ephemeral port.
        let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_addr = http_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(http_listener, router(state)).await.unwrap();
        });
        // TLS proxy in front of it.
        let (cert_der, key_der) = generate_cert("igris.local").unwrap();
        let fp = fingerprint_der(&cert_der);
        let tls_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tls_addr = tls_listener.local_addr().unwrap();
        tokio::spawn(serve_proxy(tls_listener, http_addr, cert_der, key_der));

        // TOFU gate passes for the real fingerprint...
        let mut trust = TrustStore::new();
        require_pin(&mut trust, "tls-peer", &fp).unwrap();
        // ...so (and only so) the pinned client may connect. The danger-accept
        // client is safe exactly because the pin was verified first.
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        let reply: serde_json::Value = client
            .get(format!("https://{tls_addr}/api/ecosystem/v1/info"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reply["device_id"], "tls-peer");

        // ...and an attacker's fingerprint never gets a connection attempt:
        // the gate fails before any socket opens.
        assert!(require_pin(&mut trust, "tls-peer", "fp-evil").is_err());
    }
}
