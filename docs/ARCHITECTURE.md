# IGRIS v5 — Architecture (FIELD, clean reset)

## Non-negotiable loop

Perception → Context → Reflex → Memory → Routing → Reasoning → Planning →
Orchestration → Policy → Execution → Verification → Learning.

## Crate ownership (narrow, no god objects)

* `igris-events`: `Event` enum + `EventBus(broadcast 1024)` — UI/agents/gateway consume events, never direct calls.
* `igris-runtime`: `RuntimeConfig`, `NodeHandle`, shutdown watch, timeout helper. Binary owns `#[tokio::main]`.
* `igris-gateway`: persistent daemon — sessions, jobs, heartbeats. Trusted-gateway / untrusted-exec split.
* `igris-telemetry`: bounded timeline (1000), counters, no raw prompts.
* `igris-memory`: SQLite (`rusqlite` bundled) + FTS5 + edges graph + TTL/tombstone/LWW + persisted provenance/privacy + `EmbeddingProvider` hybrid rank.
* `igris-context`: `prefetch` + `ContextBundle::compile` (dedupe + importance + char budget).
* `igris-reflex`: `Reflex{policy,cache}` engine — typed `Intent`/`Risk`/`ModelHint`, calibrated confidence, `EscalationPolicy`, bounded cache (256), `<100ms` budget. Fast path never touches LLM.
* `igris-router`: static `route()` fast path + adaptive `ModelRouter` (registry, aggregate-only stats, budgets, ordered fallbacks ending local). Cloud stubs env-gated; real inference later in `services/model-service`.
* `igris-orchestrator`: dep-aware durable missions — retries + backoff, timeouts, pause/resume/cancel, verification gates, JSON persistence, crash `resume_point()` + lease reclaim. Never restart-from-scratch.
* `igris-agents`: 7 role shells (least-privilege, tiered), scope-checked deterministic planning, handoffs, `a2a` card/task/artifact contracts with validation. `Assistant` is the front door — user talk + clarify, orchestrator routes to workers. LLM execution arrives with the reasoning pipeline.
* `igris-skills`: validated JSON manifests + registry with enable/disable, use/success tracking, promotion candidates. Example in `skills/`.
* `igris-policy`: autonomy gates + TTL approvals + hash-chained audit + shared kill-switch (denies all, beats approvals).
* `igris-sandbox`: workspace confinement (roots/commands/network/env/caps) + secret vault with redaction. OS isolation deferred to hardening; boundary contract established.
* `igris-device`: identity/TOFU/OTP trust, HMAC envelopes, token-gated transfer sessions with checksum resume, concurrent subnet discovery, axum service (info/handshake/uploads/SSE), ring-only TLS proxy with pin gate, atomic identity/trust store, LWW sync engine.
* `igris-tools`: validate-then-run runtime — root-confined filesystem, denylisted terminal (High risk), browser URL gate (transport Phase 9); async timeouts on the blocking pool; lazy `Catalog` + capability filtering. MCP JSON-RPC transport deferred to Phase 9.
* `igris-voice`: real VAD + fuzzy Arise matcher + trait-seamed STT/TTS + barge-in pipeline over reflex (no LLM on fast path). cpal capture + process TTS + auto-detect live. Neural STT / OS-native TTS / acoustic wake queued.
* `igris-vision`: invoke gate (pixels + reason) + describer seam. Multimodal on a budget.
* `apps/field-dioxus`: ops-desk shell on `FieldBackend` (live cores, pausable tick, empty states). Timeline streams the event bus live; LAN scan runs on throwaway scanners; memory write + 2s PTT capture wired. Titled "IGRIS FIELD".
* `apps/cli`: headless debug commands over the same cores (`decide/memory-put/memory-find/mission-demo/approve-demo/timeline-demo/voice-status`).

## Dependency rules

`events` leaf. `runtime` → events. `tools` → policy. `agents` → tools+memory.
`context` → memory. `gateway` → events+runtime. `orchestrator` standalone.
No cycles. No `std::Mutex` across `.await` (use `tokio::sync`). No unbounded channels.

## What was NOT inherited

No port from predecessor. No globals, no `IgrisBrain/Everything`, no vendored duplicates,
no hash-embeddings as real vectors, no faked streaming, no placeholder TOFU.
