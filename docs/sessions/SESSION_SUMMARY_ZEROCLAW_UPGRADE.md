# Operant ↔ ZeroClaw Upgrade Session Summary

**Date:** 2026-07-18
**Session:** ZeroClaw Feature Parity Audit & Upgrade Implementation

---

## Completed Progress

### Audit (✅ Complete)
- Full audit delivered: operant vs zeroclaw across all 14 core crates
- Identified ~92% structural parity at the module level
- Documented 19 specific missing features/components across 5 priority tiers
- Key finding: operant has unique features (kanban, trajectory, curator, AFT bridge, TDG memory) that zeroclaw lacks

### Commit 1: i18n Port (`6ef41ee3`)
- Fixed i18n config path references: `.zeroclaw` → `.operant`
- Copied locale `.ftl` files (en: `tools.ftl` + `cli.ftl`, zh-CN: `cli.ftl`) from `zeroclaw-runtime` to `operant-runtime`
- 637/638 tests pass (1 pre-existing schema version failure)
- `cargo check --workspace` passes clean

### Commit 2: Feature Flags System (`5714de2`)
- Added optional sub-crate dependencies to `operant-cli/Cargo.toml`:
  - `operant-channels`, `operant-tools`, `operant-runtime`, `operant-gateway`, `operant-plugins`, `operant-hardware`, `operant-memory`
- Feature flags forward correctly to sub-crate features matching zeroclaw's pattern:
  - `agent-runtime` enables all core channels (22 channel features)
  - `gateway` enables `operant-gateway`
  - `channel-*` features forward to `operant-channels/channel-*`
  - `hardware` forwards to `operant-hardware/hardware`
  - `browser-native`, `plugins-wasm`, `rag-pdf` forward to tools
  - `memory-postgres` forwards to `operant-memory/memory-postgres`
  - `ci-all` meta-feature enables everything
- `cargo check + tests` pass (637/638, 1 pre-existing schema failure)

---

## Pending Progress

### Missing CLI Subcommands (🔲 Medium Priority)
- `channel` — list/start/doctor/add/remove/bind-telegram/send
- `sop` — list/validate/show
- `hardware` — discover/introspect/info
- `peripheral` — list/add/flash
- `migrate` — openclaw import
- `service` — install/start/stop/status/uninstall/logs

### ACP Bridge Binary (🔲 Medium Priority)
- `zeroclaw-acp-bridge` binary target (`src/bin/zeroclaw-acp-bridge.rs`)
- Required for remote agent connections

### Missing Workspace Crates (🔲 Low Priority)
- `robot-kit` — robotics abstraction (config, traits, drive, emote, listen, look, sense, speak, safety)
- `apps/tauri` — Tauri v2 desktop application
- `tools/fill-translations` — i18n translation tool

### Feature Flags Enhancements (🔲 Low Priority)
- `gateway` feature only enables `dep:operant-gateway` with no sub-feature forwarding
- `operant-core` still hardcodes `features = ["anthropic"]`
- Add documentation comment to `agent-runtime` about maintenance burden for new channels
- `agent-runtime` hardcodes 22 channel features — needs updating when new channels are ported

---

## Known Issues

### Pre-existing Test Failure (Untracked)
- **Test:** `config_schema_export_contains_expected_contract_shape`
- **Location:** `crates/operant-config/src/schema.rs:12237`
- **Cause:** JSON schema version mismatch — expects `draft/2020-12` but gets `draft-07`
- **Action:** Should be filed as separate bug fix

### .ftl Branding Debt
- Copied `.ftl` files still contain "zeroclaw" references in test assertions and string content
- Tests validate format correctness, not branding — acceptable for now
- Rebranding requires updating both `.ftl` files AND test assertions simultaneously

---

## Architectural Decisions

- **No identity module** — User confirmed operant keeps hermes-agent style (learning/evolution focus, not identity modulation)
- **Feature flags at crate level** — Each sub-crate owns its feature gates; workspace-level coordination requires optional dep forwarding in operant-cli
- **No empty stubs** — Empty feature flags that don't forward to sub-crates are YAGNI and misleading

---

## Next Session Priorities

1. **Missing CLI subcommands** — `channel`, `sop`, `hardware`, `peripheral`, `migrate`, `service`
2. **Fix pre-existing test failure** — JSON schema version mismatch in operant-config
3. **Feature flags enhancements** — gateway sub-features, anthropic feature flag, documentation comments
