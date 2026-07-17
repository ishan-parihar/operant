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

### i18n Port (✅ Complete — Commit `6ef41ee3`)
- Fixed i18n config path references: `.zeroclaw` → `.operant`
- Copied locale `.ftl` files (en: `tools.ftl` + `cli.ftl`, zh-CN: `cli.ftl`) from `zeroclaw-runtime` to `operant-runtime`
- 637/638 tests pass (1 pre-existing schema version failure)
- `cargo check --workspace` passes clean

---

## Pending Progress

### Feature Flags System (🔲 High Priority)
- **Problem:** `operant/Cargo.toml` is a virtual workspace manifest — can't have `[features]`
- **Correct fix:** Add optional dependencies (`dep:operant-channels`, `dep:operant-tools`, etc.) to `operant-cli/Cargo.toml` and forward features through them, matching zeroclaw's pattern
- Each sub-crate already has its own `[features]` section (operant-channels, operant-tools, operant-runtime, operant-memory, operant-hardware, operant-gateway)
- Empty stubs were added then reverted per YAGNI — proper forwarding was never attempted

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

1. **Feature flags system** — Add optional sub-crate deps to operant-cli and forward features
2. **Missing CLI subcommands** — `channel`, `sop`, `hardware`, `peripheral`, `migrate`, `service`
3. **Fix pre-existing test failure** — JSON schema version mismatch in operant-config
