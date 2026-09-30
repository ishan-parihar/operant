# Organism → Operant: First-Class Agentic OS Outline

**Status:** outline for discussion. No code written. Every claim in this document
was measured this session against the two trees, not carried forward from a
prior doc.

**Sources read at source (2026-09-30):**

- `~/.hermes/organism/` — 26,395 files. Strata `cortex / swarm / foundations /
  ventures` (`_org.yaml:25`), 24 live pools, 272 AD records, 149 RG records,
  ~117 cron-employees, 14 `smoke.yaml` gates.
- `operant/` @ `4af601b7` — 17 crates, ~475k LOC of Rust.

---

## 0. The one-paragraph version

The organism is **a filesystem-governed operating system where the filesystem
layout *is* the API**. Its load-bearing idea is not "agents do tasks" — it is
that every agent is a **department employee with a persisted identity, a
declared workflow, an append-only worklog, and a fail-closed pre-tick gate**,
and the whole fleet is held together by four mechanisms: a decision-record
lifecycle (AD/RG), a **derived** architecture registry that must never go
stale, pool isolation enforced by a linter, and a smoke gate that blocks the
tick when the baseline is broken.

Operant already has the *hard* half of an agentic OS and none of the
*governed* half. It has a composable provider kernel, a concurrent agent loop,
context management, memory providers, channels, cron, kanban, a security stack,
and ~475k LOC of Rust that compiles clean. It has **zero** notice board, zero
employee registry, zero worklog, zero decision-record lifecycle, and zero
fail-closed pre-execution gate. It also has one piece that is *nominally*
organism-shaped and is **structurally incapable of loading a real organism
pool** — the harness `_pool.yaml` compiler, proven below.

The upgrade is therefore not "port the organism." It is: **promote operant's
existing runtime primitives into first-class OS citizens, and graft the
organism's four governance mechanisms on as enforcement layers that run
*before* work, not after.**

---

## 1. What the organism actually is

### 1.1 The three-layer stack

| Layer | Pool | Role | Implementation |
|---|---|---|---|
| **L1 runtime** | `swarm/task-grid` (AXE) | notice board, agent registry, task lifecycle, delegation chains, tick | `scripts/axe_lib.py` 2,590 LOC + `axe-tick.py` 2,404 + 30 more scripts (16.8k total) |
| **L2 operator** | `swarm/workforce-ops` (HR) | staff audit, review, lifecycle, persona evolution, efficacy | `lib/hr_lib.py` 1,830 LOC, 14 subcommands |
| **L3 design** | `swarm/agent-fabric` (FABRIC) | AgentSpec schema, per-dept worklog, PDCA loops | `lib/agent_fabric.py` 299 LOC |
| **L0 governance** | `cortex/meta-governance` | AD/RG lifecycle, pool linter, registry emitter, `arch` CLI | `lib/cli.py` 4,787 + `lib/engine.py` 6,598 + `lib/core.py` 781 (14.8k total) |

Strata below swarm: `foundations/` (9 pools: identity-core, flight-ledger,
platform-infra, research-vault, strategy-incubator, content-campaigns,
site-factory, presentation-engine, relationship-crm) and `ventures/` (9
compass-graduates).

### 1.2 The four governance mechanisms (the actual IP)

Everything else in the organism is content. These four are the system.

**M1 — Decision records as the unit of governance.** Every pool owns
`core/decisions/<PREFIX>-AD-NNN.md` and `core/regressions/<PREFIX>-RG-NNN.md`
with required frontmatter (AD: `Status, Date, Deciders, Description`; RG:
`Severity, Date, Description`). 272 ADs + 149 RGs. A decision is *only* valid if
it is filed, and `arch new` is the sole lifecycle entry point (AD-051).

**M2 — The registry is derived, never hand-edited.** `arch emit` regenerates
`architecture-registry.yaml` and the AUTO-GENERATED zones of every `AGENTS.md`
from the `_org.yaml` files. The linter **WARNs when the registry is stale**
(AD-061). This is the load-bearing idea: *the map is a build artifact, so it
cannot lie.* Corollary: dead IDs get a **tombstone row** pointing at their
successor (`graveyard/TOMBSTONES.md`) so validators resolve dangling citations
instead of erroring — "never delete history, point at where it went."

**M3 — Config-driven pool isolation, linted.** Each pool declares its own
genome in `_org.yaml` (`directories:` with per-dir `suffix` / `prefix` /
`frontmatter_required` / `required`, `required_files`, `forbidden`). The linter
(`scripts/pool_lint.py`) checks actual-vs-declared with **no hardcoded
assumptions**. `AD-074` adds a structural allowlist (`allowed_top_level`) and
orphan policies. Nothing hardcodes a pool name; pools resolve through a
registry.

**M4 — Fail-closed pre-tick smoke gate.** Every pool with a CLI surface
declares `smoke.yaml`: a list of `{name, cmd, timeout, critical, reason}`. The
gate runs **before** the tick spawns any agent; `critical: true` with a
non-zero exit means **do not spawn** (AXE-AD-010). Each check carries a
`reason:` string explaining *what breaks if this is allowed to fail* — the
gate is self-documenting.

### 1.3 The employee contract (what makes an agent an *employee*)

AXE-AD-011/012/016 + AF-AD-001 compose into a mandatory per-cron-job contract.
A cron job is not a cron job until it declares:

| Field | Source | Semantics |
|---|---|---|
| `employee` | AXE-AD-012 | `emp-<hash>`, joined to `data/employees.json` |
| `skills[]` | AXE-AD-012 | **leaf** skill names; meta-skill dir names are a lint error |
| `agent_type` | AXE-AD-012 | `service` (24h TTL) \| `session` (1h) \| `fixer` (30m) |
| `continuity` | AXE-AD-012 | persist session memory across runs |
| `registers_on_spawn` | AXE-AD-012 | auto-register in the agent registry on spawn |
| `persona` | AXE-AD-016 | `greeting`, `working_style`, `interests[]`, `voice_notes` — **fail-closed** for long-lived employees |

Plus, from FABRIC (AF-AD-001), the `AgentSpec` proper:

```python
AgentSpec(employee_id, name, role, department, agent_type, persona,
          workflow: Workflow, loop: LoopContract, skills[], reports_to, peers)
Workflow(kind: gather|process|publish|monitor|remediate|synthesize|coordinate,
         summary, steps[], tools_required[], cadence, acceptance)
LoopContract(self_reflect, peer_observe, dept_audit, org_review,
             improvement_proposal)
```

And the **worklog** (AF-AD-002) — per-department append-only JSONL, the
meta-cognitive layer, written by *the framework at session end*, not by the
agent:

```
WorklogEntry(ts, employee, department, job_id, iteration, workflow_kind,
             what_done, outcome: success|partial|failure|noop, artifacts[],
             tokens_in, tokens_out, tool_calls, duration_s, next_intent,
             blockers[], improvement_proposal)
```

`improvement_proposal` is the kaizen field: it is how a run's friction becomes
the next run's change.

### 1.4 Two structural patterns worth stealing wholesale

**The `--reason` mandate (AD-032).** *Every* mutating CLI operation across
*every* pool requires `--reason`, enforced at argparse. The stated motivation is
causal traceability, and the AD records the failure that motivated it: 1,311
decision-log entries over 5 days, **zero read by any downstream phase** — a
write-only ratchet. The fix was architectural (make the CLI reject an
unmotivated write), not cultural.

**The maintenance contract (AD-078).** Every staffed pool declares
`maintenance.md` with `cadence` + H3 checks each carrying `run:` and `accept:`.
The linter **fails closed** on a staffed pool missing one with ≥1 run-bearing
check. Employees execute it as a fallback when no dept-specific task is ready.
This is how a fleet maintains itself without a human writing a cron per pool.

---

## 2. What operant already has (do not rebuild these)

Measured, not assumed:

| Organism need | Operant reality | Verdict |
|---|---|---|
| Composable capability mounting | `operant-harness` — string-keyed claims, PENDING late binding, reversible effects (LIFO), transactional swap, composition-as-data. 6 native seams registered. | **Strong.** The kernel is the right primitive. |
| Per-tenant process isolation | `operant-runtime/src/security/` — landlock, bubblewrap, firejail, docker, seatbelt, workspace_boundary | **Strong.** |
| Approval / side-effect gating | `operant-core/src/approval.rs` (1,485 LOC) + `operant-runtime/src/approval/` | **Strong.** |
| Task lifecycle | `operant-core/src/kanban/` — `TaskStatus`, `claim_task`, `complete_task`, `block_task`, comments, dispatcher | **Strong.** |
| Cron + runs table | `operant-runtime/src/cron/` — `CronJob` (30 fields), `Schedule::{Cron,Every}`, `CronRun`, scheduler, declarative+imperative `source` | **Strong.** |
| Agent identity injection | `agent/personality.rs` (8 workspace files) + `identity.rs` (AIEOS v1.1 JSON) | **Adequate**, but not per-employee. |
| Failure classification | `agent/error_classifier.rs` (1,534 LOC), circuit breaker, stream retry budget | **Strong.** |
| Observability | `operant-runtime/src/observability/` — otel, prometheus, runtime_trace, dora, cost | **Strong.** |
| Context management | tiered eviction + decay curve, LLM compressor, auto-compress on overflow | **Strong.** |
| 7 platform adapters | telegram, discord, slack, whatsapp, email_smtp, sms_twilio, webhooks | **Adequate.** |
| Gate discipline | `scripts/clippy-warning-gate.sh` (fail-on-new-warning vs allowlist), `scripts/check.sh` | **Partial.** Build-time only; no *runtime* pre-tick gate. |

Operant is roughly 475k LOC against the organism's ~31.6k LOC of governance +
runtime Python. The size asymmetry is the point: **operant has the engine; the
organism has the operating discipline. They are complementary, not competing.**

---

## 3. The gap matrix

### 3.1 Organism has, operant lacks

| # | Capability | Organism | Operant | Severity |
|---|---|---|---|---|
| G1 | **Decision-record lifecycle (AD/RG)** | 272 ADs + 149 RGs, `arch new/update/supersede/validate`, prefix namespaces, tombstones | **none** | High |
| G2 | **Derived architecture registry** that cannot go stale | `arch emit` regenerates registry + AGENTS.md auto-zones; linter WARNs on drift | `architecture.toml` exists but hand-authored; no staleness check | High |
| G3 | **Employee registry** (`emp-<id>`, dept, role, skills, crons, status) | `data/employees.json`, `axe org` 14 surfaces | **none** | High |
| G4 | **Notice board** — routed, addressed, ack'd agent↔agent messaging | `notice-board.jsonl`, typed recipients `agent:/dept:/team:`, correlation ids, ack, retention GC + weekly summaries | **none** | High |
| G5 | **Worklog** (per-dept append-only self-report) | `data/worklog/<dept>.jsonl`, framework-written at session end | **none** | High |
| G6 | **Fail-closed pre-tick smoke gate** | `smoke.yaml` per pool, `axe smoke`, exit 2 blocks the tick | **none at runtime** | High |
| G7 | **Maintenance contracts** | `maintenance.md` per staffed pool, linter-enforced | **none** | Medium |
| G8 | **`--reason` mandate** | enforced at argparse across all pools | **none** (`cmd_kanban` has an *optional* `reason`) | Medium |
| G9 | **Persona contract** (fail-closed for long-lived) | `check_persona_contract.py` | workspace-file personality, no per-employee persona | Medium |
| G10 | **Fixer routing** | error-pattern registry → `fix-pattern:<id>` tag → specialized fixer agent | `error_classifier.rs` classifies; no registry-driven fixer spawn | Medium |
| G11 | **Pool isolation enforcement** | pool_lint actual-vs-declared | `pool.rs` compiles but **cannot parse real manifests** (see 3.3) | High |
| G12 | **PDCA loops** (SELF/PEER/DEPT/ORG) | AF-AD-003, SELF implemented | `learning_graph.rs` + `background_review.rs` are adjacent but not the same loop | Low |
| G13 | **Tombstones** for retired records | `TOMBSTONES.md`, validators resolve dangling refs | `docs/BUGS.md` status lines (6 of 35 were stale when last audited) | Low |

### 3.2 Operant has, the organism lacks

| # | Capability | Why it matters for the port |
|---|---|---|
| O1 | **Provider kernel** with reversible effects + ABA-safe swap | The organism cannot mount/unmount a capability at runtime. This is operant's genuine lead. |
| O2 | **Real sandboxing** (landlock/bubblewrap/firejail/docker/seatbelt) | The organism runs Python cron agents with user permissions. No containment story. |
| O3 | **Static types + compile-time seam enforcement** | The organism's entire pool/service graph is runtime YAML that only a linter checks. Operant can make illegal states unrepresentable. |
| O4 | **Prompt-cache stability** (frozen prefix + volatile suffix) | Not an organism concern at all. |
| O5 | **Token accounting per run** | The organism logs tokens in cron output files, not as a first-class column it can query. |
| O6 | **7 real platform adapters** | The organism reaches the outside world through CLIs. |

### 3.3 The one thing that is worse than absent: `pool.rs`

`crates/operant-harness/src/pool.rs` (222 LOC) compiles a `PoolManifest`
described in `docs/harness-kernel.md:99` as *"a hermes `_pool.yaml`"*.

> **CORRECTION (iter-514).** The original version of this section reported
> "6/6 real organism pools fail at parse". That was wrong on both the count and
> the failure stage, and it understated the blast radius. Re-measured
> 2026-09-30 against **all 32** real `_pool.yaml` files, replicating the shipped
> struct *and* the shipped `compile()` verbatim:
>
> ```
> compile OK      : 0
> parse fail      : 5    services_offered/consumed[N]: missing field `name`
> empty-name fail : 27   parses, then compile() rejects on empty `name`
> TOTAL UNUSABLE  : 32 / 32
> ```
>
> Two corrections follow. First, the manifest is `_pool.yaml`, not `_org.yaml` —
> both exist and coexist per pool, and `_pool.yaml` is the richer of the two (it
> adds `genome`, `interface`, `sla`, `target_pool`, `sub_systems`). Second, the
> failure is **not** uniform: only 5 fail at parse. The other 27 parse
> *successfully* because `#[serde(default)] name` swallows the missing top-level
> key, and then die inside `compile()`. So the true root cause is narrower and
> worse-reading than "wrong schema": **`#[serde(default)]` converts a hard parse
> failure into a deferred, mislabelled compile error** across 84% of real pools.
> `docs/harness-kernel.md:99` also documents the wrong path
> (`~/.hermes/systems/<pool>/_pool.yaml`); real pools live under
> `~/.hermes/organism/<stratum>/<pool>/_pool.yaml`.

Three independent schema mismatches, each fatal on its own:

1. **Name is nested.** Real schema has no top-level `name`; it is
   `pool.name`. The shipped `#[serde(default)] name: String` silently yields
   `""`, then `compile()` returns `"pool manifest has empty \`name\`"` — an
   error that names a symptom the operator cannot act on.
2. **Service key is `id`, not `name`.** Real entries are
   `{id, entry_point, accepts, returns}`; `PoolService.name` is a *required*
   field, so serde hard-fails.
3. **Services carry no verb prefix.** Real ids are `identity`, `values`,
   `logbook`, `employee-telemetry`, `ad-rg-lifecycle`. The `READ_ONLY_VERBS`
   check requires `query.`/`fetch.`/`get.`/… prefixes, so even a fixed parser
   would reject every real service.

**Why this matters more than a normal bug.** The `_pool.yaml` import was the
stated Phase-6 deliverable of plan 016 and is reported as
"✓ pool compiler + read-only verb check" in `docs/harness-kernel.md:180`. The
acceptance was tested against a **hand-written fixture that matches the wrong
schema** (`pool.rs:153-170` — a `relationship-intel` manifest with dotted verb
names, a shape no real pool has). So the feature is green in CI and inert in
production. This is precisely the failure mode the organism's M2 exists to
prevent: *a derived artifact that was never checked against its real source.*

Three follow-on design errors once the schema is fixed:

- `pooled_sub_systems` is read as a top-level list; in AD-060 pooling is
  expressed as **exact-name symlinks inside `sub-systems/`**. The compiler
  models a field that does not exist and misses the mechanism that does.
- `PoolSubSystem { name, path }` has no `read_only` provenance; `read_only:
  true` is hardcoded into the emitted config, so it asserts a property it never
  verified.
- The compiler drops `entry_point` / `accepts` / `returns` entirely — the
  three fields that make a service contract *executable* (AD-070: *"Entry
  points are executable verbatim"*).

---

## 4. The plan — five waves

Design rules for the whole plan:

- **Enforcement runs before work, not after.** Every gate is a pre-execution
  gate. A gate that reports after damage is an autopsy (AXE-AD-010's own
  reasoning).
- **No second store.** Every organism substrate must land in an existing
  operant store (sqlite, or the harness composition file) unless there is a
  stated reason it cannot. Three divergent locations for one concept is a bug
  operant has already paid for once (BUGS.md R5-1, the `memory/` split-brain).
- **Dark-mergeable.** Every wave is default-off and byte-stable when off, per
  the harness precedent (`docs/harness-kernel.md:7`).
- **Fail closed, and prove it.** Each gate ships with a mutation test that
  turns the gate red, mirroring `scripts/clippy-warning-gate.sh`'s discipline.

### Wave 0 — Make the pool compiler honest (prerequisite, small)

Nothing else can consume a pool until this is true.

1. **Real `OrgManifest` schema** in `operant-harness/src/pool.rs`:
   `pool{name,type,tier,category,ad_scope,parent}`, `services_offered[{id,
   entry_point, accepts, returns, sub_system?}]`, `services_consumed[{id,
   provider, purpose, access}]`, `directories{…}`, `required_files[]`,
   `forbidden[]`, `department{charter,class}`.
2. **Read-only is a property of the *access* mode, not the verb name.** Real
   pools have no verb prefixes. Replace the `READ_ONLY_VERBS` gate with
   `access: read` on the consumer edge plus a `write` capability list that is
   empty by default and structurally cannot be widened without an approval
   token. (Preserve `READ_ONLY_VERBS` as an *additional* check for pools that
   do use dotted names.)
3. **Carry the contract fields.** Keep `entry_point` / `accepts` / `returns` in
   the compiled row so a service claim is executable, per AD-070.
4. **Model pooling as symlinks.** Walk `sub-systems/` and emit one
   `pool.bundle` row per symlink target, not per declared field.
5. **Real-fixture test.** Replace the synthetic `pool.rs:153` fixture with the
   **actual bytes** of the six organism pools above, checked into
   `crates/operant-harness/tests/fixtures/`. A fixture that does not come from
   the real source is the root cause of the bug; shipping one that does is the
   cure.
6. **Discovery, not a path.** `arch route <path>` in the organism resolves a
   directory to its owning pool. Operant should ship `operant org route <path>`
   and `operant org list` walking a declared root (default
   `~/.hermes/organism`) with the same answer.

**Gate:** `cargo test -p operant-harness` green against real fixtures; a new
test asserts that a manifest whose registry is stale is rejected (M2 parity).

**Size:** ~2 days.

### Wave 1 — The org layer: employees, notice board, worklog

This is G3+G4+G5. It is the wave that turns "a cron job" into "an employee,"
and it is the largest single unlock.

1. **Employee registry.** New crate `operant-org` (or `operant-runtime::org`).
   `Employee { employee_id, name, role, department, skills[], agent_type,
   persona, home_crons[], status, created }`. Backfill from existing
   `CronJob`s — operant's `CronJob` (30 fields) already carries `name`,
   `source` (declarative vs imperative), `allowed_tools`, `uses_memory`,
   `model`, `delivery`. That is most of an employee record already.
2. **The cron-employee identity contract.** Extend `CronJob` with
   `employee`, `skills[]`, `agent_type: Session|Service|Fixer`, `continuity`,
   `registers_on_spawn`, `persona`. Map `agent_type` → registry TTL exactly as
   AXE-AD-012 does (24h / 1h / 30m). Emit a machine-readable schema so the
   linter can check it.
3. **Notice board.** `notice-board.jsonl` equivalent — but in **sqlite**, not a
   new JSONL file. Typed recipients (`agent:` / `dept:` / `team:`), sender
   provenance, `correlation_id`, `ack_required`, `ttl`, `tag`, `thread`.
   Retention: the organism's 14-day raw + 16-week weekly-summary GC
   (AXE-AD-006/RG-027) is a good default; operant's `retention_gc` equivalent
   should be batched, since the organism's own version needed a perf fix from
   65s to 0.4s for 1.8k entries.
4. **Worklog.** Per-department append-only, written by the **framework at
   session end**, never by the agent (AF-AD-007: it is a framework invariant,
   not an agent responsibility). `WorklogEntry` mirrors the organism's 15
   fields including `improvement_proposal`. Wire it to data operant already has:
   `agent/cost.rs` (tokens), `observability/` (duration, tool calls), the
   error classifier (outcome), `learning_graph.rs` (`next_intent`).
5. **Model-callable tools.** `notice_post`, `notice_inbox`, `notice_ack`,
   `worklog_append`, `org_status` — all behind the existing approval gate, all
   governed by the kernel so they can be unmounted.
6. **`--reason` mandate.** Add `reason: String` (required) to every mutating
   subcommand in the org surface, and record it on the notice/worklog/employee
   write. This is the single highest-leverage, lowest-cost item in the whole
   plan: it is ~20 lines of clap per command and it is the difference between
   an auditable system and a write-only ratchet.

**Gate:** `operant org check` (fail-closed) + a smoke-gate runner; every
existing `CronJob` backfills to a valid employee; a job missing any of the 5
identity fields **blocks the tick** (not a warning).

**Size:** ~2 weeks.

### Wave 2 — The gate: fail-closed pre-execution

This is G6+G7+G9, and it is where operant becomes *safe* to run unattended at
scale.

1. **`smoke.yaml` support in operant config** — declare checks
   `{name, cmd, timeout, critical, reason}` per component. The `reason` field
   is not decoration: AXE-AD-010 requires each check to state what breaks if
   it is allowed to fail, which makes the gate self-documenting and greppable.
2. **`operant doctor --gate`** → exit 2 on any `critical` failure, and have
   **every autonomous entry point refuse to spawn on exit 2**: `operant
   autonomous`, the cron scheduler's agent-job path, `operant run
   --autonomous`, the gateway. The organism's lesson is that a gate nobody
   blocks on is decoration (AXE-AD-010 rejected per-task preflight in the
   prompt precisely because it drifts).
3. **Maintenance contracts.** `[maintenance]` per component with `cadence` +
   `checks[{run, accept}]`; the linter fails closed on a staffed component
   missing one; employees run it as the fallback when no specific task is
   ready.
4. **Persona contract.** Fail-closed persona for `agent_type=service` OR
   `continuity=true` OR `registers_on_spawn=true` (AXE-AD-016's exact
   adoption table). Hook into the existing `agent/personality.rs` slot so the
   persona actually reaches the system prompt — the organism's failure mode
   here was 2/117 employees having one while the field was already supported.
5. **Migrate operant's existing build gates into the same model.** The clippy
   gate, the `--all-features` inventory, and the doctest gap
   (`docs/IMPLEMENTATION-PLAN.md` §1.2) are already fail-closed gates — they
   just are not *runtime* gates. Give them one schema.

**Gate (the meta-gate):** prove the gate blocks. A test that injects a
failing `critical` check and asserts (a) `doctor --gate` exits 2 and (b) the
autonomous entry point refuses to spawn. A gate without a
mutation-proven-red test is a wish.

**Size:** ~2 weeks.

### Wave 3 — Governance records: AD/RG + derived registry

This is G1+G2. It is deliberately **late**: governance records before the
runtime is governed are ceremony.

1. **`operant decision` / `operant guard` CLI.** AD and RG records with the
   same frontmatter contract, per-crate prefix namespaces
   (`CORE-AD-`, `CLI-AD-`, `RT-AD-`, matching the organism's per-pool prefixes),
   and the same lifecycle: `new` / `update` / `supersede` / `validate` /
   `show` / `stats`. `supersede` writes a pointer, never a delete.
2. **Tombstones.** `TOMBSTONES.md`-equivalent so dangling citations resolve.
   This directly fixes a live operant problem: `docs/IMPLEMENTATION-PLAN.md`
   §1.3 found **6 of 35 `BUGS.md` headlines claimed open work that was already
   done**. A tombstone + a linter that resolves citations would make that
   class of drift structurally detectable.
3. **Derived registry.** `operant architecture emit` regenerates
   `architecture-registry.yaml` and the AUTO-GENERATED zones of `AGENTS.md`
   from the `_org.yaml`-equivalents, and **the linter fails when the registry
   is stale**. This closes the M2 loop that the organism has and operant does
   not, and it retroactively fixes the `pool.rs` class of bug at the fleet
   level.
4. **Emit `maintenance.md` and the service tables from the same source**
   (AD-070's auto-zone), so the docs cannot drift from the code by
   construction.

**Gate:** every operant subsystem has ≥1 AD; every RG has a test that fails
when the guarded behavior regresses; `architecture validate` exits non-zero on
registry drift.

**Size:** ~2 weeks.

### Wave 4 — The self-evolution loop (make it *operationalize itself*)

The word in the request is *operationalize*, and this is where the loop closes.
G12, plus the organism's PDCA model, mapped onto operant's existing
`learning_graph.rs` + `background_review.rs` + `harness`.

1. **SELF loop.** Before an employee runs, inject its last-N worklog entries +
   its spec. `harness` mounts the `prompt.section` seam to do it — the kernel
   already has the slot (`docs/harness-kernel.md:35`); the handler is the
   deferred Phase-5 work.
2. **PEER loop.** Sibling worklog entries arrive via the notice board's
   `dept:` feed, injected at session start.
3. **DEPT loop.** A dept-lead job summarizes the dept worklog and files
   improvement proposals as **AD/RG candidates** (Wave 3), which a human or
   gate promotes. Never auto-applied — the organism's `identity` synthesis
   engine makes the same rule explicit: *"only PROPOSES, never auto-applies."*
4. **ORG loop.** HR-equivalent efficacy rollup per employee: runs, tokens,
   tool calls, error rate, outcomes. Feed `error_classifier` output into the
   organism's **fixer routing** (G10): a persisted error-pattern registry whose
   entries tag a task `fix-pattern:<id>` and route it to a specialized fixer
   agent on the next tick.
5. **Kaizen as a first-class write.** `improvement_proposal` from a worklog
   entry is a first-class input to the DEPT loop. This is the mechanism that
   turns a run's friction into the next run's change, and it is the one piece
   operant's `learning_graph.rs` has the raw material for but no consumer.

**Gate:** an end-to-end soak: a fake failure is classified → a task is created
and tagged → the tick routes it to a fixer → the fixer's worklog entry carries
an improvement proposal → the DEPT loop files an AD candidate. All observable
in `operant status --json` / `operant architecture dump --live`.

**Size:** ~3 weeks.

---

## 5. Sequencing and the critical path

```
Wave 0  pool compiler honesty          ~2d   ← prerequisite, blocks all
   │
Wave 1  org layer (employee/notice/worklog)  ~2w
   │
Wave 2  fail-closed gate + maintenance + persona  ~2w   ← depends on Wave 1's employee ids
   │
Wave 3  AD/RG + derived registry        ~2w   ← depends on Wave 2's gate to validate against
   │
Wave 4  self-evolution loops            ~3w   ← depends on all of the above
```

Waves 1–2 are the load-bearing pair and are the minimum for "operant is a
first-class agentic OS": persistent employee identity, inter-agent
communication, self-reported worklogs, and a gate that blocks unsafe
autonomous execution. Waves 3–4 are what make it *governed* and
*self-improving*, and Wave 3 in particular should not start before Wave 2
exists, because a decision-record system with nothing enforcing the decisions
is documentation.

Total ≈ 11 weeks of one focused engineer, or ≈ 4 weeks with two.

---

## 6. The three highest-leverage items

If only three things get done:

1. **Wave 0 (2 days).** The pool compiler is green in CI and inert in
   production. Two days removes a false "✓" from the feature matrix and
   unblocks the entire org story.
2. **`--reason` on every mutating command (part of Wave 1, ~1 day).** AD-032's
   post-mortem is that 1,311 logged decisions were read by nobody. A required
   causal field is the cheapest possible fix for the hardest possible problem
   (agentic accountability), and operant currently has an *optional* `reason`
   on two kanban subcommands.
3. **Wave 2's mutation-proven gate (part of ~1 week).** Everything else in the
   plan makes operant more capable. This makes it *safe to leave running*.
   Without a gate that is proven to block, autonomy is a liability.

---

## 7. What I would deliberately NOT port

- **The 14.8k + 16.8k LOC of Python.** The organism's governance and runtime
  are Python because they were written first. Operant should implement the
  *contracts* natively and keep the organism as a **read-only import source**,
  not as a runtime dependency. Porting the code would import its
  fail-open bugs (`LB-RG-003` sync exiting 0 on a dead link; the RG-055
  misfiling churn) along with its contracts.
- **The 26,395 files of content.** Identity, ventures, campaigns, KosmOS — that
  is the user's *data*, not the OS. Operant should be able to **mount** it
  (Wave 0's `org route` / `org list` + the pool compiler), not reimplement it.
- **`access: cli` as the only integration mode.** The organism's RG-028
  "no duplication" rule is right; its enforcement (symlinks + a linter) is
  weaker than a typed seam. Operant's harness claims are the stronger
  mechanism and should be the default, with `access: cli` as a fallback for
  pools that have no operant provider.
- **The persona fields as free text.** The organism's `hr evolve` uses
  token-frequency on prompt text and its own AD calls the output "noisy"
  (`swarm/workforce-ops/AGENTS.md:98`). If operant builds persona evolution, it
  should derive from the **worklog** (structured, per-dept, with outcomes and
  blockers) rather than from raw token frequency.

---

## 8. Open questions for the owner

1. **Read-only first, or bidirectional?** The organism's pools are live systems
   with real cron jobs. Should operant's `org` layer *read* them and let operant
   drive them, or should `operant org` become the writer and the organism
   become a projection? This determines whether Wave 1 is additive or a
   migration.
2. **Where does the operant org root live?** Default
   `~/.operant/org/` mirroring the organism layout, or read the organism tree
   in place at `~/.hermes/organism/`?
3. **Should `agent_type` change cron scheduling semantics?** Mapping to the
   organism's TTLs (24h/1h/30m) is a real behavior change for existing jobs.
   Recommend additive-only: absent `agent_type` keeps today's behavior.
4. **Is the harness the right mount point for the whole org layer**, or should
   the org layer be a native subsystem with the harness wrapping it? The
   harness is appealing (unmountable, reversible) but org state is durable, and
   durable state behind a reversible mount is a design tension worth deciding
   explicitly before Wave 1.
5. **Scope check:** is the target one operant instance governing itself, or
   operant as the runtime for the *existing* 117-employee organism? These are
   very different projects and the plan above assumes the first.

---

## Appendix A — Verification commands used to produce this document

```bash
# organism strata and pools
cd ~/.hermes/organism && cat _org.yaml && find . -maxdepth 2 -type d | sort

# governance implementation size
wc -l cortex/meta-governance/lib/*.py            # 14,810 total
wc -l swarm/task-grid/scripts/*.py                # 16,811 total
wc -l swarm/workforce-ops/lib/hr_lib.py           # 1,830
wc -l swarm/agent-fabric/lib/agent_fabric.py      # 299

# per-pool gates
find ~/.hermes/organism -maxdepth 3 -name smoke.yaml

# the pool-compiler proof (shipped struct replicated verbatim, fed real files)
# → 6/6 real organism pools fail at parse: "services_offered[0]: missing field `name`"

# operant seam presence
grep -rln "notice_board\|NoticeBoard"        --include=*.rs crates/   # none
grep -rln "employee_id\|struct Employee"     --include=*.rs crates/   # none
grep -rln "worklog\|Worklog"                 --include=*.rs crates/   # none
grep -rln '\-\-reason'                       --include=*.rs crates/   # none
grep -rn "pub struct CronJob" -A 80 crates/operant-runtime/src/cron/types.rs
```

## Appendix B — Source map

| Claim in this doc | Source |
|---|---|
| Strata + allowed_top_level + orphan policy | `~/.hermes/organism/_org.yaml:24-32` |
| L1/L2/L3 layering | `swarm/agent-fabric/AGENTS.md:7-12` |
| 24 pools, 272 ADs, 149 RGs | `cortex/meta-governance/AGENTS.md:66-93` |
| `--reason` mandate + the 1,311-entries-read-by-nobody post-mortem | `core/decisions/AD-032-*.md` |
| Derived registry, staleness WARN | `AD-061`, `AD-074` |
| Config-driven pool genome | `AD-022`, `foundations/identity-core/_org.yaml` |
| Fail-closed pre-tick gate; per-check `reason` | `swarm/task-grid/core/decisions/AXE-AD-010-*.md` |
| Departments/employees/teams | `AXE-AD-011-*.md` |
| 5-field identity contract + TTLs | `AXE-AD-012-*.md`, `swarm/task-grid/AGENTS.md:98-105` |
| Routable notice board, 99.55% broadcast stat | `AXE-AD-013-*.md` |
| Persona contract + adoption table | `AXE-AD-016-*.md` |
| AgentSpec / WorklogEntry / LoopContract | `swarm/agent-fabric/lib/agent_fabric.py:100-200` |
| Maintenance contract, linter fails closed | `AD-078-*.md` |
| Pooling via exact-name symlinks | `AD-060-*.md` |
| "Entry points are executable verbatim" | `AD-070-*.md` |
| `pool.rs` compiles the wrong schema | `crates/operant-harness/src/pool.rs:32-136` + this session's probe |
| "✓ pool compiler" acceptance claim | `docs/harness-kernel.md:180` |
| 6/35 stale BUGS.md headlines | `docs/IMPLEMENTATION-PLAN.md` §1.3 |
| Split-brain from three stores for one concept | `BUGS.md` R5-1 |
| Memory-store three-location split | `BUGS.md` R5-1/R5-2 |
