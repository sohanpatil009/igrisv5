# protocols/events — versioned event envelope (v0.1)

Mirrors `igris-events::Envelope`. JSON example:

```json
{"id":"uuid","at":"2026-09-20T00:00:00Z","event":{"TaskProgress":{"task_id":"t1","pct":50}}}
```

Compatibility: additive variants only. Consumers ignore unknown variants (log + count).
