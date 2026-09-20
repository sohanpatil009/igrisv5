# IGRIS FIELD — Dioxus native shell (live since Phase 11a)

`cargo run -p igris-field` (needs a display; headless CI uses `igris-cli`).

Ops-desk command center per the `ui-ux-pro-max` direction: dark slate,
status green, live-labeled metrics, pausable refresh, empty states.
Every panel reads `FieldBackend` (events, memory, mission, approvals,
policy, telemetry, discovery) — nothing is simulated.
