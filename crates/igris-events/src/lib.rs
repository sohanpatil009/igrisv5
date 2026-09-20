//! igris-events — typed event bus for IGRIS v5.
//! Bounded broadcast, no globals, cancellable subscriptions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

pub const BUS_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub id: Uuid,
    pub at: DateTime<Utc>,
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    UserInput { text: String },
    VoiceDetected { transcript: String },
    DeviceOnline { device_id: String },
    DeviceOffline { device_id: String },
    TaskStarted { task_id: String },
    TaskProgress { task_id: String, pct: u8 },
    TaskCompleted { task_id: String },
    TaskFailed { task_id: String, reason: String },
    ApprovalRequired { request_id: String, summary: String },
    ApprovalResolved { request_id: String, approved: bool },
    ToolStarted { tool: String },
    ToolCompleted { tool: String, ok: bool },
    MemoryUpdated { kind: String },
    AgentCreated { agent: String },
    AgentFailed { agent: String, reason: String },
    SystemWarning { msg: String },
    Heartbeat { from: String },
}

impl Envelope {
    pub fn new(event: Event) -> Self {
        Self {
            id: Uuid::new_v4(),
            at: Utc::now(),
            event,
        }
    }
}

#[derive(Debug)]
pub struct EventBus {
    tx: broadcast::Sender<Envelope>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Envelope> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: Event) -> usize {
        let env = Envelope::new(event);
        self.tx.send(env).unwrap_or(0)
    }

    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emit_reaches_subscriber() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.emit(Event::Heartbeat {
            from: "test".into(),
        });
        let got = rx.recv().await.expect("event");
        assert!(matches!(got.event, Event::Heartbeat { .. }));
    }

    #[tokio::test]
    async fn task_lifecycle_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.emit(Event::TaskStarted {
            task_id: "t1".into(),
        });
        bus.emit(Event::TaskProgress {
            task_id: "t1".into(),
            pct: 50,
        });
        bus.emit(Event::TaskCompleted {
            task_id: "t1".into(),
        });
        for _ in 0..3 {
            rx.recv().await.expect("event");
        }
    }
}
