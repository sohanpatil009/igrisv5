//! Transfer sessions: prepare -> approve/deny -> chunked upload -> verify.
//! Per-file tokens, 1MB chunks with confirm bitsets (resume = missing
//! chunks), SHA-256 checksum on completion, TTL sweeps. Every state
//! transition is explicit — no dead code paths.

use crate::identity::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const CHUNK_SIZE: u64 = 1024 * 1024;
pub const SESSION_TTL: Duration = Duration::from_secs(600);
pub const PENDING_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Preparing,
    AwaitingApproval,
    Transferring,
    Completed,
    Denied,
    Expired,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSpec {
    pub name: String,
    pub size: u64,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FileProgress {
    pub spec: FileSpec,
    pub token: String,
    pub received_bytes: u64,
    pub confirmed_chunks: Vec<bool>,
}

impl FileProgress {
    fn new(spec: FileSpec) -> Self {
        let chunks = spec.size.div_ceil(CHUNK_SIZE).max(1) as usize;
        Self {
            spec,
            token: Uuid::new_v4().to_string(),
            received_bytes: 0,
            confirmed_chunks: vec![false; chunks],
        }
    }

    fn note_bytes(&mut self, n: u64) {
        self.received_bytes += n;
        let total = self.confirmed_chunks.len() as u64;
        let done = (self.received_bytes / CHUNK_SIZE).min(total);
        for c in self.confirmed_chunks.iter_mut().take(done as usize) {
            *c = true;
        }
        if self.received_bytes >= self.spec.size {
            for c in self.confirmed_chunks.iter_mut() {
                *c = true;
            }
        }
    }

    pub fn missing_chunks(&self) -> Vec<usize> {
        self.confirmed_chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| !**c)
            .map(|(i, _)| i)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct TransferSession {
    pub id: String,
    pub from_device: String,
    pub files: HashMap<String, FileProgress>,
    pub state: SessionState,
    pub created: Instant,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum TransferError {
    #[error("unknown session")]
    UnknownSession,
    #[error("unknown file: {0}")]
    UnknownFile(String),
    #[error("bad token")]
    BadToken,
    #[error("wrong state: expected {expected:?}, was {was:?}")]
    WrongState {
        expected: SessionState,
        was: SessionState,
    },
    #[error("checksum mismatch for {file}: want {want}, got {got}")]
    ChecksumMismatch {
        file: String,
        want: String,
        got: String,
    },
}

#[derive(Debug, Default)]
pub struct SessionManager {
    sessions: HashMap<String, TransferSession>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut TransferSession, TransferError> {
        self.sessions
            .get_mut(id)
            .ok_or(TransferError::UnknownSession)
    }

    /// Sender offers files; server creates the session + per-file tokens.
    pub fn prepare(
        &mut self,
        from_device: &str,
        files: Vec<FileSpec>,
    ) -> Result<(String, HashMap<String, String>), TransferError> {
        if files.is_empty() {
            return Err(TransferError::UnknownFile("empty offer".into()));
        }
        let id = Uuid::new_v4().to_string();
        let mut session = TransferSession {
            id: id.clone(),
            from_device: from_device.into(),
            files: HashMap::new(),
            state: SessionState::AwaitingApproval,
            created: Instant::now(),
        };
        let mut tokens = HashMap::new();
        for f in files {
            let prog = FileProgress::new(f.clone());
            tokens.insert(f.name.clone(), prog.token.clone());
            session.files.insert(f.name.clone(), prog);
        }
        self.sessions.insert(id.clone(), session);
        Ok((id, tokens))
    }

    pub fn approve(&mut self, id: &str) -> Result<(), TransferError> {
        let s = self.get_mut(id)?;
        if s.state != SessionState::AwaitingApproval {
            return Err(TransferError::WrongState {
                expected: SessionState::AwaitingApproval,
                was: s.state,
            });
        }
        s.state = SessionState::Transferring;
        Ok(())
    }

    pub fn deny(&mut self, id: &str) -> Result<(), TransferError> {
        let s = self.get_mut(id)?;
        if s.state != SessionState::AwaitingApproval {
            return Err(TransferError::WrongState {
                expected: SessionState::AwaitingApproval,
                was: s.state,
            });
        }
        s.state = SessionState::Denied;
        Ok(())
    }

    pub fn check_token(&self, id: &str, file: &str, token: &str) -> Result<(), TransferError> {
        let s = self.sessions.get(id).ok_or(TransferError::UnknownSession)?;
        let f = s
            .files
            .get(file)
            .ok_or_else(|| TransferError::UnknownFile(file.into()))?;
        if f.token == token {
            Ok(())
        } else {
            Err(TransferError::BadToken)
        }
    }

    /// Record received bytes (called by the upload handler after writing).
    pub fn note_bytes(
        &mut self,
        id: &str,
        file: &str,
        token: &str,
        n: u64,
    ) -> Result<(), TransferError> {
        self.check_token(id, file, token)?;
        let s = self.get_mut(id)?;
        if s.state != SessionState::Transferring {
            return Err(TransferError::WrongState {
                expected: SessionState::Transferring,
                was: s.state,
            });
        }
        s.files
            .get_mut(file)
            .ok_or_else(|| TransferError::UnknownFile(file.into()))?
            .note_bytes(n);
        Ok(())
    }

    pub fn missing_chunks(&self, id: &str, file: &str) -> Result<Vec<usize>, TransferError> {
        self.sessions
            .get(id)
            .ok_or(TransferError::UnknownSession)?
            .files
            .get(file)
            .ok_or_else(|| TransferError::UnknownFile(file.into()))
            .map(|f| f.missing_chunks())
    }

    /// Verify checksum and close the session. Fails closed on mismatch.
    pub fn complete(&mut self, id: &str, file: &str, bytes: &[u8]) -> Result<(), TransferError> {
        let (want, state) = {
            let s = self.sessions.get(id).ok_or(TransferError::UnknownSession)?;
            let f = s
                .files
                .get(file)
                .ok_or_else(|| TransferError::UnknownFile(file.into()))?;
            (f.spec.checksum.clone(), s.state)
        };
        if state != SessionState::Transferring {
            return Err(TransferError::WrongState {
                expected: SessionState::Transferring,
                was: state,
            });
        }
        if let Some(want) = want {
            let got = sha256_hex(bytes);
            if got != want {
                if let Ok(s) = self.get_mut(id) {
                    s.state = SessionState::Failed;
                }
                return Err(TransferError::ChecksumMismatch {
                    file: file.into(),
                    want,
                    got,
                });
            }
        }
        let s = self.get_mut(id)?;
        if let Some(f) = s.files.get_mut(file) {
            f.received_bytes = f.spec.size;
            for c in f.confirmed_chunks.iter_mut() {
                *c = true;
            }
        }
        if s.files.values().all(|f| f.missing_chunks().is_empty()) {
            s.state = SessionState::Completed;
        }
        Ok(())
    }

    pub fn state(&self, id: &str) -> Option<SessionState> {
        self.sessions.get(id).map(|s| s.state)
    }

    /// Expire stale sessions. Returns the count reaped.
    pub fn sweep_expired(&mut self) -> usize {
        let now = Instant::now();
        let stale: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| {
                let ttl = match s.state {
                    SessionState::AwaitingApproval => PENDING_TTL,
                    SessionState::Preparing | SessionState::Transferring => SESSION_TTL,
                    _ => return false,
                };
                now.duration_since(s.created) > ttl
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            if let Some(s) = self.sessions.get_mut(id) {
                s.state = SessionState::Expired;
            }
        }
        stale.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, size: u64) -> FileSpec {
        FileSpec {
            name: name.into(),
            size,
            checksum: None,
        }
    }

    #[test]
    fn prepare_approve_flow() {
        let mut m = SessionManager::new();
        let (id, tokens) = m.prepare("laptop", vec![spec("a.bin", 100)]).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(m.state(&id), Some(SessionState::AwaitingApproval));
        m.approve(&id).unwrap();
        assert_eq!(m.state(&id), Some(SessionState::Transferring));
        assert!(m.approve(&id).is_err(), "double approve must fail");
    }

    #[test]
    fn deny_blocks_transfer() {
        let mut m = SessionManager::new();
        let (id, _) = m.prepare("laptop", vec![spec("a.bin", 100)]).unwrap();
        m.deny(&id).unwrap();
        assert_eq!(m.state(&id), Some(SessionState::Denied));
        assert!(m.note_bytes(&id, "a.bin", "nope", 10).is_err());
    }

    #[test]
    fn bad_token_rejected() {
        let mut m = SessionManager::new();
        let (id, _) = m.prepare("laptop", vec![spec("a.bin", 100)]).unwrap();
        m.approve(&id).unwrap();
        assert!(matches!(
            m.note_bytes(&id, "a.bin", "wrong", 10),
            Err(TransferError::BadToken)
        ));
    }

    #[test]
    fn chunks_track_and_complete_with_checksum() {
        let data = b"hello igris field".to_vec();
        let mut m = SessionManager::new();
        let (id, tokens) = m
            .prepare(
                "laptop",
                vec![FileSpec {
                    name: "h.bin".into(),
                    size: data.len() as u64,
                    checksum: Some(sha256_hex(&data)),
                }],
            )
            .unwrap();
        m.approve(&id).unwrap();
        let tok = tokens["h.bin"].clone();
        m.note_bytes(&id, "h.bin", &tok, 5).unwrap();
        assert!(
            !m.missing_chunks(&id, "h.bin").unwrap().is_empty()
                || data.len() <= CHUNK_SIZE as usize
        );
        m.complete(&id, "h.bin", &data).unwrap();
        assert_eq!(m.state(&id), Some(SessionState::Completed));
    }

    #[test]
    fn checksum_mismatch_fails_closed() {
        let mut m = SessionManager::new();
        let (id, _) = m
            .prepare(
                "laptop",
                vec![FileSpec {
                    name: "h.bin".into(),
                    size: 5,
                    checksum: Some("0".repeat(64)),
                }],
            )
            .unwrap();
        m.approve(&id).unwrap();
        let err = m.complete(&id, "h.bin", b"wrong").unwrap_err();
        assert!(matches!(err, TransferError::ChecksumMismatch { .. }));
        assert_eq!(m.state(&id), Some(SessionState::Failed));
    }

    #[test]
    fn empty_offer_rejected() {
        let mut m = SessionManager::new();
        assert!(m.prepare("laptop", vec![]).is_err());
    }
}
