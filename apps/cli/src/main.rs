//! igris-cli — headless debug surface for IGRIS v5.
//! Same cores the FIELD shell reads; printable in a terminal, scriptable,
//! and runnable where no display exists. No display, no Dioxus.

use igris_events::{Event, EventBus};
use igris_memory::{MemoryRecord, MemoryStore, MemoryType};
use igris_orchestrator::{Mission, TaskNode};
use igris_policy::{ApprovalStore, PermissionLevel};
use igris_reflex as reflex;
use igris_telemetry::Telemetry;
use std::sync::Arc;

const USAGE: &str = "\
igris-cli — IGRIS v5 headless debug
  decide <text...>     reflex decision (intent/risk/confidence)
  memory-put <text>    store an episodic memory (prints id)
  memory-find <query>  search memories (prints matches)
  mission-demo         run a 3-node mission to completion (prints progress)
  approve-demo         request -> approve -> expiry lifecycle demo
  timeline-demo        record spans and print the debug timeline
";

fn main() {
    std::process::exit(run(std::env::args().skip(1).collect()));
}

fn run(args: Vec<String>) -> i32 {
    if args.is_empty() {
        print!("{USAGE}");
        return 2;
    }
    match args[0].as_str() {
        "decide" => {
            let text = args[1..].join(" ");
            if text.trim().is_empty() {
                eprintln!("decide: empty text");
                return 2;
            }
            println!("{}", decide_json(&text));
            0
        }
        "memory-put" => {
            let text = args[1..].join(" ");
            let store = MemoryStore::new();
            let id = store
                .store(MemoryRecord::new(
                    MemoryType::Episodic,
                    &text,
                    "cli",
                    "local",
                ))
                .unwrap();
            println!("{id}");
            0
        }
        "memory-find" => {
            let store = seed_store();
            let hits = store.search(&args[1..].join(" "), 10).unwrap();
            for h in &hits {
                println!("[{:?}] {}", h.kind, h.content);
            }
            println!("{} hit(s)", hits.len());
            0
        }
        "mission-demo" => {
            println!("{}", mission_demo());
            0
        }
        "approve-demo" => {
            println!("{}", approve_demo());
            0
        }
        "timeline-demo" => {
            let t = Telemetry::new();
            let bus = Arc::new(EventBus::new());
            t.record("cli", "demo start", None, Some(true));
            bus.emit(Event::TaskStarted {
                task_id: "demo".into(),
            });
            t.record("cli", "demo end", Some(3), Some(true));
            for e in t.recent(5) {
                println!("[{}] {} — {}", e.at.format("%H:%M:%S"), e.kind, e.summary);
            }
            0
        }
        _ => {
            eprint!("unknown command\n{USAGE}");
            2
        }
    }
}

fn decide_json(text: &str) -> String {
    let d = reflex::decide(text);
    serde_json::json!({
        "intent": format!("{:?}", d.intent),
        "tool": d.tool,
        "target_device": d.target_device,
        "requires_reasoning": d.requires_reasoning,
        "risk": format!("{:?}", d.risk),
        "approval": d.approval,
        "memory_scope": d.memory_scope,
        "confidence": d.confidence,
    })
    .to_string()
}

fn seed_store() -> MemoryStore {
    let store = MemoryStore::new();
    for text in [
        "FIELD shell boots with live backends",
        "user prefers concise briefs",
        "router keeps secrets local",
    ] {
        let _ = store.store(MemoryRecord::new(
            MemoryType::Semantic,
            text,
            "cli-seed",
            "local",
        ));
    }
    store
}

fn mission_demo() -> String {
    let mut m = Mission::new();
    let a = m.add(TaskNode::new("research"));
    let b = m.add(TaskNode::new("implement").with_deps(&[a]));
    let c = m.add(TaskNode::new("verify").with_deps(&[b]));
    let mut log = Vec::new();
    while let Some(id) = m.ready_tasks().into_iter().next() {
        m.begin(&id).unwrap();
        let name = m.nodes.get(&id).unwrap().name.clone();
        m.record_success(&id, Some("cli-demo"));
        log.push(format!("{name} -> {}%", m.progress_pct()));
    }
    let _ = c;
    serde_json::json!({"steps": log, "progress": m.progress_pct(), "complete": m.is_complete()})
        .to_string()
}

fn approve_demo() -> String {
    let mut store = ApprovalStore::new();
    let id = store.request(
        "cli demo approval",
        PermissionLevel::L1Write,
        chrono::TimeDelta::minutes(5),
    );
    let approved = store.approve(&id);
    let still_pending = store.pending().len();
    serde_json::json!({"approved": approved, "pending_after": still_pending}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_json_carries_intent() {
        let out = decide_json("send the build to my laptop");
        assert!(out.contains("DeviceTransfer"));
        assert!(out.contains("fastswap"));
    }

    #[test]
    fn mission_demo_completes() {
        let out = mission_demo();
        assert!(out.contains("\"complete\":true"));
        assert!(out.contains("\"progress\":100"));
    }

    #[test]
    fn approve_demo_flow() {
        let out = approve_demo();
        assert!(out.contains("\"approved\":true"));
    }

    #[test]
    fn unknown_command_usage() {
        assert_eq!(run(vec!["nope".into()]), 2);
        assert_eq!(run(vec![]), 2);
    }

    #[test]
    fn cli_needs_chrono_dep_note() {
        // chrono is used for approval TTLs; this pins the dependency.
        let _ = chrono::Utc::now();
    }
}
