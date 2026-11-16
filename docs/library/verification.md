# Verification — the harness that proves work

Owns: operant's verification harness — recipe detection, the phase model,
the evidence ledger, the agent-facing `verify_task` tool, and the global
emergency stop (ESTOP) that halts scheduled work.
Not for: the guardrail/approval stack ([security.md](security.md)), the
autonomous loop that consumes verification results
([autonomous.md](autonomous.md)), the agent loop that calls the tool
([agent-loop.md](agent-loop.md)), the SQLite layer
([persistence.md](persistence.md)).

Source: `crates/operant-core/src/verification.rs` (recipes, phases,
runner), `crates/operant-core/src/tools/verification_tool.rs`
(`verify_task`), `crates/operant-core/src/estop.rs` (ESTOP),
`crates/operant-core/src/database.rs` (the `verification_events` table),
`crates/operant-core/src/agent/run.rs` + `crates/operant-core/src/gateway/mod.rs`
+ `crates/operant-core/src/cronjobs/scheduler.rs` (ESTOP call sites),
`crates/operant-cli/src/cmd_cron.rs`, `crates/operant-cli/src/cmd_kanban.rs`,
`crates/operant-cli/src/cmd_status.rs`, `crates/operant-cli/src/gateway_commands.rs`
(operator surfaces).

## The phase model

`verification.rs` detects a project's recipe from its manifests and runs
it as ordered phases. Five phase kinds exist; four are recipe phases and
one is opt-in:

| phase | source | default timeout |
|---|---|---|
| `bootstrap` | recipe `bootstrap` commands (install/prepare) | 120 s (`BOOTSTRAP_TIMEOUT`) |
| `build` | recipe `build` commands | 300 s (`DEFAULT_PHASE_TIMEOUT`) |
| `test` | recipe `test` commands | 300 s |
| `start` | a `StartSpec` the caller supplies (`{command, port, path}`) | 60 s readiness poll (`START_READY_TIMEOUT`, 1 s interval) |

The rules that make a result meaningful:

- **A run with no `test` phase is not ok.** `run_verify` computes
  `ran_test_phase = phases.iter().any(|p| p.name == "test")` and sets
  `ok = ran_test_phase && all phases ok`. A build-only or start-only run
  can look green while never exercising the behavior it claims to
  verify. `verify_task` with `phases: ["build"]` therefore returns
  `overall_ok: false` with a passing `build` row — there is a regression
  test for exactly that (`a_build_only_run_reports_not_ok_but_records_its_phase`).
- **Every phase runs in its own shell** (`sh -c`), with stdout and stderr
  captured concurrently into a bounded tail (`OUTPUT_TAIL_CHARS` = 4000,
  prefixed with `[...truncated N chars]` when it overflows). A timeout
  prefixes `[timed out after <dur>]` and marks the phase not ok.
- **The start phase owns a process group.** The child is spawned with
  `process_group(0)` on unix; readiness success, timeout, and teardown all
  call `kill_process_group`, which sends `-TERM` to the group, waits
  100 ms, then `-KILL`. Killing only the shell would leave background
  descendants alive after the run.
- **`kill_on_drop(true)`** on every phase child, so a `verify_task` future
  cancelled mid-phase does not orphan the process.

## Recipe detection

`detect_recipe(root)` is a first-match ladder over manifests in the
project root, in this order:

| manifest | recipe | bootstrap | build | test |
|---|---|---|---|---|
| `Cargo.toml` | `cargo` (rust) | — | `cargo build` | `cargo test` |
| `go.mod` | `go` | — | `go build ./...` | `go test ./...` |
| `package.json` | `node` | `<pm> install` | `<pm> run build` if the script exists, else `build` for non-npm | `<pm> run test` if the script exists |
| `pyproject.toml` / `requirements.txt` / `setup.py` | `python` | `uv sync` / `poetry install` / `pipenv install --dev` / `pip install -e .` (from the lockfile) | `python -m compileall -q .` | `python -m pytest -q` |
| `pom.xml` | `maven` (java) | — | `mvn -q compile` | `mvn -q test` |
| `build.gradle` / `build.gradle.kts` | `gradle` (java) | — | `gradle build -x test` | `gradle test` |
| `Makefile` / `makefile` | `make` | — | `make build` | `make test` |
| `docker-compose.yml` / `.yaml` | `docker-compose` (docker) | — | — | — |

Node's package manager comes from the lockfile: `pnpm-lock.yaml` → pnpm,
`yarn.lock` → yarn, `bun.lockb`/`bun.lock` → bun, else npm. A `build`
script that exists in `package.json` is run as `<pm> run build`; when it
does not exist, non-npm managers still get `<pm> run build` (which fails
honestly) while npm gets nothing.

`find_project_root(start)` walks UP from the given directory looking for
`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `pom.xml` or
`Makefile` (note: **not** `build.gradle` or `docker-compose.yml`, so a
Gradle- or compose-only directory is its own root), and returns `start`
unchanged if nothing matches.

## The evidence ledger

Every phase of every `verify_task` run is persisted, so a claim can be
cited later. The table is `verification_events` in the state database
(created by `Database::create_verification_events_table`, indexed on
`created_at DESC`):

| column | type | meaning |
|---|---|---|
| `id` | INTEGER PK | autoincrement |
| `recipe` | TEXT | recipe name (`cargo`, `ad_hoc`, …) |
| `phase` | TEXT | phase name (`bootstrap`/`build`/`test`/`start`/`check`) |
| `ok` | INTEGER | 1/0 |
| `exit_code` | INTEGER NULL | the command's exit code, null on spawn failure |
| `output_tail` | TEXT | bounded stdout+stderr |
| `created_at` | TEXT | RFC 3339 |

`record_verification_event` writes one row per phase;
`list_verification_events(limit)` reads the newest `limit` rows
(`limit` is clamped to 1..=200). A write failure is **logged, not
dropped**: `record_phases` emits a `tracing::warn!` naming the recipe and
phase when the row could not be written (RT-292 — the old `let _ =` made
a missing evidence row invisible while the tool still reported success).

## The `verify_task` tool

`crates/operant-core/src/tools/verification_tool.rs` registers
`verify_task` (in `builtin_tool_names()`, so `operant tools list` and
`tools disable` both see it). It has four actions:

| action | arguments | result |
|---|---|---|
| `detect` | `cwd?` | the detected recipe, or `{detected: false, hint}` naming `ad_hoc` |
| `run` | `cwd?`, `phases?`, `start?` | `overall_ok` + per-phase rows |
| `ad_hoc` | `checks: [{cmd, expectedExit?, expectContains?}]` | one `check` row per entry + `overall_ok` |
| `evidence` | `limit?` (default 10) | `{count, events}` from the ledger |

`cwd` defaults to the process CWD; the tool then calls
`find_project_root`, so `cwd: "crates/operant-core"` resolves to the
workspace root. `phases` is a SUBSET filter over `bootstrap|build|test`
— an unknown phase name is an error listing the three valid names, never
a silent "run everything" (RT-290). The `start` phase is requested by
supplying a `start` object, not by naming it in `phases`.

`run_check` (the `ad_hoc` path) runs the command and then applies the
assertions: `expectedExit` defaults to 0, and `expectContains` is a
substring that must appear in the captured tail. A missing needle is
recorded in the tail as `[missing expected output: "…"]`.

## ESTOP — the operator's brake

`estop.rs` is a sentinel-file pause. `operant pause [reason]` writes
`<data_dir>/ESTOP` containing optional JSON
`{"reason": ..., "engaged_at": <unix millis>}`; `operant resume`
removes it. The check is one `Path::exists()` per call, so it is safe to
run every tick with no caching, and engage/disengage takes effect on the
very next check.

**Fail-safe by construction.** Any present file means engaged — a bare
`touch ~/.operant/ESTOP` or a corrupt/truncated JSON body still reports
`engaged: true` (with `reason: None`). The pause must hold even when the
file was not written by operant.

**New work only, and never in-flight work.** The gates:

| call site | behavior while engaged |
|---|---|
| `agent/run.rs` pre-model-call | the turn HOLDS in a 1 s poll loop before the next model call |
| `agent/run.rs` pre-tool-batch | the turn HOLDS before the tool batch fires |
| `gateway/mod.rs::route_message` | a new turn gets "⏸️ Operant is paused… Send /unpause here, or run `operant resume`" instead of an agent run |
| `cronjobs/scheduler.rs::tick` | due jobs are not dispatched (`debug!` + `Ok(())`) |
| `cronjobs/scheduler.rs::run_job_now` | a manual run returns an error naming `operant resume` |
| `cmd_cron.rs` (`run`) | the CLI manual-run path refuses, printing "Resume with `operant resume`" |
| `cmd_kanban.rs` dispatch | no new task is claimed; prints "Resume with `operant resume`" |
| `cmd_status.rs` | `operant status` reports the ESTOP state (reason + engaged_at) |

`crates/operant-core/tests/pause_gate.rs` pins that the gate is consulted
at exactly those boundaries. Note the agent loop's two holds mean an
in-flight turn does not die mid-write: it stops at the next action
boundary and waits.

The clear command in a channel is **`/unpause`**, not `/resume`
(`/resume` already means "resume a saved session"); the CLI form is
`operant resume`. Both `operant pause` and `/pause [reason]` record a
reason; the gateway's default reason names the user and platform.

## Configuration

The harness itself has no config keys — `verification.rs` is
constants-only (`DEFAULT_PHASE_TIMEOUT`, `BOOTSTRAP_TIMEOUT`,
`START_READY_TIMEOUT`, `READY_POLL_INTERVAL`, `OUTPUT_TAIL_CHARS`) and
reads no `AppConfig` section. What an operator can change is what the
`verify_task` tool is ASKED to do (`phases`, `start`, `checks`) and, for
the ESTOP, the sentinel file at `platform::operant_data_dir()` — which
follows the one home resolver (`HERMES_HOME` → active profile →
`~/.operant`). The autonomous loop's `[autonomous] test_command` is the
validation gate that sits on top of this harness; see
[autonomous.md](autonomous.md).

## Surfaces

- Tool: `verify_task` (actions `detect` / `run` / `ad_hoc` / `evidence`).
- CLI: `operant pause [reason]`, `operant resume`,
  `operant status` (ESTOP row), `operant cron run <id>` (refuses while
  engaged). **`operant cron resume <id>` does NOT re-arm the scheduler** —
  it flips one paused job back to `state = "scheduled"`, and nothing in
  `cmd_cron.rs` calls `estop::disengage`. The scheduler's own refusal text
  (`cronjobs/scheduler.rs:304`) names `operant cron resume` anyway, so an
  operator who follows it stays stopped; the command that clears an ESTOP
  is `operant resume`.
- Channels: `/pause`, `/unpause`, and the paused-turn reply that names
  `/unpause`.
- Database: `verification_events`, readable through
  `verify_task action="evidence"`.

## Failure modes

- **No recipe detected** → `detect` says so with the manifest list and
  names `ad_hoc`; `run` is an error naming the same fallback.
- **A phase cannot spawn** (missing binary, bad cwd) → that phase is a
  `PhaseResult` with `exit_code: None` and `output_tail: "Failed to spawn:
  …"`; the run is not ok.
- **A phase times out** → not ok, tail prefixed with the timeout.
- **The start phase never answers HTTP 200** → not ok, and the tail
  carries the last error ("HTTP 500", a connection error, or "not yet
  polled").
- **The app boots but the readiness probe passes on the first poll** →
  the group is killed and the phase records the elapsed boot time.
- **A build-only run** → `overall_ok: false` with every row green. This
  is intended.
- **An evidence write fails** → the tool still returns its result, but a
  warning names the recipe and phase. The ledger is missing the row; the
  log says so.
- **ESTOP engaged mid-turn** → the turn holds at the next boundary
  (1 s polls) until `operant resume`. Nothing is killed.

## Intention

The harness exists because "the agent says it is done" is not evidence.
Recipe detection from manifests rather than a configured command keeps it
zero-config across arbitrary projects; the phase model mirrors what a
human run would do; the `test`-phase requirement exists because a green
build is not a verified behavior; and the `verification_events` ledger
makes every check citable after the fact, which is what lets the
autonomous loop and the org layer reason about what was actually run.

The ESTOP is the operator's guarantee that a self-directed loop can always
be stopped at a boundary rather than mid-write: it pauses new work, holds
a running turn at its next action boundary, and fails safe if the
sentinel is damaged. That is why it is a file and not a signal — a file
survives a restart, works from any process on the machine, and cannot be
lost in a signal mask.
