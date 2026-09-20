//! igris-telemetry — counters + bounded debug timeline. Privacy-respecting (no raw prompts).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub at: DateTime<Utc>,
    pub kind: String,
    pub summary: String,
    pub latency_ms: Option<u64>,
    pub ok: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct Telemetry {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    timeline: VecDeque<TimelineEntry>,
    events_total: u64,
    errors_total: u64,
}

impl Telemetry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, kind: &str, summary: &str, latency_ms: Option<u64>, ok: Option<bool>) {
        let mut g = self.inner.lock().unwrap();
        if g.timeline.len() >= 1000 {
            g.timeline.pop_front();
        }
        g.timeline.push_back(TimelineEntry {
            at: Utc::now(),
            kind: kind.into(),
            summary: summary.chars().take(240).collect(),
            latency_ms,
            ok,
        });
        g.events_total += 1;
        if ok == Some(false) {
            g.errors_total += 1;
        }
    }

    pub fn recent(&self, n: usize) -> Vec<TimelineEntry> {
        let g = self.inner.lock().unwrap();
        g.timeline.iter().rev().take(n).cloned().collect()
    }

    pub fn totals(&self) -> (u64, u64) {
        let g = self.inner.lock().unwrap();
        (g.events_total, g.errors_total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_timeline() {
        let t = Telemetry::new();
        for i in 0..1200 {
            t.record("test", &format!("e{i}"), None, Some(true));
        }
        assert_eq!(t.recent(2000).len(), 1000);
        assert_eq!(t.totals().0, 1200);
    }
}
