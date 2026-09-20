//! igris-reflex — fast decision layer, no LLM on fast path.
//! Budget: <100ms per decision (heuristic path runs in microseconds).
//! Typed intents + calibrated confidence + escalation + bounded cache.
//! A tiny decision model can replace the rule core later; the API is stable.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

// ---------------------------------------------------------------------------
// Taxonomy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intent {
    DeviceTransfer,
    DeviceStatus,
    MemoryQuery,
    ToolExec,
    Communication,
    RiskyOp,
    General,
}

impl Intent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Intent::DeviceTransfer => "device_transfer",
            Intent::DeviceStatus => "device_status",
            Intent::MemoryQuery => "memory_query",
            Intent::ToolExec => "tool_exec",
            Intent::Communication => "communication",
            Intent::RiskyOp => "risky_op",
            Intent::General => "general",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Risk {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelHint {
    LocalTiny,
    LocalSmall,
    FastCloud,
    ReasoningCloud,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflexDecision {
    pub intent: Intent,
    pub tool: Option<String>,
    pub target_device: Option<String>,
    pub requires_reasoning: bool,
    pub risk: Risk,
    pub approval: bool,
    pub memory_scope: String,
    pub model_hint: ModelHint,
    pub confidence: f32,
    /// True when served from the decision cache.
    pub cached: bool,
}

// ---------------------------------------------------------------------------
// Escalation policy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct EscalationPolicy {
    /// Below this confidence the decision is forced to reasoning.
    pub min_confidence: f32,
    /// Risk at or above this always requires approval.
    pub approval_from: Risk,
}

impl Default for EscalationPolicy {
    fn default() -> Self {
        Self {
            min_confidence: 0.6,
            approval_from: Risk::High,
        }
    }
}

impl EscalationPolicy {
    pub fn apply(&self, d: &mut ReflexDecision) {
        if d.confidence < self.min_confidence {
            d.requires_reasoning = true;
        }
        if d.risk >= self.approval_from {
            d.approval = true;
            d.requires_reasoning = true;
        }
        if d.risk == Risk::Critical {
            d.model_hint = ModelHint::ReasoningCloud;
        }
    }
}

// ---------------------------------------------------------------------------
// Bounded decision cache (LRU-ish, cap 256)
// ---------------------------------------------------------------------------

const CACHE_CAP: usize = 256;

#[derive(Debug, Default)]
pub struct DecisionCache {
    map: HashMap<String, ReflexDecision>,
    order: VecDeque<String>,
    pub hits: u64,
    pub misses: u64,
}

impl DecisionCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&mut self, key: &str) -> Option<ReflexDecision> {
        if let Some(d) = self.map.get(key) {
            self.hits += 1;
            let mut d = d.clone();
            d.cached = true;
            Some(d)
        } else {
            self.misses += 1;
            None
        }
    }

    pub fn put(&mut self, key: String, mut decision: ReflexDecision) {
        decision.cached = false;
        if !self.map.contains_key(&key) {
            self.order.push_back(key.clone());
            while self.order.len() > CACHE_CAP {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
        self.map.insert(key, decision);
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Reflex engine
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Reflex {
    pub policy: EscalationPolicy,
    pub cache: DecisionCache,
}

impl Default for Reflex {
    fn default() -> Self {
        Self::new()
    }
}

impl Reflex {
    pub fn new() -> Self {
        Self {
            policy: EscalationPolicy::default(),
            cache: DecisionCache::new(),
        }
    }

    pub fn decide(&mut self, text: &str) -> ReflexDecision {
        let key = normalize(text);
        if let Some(hit) = self.cache.get(&key) {
            return hit;
        }
        let mut d = classify(&key);
        self.policy.apply(&mut d);
        self.cache.put(key, d.clone());
        d
    }
}

/// Stateless entry point (no cache). Prefer `Reflex` for hot paths.
pub fn decide(text: &str) -> ReflexDecision {
    let key = normalize(text);
    let mut d = classify(&key);
    EscalationPolicy::default().apply(&mut d);
    d
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn count_hits(t: &str, words: &[&str]) -> usize {
    words.iter().filter(|w| t.contains(**w)).count()
}

fn calibrate(base: f32, extra_signals: usize) -> f32 {
    (base + 0.05 * extra_signals as f32).min(0.97)
}

fn classify(t: &str) -> ReflexDecision {
    // 1. Risky operations — checked first so destructive intent is never fast-pathed.
    let destructive = count_hits(
        t,
        &[
            "delete",
            "format",
            "rm -rf",
            "wipe",
            "shutdown",
            "pay ",
            "transfer money",
        ],
    );
    let external_send = count_hits(
        t,
        &[
            "send email",
            "send sms",
            "send message",
            "post publicly",
            "publish",
        ],
    );
    if destructive + external_send > 0 {
        let critical = destructive > 0;
        return ReflexDecision {
            intent: Intent::RiskyOp,
            tool: None,
            target_device: None,
            requires_reasoning: true,
            risk: if critical { Risk::Critical } else { Risk::High },
            approval: true,
            memory_scope: "global".into(),
            model_hint: ModelHint::ReasoningCloud,
            confidence: calibrate(0.7, destructive + external_send - 1),
            cached: false,
        };
    }

    // 2. Device transfer: send/share/push + payload + known target.
    let targets = ["laptop", "phone", "server"];
    let target = targets
        .iter()
        .find(|d| t.contains(**d))
        .map(|s| s.to_string());
    let transfer_verbs = count_hits(t, &["send", "share", "push", "copy to", "move to"]);
    let payload = count_hits(
        t,
        &[
            "build", "file", "photo", "video", "document", "doc ", "zip", "apk",
        ],
    );
    if transfer_verbs > 0 && (target.is_some() || payload > 0) {
        let conf = if target.is_some() {
            calibrate(0.85, transfer_verbs + payload)
        } else {
            0.55
        };
        return ReflexDecision {
            intent: Intent::DeviceTransfer,
            tool: Some("fastswap".into()),
            target_device: target,
            requires_reasoning: false,
            risk: Risk::Low,
            approval: false,
            memory_scope: "current_project".into(),
            model_hint: ModelHint::LocalTiny,
            confidence: conf,
            cached: false,
        };
    }

    // 3. Device status: battery/cpu/storage/health probes.
    let status_hits = count_hits(
        t,
        &[
            "battery",
            "cpu",
            "storage",
            "disk space",
            "memory usage",
            "how much",
            "status",
            "health",
        ],
    );
    if status_hits > 0 {
        return ReflexDecision {
            intent: Intent::DeviceStatus,
            tool: Some("device_query".into()),
            target_device: None,
            requires_reasoning: false,
            risk: Risk::Low,
            approval: false,
            memory_scope: "device".into(),
            model_hint: ModelHint::LocalTiny,
            confidence: calibrate(0.88, status_hits - 1),
            cached: false,
        };
    }

    // 4. Memory queries: recall-flavored verbs route to local memory, never cloud reasoning.
    let mem_hits = count_hits(
        t,
        &[
            "remember",
            "recall",
            "what did",
            "forgot",
            "note ",
            "my preference",
            "remind me",
            "what is my",
        ],
    );
    if mem_hits > 0 {
        return ReflexDecision {
            intent: Intent::MemoryQuery,
            tool: Some("memory_search".into()),
            target_device: None,
            requires_reasoning: false,
            risk: Risk::Low,
            approval: false,
            memory_scope: "long_term".into(),
            model_hint: ModelHint::LocalTiny,
            confidence: calibrate(0.78, mem_hits - 1),
            cached: false,
        };
    }

    // 5. Communication (non-risky): drafts and reminders need planning but not approval.
    let comm_hits = count_hits(t, &["draft", "remind", "notify", "schedule", "calendar"]);
    if comm_hits > 0 {
        return ReflexDecision {
            intent: Intent::Communication,
            tool: Some("messaging".into()),
            target_device: None,
            requires_reasoning: true,
            risk: Risk::Medium,
            approval: false,
            memory_scope: "recent".into(),
            model_hint: ModelHint::FastCloud,
            confidence: calibrate(0.62, comm_hits - 1),
            cached: false,
        };
    }

    // 6. Local tool execution: build/test/open/run fall back to reasoning planner.
    let tool_hits = count_hits(
        t,
        &[
            "build",
            "test",
            "run",
            "open",
            "compile",
            "search for",
            "find ",
        ],
    );
    if tool_hits > 0 {
        let tool = if t.contains("search for") || t.contains("find ") {
            Some("browser".into())
        } else if t.contains("open") {
            Some("app_launcher".into())
        } else {
            Some("terminal".into())
        };
        return ReflexDecision {
            intent: Intent::ToolExec,
            tool,
            target_device: None,
            requires_reasoning: true,
            risk: Risk::Medium,
            approval: false,
            memory_scope: "current_project".into(),
            model_hint: ModelHint::LocalSmall,
            confidence: calibrate(0.6, tool_hits - 1),
            cached: false,
        };
    }

    // 7. Fallback: escalate to reasoning.
    ReflexDecision {
        intent: Intent::General,
        tool: None,
        target_device: None,
        requires_reasoning: true,
        risk: Risk::Medium,
        approval: false,
        memory_scope: "recent".into(),
        model_hint: ModelHint::FastCloud,
        confidence: 0.5,
        cached: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn fast_path_transfer() {
        let d = decide("Send the latest build to my laptop");
        assert_eq!(d.intent, Intent::DeviceTransfer);
        assert!(!d.requires_reasoning);
        assert_eq!(d.tool.as_deref(), Some("fastswap"));
        assert_eq!(d.target_device.as_deref(), Some("laptop"));
        assert_eq!(d.risk, Risk::Low);
        assert_eq!(d.model_hint, ModelHint::LocalTiny);
    }

    #[test]
    fn device_status_battery() {
        let d = decide("  BATTERY status?  ");
        assert_eq!(d.intent, Intent::DeviceStatus);
        assert!(!d.requires_reasoning && !d.approval);
    }

    #[test]
    fn risky_escalates() {
        let d = decide("delete everything");
        assert_eq!(d.intent, Intent::RiskyOp);
        assert_eq!(d.risk, Risk::Critical);
        assert!(d.requires_reasoning && d.approval);
    }

    #[test]
    fn external_send_needs_approval() {
        let d = decide("send email to the team about the release");
        assert_eq!(d.intent, Intent::RiskyOp);
        assert!(d.approval && d.requires_reasoning);
    }

    #[test]
    fn memory_query_stays_local() {
        let d = decide("what did we decide about the router?");
        assert_eq!(d.intent, Intent::MemoryQuery);
        assert_eq!(d.model_hint, ModelHint::LocalTiny);
        assert!(!d.requires_reasoning);
    }

    #[test]
    fn gibberish_escalates_to_reasoning() {
        let d = decide("florp wobble zephyr quantum banana");
        assert_eq!(d.intent, Intent::General);
        assert!(d.requires_reasoning);
        assert!(d.confidence < 0.6);
    }

    #[test]
    fn normalization_is_case_and_space_insensitive() {
        let a = decide("Send the build to my LAPTOP");
        let b = decide("  send   the build to my laptop  ");
        assert_eq!(a.intent, b.intent);
        assert!((a.confidence - b.confidence).abs() < f32::EPSILON);
    }

    #[test]
    fn cache_hit_marks_cached() {
        let mut r = Reflex::new();
        let first = r.decide("battery status please");
        assert!(!first.cached);
        let second = r.decide("battery status please");
        assert!(second.cached);
        assert_eq!(r.cache.hits, 1);
        assert_eq!(second.intent, first.intent);
    }

    #[test]
    fn cache_is_bounded() {
        let mut r = Reflex::new();
        for i in 0..400 {
            r.decide(&format!("totally unique query number {i} xyzzy"));
        }
        assert!(r.cache.len() <= 256);
    }

    #[test]
    fn latency_budget() {
        // Budget: every decision <100ms; heuristic path should average <5ms.
        let inputs = [
            "Send the latest build to my laptop",
            "battery?",
            "what did we decide about the router?",
            "delete everything",
            "draft a reminder for tomorrow",
            "run the test suite",
        ];
        let start = Instant::now();
        for _ in 0..500 {
            for input in &inputs {
                let t0 = Instant::now();
                let _ = decide(input);
                assert!(
                    t0.elapsed().as_millis() < 100,
                    "reflex exceeded 100ms budget"
                );
            }
        }
        let avg_us = start.elapsed().as_micros() as f64 / 3000.0;
        assert!(avg_us < 5000.0, "avg {avg_us:.1}us exceeds 5ms");
    }
}
