# ADR-001 — Clean reset, no predecessor reuse

Status: Accepted. Date: 2026-09-20.

Context: `igris/` + `igris-ecosystem/` contain duplication, globals, god objects.
Decision: New `igrisv5/` workspace from scratch. Old repos are inventory only.
Consequences: Slower start, correct foundation, `cargo-deny` GPL-free from day 1.
