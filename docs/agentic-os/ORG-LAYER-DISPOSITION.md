# Org layer disposition: keep, rewire, retire

Status: **proposed (2026-10-14)**. The org layer is `crates/operant-core/src/org/`: 26 files,
21,641 lines, a native port of the legacy Hermes organism (`docs/ORGANISM-ARCHITECTURE.md`). This
file decides each module's fate against Holon, which supersedes that organism. The rule is in
`BOUNDARY.md`.

## The verdicts

| Verdict | Meaning |
|---|---|
| **KEEP** | operant's own runtime mechanism; unchanged in both modes |
| **REWIRE** | the mechanism stays; in organization-runtime mode its data comes from Holon (read API, `policy.json`) and its output goes to Holon (the run contract), never to an operant org table |
| **RETIRE** | it duplicates a Holon mechanism. It is disabled in organization-runtime mode at once, and deleted one release after phase G (archive first, never delete data) |

Holon references below are in Holon's repository: `docs/INTENT.md` (terms), `docs/GENOME.md`,
`holon/execution/*.py`.

## 1. `operant-core/src/org/`

| Module | Lines | What it does now | Verdict | Replacement, or what changes | Proof (test) |
|---|---|---|---|---|---|
| `seat_policy.rs` | 402 | pure policy engine: mode + allow/deny globs → verdict | **KEEP** | becomes the enforcement core for `policy.json` | a run with `policy.json` denying writes outside `roots` refuses `file_write` there |
| `seat_policy_db.rs` | 505 | policy rows per seat | **REWIRE** | personal mode: unchanged (operator seats). Org mode: not consulted; the run's `policy.json` is the policy | org-mode run ignores a conflicting row |
| `seat_budgets.rs` | 415 | per-seat budget overrides, window math, turn-boundary enforcement | **REWIRE** | enforcement and window math stay; caps come from `policy.json.budget` (declared in Holon `[budget]`); the `seat_budgets` table is personal-mode only | hard cap in `policy.json` stops the turn with a budget refusal |
| `worklog.rs` (`UsageAccumulator`, `OutcomeSignals`) | 1,075 | worklog types plus the usage accumulator | **REWIRE** (the accumulator) / **RETIRE** (worklog types) | the accumulator feeds `result.usage`; Holon's `runs` and events are the objective log | `result.usage` populated on every org run |
| `worklog_db.rs` | 915 | the worklog store | **RETIRE** | Holon `runs`, `outputs`, hash-chained `events` | — |
| `write_barrier.rs` | 1,683 | a run is incomplete until its log rows exist | **RETIRE** | Holon's contract: a run without a valid `result.json` is a breach, salvaged at lease expiry; post-execute is kernel-owned | — |
| `decisions_db.rs` | 1,601 | subjective log + decision objects with dissent | **RETIRE** | `result.decisions` / `result.learnings` annotations, consolidated into Holon memory (kind `decision`); plan rationale lives in the plan output | — |
| `context_injection.rs` | 1,184 | ranked, quota-bounded DM/feed/global/dept/self context | **REWIRE** | keeps channel-sourced aspects (DM, feed) for chats; `self`/`dept`/`global` sources are replaced by Holon messages and memory (and the brief in a Holon run) | org-mode prompt carries no worklog-sourced lines |
| `cast.rs` | 757 | the nine standing seats, their charters and default crons, seeded at first run | **RETIRE** | Holon departments and employees (§5 maps each seat); seeding is `holon dept create` / `hr hire` | org mode seeds nothing |
| `employee.rs`, `employee_db.rs` | 907 | employee registry, `employee_id` derivation, backfill from crons | **RETIRE** | Holon employees via `api_employees`; the session binding stores a Holon employee id | — |
| `department_db.rs` | 1,148 | departments: mandate, protocols, rules, seats | **RETIRE** | Holon departments (mission, profile, `[scope]` roots); protocols and rules become department skills and playbooks | — |
| `hierarchy.rs`, `hierarchy_edges.rs` | 1,591 | reporting tree, positions, org-lead topology | **RETIRE** | Holon `employees.manager`, department heads, the CEO; authority by grade (`charter authority`) | — |
| `authority.rs` | 1,886 | scope lattice, grant table, cross-department capability grants | **RETIRE** | Holon: an approved plan *is* the grant for its run (compiled to `policy.json`); cross-department work is `msg request`, which becomes work there; approval by grade and earned authority | — |
| `seat_authority.rs` | 1,244 | escalation core: `Escalate` verdict → durable ask; approval → standing grant | **REWIRE** | personal mode: unchanged (interactive approve prompt). Org mode: `Escalate` becomes a refusal in `result.refusals[]`; Holon turns blockers into rework, replan or a Chair question | escalation in an org run lands in `refusals`, not `pending_requests` |
| `pending_requests.rs` | 792 | durable escalation queue for unattended runs | **RETIRE** (org) | Holon's board and approvals queue; personal crons keep the interactive lapse | — |
| `identity_gate.rs` | 395 | fail-closed: a cron job without a complete employee identity does not tick | **RETIRE** | Holon run tokens: no token, no organization; Holon refuses an employee without slot or roles at claim | — |
| `notice.rs`, `notice_db.rs`, `resolver.rs` | 2,597 | notice board with typed recipients resolved at read time | **RETIRE** | Holon messages (`msg send` to employee, department, org or Chair; `msg request` becomes work); operant renders `api_messages` | — |
| `dm_thread.rs` | 1,502 | synchronous employee-to-employee DM with a 3-turn shared budget | **RETIRE** (store) | async: `holon msg`; a synchronous consult, if the owner wants one, is a runtime feature rebuilt later on Holon identities, its turns recorded as messages | — |
| `socialization.rs` | 242 | daily seat pairings | **RETIRE** | if wanted, a Holon routine in the executive department (collaboration skill), judged like any routine | — |
| `synthesis.rs` | 327 | mechanical chief-of-staff digest into the org memory bank | **RETIRE** | Holon memory promotion, `holon metrics`, objective dashboards, the Notion mirror; a digest is a routine with `--deliver` | — |
| `schema.rs`, `mod.rs` | 473 | org schema reconciliation, module root | **RETIRE** with the stores; `mod.rs` keeps the kept modules | — | — |

**Kept in operant:**
- `seat_policy`;
- the enforcement half of `seat_budgets`;
- the usage accumulator;
- channel-sourced `context_injection`;
- `seat_authority` for personal mode.

That is about 3.8k of 21.6k lines (`seat_policy` and its db, `seat_budgets`, `context_injection`, `seat_authority`, and the accumulator out of `worklog.rs`), plus their tests. Everything else is a second organization.

## 2. Satellites of the org layer

| Module | Where | Verdict | Replacement, or what changes |
|---|---|---|---|
| Kanban (board, dispatcher, triage, notify; `kanban` tool) | `operant-core/src/kanban/` (1,744), `tools/kanban_tool.rs` | **RETIRE** in org mode | Holon's work graph. The agent tool is replaced by native Holon tools; `operant kanban` shows `api_board`. Personal mode keeps the personal board |
| Cron store and scheduler; `cron` tool | `operant-core/src/cronjobs/`, `tools/cron_tool.rs` | **REWIRE** | the scheduler stays as the **clock** (fires `holon tick`) and runs personal crons. In org mode `operant cron add` and the `cron` tool *propose a routine* (`holon routine add` via the broker as the agent; approval follows Holon's grade). The cast crons are disabled in phase G |
| Background review: memory and skill nudges | `agent/background_review.rs` | **REWIRE** | in a Holon run the nudges write `result.learnings` and `result.skill_changes` |
| `skill_manage`, skill usage, curator | `tools/skills_tool.rs`, `skill_usage.rs`, `curator/` | **REWIRE** | personal skills unchanged. In a Holon run, `skill_manage` writes proposals. Org skills are curated by Holon (stewards, playbook refinement); curator usage data can feed Holon as evidence later |
| Memory: memory-wire / builtin, `memory_*` tools | `memory_wire.rs`, `memory_provider.rs`, `operant-tools/src/memory_*` | **REWIRE** | personal memory unchanged. In a Holon run: recall merges `holon_memory_search` (institutional) with the run's working memory; `memory_store` writes `result.learnings`. Later option: memory-wire indexes `api_memory` read-only for faster recall |
| Kernel sidecar: `kernel_exec` | `kernel-sidecar/`, `tools/kernel/` | **KEEP** (opt-in tool) | executing Python for the user is inherently Python; idle-reaped; off by default |
| Kernel sidecar: continual-harness store (`kernel_state`, `kernel_refine`, `harness_auto_learn`) | same | **RETIRE** (org) | prompt and subagent lessons are skills and memory in Holon, verified before they apply. In a Holon run `kernel_refine` writes proposals |
| `pk-sidecar/` | repo root | **RETIRE** | a Phase 0 stub superseded by `kernel-sidecar` (plan 015) |
| `operant-runtime` `sop` (6,055) | used only by the MQTT orchestrator | **RETIRE** | Holon playbooks (protocol + rubric, versioned) and the script lane |
| `operant-runtime` `skills/improver.rs`, `skillforge` (1,194) | unused | **RETIRE** | Holon playbook refinement after verification; skill scouting is a research routine |
| `operant-runtime` `routines` (660) | unused | **KEEP and wire** | the event matcher for Holon's event triggers (`BOUNDARY.md` §3.6) |
| `operant-runtime` `security` (8,547): sandboxes, pairing | used by channels | **KEEP** | the runner's sandbox layer; pairing stays with channels |
| `operant-harness` `pool` seam (`_pool.yaml` from the Hermes organism) | `operant-harness/src/pool.rs` | **RETIRE** | departments come from Holon; the harness kernel itself (providers, patches, reversible effects) is **KEEP** |
| `doctor.rs` genome checks (ungoverned cron seats, unbound identities) | `operant-core/src/doctor.rs` | **RETIRE** (those checks) | Holon doctor. `operant doctor` itself is **KEEP**, as a Holon smoke check |
| Gateway session binding, rolling summaries, platform identities, pairing | `gateway_session.rs`, `gateway_runner.rs` | **KEEP** | `employee_id` holds a Holon employee id; platform-identity binding is how the Chair is recognized (Holon `[chair] channels` lists the identities) |
| `escalate` tool | `operant-tools/src/escalate.rs` | **REWIRE** | in a Holon run it returns a refusal or blocker into the result |
| Other `operant-runtime` modules (`verifiable_intent`, `trust`, `tunnel`, `nodes`, `rag`, `platform`, `service`, `hooks`) | | untouched | not part of the agentic-OS boundary |

## 3. `operant org` subcommands

| Subcommand | Becomes (org mode) |
|---|---|
| `employee`, `list`, `check` | a view over `api_employees` (`holon emp list` is authoritative) |
| `department` | a view over `api_departments` |
| `notice` | a view over `api_messages`; sending goes through `holon msg send` |
| `worklog` | a view over Holon runs (`holon run show`, `holon events`) |
| `decision` | `holon memory search` (decisions are memory items of kind `decision`; a `--kind` filter is a small Holon addition) |
| `grant` | removed (approval of plans is `holon plan approve`) |
| `dm` | `holon msg` |
| `budget` | a view of declared caps and metered use |
| `audit` | `holon hr review`, `holon audit samples` |
| `cast` | removed after phase G |
| `sync`, `import` | replaced by `holon org import-operant` (phase G) |
| `synthesize` | removed (a Holon digest routine) |

## 4. Data: archive, never delete

Operant already keeps org tables in **two** databases that disagree. This is the split-brain the
`docs/audit/2026-06-19-BUGS.md` R5-1 rule was meant to prevent.

| Table | `~/.operant/database.db` | `~/.operant/operant_kanban.db` |
|---|---|---|
| `employees` | 9 | 10 |
| `seat_policies` | 0 | 1 |
| `hierarchy_edges` | 8 | 8 |
| `authority_grants` | 1 | 1 |

Also: `worklog` 385, `notices` 23, kanban `tasks` 17; `operant_decisions.db`: `subjective_log`
385, `org_decisions` 1.

Phase G:
1. Copy each org store to `~/.operant/archive/org-<date>/` with a checksum manifest.
2. Run `holon org import-operant --dry-run`, then `--apply` (§5).
3. Mark the archived tables read-only (the code that writes them is gone).

Nothing is deleted. The 385 worklog rows remain history; Holon starts its own record.

## 5. The cast in Holon

| Seat | Holon home |
|---|---|
| `premiere` | the CEO (head of `executive`). The owner's channel binds here |
| `chief-of-staff` | an `executive` employee (planner/steward slots); its daily brief is an executive routine with `--deliver` |
| `governor` | the executive's audit duty: `holon audit samples`, doctor and lint triage, retros. A routine, not a separate department |
| `compass` | strategy: an executive employee or a strategy department reading the Compass goals Holon links work to (`project_ref`) |
| `identity-warden` | a routine in the department that owns identity, whose hourly audit becomes a script-lane routine if deterministic (the classifier decides) |
| `hrmaster` | **a kernel mechanism, not an agent**: `hr review`, probation, earned authority |
| `crew-chief` | **a kernel mechanism**: authority and grades, isolation, scope |
| `dispatcher` | **a kernel mechanism**: claims, leases, `tick`, the run supervisor in operant |
| `dp-the-program` | the run supervisor plus the executor slots of each department |

Their 11 cron jobs (6 failing on 120 s model timeouts as of 2026-10-14) are re-proposed through
`holon routine import` / `routine add`. Each lands in the cheapest lane the classifier allows, with
a playbook, and only then are the operant crons disabled.
