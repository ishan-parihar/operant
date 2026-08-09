# 007 — Telemetry consolidation: one `.usage.json` writer + one reader API

Stamped: `d394c136`. Priority: **P1** (small).

## Why

R20/R21 fixed a write race in skill `.usage.json` telemetry and unified the curator
into `.curator/usage.json` under a process-wide lock (`SkillUsageTracker`), but the
round explicitly flagged a **dual telemetry implementation**: `record_usage` (writes
`.usage.json`) vs `SkillUsageTracker` (writes `.curator/usage.json`). Two stores for
one concept means inconsistent data and two maintenance surfaces.

## Files in scope

- `crates/operant-core/src/tools/skills_tool.rs` (`record_usage` path)
- `crates/operant-core/src/skill_usage.rs` (`SkillUsageTracker`, lock, save/load)
- `crates/operant-core/src/curator/mod.rs` (tracker construction + transactions)
- Any other `.usage.json` / `.curator/usage.json` reader/writer (grep `usage.json`)

## Files out of scope

- OTel observer / runtime observability (separate; not part of skill usage telemetry).

## Current state (evidence)

- R20: `.usage.json` write race fixed with process-wide lock + atomic tmp+rename.
- R21: `skill_manage` feeds `.curator/usage.json` (create/patch/delete), curator
  transactions, `USAGE_TELEMETRY_LOCK`.
- R20-followup: "verify no other .usage.json writers bypass the lock (only record_usage
  writes it; SkillUsageTracker uses .curator/usage.json) — flag dual implementations".

## Steps

1. **Map all sites**: grep `usage.json`, `record_usage`, `SkillUsageTracker`,
   `USAGE_TELEMETRY_LOCK`, `bump_use/bump_view/bump_patch/mark_agent_created/forget`
   across the workspace. Produce a table: writer → path → lock.
2. **Decide the single store** (prefer `.curator/usage.json` — it already carries the
   full create/patch/delete event set and the curator reads it). 
3. **Consolidate**: route `record_usage` (skills_tool) through `SkillUsageTracker`
   under the same lock; delete the old `.usage.json` writer. Keep the reader API surface
   (`SkillUsageTracker::*`) unchanged so curator/CLI callers don't churn.
4. Migration: on load, if a legacy `.usage.json` exists and the curator store is empty
   (or missing keys), merge legacy patch counts in, then remove the legacy file.
5. Update tests that assert on the old path; add a merge test.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-core --all-targets -- -D warnings
cargo test -p operant-core --lib skill_usage && cargo test -p operant-core --lib curator && cargo test -p operant-core --lib skills
cargo test --workspace --all-features --lib          # final gate
```
`grep -rn 'usage.json' crates/` shows exactly **one** writer path and one reader API.

## Test plan

- `test_legacy_usage_json_merged_then_removed`: seed legacy `.usage.json` + curator
  store, run the migration, assert merged counts and legacy file gone.
- Keep the existing 8-thread concurrent-write test (must still pass — proves the single
  store is lock-safe).

## Maintenance note

- Any new telemetry reader must import the shared tracker — no direct file reads.
- The flock test from R21 (`SkillUsageTracker::with_exclusive_lock`) is the contract.

## Escape hatches

- If `record_usage` callers depend on the `.usage.json` file existing at a fixed path
  (external tooling), keep a compatibility shim that mirrors writes to the legacy path
  but logs a deprecation warning — do not maintain two independent stores.
