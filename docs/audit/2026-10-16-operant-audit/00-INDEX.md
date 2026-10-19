# 2026-10-16 operant audit — index for the executor agent

**Written for:** the agent that implements the fixes. Read this file, then
`01-ponytail-audit.md` (the ranked findings), then the plan file for the item
you pick up. Each plan file stands alone: evidence with `file:line`, root
cause, the smallest fix, the test to write first, and the acceptance check.

**Ground state:** `main` @ `0ee25739` (iter-858/859), 2026-10-16. Line
numbers come from that tree. If they drift, grep the quoted symbol.
Installed binary: `operant 0.2.1 (memory-wire 11c2259)`. (`CLAUDE.md` still says
release line `0.1.4`. That is doc drift; fix it when you touch release docs.)

**How this audit was made:** I read the code paths in full and confirmed the
TUI behaviour with the headless simulator on a fresh debug build (`target/debug/operant`,
2026-10-16 06:03). I compiled the test targets with
`cargo check --offline --keep-going --tests -p operant-core -p operant-cli -p operant-os`.
I made no code changes.

## Files

| # | File | What it fixes | Size |
|---|---|---|---|
| 01 | [01-ponytail-audit.md](01-ponytail-audit.md) | The ranked findings (20), in the ponytail-audit format | — |
| 02 | [02-test-gate-repair.md](02-test-gate-repair.md) | 10 test targets do not compile, so no gate is real | S |
| 03 | [03-tui-agent-state-sync.md](03-tui-agent-state-sync.md) | `/clear` `/new` `/undo` `/retry` `/stop` `/fast` `/plan` `/yolo` `/permissions` only change the screen, not the agent; concurrent runs; recap | M |
| 04 | [04-slash-commands-one-registry.md](04-slash-commands-one-registry.md) | Two command tables that disagree; dead and lying commands; CLI-subcommand parity matrix | M |
| 05 | [05-model-picker-jcode-inline.md](05-model-picker-jcode-inline.md) | `/model` shows the wrong catalogue and never switches the agent; port jcode's inline picker; retire legacy overlays | L |
| 06 | [06-gateway-concurrency-and-seat-memory.md](06-gateway-concurrency-and-seat-memory.md) | Concurrent chats share one agent's conversation, model and stop flag; role-specific memory banks (11E remainder) | M |
| 07 | [07-native-scroll.md](07-native-scroll.md) | Native-scroll watermark bugs + the remaining plan-09 steps, ordered | L |
| 08 | [08-memory-unification.md](08-memory-unification.md) | `operant memory`, doctor, `/whoami`, `/journey` still use MEMORY.md; agentmemory defaults warn on every start | M |
| 09 | [09-config-writes.md](09-config-writes.md) | The TUI rewrites the whole `operant.toml` (comments, base_url, secrets); `operant config set` / `model set` persist nothing | S |
| 10 | [10-crate-and-dead-code-purge.md](10-crate-and-dead-code-purge.md) | Plan 11D with evidence: ~176k lines in crates the binary never calls | L |
| 11 | [11-batch-redirection.md](11-batch-redirection.md) | Answers to the 2026-10-15 re-audit brief: which batch plans are still the right work | — |

## Execution order

Correctness and safety come before features, and the test gate comes before
everything (without it no change can be verified).

1. **02** test gate (nothing else is verifiable until this is green)
2. **06 part A** gateway turn lock (cross-user data mixing, live on the workstation gateway)
3. **03** TUI ↔ agent state sync (context lies, concurrent runs, cosmetic permission mode)
4. **09** config writes (data loss / plain-text secrets)
5. **05 part A** `/model` switches the agent and lists the configured endpoint's models
6. **04** one command table
7. **07 steps 1–3** native scroll correctness (reset + replay)
8. **08** memory unification, then **06 part B** seat memory banks
9. **05 part B** jcode inline pickers replace the legacy overlays
10. **07 steps 4–7** native scroll quality bar and default flip
11. **10** crate purge (last, so deletions don't race the work above)
12. Batch plans 06 / 07.2 as re-scoped in **11**

## Working conventions (unchanged from the batch, restated so you don't miss them)

- Work in a worktree from `origin/main`, never in the shared tree. The shared
  tree has peer uncommitted edits right now (deleted
  `crates/operant-core/src/tools/authority_tools_tests.rs`, deleted
  `rewind-flow` scenario files, modified `Cargo.lock`, an `xd:/` stray dir).
  Do not commit those.
- One iteration = one commit = one push, label `iter-N` from
  `git fetch && git log origin/main --oneline | grep -o 'iter-[0-9]*' | head -1` + 1.
  Check `git log origin/main --oneline | grep -c "iter-N"` == 1 after you push.
- Build through `scripts/build.sh` (it syncs memory-wire first). `export AR=/usr/bin/ar`.
- Gates every iteration (once 02 lands they are real again):
  `cargo fmt --all` (revert unrelated churn) →
  `cargo test -p operant-cli --bin operant` →
  `cargo test -p operant-core --lib` (only K-1 may fail) →
  screen corpus `cd crates/operant-cli/tests/tui_scenarios && OPERANT_BIN=<copy> python3 _run.py verify` →
  clippy parity (`bash scripts/clippy-warning-gate.sh`).
- Test first: the new test must be RED on the old code.
- Same commit: `docs/CHANGELOG.md` (Unreleased), `README.md` if user-facing,
  `operant.example.toml` if a config field changes, `TODO.md`, `docs/audit/2026-06-19-BUGS.md` for
  anything found and not fixed.
- No `.unwrap()`/`.expect()` in production code (AGENTS.md "Rust Code Standards").
- **Never run `tui debug simulate` without `--agent-script` while
  `OPENAI_BASE_URL` points at a live gateway.** It sends real requests (finding
  16). Use `env -u OPENAI_BASE_URL -u OPENAI_API_KEY` or a mock script.

## Verdict (short)

The agent core is solid, but the TUI and the CLI often say they did something
they didn't do. The TUI's model, permission, conversation and branch state is
a parallel copy that never reaches the agent. The gateway runs different chats
at the same time on one agent with one conversation slot. The test gate has
been red since the org layer was retired, so recent "green" claims could not
have run. About a quarter of the workspace is in crates the binary never calls.
Fix the gate, then the state sync, then delete.
