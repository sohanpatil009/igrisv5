# System Contracts (v0.1 — Phase 1)

## Events
`Event` enum in `igris-events` is the wire for UI/agents/gateway/telemetry.
Envelope `{id, at, event}`. Capacity 1024. Consumers subscribe, never call internals.

## Memory
`MemoryRecord{id,kind,content,source,created_at,updated_at,confidence,importance,privacy,owner,provenance{source,device_id,agent,tool,confidence},tags,deleted}`.
Privacy `LocalOnly/Hybrid/CloudAllowed`. Store: `store/get/soft_delete/search/count`.

## Context
`ContextBundle{items,budget_chars,used_chars}`. `prefetch(store,query,limit,budget)` then `compile` (dedupe+importance+budget).

## Reflex
`ReflexDecision{intent:Intent,tool,target_device,requires_reasoning,risk:Risk{Low,Medium,High,Critical},approval,memory_scope,model_hint,cached,confidence}`.
`Reflex{policy,cache}` engine: `decide()` classifies (risky-first), `EscalationPolicy{min_confidence 0.6, approval_from High}` applies, `DecisionCache` (cap 256) serves hot paths. Fast path never calls LLM. Budget <100ms/decision.

## Router
Static `route({privacy,task,needs_vision,needs_reasoning,network_up}) -> ModelClass` (Secret/Confidential always local).
Adaptive `ModelRouter{providers,stats,policy}`: `route_to(req,tokens) -> RoutingDecision{provider_id,class,reason,estimated_cost,fallbacks}`.
`RouteStats` is aggregate counters only (no prompts). `RoutingPolicy{prefer_local,allow_cloud,max_cost_per_request}`; over-budget → `BudgetExceeded`. Local echo is always the last fallback.

## Orchestrator
`Mission{nodes,order,state}` + `TaskNode{id,name,state,attempts,max_retries,timeout_ms,depends_on,requires_verification,verified,last_error,checkpoint}`.
`ready_tasks()` = pending + deps done + mission Running. `begin/record_success/record_failure(retry while attempts<=max)/verify_node/pause/resume/cancel`.
`to_json/from_json` for crash recovery; `resume_point()` is dep-aware; `reclaim_running()` frees dead leases. `run_with_timeout()` for executor deadlines.

## Policy / Security
`Policy{autonomy,max_auto_level,kill}` — kill-switch denies all, incl. live approvals. `ApprovalStore` TTL-expires; `check_with_approval()` bypasses the autonomy gate only with a live approval.
`AuditLog` hash-chained (`record/verify_chain`). `Workspace` confines paths/commands/network/env with caps. `SecretsVault` isolates values; `redact()` scrubs all text. Cross-crate `security_chain` tests prove every injected failure closes safe.

## Device
Ports `53317/53318/53327/53328`. `DeviceInfo{id,name,platform,caps,trusted}` + `compute_score()` + `announcement()`.
`Identity` (secret never transmitted) → SHA-256 fingerprint → TOFU `TrustStore` (mismatch = MITM error). OTP pairing (6-digit/120s/3-attempts).
`EcoMessage{version,msg_type,sender,timestamp,payload,signature}` HMAC-signed; stale/version/off-key rejected.
Transfers: prepare→approve/deny→token upload→checksum complete; 1MB chunk resume; TTL sweeps.
HTTP routes: `GET info`, `POST prepare/confirm/deny`, `POST upload/:session/:file?token=`, `POST complete`. TLS proxy in 9b.

## Tools/Agents
`Tool{definition{name,description,version,permission,risk,requires_network,timeout_ms},validate,run}`. `RiskLevel` Medium+ needs approval.
Registry validates before running; `execute_async` isolates `run()` on the blocking pool with timeouts; 10k audit.
Natives: `filesystem` (root-confined), `terminal` (denylisted, High risk), `browser` (URL validation now, transport Phase 9).
`Catalog` lazy factories + `definitions_for()` capability filtering — models see only allowed tools.
`AgentDef{name,role,capabilities,allowed_tools,tier,max_iterations,system_prompt}` (7 roles, least-privilege, workers can't delegate).
`Assistant` (tier 1) is the user-facing front door: clarifies intent, delegates to the orchestrator for worker routing, presents results; no worker tools.
`execute_task()` validates scope then plans deterministically. `Handoff{from,to,reason,context}` gated by `can_handoff()`.
A2A: `AgentCard/AgentTaskRequest/Artifact` + `validate_task()` + `verify_artifact()` (known-producer + hash). Externals never trusted.

## Skills
`SkillManifest{name,description,version,inputs,preconditions,permissions(L0-L4),tools,steps,validation,failure_recovery,cautions}` validated on register.
`requires_approval()` = L3+. `SkillRegistry{enable/disable/record_use/record_success/promotion_candidates/approval_gated}`. Manifests live in `skills/*.json` (18-skill JARVIS pack; guard test loads all through the validator). Nothing self-installs.

## Voice / Vision
`Vad` (energy + ZCR + hangover) → segment → `SpeechToText` → `WakeMatcher::arise()` (fuzzy≤2) → reflex fast path → short ack; escalations stay silent. Barge-in cancels live synthesis. `VoiceState{Idle,Listening,Processing,Speaking,Error}` + `VoiceEvent` stream. Neural STT/TTS/mic in 10b.
`VisionGate::should_invoke()` requires pixels + reason; `ImageDescriber` seam with honest null. Multimodal never pays the image tax by accident.
