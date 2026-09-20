//! igris-context — SuperContext prefetch + bounded compiler.

use igris_memory::{MemoryError, MemoryRecord, MemoryStore};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBundle {
    pub items: Vec<MemoryRecord>,
    pub budget_chars: usize,
    pub used_chars: usize,
}

impl ContextBundle {
    pub fn compile(mut records: Vec<MemoryRecord>, budget_chars: usize) -> Self {
        // Deduplicate by id, sort by importance, truncate to budget.
        records.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        records.dedup_by(|a, b| a.id == b.id);
        let mut used = 0;
        let mut items = Vec::new();
        for r in records {
            let len = r.content.len();
            if used + len > budget_chars {
                continue;
            }
            used += len;
            items.push(r);
        }
        Self {
            items,
            budget_chars,
            used_chars: used,
        }
    }
}

pub fn prefetch(
    store: &MemoryStore,
    query: &str,
    limit: usize,
    budget_chars: usize,
) -> Result<ContextBundle, MemoryError> {
    let hits = store.search(query, limit)?;
    Ok(ContextBundle::compile(hits, budget_chars))
}

#[cfg(test)]
mod tests {
    use super::*;
    use igris_memory::{MemoryRecord, MemoryType};
    #[test]
    fn budget_respected() {
        let s = MemoryStore::open_in_memory().unwrap();
        for i in 0..5 {
            s.store(MemoryRecord::new(
                MemoryType::Episodic,
                &format!("memory item number {i} dioxus"),
                "t",
                "d",
            ))
            .unwrap();
        }
        let b = prefetch(&s, "dioxus", 5, 40).unwrap();
        assert!(b.used_chars <= 40);
    }

    #[test]
    fn budget_compile_500_records_under_300ms() {
        use std::time::Instant;
        let recs: Vec<MemoryRecord> = (0..500)
            .map(|i| {
                let mut r = MemoryRecord::new(
                    MemoryType::Episodic,
                    &format!("context item {i} with some body text for sizing"),
                    "t",
                    "d",
                );
                r.importance = ((500 - i) % 10) as f32 / 10.0;
                r
            })
            .collect();
        let t0 = Instant::now();
        let b = ContextBundle::compile(recs, 4000);
        let elapsed = t0.elapsed();
        assert!(b.used_chars <= 4000);
        assert!(!b.items.is_empty());
        assert!(
            elapsed.as_millis() < 300,
            "compile took {elapsed:?}, budget 300ms"
        );
    }
}
