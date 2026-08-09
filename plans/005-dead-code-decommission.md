# 005 — Dead-code decommission (inventory → verify → remove or gate)

Stamped: `d394c136`. Priority: **P1**. Do after 001 (needs a green baseline) and
coordinate with 003 (gateway stubs) + 007 (telemetry) where surfaces overlap.

## Why

~150k LOC of the workspace is dead or placeholder code that ships in the default
binary: it inflates build time, attack surface, clippy debt, and review burden, and it
misleads auditors. Each item below was flagged with evidence in prior rounds.

## Files in scope (each item: confirm zero live callers, then act)

| Item | Evidence | Action |
|------|----------|--------|
| `crates/operant-channels/src/orchestrator/mod.rs` (14k LOC legacy orchestrator) | flagged R14/R15 as dead-wired; live gateway has its own loop | Verify no caller (grep `orchestrator::` outside the file). Remove module + its `mod` decl, or gate behind a non-default `legacy-orchestrator` feature. Prefer removal. |
| `crates/robot-kit/` (473 LOC; "TODO: implement actual TTS/camera/STT/motor" placeholders) | `speak.rs:40`, `look.rs:36`, `listen.rs:33`, `drive.rs:49`, `emote.rs:35` | Remove from workspace `members` (keep files if roadmap needs them) — do not ship stub hardware tools. |
| `crates/operant-eval/` (492-LOC skeleton: `task.rs`/`verifier.rs`/`reporter.rs`) | skeleton only | Either remove from members or keep with explicit `eval` feature; document that eval is not production surface. |
| `crates/operant-plugins/src/wasm_channel.rs` (placeholder) | "send/listen not yet wired" (lines 3, 30, 41) | Gate the wasm channel behind a non-default feature or remove; `wasm_tool.rs`/`host.rs` may stay if live. |
| `session_events` table | flagged R4 "fully dead"; `record_event` zero runtime callers | Remove table + writer + readers; run session regression tests. |
| `WhatsAppAdapter::with_phone_number_id` | flagged R14-5 zero callers | Remove constructor + tests, or wire it (R22 wired the adapter factory — re-verify callers first). |
| `whatsapp_web.rs` / `qq.rs` WIP dead-code blocks | `#[allow(dead_code)] // WIP: not yet wired` blocks | Remove the allow(dead_code) WIP blocks; keep only live send/receive paths. |
| TUI `commands.rs:1152` "TODO: populate with live MCP server data" | dead TUI surface | Implement (show live `core_mcp_manager` data) or remove the panel. |
| Gateway `api.rs` stubs | see plan 003 | Handled by plan 003 — do not duplicate. |
| `operant-runtime/src/agent/` legacy submodules if truly unreferenced | R5-3 flagged agent/classifier, context_analyzer, etc. — **RE-VERIFY**: since R24/R25 the gateway uses `operant_runtime::agent::Agent` (ws.rs:343). Only remove modules with zero callers, e.g. `history_pruner.rs`, `classifier.rs` if grep-cold. | Grep-verify each before removal. |

## Files out of scope

- Live gateway agent (`operant-runtime::agent::Agent`) — used by ws.rs/acp_server.rs.
- Any tool registered in `builtin.rs` with a live caller.
- The channels platform adapters themselves (feature-gated and unit-tested: 1,262 tests).

## Steps

1. For each table row: `grep -rn '<symbol>' crates/ --include='*.rs'` across the whole
   workspace (tests included). Only proceed if the only hits are the definition + its
   own tests.
2. Remove (preferred) or gate behind a non-default feature (if the user wants the code
   preserved for roadmap). When gating, the feature must NOT be in `default = [...]`.
3. Remove dead config fields the dead code references (e.g. `schema.rs` "reserved for
   future use" blocks, `schema.rs:9572`), unless plan 014's README says otherwise.
4. After each removal batch: `cargo check --workspace`, run the affected crate's full
   lib tests, then `cargo test --workspace --all-features --lib` at the end.
5. Record each removal in `BUGS.md` (one entry per item, with the grep evidence).

## Done criteria

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings            # zero hits (base from 001)
cargo test --workspace --all-features --lib                      # green
cargo build --release -p operant-cli --bin operant && ls -la target/release/operant   # binary shrinks or stays; note before/after size in BUGS.md
```

## Test plan

- Existing suites must stay green (they exercise live paths). If a removed module had
  tests that were the *only* coverage, note the coverage loss in BUGS.md and, where the
  behavior was live, move the test to the live module.

## Maintenance note

- The `#[expect(dead_code)]` convention (migrated in earlier rounds) should be removed
  along with the code it excuses — dead-code allowances must not accumulate.
- Re-run the workspace clippy gate after this plan: dead `#[allow]`s often hide lints.

## Escape hatches

- If grep shows any doubt about a caller (e.g. dynamic/reflection-based registration),
  do NOT remove — gate behind a feature flag instead and note it.
- If removing `robot-kit`/`operant-eval` breaks the workspace (e.g. referenced by CI
  build.yml artifacts), update `build.yml` in the same commit.
