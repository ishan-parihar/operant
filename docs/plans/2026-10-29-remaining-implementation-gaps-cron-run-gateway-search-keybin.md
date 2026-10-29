# Remaining Implementation Gaps — cron run, gateway search, keybinding dispatch, reasoning unify, tool smoke

Dated-frozen: published `2026-10-29`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**Minted:** 2026-10-29 (genome `plan`)
**Author:** agent-omarchy (the red-team/audit agent; the same agent that landed iter-1467 + iter-1472)
**Inputs:** `docs/audit/2026-06-19-BUGS.md` (the ledger), `TODO.md` `## Pending`,
`docs/kb/workflows/remaining-gaps-1364.md` (the live claim board), and
`docs/audit/2026-10-28-rearchitecture-red-team-session-panes-and-jcode-parity.md`
(the session audit whose R1/MMD items closed as iter-1467/1472).

## Why this plan exists

The unified-session and red-team work is DONE (iter-1467 deleted the second
pane; iter-1472 made the gateway view honest). What is left is a sparse set of
gaps that no agent currently owns. This plan enumerates them from the
authoritative ledgers, verifies each against source, and sequences them so the
cheapest and highest-trust-value items land first.

**Every item below was verified against the current tree** (2026-10-29 01:05,
origin/main `18261018`), not assumed from ledger prose.

## The verified gap inventory

| id | gap | verified state | cost | value |
|---|---|---|---|---|
| **G1** | `operant cron run <id>` never runs | `cmd_cron.rs:564` prints "Manual execution via CLI is not yet implemented" after marking triggered | high | high |
| **G2** | gateway `/search` has no live arm | `gateway_commands.rs:2288` is the only arm and it redirects to the CLI; the resolver at `:513` accepts the command, so the reply is a pointer, not a search | medium | high |
| **G3** | `[keybindings]` overrides read-only | `key_handling.rs` is an if-chain; the registry is a catalogue | high | medium |
| **G4** | gateway `show_reasoning` un-unified | `gateway_commands.rs:1733` reads `config.agent.show_reasoning`; the TUI's 3-state mode is a process-global cell (`config_shim.rs:288`) | medium | medium |
| **G5** | 22 unexercised tools (RT-196) | claimable, currently unclaimed on the peer board | medium | medium |
| **G6** | 56 corpus drifts | the peer's `feat(iter-1461)` consequence; baselines pinned 2026-10-10/19 | low | high |
| **G7** | Ctrl+Tab model-cycle alias | `providers.rs:381` already has `cycle_configured_model`; no key arm | trivial | low |

## Out of scope (recorded so it is not re-litigated)

- **G6 baseline regeneration.** Fleet rule 7: the tip's red is the peer's
  in-flight subsystem; fixing it from another slice is how oscillation starts.
  It is documented in the iter-1467 commit body, the feed (F0162) and here.
- **The TUI shell-outs** (`/cron`, `/kanban`, `/curator`, `/org`) getting
  gateway twins — "use the CLI" is the correct answer for a subprocess launcher
  (RT-179's own verdict).
- **RT-135's `/usr/local/bin` sibling** — CR-6, shadowed and inert, `sudo`
  unavailable. Not code.
- **Memory bank topology (RT-190 option b)** — an owner ruling, explicitly not
  an agent decision.
- **`operant tui keybindings` opening a dead file** — already fixed (iter-1133,
  RT-146).

## Sequencing rationale

The gate is trust, not feature count. G1 and G2 are both *a surface that says it
does something and does not*, which is the exact defect class this audit family
tracks (RT-175 found eleven of them). G1 is the harder of the two because
`CronScheduler` requires a live `Arc<OperantAgent>` the CLI does not build, so
it is a feature with a real cost. G2 is the cheaper and lands first.

## Phase plan

### Phase 0 — claim discipline (before any code)
`genome wf add remaining-gaps-1364` rows already exist. Claim G5 from the
peer's board (it is `open`, unclaimed). Do NOT claim `remaining-gaps-1364-001`
(the peer holds it). Post a claim on the feed naming the slice.

### Phase 1 — G2: gateway `/search` gains a live arm
Search the agent's own store through the seam `/memory` already proved
(`run_session_scoped_command` + `agent.conversation()`). Bare `/search` lists
recent sessions; `/search <q>` filters by title/query. Sync arm names the live
command (unchanged — it is already honest).
*Gate:* resolver answers `Found`; one mutation-proven test.

### Phase 2 — G1: `operant cron run <id>` actually runs
Add `CronScheduler::run_job_now(id)` that fires the one job through the same
dispatch path `tick()` uses (so seat/budget/delivery/observer all apply), then
have `cmd_run` build the agent and call it. The distinguishing detail: the CLI
must NOT mark the job run before the run succeeds — the current code marks
`triggered_manually` then prints "not implemented", so a failed run still
records a trigger. Invert that ordering.
*Gate:* one test per outcome (success marks success; failure leaves the row
markable and reports the error); estop still refuses (mirror `tick()`).

### Phase 3 — G4: unify gateway reasoning with the TUI mode
The TUI's mode is a 3-state contract (`Off`/`Current`/`Full`) over a
process-global cell; the gateway reads one bool. Replace the bool read with the
same resolver used at TUI boot (`main.rs:3249`) so the gateway's answer cannot
drift from the operator's setting. Do not invent a second table.
*Gate:* one test pinning the resolver, and the existing `/reasoning` test suite.

### Phase 4 — G5: the 22-tool smoke pass (claimed from the peer board)
Side-effect tools (`send_message`, `react_to_message`, `image_generate`) behind
an explicit consent flag; read-only tools exercised directly. Each failure is
either a real bug (fix) or a description defect (fix + contract pin) — the
description half of this row is already closed (iter-1363), so this is the
execution half.

### Phase 5 — G7: the Ctrl+Tab alias (trivial, last)
One arm aliasing `cycle_configured_model`. No new cycle state (TODO.md's own
instruction: "do NOT grow it a new cycle state").

### Not scheduled: G3 (keybinding dispatch)
G3 is a refactor of the dispatcher, not a feature. It is the single riskiest
item in this inventory (key_handling.rs is contended with the peer), and it
delivers no capability the operator asked for. It stays in TODO.md with its
blocker note until someone has the tree to themselves.

## Verification contract (every phase)

- `cargo fmt --all`
- `./scripts/check.sh check -p operant-cli --bin operant`
- `./scripts/check.sh test -p operant-cli --bin operant` (full bin sweep; note
  the two known order-dependent flakes: `wizard_persist_is_surgical` and
  `cmd_context::tests::sessions_report_lists_multiple_sessions` — both pass
  isolated, neither is touched by this plan)
- `bash scripts/clippy-warning-gate.sh`
- Code phases deploy: provenance-gated release build, `~/.local/bin`, gateway
  restart, `/proc/<pid>/exe` three-way md5. `/usr/local/bin` documented as
  sudo-blocked, never silently skipped.
- Every phase is its own iteration, labelled from `origin/main` AFTER a
  `git fetch origin` — the numbering authority is the remote ref, and a stale
  local mirror already cost this tree one phantom base this week.
- Shared-tree discipline: `key_handling.rs`, `tests.rs`, `gateway_commands.rs`
  are contended. Hunk ranges checked disjoint before staging; path-limited
  `git add`; never a bare `git add -A`; never stash/rebase over peer WIP.

## Definition of done

G2, G1, G4, G5, G7 closed with regression tests and deployed; G6 documented as
the peer's; G3 left in TODO.md with its blocker. BUGS.md rows for whatever this
closes are edited IN THE SAME ITERATION that closes them.
