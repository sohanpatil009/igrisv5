//! LWW sync engine: generic over record blobs, merge by
//! (updated_at, device_id) with tombstones. Newer wins; ties break
//! deterministically by device id so every peer converges identically.
//! Memory/task stores adapt their records into `SyncRecord`; transport
//! (already-signed envelopes) carries the batches.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRecord {
    pub id: String,
    pub updated_ms: i64,
    pub device_id: String,
    pub deleted: bool,
    pub payload: Vec<u8>,
}

impl SyncRecord {
    /// True when `other` supersedes `self`.
    pub fn superseded_by(&self, other: &SyncRecord) -> bool {
        (other.updated_ms, &other.device_id) > (self.updated_ms, &self.device_id)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SyncEngine {
    local: HashMap<String, SyncRecord>,
}

#[derive(Debug, Clone, Default)]
pub struct MergeReport {
    pub applied: usize,
    pub skipped_stale: usize,
    pub tombstones_applied: usize,
}

impl SyncEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_local(&mut self, record: SyncRecord) {
        self.local.insert(record.id.clone(), record);
    }

    pub fn get(&self, id: &str) -> Option<&SyncRecord> {
        self.local.get(id)
    }

    pub fn live(&self) -> Vec<&SyncRecord> {
        self.local.values().filter(|r| !r.deleted).collect()
    }

    /// Merge a remote batch. Returns what changed; stale records never clobber.
    pub fn merge(&mut self, remote: Vec<SyncRecord>) -> MergeReport {
        let mut report = MergeReport::default();
        for r in remote {
            match self.local.get(&r.id) {
                None => {
                    if r.deleted {
                        report.tombstones_applied += 1;
                    } else {
                        report.applied += 1;
                    }
                    self.local.insert(r.id.clone(), r);
                }
                Some(cur) if cur.id == r.id && !cur.superseded_by(&r) && !r.superseded_by(cur) => {
                    // Identical stamp: idempotent re-delivery, ignore.
                    report.skipped_stale += 1;
                }
                Some(cur) => {
                    if cur.superseded_by(&r) {
                        if r.deleted {
                            report.tombstones_applied += 1;
                        } else {
                            report.applied += 1;
                        }
                        self.local.insert(r.id.clone(), r);
                    } else {
                        report.skipped_stale += 1;
                    }
                }
            }
        }
        report
    }

    /// Records newer than `since_ms` for push/pull exchange.
    pub fn delta_since(&self, since_ms: i64) -> Vec<SyncRecord> {
        self.local
            .values()
            .filter(|r| r.updated_ms > since_ms)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.local.len()
    }

    pub fn is_empty(&self) -> bool {
        self.local.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, ts: i64, dev: &str) -> SyncRecord {
        SyncRecord {
            id: id.into(),
            updated_ms: ts,
            device_id: dev.into(),
            deleted: false,
            payload: b"v".to_vec(),
        }
    }

    #[test]
    fn newer_wins_older_loses() {
        let mut e = SyncEngine::new();
        e.put_local(rec("a", 100, "dev1"));
        let rep = e.merge(vec![rec("a", 50, "dev2")]);
        assert_eq!(rep.skipped_stale, 1);
        assert_eq!(e.get("a").unwrap().updated_ms, 100);
        let rep = e.merge(vec![rec("a", 150, "dev2")]);
        assert_eq!(rep.applied, 1);
        assert_eq!(e.get("a").unwrap().device_id, "dev2");
    }

    #[test]
    fn tie_breaks_by_device_id_deterministically() {
        let mut e1 = SyncEngine::new();
        let mut e2 = SyncEngine::new();
        e1.put_local(rec("a", 100, "dev1"));
        e2.put_local(rec("a", 100, "dev2"));
        // Same stamps exchanged both ways converge identically.
        let r1 = e1.merge(vec![rec("a", 100, "dev2")]);
        let r2 = e2.merge(vec![rec("a", 100, "dev1")]);
        assert_eq!(
            e1.get("a").unwrap().device_id,
            e2.get("a").unwrap().device_id
        );
        assert_eq!(r1.skipped_stale + r1.applied, 1);
        assert_eq!(r2.skipped_stale + r2.applied, 1);
    }

    #[test]
    fn tombstones_delete_and_stale_resurrect_fails() {
        let mut e = SyncEngine::new();
        e.put_local(rec("a", 100, "dev1"));
        let mut tomb = rec("a", 150, "dev2");
        tomb.deleted = true;
        let rep = e.merge(vec![tomb]);
        assert_eq!(rep.tombstones_applied, 1);
        assert!(e.live().is_empty());
        // Stale live copy cannot resurrect the tombstone.
        let rep = e.merge(vec![rec("a", 120, "dev1")]);
        assert_eq!(rep.skipped_stale, 1);
        assert!(e.live().is_empty());
    }

    #[test]
    fn idempotent_redelivery_ignored() {
        let mut e = SyncEngine::new();
        e.put_local(rec("a", 100, "dev1"));
        let rep = e.merge(vec![rec("a", 100, "dev1")]);
        assert_eq!(rep.skipped_stale, 1);
        assert_eq!(rep.applied, 0);
    }

    #[test]
    fn delta_since_drives_exchange() {
        let mut e = SyncEngine::new();
        e.put_local(rec("a", 100, "dev1"));
        e.put_local(rec("b", 200, "dev1"));
        assert_eq!(e.delta_since(150).len(), 1);
        assert_eq!(e.delta_since(0).len(), 2);
    }
}
