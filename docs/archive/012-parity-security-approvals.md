# 012 — Parity: write-approval gate + security/approvals surface verification

Stamped: `d394c136`. Priority: **P1** (core — user-designated "security/threat/approvals").

## Why

Operant already has strong pieces (skills_guard hard-blocks — R10; threat-pattern
integration in `skills_guard.rs`; `ApprovalTool`; write-origin provenance in
`write_origin.rs`), but hermes's **staged write-approval gate** (`write_approval.py`,
493 LOC) has no equivalent: writes originating from **remote/background** contexts
(channel messages, gateway, cron) are gated and **staged for approval** instead of
executing. This is the missing approval surface for non-interactive origins.

## Hermes reference (write_approval.py semantics)

- `write_approval_enabled(subsystem)` — per-subsystem toggle (normalized bool).
- `stage_write(subsystem, payload)`, `list_pending`, `get_pending`,
  `discard_pending(pending_id)`, `pending_count`.
- `GateDecision { allow | blocked | stage, message }`.
- `current_origin()` / `is_background()` — decide gating by where the write came from.

## Files in scope

- New `crates/operant-core/src/write_approval.rs` (gate: `stage_write`,
  `list_pending`, `discard_pending`, `pending_count`, `GateDecision`,
  `write_approval_enabled`, origin detection `current_origin`/`is_background`)
- `crates/operant-core/src/write_origin.rs` — verify completeness vs hermes
  `skill_provenance.py`/write-origin semantics; close any deltas found
- Write-path callers: `file_tools.rs`, `patch_tool.rs`, `skills_tool.rs` (the write
  gate call sites — apply gate when origin is remote/background AND approval is
  enabled; keep interactive origin bypassing, matching hermes)
- `crates/operant-core/src/security.rs` / `skills_guard.rs` — verification pass only
  (threat patterns + tirith parity; fix only concrete deltas found)

## Files out of scope

- New threat rules (verification only).
- Channel/gateway origin plumbing beyond `current_origin`/`is_background` detection.

## Steps

1. **Audit first** (read-only): read `write_origin.rs` and compare its origin semantics
   against hermes `write_approval.current_origin`/`is_background` + `skill_provenance.py`.
   Write the delta list into `BUGS.md`.
2. **Implement `write_approval.rs`**: pure gate + origin detection. Persisted pending
   store (JSON under `~/.operant/approvals/` — atomic write, 0600, via the core
   secret-write helper from plan 002, `crates/operant-core/src/fs_secrets.rs`).
3. **Wire the gate**: in the write paths, when `is_background()` && `write_approval_enabled`
   → return `GateDecision::Stage` (write is staged, message tells the user where);
   interactive origin keeps current behavior. This must be opt-in (config
   `approvals.write_gate = true`) so existing behavior is unchanged by default.
4. **CLI surface**: `operant approvals list|approve|discard` (mirror `list_pending`/
   `get_pending`/`discard_pending`).
5. **Verification pass** on `security.rs` (tirith) + `skills_guard.rs` (threat
   patterns): diff against hermes `tirith_security.py`/`threat_patterns.py`; fix only
   concrete missing checks; record the verdict.
6. Update `BUGS.md`; run suites.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-core --all-targets -- -D warnings && cargo clippy -p operant-cli --all-targets -- -D warnings
cargo test -p operant-core --lib write_approval && cargo test -p operant-core --lib write_origin && cargo test -p operant-core --lib skills_guard
cargo test --workspace --all-features --lib          # final gate
```
Manual: with the gate enabled and a background origin, a `file_write` returns
"staged for approval" and the file is not written; `operant approvals approve <id>`
completes it.

## Test plan

- `gate_stages_when_background_and_enabled` / `gate_bypasses_when_interactive`.
- `gate_disabled_by_default`: config off → background write executes (parity with
  current behavior).
- `pending_store_roundtrip`: stage → list → get → discard; persisted across reload.
- `write_origin_delta_closure`: one test per delta found in step 1 (or a note if zero).
- `skills_guard_verification_unchanged`: existing skills_guard suite stays green
  (verification-only round).

## Maintenance note

- The gate is an opt-in behavioral change — it must never change interactive behavior.
- Pending approvals are user data: 0600 + atomic writes; expired/old pendings should be
  pruned on `list`.

## Escape hatches

- If `current_origin` can't be detected reliably on a path (no origin context), default
  to **interactive/bypass** (current behavior) and log — never block interactively.
- If the verification pass finds >3 real deltas in `security.rs`/`skills_guard.rs`,
  split the security-verification work into its own follow-up round.
