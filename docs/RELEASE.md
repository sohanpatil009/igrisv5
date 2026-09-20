# Release engineering — IGRIS v5 (Phase 12 baseline)

## Versioning

SemVer per crate, workspace releases tagged `vX.Y.Z`. `0.1.0` throughout:
pre-release while 9b/10b/11b land. No public API stability promise yet.

## Channels

* `dev`: `cargo run -p igris-field` / `igris-cli` from source.
* `beta`: git tags, signed where infra exists. Ask: code-signing certs?
* `stable`: installer + auto-update — not built yet (queued, needs signing).

## Pre-release gate (run in order)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
# cargo deny check   (needs cargo-deny in CI; deny.toml is ready)
```

## Supply chain

* `deny.toml`: GPL/AGPL/LGPL-3 denied, MIT/Apache/BSD/ISC/Unlicense (+Zlib,
  BSL-1.0, NCSA, Unicode, OpenSSL, Proprietary) allowed. Verified 2026-09-20
  via `cargo metadata` inventory: zero GPL-family licenses in the graph;
  `r-efi` triple-license consumed under MIT/Apache-2.0; all 19 first-party
  crates stamped `Proprietary`.
* `cargo-deny` / `cargo-audit` are NOT installed on this machine — CI must
  run them (deny.toml + advisory DB configured). Do not ship without a green
  deny check after adding dependencies.
* `Cargo.lock` is committed (binary workspace): reproducible builds.

## Known hardening debt (queued, explicit)

* Audit hash chain is `DefaultHasher` (structural) → HMAC-SHA256.
* Secrets vault is in-memory → OS keyring integration.
* Sandbox is boundary-checked, not OS-isolated (containers/namespaces).
* Browser fetch transport unwired (Phase 9b networking).
* Neural STT/TTS, mic capture, acoustic wake (Phase 10b).
* SSE live stream, device scan button, memory write panel, voice PTT (11b).
* Fuzz targets + property tests for parsers/envelopes.
