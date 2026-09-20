//! igris-orchestrator — durable task graphs.
//!
//! Missions survive crashes: state + checkpoints serialize to JSON,
//! and `resume_point()` restarts from the first incomplete node —
//! long-running work is never restarted from scratch.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionState {
    Running,
    Paused,
    Cancelled,
    Completed,
    Failed,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum OrchestratorError {
    #[error("unknown node: {0}")]
    UnknownNode(String),
    #[error("mission is not running")]
    NotRunning,
    #[error("dependency not satisfied for: {0}")]
    DependencyBlocked(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: Uuid,
    pub name: String,
    pub state: NodeState,
    pub attempts: u32,
    pub max_retries: u32,
    /// Per-attempt timeout. `None` = no limit.
    pub timeout_ms: Option<u64>,
    /// Node IDs that must complete first.
    pub depends_on: Vec<Uuid>,
    /// When true the node is done only after explicit verification.
    pub requires_verification: bool,
    pub verified: bool,
    pub last_error: Option<String>,
    pub checkpoint: Option<String>,
}

impl TaskNode {
    pub fn new(name: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            state: NodeState::Pending,
            attempts: 0,
            max_retries: 3,
            timeout_ms: None,
            depends_on: vec![],
            requires_verification: false,
            verified: false,
            last_error: None,
            checkpoint: None,
        }
    }

    pub fn with_deps(mut self, deps: &[Uuid]) -> Self {
        self.depends_on = deps.to_vec();
        self
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    pub fn with_verification(mut self) -> Self {
        self.requires_verification = true;
        self
    }

    /// Retry backoff: 1s, 2s, 4s … capped at 30s. Attempt counting starts at 1.
    pub fn retry_delay(&self) -> Duration {
        let secs = 2u64.saturating_pow(self.attempts.saturating_sub(1)).min(30);
        Duration::from_secs(secs.max(1))
    }

    pub fn is_done(&self) -> bool {
        self.state == NodeState::Completed && (!self.requires_verification || self.verified)
    }

    pub fn is_timed_out(&self, started: Instant, now: Instant) -> bool {
        self.timeout_ms
            .map(|ms| now.duration_since(started) > Duration::from_millis(ms))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub nodes: HashMap<Uuid, TaskNode>,
    pub order: Vec<Uuid>,
    pub state: MissionState,
}

impl Default for Mission {
    fn default() -> Self {
        Self {
            nodes: HashMap::new(),
            order: Vec::new(),
            state: MissionState::Running,
        }
    }
}

impl Mission {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, node: TaskNode) -> Uuid {
        let id = node.id;
        self.order.push(id);
        self.nodes.insert(id, node);
        id
    }

    /// Legacy direct state set (kept for tooling/debug; prefer record_*).
    pub fn mark(&mut self, id: &Uuid, state: NodeState) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.state = state;
        }
    }

    pub fn checkpoint(&mut self, id: &Uuid, data: &str) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.checkpoint = Some(data.into());
        }
    }

    pub fn start(&mut self) {
        self.state = MissionState::Running;
    }

    pub fn pause(&mut self) {
        self.state = MissionState::Paused;
    }

    pub fn resume(&mut self) {
        if self.state == MissionState::Paused {
            self.state = MissionState::Running;
        }
    }

    pub fn cancel(&mut self) {
        self.state = MissionState::Cancelled;
        for n in self.nodes.values_mut() {
            if n.state == NodeState::Pending || n.state == NodeState::Running {
                n.state = NodeState::Cancelled;
            }
        }
    }

    fn deps_satisfied(&self, n: &TaskNode) -> bool {
        n.depends_on
            .iter()
            .all(|d| self.nodes.get(d).map(|x| x.is_done()).unwrap_or(false))
    }

    /// Nodes ready to execute: mission running, node pending, deps done.
    pub fn ready_tasks(&self) -> Vec<Uuid> {
        if self.state != MissionState::Running {
            return vec![];
        }
        self.order
            .iter()
            .filter(|id| {
                self.nodes
                    .get(*id)
                    .map(|n| n.state == NodeState::Pending && self.deps_satisfied(n))
                    .unwrap_or(false)
            })
            .copied()
            .collect()
    }

    pub fn begin(&mut self, id: &Uuid) -> Result<(), OrchestratorError> {
        if self.state != MissionState::Running {
            return Err(OrchestratorError::NotRunning);
        }
        let node = self
            .nodes
            .get(id)
            .ok_or_else(|| OrchestratorError::UnknownNode(id.to_string()))?;
        if !self.deps_satisfied(node) {
            return Err(OrchestratorError::DependencyBlocked(node.name.clone()));
        }
        if let Some(n) = self.nodes.get_mut(id) {
            n.state = NodeState::Running;
            n.attempts += 1;
            n.last_error = None;
        }
        Ok(())
    }

    pub fn record_success(&mut self, id: &Uuid, checkpoint: Option<&str>) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.state = NodeState::Completed;
            if let Some(c) = checkpoint {
                n.checkpoint = Some(c.into());
            }
            if !n.requires_verification {
                n.verified = true;
            }
        }
        self.refresh_state();
    }

    /// Failure: requeue while attempts remain, else mark Failed.
    /// Returns true when the node will be retried.
    pub fn record_failure(&mut self, id: &Uuid, err: &str) -> bool {
        let retry = if let Some(n) = self.nodes.get_mut(id) {
            n.last_error = Some(err.into());
            if n.attempts <= n.max_retries {
                n.state = NodeState::Pending;
                true
            } else {
                n.state = NodeState::Failed;
                false
            }
        } else {
            return false;
        };
        self.refresh_state();
        retry
    }

    pub fn verify_node(&mut self, id: &Uuid) -> bool {
        if let Some(n) = self.nodes.get_mut(id) {
            if n.state == NodeState::Completed {
                n.verified = true;
                self.refresh_state();
                return true;
            }
        }
        false
    }

    fn refresh_state(&mut self) {
        if self.state != MissionState::Running {
            return;
        }
        if self.nodes.values().any(|n| n.state == NodeState::Failed) {
            self.state = MissionState::Failed;
        } else if !self.nodes.is_empty() && self.nodes.values().all(|n| n.is_done()) {
            self.state = MissionState::Completed;
        }
    }

    pub fn is_complete(&self) -> bool {
        self.state == MissionState::Completed
    }

    /// Resume point: first ready-or-running node in insertion order.
    /// After a crash, Running nodes are treated as Pending (lease expired).
    pub fn resume_point(&self) -> Option<Uuid> {
        self.order
            .iter()
            .find(|id| {
                self.nodes
                    .get(*id)
                    .map(|n| {
                        (n.state == NodeState::Pending && self.deps_satisfied(n))
                            || n.state == NodeState::Running
                    })
                    .unwrap_or(false)
            })
            .copied()
    }

    /// On restart, leases held by a dead process must not block the queue.
    pub fn reclaim_running(&mut self) {
        for n in self.nodes.values_mut() {
            if n.state == NodeState::Running {
                n.state = NodeState::Pending;
            }
        }
    }

    pub fn progress_pct(&self) -> u8 {
        if self.order.is_empty() {
            return 100;
        }
        let done = self
            .order
            .iter()
            .filter(|id| self.nodes.get(*id).map(|n| n.is_done()).unwrap_or(false))
            .count();
        ((done * 100) / self.order.len()) as u8
    }

    // -- durability ------------------------------------------------------

    pub fn to_json(&self) -> Result<String, OrchestratorError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| OrchestratorError::UnknownNode(e.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, OrchestratorError> {
        serde_json::from_str(json).map_err(|e| OrchestratorError::UnknownNode(e.to_string()))
    }
}

/// Run a future with a node-style timeout. Used by executors, tested here.
pub async fn run_with_timeout<F, T>(fut: F, timeout_ms: u64) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_skips_completed() {
        let mut m = Mission::new();
        let a = m.add(TaskNode::new("research"));
        let b = m.add(TaskNode::new("implement"));
        m.mark(&a, NodeState::Completed);
        // Completed without verification requirement counts as done.
        m.nodes.get_mut(&a).unwrap().verified = true;
        assert_eq!(m.resume_point(), Some(b));
        assert_eq!(m.progress_pct(), 50);
    }

    #[test]
    fn deps_block_until_done() {
        let mut m = Mission::new();
        let a = m.add(TaskNode::new("research"));
        let b = m.add(TaskNode::new("implement").with_deps(&[a]));
        assert_eq!(m.ready_tasks(), vec![a]);
        assert!(m.begin(&b).is_err());
        m.begin(&a).unwrap();
        m.record_success(&a, None);
        assert_eq!(m.ready_tasks(), vec![b]);
    }

    #[test]
    fn multi_dep_needs_all_parents() {
        let mut m = Mission::new();
        let a = m.add(TaskNode::new("a"));
        let b = m.add(TaskNode::new("b"));
        let c = m.add(TaskNode::new("c").with_deps(&[a, b]));
        m.begin(&a).unwrap();
        m.record_success(&a, None);
        assert!(m.ready_tasks().iter().all(|id| *id != c));
        m.begin(&b).unwrap();
        m.record_success(&b, None);
        assert_eq!(m.ready_tasks(), vec![c]);
    }

    #[test]
    fn retry_then_fail() {
        let mut m = Mission::new();
        let mut n = TaskNode::new("flaky");
        n.max_retries = 2;
        let id = m.add(n);
        m.begin(&id).unwrap(); // attempts=1
        assert!(m.record_failure(&id, "boom")); // 1<=2 -> retry
        m.begin(&id).unwrap(); // attempts=2
        assert!(m.record_failure(&id, "boom")); // 2<=2 -> retry
        m.begin(&id).unwrap(); // attempts=3
        assert!(!m.record_failure(&id, "boom")); // 3>2 -> failed
        assert_eq!(m.nodes.get(&id).unwrap().state, NodeState::Failed);
        assert_eq!(m.state, MissionState::Failed);
    }

    #[test]
    fn retry_backoff_grows() {
        let mut n = TaskNode::new("x");
        n.attempts = 1;
        assert_eq!(n.retry_delay(), Duration::from_secs(1));
        n.attempts = 3;
        assert_eq!(n.retry_delay(), Duration::from_secs(4));
        n.attempts = 99;
        assert_eq!(n.retry_delay(), Duration::from_secs(30));
    }

    #[test]
    fn pause_and_cancel_block_ready() {
        let mut m = Mission::new();
        let a = m.add(TaskNode::new("a"));
        m.pause();
        assert!(m.ready_tasks().is_empty());
        assert!(m.begin(&a).is_err());
        m.resume();
        assert_eq!(m.ready_tasks(), vec![a]);
        m.cancel();
        assert!(m.ready_tasks().is_empty());
        assert_eq!(m.nodes.get(&a).unwrap().state, NodeState::Cancelled);
    }

    #[test]
    fn verification_gates_completion() {
        let mut m = Mission::new();
        let a = m.add(TaskNode::new("deploy").with_verification());
        m.begin(&a).unwrap();
        m.record_success(&a, Some("deployed-sha123"));
        assert!(
            !m.is_complete(),
            "unverified node must not complete the mission"
        );
        assert!(
            !m.verify_node(&Uuid::new_v4()),
            "unknown node verifies false"
        );
        assert!(m.verify_node(&a));
        assert!(m.is_complete());
    }

    #[test]
    fn persistence_roundtrip_recovers() {
        let mut m = Mission::new();
        let a = m.add(TaskNode::new("research"));
        let b = m.add(TaskNode::new("implement").with_deps(&[a]));
        m.begin(&a).unwrap();
        m.record_success(&a, Some("notes..."));
        let json = m.to_json().unwrap();
        // Simulate a crash: drop and restore.
        let restored = Mission::from_json(&json).unwrap();
        assert_eq!(restored.resume_point(), Some(b));
        assert_eq!(
            restored.nodes.get(&a).unwrap().checkpoint.as_deref(),
            Some("notes...")
        );
        assert_eq!(restored.progress_pct(), 50);
    }

    #[test]
    fn reclaim_running_after_crash() {
        let mut m = Mission::new();
        let a = m.add(TaskNode::new("long-job"));
        m.begin(&a).unwrap();
        let json = m.to_json().unwrap();
        let mut restored = Mission::from_json(&json).unwrap();
        restored.reclaim_running();
        assert_eq!(restored.nodes.get(&a).unwrap().state, NodeState::Pending);
        assert_eq!(restored.resume_point(), Some(a));
    }

    #[test]
    fn timeout_detection() {
        let n = TaskNode::new("slow").with_timeout(50);
        let t0 = Instant::now();
        assert!(!n.is_timed_out(t0, t0 + Duration::from_millis(10)));
        assert!(n.is_timed_out(t0, t0 + Duration::from_millis(100)));
        let unlimited = TaskNode::new("free");
        assert!(!unlimited.is_timed_out(t0, t0 + Duration::from_secs(3600)));
    }

    #[tokio::test]
    async fn run_with_timeout_respects_deadline() {
        let fast = run_with_timeout(async { 42 }, 200).await;
        assert_eq!(fast, Some(42));
        let slow = run_with_timeout(
            async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                7
            },
            20,
        )
        .await;
        assert_eq!(slow, None);
    }

    #[test]
    fn budget_ready_tasks_200_nodes_under_50ms() {
        use std::time::Instant;
        let mut m = Mission::new();
        let mut prev = None;
        for i in 0..200 {
            let node = match prev {
                None => TaskNode::new(&format!("step-{i}")),
                Some(p) => TaskNode::new(&format!("step-{i}")).with_deps(&[p]),
            };
            prev = Some(m.add(node));
        }
        let t0 = Instant::now();
        let ready = m.ready_tasks();
        let elapsed = t0.elapsed();
        assert_eq!(ready.len(), 1);
        assert!(
            elapsed.as_millis() < 50,
            "ready_tasks took {elapsed:?}, budget 50ms"
        );
    }
}
