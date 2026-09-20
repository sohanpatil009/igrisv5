//! HTTP service: device info, pairing-adjacent transfer handshake, uploads.
//! Plain HTTP in Phase 9a (LAN scope); the TLS proxy + cert pinning ride
//! on top in 9b without changing these routes.

use crate::transfer::{FileSpec, SessionManager};
use crate::DeviceInfo;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub info: DeviceInfo,
    pub sessions: Arc<Mutex<SessionManager>>,
    pub chunk_dir: PathBuf,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/ecosystem/v1/info", get(info))
        .route("/api/igris/v1/share/prepare", post(prepare))
        .route("/api/igris/v1/share/confirm", post(confirm))
        .route("/api/igris/v1/share/deny", post(deny))
        .route("/api/igris/v1/share/upload/:session/:file", post(upload))
        .route("/api/igris/v1/share/complete", post(complete))
        .with_state(state)
}

#[derive(Serialize)]
struct InfoReply {
    device_id: String,
    name: String,
    platform: String,
}

async fn info(State(s): State<AppState>) -> impl IntoResponse {
    Json(InfoReply {
        device_id: s.info.id.clone(),
        name: s.info.name.clone(),
        platform: s.info.platform.clone(),
    })
}

#[derive(Deserialize)]
struct PrepareReq {
    from_device: String,
    files: Vec<FileSpec>,
}

async fn prepare(State(s): State<AppState>, Json(req): Json<PrepareReq>) -> impl IntoResponse {
    let mut sessions = s.sessions.lock().unwrap();
    match sessions.prepare(&req.from_device, req.files) {
        Ok((id, tokens)) => (
            StatusCode::OK,
            Json(serde_json::json!({"session_id": id, "tokens": tokens})),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct SessionReq {
    session_id: String,
}

async fn confirm(State(s): State<AppState>, Json(req): Json<SessionReq>) -> impl IntoResponse {
    match s.sessions.lock().unwrap().approve(&req.session_id) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ready"}))).into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn deny(State(s): State<AppState>, Json(req): Json<SessionReq>) -> impl IntoResponse {
    match s.sessions.lock().unwrap().deny(&req.session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "denied"})),
        )
            .into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct TokenQ {
    token: String,
}

async fn upload(
    State(s): State<AppState>,
    Path((session, file)): Path<(String, String)>,
    Query(q): Query<TokenQ>,
    body: Bytes,
) -> impl IntoResponse {
    // Token + state gate first: unauthenticated bytes never touch disk.
    {
        let mut sessions = s.sessions.lock().unwrap();
        if let Err(e) = sessions.note_bytes(&session, &file, &q.token, body.len() as u64) {
            let code = match e {
                crate::transfer::TransferError::BadToken => StatusCode::UNAUTHORIZED,
                crate::transfer::TransferError::UnknownSession
                | crate::transfer::TransferError::UnknownFile(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::CONFLICT,
            };
            return (code, e.to_string()).into_response();
        }
    }
    let dest = s.chunk_dir.join(&session);
    if let Err(e) = tokio::fs::create_dir_all(&dest).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let path = dest.join(format!("{file}.part"));
    let mut opts = tokio::fs::OpenOptions::new();
    match opts.create(true).append(true).open(&path).await {
        Ok(mut f) => {
            use tokio::io::AsyncWriteExt;
            if let Err(e) = f.write_all(&body).await {
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
    let missing = s
        .sessions
        .lock()
        .unwrap()
        .missing_chunks(&session, &file)
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(serde_json::json!({"received": body.len(), "missing_chunks": missing})),
    )
        .into_response()
}

#[derive(Deserialize)]
struct CompleteReq {
    session_id: String,
    file: String,
}

async fn complete(State(s): State<AppState>, Json(req): Json<CompleteReq>) -> impl IntoResponse {
    let path = s
        .chunk_dir
        .join(&req.session_id)
        .join(format!("{}.part", req.file));
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    };
    match s
        .sessions
        .lock()
        .unwrap()
        .complete(&req.session_id, &req.file, &bytes)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "completed"})),
        )
            .into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::Discovery;
    use crate::identity::sha256_hex;
    use crate::{DeviceCaps, ECO_HTTP_PORT};
    use std::net::Ipv4Addr;

    async fn test_server() -> (String, AppState) {
        let info = DeviceInfo {
            id: "server-1".into(),
            name: "test-server".into(),
            platform: "test".into(),
            caps: DeviceCaps {
                cpu_cores: 4,
                ram_mb: 8000,
                has_gpu: false,
                storage_mb: 100000,
                models: vec![],
            },
            trusted: true,
        };
        let state = AppState {
            info,
            sessions: Arc::new(Mutex::new(SessionManager::new())),
            chunk_dir: std::env::temp_dir().join(format!("igris-srv-{}", std::process::id())),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = router(state.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://127.0.0.1:{port}"), state)
    }

    #[tokio::test]
    async fn info_and_discovery_probe() {
        let (base, _) = test_server().await;
        let port: u16 = base.rsplit(':').next().unwrap().parse().unwrap();
        // Raw info route.
        let reply: serde_json::Value = reqwest::get(format!("{base}/api/ecosystem/v1/info"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reply["device_id"], "server-1");
        // Discovery probe against the live server.
        let d = Discovery::new(crate::discovery::DiscoveryConfig {
            port,
            ..Default::default()
        })
        .unwrap();
        let found = d
            .probe(Ipv4Addr::new(127, 0, 0, 1), port)
            .await
            .expect("probe hits test server");
        assert_eq!(found.id, "server-1");
        assert_eq!(ECO_HTTP_PORT, 53327); // contract constant untouched
    }

    #[tokio::test]
    async fn full_transfer_loop() {
        let (base, state) = test_server().await;
        let client = reqwest::Client::new();
        let data = b"field mesh live bytes".to_vec();
        let checksum = sha256_hex(&data);

        // Prepare.
        let prep: serde_json::Value = client
            .post(format!("{base}/api/igris/v1/share/prepare"))
            .json(&serde_json::json!({"from_device": "laptop", "files": [{"name": "live.bin", "size": data.len(), "checksum": checksum}]}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let session = prep["session_id"].as_str().unwrap().to_string();
        let token = prep["tokens"]["live.bin"].as_str().unwrap().to_string();

        // Upload before confirm must fail (wrong state).
        let early = client
            .post(format!(
                "{base}/api/igris/v1/share/upload/{session}/live.bin?token={token}"
            ))
            .body(data.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(early.status(), StatusCode::CONFLICT);

        // Confirm, upload with a bad token (401), then for real.
        client
            .post(format!("{base}/api/igris/v1/share/confirm"))
            .json(&serde_json::json!({"session_id": session}))
            .send()
            .await
            .unwrap();
        let bad = client
            .post(format!(
                "{base}/api/igris/v1/share/upload/{session}/live.bin?token=wrong"
            ))
            .body(data.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);
        let up: serde_json::Value = client
            .post(format!(
                "{base}/api/igris/v1/share/upload/{session}/live.bin?token={token}"
            ))
            .body(data.clone())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(up["missing_chunks"].as_array().unwrap().len(), 0);

        // Complete verifies checksum and closes the session.
        let done: serde_json::Value = client
            .post(format!("{base}/api/igris/v1/share/complete"))
            .json(&serde_json::json!({"session_id": session, "file": "live.bin"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(done["status"], "completed");
        assert_eq!(
            state.sessions.lock().unwrap().state(&session),
            Some(crate::transfer::SessionState::Completed)
        );
        // Bytes on disk match what was sent.
        let on_disk = tokio::fs::read(state.chunk_dir.join(&session).join("live.bin.part"))
            .await
            .unwrap();
        assert_eq!(on_disk, data);
    }

    #[tokio::test]
    async fn denied_session_cannot_upload() {
        let (base, _) = test_server().await;
        let client = reqwest::Client::new();
        let prep: serde_json::Value = client
            .post(format!("{base}/api/igris/v1/share/prepare"))
            .json(&serde_json::json!({"from_device": "laptop", "files": [{"name": "x.bin", "size": 3, "checksum": null}]}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let session = prep["session_id"].as_str().unwrap();
        client
            .post(format!("{base}/api/igris/v1/share/deny"))
            .json(&serde_json::json!({"session_id": session}))
            .send()
            .await
            .unwrap();
        let up = client
            .post(format!(
                "{base}/api/igris/v1/share/upload/{session}/x.bin?token={}",
                prep["tokens"]["x.bin"].as_str().unwrap()
            ))
            .body("abc")
            .send()
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::CONFLICT);
    }
}
