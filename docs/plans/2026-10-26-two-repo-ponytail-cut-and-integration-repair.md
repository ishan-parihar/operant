# two-repo ponytail cut + integration repair

Dated-frozen: published `2026-10-26`. A plan that changes is a new plan.

Goal: execute the ranked removals and repairs the 2026-10-25/26 two-repo
`/ponytail-audit` found, with every claim re-measured in the tree it names,
and every cut gated by the full suite on both repos.

## Scope boundary (law 10)

This plan is stack **A**: the holon kernel (`/home/ishanp/Documents/github/my-projects/holon`)
and the operant runtime (`/home/ishanp/Documents/github/my-projects/agentic-harness/operant`).
Stack **B** — the Chair's venture runtime, the organism pools, the VPS — shares
vocabulary and nothing else. A `holon doctor` error about a career-pipeline pool
or an outreach lane is B's to close, and is not evidence about A.

## Audit result (19 findings, 4 withdrawn by measurement)

The audit's credibility is its self-corrections; the same discipline applies to
execution. Four findings were withdrawn when measured rather than asserted:

| finding | first claim | measured result |
|---|---|---|
| skills export staleness | export auditor is weak | the guard exists and is render-aware; **0 stale** |
| `seat_*` / `kanban` / `cronjobs` | dead surface, cuttable | all live — the fence, 19 personal tasks, 3 enabled jobs |
| vendor drift test | does not work | works: fails against the live kernel when `HOLON_REPO` is set |
| worklog orphan | ~60% of the file | ~439 lines of a 1099-line file |

The decisive experiment, run live:
`HOLON_REPO=<holon checkout> cargo test -p operant-os --test native_tools vendored_tool_list_matches_the_kernel_registry`
**fails** — the vendor carries 23 tools where the kernel carries 58. Of the 35
missing, 34 are agent-reachable; only `chair_channels` is not, and should not be.

## Phase 1 — Tier 0 removals (zero-risk, complete)

### T0.1 `worklog.rs` (executed)

`crates/operant-core/src/org/worklog.rs` **1099 → 418 lines (-681, -62%)**.
Removed `TurnObservation`, `WorklogEntry`, `WorklogRecord` — no producers or
consumers outside the file's own tests — their 3 test helpers, an orphaned
`to_i64` and the orphaned imports, and rewrote the module docstring to describe
what remains.

Two precision errors were caught before the cut landed, by asserting on the
content rather than trusting the boundary:

1. `UsageAccumulator` is mentioned *inside* a dead struct's docstring — the
   mention was not the definition, and the definition is live.
2. `derive_employee_id` at L819-840 is **live** — `cronjobs/scheduler.rs`,
   `cmd_cron.rs` and `cron_session_isolation.rs` all call it — so the cut
   boundary moved from L841 to L817.

The old docstring cited `tests/org_worklog_framework_only.rs`, which was
deleted with the G+1 cast retirement (`docs/audit/2026-10-16-operant-audit/02-test-gate-repair.md`).
`git log -S` on that name is empty: the citation was already a ghost, and the
replacement docstring drops it.

**Gate:** full `operant-core` suite — 2574 passed, 0 failed across 33 test
binaries; `cargo clippy -p operant-core --all-targets` 0 warnings.

A note on suite flakes: a workspace run reported 9 failures in
`gateway::*`, `mcp::tests::*` and `model_catalog::tests::*` — every one a
`ConnectionRefused` on a loopback port. They pass in isolation **with and
without the cut**, and the full lib suite is green on re-run. Loopback port
contention between crates, not a regression.

### T0.2 `pk-sidecar/` (executed)

`pk-sidecar/main.py` (74 lines) + `scripts/check-pk-sidecar.sh` (91 lines) are
orphaned: no test, no CI gate, no reader. The only text reference is a
docstring in `config.rs`.

**Three corrections to the plan's own assumption, found by measuring:**

1. `PkSettings` **stays**. It is deliberately-kept dead config — RT-136 records
   that it is read by nothing so that older configs still parse and
   `unused_setting_warnings` reports the keys.
2. `config.rs`'s `sidecar_script` docstring names `pk-sidecar/main.py` as its
   default — **a citation to a file this repo no longer carries.** The field's
   real default is `None`, and the live kernel mechanism is
   `[tools.kernel]` driving `-m n.server` in `kernel-sidecar/`. This is the
   citations defect class: a pointer that resolves nowhere.
3. `scripts/check-pk-sidecar.sh` asserts `SCAFFOLD="$REPO_ROOT/pk-sidecar/main.py"`
   exists. It would **fail the moment the directory is removed**, so the gate
   goes with its subject.

### T0.3 repo-root droppings (measured, then executed)

21 untracked `*.sqlite` files, 3.0 MB, all mtime 2026-10-17 21:10 — one test
run's droppings, no open handles, already gitignored by `.gitignore:46`.

**Correction to the plan's assumption:** `~/.operant/config.toml` (5420 bytes,
mtime 2026-09-30) is **not** the file that warns. `operant config check`
reports the kernel reads `~/.operant/operant.toml`; `config.toml` is a dead
filename. Deleting it removes exactly **1 of the 3 warnings**. The other two
— `[memory] agentmemory_auto_spawn` (operant.toml L216) and `[socialization]`
(L285) — live in the **live** file, which needs an edit, not a deletion.

### T0.4 `holon/holon.toml.pre-isolation.bak` (measured)

Exactly **5 lines of diff** from the live `holon.toml`: the live file adds
`[isolation] agent_user` and `runner`. The boundary is **applied and live** —
the agent user exists, and `holon-broker`, `holon-notion` and `holon-tick.timer`
are all running against it. The `.bak` is a pre-isolation snapshot with no
remaining rollback value in the instance directory.

## Phase 2 — Tier 1 (owner-held)

`operant-harness` (7,878 lines, `[harness] enabled = false`) either becomes the
R-04 consumer or is cut. **Not executing without the Chair's verdict** — a cut
this size is not reversible by a commit.

## Phase 3 — integration repairs (I-1 .. I-6)

| id | repair | gate |
|---|---|---|
| I-1 | export `HOLON_REPO` in the two CI workflows that run `cargo test` | the vendored-tool guard fires in CI instead of only locally |
| I-2 | regenerate `vendor/holon-tools.json` + `vendor/schemas/*.json` | the live experiment above goes green; 7/7 drifted schemas (`guard.cmd` → `guard.check`) repaired |
| I-3 | write the schema-drift test `contract.rs` already promises | a drift fails a test, not a docstring |
| I-4 | re-run `adapters/operant/install.py` on the live instance | the live instance carries the repaired tool list; `doctor` clean |
| I-5 | supervisor decision: `operant org holon supervise` needs `--employee` or `[holon] supervise_employees` | 33 ready stages and 26 chair-owned approvals (39 h) stop waiting — **Chair-gated** |
| I-6 | conditional on the Tier 1 verdict | — |

## Phase 4 — what is explicitly not cut

Measured live, twice, and verified by name:

- `seat_policy` (431) — the fence every Holon body mounts
- `seat_policy_db` / `seat_budgets` / `seat_authority` (1,023) — initialized at
  gateway start
- `kanban` (2,003) — 19 live personal tasks
- `cronjobs` (3,709) — 3 enabled jobs
- `adapters/hermes/` in holon (108) — guarded by
  `test_hermes_driver_reaches_custom_endpoints_and_detects_stalls`
