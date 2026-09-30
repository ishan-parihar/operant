# Wave 1 Decision Document

**Status:** decisions of record. Four implementers build against this file. Every
schema below is binding; where it differs from
`docs/ORGANISM-OS-UPGRADE-OUTLINE.md`, **this file wins**.

**Measured against:** `operant/` @ `a3f73118`, 17 crates, working tree dirty
from a concurrent agent (25 modified files, all `crates/operant-core` +
`crates/operant-cli` telegram/loop work). The organism probed at
`~/.hermes/organism/` on 2026-09-30 16:48–16:52 IST.

**Companion doc:** `docs/ORGANISM-ARTIFACT-REFERENCE.md` did **not exist** at
the time this was written (checked, `ls` failed). Everything organism-side
below was read directly from `~/.hermes/organism/` rather than from a
verified-schema table. If that doc lands later and contradicts an organism
claim here, **re-verify the organism claim against the live files** before
acting — this doc's organism citations are direct file reads, but they are
not independently cross-checked.

---

## 0. The five answers, one line each

| # | Question | Decision | Tradeoff accepted |
|---|---|---|---|
| **Q1** | Read-only or bidirectional? | **Read-only import first. Operant never writes the organism tree in Wave 1.** The organism is *cold* (see §1.1), which makes a migration a cold-start, not a cutover — so there is no reason to pay for it now. | We give up the chance to unify the two rosters now. Cost: a one-time import step instead of a continuous sync, and the organism's ~117 employees will be shadow copies, not live ones. |
| **Q2** | Where does the org root live? | **`~/.operant/org/`, a *subdirectory* of `operant_home()`, for **content** (pool manifests, service rows). No `org_root` config key — one convention, not two.** Derived state goes in the existing sibling DBs, not a new tree. | We lose the "read the organism tree in place" convenience for ad-hoc tooling. Cost: one copy of pool manifests on disk, which must be re-imported to pick up organism edits. |
| **Q3** | Does `agent_type` change scheduling? | **Additive-only is SAFE, and confirmed by reading the code** — but the plan's premise is wrong: operant has **no TTL concept at all**, so the 24h/1h/30m map is not a behavior change *here*. See §1.3. The real risk is the **positional row mapper**, not the TTL. | Additive-only means we ship a TTL that governs *nothing* on the first tick. Cost: `agent_type` is inert until Wave 2 wires a consumer; we must say so in the docs or implementers will assume it does something. |
| **Q4** | Harness or native subsystem? | **Split. Durable state (employee registry, notice board, worklog) = native, plain sqlite, in operant-core, never unmounted. Model-callable *access* to it = a harness provider behind the `tool` seam, so the tools are unmountable.** | The durable stores gain one unshippable dependency (rusqlite) that operant-harness must not take. Cost: the org layer cannot be mounted as a unit; unmounting the provider removes the *tools*, not the *data*. |
| **Q5** | One instance or 117 employees? | **One operant instance governing itself, for the next 3 months.** But note the organism is dormant, so "governing the organism" is *not* the high-risk project it appears — the real risk is operant governing its own live cron. | We defer the 117-employee story, which means the org layer ships with a handful of real employees and thin test coverage from natural traffic. Cost: the DEPT/PEER loops in Wave 4 have almost no real data until the organism is reactivated. |

---

## 1. The five decisions, with evidence

### 1.1 Q1 — Read-only first, and the organism is *cold*

**Recommendation: read-only.** But the decisive evidence is not the one the
plan led with.

The plan (§8 Q1) asserts "the organism's pools are live systems with real cron
jobs." **That is not true as of 2026-09-30.** Measured:

```
~/.hermes/cron/jobs.json          102 jobs, 64 enabled
  updated_at                       2026-09-28T04:45:58  (2.4 days stale)
  max(last_run_at)                 2026-09-11T14:50:58+05:30  (19 days stale)
  jobs with last_run_at set        78 / 102
  max(next_run_at)                 2026-10-01T04:25:00+05:30  (tomorrow, never fires)
  jobs w/ last_run_at within 24h    0
  jobs w/ next_run_at within 1h     0
~/.hermes/cron/executions.db       executions table: 0 rows
```

A scheduler that had fired in the last day would have written both a fresh
`updated_at` and a non-zero `last_run_at` on the 64 enabled jobs. Neither is
present. No `axe`/tick process is running (`ps aux | grep -E "axe|tick"` — no
hits). The user's `crontab -l` contains three `pioneer-qf` watchdog lines and
**nothing** from the organism; `systemctl --user list-timers` shows 8 timers,
none organism-owned.

What *is* live is the **sync watcher**, not the fleet:

```
/home/ishanp/.hermes/scripts/hermes-sync-watch.sh   (PID 1274518, since Sep28)
  inotifywait ... /home/ishanp/.hermes/{systems,scripts,data,memories,skills,cron}
```

That is why 237 organism files have Sep-29/30 mtimes: the `references/INDEX.md`
and `AGENTS.md` churn is `arch emit` running from a human/agent session, not a
tick spawning employees. The organism's own content is being maintained; its
**execution loop is stopped**.

**Blast radius of the recommendation:**

- *Read-only (chosen):* operant imports 133 employee records
  (`~/.hermes/organism/swarm/task-grid/data/employees.json`, 135,667 bytes) and
  102 cron jobs as **frozen snapshots**. No operant code touches the organism
  tree. Blast radius if wrong: zero — a stale import is a re-run.
- *Bidirectional (rejected):* operant becomes the writer for
  `employees.json` + `jobs.json` + `notice-board.jsonl` + 24
  `core/worklogs/*.jsonl`. That is 4 live stores with 3 different owners
  (hermes runtime, the sync watcher, `arch`). Blast radius if wrong: silent
  corruption of the sync watcher's inputs, and the watcher's
  `close_write` trigger means operant writing `jobs.json` races a
  `git`-syncing process. **Not acceptable while the watcher is live.**

**Migration cost if we later flip to bidirectional:** near zero, and that is
the argument for waiting. The organism is cold, so a migration is a cold-start
copy, not a live cutover. There is no window where we must act now.

**One caveat I could not fully close:** the organism's `_org.yaml` at
`~/.hermes/organism/_org.yaml:1-4` says the employee roster is "a DERIVED cache
regenerated by `axe org converge` — never hand-edit it." So `employees.json`
is a derived artifact, not a source. Importing it is correct (it is the
*converged* view), but operant must not later write it. Reinforces read-only.

### 1.2 Q2 — `~/.operant/org/`, and the convention that decides it

**The convention: every operant path derives from `operant_home()`.** Not
`$XDG_DATA_HOME`, not a config key, no per-subsystem root override.

`crates/operant-core/src/platform.rs:95-119`:

```rust
/// Resolution order:
/// 1. `$HERMES_HOME` environment variable (if set and non-empty).
/// 2. `~/.operant` on Unix, `%APPDATA%\operant` on Windows.
pub fn operant_home() -> PathBuf {
    if let Ok(val) = env::var("HERMES_HOME") && !val.is_empty() {
        return PathBuf::from(val);
    }
    #[cfg(target_os = "windows")] { /* %APPDATA%\operant */ }
    #[cfg(not(target_os = "windows"))] {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".operant")
    }
}
```

And the *named* sub-path helpers follow mechanically —
`platform.rs:121-140` defines `operant_config_dir()`, `operant_data_dir()`,
`operant_memories_dir()`, `operant_skills_dir()`, `operant_sessions_dir()`,
each a one-line `.join()` onto `operant_home()`. 65 call sites across
20+ files use `operant_home()` directly.

**Decision:** add `operant_org_dir() -> PathBuf { operant_home().join("org") }`
in `platform.rs` next to its five siblings. It is the same shape as the other
five, costs four lines, and makes the invariant greppable.

**Why no `org_root` config key:** `$HERMES_HOME` already provides the override
(`platform.rs:99-103`). A second knob is a second precedence rule, and the
repo has already paid for a split-brain from a third location
(`BUGS.md` R5-1, the `memory/` split). The plan's own design rule §4 says
"no second store ... Three divergent locations for one concept is a bug operant
has already paid for once." A config-key org root is exactly that shape.

**What goes where — this matters and the plan does not say:**

| Artifact | Location | Why |
|---|---|---|
| Pool manifests / service rows (content) | `~/.operant/org/pools/<pool>.yaml` | new content tree under `operant_org_dir()` |
| Employee registry (durable state) | `operant_kanban.db` | see §3.1 — sibling-DB convention |
| Notice board (durable state) | `operant_kanban.db` | same |
| Worklog (durable state) | `operant_kanban.db` | same |

**The convention for derived state is "sibling DB next to the domain DB", not
"a directory under the org root."** Evidence:
`crates/operant-cli/src/main.rs:967-971` derives every subsystem DB from one
base:

```rust
let db_dir = config.database_path.parent().unwrap_or_else(|| std::path::Path::new("."));
let cron_path    = db_dir.join("operant_cron.db");
let kanban_path  = db_dir.join("operant_kanban.db");
```

with the same rule restated at `crates/operant-cli/src/cmd_cron.rs:82-92`:
"MUST match `main.rs` ... pointing the CLI at the shared `database_path` made
CLI-created jobs invisible to the scheduler and tripped the shared-PRAGMA
migration guard (R39-7)." That comment is a scar from getting this wrong.
Reproduce the rule; do not invent a new axis.

Live layout confirms it: `~/.operant/` holds `operant_cron.db`,
`operant_kanban.db`, `database.db`, `memory_wire.sqlite` — all flat siblings,
none in a subdirectory.

### 1.3 Q3 — Additive-only is safe. The plan's *premise* is wrong.

**Finding 1 — operant has no TTL concept anywhere.** Grep for TTL/session
expiry in the cron path returns nothing. `CronJob`
(`crates/operant-runtime/src/cron/types.rs:143-174`) has no TTL, no
`agent_type`, no `session_lifetime`, no expiry. The scheduler
(`scheduler.rs:92-118`) polls on an interval, reads `due_jobs`, and executes;
it never ages anything out. So mapping `agent_type` → 24h/1h/30m introduces a
**new** field, not a **changed** one. The plan's phrasing — "is a real behavior
change for existing jobs" (`OUTLINE.md:544-546`) — does not hold for operant.

**Finding 2 — the additive pattern is already proven safe.** Two new columns
(`source`, `uses_memory`) were added post-hoc exactly this way:

- Struct side: `types.rs:166-168` and `types.rs:161-165` both use
  `#[serde(default = "...")]` on the struct fields, so absent JSON is `None`/
  `"imperative"`/`true` rather than a deserialize error.
- DB side: `store.rs:1224-1225` calls `add_column_if_missing(conn, "source",
  "TEXT DEFAULT 'imperative'")` and `add_column_if_missing(conn, "uses_memory",
  "INTEGER NOT NULL DEFAULT 1")`. `add_column_if_missing`
  (`store.rs:1015-1044`) PRAGMA-checks `table_info` first, and tolerates
  `duplicate column name` for the concurrent-migration race.
- Read side: `map_cron_job_row` (`store.rs:648-690`) reads the new columns
  positionally (`row.get(18)`, `row.get(19)`) and coalesces with
  `source.unwrap_or_else(|| "imperative")` / `uses_memory != Some(0)`.

**So: no existing code path can be perturbed by a new field with a default,
because the pattern is already in production.** Verified.

**Finding 3 — the ACTUAL risk the plan missed: positional row mapping.**
`map_cron_job_row` reads by **integer index** (`row.get(17)` for
`allowed_tools`, `row.get(19)` for `uses_memory`), and the four SELECTs that
feed it all hard-code the column list in the same order:

```
store.rs:154  (get_job)
store.rs:178  (list_jobs)
store.rs:217  (due_jobs)
store.rs:252  (all_overdue_jobs)
```

**Any implementer who appends a column to those SELECTs without appending the
identical column to all four will silently mis-map every field after the
insertion point** — a `row.get(N)` of the wrong type surfaces as a
`sql_conversion_error`, but a same-typed column (two TEXT columns adjacent)
reads as a *plausible wrong value*. This is a real hazard, not a theoretical
one, and it is the thing to put in the PR description.

**Answer to the literal question:** yes, additive-only is safe, and it is safe
*by an already-proven mechanism* rather than by luck. The constraint is: the
new columns must be added to all four SELECTs, the `map_cron_job_row` literal,
`initialize_schema`'s CREATE TABLE, and the `add_column_if_missing` list, in
the same commit.

### 1.4 Q4 — Native stores, harness-mounted tools. Split.

**The tension is real but it is not a genuine dilemma** once the two halves
are named separately. The harness is a *capability registry with reversible
side effects* (`crates/operant-harness/src/lib.rs:3-8`: string-keyed claims,
PENDING late binding, "reversible effects — every registration returns an undo
handle; unload unwinds LIFO"). Durability is not a harness concept at all. The
`Effect` type (`effect.rs:43-60`) is a `Box<dyn FnOnce() -> BoxUndoFuture>` —
a *teardown callback*, not a *state store*. Asking the harness to own the
employee registry means asking an undo closure to own a sqlite table.

**Concrete split:**

| Store | Home | Mounted? | Reason |
|---|---|---|---|
| Employee registry table | sqlite, `operant_kanban.db` | **No** | durable identity; an unmount that drops it is data loss |
| Notice board tables | sqlite, `operant_kanban.db` | **No** | durable audit trail (organism's own AD-006 calls it one) |
| Worklog table | sqlite, `operant_kanban.db` | **No** | append-only record, must survive restarts |
| `notice_post` / `notice_inbox` / `notice_ack` / `worklog_append` / `org_status` tools | `ToolRegistry` via the `tool` seam | **Yes** | capability surface; unmount = "this deployment may not use the board" |
| SELF-loop prompt injection | `prompt.section` seam | **Yes** | genuinely a swappable context provider |

**Why sqlite and not the harness composition file:** the composition file is
`architecture.toml`, declared state, read at boot
(`docs/harness-kernel.md:44-55`). Worklog and notice rows are *emitted* at
runtime by agents. Writing runtime data into boot config is the inverse of the
design.

**The load-bearing constraint the implementers must respect:**
`operant-harness` currently depends on **no** other operant crate and no
database driver — see `crates/operant-harness/Cargo.toml` in full
(`async-trait`, `serde`, `serde_json`, `serde_yaml`, `thiserror`, `tokio`,
`toml`, `tracing`; dev-dep `tempfile`). Its lib.rs:11-12 states the crate "is
**PURE**: it depends on no other operant crate." **Adding rusqlite to
operant-harness would be the first violation of that stated invariant.** The
org stores therefore live in `operant-core` (which already has rusqlite:
`crates/operant-core/src/database.rs:11` `use rusqlite::{Connection, params};`,
and `kanban/db.rs:150` `Connection::open(path)`), and the *provider* that
exposes them as tools lives in operant-core too, implementing
`operant_harness::Provider`.

**Follow the existing pattern for exactly this split.** `pool.rs` in the
harness is a *pure compiler* (manifest → data, no I/O), and the I/O adapter is
`pool_provider.rs` in the same crate but registered by the host. The analogous
split for org: pure schema/types in one place, the `Provider` that owns the
sqlite handle in `operant-core`, registered through the `tool` seam
(`provider.rs:143-149` `Seam::install` returning an `Effect` whose undo calls
`ToolRegistry::unregister_tool`).

### 1.5 Q5 — One operant instance, for three months

**Recommendation: one instance governing itself.**

The risk asymmetry, measured:

- Operant's cron is **live**: `~/.operant/operant_cron.db` was written
  2026-09-30 16:14 (today), and `~/.operant/database.db` at 16:14. It runs via
  `crates/operant-core/src/cronjobs/scheduler.rs:44-53` (`start()`, a 60s
  poll loop) spawned from
  `crates/operant-cli/src/gateway_runner.rs:2724-2731`.
- The organism's cron is **cold** (19-day-stale `last_run_at`, 0 executions).

So "operant governing itself" is the *harder* and therefore the more valuable
project — it is the one with a live tick to fail closed on. "Operant governing
the 117-employee organism" is, right now, the *easier* project, because the
organism has no running tick to break. The plan's §8 Q5 implies the opposite
difficulty ordering.

**Risk of the 117-employee path:** the import would carry 133 employee records
and 102 job definitions into operant's employee table, 102 of which would then
be subject to a fail-closed gate. From the measured data (§3.1), of the 102 jobs
**2 have `agent_type = None`**, **34 lack `continuity`**, and **6 have an empty
`skills` array**. So a naive import trips the gate on day one and blocks the
tick — and if `continuity` joins the Wave 1 identity set, 34 jobs block, not 2.
Shipping the gate against 102 real jobs before the gate is proven is the wrong
order of tests.

**What unblocks the 117 path (Wave 3+):** run
`operant org import ~/.hermes/organism` once the gate is mutation-proven, in
report-only mode, and count violations.

---

## 2. Corrections to the plan document

Each of these is a place `docs/ORGANISM-OS-UPGRADE-OUTLINE.md` is wrong or
optimistic, with the evidence.

| Plan claim | Line | Finding |
|---|---|---|
| "The organism's pools are live systems with real cron jobs" | `OUTLINE.md:536-537` | **False as of 2026-09-30.** 0 jobs ran in 24h; max `last_run_at` is 2026-09-11; `executions.db` has 0 rows; no tick process. The *sync watcher* is live, which is what makes the trees look fresh. |
| "Wire it to data operant already has: `agent/cost.rs` (tokens)" | `OUTLINE.md:347-348` | **Wrong path.** `crates/operant-core/src/agent/cost.rs` does not exist. Token data lives in `crates/operant-runtime/src/agent/cost.rs` (a different crate) and, for the *core* agent, in the `AgentEvent::Usage` emit at `crates/operant-core/src/agent/compress.rs:222`. See §3.4. |
| "`learning_graph.rs` (`next_intent`)" | `OUTLINE.md:348` | **`next_intent` does not exist anywhere in operant.** `grep -rn next_intent crates/` returns zero hits; the only two hits in the repo are in the plan itself. `learning_graph.rs` does exist but exposes `build_learning_graph`, `node_detail`, `NodeKind`, `GraphEdge` — no intent field. See §3.4. |
| "Worklog ... mirrors the organism's **15** fields" | `OUTLINE.md:345-346` | **20 fields, and the plan's own list has 16.** The real `WorklogEntry` dataclass at `~/.hermes/organism/swarm/agent-fabric/lib/agent_fabric.py:141-167` has 20 attributes: the 16 the plan enumerates at `OUTLINE.md:120-124` plus 4 framework-only FK/link fields (`correlation_id`, `notice_ids`, `session_id` — plus `id`, which the plan's list omits entirely). So the plan's prose number (15), its enumerated list (16), and the organism's reality (20) are three different numbers. See §3.4. |
| "`CronJob` (30 fields)" | `OUTLINE.md:158`, `OUTLINE.md:328` | **31** in `operant-core/src/cronjobs/db.rs:37-69`; **20** in `operant-runtime/src/cron/types.rs:143-174`. The plan's number matches neither. |
| "operant's `CronJob` already carries `name`, `source`, `allowed_tools`, `uses_memory`, `model`, `delivery`" | `OUTLINE.md:328-330` | True of `operant-runtime`'s struct; **false of the one that actually runs** (§3.0). The live `operant-core` `CronJob` has `name`/`model` but **no `source`, no `allowed_tools`, no `uses_memory`**, and `delivery` is a flat `deliver: String`. |
| "`operant-runtime/src/cron/` ... **Strong**" | `OUTLINE.md:158` | Half-true. That crate's scheduler is reachable **only** from `operant-runtime/src/daemon/mod.rs:266`, and the CLI has no daemon entry point (grep: only two *comments* reference it, `cmd_channel.rs:99` and `cmd_gateway.rs:388`). The cron that runs in the shipped binary is `operant-core`'s. See §3.0. |
| "`--reason` ... `none` (`cmd_kanban` has an *optional* `reason`)" | `OUTLINE.md:185` | Half-true, and it under-counts. There are **two** `reason` fields in kanban, and they **disagree**: `cmd_kanban.rs:71` is `Option<String>` (optional) and `cmd_kanban.rs:115` is `String` (**required**). See §3.5. |
| "the organism's own version needed a perf fix from 65s to 0.4s for 1.8k entries" | `OUTLINE.md:341-342` | **Could not verify.** `axe_lib.py:753-758` `retention_gc` has a `max_weekly_summaries_per_tick=4` batching parameter, which corroborates the *shape* of the claim, but the 65s→0.4s numbers appear in no file I read. Treat as folklore. The batching parameter is the real, citable lesson. |
| "`operant doctor --gate` → exit 2 ... have every autonomous entry point refuse to spawn" | `OUTLINE.md:373-375` | Directionally right, but the plan does not name the **one** tick entrypoint that must carry the check, and there are **two** candidate loops (§2.2). Naming both is a prerequisite this doc supplies. |
| "every existing `CronJob` backfills to a valid employee" | `OUTLINE.md:358-360` | **Falsified by the live DB.** 2 of 102 real jobs have no `employee` and no `agent_type`; **6 have an empty `skills` array**; 34 lack `continuity` (`has employee: 100`, `skills non-empty: 96`). The acceptance criterion as written cannot pass today. §2.2 adjusts it. |

---

## 3. Wave 1 schemas

**Storage decision for all three stores:** new tables in
`~/.operant/operant_kanban.db` (derived per §1.2 from
`config.database_path.parent()`). Not a new DB file, not a JSONL file, not the
harness composition file. This satisfies the plan's "no second store" rule
(`OUTLINE.md:277-280`) and adds zero new paths to reason about.

The `kanban/db.rs` `CREATE TABLE IF NOT EXISTS` batch runs inside
`KanbanDb::init` (`kanban/db.rs:150` `Connection::open(path)`, schema block at
`kanban/db.rs:168-232`), so the new tables are **additive and
backward-compatible** with existing installs — the same idempotent style as
`cronjobs/db.rs:168-172`'s comment ("the `IF NOT EXISTS` guards are kept
inside each entry so the entries remain idempotent at the SQL level").

### 3.1 Employee registry

```sql
CREATE TABLE IF NOT EXISTS employees (
    employee_id      TEXT PRIMARY KEY,          -- 'emp-' || 12 hex; see §3.1.1
    name             TEXT NOT NULL,             -- human label, from cron job name
    role             TEXT NOT NULL DEFAULT 'automator',
    department       TEXT,                      -- NULL until Wave 3 registry
    skills           TEXT NOT NULL DEFAULT '[]',-- JSON array of leaf names
    agent_type       TEXT,                      -- NULL|session|service|fixer  (§3.1.2)
    persona          TEXT,                      -- JSON Persona, NULL until Wave 2
    status           TEXT NOT NULL DEFAULT 'active',
    reason           TEXT NOT NULL,             -- the --reason of the creating write
    created_at       TEXT NOT NULL,             -- RFC3339
    updated_at       TEXT NOT NULL              -- RFC3339
);

CREATE TABLE IF NOT EXISTS employee_cron_jobs (
    employee_id  TEXT NOT NULL,
    cron_job_id  TEXT NOT NULL,
    schedule     TEXT,          -- human display string, e.g. '*/15 * * * *'
    enabled      INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (employee_id, cron_job_id),
    FOREIGN KEY (employee_id) REFERENCES employees(employee_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_employees_dept     ON employees(department);
CREATE INDEX IF NOT EXISTS idx_employees_status   ON employees(status);
CREATE INDEX IF NOT EXISTS idx_employee_cron_job  ON employee_cron_jobs(cron_job_id);
```

Rust model:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    /// A long-lived operational role — infrastructure, monitoring, a service
    /// that is always on.
    Service,
    /// A role that lives for one working session: research, drafting, a
    /// scheduled piece of work with a beginning and an end.
    Session,
    /// A standing remediation role. A fixer is *hired*, like a janitor or an
    /// engineer: it has a permanent identity and a persistent seat, and it is
    /// on call rather than being consumed by the work.
    Fixer,
}
```

`AgentType` has **no `#[default]`** — absence is `Option<AgentType> == None`,
which is distinct from all three values and preserves "absent keeps today's
behavior" (Q3).

**Correction (iter-518): there is no TTL in this enum, and there never was.**
The draft above carried the organism's doc comments — `Service` = "24h registry
TTL", `Session` = "1h TTL, GC'd after the run", `Fixer` = "30m TTL". Those
numbers exist **only in a Python comment** in the organism and are enforced
nowhere: no code reads them, and Operant has no expiry mechanism on employee
rows. Keeping them in the Rust doc would have been inventing a contract. The
variants are now described by what the role *is*:

- `Fixer` is a **standing permanent role**, not an expiring contractor. A
  fixer holds a seat and an identity and is on call.
- `Service` is a persistent operational role.
- `Session` describes **bounded work**, not employee expiry.

**No employee will ever be deleted because of age or role type.** Deletion is
an explicit, reason-bearing operator action (`org employee retire`), and the
only GC in this surface is `NoticeBoard::retention_gc`, which expires
*notices* by their own TTL — never identities.

**Required vs optional, explicitly:**

| Field | Required | Default on backfill | Notes |
|---|---|---|---|
| `employee_id` | **yes** | derived, §3.1.1 | primary key |
| `name` | **yes** | `cron_job.name` | `String`, non-empty enforced |
| `role` | no | `"automator"` | organism uses `automator`; see `employees.json` sample |
| `department` | no | `None` | operant has no department concept yet; NULL is honest |
| `skills` | **yes** | `[]` — **fails the gate** | see §2.2 |
| `agent_type` | no (enforced only for long-lived, §2.2) | `None` | |
| `persona` | no | `None` | Wave 2 fail-closed |
| `status` | no | `"active"` | |
| `reason` | **yes** | `"org import: backfill from cron job"` | every row records why it exists |
| `created_at` | **yes** | `cron_job.created_at` | |
| `updated_at` | **yes** | now | |

#### 3.1.1 Backfill map from the live `CronJob`

**There are two `CronJob` types and only one is live.** This is the single
most important thing in this document for the implementer.

| | `operant-core/src/cronjobs/db.rs:37-69` | `operant-runtime/src/cron/types.rs:143-174` |
|---|---|---|
| fields | **31** | 20 |
| table | `cron_jobs` in `operant_cron.db` (`db.rs:131-163`) | `cron_jobs` in `<workspace_dir>/cron/jobs.db` (`store.rs:1177-1197`, path at `store.rs:1045-1047`) |
| runs from | **yes** — `cronjobs/scheduler.rs:44` `start()`, spawned at `gateway_runner.rs:2724-2731` | only via `operant-runtime/src/daemon/mod.rs:266`, which the CLI never calls |
| chosen by | `CronDb::init(cron_db_path)`, `cmd_cron.rs:86-92` + `main.rs:967-970` + `gateway_runner.rs:2723` | — |

**Backfill reads `operant-core`'s `CronJob`.** Field map:

| `employees` column | ← `CronJob` field | Conversion |
|---|---|---|
| `employee_id` | `id` | `format!("emp-{}", &job.id[..12])` — see §3.1.2 |
| `name` | `name` | direct (`String`, `db.rs:38`) |
| `role` | — | constant `"automator"` |
| `department` | — | `None` |
| `skills` | `skills: Option<Vec<String>>` (`db.rs:50`) | `serde_json::to_string(skills.unwrap_or_default())` |
| `agent_type` | — | `None` (absent) |
| `persona` | — | `None` |
| `status` | `state: String` (`db.rs:60`) | `"active"` if `state == "scheduled"`, else `state` |
| `created_at` | `created_at: String` (`db.rs:63`) | direct |
| — | `enabled: bool` (`db.rs:59`) | → `employee_cron_jobs.enabled` |
| — | `schedule: String` (`db.rs:41`) | → `employee_cron_jobs.schedule` |
| — | `prompt`, `model`, `deliver`, `script`, `no_agent`, `workdir`, `origin_*`, `last_*`, `repeat_*` | **not carried** — the cron row already owns them |

Fields the plan names as already present on `CronJob` that are **absent** on
the live struct: `source`, `allowed_tools`, `uses_memory`. **Do not write a
backfill that reads them.** (`skills` *is* present, at `db.rs:50`.)

#### 3.1.2 `employee_id` derivation and collisions

`id: String` (`db.rs:37`) is a caller-supplied job id with **no uniqueness
guarantee across the 12-hex truncation**. The organism uses
`emp-<first 12 hex of the job id>` (verified: `employees.json` has
`emp-a02e3f692fb0` whose `home_crons[0].cron_id` is `a02e3f692fb0`).

**Rule:** `employee_id = "emp-" + job.id[..min(12, len)]`, and if that
truncation collides with an existing employee, the backfill must **fail loudly
and skip the job** — not silently append a disambiguator. A colliding id means
two different jobs map to one identity, which is exactly the drift the
organism's `axe org check` exists to catch (`smoke.yaml`, last check: "org
roster drift (orphan employees / jobs without employees / hollow teams) blocks
the tick"). Determinism beats cleverness here; a collision is a bug to see.

**What happens to a `CronJob` lacking required identity fields:** it does not
get a partial employee row. It gets **no row**, and the job is recorded in the
gate's violation set (§2.2). The tick blocks. A half-employee that cannot
carry `skills` is worse than a missing employee, because the missing one is
visible.

### 3.2 Backfill + fail-closed rule

**Required identity fields (exactly three):**

1. `employee_id` — derivable, but must be non-empty and collision-free (§3.1.2)
2. `name` — non-empty after trim
3. `skills` — **present and non-empty**

`agent_type`, `department`, `persona` are **not** in the required set for
Wave 1. This is a deliberate narrowing of the organism's 5-field contract
(AXE-AD-012: `employee`, `skills`, `agent_type`, `continuity`,
`registers_on_spawn`) and is required to make the gate passable against the
live DB. Measured against the 102 real jobs, that choice is what keeps the
block set small: `skills` is empty on 4 jobs, `agent_type` is absent on 2, and
`continuity` is absent on 34. Requiring the organism's full 5-field set would
block 34+ jobs on day one.

**Correction (iter-518): the block set is 4, not 6.** The original figure read
`CronJob.skills` (plural) alone. `CronJob` carries **both** `skill` and
`skills` (`db.rs:49-50`); over the live 102 jobs, 91 set `skill`, 96 set
`skills`, 89 set both, and 6 have an empty plural. Two of those six were
false positives — jobs that declare their skill only in the singular field, so
the plural-first read produced `[]` for a perfectly well-specified job (e.g.
`210544ad44bd` Web Deploy Health Daily, `skill: website-design`). The
backfill is now plural-first with a **singular fallback**; a job with neither
field set still backfills to `[]` and is still reported invalid, because the
fallback must use the job's own declared skill and never invent one. The four
genuine blockers are two disabled legacy jobs and two enabled empty-prompt
placeholders.

**Where the check runs.** Both cron loops, because both can execute jobs:

**Primary (this is the one the plan should have named):**
`crates/operant-runtime/src/cron/scheduler.rs:254-276`

```rust
async fn execute_and_persist_job(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
    component: &str,
    observer: Option<Arc<dyn crate::observability::Observer>>,
) -> (String, bool, String) {
    crate::health::mark_component_ok(component);
    warn_if_high_frequency_agent_job(job);
    let started_at = Utc::now();
    let (success, output) = Box::pin(execute_job_with_retry(config, security, job, observer)).await;
```

This is the **single funnel** for the runtime loop: `process_due_jobs`
(`scheduler.rs:207-253`) maps every due job through it, and
`catch_up_overdue_jobs` (`scheduler.rs:122-158`) calls `process_due_jobs` too.
So one insertion point at the top of `execute_and_persist_job` — before
`execute_job_with_retry` — covers both the poll loop and startup catch-up.

Two more entry points bypass it and need the same check:
- `scheduler.rs:160-167` `execute_job_now` (public; called by
  `operant-gateway/src/api.rs:458` and `operant-runtime/src/tools/cron_run.rs:119`)
- `operant-core/src/cronjobs/scheduler.rs:80` `run_job` (the **live** loop)

**Secondary (live today):** `crates/operant-core/src/cronjobs/scheduler.rs:80-86`

```rust
async fn run_job(&self, job: &CronJob) -> Result<(), Error> {
    info!("Executing cron job {}: {}", job.id, job.name);
    let (success, _output, final_response, error_msg) = if job.no_agent {
```

`tick()` (`scheduler.rs:54-77`) is called by `start()`'s 60s loop
(`scheduler.rs:44-53`) from `gateway_runner.rs:2731`.

**Exact error surface.** Two rules, and the asymmetry is deliberate.

1. **At the tick funnel: BLOCK.** The gate returns before
   `execute_job_with_retry` is called. It must:
   - emit `tracing::error!` (not `warn!`) with the job id, the employee id (or
     the reason it could not be derived), and the **names of the missing
     fields**;
   - call `crate::health::mark_component_error(SCHEDULER_COMPONENT, msg)` —
     the same function `run()` already calls on a failed `due_jobs` query
     (`scheduler.rs:97-102`), so the failure is visible on
     `operant status` with no new plumbing;
   - broadcast `{"type": "cron_result", "job_id": ..., "success": false,
     "output": "...blocked by org gate: missing <fields>...", "timestamp": ...}`
     over `event_tx` (`scheduler.rs:242-250`) so the dashboard shows it;
   - **return** without executing. No retry — the job's next attempt is its
     next scheduled tick, and a retry loop on a *configuration* error is a
     busy-spin. This mirrors the existing non-retryable branch at
     `scheduler.rs:193-196`, where a `blocked by security policy:` prefix is
     returned immediately rather than retried.

2. **At the CLI: `operant org check` exits 2.** Same message set. Non-zero
   exit is the established operant convention for gate failure (plan §4's
   "mutation-proven-red" discipline), and the smoke-gate precedent in the
   organism also uses exit 2 (`OUTLINE.md:183`).

**Exact block message** (so it is greppable, like the organism's self-
documenting `reason:` fields in `smoke.yaml`):

```
blocked by org gate: cron job '{job_id}' has no valid employee record \
(missing: {comma_separated_field_names}); fix with `operant org sync` or \
`operant org backfill` and re-run
```

The `blocked by org gate:` prefix is the machine-readable marker, in the same
spirit as the existing `blocked by security policy:` prefix at
`scheduler.rs:194` that callers already string-match on.

**Adjusted acceptance criterion.** The plan's gate — "every existing
`CronJob` backfills to a valid employee" (`OUTLINE.md:358-360`) — **cannot pass
today** and must be restated as:

> Every existing `CronJob` backfills to an employee record **or** is reported
> in `operant org check` output as a named violation, and the tick blocks for
> exactly that set.

A test must construct a `CronJob` with empty `skills`, assert the gate
returns the block, and assert `execute_job_with_retry` was never called
(mutation-proven red, per `OUTLINE.md:283-284`).

### 3.3 Notice board

Sqlite in `operant_kanban.db`. No JSONL file.

```sql
CREATE TABLE IF NOT EXISTS notices (
    id              TEXT PRIMARY KEY,      -- 'n_' || uuid v4 (organism: AXE-AD-013)
    created_at      TEXT NOT NULL,         -- RFC3339; indexed for GC + inbox queries
    epoch           INTEGER NOT NULL,      -- unix seconds; the inbox-bridge query key
    sender          TEXT NOT NULL,         -- employee_id, or 'user', or 'system'
    from_dept       TEXT,                  -- sender provenance; NULL => 'unknown' (AD-013 §2)
    recipients      TEXT NOT NULL,         -- JSON array of typed selectors
    subject         TEXT,
    body            TEXT NOT NULL,
    correlation_id  TEXT,                  -- uuid; request -> ack -> result chain
    ack_required    INTEGER NOT NULL DEFAULT 0,
    acked_by        TEXT,                  -- JSON array of employee_ids that acked
    acked_at        TEXT,
    ttl_expires_at  TEXT,                  -- RFC3339; NULL => never expires
    tags            TEXT NOT NULL DEFAULT '[]',  -- JSON array
    thread_id       TEXT,                  -- explicit thread anchor
    pinned          INTEGER NOT NULL DEFAULT 0, -- retention pin (axe retention-pins.jsonl)
    reason          TEXT NOT NULL          -- the --reason of the posting write
);

CREATE INDEX IF NOT EXISTS idx_notices_created    ON notices(created_at);
CREATE INDEX IF NOT EXISTS idx_notices_correlation ON notices(correlation_id);
CREATE INDEX IF NOT EXISTS idx_notices_thread      ON notices(thread_id);
CREATE INDEX IF NOT EXISTS idx_notices_epoch       ON notices(epoch);
```

**Typed recipients** — the AD-013 selector set, stored as a JSON array so
multi-recipient is first-class and no schema change is needed to add a kind:

| Selector | Resolves to | Stored |
|---|---|---|
| `agent:<employee_id>` | one employee | verbatim |
| `dept:<slug>` | all active employees in dept | verbatim (resolved at read time) |
| `team:<slug>` | all members of a team | verbatim (Wave 3) |
| `role:<capability>` | employees with that skill | verbatim |
| `broadcast` | every reader | verbatim, default |

Resolution is **at read time, not write time**, so a notice posted to
`dept:platform-infra` reaches an employee onboarded an hour later. This is a
deliberate divergence from the organism's JSONL, where a `dept:` notice is a
tag and the inbox bridge has to query it separately
(`AXE-AD-013` §5).

**Provenance.** `sender` + `from_dept` are separate and both required-ish
(AD-013 §2 makes `from_dept` default to `unknown` for legacy; here `NULL` and
a read-time `COALESCE(from_dept, 'unknown')` reproduce that).

**Ack.** `ack_required` + `acked_by` array. An ack is a `notice_post` with the
same `correlation_id` and tag `ack` — matching the organism's request→ack→
result protocol (AD-013 §3-4) — *plus* a direct `UPDATE` on the parent row so
inbox queries do not have to join. Both are written in one transaction.

**Retention / GC — batched, and the batching is the point.**

Constants (from `~/.hermes/organism/swarm/task-grid/scripts/axe_lib.py:91-92`):
```python
RAW_RETENTION_DAYS = 14       # Keep 14 days of raw entries
SUMMARY_RETENTION_WEEKS = 16  # Keep 16 weeks of weekly summaries (~4 months)
```

The organism's own `NoticeBoard.retention_gc` takes
`max_weekly_summaries_per_tick=4` (`axe_lib.py:753-758`) — it summarizes at
most 4 weeks per tick. **Carry that parameter.** A GC that summarizes every
elapsed week in one pass is the unbounded-work failure the parameter exists to
prevent. Operant's version, per tick:

1. `DELETE FROM notices WHERE created_at < :raw_cutoff AND pinned = 0 AND
   correlation_id NOT IN (SELECT correlation_id FROM notices WHERE
   correlation_id IS NOT NULL GROUP BY correlation_id HAVING
   COUNT(*) > 1)` — a chain is kept whole or not at all.
2. Summarize at most 4 completed weeks into synthetic
   `tags = ["weekly-summary"]` rows carrying
   `metadata = {week_start, week_end, entry_count, tag_distribution,
   agent_distribution, outcomes}` (AD-006 §"Weekly summaries").
3. `DELETE FROM notices WHERE tags = '["weekly-summary"]' AND
   created_at < :summary_cutoff`.

Pinned rows are never deleted — `axe_lib.py:761-763` documents that pins come
from worklog links, open chains, and `retention-pins.jsonl`, which is why
`pinned` is a column here rather than a sidecar file. The plan's "no second
store" rule (`OUTLINE.md:277-280`) forbids the sidecar.

Run GC from the same place the organism runs it (every tick, step 0c per
AD-006), i.e. at the **top of `process_due_jobs`**
(`scheduler.rs:207`), not inside the per-job path.

### 3.4 Worklog

**Who writes it: the framework, at session end. Never the agent.**
The organism's own contract is explicit — `agent_fabric.py:142-147`: "Objective
temporal log (framework-appended, AF-AD-007) — must never carry board-routing
fields."

**The session-end hook is `TurnEndBus::emit`, called at
`crates/operant-core/src/agent/run.rs:1271-1273`.**

```rust
if let Some(ref bus) = self.turn_end_bus {
    bus.emit(&session_id, iteration, total_tool_calls, &result.content);
}
```

**Why this one and not `HookEvent::TurnEnd`.** There are two candidate
seams and only one is the right hook:

- `HookEvent::TurnEnd` (`crates/operant-core/src/gateway_pipeline.rs:81`,
  emitted at `run.rs:1200-1208`) carries a `HookContext` with only
  `session_id` + two `metadata` strings (`gateway_pipeline.rs:105-115`) — no
  tokens, no duration, no tool timings. It cannot populate the worklog.
- `TurnEnd` (`crates/operant-core/src/turn_end.rs:109-134`) carries
  `turn_id`, `session_id`, `iterations`, `tool_calls`, `tool_durations_ms`,
  `result_summary`, `result_truncated` — four of the fields we need, typed,
  with a documented synchronous-never-awaits contract
  (`turn_end.rs:19-24`).

**Coverage gap the implementer must close.** `run.rs:1271-1273` is inside the
`if tool_calls.is_empty()` early-return branch (the "no tool calls, we're
done" path at `run.rs:1163-1170`). **`HookEvent::TurnEnd` has exactly one
emit site in the whole crate** (grep: `run.rs:1206` only), and `TurnEndBus::emit`
likewise has exactly one production site (`run.rs:1272`; the three
`stream.rs` hits are `record_tool_duration`, which is what fills
`tool_durations_ms`). So on the
`tool_calls.is_empty()` path neither fires. A worklog subscriber therefore
**misses every zero-tool turn** — which is most digest and summary jobs.
Fix: move the `turn_end_bus.emit` call to a single site that both branches
reach, or add a second emit for the early-return path. Flagging this because
it is invisible until the worklog has mysteriously empty rows.

**Schema** — all 20 organism fields, not 15:

```sql
CREATE TABLE IF NOT EXISTS worklog (
    id                    TEXT PRIMARY KEY,  -- uuid v4
    ts                    INTEGER NOT NULL,  -- unix seconds
    ts_iso                TEXT NOT NULL,     -- RFC3339, for humans
    employee              TEXT NOT NULL,     -- emp-<hex>; 'unknown' if unbackfilled
    department            TEXT,              -- NULL in Wave 1 (no dept concept yet)
    job_id                TEXT,              -- cron job id, NULL for interactive turns
    iteration             INTEGER NOT NULL DEFAULT 0,
    workflow_kind         TEXT NOT NULL DEFAULT 'process',  -- gather|process|publish|monitor|remediate|synthesize|coordinate
    what_done             TEXT NOT NULL,     -- result_summary, truncated
    outcome               TEXT NOT NULL,     -- success|partial|failure|noop
    artifacts             TEXT NOT NULL DEFAULT '[]',  -- JSON array
    tokens_in             INTEGER NOT NULL DEFAULT 0,
    tokens_out            INTEGER NOT NULL DEFAULT 0,
    tool_calls            INTEGER NOT NULL DEFAULT 0,
    duration_s            REAL NOT NULL DEFAULT 0.0,
    next_intent           TEXT NOT NULL DEFAULT '',
    blockers              TEXT NOT NULL DEFAULT '[]',  -- JSON array
    improvement_proposal  TEXT,              -- NULL, the kaizen field
    correlation_id        TEXT,              -- framework-only FK to notices
    notice_ids            TEXT NOT NULL DEFAULT '[]',  -- framework-only, JSON array
    session_id            TEXT NOT NULL      -- the operant session that produced this
);

CREATE INDEX IF NOT EXISTS idx_worklog_employee ON worklog(employee, ts);
CREATE INDEX IF NOT EXISTS idx_worklog_session  ON worklog(session_id);
CREATE INDEX IF NOT EXISTS idx_worklog_dept     ON worklog(department, ts);
```

The last three are framework-populated and carry **no board-routing fields**
(`to` / `ack_required`), per `agent_fabric.py:144-146`. That constraint is
preserved: there is no `to` column.

`ts_iso` is the one column with no organism counterpart. It is a read-side
convenience so `operant org` can render a timestamp without a `datetime`
conversion on every row; `ts` remains the stored truth. The organism's
empirical record set agrees on the rest: `docs/ORGANISM-ARTIFACT-REFERENCE.md`
§4.4(a) enumerates 19 keys over 1,127 live worklog records, and confirms the
3 framework-only fields are populated on just **327 of 1,127 rows (29%)** —
consistent with being framework-written. That 29% is a useful cross-check:
a *non*-framework writer would populate them on 100%, so a spike in that
column's fill rate is a signal that an agent started writing its own worklog.

**Field provenance — what actually populates what.** Two of the plan's four
named sources are wrong; here is the verified map.

| Field | Source | Verified |
|---|---|---|
| `ts` / `ts_iso` | subscriber's own `SystemTime::now()` at receive | no operant source needed |
| `session_id` | `TurnEnd.session_id` | `turn_end.rs:114` |
| `iteration` | `TurnEnd.iterations` | `turn_end.rs:117` |
| `tool_calls` | `TurnEnd.tool_calls` | `turn_end.rs:123`. **Caveat, quoted from `turn_end.rs:118-122`:** it counts *requests*, and "can differ" from `tool_durations_ms` when a call was rejected pre-flight (unparseable args, guardrail skip, unknown tool, user-denied approval). The organism's worklog means *executed* calls. Record the request count and say so. |
| `duration_s` | `turn_start.elapsed()` (`run.rs:265` `let turn_start = std::time::Instant::now()`), consumed at `run.rs:1294` where the existing `ObserverEvent::AgentEnd` already carries `duration: turn_start.elapsed()` | `observer.rs:35-41`. **This is the real duration source, not `observability/` generally.** |
| `tokens_in` / `tokens_out` | `AgentEvent::Usage { input_tokens, output_tokens, .. }` emitted by `emit_usage_and_cost` at `compress.rs:217-226` from `usage.prompt_tokens` / `usage.completion_tokens` | `compress.rs:222-226`. **This replaces the plan's `agent/cost.rs`.** That file is `crates/operant-runtime/src/agent/cost.rs` (different crate) and exposes `TurnUsage` / `snapshot_turn_usage` (`cost.rs:15,44`); the *core* agent has no such file. Either accumulate `AgentEvent::Usage` in the subscriber or read the `sessions` table. |
| `what_done` | `TurnEnd.result_summary` | `turn_end.rs:130`, already truncated to `RESULT_SUMMARY_LIMIT = 512` bytes on a char boundary (`turn_end.rs:90`); `result_truncated` at `turn_end.rs:133` says which. |
| `outcome` | derived: `success` if the turn emitted `AgentEvent::Done`; `failure` if `run()` returned `Err` or the classifier fired; `partial` if `last_status` was non-clean; `noop` if `result_summary` is empty. The `error_classifier` supplies the *cause*, not the outcome: `ClassifiedError { reason, status_code, message, retryable, should_fallback, should_compress, should_rotate_credential }` (`error_classifier.rs:123-137`), produced by `classify_api_error` (`error_classifier.rs:357`). | verified; note the classifier has **no** `Outcome`-shaped field, so the mapping above is new work |
| `workflow_kind` | **no operant source.** Default `"process"`. Needs a `[org]` config or a job-level field; not derivable from anything today. | gap, stated as a gap |
| `next_intent` | **no operant source.** `grep -rn next_intent crates/` → **zero hits.** `learning_graph.rs` exists (`build_learning_graph` at `:515`, `NodeKind` at `:71`, `GraphEdge` at `:82`) but has no intent field. Default `''`. This is a **Wave 4** consumer. | plan claim falsified |
| `improvement_proposal` | **framework, not data.** Default `NULL`. See below. | |
| `blockers` | `ClassifiedError` + `HookEvent::Error { error: String }` (`mod.rs:222`) | derived |
| `artifacts` | **no operant source** in the turn path. Default `[]`. (Operant does record trajectory when `record_trajectories` is on, `run.rs:1172-1180`, but that is a different sink.) | gap |
| `correlation_id` / `notice_ids` | framework only; default `NULL` / `[]` in Wave 1 | per `agent_fabric.py:165-166` |
| `job_id` | `None` for interactive turns; the cron job id when the session came from `run_agent_job` (`scheduler.rs:280-395`) | |
| `department` | `None` in Wave 1 | |
| `employee` | `'unknown'` when the session has no backfilled employee | the honest value; do not invent one |

**On `improvement_proposal` — the one field with no data source at all.**
It is the kaizen field and the plan calls it "the mechanism that turns a run's
friction into the next run's change." Operant has no mechanism. Writing it
requires an LLM call at session end, which means:

- it is **optional** and defaults `NULL` in Wave 1;
- when it is produced, it must be produced by a **framework** task, never by
  the agent in its own turn (the agent cannot be trusted to grade itself, and
  `agent_fabric.py:144` makes framework authorship the invariant);
- it is a **Wave 4 DEPT-loop input** (plan §4 Wave 4 item 5), not a Wave 1
  deliverable. The plan lists it inside Wave 1's "15 fields" without noting it
  has no producer. Shipping the column with a `NULL` default and an honest
  doc line is the right Wave 1 scope.

### 3.5 `--reason` mandate

**The exact clap pattern.** A required, long-only string on every mutating
subcommand, declared in the enum variant so clap enforces it before any I/O:

```rust
    /// Block a task
    Block {
        /// Task ID to block
        id: String,
        /// Why this mutation is being made (required; recorded on the task event)
        #[arg(long, value_name = "REASON")]
        reason: String,
    },
```

A bare `String` field in a clap derive variant is **already required** — no
`required = true` needed. The value is then threaded into the same store as
the mutation, so the reason and the effect are one transaction.

**Every mutating subcommand in the org surface.** `Org` does not exist yet
(`grep -n "Org\b" crates/operant-cli/src/main.rs` → no hits; the `Commands`
enum at `main.rs:190-540` has no `Org` variant), so this is the new surface:

| Command | Mutates | `--reason` |
|---|---|---|
| `org employee create` | `employees` insert | **required** |
| `org employee update` | `employees` update | **required** |
| `org employee retire` | `employees.status` | **required** |
| `org sync` | bulk backfill of `employees` + `employee_cron_jobs` | **required** |
| `org notice post` | `notices` insert | **required** |
| `org notice ack` | `notices.acked_by` / `acked_at` | **required** |
| `org notice pin` / `unpin` | `notices.pinned` | **required** |
| `org worklog append` | `worklog` insert | **required** |
| `org import <path>` | bulk import from a foreign tree | **required** |
| `org check` | — (read-only) | none |
| `org list` / `org route` / `org show` / `org notice list` / `org notice inbox` / `org worklog list` | — (read-only) | none |

**Convergence with the existing kanban `reason`: YES, converge — and note they
already disagree.** The two fields are:
- `crates/operant-cli/src/cmd_kanban.rs:71` — `Block { id, reason: Option<String> }`,
  declared with `#[arg(long)]` at `:70`. **Optional.** It is then defaulted
  downstream at `:508-511`:
  ```rust
  reason: Option<&str>,
  ...
  db.block_task(id, reason.unwrap_or("Blocked via CLI"), None)
  ```
  So a `block` with no `--reason` writes the literal string
  `"Blocked via CLI"` — a reason-shaped value that carries no causal
  information. **This is precisely AD-032's failure mode, already present in
  operant.**
- `crates/operant-cli/src/cmd_kanban.rs:115` — `Unblock { id, reason: String }`.
  **Already required.** A precedent that works.

**Decision: converge all four.** `Block.reason` becomes `String` (required),
and the same `#[arg(long, value_name = "REASON")]` attribute is applied to the
other mutating kanban subcommands that lack it (`Create` at `:50-59`,
`Comment` at `:73-79`, `Link`/`Unlink` at `:81-91`, `Assign` at `:98-105`,
`Archive` at `:117-122`) — a breaking CLI change, so it must land under a
version bump and be called out in `CHANGELOG.md`.

`Unblock` is the template: it is already required, and its handler at
`cmd_kanban.rs:699-701` shows the shape —

```rust
async fn cmd_unblock(config: &AppConfig, board_slug: &str, id: &str, reason: &str) -> Result<()> {
    ...
    db.add_comment(id, "cli", &format!("Unblocked: {}", reason))
```

`block_task` already takes `reason: &str` as a required positional
(`kanban/db.rs:487-492`), so the storage side needs no change. The `task_events`
table (`kanban/db.rs:215-223`, columns `kind` + `payload`) is where the reason
belongs; today the reason is smuggled into a comment string with a
`"Unblocked: "` prefix, which is stringly-typed. Write it to `task_events`.

#### 3.5.1 A validated reason that is not persisted is not a reason (iter-518)

Requiring `--reason` at the clap layer proves the *operator* supplied one. It
proves nothing about what landed in the row. The first implementation of
`org sync` passed every check and still violated the mandate: the CLI
validated and echoed the operator's reason, while
`EmployeeDb::employee_from_cron_job` stamped a hardcoded `BACKFILL_REASON`
into every employee row. The audit trail named a party that did not make the
write.

Two rules follow, and both are now enforced rather than documented:

1. **The validated value is the stored value.** `require_reason` decides
   emptiness on a trimmed string but returns the *untrimmed* original, so a
   handler that validates `reason` and separately writes `reason` persists the
   operator's shell padding. `validated_reason` binds the trimmed value so
   "what was checked" and "what was written" are the same string by
   construction.
2. **The reason is threaded to the store, not summarised.** `reason` is now a
   parameter of `employee_from_cron_job` and `backfill_from_cron_jobs`,
   stored verbatim. A blank reason is rejected at the DB boundary too:
   `''` satisfies the `NOT NULL` constraint while recording nothing, so a
   library caller that bypassed clap must not be able to write an empty audit
   trail. `BACKFILL_REASON` survives only as the fallback for callers with no
   operator in the loop.

#### 3.5.2 One schema, one owner (iter-517, extended iter-518)

The org tables live in `operant_kanban.db`, and **the DDL is owned by
`operant-core/src/org/`**. An earlier revision carried a second, locally
transcribed copy in `cmd_org.rs`; the two had already drifted (the CLI's
`worklog` had 14 columns against core's organism-aligned 20), and because
both used `CREATE TABLE IF NOT EXISTS` whichever initialised first would make
the other fail unpredictably. iter-517 removed the CLI copy and routed the
surface through the core types.

iter-518 closed the same class of leak one layer over: the CLI still
hand-wrote its own `INSERT INTO notices` and ack transaction even though
`NoticeBoard` owns that schema, and the copies had already diverged — the post
duplicated `--correlation-id` into both `correlation_id` and `thread_id`, and
the ack was **not idempotent**, so three acks by one employee wrote three
ack rows and a duplicated `acked_by` entry. Both now call `NoticeBoard::post`
and `NoticeBoard::ack`.

The general rule, since it is the one that keeps recurring: **a store's schema
is defined once, next to its types, and every writer — CLI included — goes
through the typed API.** A hand-written `INSERT` against a table another
module owns is a competing definition waiting to drift.

---

## 4. What I could not verify

Stated plainly, because a wrong citation costs more than an admitted gap.

1. **`docs/ORGANISM-ARTIFACT-REFERENCE.md` landed mid-review** (16:25, while
   this file was being written at 16:29). Every claim below was checked
   against it. It **agrees** with this document on all three points that
   matter: 133 employees (not 117, its `:1021`), `WorklogEntry` at
   `agent_fabric.py:141-167` with 3 framework-only fields (its `:1366-1368`),
   and the 65s→0.4s claim marked **UNVERIFIED** (its `:1633`). One
   correction to my own wording: the *live record* field union is **19** keys,
   because the real data omits `ts_iso`, which I added. The dataclass itself
   is 20. See §3.4.
2. **The 65s → 0.4s retention-GC perf claim** (`OUTLINE.md:341-342`). The
   `max_weekly_summaries_per_tick=4` parameter at `axe_lib.py:753-758`
   corroborates the shape but not the numbers. The numbers appear in no file I
   read. Do not cite them.
3. **Whether `operant org` should read the organism tree in place.** I
   recommended against it (§1.2) on the "no second path convention" rule. If
   the owner wants in-place reads, the correct form is an `operant org import
   <path>` command (§3.5) — a one-shot import, not a live path resolution. That
   is a product decision, not a code one.
4. **Exact `operant-runtime` → CLI daemon reachability.** I established that
   `operant-runtime/src/daemon/mod.rs:266` is the only caller of
   `cron::scheduler::run` and that no CLI file calls into `operant_runtime::daemon`
   (only two comments mention it). I did **not** trace every binary target in
   `operant-cli/Cargo.toml` for an indirect daemon spawn. Treat "the runtime
   loop is not the live loop" as strong-but-not-airtight; the backfill in §3.1
   is correct either way, because the runtime `CronJob` has strictly fewer
   fields and the *same* `id`/`name`/`skills`-or-`allowed_tools` shape.
5. **`operant status --json` / `architecture dump --live` observability
   claims** for the Wave 4 gate (`OUTLINE.md:441-447`). Not checked — out of
   Wave 1 scope.
6. **Employee count.** The plan says ~117 cron-employees
   (`OUTLINE.md:11`); I measured **133** records in `employees.json` and **102**
   jobs in `jobs.json` (100 with an `employee`, 96 with non-empty `skills`).
   These are different denominators (roster vs jobs) and the plan does not
   reconcile them. The gap between 133 and 100 is the orphan-drift that the
   organism's own `axe org check` smoke check exists to catch.
