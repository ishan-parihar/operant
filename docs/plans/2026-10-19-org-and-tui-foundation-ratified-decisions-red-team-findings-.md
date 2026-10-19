# Org and TUI foundation: ratified decisions, red-team findings, and next phases

Dated-frozen: published `2026-10-19`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Mechanism

The owner asked for two things on 2026-10-19: brainstorm on the four decisions and
ratify them, then red-team the Holon seam, the agentic-core runtime and the TUI
interaction layer, and record what the red-team finds. This plan holds the
ratification, the options weighed for each decision, the findings, and the phases
that carry them. Owner rulings are recorded in `docs/kb/ASKS.md` through `genome q`.
The decision records AD-001 to AD-004 are immutable, so ratification does not edit
them; it is recorded here and in the worklog, with each record cited by its literal id.

### Ratification

| decision | ratified as | conditions |
|---|---|---|
| AD-001 — a body run writes no `sessions` row; attribution lives in trajectory metadata | ratified | none |
| AD-002 — the TUI org views read `HolonApi` in-process, read-only | ratified | the reason is the alternate-screen suspend and the need for refreshable state, not latency (see below) |
| AD-003 — the TUI keeps the default operant home; `HERMES_HOME` belongs to the Holon driver | ratified | the driver-form correction stands |
| AD-004 — org mode validates an explicit `/session <id>` against the roster | ratified for the explicit path only | the default binding is NOT covered; see G-1 |

### Options weighed

**AD-001, where body history is written.**
- (a) A body run writes a `sessions` row. Rejected: the session browser and `operant sessions` would list employee runs beside the operator's own chats, and the body home has no `database.db` to hold them.
- (b) Trajectory attribution only. Chosen. `holon_run_id` and `holon_employee` ride in `Trajectory.metadata`. The kernel `trajectory.json` keeps its shape, because `holon_trajectory_json` emits only `steps`.
- (c) A separate org-sessions table in the operant home. Deferred, not rejected. It becomes the candidate if the session browser must show employees; a later record would supersede AD-001.
- Accepted cost: body conversation text reaches operator-side storage only as trajectory steps. Trajectories written before this change carry `metadata: {}` and stay unattributed.

**AD-002, how the TUI reads Holon.**
- (a) Shell out to `operant org holon <x> --json`. Keeps one code path, but `/org` suspends the alternate screen on every use, and a panel that must refresh cannot leave the screen each time.
- (b) In-process `HolonApi`. Chosen. Structurally read-only (`mode=ro`), and it refuses an unknown `api_version`.
- (c) A Holon socket API. Not available; BOUNDARY §3.2 names the direct read.
- Measured on the live box, not assumed: `operant org holon board --json` exits 0 and takes about 13 ms per call. Latency therefore does not decide this. The decision rests on the alternate-screen suspend and on refreshability, and the two were not timed.

**AD-003, which home the TUI launches under.**
- (a) The default operant home. Chosen. The personal seat's stores stay with the operator, and the org views read Holon directly.
- (b) The Holon home. Rejected: it would put the operator's seat into the body stores that the driver writes under `HERMES_HOME`.
- (c) A per-launch home flag. Not added. No caller needs it yet.
- Verified: the gateway service environment carries no `HERMES_HOME`, and `operant_home()` falls back to `~/.operant`.

**AD-004, whether `/session <id>` is validated.**
- (a) Validate an explicit id against `api_employees` in org mode. Chosen. The live roster is Tess, Milo, Cara and Oto (ids `emp_01M…`). None of the ids bound today (`premiere` 26, `chief-of-staff` 13, `crew-chief` 6, `hrmaster` 3, `compass` 1, eight `emp-cron_*`) is a roster id.
- (b) Validate against a local employee registry. Rejected: no such registry exists, and Holon owns the roster.
- (c) Rebind stale rows to a roster id. Not done. It is an owner decision, and it needs a choice of target for each row.
- (d) Keep binding any id and let the seat policy resolve it. This is today's personal behaviour, and it is exactly what AD-004 refuses in org mode.

### G-1: the default binding contradicts AD-004 and the UI

Verified by reading, not reproduced on live data:
- `default_employee_binding()` returns `None` (`crates/operant-core/src/gateway_session.rs:450`). A new session is therefore written with a `NULL` binding, through `get_or_create_session` (`:976`) and the INSERT at `:848` and `:875`.
- The runner treats `NULL` as the unbound personal seat, with the personal memory bank (`crates/operant-cli/src/gateway_runner.rs:1256-1281`, the `else` arm).
- `/session` with no argument reports `This session runs as \`premiere\`` for that same row (`crates/operant-cli/src/gateway_commands.rs:835`, via `unwrap_or_else`). The no-entry message also promises "it will bind to `premiere`" (`:840`).
- The comments disagree with the code. The runner says the store backfills `premiere` and that `employee_id` is `Some` in every post-migration row (`gateway_runner.rs:1222-1224`). The migration says "the org backfill is gone… NULL = ungoverned" (`gateway_session.rs:429`).
- No live row is `NULL` today. The newest `gateway_sessions` row is from 2026-10-15T02:16, so the `NULL` path has not run on the live gateway. The defect is in the code, and no data shows it yet.
- Under AD-004, `premiere` is not on the roster, so a default of `premiere` binds an unbacked id in org mode.

Options for the default:
- (a) Keep `premiere` as the default. Conflicts with AD-004.
- (b) A new `[holon] default_employee` key naming a roster id. Needs an `operant.example.toml` entry and an owner choice of employee.
- (c) New chats stay unbound, and `/session` reports that truthfully: "unbound — personal seat". Matches what the code does now, and needs no config.

Recommendation: (c) now, because it fixes a misstatement with no new configuration. (b) follows once the owner names the roster employee.

## Red-team findings

Three read-only lanes ran on 2026-10-19: the Holon seam (`crates/operant-os`, and the Holon-side branches of `cmd_org.rs`), the agentic-core and gateway seat (`gateway_session.rs`, `gateway_runner.rs`, `cronjobs/scheduler.rs`, `memory_provider.rs`), and the TUI interaction layer (`crates/operant-cli/src/tui/`). Each finding is a row in the `OPEN — Red-team: Holon seam, agentic-core seat and TUI (2026-10-19)` section of `BUGS.md`, with its file:line, a dedupe note and its verification level.

| lane | rows | H | M | L | ids |
|---|---|---|---|---|---|
| Holon seam | 11 | 6 | 2 | 3 | OS-1 to OS-11 |
| agentic-core and gateway seat | 9 | 4 | 5 | 0 | GW-1 to GW-9 (GW-9 is the G-1 defect above) |
| TUI interaction layer | 13 | 4 | 8 | 1 | TUI-1 to TUI-13 |

Verification levels. "Verified in code" means the file:line was re-read in this pass. "Read in the lane" means a red-team agent traced it and this pass did not re-read it. Nothing was run to reproduce a finding: no body was launched, and no live state was written. The read-only live checks were `gateway_sessions` (57 rows, all bound, newest 2026-10-15T02:16) and the `chief-of-staff` budget rows behind GW-1.

Not verified, and recorded as such: OS-1 depends on the supervisor's launch environment. GW-4's timing was not reproduced. TUI-6's trigger (`operant update` replacing the binary) is an inference. TUI-10's Ctrl-C effect is an inference. TUI-9 does not check whether the core emits a completion for a cancelled delegate.

Severity changed from the lane ratings: TUI-1, TUI-5, TUI-7 and TUI-8 went from H to M. TUI-1 stalls and does not bypass, since the browser swallows the approval keys. TUI-5 keeps the old path. TUI-7 changes only a message, and TUI-8 changes only a title. One TUI-4 sub-claim (a `tools:` continuation line overriding the list) is not recorded, because it was not verified.

Folded and known: GW-9 is the G-1 defect. OS-6 and OS-10 extend O-12 (`docs/agentic-os/AUDIT-2026-10-15.md:260-261`, group kill on timeout). OS-1 is outside O-07, which covers only the environment-variable path. OS-2 is outside O-11, which covers only the isolated path. OS-8 is new against O-04 and O-09. TUI-1's ordering divergence is known and deferred (`docs/ROADMAP-TUI-FLEET.md` §6.1); the stalled-approval consequence is new.

## Phases

Each finding becomes a board item and, where autonomous mode can take it, a TODO row. The boards are `holon-seat-redteam-2026-10-19` (17 rows), `tui-redteam-2026-10-19` (12 rows) and `org-tui-foundation` (2 open).

**Phase 0, ratify and record (this iteration).** AD-001 to AD-004 are ratified (see Ratification). The red-team rows, both boards, the TODO rows and this plan are committed together and pushed to `origin/main`.

**Phase 1, autonomous-safe fixes (TODO `## Pending`, 23 rows).** Each row names its BUGS.md id and the test that proves the fix. The order follows what each fix protects. Wrong data comes first: TUI-3 (overwrite), TUI-4 (truncation), TUI-2 (wrong session), GW-2 (cron memory scope), GW-3 (stale seat). Then the Holon safety checks: OS-5 (notify exit), OS-7 (clean-stop masking), OS-6 (signal forwarding), OS-4 (observe deny list). Then the rest.

**Phase 2, owner decisions (TODO `## Review`, 9 rows).** No code for these until the owner rules. The order is the order they block work. First G-1's default employee (`org-tui-foundation-003`). Then GW-1 budget attribution, which blocks any budget change. Then OS-1 and OS-2 isolation, the largest exposure. Then OS-3 fail-closed in org mode, GW-6, GW-8, TUI-1 and TUI-11. OS-11 is a Holon-repo change and is listed for the Holon side.

**Phase 3, verify and ship.** Operant changes ship through the local release procedure in AGENTS.md. Installing the binary (`sudo cp` and `mv` to `/usr/local/bin/operant`) is the owner's to run. Before and after numbers: the `chief-of-staff` budget rows for GW-1 (the snapshot of 8,232,833 tokens across 3 rows), and an unbound `/session` report for G-1.

## Gates / success criteria

1. **Each Phase 1 fix.** The test named in its TODO row fails before the change and passes after it. Then `cargo fmt --all`, `cargo check --workspace`, `cargo test --workspace` and `bash scripts/clippy-warning-gate.sh` pass. The BUGS.md row is marked fixed with its iteration id in the same commit.
2. **Board close.** A board item closes with `genome done <id> done --evidence <path>` only after its test is in the tree.
3. **Owner rulings first.** A Phase 2 ruling is recorded with `genome q` before the code that depends on it. A ruling that changes a ratified decision is a new AD record, not an edit to an existing one.
4. **Contract changes stop.** A Phase 1 fix that changes a Holon contract (kernel fields, notify semantics, the driver) moves to Review instead of being done autonomously.
5. **Two failures stop the row.** If a gate fails twice on the same row, the row returns to Review with the output as evidence.

## Cost

Not priced. No token or wall-clock estimate was measured in this pass, so the sizing unit is the row: 23 Pending rows, 9 Review decisions and 31 open board items. A board item can cover more than one BUGS.md row (the TUI-12 item covers TUI-12 and TUI-13).

The cost of not fixing is in the evidence:
- GW-1: `chief-of-staff` has 3 rows updated since 2026-10-14 totalling 8,232,833 tokens, and 4,111,430 of them are on rows created before that date. Per-employee totals move with the binding, so they are wrong in both directions.
- GW-8: a locked database lifts the budget cap for that turn, and cron does the same.
- OS-3: with no `policy.json`, org-mode enforcement is a silent no-op.

Outside this repo: OS-11 needs a change in the Holon repo. Owner-only: the binary install step, and the Phase 2 rulings.

