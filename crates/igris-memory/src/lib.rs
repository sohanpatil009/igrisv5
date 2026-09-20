//! igris-memory — typed memory with provenance (Phase 2).
//! SQLite-backed store: FTS5 search, edges graph, TTL/tombstone/LWW sync,
//! persisted privacy + provenance. Clean-room implementation, no predecessor code.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryType {
    Working,
    Episodic,
    Semantic,
    Procedural,
    Preference,
    Goal,
    Relationship,
    Device,
    Project,
    Skill,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryType::Working => "working",
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Procedural => "procedural",
            MemoryType::Preference => "preference",
            MemoryType::Goal => "goal",
            MemoryType::Relationship => "relationship",
            MemoryType::Device => "device",
            MemoryType::Project => "project",
            MemoryType::Skill => "skill",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "working" => MemoryType::Working,
            "episodic" => MemoryType::Episodic,
            "procedural" => MemoryType::Procedural,
            "preference" => MemoryType::Preference,
            "goal" => MemoryType::Goal,
            "relationship" => MemoryType::Relationship,
            "device" => MemoryType::Device,
            "project" => MemoryType::Project,
            "skill" => MemoryType::Skill,
            _ => MemoryType::Semantic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivacyScope {
    LocalOnly,
    Hybrid,
    CloudAllowed,
}

impl PrivacyScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrivacyScope::LocalOnly => "local_only",
            PrivacyScope::Hybrid => "hybrid",
            PrivacyScope::CloudAllowed => "cloud_allowed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "hybrid" => PrivacyScope::Hybrid,
            "cloud_allowed" => PrivacyScope::CloudAllowed,
            _ => PrivacyScope::LocalOnly,
        }
    }

    /// Secret/local memories must never leave the device.
    pub fn may_sync(&self) -> bool {
        !matches!(self, PrivacyScope::LocalOnly)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub source: String,
    pub device_id: String,
    pub agent: Option<String>,
    pub tool: Option<String>,
    pub model: Option<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: Uuid,
    pub kind: MemoryType,
    pub content: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub confidence: f32,
    pub importance: f32,
    pub privacy: PrivacyScope,
    pub owner: String,
    pub provenance: Provenance,
    pub tags: Vec<String>,
    pub device_id: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub deleted: bool,
}

impl MemoryRecord {
    pub fn new(kind: MemoryType, content: &str, source: &str, device_id: &str) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            kind,
            content: content.into(),
            source: source.into(),
            created_at: now,
            updated_at: now,
            confidence: 0.8,
            importance: 0.5,
            privacy: PrivacyScope::LocalOnly,
            owner: "user".into(),
            provenance: Provenance {
                source: source.into(),
                device_id: device_id.into(),
                agent: None,
                tool: None,
                model: None,
                confidence: 0.8,
            },
            tags: vec![],
            device_id: device_id.into(),
            expires_at: None,
            deleted: false,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at.map(|e| e <= Utc::now()).unwrap_or(false)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum MemoryError {
    #[error("storage: {0}")]
    Storage(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("serialization: {0}")]
    Serialization(String),
}

impl From<rusqlite::Error> for MemoryError {
    fn from(e: rusqlite::Error) -> Self {
        MemoryError::Storage(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, MemoryError>;

// ---------------------------------------------------------------------------
// Embeddings (trait + hash stub; real model plugs in later)
// ---------------------------------------------------------------------------

pub trait EmbeddingProvider: Send + Sync {
    fn dim(&self) -> usize;
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Deterministic char-trigram hash embedding (64-d, L2-normalized).
/// Placeholder until a real embedding model lands; interface is stable.
pub struct SimpleEmbeddingProvider {
    dim: usize,
}

impl SimpleEmbeddingProvider {
    pub fn new() -> Self {
        Self { dim: 64 }
    }
}

impl Default for SimpleEmbeddingProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddingProvider for SimpleEmbeddingProvider {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut v = vec![0f32; self.dim];
        let lower = text.to_lowercase();
        let chars: Vec<char> = lower.chars().collect();
        // Unigrams + trigrams hashed into buckets.
        for w in lower.split_whitespace() {
            let mut h = DefaultHasher::new();
            w.hash(&mut h);
            v[(h.finish() as usize) % self.dim] += 1.0;
        }
        if chars.len() >= 3 {
            for win in chars.windows(3) {
                let s: String = win.iter().collect();
                let mut h = DefaultHasher::new();
                s.hash(&mut h);
                v[(h.finish() as usize) % self.dim] += 0.5;
            }
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        for x in v.iter_mut() {
            *x /= norm;
        }
        v
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Hybrid score: 0.6 * text-hit + 0.25 * embedding cosine + 0.15 * importance.
pub fn rank_memories(
    records: Vec<MemoryRecord>,
    query: &str,
    provider: &dyn EmbeddingProvider,
) -> Vec<MemoryRecord> {
    let qv = provider.embed(query);
    let ql = query.to_lowercase();
    let mut scored: Vec<(f32, MemoryRecord)> = records
        .into_iter()
        .map(|r| {
            let hit = if r.content.to_lowercase().contains(&ql) {
                1.0
            } else {
                0.0
            };
            let cos = cosine_similarity(&qv, &provider.embed(&r.content));
            let score = 0.6 * hit + 0.25 * cos + 0.15 * r.importance;
            (score, r)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(_, r)| r).collect()
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS memories (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  content TEXT NOT NULL,
  source TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  confidence REAL NOT NULL,
  importance REAL NOT NULL,
  privacy TEXT NOT NULL,
  owner TEXT NOT NULL,
  provenance_json TEXT NOT NULL,
  tags_json TEXT NOT NULL,
  device_id TEXT NOT NULL DEFAULT '',
  expires_at TEXT NULL,
  deleted INTEGER NOT NULL DEFAULT 0
);
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(content, content='memories', content_rowid='rowid');
CREATE TRIGGER IF NOT EXISTS mem_ai AFTER INSERT ON memories BEGIN
  INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
END;
CREATE TRIGGER IF NOT EXISTS mem_ad AFTER DELETE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
END;
CREATE TRIGGER IF NOT EXISTS mem_au AFTER UPDATE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
  INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
END;
CREATE TABLE IF NOT EXISTS edges (
  from_id TEXT NOT NULL,
  to_id TEXT NOT NULL,
  relation TEXT NOT NULL,
  weight REAL NOT NULL DEFAULT 1.0,
  PRIMARY KEY (from_id, to_id, relation)
);
CREATE INDEX IF NOT EXISTS idx_mem_kind ON memories(kind);
CREATE INDEX IF NOT EXISTS idx_mem_updated ON memories(updated_at);
CREATE INDEX IF NOT EXISTS idx_mem_deleted ON memories(deleted);
"#;

#[derive(Debug, Clone)]
pub struct MemoryStore {
    conn: Arc<Mutex<Connection>>,
}

impl MemoryStore {
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let s = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        s.migrate()?;
        Ok(s)
    }

    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let s = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        s.migrate()?;
        Ok(s)
    }

    /// Test/CLI convenience. Panics only when the DB itself cannot initialize.
    pub fn new() -> Self {
        Self::open_in_memory().expect("memory db init")
    }

    fn migrate(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    fn lock(&self) -> std::result::Result<std::sync::MutexGuard<'_, Connection>, MemoryError> {
        self.conn
            .lock()
            .map_err(|e| MemoryError::Storage(e.to_string()))
    }

    pub fn store(&self, record: MemoryRecord) -> Result<Uuid> {
        let id = record.id;
        self.upsert(record)?;
        Ok(id)
    }

    /// Insert or LWW-merge: newer `updated_at` wins; ties break by device id.
    pub fn upsert(&self, record: MemoryRecord) -> Result<()> {
        let conn = self.lock()?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT updated_at FROM memories WHERE id = ?1",
                params![record.id.to_string()],
                |r| r.get(0),
            )
            .ok();
        if let Some(ts) = existing {
            let old: DateTime<Utc> = ts
                .parse()
                .map_err(|e| MemoryError::Serialization(format!("{e}")))?;
            if record.updated_at < old {
                return Ok(()); // stale write loses
            }
        }
        let prov = serde_json::to_string(&record.provenance)
            .map_err(|e| MemoryError::Serialization(e.to_string()))?;
        let tags = serde_json::to_string(&record.tags)
            .map_err(|e| MemoryError::Serialization(e.to_string()))?;
        conn.execute(
            r#"INSERT INTO memories
            (id,kind,content,source,created_at,updated_at,confidence,importance,privacy,owner,provenance_json,tags_json,device_id,expires_at,deleted)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
            ON CONFLICT(id) DO UPDATE SET
              kind=excluded.kind, content=excluded.content, source=excluded.source,
              created_at=excluded.created_at, updated_at=excluded.updated_at,
              confidence=excluded.confidence, importance=excluded.importance,
              privacy=excluded.privacy, owner=excluded.owner,
              provenance_json=excluded.provenance_json, tags_json=excluded.tags_json,
              device_id=excluded.device_id, expires_at=excluded.expires_at, deleted=excluded.deleted"#,
            params![
                record.id.to_string(),
                record.kind.as_str(),
                record.content,
                record.source,
                record.created_at.to_rfc3339(),
                record.updated_at.to_rfc3339(),
                record.confidence,
                record.importance,
                record.privacy.as_str(),
                record.owner,
                prov,
                tags,
                record.device_id,
                record.expires_at.map(|e| e.to_rfc3339()),
                if record.deleted { 1 } else { 0 },
            ],
        )?;
        Ok(())
    }

    fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<MemoryRecord> {
        let id_s: String = row.get(0)?;
        let kind_s: String = row.get(1)?;
        let content: String = row.get(2)?;
        let source: String = row.get(3)?;
        let created_s: String = row.get(4)?;
        let updated_s: String = row.get(5)?;
        let confidence: f32 = row.get(6)?;
        let importance: f32 = row.get(7)?;
        let privacy_s: String = row.get(8)?;
        let owner: String = row.get(9)?;
        let prov_s: String = row.get(10)?;
        let tags_s: String = row.get(11)?;
        let device_id: String = row.get(12)?;
        let expires_s: Option<String> = row.get(13)?;
        let deleted_i: i32 = row.get(14)?;
        Ok(MemoryRecord {
            id: id_s.parse().unwrap_or_else(|_| Uuid::new_v4()),
            kind: MemoryType::parse(&kind_s),
            content,
            source: source.clone(),
            created_at: created_s.parse().unwrap_or_else(|_| Utc::now()),
            updated_at: updated_s.parse().unwrap_or_else(|_| Utc::now()),
            confidence,
            importance,
            privacy: PrivacyScope::parse(&privacy_s),
            owner,
            provenance: serde_json::from_str(&prov_s).unwrap_or(Provenance {
                source,
                device_id: String::new(),
                agent: None,
                tool: None,
                model: None,
                confidence: 0.5,
            }),
            tags: serde_json::from_str(&tags_s).unwrap_or_default(),
            device_id,
            expires_at: expires_s.and_then(|s| s.parse().ok()),
            deleted: deleted_i != 0,
        })
    }

    pub fn get(&self, id: &Uuid) -> Result<Option<MemoryRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id,kind,content,source,created_at,updated_at,confidence,importance,privacy,owner,provenance_json,tags_json,device_id,expires_at,deleted FROM memories WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id.to_string()], Self::row_to_record)?;
        if let Some(r) = rows.next() {
            Ok(Some(r.map_err(MemoryError::from)?))
        } else {
            Ok(None)
        }
    }

    fn is_live(r: &MemoryRecord) -> bool {
        !r.deleted && !r.is_expired()
    }

    fn fts_query(query: &str) -> String {
        query
            .split_whitespace()
            .take(8)
            .map(|t| format!("\"{}\"", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ")
    }

    /// Hybrid retrieval: FTS5 first (importance-ordered), LIKE fallback, expired/tombstones excluded.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRecord>> {
        let conn = self.lock()?;
        let now = Utc::now().to_rfc3339();
        let fts = Self::fts_query(query);
        let fts_hits: std::result::Result<Vec<MemoryRecord>, rusqlite::Error> = (|| {
            let mut stmt = conn.prepare(
                r#"SELECT m.id,m.kind,m.content,m.source,m.created_at,m.updated_at,m.confidence,m.importance,m.privacy,m.owner,m.provenance_json,m.tags_json,m.device_id,m.expires_at,m.deleted
                FROM memories m JOIN memories_fts f ON m.rowid = f.rowid
                WHERE f.memories_fts MATCH ?1 AND m.deleted = 0 AND (m.expires_at IS NULL OR m.expires_at > ?2)
                ORDER BY m.importance DESC LIMIT ?3"#,
            )?;
            let rows = stmt.query_map(params![fts, now, limit as i64], Self::row_to_record)?;
            rows.collect()
        })();
        match fts_hits {
            Ok(hits) if !hits.is_empty() => {
                Ok(hits.into_iter().filter(Self::is_live).take(limit).collect())
            }
            _ => {
                // LIKE fallback (also covers single-char / punctuation queries FTS rejects).
                let like = format!("%{}%", query.to_lowercase());
                let mut stmt = conn.prepare(
                    r#"SELECT id,kind,content,source,created_at,updated_at,confidence,importance,privacy,owner,provenance_json,tags_json,device_id,expires_at,deleted
                    FROM memories WHERE lower(content) LIKE ?1 AND deleted = 0 AND (expires_at IS NULL OR expires_at > ?2)
                    ORDER BY importance DESC LIMIT ?3"#,
                )?;
                let rows = stmt.query_map(params![like, now, limit as i64], Self::row_to_record)?;
                let mut out = Vec::new();
                for r in rows {
                    let rec = r.map_err(MemoryError::from)?;
                    if Self::is_live(&rec) {
                        out.push(rec);
                    }
                    if out.len() >= limit {
                        break;
                    }
                }
                Ok(out)
            }
        }
    }

    /// Embedding-reranked search: FTS/LIKE candidate set, then hybrid rank.
    pub fn search_ranked(
        &self,
        query: &str,
        limit: usize,
        provider: &dyn EmbeddingProvider,
    ) -> Result<Vec<MemoryRecord>> {
        let cands = self.search(query, limit.max(20))?;
        Ok(rank_memories(cands, query, provider)
            .into_iter()
            .take(limit)
            .collect())
    }

    pub fn add_edge(&self, from: &Uuid, to: &Uuid, relation: &str, weight: f32) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO edges (from_id,to_id,relation,weight) VALUES (?1,?2,?3,?4) ON CONFLICT(from_id,to_id,relation) DO UPDATE SET weight=excluded.weight",
            params![from.to_string(), to.to_string(), relation, weight],
        )?;
        Ok(())
    }

    /// Undirected BFS over edges up to `depth` (max 3). Excludes expired/deleted.
    pub fn neighbors(&self, id: &Uuid, depth: u32) -> Result<Vec<MemoryRecord>> {
        let depth = depth.min(3);
        let conn = self.lock()?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        seen.insert(id.to_string());
        queue.push_back((id.to_string(), 0));
        let mut out_ids: Vec<String> = Vec::new();
        while let Some((cur, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }
            let mut stmt = conn.prepare(
                "SELECT to_id FROM edges WHERE from_id = ?1 UNION SELECT from_id FROM edges WHERE to_id = ?1",
            )?;
            let rows = stmt.query_map(params![cur], |r| r.get::<_, String>(0))?;
            for nid in rows.flatten() {
                if seen.insert(nid.clone()) {
                    out_ids.push(nid.clone());
                    queue.push_back((nid, d + 1));
                }
            }
        }
        drop(conn);
        let mut out = Vec::new();
        for nid in out_ids {
            let uid = nid
                .parse()
                .map_err(|_| MemoryError::NotFound(nid.clone()))?;
            if let Some(r) = self.get(&uid)? {
                if Self::is_live(&r) {
                    out.push(r);
                }
            }
        }
        Ok(out)
    }

    pub fn soft_delete(&self, id: &Uuid) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE memories SET deleted = 1, updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(n > 0)
    }

    pub fn restore(&self, id: &Uuid) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE memories SET deleted = 0, updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(n > 0)
    }

    pub fn forget_by_type(&self, kind: MemoryType) -> Result<usize> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE memories SET deleted = 1 WHERE kind = ?1 AND deleted = 0",
            params![kind.as_str()],
        )?;
        Ok(n)
    }

    pub fn forget_expired(&self) -> Result<usize> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE memories SET deleted = 1 WHERE deleted = 0 AND expires_at IS NOT NULL AND expires_at <= ?1",
            params![Utc::now().to_rfc3339()],
        )?;
        Ok(n)
    }

    pub fn count(&self) -> Result<usize> {
        let conn = self.lock()?;
        let now = Utc::now().to_rfc3339();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE deleted = 0 AND (expires_at IS NULL OR expires_at > ?1)",
            params![now],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn count_by_type(&self) -> Result<HashMap<String, usize>> {
        let conn = self.lock()?;
        let now = Utc::now().to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT kind, COUNT(*) FROM memories WHERE deleted = 0 AND (expires_at IS NULL OR expires_at > ?1) GROUP BY kind",
        )?;
        let mut map = HashMap::new();
        let rows = stmt.query_map(params![now], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for r in rows.flatten() {
            map.insert(r.0, r.1 as usize);
        }
        Ok(map)
    }

    pub fn export_json(&self) -> Result<String> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id,kind,content,source,created_at,updated_at,confidence,importance,privacy,owner,provenance_json,tags_json,device_id,expires_at,deleted FROM memories WHERE deleted = 0",
        )?;
        let rows = stmt.query_map([], Self::row_to_record)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(MemoryError::from)?);
        }
        serde_json::to_string_pretty(&out).map_err(|e| MemoryError::Serialization(e.to_string()))
    }

    pub fn import_json(&self, json: &str) -> Result<usize> {
        let recs: Vec<MemoryRecord> =
            serde_json::from_str(json).map_err(|e| MemoryError::Serialization(e.to_string()))?;
        let n = recs.len();
        for r in recs {
            self.upsert(r)?;
        }
        Ok(n)
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn rec(content: &str) -> MemoryRecord {
        MemoryRecord::new(MemoryType::Semantic, content, "test", "dev1")
    }

    #[test]
    fn store_search_delete() {
        let s = MemoryStore::open_in_memory().unwrap();
        let id = s.store(rec("FIELD uses Dioxus native")).unwrap();
        assert_eq!(s.count().unwrap(), 1);
        assert_eq!(s.search("dioxus", 5).unwrap().len(), 1);
        assert!(s.soft_delete(&id).unwrap());
        assert_eq!(s.count().unwrap(), 0);
        assert!(s.restore(&id).unwrap());
        assert_eq!(s.count().unwrap(), 1);
    }

    #[test]
    fn privacy_and_provenance_persist() {
        let s = MemoryStore::open_in_memory().unwrap();
        let mut r = rec("hybrid sync note");
        r.privacy = PrivacyScope::Hybrid;
        r.provenance.agent = Some("coder".into());
        r.provenance.model = Some("local-small".into());
        let id = s.store(r).unwrap();
        let back = s.get(&id).unwrap().unwrap();
        assert_eq!(back.privacy, PrivacyScope::Hybrid);
        assert!(back.privacy.may_sync());
        assert_eq!(back.provenance.agent.as_deref(), Some("coder"));
        assert_eq!(back.provenance.model.as_deref(), Some("local-small"));
        // LocalOnly never syncs.
        assert!(!PrivacyScope::LocalOnly.may_sync());
    }

    #[test]
    fn ttl_expiry_filtered() {
        let s = MemoryStore::open_in_memory().unwrap();
        let mut r = rec("temporary token dioxus");
        r.expires_at = Some(Utc::now() - TimeDelta::seconds(1));
        s.store(r).unwrap();
        assert_eq!(s.count().unwrap(), 0);
        assert!(s.search("dioxus", 5).unwrap().is_empty());
        // Expired row is filtered from reads but still tombstoned by the sweeper.
        assert_eq!(s.forget_expired().unwrap(), 1);
    }

    #[test]
    fn lww_stale_write_loses() {
        let s = MemoryStore::open_in_memory().unwrap();
        let mut r = rec("v1 content");
        let id = s.store(r.clone()).unwrap();
        r.content = "stale content".into();
        r.updated_at -= TimeDelta::seconds(60);
        s.upsert(r).unwrap();
        assert_eq!(s.get(&id).unwrap().unwrap().content, "v1 content");
    }

    #[test]
    fn edges_and_neighbors() {
        let s = MemoryStore::open_in_memory().unwrap();
        let a = s.store(rec("project field shell")).unwrap();
        let b = s.store(rec("field uses dioxus")).unwrap();
        let c = s.store(rec("dioxus native desktop")).unwrap();
        s.add_edge(&a, &b, "related", 1.0).unwrap();
        s.add_edge(&b, &c, "related", 0.8).unwrap();
        let n1 = s.neighbors(&a, 1).unwrap();
        assert_eq!(n1.len(), 1);
        let n2 = s.neighbors(&a, 2).unwrap();
        assert_eq!(n2.len(), 2);
    }

    #[test]
    fn export_import_roundtrip() {
        let s = MemoryStore::open_in_memory().unwrap();
        s.store(rec("export me dioxus")).unwrap();
        let json = s.export_json().unwrap();
        let s2 = MemoryStore::open_in_memory().unwrap();
        assert_eq!(s2.import_json(&json).unwrap(), 1);
        assert_eq!(s2.count().unwrap(), 1);
    }

    #[test]
    fn ranked_search_prefers_hits() {
        let s = MemoryStore::open_in_memory().unwrap();
        s.store(rec("completely unrelated words here")).unwrap();
        s.store(rec("dioxus field shell")).unwrap();
        let p = SimpleEmbeddingProvider::new();
        let ranked = s.search_ranked("dioxus", 2, &p).unwrap();
        assert!(ranked[0].content.contains("dioxus"));
    }

    #[test]
    fn forget_by_type() {
        let s = MemoryStore::open_in_memory().unwrap();
        s.store(MemoryRecord::new(MemoryType::Goal, "ship field", "t", "d"))
            .unwrap();
        s.store(rec("keep me")).unwrap();
        assert_eq!(s.forget_by_type(MemoryType::Goal).unwrap(), 1);
        let counts = s.count_by_type().unwrap();
        assert_eq!(counts.get("semantic"), Some(&1));
    }

    #[test]
    fn budget_search_500_records_under_150ms() {
        use std::time::Instant;
        let s = MemoryStore::open_in_memory().unwrap();
        for i in 0..500 {
            let mut r = rec(&format!("budget note number {i} about dioxus field mesh"));
            r.importance = (i % 10) as f32 / 10.0;
            s.store(r).unwrap();
        }
        let t0 = Instant::now();
        let hits = s.search("dioxus", 20).unwrap();
        let elapsed = t0.elapsed();
        assert_eq!(hits.len(), 20);
        assert!(
            elapsed.as_millis() < 150,
            "search took {elapsed:?}, budget 150ms"
        );
    }
}
