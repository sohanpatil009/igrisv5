//! HTTP service: device info, pairing-adjacent transfer handshake, uploads,
//! and a progress event stream. Plain HTTP behind the 9b TLS proxy on LAN;
//! the same router serves both.

use crate::transfer::{FileSpec, SessionManager};
use crate::DeviceInfo;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Json,
    },
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct AppState {
    pub info: DeviceInfo,
    pub sessions: Arc<Mutex<SessionManager>>,
    pub chunk_dir: PathBuf,
    pub progress_tx: tokio::sync::broadcast::Sender<String>,
}

impl AppState {
    pub fn new(info: DeviceInfo, chunk_dir: PathBuf) -> Self {
        let (progress_tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            info,
            sessions: Arc::new(Mutex::new(SessionManager::new())),
            chunk_dir,
            progress_tx,
        }
    }

    fn emit(&self, payload: serde_json::Value) {
        let _ = self.progress_tx.send(payload.to_string());
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/ecosystem/v1/info", get(info))
        .route("/api/igris/v1/share/prepare", axum::routing::post(prepare))
        .route("/api/igris/v1/share/confirm", axum::routing::post(confirm))
        .route("/api/igris/v1/share/deny", axum::routing::post(deny))
        .route(
            "/api/igris/v1/share/upload/:session/:file",
            axum::routing::post(upload),
        )
        .route(
            "/api/igris/v1/share/complete",
            axum::routing::post(complete),
        )
        .route("/api/igris/v1/share/events/:session", get(session_events))
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
        Ok(()) => {
            s.emit(serde_json::json!({"session": req.session_id, "state": "Transferring"}));
            (StatusCode::OK, Json(serde_json::json!({"status": "ready"}))).into_response()
        }
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn deny(State(s): State<AppState>, Json(req): Json<SessionReq>) -> impl IntoResponse {
    match s.sessions.lock().unwrap().deny(&req.session_id) {
        Ok(()) => {
            s.emit(serde_json::json!({"session": req.session_id, "state": "Denied"}));
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "denied"})),
            )
                .into_response()
        }
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
    s.emit(serde_json::json!({"session": session, "file": file, "received": body.len(), "missing_chunks": missing.len()}));
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
        Ok(()) => {
            s.emit(serde_json::json!({"session": req.session_id, "file": req.file, "state": "Completed"}));
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "completed"})),
            )
                .into_response()
        }
        Err(e) => {
            s.emit(
                serde_json::json!({"session": req.session_id, "file": req.file, "state": "Failed"}),
            );
            (StatusCode::CONFLICT, e.to_string()).into_response()
        }
    }
}

/// Progress stream for one session: chunk/state events as SSE.
/// Lagged receivers drop to the latest event rather than stalling the sender.
async fn session_events(
    State(s): State<AppState>,
    Path(session): Path<String>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    use tokio_stream::{wrappers::BroadcastStream, StreamExt};
    let rx = s.progress_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |msg| {
        let want = session.clone();
        match msg {
            Ok(text) => {
                if text.contains(&want) {
                    Some(Ok(SseEvent::default().data(text)))
                } else {
                    None
                }
            }
            Err(_) => None, // lagged: skip, keep streaming
        }
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
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
        let state = AppState::new(
            info,
            std::env::temp_dir().join(format!("igris-srv-{}", std::process::id())),
        );
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

    #[tokio::test]
    async fn progress_stream_reports_session_events() {
        use tokio_stream::StreamExt;
        let (base, _) = test_server().await;
        let client = reqwest::Client::new();
        let prep: serde_json::Value = client
            .post(format!("{base}/api/igris/v1/share/prepare"))
            .json(&serde_json::json!({"from_device": "laptop", "files": [{"name": "s.bin", "size": 4, "checksum": null}]}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let session = prep["session_id"].as_str().unwrap().to_string();
        let token = prep["tokens"]["s.bin"].as_str().unwrap().to_string();

        // Subscribe first, then drive events through the session.
        let mut stream = client
            .get(format!("{base}/api/igris/v1/share/events/{session}"))
            .send()
            .await
            .unwrap()
            .bytes_stream();
        client
            .post(format!("{base}/api/igris/v1/share/confirm"))
            .json(&serde_json::json!({"session_id": session}))
            .send()
            .await
            .unwrap();
        client
            .post(format!(
                "{base}/api/igris/v1/share/upload/{session}/s.bin?token={token}"
            ))
            .body("data")
            .send()
            .await
            .unwrap();
        // First streamed event belongs to this session (confirm or chunk).
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("event within 5s")
            .expect("stream open")
            .expect("bytes readable");
        let text = String::from_utf8_lossy(&first);
        assert!(
            text.contains(&session),
            "stream carries this session's events: {text}"
        );
    }
}
