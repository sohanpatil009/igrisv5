# IGRIS v5 — Personal AI Operating System

> Clean architectural reset. No code ported from `igris/` / `igris-ecosystem/` predecessor.
> Those repos are lessons-learned / feature-inventory only. Architecture is derived
> from the reference systems in the v5 spec (Jev/System-One, OpenHuman, OpenClaw,
> Harness, MCP, A2A, OpenAI Agents patterns). UI is Dioxus native — **IGRIS FIELD**,
> not Sentinel, not a website.

## 0. What this is

Production-grade personal AI ecosystem / AI OS, not a chatbot demo:

* intelligent, fast, responsive, persistent, proactive, multimodal, agentic
* secure, observable, extensible, cross-platform, device-aware, privacy-first
* local / remote / hybrid per user policy, offline-first
* maintainable for years

The LLM is ONE component. Intelligence comes from the system around the models.

## 1. Cognitive loop (non-negotiable)

```text
Perception
  -> Context (prefetch + compile, bounded)
  -> Reflex / Fast Decision (<100ms, no LLM on fast path)
  -> Memory (typed, provenance)
  -> Model Routing (privacy + task + hardware + cost)
  -> Reasoning -> Planning -> Orchestration (durable)
  -> Policy / Security gate
  -> Execution (sandboxed) -> Verification
  -> Memory / Skill Learning
```

Never: `UI -> LLM -> Tools`.

## 2. Tech strategy

* **Rust = trusted core:** runtime, event bus, IPC, device mesh, permissions,
  storage coordination, memory, scheduler, router infra, agent primitives,
  sandbox, MCP/A2A transport, inference wrappers, voice orchestration,
  crypto, telemetry, gateway daemon, cancellation/timeouts.
  Tokio, channels, structured concurrency, typed errors, bounded queues,
  backpressure, streaming. No blocking in async, no god objects.
* **Python =** ML experimentation / eval / fine-tune / benchmarks only.
* **Dioxus (Rust) =** FIELD desktop shell. No Tauri, no TypeScript web app.
* **Kotlin/Swift =** thin device shims only, via UniFFI / C-ABI.
* All cross-language boundaries are explicit versioned contracts in `protocols/`.

## 3. UI — IGRIS FIELD (Dioxus native)

Sentinel dropped. FIELD is a living operational space:

```text
+--------------------------------------------------+
| * ARISE - LISTENING    IGRIS FIELD         - [] x |
+------+--------------------------------+----------+
| rail | WHAT ARE WE DOING?             | presence |
| nav  | + mission 76% 3 agents 1 appr  + | devices  |
|      | + research 42%                 + | memory   |
|      | activity timeline (streaming)  | background |
+------+--------------------------------+----------+
| eco:53327 net uptime clock  voice PTT            |
+--------------------------------------------------+
```

* Missions-first, not chat-first. Presence `READY/LISTENING/THINKING/WORKING/
  VERIFYING/NEEDS-APPROVAL/PAUSED/COMPLETED/FAILED` with operational detail,
  never raw chain-of-thought.
* Global invoke: hotkey + `Arise` wake + tray + PTT. Event-driven, optimistic,
  cancellable. No sync LLM/filesystem/network on UI path.
* Design authority: `ui-ux-pro-max` skill. Tokens: near-black graphite,
  Signal Mint `#2ee6a8`, Electric Blue `#4d7cff`.

## 4. Repo layout

```text
igrisv5/
 apps/
  field-dioxus/   # Dioxus native FIELD shell (Phase 11)
  cli/            # headless CLI + debug timeline
  setup/          # installer / first-run
 crates/
  igris-events/       # typed event bus (Phase 1)
  igris-runtime/      # lifecycle, config, cancellation (Phase 1)
  igris-gateway/      # persistent daemon: sessions/devices/jobs (Phase 1/5)
  igris-telemetry/    # traces, metrics, debug timeline (Phase 1)
  igris-memory/       # working/episodic/semantic/procedural/preference/goal (Phase 2)
  igris-context/      # SuperContext prefetch + context compiler (Phase 2)
  igris-reflex/       # fast intent/risk/tool/model router (Phase 3)
  igris-router/       # adaptive model routing + providers (Phase 4)
  igris-orchestrator/ # durable task graphs + checkpoint/resume (Phase 5)
  igris-agents/       # specialized workers (Phase 6)
  igris-skills/       # procedural skill registry (Phase 6)
  igris-tools/        # permissioned tools (Phase 7)
  igris-policy/       # capabilities, approvals, audit, kill-switch (Phase 8)
  igris-sandbox/      # workspace confinement + secrets (Phase 8)
  igris-voice/        # VAD + wake + barge-in pipeline (Phase 10a)
  igris-vision/       # invoke-gated multimodal (Phase 10a)
  igris-policy/       # capabilities, approvals, ASSIST/OPERATE/AUTONOMOUS (Phase 8)
  igris-device/       # discovery/trust/caps/remote-exec/sync (Phase 9)
  # voice/vision/mcp/a2a/browser/computer/sandbox/security land in later phases
 protocols/{events,device,task,agent}/
 skills/ docs/ tests/ benchmarks/
 .igris/engineering/{ARCHITECTURE,ROADMAP,DECISIONS,SYSTEM_CONTRACTS,TASK_BOARD,...}
```

No empty-spec modules beyond Phase 1 scope are added to workspace until needed.

## 5. Reference architecture (concepts only, no cloning)

* **Jev/System-One:** typed fast decisions, confidence, kill unnecessary generative calls → `igris-reflex`.
* **OpenHuman:** Memory Tree, SuperContext, goals, durable workflows, idle continuation, skills, routing, compression, local-first → typed memory + provenance.
* **OpenClaw:** gateway daemon, event-driven, channels, skills, heartbeats/cron, trusted/untrusted split → `igris-gateway`.
* **Harness:** approvals, secrets, policy, MCP, workspaces, observability, isolation → `igris-policy`.
* **MCP:** Resources/Tools/Prompts + filtered lazy tool exposure.
* **A2A:** typed caps + task/artifact contracts, zero trust for externals.
* **Agents SDK patterns:** handoffs, agents-as-tools, guardrails, sessions, tracing — re-implemented in Rust.

## 6. Device mesh (clean re-design, same UX contract)

* Ports: `53317 HTTP / 53318 TLS FastSwap`, `53327 HTTP / 53328 TLS Eco`.
* Subnet-scan zero-config LAN (all interfaces, skip self, private-first, 50-way parallel, 2s timeout, 30s heartbeat, 120s expiry). mDNS later, not now.
* TLS 1.3 ring-only rustls, self-signed per-device, TOFU `SHA256(DER)` pin, OTP pairing, `EcoMessage{version,msg_type,sender,timestamp,payload,hmac}`, LocalSend-v2 wire compat.
* Every device advertises `identity/caps/compute/storage/net/battery/models/permissions`.

## 7. Security / privacy / autonomy

* Capabilities: `read/write/delete/execute/network/credentials/device_control/communication/financial`.
* Levels `ASSIST / OPERATE / AUTONOMOUS`. Risky → approval. Secrets never in model context. Workspaces define `files/commands/net/env/CPU/RAM/timeout`. Audit everything. Kill-switch.
* Privacy `LOCAL-ONLY / HYBRID / CLOUD-ALLOWED` per tool/model/source. Private never leaves device for convenience.

## 8. Performance budgets (targets, benchmarked)

```text
UI event -> paint <16ms where possible
local reflex <100ms
memory/context <100-150ms
warm local path <300ms
non-reasoning first response <500ms perceived
streaming LLM = provider speed, cancellable
```

## 9. Build

```bash
cd igrisv5
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

FIELD app (Phase 11):
```bash
cargo run -p igris-field
```

## 10. Status

* Phase 0 Discovery: in progress (arch proposal + ADRs + contracts).
* Phase 1 Core Runtime: scaffolding now (events/runtime/gateway/telemetry).
* Phase 2 Next: Memory + SuperContext (voted first), then Reflex.
* Predecessor: NOT a dependency. Do not import from `../igris`.

License: Private — All rights reserved (commercial tiers TBD, GPL-free, `cargo-deny` later).
