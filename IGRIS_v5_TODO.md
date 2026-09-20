# IGRIS v5 — TODO / Task Board (CANONICAL)

> Canonical board: `igrisv5/IGRIS_v5_TODO.md` (replaces old `TODO.md`).
> Source of truth for build order. Clean reset: no predecessor code reuse.
> UI = Dioxus FIELD. Arch source = reference systems. Order: Memory → Reflex.

## Decisions locked

* [x] New workspace `igrisv5/` (not in-place refactor)
* [x] Dioxus native FIELD shell (not Sentinel, not Tauri/website)
* [x] Zero port from `igris/` predecessor (inventory only)
* [x] Subnet-scan + LocalSend-compat contract kept, clean re-implementation
* [x] Vertical slice order: Memory + Context first, then Reflex
* [x] Canonical todo file = `IGRIS_v5_TODO.md` (old `TODO.md` retired 2026-09-20)
* [ ] Confirm commercial licensing tiers (Free/Pro/Family/Enterprise) — open
* [ ] Confirm wake word `Arise` configurability — default `Arise`

## Phase 0 — Discovery (current)

* [x] Repo scaffold `igrisv5/` + workspace + docs
* [ ] `graphify update` + `query` baselines for new workspace
* [ ] Reference research notes: Jev / OpenHuman / OpenClaw / Harness / MCP / A2A / Agents-SDK
* [x] ADRs 01–02 in `.igris/engineering/ADR/` (clean-reset, Dioxus-FIELD)
* [ ] ADRs 03–12 (runtime, memory, reflex, router, tasks, tools, security, device, UI, licensing)
* [x] `SYSTEM_CONTRACTS.md` (events/memory/tool/agent/device/task)
* [ ] `SECURITY_MODEL.md`, `PERFORMANCE_BUDGET.md`, `UX_SYSTEM.md` (FIELD)
* [ ] Arch review gate sign-off

## Phase 1 — Core Runtime (DONE 2026-09-20)

* [x] `igris-events`: typed `Event` enum + `EventBus` (broadcast 1024, cancellable)
* [x] `igris-runtime`: `RuntimeConfig`, `NodeHandle`, shutdown watch, timeout helper
* [x] `igris-telemetry`: bounded timeline (1000), counters, no raw prompts
* [x] `igris-gateway`: daemon sessions/jobs/heartbeats skeleton
* [x] `igris-policy`: `Autonomy{Assist,Operate,Autonomous}`, `PermissionLevel L0-L4`, `check()` gate
* [x] `igris-memory`: `MemoryRecord` + `MemoryStore` in-mem + provenance + privacy (SQLite next)
* [x] `igris-context`: `prefetch` + `ContextBundle::compile` (dedupe+importance+budget)
* [x] `igris-reflex`: `decide()` heuristic fast path + escalation
* [x] `igris-router`: privacy-first `route()` + `ModelClass`
* [x] `igris-orchestrator`: `Mission` DAG + `checkpoint/resume_point/progress_pct`
* [x] `igris-device`: ports 53317/53318/53327/53328 + `DeviceInfo::compute_score`
* [x] `igris-tools`: `Tool` trait + `Registry` + 10k bounded audit
* [x] `igris-agents`: `AgentDef{capabilities,allowed_tools,tier}` + least-privilege
* [x] `cargo check --workspace` green
* [x] `cargo test --workspace` green — 17 unit tests passed, 0 failed
* [x] `cargo fmt --all` clean, `cargo clippy --workspace -- -D warnings` clean
* [ ] Arch review: no globals, no god objects, no blocking-in-async (self-check passed, formal review open)

## Phase 2 — Context + Memory (DONE 2026-09-20)

* [x] `igris-memory`: SQLite (`rusqlite` bundled) + `memories` table + FTS5 + triggers + `edges` + indexes
* [x] TTL/`expires_at` filter + `forget_expired` sweeper + soft-delete/restore tombstones + LWW `upsert`
* [x] `EmbeddingProvider` trait + `SimpleEmbeddingProvider` (64-d hash stub) + `cosine_similarity` + `rank_memories` hybrid score
* [x] `igris-context`: `prefetch` returns `Result` + `ContextBundle::compile` (dedupe+importance+budget)
* [x] Provenance end-to-end at storage layer: `provenance{source,device_id,agent,tool,model,confidence}` JSON-persisted + roundtrip-tested
* [x] Persist `privacy` (`local_only/hybrid/cloud_allowed`), `may_sync()` gate (`LocalOnly` never syncs)
* [x] Tests: 8 memory (`store_search_delete`, `privacy_provenance`, `ttl_expiry`, `lww`, `edges_neighbors`, `export_import`, `ranked_search`, `forget_by_type`) + 1 context — all green
* [x] `cargo check/test/fmt/clippy` green (24 unit tests, 0 failed)

## Phase 3 — Reflex (DONE 2026-09-20)

* [x] `igris-reflex`: typed `Intent{DeviceTransfer,DeviceStatus,MemoryQuery,ToolExec,Communication,RiskyOp,General}` + `Risk{Low,Medium,High,Critical}` + `ModelHint` + `ReflexDecision{+model_hint,cached}`
* [x] Intent + risk + tool + model-hint + device selection; risky-first ordering (destructive/external-send never fast-pathed)
* [x] `EscalationPolicy{min_confidence 0.6, approval_from High}`: low-confidence → reasoning, High+ → approval+reasoning, Critical → ReasoningCloud
* [x] `DecisionCache` bounded LRU (cap 256, hit/miss stats, `cached` flag)
* [x] Confidence calibration: base per rule + 0.05/extra-signal, cap 0.97
* [x] Latency budget test: 3000 decisions, each <100ms, avg <5ms (real: ~10us)
* [x] Tests: 10 reflex (transfer/status/risky/external-send/memory/gibberish/normalization/cache-hit/cache-bound/latency) — all green
* [x] Workspace: 32 unit tests green, clippy clean, fmt applied

## Phase 4 — Model Router (DONE 2026-09-20)

* [x] `ModelInfo{id,class,max_context,cost_per_1k,supports_vision,supports_reasoning}` + `ModelProvider{info,is_available,generate}` trait
* [x] `LocalEchoProvider{timy,small}` always-available offline (deterministic echo stub; real inference later in `services/model-service`)
* [x] `CloudStubProvider{fast_cloud,reasoning_cloud}` env-key gated (`is_available` checks key; transport explicitly not wired yet)
* [x] `ModelRouter{providers,order,stats,policy}`: privacy gates (Confidential+ never cloud), capability/network filters, score = class-match + 0.6*success_rate + latency bonus + local bias
* [x] `RouteStats{successes,failures,total_latency_ms}` — aggregate counters only, neutral 0.5 prior; `record_feedback()` takes no prompts (no private-data training)
* [x] Budget: `RoutingPolicy{prefer_local,allow_cloud,max_cost_per_request}` + `estimate_cost()` + ranked walk picks first affordable; `BudgetExceeded` error otherwise
* [x] Fallback chains: ordered by score, local-echo sorted last as last resort
* [x] Tests: 11 router (static fast path x2, offline-local, confidential-never-cloud, budget-free-only, adaptive-preference, fallback-ends-local, env-gate, echo-generates, vision-no-provider, stats-math) — all green
* [x] Workspace: 41 unit tests green, clippy clean, fmt applied

## Phase 5 — Orchestrator (DONE 2026-09-20)

* [x] `TaskNode{depends_on,timeout_ms,requires_verification,verified,last_error,retry_delay}` + `MissionState{Running,Paused,Cancelled,Completed,Failed}`
* [x] Dependency-aware `ready_tasks()` (pending + parents done + mission running); `begin()` enforces deps + running state
* [x] Retry: `record_failure()` requeues while `attempts <= max_retries` else Failed; backoff 1s/2s/4s… cap 30s; mission Failed on node failure
* [x] `pause/resume/cancel` — paused/cancelled missions yield no ready tasks; cancel tombstones pending/running nodes
* [x] Verification gate: `requires_verification` nodes complete only after `verify_node()`; `is_complete()` checks all done
* [x] Timeouts: `is_timed_out(started,now)` + async `run_with_timeout()` (tokio deadline) for executors
* [x] Durability: `to_json/from_json` roundtrip (states + checkpoints); `resume_point()` dep-aware; `reclaim_running()` releases dead-process leases after crash
* [x] Tests: 11 orchestrator (deps, multi-dep, retry-fail, backoff, pause/cancel, verify-gate, persistence, reclaim, timeout, async-deadline, resume) — all green
* [x] Workspace: 51 unit tests green, clippy clean, fmt applied

## Phase 6 — Agents + Skills (DONE 2026-09-20)

* [x] `AgentRole{Planner,Coder,Researcher,Developer,DevOps,Communicator,Assistant}` + `AgentDef{name,role,capabilities,allowed_tools,tier,max_iterations,system_prompt}` — 7 shells, least-privilege (no role gets `gmail.send`)
* [x] `AgentTask/AgentResult/AgentContext` + `execute_task()` scope-validating deterministic planner (LLM execution deferred to reasoning pipeline — no fake results)
* [x] `Assistant` front door (tier 1): talks to the user, clarifies intent, hands work to the orchestrator for routing to workers; never touches worker tools; presents results back
* [x] Tiers: workers (tier 2) cannot delegate; `Handoff` + `can_handoff()` (different target + non-empty reason + delegating source)
* [x] `a2a` module: `AgentCard/AgentTaskRequest/Artifact` + `validate_task()` (advertised-capability + non-empty input) + `make_artifact/verify_artifact()` (known-producer + hash check)
* [x] New `igris-skills` crate: `SkillManifest` validation (kebab-case, tools/steps, L0-L4, validation text, explicit `cautions`), `SkillRegistry{register/enable/disable/record_use/record_success/promotion_candidates/approval_gated}` + JSON manifests
* [x] JARVIS library: 18 skills in `skills/*.json` (status, research, translate, errors, meeting-prep, reminders, organizer, backup, net-diag, code-review, lockdown-check, open-app, run-tests, clean-temp, restart-service[L3], deploy-service[L3-L4]) — every skill carries cautions; L3+ approval-gated; guard test loads all 18 through the real validator
* [x] Tests: 10 agents + 7 skills — all green
* [x] Workspace: 67 unit tests green, clippy clean, fmt applied

## Phase 7 — Tools (DONE 2026-09-20)

* [x] `Tool{definition,validate,run}` + `ToolDef{+version,risk,timeout_ms}` + `RiskLevel{None..Critical}` (Medium+ needs approval) + `ToolError{Denied,BadArgs,Timeout,Exec}`
* [x] `FilesystemTool{root}`: root-confined read/list/write via JSON args; canonical root, lexical `..`/absolute rejection; 64KB read cap, 8KB output truncation
* [x] `TerminalTool`: 18-pattern denylist (`rm -rf /`, `mkfs`, fork bombs, raw-disk, pipe-to-shell…), shell exec with captured output; `High` risk + L2
* [x] `BrowserTool`: http(s) URL validation now, honest `Phase 9` deferral for transport (never fakes a fetch)
* [x] Registry: validate-before-run on both paths; `execute_async` runs `run()` on the blocking pool with per-tool timeout (default 30s); bounded 10k audit + `success_rate`
* [x] `Catalog` lazy factory registry: defs visible before load, instantiate-once on first use; `definitions_for()` merges factories + loaded (model sees only its allowed set)
* [x] Tests: 10 tools (roundtrip, risk-gate, fs-roundtrip, traversal-blocked, terminal echo/denied, async timeout cap, async success, browser deferral, filter, lazy-once) — all green
* [x] Workspace: 83 tests green (82 unit + 1 library-guard integration), clippy `--all-targets -D warnings` clean, fmt applied

## Phase 8 — Security (DONE 2026-09-20)

* [x] `Policy{autonomy,max_auto_level,kill}` + `KillSwitch` (shared across clones; engaged denies everything incl. reads and live approvals)
* [x] `ApprovalStore{request/approve/deny/pending/is_approved}` with TTL expiry (stale approvals auto-`Expired`, decided ones immutable, unknown IDs safe no-ops)
* [x] `AuditLog` hash-chained entries (`record` + `verify_chain` detects tamper; non-crypto hash stub, HMAC-SHA256 in hardening)
* [x] New `igris-sandbox` crate: `Workspace{name,allowed_roots,denied_commands,network_allowed,allowed_env,timeout_ms,max_output}` (`confine/check_command/check_network/filter_env/cap_output`) + `SecretsVault{set/reveal/redact}` (values never logged; `redact()` scrubs logs/output)
* [x] Failure-injection chain test (`security_chain`, 5 tests): destructive cmd denied at sandbox + tool + kill layers; path escape blocked pre-tool; stale approval denies; leaked secret redacted from error logs; offline workspace refuses network tools
* [x] Tests: 7 policy + 6 sandbox + 5 chain — all green
* [x] Workspace: 98 tests green, clippy `--all-targets -D warnings` clean, fmt applied

## Phase 9 — Device Mesh (9a + 9b DONE 2026-09-20)

* [x] `identity`: stable id + secret, SHA-256 `fingerprint()`, per-peer `pairing_key()` (secret never on wire)
* [x] `trust`: TOFU `verify_or_pin()` (mismatch = hard MITM error, old pin survives), revoke, JSON export/import
* [x] `pairing`: 6-digit OTP, 120s TTL, 3-attempt burn, expiry sweep
* [x] `message`: versioned `EcoMessage` + HMAC-SHA256 sign/verify; rejects bad sig, >5min skew, version mismatch
* [x] `transfer`: prepare→approve/deny→token-gated chunked upload→SHA-256 `complete()`; 1MB chunk bitsets + `missing_chunks()` resume; TTL sweeps
* [x] `discovery`: local /24 enumeration, 32-way concurrent HTTP probes, 120s-stale prune
* [x] `server` (axum): `GET info`, `POST prepare/confirm/deny`, token-gated uploads, checksum `complete`, per-session SSE `events/:session` (broadcast, lag-tolerant, keep-alive)
* [x] `tls` (9b): rcgen self-signed certs, ring-only rustls proxy (TLS→local HTTP), `fingerprint_der` pins, `require_pin` TOFU gate (MITM fails before any socket opens); pinned HTTPS loop proven live
* [x] `store` (9b): `identity.json` + `trusted_devices.json` under exe-relative `pkg/ecosystem`, atomic writes, load-or-create identity, corrupt files reported (keyring migration tracked)
* [x] `sync` (9b): generic LWW merge `(updated_ms, device_id)` with tombstones, deterministic tie-break, idempotent redelivery, `delta_since` exchange
* [x] Tests: 37 device (identity/trust/OTP/envelope/sessions/discovery/TLS/persistence/sync/server/SSE) — all green
* [x] Workspace: 164 tests green, clippy `--all-targets -D warnings` clean, fmt applied
* [x] FIELD window titled "IGRIS FIELD" (1440×860) + software-render flags baked in

## Phase 10 — Voice / Multimodal (10a + 10b-core DONE 2026-09-20)

* [x] `igris-voice`: `AudioFrame` (16kHz frames, RMS + zero-crossing metrics, synthetic tone/silence builders for tests)
* [x] `Vad` (energy gate + ZCR band + hangover segmentation) + `VadState{Idle,Speaking}`
* [x] `WakeMatcher::arise()` (exact + Levenshtein≤2, configurable phrase)
* [x] `SpeechToText` / `TextToSpeech` traits + `ScriptStt` (queued transcripts) + `RecordingTts` (records + cancel)
* [x] `VoicePipeline`: VAD → segment → STT → wake gate → reflex (never an LLM on fast path) → short ack; risky/general escalates silently
* [x] Barge-in: fresh speech over live synthesis cancels it (`SpeakingCancelled`); explicit `interrupt()` safe when idle
* [x] `capture` (10b): cpal `list_mics()` (headless-safe) + `record_blocking()` (mono f32, F32/I16/U16, honest NoDevice); live mic detected on dev machine
* [x] `tts` (10b): `PiperTts` (process CLI shape, spawn + kill-cancel, utterance log) + `auto_detect()` backend status; proven with echo stand-in
* [x] Models provisioned (2026-09-20, `models/`, git-ignored, `MODELS.md` manifest): Piper engine + Lessac voice LIVE (3s audio in 0.2s); SenseVoice-int8 + MiniLM ONNX on disk with seams ready (`SpeechToText`, `EmbeddingProvider`); `PiperTts::bundled()` resolver
* [x] `igris-cli voice-status`: live mic/TTS/wake report (runs headless)
* [x] `igris-vision`: `VisionGate::should_invoke()` (pixels + reason required — text stays text), `ImageDescriber` seam + honest `NullDescriber`, serializable `VisionRequest`
* [x] Tests: 17 voice + 3 vision — all green
* [x] Workspace: 176 tests green, clippy `--all-targets -D warnings` clean, fmt applied
* [ ] Queued: neural STT wiring (`sherpa-onnx` adapter — model already on disk), OS-native TTS (SAPI/WinRT), acoustic wake model, local reasoning LLM (no runner integrated — decide before downloading GBs)

## Phase 11 — FIELD UX (11a + 11b DONE 2026-09-20)

* [x] `apps/field-dioxus` (`igris-field`, Dioxus 0.7 desktop): title bar with LIVE/KILLED presence + tick age, icon rail (7 tabs, approvals badge), center panels, status strip — dark slate + status green per `ui-ux-pro-max` ops-desk direction
* [x] `FieldBackend` on live cores only: seeded memory (3 notes), 3-node demo mission, 1 pending approval; mission progress/stepping, memory search + telemetry, approve/deny with events, reflex summaries, autonomy switch, kill-switch engage/release, discovery + local IPs
* [x] Panels: Overview (mission/memory/approvals/events cards), Missions (states + checkpoints + step button), Memory (live search + Remember write), Approvals (approve/deny), Timeline (live event stream), Devices (interfaces + peers + SCAN LAN), Settings (autonomy + kill-switch)
* [x] 11b: event-driven Timeline (bus subscription + telemetry seed, newest-last, capped), throwaway-scanner LAN sweep (no shared lock across I/O), memory write path, title-bar PTT (2s capture + VAD verdict, honest about neural STT)
* [x] Pausable 1s live tick; empty states on every thin panel; keyboard-focusable buttons
* [x] `apps/cli` (`igris-cli`): `decide/memory-put/memory-find/mission-demo/approve-demo/timeline-demo/voice-status` — proven live
* [x] Tests: 10 backend + 6 CLI — all green
* [x] Workspace: 176 tests green, clippy `--all-targets -D warnings` clean, fmt applied

## Phase 12 — Eval / Hardening (DONE 2026-09-20, conditional pass — see review)

* [x] Budget gates as tests: reflex <100ms, memory-500 <150ms, context-500 <300ms, route <10ms, mission-200 <50ms (`benchmarks/BUDGETS.md`)
* [x] Regression corpus: 13-case `tests/regression_corpus.json` + guard test (fails loudly on behavior drift)
* [x] Supply chain: `deny.toml` (GPL-family denied); `cargo metadata` inventory — zero GPL, r-efi dual-license consumed as MIT/Apache, all 19 first-party crates stamped `Proprietary`
* [x] Release: `docs/RELEASE.md` (SemVer, channels, pre-release gate, committed lockfile); `cargo build --workspace --release` green (2m45s)
* [x] Eval review: `.igris/engineering/REVIEWS/phase-12-eval.md` — suite inventory, failure-injection coverage, CONDITIONAL PASS with explicit beta blockers
* [x] Workspace: 151 tests green, clippy `--all-targets -D warnings` clean, fmt applied
* [ ] Queued (tracked, not silent): 9b TLS/sync, 10b neural audio, 11b SSE stream, HMAC audit, keyring secrets, OS sandbox, fuzz targets, CI deny+audit, signing certs

## Definition of Done (per feature)

`impl + tests + errors + security review + perf validation + observability + docs + integration`

---

**BUILD COMPLETE.** All 12 phases are DONE (9b/10b/11b queued as explicit follow-ups).
Next: beta blockers above, or new directives.

## Immediate next commands

```bash
cd igrisv5
cargo check --workspace
cargo test --workspace
```

## Open questions

* Commercial tiers still wanted? (old ecosystem plan had Free/Pro/Family/Enterprise)
* mDNS required for v1 or LAN scan enough?
* Which local models day-1 (tiny/small/reflex/embedding)?
* Mobile shims in v1 scope or deferred?
