# 010 — Parity: TUI niceties (focus pane, slash confirm, terminal hints)

Stamped: `d394c136`. Priority: **P1** (core — user-designated "tui-niceties").

## Why

Three hermes QoL surfaces are absent in operant's TUI/terminal stack:
1. `terminal_hints.py` — annotate terminal command failures with actionable hints.
2. `slash_confirm.py` — pending-confirmation store for async/slash confirmations.
3. `focus_pane_tool.py` — model tool that focuses agent attention on a named pane.

## Hermes references (behavior)

- `terminal_hints.py` (170 LOC): `annotate_failure(command, exit_code, output) -> Option<String>`
  with 7 heuristics: `gh` unknown json field, command-not-found, module-not-found,
  merge conflict, already-exists, gh rate-limit, permission-denied.
- `slash_confirm.py` (167 LOC): `register(session_key, payload)`, `get_pending`,
  `clear`, `clear_if_stale(timeout=DEFAULT)`, `resolve(...)` — per-session pending
  confirmations with a staleness timeout.
- `focus_pane_tool.py` (64 LOC): `focus_pane_tool(pane) -> str` — model calls it to
  direct attention (pane names like "messages"/"context" surface as a UI hint).

## Files in scope

- **Hints**: `crates/operant-core/src/tools/terminal_tool.rs` (append hint to tool
  error output when the child fails) + new pure helper
  `crates/operant-core/src/tools/terminal_hints.rs` (unit-testable, no I/O).
- **Slash confirm**: new `crates/operant-cli/src/tui/slash_confirm.rs` (in-memory
  session-keyed store with timeout) + wiring in the TUI input/commands path
  (`crates/operant-cli/src/tui/app/commands.rs`, `crates/operant-cli/src/tui/input.rs`).
- **Focus pane**: new tool `crates/operant-core/src/tools/focus_pane_tool.rs` (register
  in `builtin.rs`) + a lightweight TUI indicator consuming the focused-pane hint
  (e.g. a status line / overlay in `crates/operant-cli/src/tui/render/`).

## Files out of scope

- Terminal backend execution semantics (R11/R27/R30 work stays untouched).

## Steps

1. **Hints**: implement the 7 pure heuristics + `annotate_failure(command, exit_code,
   output)`. In `terminal_tool.rs`, when a command fails (non-zero exit), append
   `annotate_failure(...)` output as a `HINT:` line to the tool result. Tests for each
   heuristic.
2. **Slash confirm**: implement the store (HashMap keyed by session, with a
   `Instant`-based staleness timeout, prune on access). Wire at least one real use in a
   TUI-local flow (note: `/yolo` and gateway permission bypass live in the gateway
   channel layer, NOT the TUI — pick a TUI command from `tui/app/commands.rs` that
   should confirm before executing, e.g. a destructive `/clear`-style or settings
   mutation) — follow the existing dialogs pattern (`dialogs.rs`,
   `bypass_permissions_dialog.rs`).
3. **Focus pane**: implement the tool (validates pane name against a known set, returns
   the current focus). Register it. In the TUI, render the focused pane as a status
   indicator (small: a mode line / banner segment). If the TUI has no natural pane
   concept yet, scope to the tool + status-line indicator and note it.
4. Update `BUGS.md`; run suites.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-core --all-targets -- -D warnings && cargo clippy -p operant-cli --all-targets -- -D warnings
cargo test -p operant-core --lib terminal_hints && cargo test -p operant-core --lib focus_pane && cargo test -p operant-cli --lib slash_confirm
cargo test --workspace --all-features --lib          # final gate
```

## Test plan

- Hints: 7 unit tests, one per heuristic (`gh` unknown field, `command not found`,
  `ModuleNotFoundError`, `merge conflict`, `already exists`, gh rate limit,
  permission denied), each asserting the hint fires only for its pattern.
- Slash confirm: register/get/clear, `clear_if_stale` expires after timeout, per-session
  isolation.
- Focus pane: valid pane accepted, invalid pane rejected with the allowed set.

## Maintenance note

- Hint heuristics must stay pattern-based and additive (a new OS message may need a new
  rule) — keep them in the pure module.
- The slash-confirm store must never block the agent loop; it is TUI-side only.

## Escape hatches

- If a heuristic pattern is ambiguous (false positives), omit it and note the decision —
  hints must be trustworthy; a wrong hint is worse than none.
- If the TUI's input pipeline can't cleanly host the confirm store in one round, ship
  the store + tests and wire the first consumer in a follow-up round.
