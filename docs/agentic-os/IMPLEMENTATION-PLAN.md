# Implementation plan: operant as the organization's runtime

Status: **proposed (2026-10-14)**. Phases A–I below. Each phase needs the owner's approval to
start, and each lands as its own iterations under this repository's protocol (`AGENTS.md`):
iter-N from `origin/main`, push, and deploy after code changes. Holon-side packages (H1–H11) are
defined in Holon's `docs/design/2026-10-14-holon-operant-boundary.md` §5 and land in Holon's
repository under its laws (stdlib, tests first, `lint --strict`).

## Where the code goes

- **New crate `crates/operant-os/`**, behind the cargo feature `holon` (off by default) in
  `operant-cli`. It holds everything that talks to Holon:

  | Module | Owns |
  |---|---|
  | `api` | read-only client over `api_*` views and the `api_version` check |
  | `run` | the run protocol: prepare, heartbeat, finish |
  | `supervisor` | |
  | `runner` | |
  | `broker` | |
  | `policy` | `policy.json` loaded into a `SeatPolicySource` |
  | `tools` | native Holon tools |
  | `events` | triggers compiled for `operant-runtime::routines` |
  | `chair` | the channel command path |
  | `contract` | the vendored JSON Schemas and validation |

- **Kept org modules stay where they are** (`operant-core/src/org/`: `seat_policy`,
  `seat_budgets`, `context_injection`, `seat_authority`, the usage accumulator). Moving them buys
  nothing. Retired modules are deleted in phase G+1.
- **Config `[holon]`** (documented in `operant.example.toml` as each key lands):

  ```toml
  [holon]
  enabled = false                  # organization-runtime mode
  home = "~/holon"                 # the Holon instance ($HOLON_HOME)
  bin = "~/…/holon/bin/holon"      # pinned launcher
  supervise = false                # phase E: operant claims and runs Holon stages
  max_concurrent_runs = 2
  clock = false                    # phase E: operant fires `holon tick` instead of the systemd timer
  chair_channel = false            # phase F
  ```

## Phase A: operant as a Holon body (no operant code change)

- **Holon (H1):** `adapters/operant/` with:
  - driver `operant -c {posture} run --no-stream -s {identity} -q {briefing}` at a pinned binary;
  - an installer that writes `holon-<scope>-observe.toml` and `-act.toml`:
    - `[tools] disabled_tools` / `disabled_toolsets` for observe;
    - `[mcp.servers]` → `holon mcp`;
    - `[skills] root_dir` → the exported skills (Holon's `holon skill export`);
    - `[genome] unattended_posture = "lockdown"`;
    - `[client]` → the model;
  - a README of real-harness lessons.
- **Operant:** none. One gap to confirm: `[skills]` has a single `root_dir`. Either the installer
  links operant's own pool into the export directory, or phase B adds `[skills] extra_dirs`.
- **Accept:**
  - `holon adapter test operant` passes 5 of 5;
  - the Tools routine `Notes digest` runs on operant and is judged and verified;
  - `holon doctor` is clean.
- **Docs:** Holon `adapters/operant/README.md` and STATUS; `README.md` here (phase A done).

## Phase B: the read API and the views

- **Holon (H2):** a migration adding the `api_*` views and the `api_version` row; lint `api.views`.
- **Operant:**
  - `operant-os::api` opens Holon's databases with `mode=ro` and only reads `api_*`;
  - `operant org` subcommands become views (`ORG-LAYER-DISPOSITION.md` §3);
  - TUI panels Org, Board, Approvals, Playbooks and Objectives;
  - `[skills] extra_dirs` if phase A needed it.
- **Accept:**
  - a Rust test fails on any non-`api_*` table name or any write;
  - an `api_version` mismatch disables org mode with one clear doctor line;
  - views render on the workstation instance.

## Phase C: plan enforcement and budgets

- **Holon (H3, H5):**
  - `policy.json` written at claim (posture, roots, writes, commands, deny, budget);
  - `result.refusals[]` in the contract (blockers, not breaches);
  - `[budget]` token and cost caps.
- **Operant:**
  - `operant-os::policy` loads `$HOLON_RUN_DIR/policy.json` into an ephemeral `SeatPolicySource`
    consulted by the existing guard before every tool call;
  - refusals are told to the model and recorded;
  - `seat_budgets` enforcement reads `policy.json.budget`;
  - `escalate` and `Escalate` verdicts become refusals in a Holon run.
- **Accept:**
  - a write outside `roots` is refused and never executed, and appears in `refusals`; the Holon
    stage reworks instead of breaching;
  - a hard token cap ends the turn with a budget refusal;
  - personal mode's test suite is unchanged.

## Phase D: learning capture, trajectories, usage

- **Holon (H4):**
  - trajectory storage;
  - `Work.gold_standard` returns the verified result plus its trajectory; briefs and the judge get
    a compacted form;
  - `result.usage` is filled.
- **Operant:**
  - in a Holon run, background-review nudges, `skill_manage`, curator actions, `memory_store` and
    `kernel_refine` write `result.json` annotations (`learnings`, `skill_changes`, `proposals`)
    instead of files;
  - the trajectory recorder writes `$HOLON_RUN_DIR/trajectory.json`;
  - the usage accumulator fills `result.usage`, with the operant build (version and memory-wire
    revision);
  - the advisor's findings are added as `learnings` of kind `lesson` (low confidence).
- **Accept:**
  - a Holon run leaves `~/.operant/skills` and the memory store untouched, and the proposals are in
    the result;
  - a routine's gold standard includes its last verified trajectory;
  - `holon metrics` shows cost per verified result.

## Phase E: supervisor, runner, broker, clock, native tools

- **Holon (H6):** `holon dispatch --prepare --json` and `holon run finish` (kernel actor); one
  conformance suite for any supervisor.
- **Operant:**
  - `operant-os::supervisor`, a task in the operant daemon: for each employee whose driver is
    `operant` (or any driver, when operant supervises the instance), it:
    - prepares;
    - spawns through the runner;
    - heartbeats every lease/3;
    - waits;
    - salvages and finishes.

    Concurrency comes from `[holon] max_concurrent_runs` and Holon's `[budget]`.
  - `operant-os::runner` and `broker` are Rust ports of Holon's `isolation.serve_runner` /
    `serve_broker`, keeping the same socket protocol (SO_PEERCRED uid check, allowlist,
    `--home`/`--token` stripped, files read only from the request). They are served by the operant
    daemon as user services, and the runner layers `operant-runtime::security` sandboxes.
  - Native Holon tools in operant's registry replace `holon mcp` in operant postures.
  - The clock: a `no_agent` cron job firing `holon tick` every 2 minutes. The systemd timer stays
    off.
- **Accept:**
  - Holon's isolation tests (`tests/test_isolation.py`) pass against the Rust runner and broker;
  - the supervisor completes the Notes digest routine end to end with no Python process alive
    during the agent's run other than short `holon` calls;
  - memory measured per concurrent run is recorded in `README.md`.

## Phase F: the owner's channel and event routines

- **Holon (H7, H8):**
  - routine event triggers (`routine add --on event:<pattern>`), `api_triggers`, `holon routine
    fire` (kernel);
  - `[chair] channels`;
  - `holon notion apply`.
- **Operant:**
  - `operant-os::chair`: the gateway recognizes the owner's verified identity, classifies intent
    through the decision model (`[classifier]`, read from Holon's config), confirms, then runs
    `holon … ` as the Chair;
  - Holon notifications are delivered to the same thread;
  - `operant-os::events` compiles `api_triggers` into `operant-runtime::routines` matchers over
    channel, webhook and file events and calls `holon routine fire`;
  - the gateway hosts the Notion webhook route.
- **Accept:**
  - "approve <id>" from the owner's Telegram approves a pending plan (a Chair event); the same text
    from anyone else is a chat;
  - the broker refuses every Chair command;
  - a webhook matching a trigger fires its routine once per cooldown;
  - `holon-notion.service` is no longer needed.

## Phase G: one organization

- **Holon (H9):** `holon org import-operant` (dry run by default) maps the cast
  (`ORG-LAYER-DISPOSITION.md` §5) and re-proposes the cast crons as routines through the
  classifier.
- **Operant:**
  - archive the org stores (`ORG-LAYER-DISPOSITION.md` §4);
  - in org mode, disable every RETIRE module's code path: no cast seeding, no worklog or notices
    writes, no grants, no kanban for org work;
  - disable the cast cron jobs once their routines are approved;
  - `operant cron add` and the `cron` tool propose routines.
- **Accept:**
  - an org-mode day leaves no new row in any archived table;
  - every former cast duty has a Holon routine or kernel mechanism listed in `holon routine list`
    or the disposition;
  - personal mode is unchanged.
- **G+1 (one release later):**
  - delete the retired modules and their tests;
  - mark `ORGANISM-ARCHITECTURE.md` and `ORG-AUTHORITY-ARCHITECTURE.md` superseded, keeping them
    as history.

## Phase H: the harness improves under the organization's loop

- **Holon (H10):** the build recorded per run; judgments grouped by build; a regression opens
  incident work; smoke checks for operant departments.
- **Operant:**
  - `operant doctor --json` is stable enough to be a smoke check;
  - an "Operant" department (or Tools, scoped to this repository) takes `TODO.md` and `BUGS.md` as
    intake;
  - the iteration protocol in `AGENTS.md` becomes its playbook, refined from its own verified
    iterations.
- **Accept:**
  - a deliberately regressed build is detected from judgments and opens an incident;
  - one fleet iteration runs as Holon work (plan, approve, execute, verify) end to end.

## Phase I: one implementation of each runtime organ (owner decision)

When no Holon deployment runs without operant (the VPS runs ZeroClaw today), Holon retires its
Python runner, broker, MCP server and dispatcher spawn (H11). Until then both implementations pass
the same conformance suite.

## Order and dependencies

```
A ──> B ──> C ──> D
       └──> E ──> F ──> G ──> H
                          └──> I (owner)
```

A needs nothing from operant and proves the body. B is the foundation of every native part. C and D
make operant runs better than any other harness's. E removes the resident Python. F gives the owner
one place to talk to the organization. G removes the second organization.

## Prerequisites (owner)

- **A faster model for plan and execute stages.** Free-stack's `small-stack` times out at 120 s on
  operant's crons, and Holon's planner took 878 s per call. No phase fixes throughput.
- **The one-time isolation root setup** (Holon `docs/design/2026-10-13-trust-boundary.md` §3), before
  phase E.
- **`[genome] unattended_posture`** for operant's personal crons (now `yolo`).
