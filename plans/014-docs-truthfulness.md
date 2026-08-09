# 014 — Docs truthfulness: README, SECURITY, CHANGELOG

Stamped: `d394c136`. Priority: **P2** (last — reflects finalized decisions from 003,
005, 009, 012).

## Why

`README.md` describes a llama.cpp-era product and is false for the shipped binary:
- Claims "15 built-in tools" (reality: 60+ registered in `builtin.rs`), "100% local,
  zero telemetry" (reality: skill `.usage.json`/curator telemetry + OTel observer
  exist), "Rust 1.78+" (reality: `rust-version = 1.88`), REPL-only (reality: REPL, TUI,
  gateway, ACP, channels, cron, kanban, MCP).
- No `SECURITY.md`, no `CHANGELOG.md`, no migration convention doc.

## Files in scope

- `README.md` (full rewrite)
- New `SECURITY.md` (auth posture from 003, secret handling from 002, SSRF/skills-guard
  summary)
- New `CHANGELOG.md` (skeleton: v0.1.4 baseline + the R-round log pointer to BUGS.md)
- New `docs/migrations.md` (from 004) if not already created there
- `AGENTS.md` — only if a convention changed (e.g. workspace clippy gate from 001)

## Files out of scope

- Code behavior; `BUGS.md` (keep as the audit log — reference it, don't rewrite it).

## Steps

1. **Rewrite README** to match reality:
   - What it is (Rust agent, hermes-parity core), Quick start via **local build**
     (per user: local builds until baseline finalized — show `cargo build --release -p
     operant-cli --bin operant` + `cargo install --path`), not a stale `cargo install --git`.
   - Accurate feature table: agents (CLI/TUI/gateway/ACP), tools (count + categories),
     memory, skills (guard/marketplace/curator), channels (list the feature-gated set),
     cron/kanban, providers (list), telemetry disclosure (what is stored where, local
     by default, OTel opt-in).
   - Requirements: Rust 1.88+, and the OS-level deps (git, ripgrep, docker optional).
2. **SECURITY.md**: gateway auth (token/bearer + default unauthenticated + host:port
   default), config/.env/state 0600 policy, SSRF fail-closed on URL tools, skills_guard
   hard-block semantics, write-approval gate (after 012), reporting channel.
3. **CHANGELOG.md**: v0.1.4 baseline entry + "see BUGS.md for the R1–R30 audit log"
   pointer.
4. **Cross-check numbers**: tool count must come from `builtin.rs` (run a grep count,
   don't guess); test count from the last green `cargo test --workspace --all-features
   --lib` run.
5. Update `AGENTS.md` only if the workspace clippy gate (001) is now a standing
   requirement — add it to the verify section.

## Done criteria

```bash
cargo fmt --all --check        # no code touched; gate is for safety
```
Manual review: every factual claim in README/SECURITY maps to a grep-able source
(tool count, MSRV, test counts, telemetry paths, auth methods). No "100% local, zero
telemetry"-style claims survive unless true.

## Test plan

- No code tests. Review checklist instead: `grep -c 'register(' builtin.rs` matches the
  README tool count; `grep rust-version Cargo.toml` matches Requirements; telemetry
  claims reference the actual `.curator/usage.json` path.

## Maintenance note

- Docs are part of the iteration: any round that changes a public surface (new tool,
  auth change, new command) updates the README table in the same commit.
- Keep BUGS.md as the working audit log; CHANGELOG gets the user-facing entries.

## Escape hatches

- If a claim can't be verified from the code in this round, write "see
  <file>:<line>" instead of asserting it — never guess numbers.
