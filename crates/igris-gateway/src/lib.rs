//! igris-gateway — persistent daemon: sessions, devices, jobs, heartbeats.

use igris_events::{Event, EventBus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub device_id: String,
}

#[derive(Debug, Default)]
pub struct Gateway {
    bus: Arc<Mutex<Option<Arc<EventBus>>>>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}

impl Gateway {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach_bus(&self, bus: Arc<EventBus>) {
        *self.bus.lock().unwrap() = Some(bus);
    }

    pub fn register_session(&self, id: &str, device_id: &str) {
        self.sessions.lock().unwrap().insert(
            id.into(),
            Session {
                id: id.into(),
                device_id: device_id.into(),
            },
        );
        if let Some(bus) = self.bus.lock().unwrap().as_ref() {
            bus.emit(Event::AgentCreated { agent: id.into() });
        }
    }

    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Heartbeat loop — cancellable via shutdown channel in real daemon.
    pub async fn heartbeat_forever(&self, from: String, every: Duration) {
        let mut ticker = tokio::time::interval(every);
        loop {
            ticker.tick().await;
            if let Some(bus) = self.bus.lock().unwrap().as_ref() {
                bus.emit(Event::Heartbeat { from: from.clone() });
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_register() {
        let g = Gateway::new();
        g.attach_bus(Arc::new(EventBus::new()));
        g.register_session("s1", "laptop");
        assert_eq!(g.session_count(), 1);
    }
}
