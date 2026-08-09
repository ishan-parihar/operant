# 001 — Local Quality Baseline: green `clippy -D warnings` + `test --workspace --all-features`

Stamped: `d394c136`. Priority: **P0**. Do first — every later plan's done-criteria
rely on these gates being locally green.

## Why

The workspace cannot pass the project's own quality commands locally:
- `cargo clippy --workspace --all-targets -- -D warnings` → **146 errors** (pre-existing
  debt, concentrated in `operant-core`; 2 are hard compile errors in `operant-providers`).
- `cargo test --workspace --all-features --lib` → **1 failing test**
  (`tool_registry::tests::hardware_feature_registers_all_six_tools`).
- MSRV drift: `Cargo.toml` declares `rust-version = "1.88"` but `.github/workflows/ci.yml`
  `msrv` job pins toolchain `1.86` (cannot pass) and `README.md` claims "Rust 1.78+".

CI is intentionally manual (user decision) — this plan makes the **local** baseline
green so every future plan can validate against it. It does not enable any automation.

## Files in scope

- `crates/operant-core/src/` (all files with lint hits — see inventory below)
- `crates/operant-providers/src/auth/gemini_oauth.rs` (line 274)
- `crates/operant-hardware/src/tool_registry.rs` (test at ~line 280–315)
- `Cargo.toml` (no change expected; MSRV stays 1.88)
- `.github/workflows/ci.yml` (msrv job pin only)
- `README.md` (Rust-version claim only; full rewrite is plan 014)

## Files out of scope

- No behavior changes to any live path. Lint fixes only. If a lint fix would change
  runtime behavior, STOP and report instead.
- Do not touch `.github/workflows/*` beyond the msrv pin (CI stays manual/tag-gated).

## Current state (evidence)

- Clippy error inventory (top files, from `grep -oE 'crates/...rs'` on a
  `--workspace --all-targets -- -D warnings` run): `operant-core/src/gateway/mod.rs` 16,
  `agent/mod.rs` 9, `tools/skills_tool.rs` 8, `parser.rs` 7, `skills.rs` 6,
  `oauth_refresh.rs` 6, `mcp_oauth.rs` 6, `tools/file_tools.rs` 5, `schema.rs` 5,
  `tools/transcription_tool.rs` 4, `tools/debug_helpers.rs` 4, `agent/fallback.rs` 4,
  plus `accessibility.rs`, `acp/mod.rs`, `agent/background_review.rs`, `agent/insights.rs`,
  `agent/llm_compressor.rs`, `agent/message_safety.rs`, `agent/turn_context.rs`,
  `database.rs:2101` (`needless_mut`).
- Dominant lint classes: `collapsible_if` (majority), `sort_by_key`, `manual_div_ceil`,
  `needless_mut`, `format_push_string`, redundant `format!` references.
- **Compile errors (fix first — they block `operant-providers`):**
  `crates/operant-providers/src/auth/gemini_oauth.rs:274:68` and `:274:87` —
  "redundant reference in `format!` argument".
- Failing test: `crates/operant-hardware/src/tool_registry.rs` test
  `hardware_feature_registers_all_six_tools` panics at line 309 under `--all-features`
  (asserts an exact tool count; `--all-features` registers additional tools).

## Steps

1. **Fix the 2 compile errors** in `gemini_oauth.rs:274` (drop the redundant `&` in the
   `format!` args). Verify: `cargo check -p operant-providers --lib` exits 0.
2. **Sweep the clippy errors crate by crate**, starting with `operant-core`.
   For each file: run `cargo clippy -p <crate> --all-targets -- -D warnings`, fix every
   `error:`/`warning:` with a *mechanical* fix (collapse `if`, use `sort_by_key`,
   `div_ceil`, drop `mut`, drop redundant refs). Run `cargo fmt --all` after each batch.
   Do not `#[allow(...)]` — fix properly. If a fix is non-mechanical (would change
   semantics), STOP and flag it.
3. **Fix the hardware test.** Read `crates/operant-hardware/src/tool_registry.rs`
   (the `tool_registry::tests` module). The test asserts an exact registered-tool count
   that `--all-features` breaks. Fix by asserting the six named tools are **present**
   (set containment / `assert!(names.contains(...))` for each of the six) rather than an
   exact count — or make registration feature-independent if the six are core.
   Verify: `cargo test -p operant-hardware --lib --all-features tool_registry` green,
   and `cargo test -p operant-hardware --lib` (default features) still green.
4. **Align MSRV**: set the `msrv` job toolchain in `.github/workflows/ci.yml` to `1.88`
   (match `rust-version`). Update the README requirement line to "Rust 1.88+" (full
   README rewrite is plan 014 — only the version line here).

## Done criteria (all must pass locally)

```bash
cargo fmt --all --check                                          # exit 0, no diffs
cargo clippy --workspace --all-targets -- -D warnings            # exit 0, zero hits
cargo test  --workspace --all-features --lib                     # exit 0, 0 failed
```
Plus the four per-crate suites (core / cli / runtime / gateway) stay green.

## Test plan

- No new tests required for lint fixes (behavior-neutral). The hardware test fix IS the
  test change; it must pass under both `--all-features` and default features.

## Maintenance note

- Every future round must keep the workspace clippy gate clean (the rounds currently
  only lint touched crates — from now on, run the workspace gate at round end).
- The `collapsible_if` class is the most common; new `if let ... && ...` chains should
  be written collapsed from the start.

## Escape hatches

- If a lint fix would alter runtime behavior (e.g., a `collapsible_if` with side-effect
  orderings), STOP, revert that hunk, and report it — do not force it.
- If the workspace gate surfaces > 20 new lints mid-sweep, re-baseline the inventory
  before continuing.
