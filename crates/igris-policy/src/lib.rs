//! igris-policy — capabilities, autonomy levels, approvals, audit, kill-switch.
//!
//! Phase 8: every dangerous action funnels through `Policy::check`, approvals
//! expire, the audit log is hash-chained (tamper-evident), and one switch
//! halts all autonomous execution. Chain hashing uses a non-cryptographic
//! hash as a structural placeholder — HMAC-SHA256 lands in hardening.

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Autonomy {
    Assist,
    Operate,
    Autonomous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PermissionLevel {
    L0Read = 0,
    L1Write = 1,
    L2Exec = 2,
    L3Sensitive = 3,
    L4Destructive = 4,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PolicyError {
    #[error("denied: {0}")]
    Denied(String),
    #[error("approval required: {0}")]
    ApprovalRequired(String),
    #[error("killed: {0}")]
    Killed(String),
}

// ---------------------------------------------------------------------------
// Kill switch — one shared flag across all Policy clones.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct KillSwitch {
    engaged: Arc<AtomicBool>,
}

impl KillSwitch {
    pub fn new() -> Self {
        Self {
            engaged: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn engage(&self) {
        self.engaged.store(true, Ordering::SeqCst);
    }

    pub fn disengage(&self) {
        self.engaged.store(false, Ordering::SeqCst);
    }

    pub fn is_engaged(&self) -> bool {
        self.engaged.load(Ordering::SeqCst)
    }
}

impl Default for KillSwitch {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Policy gate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Policy {
    pub autonomy: Autonomy,
    pub max_auto_level: PermissionLevel,
    pub kill: KillSwitch,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            autonomy: Autonomy::Assist,
            max_auto_level: PermissionLevel::L1Write,
            kill: KillSwitch::new(),
        }
    }
}

impl Policy {
    /// Returns Ok(true) = auto-approved, Err(ApprovalRequired) = must ask user.
    /// Engaged kill-switch denies everything, including reads.
    pub fn check(&self, level: PermissionLevel, risky: bool) -> Result<bool, PolicyError> {
        if self.kill.is_engaged() {
            return Err(PolicyError::Killed("kill-switch engaged".into()));
        }
        if level > PermissionLevel::L4Destructive {
            return Err(PolicyError::Denied("unknown level".into()));
        }
        match self.autonomy {
            Autonomy::Assist => Err(PolicyError::ApprovalRequired(format!(
                "assist mode, L{level:?}"
            ))),
            Autonomy::Operate => {
                if !risky && level <= self.max_auto_level {
                    Ok(true)
                } else {
                    Err(PolicyError::ApprovalRequired(format!(
                        "operate gate, L{level:?} risky={risky}"
                    )))
                }
            }
            Autonomy::Autonomous => {
                if level <= self.max_auto_level {
                    Ok(true)
                } else {
                    Err(PolicyError::ApprovalRequired(format!(
                        "autonomous cap, L{level:?}"
                    )))
                }
            }
        }
    }

    /// Approval-backed check: a live approval for this summary bypasses the
    /// autonomy gate (kill-switch still denies).
    pub fn check_with_approval(
        &self,
        level: PermissionLevel,
        risky: bool,
        approvals: &mut ApprovalStore,
        approval_id: &Uuid,
    ) -> Result<bool, PolicyError> {
        if self.kill.is_engaged() {
            return Err(PolicyError::Killed("kill-switch engaged".into()));
        }
        if approvals.is_approved(approval_id) {
            return Ok(true);
        }
        self.check(level, risky)
    }
}

// ---------------------------------------------------------------------------
// Approvals — requested, decided, and expired on a deadline.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: Uuid,
    pub summary: String,
    pub level: PermissionLevel,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub state: ApprovalState,
}

#[derive(Debug, Default)]
pub struct ApprovalStore {
    items: HashMap<Uuid, Approval>,
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&mut self, summary: &str, level: PermissionLevel, ttl: TimeDelta) -> Uuid {
        let now = Utc::now();
        let id = Uuid::new_v4();
        self.items.insert(
            id,
            Approval {
                id,
                summary: summary.into(),
                level,
                requested_at: now,
                expires_at: now + ttl,
                state: ApprovalState::Pending,
            },
        );
        id
    }

    pub fn approve(&mut self, id: &Uuid) -> bool {
        self.settle(id, ApprovalState::Approved)
    }

    pub fn deny(&mut self, id: &Uuid) -> bool {
        self.settle(id, ApprovalState::Denied)
    }

    fn settle(&mut self, id: &Uuid, to: ApprovalState) -> bool {
        match self.items.get_mut(id) {
            Some(a) if a.state == ApprovalState::Pending && Utc::now() <= a.expires_at => {
                a.state = to;
                true
            }
            _ => false,
        }
    }

    fn sweep(&mut self, id: &Uuid) {
        if let Some(a) = self.items.get_mut(id) {
            if a.state == ApprovalState::Pending && Utc::now() > a.expires_at {
                a.state = ApprovalState::Expired;
            }
        }
    }

    pub fn get(&mut self, id: &Uuid) -> Option<Approval> {
        self.sweep(id);
        self.items.get(id).cloned()
    }

    pub fn is_approved(&mut self, id: &Uuid) -> bool {
        self.sweep(id);
        self.items
            .get(id)
            .map(|a| a.state == ApprovalState::Approved)
            .unwrap_or(false)
    }

    pub fn pending(&self) -> Vec<Approval> {
        let mut out: Vec<Approval> = self
            .items
            .values()
            .filter(|a| a.state == ApprovalState::Pending)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.expires_at.cmp(&b.expires_at));
        out
    }
}

// ---------------------------------------------------------------------------
// Hash-chained audit log (tamper-evident stub)
// ---------------------------------------------------------------------------

fn chain_hash(
    prev: u64,
    seq: u64,
    actor: &str,
    action: &str,
    target: &str,
    ok: bool,
    at: &DateTime<Utc>,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    prev.hash(&mut h);
    seq.hash(&mut h);
    actor.hash(&mut h);
    action.hash(&mut h);
    target.hash(&mut h);
    ok.hash(&mut h);
    at.to_rfc3339().hash(&mut h);
    h.finish()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub id: Uuid,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub ok: bool,
    pub prev_hash: u64,
    pub hash: u64,
}

#[derive(Debug, Default)]
pub struct AuditLog {
    entries: Vec<AuditEntry>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, actor: &str, action: &str, target: &str, ok: bool) -> u64 {
        let seq = self.entries.len() as u64;
        let prev = self.entries.last().map(|e| e.hash).unwrap_or(0);
        let at = Utc::now();
        let hash = chain_hash(prev, seq, actor, action, target, ok, &at);
        self.entries.push(AuditEntry {
            seq,
            id: Uuid::new_v4(),
            at,
            actor: actor.into(),
            action: action.into(),
            target: target.into(),
            ok,
            prev_hash: prev,
            hash,
        });
        hash
    }

    pub fn verify_chain(&self) -> bool {
        let mut prev = 0u64;
        for e in &self.entries {
            if e.prev_hash != prev {
                return false;
            }
            if chain_hash(
                e.prev_hash,
                e.seq,
                &e.actor,
                &e.action,
                &e.target,
                e.ok,
                &e.at,
            ) != e.hash
            {
                return false;
            }
            prev = e.hash;
        }
        true
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assist_always_asks() {
        let p = Policy::default();
        assert!(p.check(PermissionLevel::L0Read, false).is_err());
    }

    #[test]
    fn operate_auto_low_risk() {
        let p = Policy {
            autonomy: Autonomy::Operate,
            ..Default::default()
        };
        assert!(p.check(PermissionLevel::L1Write, false).is_ok());
        assert!(p.check(PermissionLevel::L3Sensitive, false).is_err());
    }

    #[test]
    fn kill_switch_denies_everything() {
        let p = Policy {
            autonomy: Autonomy::Autonomous,
            max_auto_level: PermissionLevel::L4Destructive,
            ..Default::default()
        };
        assert!(p.check(PermissionLevel::L0Read, false).is_ok());
        p.kill.engage();
        let err = p.check(PermissionLevel::L0Read, false).unwrap_err();
        assert!(matches!(err, PolicyError::Killed(_)));
        // Clones share the same switch.
        let q = p.clone();
        assert!(q.kill.is_engaged());
        p.kill.disengage();
        assert!(p.check(PermissionLevel::L0Read, false).is_ok());
    }

    #[test]
    fn approval_bypasses_gate_until_used() {
        let p = Policy::default();
        let mut store = ApprovalStore::new();
        let id = store.request(
            "deploy staging",
            PermissionLevel::L3Sensitive,
            TimeDelta::minutes(5),
        );
        assert_eq!(store.pending().len(), 1);
        assert!(store.approve(&id));
        assert!(store.is_approved(&id));
        assert!(p
            .check_with_approval(PermissionLevel::L3Sensitive, true, &mut store, &id)
            .is_ok());
    }

    #[test]
    fn approval_deny_and_expiry() {
        let mut store = ApprovalStore::new();
        let denied = store.request(
            "wipe disk",
            PermissionLevel::L4Destructive,
            TimeDelta::minutes(5),
        );
        assert!(store.deny(&denied));
        assert!(!store.is_approved(&denied));
        // Second settle on a decided approval fails.
        assert!(!store.approve(&denied));

        let stale = store.request(
            "old request",
            PermissionLevel::L1Write,
            TimeDelta::milliseconds(1),
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(!store.is_approved(&stale));
        assert_eq!(store.get(&stale).unwrap().state, ApprovalState::Expired);
        assert!(store.pending().is_empty());
        // Unknown IDs are safe no-ops.
        assert!(!store.approve(&Uuid::new_v4()));
    }

    #[test]
    fn kill_beats_approval() {
        let p = Policy::default();
        let mut store = ApprovalStore::new();
        let id = store.request("x", PermissionLevel::L1Write, TimeDelta::minutes(5));
        store.approve(&id);
        p.kill.engage();
        assert!(matches!(
            p.check_with_approval(PermissionLevel::L1Write, false, &mut store, &id),
            Err(PolicyError::Killed(_))
        ));
    }

    #[test]
    fn audit_chain_verifies_and_detects_tamper() {
        let mut log = AuditLog::new();
        log.record("assistant", "tool.execute", "terminal", true);
        log.record("assistant", "tool.execute", "filesystem", false);
        assert_eq!(log.len(), 2);
        assert!(log.verify_chain());
        // Tamper with one entry: link breaks.
        log.entries[0].ok = !log.entries[0].ok;
        assert!(!log.verify_chain());
    }
}
