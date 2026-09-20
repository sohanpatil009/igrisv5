//! FIELD backend: every panel reads live cores — no simulated telemetry.
//! Thin, synchronous, and fully unit-tested. The Dioxus tree polls it on a
//! 1s tick (pausable); live event subscription arrives as an SSE stream.

use chrono::TimeDelta;
use igris_device::discovery::{DiscoveredDevice, Discovery, DiscoveryConfig};
use igris_events::{Event, EventBus};
use igris_memory::{MemoryRecord, MemoryStore, MemoryType};
use igris_orchestrator::{Mission, TaskNode};
use igris_policy::{Approval, ApprovalStore, Autonomy, PermissionLevel, Policy};
use igris_telemetry::{Telemetry, TimelineEntry};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct FieldBackend {
    pub events: Arc<EventBus>,
    pub memory: MemoryStore,
    pub mission: Arc<Mutex<Mission>>,
    pub approvals: Arc<Mutex<ApprovalStore>>,
    pub policy: Arc<Mutex<Policy>>,
    pub telemetry: Telemetry,
    pub discovery: Arc<Mutex<Discovery>>,
}

// Component props need equality: identity is the shared event bus.
impl PartialEq for FieldBackend {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.events, &other.events)
    }
}

#[derive(Debug, Clone)]
pub struct NodeView {
    pub name: String,
    pub state: String,
    pub checkpoint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DecideView {
    pub intent: String,
    pub risk: String,
    pub confidence: f32,
    pub requires_reasoning: bool,
    pub approval: bool,
}

#[derive(Debug, Clone)]
pub struct DeviceView {
    pub name: String,
    pub platform: String,
    pub ip: String,
}

impl FieldBackend {
    pub fn boot() -> Self {
        let backend = Self {
            events: Arc::new(EventBus::new()),
            memory: MemoryStore::new(),
            mission: Arc::new(Mutex::new(Mission::new())),
            approvals: Arc::new(Mutex::new(ApprovalStore::new())),
            policy: Arc::new(Mutex::new(Policy::default())),
            telemetry: Telemetry::new(),
            discovery: Arc::new(Mutex::new(
                Discovery::new(DiscoveryConfig::default()).expect("http client"),
            )),
        };
        backend.seed();
        backend
            .telemetry
            .record("boot", "FIELD backend online", None, Some(true));
        backend.events.emit(Event::Heartbeat {
            from: "field".into(),
        });
        backend
    }

    fn seed(&self) {
        for (content, kind) in [
            (
                "FIELD command-center online. Missions, memory, approvals live.",
                MemoryType::Episodic,
            ),
            (
                "User prefers concise status briefs over verbose reports.",
                MemoryType::Preference,
            ),
            (
                "Goal: ship IGRIS v5 Phase 11 with all panels wired.",
                MemoryType::Goal,
            ),
        ] {
            let mut r = MemoryRecord::new(kind, content, "field-boot", "local");
            r.importance = 0.7;
            let _ = self.memory.store(r);
        }
        let mut mission = self.mission.lock().unwrap();
        let research = mission.add(TaskNode::new("Research panel data sources"));
        let implement = mission.add(TaskNode::new("implement").with_deps(&[research]));
        mission.add(TaskNode::new("verify panels").with_deps(&[implement]));
        mission.checkpoint(&research, "sources mapped");
        drop(mission);
        let mut approvals = self.approvals.lock().unwrap();
        approvals.request(
            "Enable Operate autonomy for low-risk tasks",
            PermissionLevel::L1Write,
            TimeDelta::minutes(30),
        );
    }

    // -- missions --------------------------------------------------------

    pub fn mission_progress(&self) -> u8 {
        self.mission.lock().unwrap().progress_pct()
    }

    pub fn mission_nodes(&self) -> Vec<NodeView> {
        let m = self.mission.lock().unwrap();
        m.order
            .iter()
            .filter_map(|id| {
                m.nodes.get(id).map(|n| NodeView {
                    name: n.name.clone(),
                    state: format!("{:?}", n.state),
                    checkpoint: n.checkpoint.clone(),
                })
            })
            .collect()
    }

    /// Complete the next ready task (demo driver + manual stepping).
    pub fn demo_step(&self) -> Option<String> {
        let next = {
            self.mission
                .lock()
                .unwrap()
                .ready_tasks()
                .into_iter()
                .next()
        };
        let id = next?;
        let name = {
            let mut m = self.mission.lock().unwrap();
            m.begin(&id).ok()?;
            let name = m.nodes.get(&id).map(|n| n.name.clone())?;
            m.record_success(&id, Some("stepped from FIELD"));
            name
        };
        self.telemetry
            .record("mission", &format!("completed {name}"), None, Some(true));
        self.events.emit(Event::TaskProgress {
            task_id: name.clone(),
            pct: self.mission_progress(),
        });
        Some(name)
    }

    // -- memory ----------------------------------------------------------

    pub fn memory_count(&self) -> usize {
        self.memory.count().unwrap_or(0)
    }

    pub fn memory_search(&self, query: &str) -> Vec<MemoryRecord> {
        match self.memory.search(query, 20) {
            Ok(hits) => {
                self.telemetry.record(
                    "memory",
                    &format!("search '{query}' -> {} hits", hits.len()),
                    None,
                    Some(true),
                );
                hits
            }
            Err(e) => {
                self.telemetry
                    .record("memory", &format!("search failed: {e}"), None, Some(false));
                vec![]
            }
        }
    }

    // -- approvals -------------------------------------------------------

    pub fn pending_approvals(&self) -> Vec<Approval> {
        self.approvals.lock().unwrap().pending()
    }

    pub fn approve(&self, id: &Uuid) -> bool {
        let ok = self.approvals.lock().unwrap().approve(id);
        self.telemetry
            .record("approval", &format!("approved {id}: {ok}"), None, Some(ok));
        self.events.emit(Event::ApprovalResolved {
            request_id: id.to_string(),
            approved: ok,
        });
        ok
    }

    pub fn deny(&self, id: &Uuid) -> bool {
        let ok = self.approvals.lock().unwrap().deny(id);
        self.telemetry
            .record("approval", &format!("denied {id}: {ok}"), None, Some(ok));
        self.events.emit(Event::ApprovalResolved {
            request_id: id.to_string(),
            approved: false,
        });
        ok
    }

    // -- reflex ----------------------------------------------------------

    pub fn decide_summary(&self, text: &str) -> DecideView {
        let d = igris_reflex::decide(text);
        self.telemetry.record(
            "reflex",
            &format!("intent={:?} conf={:.2}", d.intent, d.confidence),
            None,
            Some(true),
        );
        DecideView {
            intent: format!("{:?}", d.intent),
            risk: format!("{:?}", d.risk),
            confidence: d.confidence,
            requires_reasoning: d.requires_reasoning,
            approval: d.approval,
        }
    }

    // -- policy ----------------------------------------------------------

    pub fn autonomy(&self) -> Autonomy {
        self.policy.lock().unwrap().autonomy
    }

    pub fn set_autonomy(&self, autonomy: Autonomy) {
        self.policy.lock().unwrap().autonomy = autonomy;
        self.telemetry.record(
            "policy",
            &format!("autonomy -> {autonomy:?}"),
            None,
            Some(true),
        );
    }

    pub fn kill(&self) {
        self.policy.lock().unwrap().kill.engage();
        self.telemetry
            .record("policy", "kill-switch ENGAGED", None, Some(true));
        self.events.emit(Event::SystemWarning {
            msg: "kill-switch engaged".into(),
        });
    }

    pub fn revive(&self) {
        self.policy.lock().unwrap().kill.disengage();
        self.telemetry
            .record("policy", "kill-switch released", None, Some(true));
    }

    pub fn killed(&self) -> bool {
        self.policy.lock().unwrap().kill.is_engaged()
    }

    // -- observability ---------------------------------------------------

    pub fn timeline(&self, n: usize) -> Vec<TimelineEntry> {
        self.telemetry.recent(n)
    }

    pub fn event_totals(&self) -> (u64, u64) {
        let events = self.telemetry.totals();
        (events.0, events.1)
    }

    // -- devices ---------------------------------------------------------

    pub fn known_devices(&self) -> Vec<DeviceView> {
        self.discovery
            .lock()
            .unwrap()
            .known()
            .into_iter()
            .map(|d| DeviceView {
                name: d.name,
                platform: d.platform,
                ip: d.ip,
            })
            .collect()
    }

    pub fn local_ips(&self) -> Vec<String> {
        Discovery::local_ipv4s()
            .iter()
            .map(|ip| ip.to_string())
            .collect()
    }

    /// Bulk-merge sweep results (throwaway scanners merge here).
    pub fn merge_devices(&self, fresh: Vec<DiscoveredDevice>) -> Vec<DeviceView> {
        let mut d = self.discovery.lock().unwrap();
        d.merge_many(fresh);
        drop(d);
        self.known_devices()
    }

    /// Full LAN sweep on a throwaway scanner — no shared lock held across
    /// the network I/O. Returns the merged live device list.
    pub async fn run_lan_scan(&self) -> Vec<DeviceView> {
        let found = match Discovery::new(DiscoveryConfig::default()) {
            Ok(mut tmp) => tmp.scan().await,
            Err(e) => {
                self.telemetry.record(
                    "discovery",
                    &format!("scan client failed: {e}"),
                    None,
                    Some(false),
                );
                vec![]
            }
        };
        self.merge_devices(found)
    }

    // -- memory write ------------------------------------------------------

    pub fn memory_put(&self, content: &str) -> Option<String> {
        let text = content.trim();
        if text.is_empty() {
            return None;
        }
        let mut r = MemoryRecord::new(MemoryType::Episodic, text, "field-ui", "local");
        r.importance = 0.6;
        let id = self.memory.store(r).ok()?;
        self.telemetry
            .record("memory", "stored note from FIELD", None, Some(true));
        self.events.emit(Event::MemoryUpdated {
            kind: "episodic".into(),
        });
        Some(id.to_string())
    }

    // -- voice -------------------------------------------------------------

    /// Record `millis` ms from the default mic and judge speech content.
    /// Blocking — callers run it on a worker thread. Transcription itself
    /// needs the 10b neural model; this reports capture truthfully.
    pub fn record_ptt_blocking(&self, millis: u64) -> PttResult {
        use igris_voice::{capture::kennedy::record_blocking, Vad, VadConfig};
        match record_blocking(millis, 0) {
            Err(e) => PttResult {
                captured_ms: 0,
                samples: 0,
                speech: false,
                note: format!("no mic: {e}"),
            },
            Ok(s) => {
                let vad = Vad::new(VadConfig::default());
                let frame = igris_voice::AudioFrame {
                    samples: s.data.clone(),
                    sample_rate: s.sample_rate,
                };
                let speech = vad.is_speech_frame(&frame);
                PttResult {
                    captured_ms: millis,
                    samples: s.data.len(),
                    speech,
                    note: if speech {
                        "speech detected — transcription needs the 10b neural model".into()
                    } else {
                        "silence — nothing to transcribe".into()
                    },
                }
            }
        }
    }
}

/// Human one-liners for live bus events (timeline stream).
pub fn summarize_event(event: &igris_events::Event) -> String {
    use igris_events::Event::*;
    match event {
        UserInput { text } => format!("you: {text}"),
        VoiceDetected { transcript } => format!("heard: {transcript}"),
        DeviceOnline { device_id } => format!("device online: {device_id}"),
        DeviceOffline { device_id } => format!("device offline: {device_id}"),
        TaskStarted { task_id } => format!("started {task_id}"),
        TaskProgress { task_id, pct } => format!("{task_id} {pct}%"),
        TaskCompleted { task_id } => format!("done {task_id}"),
        TaskFailed { task_id, reason } => format!("{task_id} failed: {reason}"),
        ApprovalRequired { summary, .. } => format!("approval needed: {summary}"),
        ApprovalResolved {
            request_id,
            approved,
        } => {
            format!(
                "approval {request_id} -> {}",
                if *approved { "granted" } else { "denied" }
            )
        }
        ToolStarted { tool } => format!("tool {tool}…"),
        ToolCompleted { tool, ok } => format!("tool {tool} {}", if *ok { "ok" } else { "failed" }),
        MemoryUpdated { kind } => format!("memory updated ({kind})"),
        AgentCreated { agent } => format!("agent up: {agent}"),
        AgentFailed { agent, reason } => format!("agent {agent} failed: {reason}"),
        SystemWarning { msg } => format!("warning: {msg}"),
        Heartbeat { .. } => "heartbeat".into(),
    }
}

#[derive(Debug, Clone)]
pub struct PttResult {
    pub captured_ms: u64,
    pub samples: usize,
    pub speech: bool,
    pub note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_seeds_live_state() {
        let b = FieldBackend::boot();
        assert!(b.memory_count() >= 3);
        assert_eq!(b.mission_nodes().len(), 3);
        assert_eq!(b.pending_approvals().len(), 1);
        assert_eq!(b.mission_progress(), 0);
    }

    #[test]
    fn demo_step_advances_mission() {
        let b = FieldBackend::boot();
        assert_eq!(
            b.demo_step().as_deref(),
            Some("Research panel data sources")
        );
        assert_eq!(b.demo_step().as_deref(), Some("implement"));
        assert_eq!(b.demo_step().as_deref(), Some("verify panels"));
        assert_eq!(b.demo_step(), None);
        assert_eq!(b.mission_progress(), 100);
        assert!(b.timeline(10).iter().any(|e| e.kind == "mission"));
    }

    #[test]
    fn approvals_flow_with_events() {
        let b = FieldBackend::boot();
        let pending = b.pending_approvals();
        assert_eq!(pending.len(), 1);
        assert!(b.approve(&pending[0].id));
        assert!(b.pending_approvals().is_empty());
        assert!(!b.approve(&pending[0].id), "double approve fails");
    }

    #[test]
    fn memory_search_records_telemetry() {
        let b = FieldBackend::boot();
        let hits = b.memory_search("concise");
        assert!(!hits.is_empty());
        assert!(b.memory_search("zzz-no-match").is_empty());
    }

    #[test]
    fn decide_and_policy_roundtrip() {
        let b = FieldBackend::boot();
        let v = b.decide_summary("send the build to my laptop");
        assert_eq!(v.intent, "DeviceTransfer");
        assert!(!v.requires_reasoning);
        b.set_autonomy(Autonomy::Operate);
        assert_eq!(b.autonomy(), Autonomy::Operate);
        b.kill();
        assert!(b.killed());
        b.revive();
        assert!(!b.killed());
    }

    #[test]
    fn node_states_render() {
        let b = FieldBackend::boot();
        b.demo_step();
        let nodes = b.mission_nodes();
        assert_eq!(nodes[0].state, "Completed");
        assert!(nodes[0].checkpoint.is_some());
        assert_eq!(nodes[1].state, "Pending");
    }

    #[test]
    fn memory_put_stores_and_rejects_empty() {
        let b = FieldBackend::boot();
        let before = b.memory_count();
        assert!(b.memory_put("  ").is_none());
        let id = b.memory_put("field note from test").expect("stored");
        assert!(!id.is_empty());
        assert_eq!(b.memory_count(), before + 1);
        let hits = b.memory_search("field note from test");
        assert!(hits.iter().any(|r| r.content == "field note from test"));
    }

    #[test]
    fn merge_devices_joins_sweep() {
        use std::time::Instant;
        let b = FieldBackend::boot();
        assert!(b.known_devices().is_empty());
        let fresh = vec![DiscoveredDevice {
            id: "peer-9".into(),
            name: "peer".into(),
            platform: "linux".into(),
            ip: "192.168.1.20".into(),
            port: 53327,
            last_seen: Instant::now(),
        }];
        let merged = b.merge_devices(fresh);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "peer");
    }

    #[test]
    fn summarize_event_reads_clean() {
        assert_eq!(
            summarize_event(&igris_events::Event::TaskProgress {
                task_id: "t".into(),
                pct: 42
            }),
            "t 42%"
        );
        assert!(
            summarize_event(&igris_events::Event::Heartbeat { from: "x".into() })
                .contains("heartbeat")
        );
    }

    #[test]
    fn ptt_record_is_honest() {
        let b = FieldBackend::boot();
        let r = b.record_ptt_blocking(50);
        assert!(!r.note.is_empty());
        // Either hardware recorded frames or the error path said so.
        assert!(r.samples > 0 || r.captured_ms == 0);
    }
}
