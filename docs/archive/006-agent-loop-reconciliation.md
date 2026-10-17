# 006 — Agent-loop reconciliation: one shared behavior core for both live agents

Stamped: `d394c136`. Priority: **P1** (core architecture). Largest plan — allow two
rounds (006-a extraction, 006-b parity harness).

## Why

There are **two live agent loop implementations** with duplicated behavior:
- `operant_core::OperantAgent` — used by CLI / TUI / `operant run` (`operant-cli/src/main.rs:86,895,926`).
- `operant_runtime::agent::Agent` — used by the gateway WS + ACP paths
  (`operant-gateway/src/ws.rs:343`, `acp_server.rs`).

R23/R24/R25 already had to port the **same** fixes to both (empty-response retry
ladder, evolution/memory/skill triggers, compression + todo re-injection). Every future
hermes behavior change must be applied twice — this is where silent divergence enters.
Goal: extract the shared turn-behavior rules into one module both loops call, and add a
parity test that proves identical turn semantics under identical scripted input.

## Files in scope

- `crates/operant-core/src/agent/mod.rs` (OperantAgent loop internals)
- `crates/operant-runtime/src/agent/loop_.rs`, `agent.rs`, `turn_context.rs`
  (runtime Agent loop internals)
- New shared module: `crates/operant-core/src/agent/turn_rules.rs` (or extend an
  existing shared module) + re-exports
- New parity test file: `crates/operant-core/tests/agent_parity.rs` (integration test;
  this crate can depend on operant-runtime? If not, put the harness in
  `crates/operant-runtime/tests/` or `crates/operant-cli/tests/` — pick the crate that
  can import both agent types; operant-cli links both via features, verify at step 1)

## Files out of scope

- Provider/model layers, tool registries (different by design).
- Channel adapters, gateway server plumbing.

## Current state (evidence)

- Same bugs fixed twice: R23 empty-response ladder (`run_tool_call_loop`), R24 evolution
  triggers on `turn()` + `turn_streamed()`, R25 same ladder on `turn()`/`turn_streamed()`
  (ws.rs:721 + acp_server.rs:661). `EMPTY_RESPONSE_MAX_RETRIES = 3` exists in both.
- Both implement: memory/skill nudge counters, post-turn memory review, todo
  re-injection after compression, loop detection, history pruning.

## Steps

1. **Inventory the duplication**: list each shared behavior (empty-response retry,
   evolution triggers, compression+todo re-injection, loop detection thresholds,
   budget accounting) with its implementation sites in both agents. Write the list into
   the plan/`BUGS.md` as the ground truth.
2. **Extract** each behavior into pure functions/structs in the shared module (no agent
   type dependency — operate on `&mut TurnState`-style plain data). The agents keep
   their own I/O (provider calls, tool execution) but delegate the *rules* to the shared
   module. Ship behavior-identical refactors — no semantic change in this phase.
3. **Parity harness**: a test that constructs BOTH agents (scripted provider returning
   scripted turn sequences — see the existing scripted-provider test pattern used in
   R23/R25 runtime tests) and asserts identical outputs/tool calls/refunds for a matrix
   of scenarios: empty response ×3 then answer; empty response ×4 (exhaustion); memory
   nudge interval firing; skill nudge firing; compression + todo re-injection; budget
   exhaustion.
4. Run the full suites; update `BUGS.md`.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-core --all-targets -- -D warnings && cargo clippy -p operant-runtime --all-targets -- -D warnings
cargo test -p operant-core --lib && cargo test -p operant-runtime --lib
cargo test -p operant-core --test agent_parity       # or wherever the harness lands
cargo test --workspace --all-features --lib          # final gate
```

## Test plan

- The parity harness (3–5 scenario tests) is the deliverable test.
- Each extraction ships with its own unit tests in the shared module.

## Maintenance note

- From this point, new turn behaviors must be implemented in the shared module; the two
  agents become thin wrappers. Enforce in code review: any behavioral constant
  (retry counts, intervals, caps) must live in the shared module.
- The `hermes-agent-ultra` port keeps its own loop; parity with ultra remains a separate
  concern (their tests are the reference oracle).

## Escape hatches

- If a behavior is genuinely loop-specific (streaming vs non-streaming), keep it local
  but document why in the shared module. Do not force unification that breaks streaming.
- If the two agents' message/history types differ too much to share code cleanly, fall
  back to a **shared constants + shared pure-decision module** (no shared types) — the
  parity test still enforces behavioral equality.
