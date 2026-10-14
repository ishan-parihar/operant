# Boundary: what operant owns as an agentic OS

Status: **proposed (2026-10-14)**. The canonical rule and the full ownership table are in Holon's
`docs/design/2026-10-14-holon-operant-boundary.md`. This file is operant's side: what operant must
do natively, in Rust, so that the organization runs *in* it, and what it must not keep, so that
there is only one organization.

## 1. The rule in one paragraph

Operant is in Rust to be a full-featured, low-resource agentic runtime. A capability belongs in
operant when it is:
- **resident**: daemons, supervisors, servers, watchers;
- **per turn or per tool call**: enforcement, metering, trajectories, fitting context;
- **acting on the OS for agents**: processes, Unix users, sandboxes, sockets;
- **real-time with the outside world**: channels, webhooks, MCP.

A capability belongs in Holon's Python only when it is a decision about the organization, made
per stage or per tick, run as a short-lived process (zero resident memory), and rewritten by the
organization's own improvement loops (the reason it must change without a compile).

Measured on the workstation:

| Process | Peak memory | Resident? |
|---|---|---|
| a Holon command | 28–44 MB | no: it exits in 0.5–2 s |
| one operant model turn | 40 MB | for the run |
| a Hermes body, before its first model call | 100 MB | for the run |

So the memory saved is in the body and in the per-run helpers, both of which come to operant.

## 2. Operant's two modes

| Mode | When | Organization state | Behavior |
|---|---|---|---|
| **Personal agent** | `[holon] enabled = false` (default) | none | today's operant: chats, personal skills (`~/.operant/skills`), personal memory, personal crons; ungoverned seats behave exactly as before |
| **Organization runtime** | `[holon] enabled = true` and Holon's `api_version` matches | **all** of it from Holon, read through the read API; reported back through the run contract and the `holon` CLI | below |

There is no third mode. Operant keeps no org store of its own in either mode. The org tables
operant has today are archived (`ORG-LAYER-DISPOSITION.md` §4).

## 3. What operant does natively (organization-runtime mode)

Nothing here is a thin shim: these are the organs of the OS, in Rust.

### 3.1 The body

- **Employees run in operant.** A Holon employee whose driver is `operant` runs as an operant
  session bound to that employee:
  - identity card and stage brief injected;
  - the employee's department skills and playbooks rendered (from `holon skill export`);
  - posture by stage: observe (read-only tool set) for research, plan and verify; act for execute.
- **Chat sessions bind to employees.** `gateway_sessions.employee_id` holds a Holon employee id.
  The owner's default session binds to the executive head (the CEO). Rolling session summaries
  stay operant's: session continuity is runtime.

### 3.2 Run supervision, runner, broker

- **Supervisor.** It claims ready stages through Holon (`holon dispatch --prepare`), spawns the
  body, heartbeats the lease, waits, salvages and hands in (`holon run finish`). One task inside
  the operant daemon replaces one resident Python dispatcher per run. Concurrency limits come from
  Holon's `[budget]`.
- **Runner.** It starts agent processes as the agent Unix user, with the `operant-runtime`
  sandbox backends (landlock, bubblewrap, firejail) layered on top. It serves every harness, not
  only operant, which makes Holon's Python runner the fallback.
- **Broker.** It serves the organization to isolated agents:
  - **reads** from Holon's read API directly (no process spawn);
  - **writes** by invoking `holon` with the run's token;
  - never runs a Chair command (allowlist).
- **Native Holon tools** for agents: `holon_whoami`, `holon_memory_search`, `holon_msg`,
  `holon_submit`, … These are registered in operant's tool registry and replace the per-run
  `holon mcp` Python server. Non-operant harnesses get them through the broker.

### 3.3 Enforcement and metering

- **Plan enforcement per tool call.** Holon writes `policy.json` into the run directory: posture,
  roots, writes, commands, deny, budget. Operant compiles it into an ephemeral seat policy and
  calls `org::seat_policy::decide` before every tool call. A refused call:
  - is never executed;
  - is told to the model;
  - is recorded in `result.refusals[]`, which Holon turns into a blocker (rework or escalation),
    not a breach.

  Holon's after-the-fact checks (fingerprints, held skill writes, machine checks) stay as the
  second line.
- **Budgets.** Caps are declared in Holon (`[budget]` tokens and cost per department and
  employee) and arrive in `policy.json`. Operant meters with the existing usage accumulator,
  injects the remaining budget into the loop, and enforces at the turn boundary (soft or hard).
- **Usage** (`tokens_in`, `tokens_out`, `cost`, `tool_calls`, `duration_s`) goes into
  `result.usage` on every run.

### 3.4 Learning capture: evidence, not edits

- In a Holon run, these produce `result.json` annotations instead of files:
  - the background review's memory and skill nudges;
  - `skill_manage`;
  - curator actions;
  - `memory_store`.

  The annotations are `learnings`, `skill_changes` and `proposals`; Holon verifies, versions and
  promotes them. Outside Holon runs nothing changes.
- **Trajectories.** Every Holon run writes `$HOLON_RUN_DIR/trajectory.json` (the existing
  trajectory recorder, redirected). Holon keeps the verified one as the job's golden trajectory
  and feeds a compacted form to the next executor and the judge.
- The **advisor** (transcript watchdog) keeps running. Its findings in a Holon run are added to
  `result.learnings` with kind `lesson` (low confidence).

### 3.5 Context

- Holon **selects** what a stage sees (the brief: memory, lessons, playbook, gold standard,
  history). That is learned policy.
- Operant **fits** it: the token budget against the model's window, a cache-stable prefix (identity
  and playbook frozen, brief volatile), and tiered eviction under pressure.
- `org::context_injection` keeps its ranked, quota-bounded injection for **channel-sourced**
  context: Telegram feeds and DMs, which are platform data operant owns. Its org-internal sources
  (worklog `self` and `dept`) are replaced by Holon's messages and memory.

### 3.6 The clock and events

- **Clock.** Operant's scheduler fires `holon tick` (as a `no_agent` job, every 2 minutes). This
  replaces the systemd timer in the package. Holon owns what is due; operant owns *when the tick
  happens*.
- **Event intake.** The `operant-runtime` routines engine (event matcher with cooldowns, built and
  unused today) compiles Holon's event triggers (`api_triggers`). It matches channel, webhook and
  file events, and calls `holon routine fire ID --payload`. What an event *means* is a routine in
  Holon. *Seeing* the event, fast and resident, is operant.

### 3.7 The owner's channel

- **Inbound.** The gateway checks the sender against Holon's `[chair] channels` and classifies the
  message with the decision model (approve, reject, answer, note, new work). It confirms before
  any approval, then runs `holon` as the Chair. Every other message is a normal chat with the
  bound employee.
- **Outbound.** Holon's notifications (approvals waiting, failures, routine deliveries) go to the
  same thread through operant's channels.
- **Webhooks.** The gateway hosts Holon's Notion webhook route and invokes `holon notion apply`
  per event, so no Python daemon stays up.

### 3.8 Views

TUI panels and `operant org …` read the read API:
- Org: departments, employees, records;
- Board: ready and running stages;
- Approvals;
- Playbooks: versions and scores;
- Objectives: key results.

The `holon` CLI remains the authoritative command line. Operant's views are presentation, plus
the owner's approve and answer shortcuts, which call `holon`.

### 3.9 Health and self-evaluation

- `operant doctor` and a one-turn `operant run` are smoke checks Holon defines for every department
  whose employees run on operant. They are probed by the runner as the agent user.
- Every run records the operant build (`operant --version`, memory-wire revision) in its usage
  block. Holon groups judgments by build, so a build that lowers a playbook's scores opens incident
  work. This is how the harness itself improves under the organization's loop.

## 4. What operant must not keep

Any store, schedule or decision that duplicates Holon's organization:
- employees, departments, hierarchy;
- grants and an escalation queue for org work;
- notices, worklog, decisions;
- synthesis, socialization;
- a kanban of org work;
- SOP and skill-improver loops;
- a continual-harness lesson store.

`ORG-LAYER-DISPOSITION.md` lists each module and its replacement.

## 5. Invariants (each a test in the phase that introduces it)

1. In organization-runtime mode, operant opens Holon's databases only with `mode=ro`, and only
   through `api_*` views. A Rust test fails on any other table name.
2. The vendored JSON Schemas (`holon contract schemas --write`) match the shapes operant writes. A
   Rust test validates a produced `result.json` against them.
3. A refused tool call never runs. Every refusal appears in `result.refusals[]`.
4. In a Holon run, `~/.operant/skills` and operant's memory store are never written.
5. With `[holon] enabled = false`, behavior is unchanged (the existing test suite passes untouched).
6. The broker refuses every Chair-class command, whatever the caller sends.
