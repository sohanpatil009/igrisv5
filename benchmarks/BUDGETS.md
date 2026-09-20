# Performance budgets — IGRIS v5 (Phase 12 gate)

Targets from the v5 spec, each enforced by an in-tree budget test
(`cargo test --workspace` fails if a budget regresses).

| Path | Budget | Gate test | Measured (dev, Windows x64) |
|---|---|---|---|
| Local reflex decision | <100ms each, avg <5ms | `igris-reflex::latency_budget` (3000 decisions) | ~10µs avg |
| Memory search, 500 records | <150ms | `igris-memory::budget_search_500_records_under_150ms` | <100ms |
| Context compile, 500 records | <300ms | `igris-context::budget_compile_500_records_under_300ms` | <10ms |
| Model route decision | <10ms each, avg <1ms | `igris-router::budget_route_1000_decisions_under_10ms_each` | ~µs |
| Mission ready-scan, 200 nodes | <50ms | `igris-orchestrator::budget_ready_tasks_200_nodes_under_50ms` | <5ms |
| Tool timeout ceiling | default 30s, per-tool override | `igris-tools::async_timeout_caps_blocking_tool` | 50ms cap proven |

UI budgets (event→paint <16ms, first response <500ms perceived) are
architectural (event-driven, optimistic, cancellable) and verified by
construction + the pausable 1s tick in FIELD — instrumented frame timing
lands with the 11b SSE stream work.

Run: `cargo test --workspace budget`
