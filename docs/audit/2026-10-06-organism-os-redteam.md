# Organism OS — Red-Team Audit (2026-10-06)

Auditor: coding agent, session against `origin/main` = `8ecf1cc8` (iter-638).
Method: built the real binary from a clean `origin/main` worktree, deployed it,
restarted the live gateway, exercised the command surface with real exit codes,
queried the live SQLite stores, and cross-read the org layer for redundancy.
Every claim below is either quoted output from that run or a `file:line` anchor
re-verified in this session.

---

## 0. Verdict

**The organism OS is NOT fully and effectively complete.** The genome code is
substantially built (18,913 lines in `crates/operant-core/src/org/`, 23 files,
12,262 production / 6,632 test, **zero** `unwrap`/`expect` in production) and
the hygiene is genuinely good. But the audit found that **the operator surface
and the running daemon point at two different SQLite files**, so the organism
the operator can inspect and govern is not the organism the gateway runs. That
is a correctness defect in the central claim of the whole layer, and it is not
visible from any unit test, because every test constructs its own store.

There is also a class of code that is landed, documented as protective, and
**unreachable from any production path**. In an organism whose premise is
"unresolvable authority fails closed", a fail-closed gate that can never be
reached is worse than no gate — it is a false assurance in a security-relevant
module.

Nothing below is a claim about intent. These are observed states.

---

## 1. Deployment state (was materially wrong; now corrected)

| Fact | Evidence |
|---|---|
| `~/.local/bin/operant` was built 00:48, `origin/main` landed 05:05 | `ls --time-style=full-iso` vs `git log -1 %cI` |
| `/usr/local/bin/operant` was built **2026-10-05 09:24** — a *different, older* build (35 MB vs 55 MB), distinct md5 | `md5sum` of both paths |
| The running gateway executed the 00:48 binary | `md5sum /proc/$(systemctl --user show -p MainPID --value operant-gateway)/exe` |
| The shared working tree carries **62 dirty files**, incl. 54 untracked `tui/jcode_app/*` peer-WIP files | `git status --porcelain` |

**A release build from the shared tree is NOT `origin/main`.** It silently
embeds a peer's in-flight jcode refactor (warnings appeared in
`jcode_app/info_widget/mod.rs:56,80,81,89` — a directory that is *untracked*).
The dirty build also produced **8** warnings vs **1** for the clean build, which
is itself a signal that the dirty tree is mid-refactor.

Corrected: built in a clean `git worktree` at `8ecf1cc8`
(+`git submodule update --init --recursive`), deployed `21eb19ee…` to **both**
install paths via rename-over, and restarted the daemon. Post-restart:

```
md5sum /proc/2048479/exe -> 21eb19ee571d93944ee1b1be5e454594   (matches both install paths)
journalctl: "Gateway started with 1 platform(s)."   no panic
```

**Recommendation:** the deploy step in `AGENTS.md` says "build release", not
"build release from a clean checkout of `origin/main`". Under fleet conditions
those are different binaries. The deploy procedure needs the worktree hop.

---

## 2. HEADLINE — the organism registry is split across two databases

Reproduced **on the deployed `origin/main` binary**, not on a dirty tree.

```
$ operant doctor   →  ✓ employee registry: 11 seat(s)
$ operant org check→    employees: 0 (0 active)
$ operant org list →    (empty)
```

Root cause, anchored:

| Writer | Path it uses | Anchor |
|---|---|---|
| Gateway boot (seeds the 9-seat cast) | `operant_home().join("database.db")` | `crates/operant-cli/src/gateway_runner.rs:1696` |
| Gateway cron backfill | `operant_home().join("database.db")` | `crates/operant-cli/src/cmd_cron.rs:313` |
| Doctor genome census | `operant_home().join("database.db")` | `crates/operant-core/src/doctor.rs:100` |
| **Entire `operant org` CLI** | `database_path.parent()/operant_kanban.db` | `cmd_org.rs:659` → `org_db_path()`, `crates/operant-core/src/org/employee_db.rs:64-69` |

Live state confirms the split:

```
database.db          → employees=11, seat_policies=1, 7 org tables present
operant_kanban.db    → employees=0, worklog=1, notices=0, no seat_policies table
```

**Consequence.** `operant org employee create`, `org grant give`,
`org notice post`, `org decision accept` all write to a file the gateway never
opens. The operator believes they have seated, granted, or notified; the running
organism sees none of it. Every authority grant issued from the CLI is inert.

There is a second-order hazard: `WriteBarrier::for_app`
(`crates/operant-core/src/org/write_barrier.rs:588-595`) derives its employee
store via `org_db_path` while the gateway passes `database.db` as the base — so
the barrier's employee view and the CLI's employee view disagree about who
exists, on the same box.

The fix direction already exists in-tree: `cmd_cron::cron_db_path`
(`cmd_cron.rs:99`) is the "one canonical path helper" precedent. Give the org
layer the same authority and delete the two ad-hoc `operant_home().join(...)`
sites. Note `cmd_org.rs:1836` (`org_reason_db_path_is_the_kanban_sibling`)
asserts the current, wrong target and must change with it.

---

## 3. Implementation gaps — landed, documented as protective, unreachable

Both verified by grep in this session: **zero** non-test references.

### 3.1 `IdentityGate` — 395 lines, no production caller (HIGH)

`crates/operant-core/src/org/identity_gate.rs:181`. The scheduler holds
`org_gate: Option<Arc<IdentityGate>>` (`cronjobs/scheduler.rs:39`), mounted
only by `with_org_gate` (`:74`). The **only** callers of `with_org_gate` in the
entire tree are `crates/operant-core/tests/org_identity_gate.rs:159,183,219,249`
— tests. The gateway boot installs `with_write_barrier`
(`gateway_runner.rs:3581-3594`) and never `with_org_gate`.

The module doc (`identity_gate.rs:36-38`) states the gate is unreachable by
design ("dark-mergeable", `docs/WAVE1-DECISIONS.md` §3.2). That is a legitimate
dark-merge rationale — but it was never *finished*, and a reader of the doc
would reasonably believe the organism is gated when it is not. Either wire it at
boot or record it as explicitly-not-yet-active. Today it is a 395-line
fail-closed mechanism that cannot fail closed.

### 3.2 `authority.rs` containment rules govern nothing (HIGH)

`crates/operant-core/src/org/authority.rs` is 1,611 lines — the second-largest
org file — and exports `can_post_to` (`:353`) and `can_accept_decision`
(`:424`). Grep shows these appear **only** in `authority.rs` itself, its module
doc (`:55`), and its own `#[cfg(test)]` tests. The two production write paths
never call them:

- `cmd_org.rs:1362-1410` — `org notice post` → `NoticeBoard::post`, no scope check.
- `cmd_org.rs:933-941` — `org decision accept` → `db.accept`, no `can_accept_decision`.

So the containment rules the genome was designed around are **pure predicates
with no enforcement point**. Combined with §2 (writes land in the wrong DB),
the authority layer is currently non-operative on both axes.

### 3.3 Wave 4 budget enforcement cannot fire (HIGH)

`seat_budgets` has **0 rows** in both `database.db` and the kanban sibling. No
cap is configured, so the iter-635 mid-flight check (`agent/run.rs:558-560`,
comparing `used_at_turn_start + turn_spend_tokens` against `cap_tokens`) is
correct code with nothing to compare against. There is no CLI command to
provision a seat budget — `operant org` has `employee/notice/worklog/department/
decision/grant/dm/sync/import/check/list` and **no budget, no cast, no
topology**. An operator cannot set a cap even by hand.

### 3.4 Five independent token accumulators, no canonical usage event

Mechanical count of `total_tokens +=` / `update_tokens` sites:

| # | Sink | Anchor | Counts |
|---|---|---|---|
| 1 | Gateway session store | `gateway_session.rs:1306-1325` | `input + output` |
| 2 | Seat budget turn counter | `agent/run.rs:384,558` | per-turn `total_tokens` |
| 3 | Cost tracker | `operant-config/src/cost/tracker.rs:250` | API `total_tokens` |
| 4 | Insights | `agent/insights.rs:276` | `input + output` |
| 5 | Memory session | `memory.rs:151` | caller-supplied |

Five sinks, two different formulas, no single usage event any of them observes.
The `AgentEvent::Usage` event already carries the data (emitted from
`agent/compress.rs:450` on both the streaming and non-streaming paths), so a
canonical fold is available without touching the agent loop.

**Live check:** all 17 rows in `gateway_sessions` have `total_tokens = 0` and
`estimated_cost_usd = 0.0`. The iter-632 drain writes only on the *gateway*
turn path — a CLI `operant run` does not touch that store — so these zeros are
expected for CLI traffic, but they also mean **no budget has ever been enforced
in production**, because nothing has ever been metered there.

### 3.5 `org cast` does not exist as a command (MEDIUM)

The nine-seat cast is a core concept (`org/cast.rs`, 699 lines) and doctor
reports on it, but `operant org cast` is `error: unrecognized subcommand`. The
topology the organism boots with is not inspectable from the CLI.

### 3.6 Pre-existing dead match arm in the released binary (LOW)

Build warning on clean `origin/main`:
`crates/operant-cli/src/gateway_commands.rs:999` — `"session" =>` is
unreachable; an earlier arm at `:809` already matches all values
(`#[warn(unreachable_patterns)]`, 1 warning in the clean build). Shipping a
dead arm in a released binary.

---

## 4. Redundancy / fragmentation / over-engineering

### What is genuinely good — do not touch

- **One authority consult seam.** `SeatAuthority` + `SeatApprover`
  (`org/seat_authority.rs`) — single implementation, no rival. Verified.
- **One doctor engine** (iter-614), consumed by both CLI and gateway.
- **Zero** `unwrap`/`expect` in 12,262 production lines of org code.
- **Zero** TODO/FIXME/XXX/HACK in the org layer.
- `org import` correctly refuses loudly rather than faking a write
  (`cmd_org.rs:1621-1629`) — good discipline, not a stub to paper over.

### Fragmentation

1. **Three DB-path derivations for the same organism** (§2). Highest-value fix.
2. **Five token accumulators** (§3.4).
3. **`WriteBarrier` re-derives the employee path** independently of the gateway
   and the CLI — a third opinion on where employees live.
4. **Doctor vs `org check` disagree** on the same census — two implementations
   of "how many seats exist", one reading `database.db`, one the kanban sibling.

### Over-engineering (honest read)

- `authority.rs` at **1,611 lines** is the largest org file, and per §3.2 its
  two most prominent exports are never called. That is not "too clever" — it is
  **too large relative to its enforcement reach**. The consolidation is not to
  delete it; it is to wire it and then re-measure.
- `write_barrier.rs` (1,685 lines), `dm_thread.rs` (1,502), `hierarchy.rs`
  (1,257), `notice_db.rs` (1,170), `department_db.rs` (1,148) — the org layer is
  **18,913 lines** for 11 seats and 1 policy row in production. The ratio is
  the finding: the genome is far ahead of the organism it governs. That is
  acceptable *if* it is being filled in, and not acceptable *if* the code is
  meant to be live. §2 and §3.1/§3.2 say the latter.

**No speculative-config or one-implementation-interface findings.** The design
preferences in `AGENTS.md` (Kokoro, memory-wire, sourcehound) are untouched and
correctly so.

---

## 5. Live-loop verification — what actually ran

| Probe | Result |
|---|---|
| Clean release build from `8ecf1cc8` | exit 0, 8m48s, 1 warning |
| Deploy to both install paths + daemon restart | `21eb19ee…`, active, PID 2048479 |
| Gateway boot log | `Gateway started with 1 platform(s).` — no panic, cast seeded |
| 43 top-level commands `--help` | **43/43 resolve, 0 failures** |
| Real execution probes (13 commands) | exit 0 for `org check`, `org list`, `cron status`, `sessions list`, `memory stats`, `hooks list`, `insights sessions`, `kanban list`, `webhook list`, `skills list`, `plugins list`, `mcp list`, `cookies list`, `trajectory list`, `service status` |
| **Live agentic turn** | `operant run --query "Reply with exactly: LOOP-OK" --max-iterations 1` → `LOOP-OK`, exit 0 |
| `operant doctor` | genome checks fire, cast census reports 11 seats |

**Non-zero exits investigated, all correct behavior — not defects:**
`insights`/`context`/`hardware` with a bare name (clap needs a subcommand, exit 2);
`context status` exit 1 = correct config gating
(`context engine is 'compact' (expected "lcm")`); `architecture dump` exit 1 =
correct missing-`architecture.toml` error.

*Method note:* my first sweep reported `exit=0` for `operant org cast` even
though clap exits 2 — `$?` was capturing `head`, not `operant`. Every exit code
in the table above was re-measured with the redirect-to-file form.

### Live failures found

- **`cron_c7c03b17 wave5-test` — `Last Status: error`.**
  `Agent run failed: The model provider's stream died mid-response and did not
  recover: Network error: error decoding response body`. Schedule `0 0 */6 * * *`.
  This is the Wave-5 governance test job failing in production.
- **Six test/verification cron jobs are still scheduled on the real cron db**,
  including `zz-smoketest-temp-20261005` (Active, fires 2026-11-01) and four
  `tooltest-*` jobs. These are smoke-test residue, not workloads.
- Doctor: `1 cron automaton seat(s) run ungoverned (no policy row)` — expected
  default, correctly reported.

---

## 6. Feature gaps — what an organism OS needs that this lacks

1. **No `operant org cast`** — the topology the daemon boots with is not
   inspectable.
2. **No budget provisioning command** — `seat_budgets` cannot be populated by
   any CLI path, so iter-635's enforcement is unreachable in practice.
3. **No org-wide spend report** — with five token sinks and no canonical usage
   event, "what did the org spend this week" has no answer surface.
4. **No per-seat audit query** — decisions/grants exist as tables, but there is
   no `org audit <employee>` that answers "what did this seat decide, grant, and
   spend". For an organism whose stated post-mortem was *"1,311 logged
   decisions read by nobody"*, this is the exact missing surface.
5. **No seat lifecycle beyond create/update/retire** — no reassignment,
   department transfer with history, or seat-to-human binding lifecycle.
6. **`org import` is a deliberate refusal**, not an implementation gap — §4.3 is
   an open product decision for the owner. Recording it so it is not mistaken
   for an oversight.

---

## 7. Recommended order of work

1. **Fix the DB-path split** (§2). Everything else in this document is a
   smaller problem, and until it is fixed the org CLI cannot govern the org.
   Delete the two ad-hoc `operant_home().join("database.db")` sites in favour of
   the `cmd_cron::cron_db_path` precedent; update `cmd_org.rs:1836`.
2. **Wire or explicitly retire `IdentityGate` and the two `authority.rs`
   predicates** (§3.1, §3.2). A protective module that cannot protect is a
   liability.
3. **Add budget provisioning to the CLI** (§3.3) so iter-635 is reachable.
4. **Add `org cast` and `org audit`** (§6.1, §6.4) — the two read-only surfaces
   that make the organism legible.
5. **Land the CHANGELOG + `AGENTS.md` deploy-procedure correction** (deploy from
   a clean `origin/main` worktree, not the shared tree).
6. **Clean the cron db** of the six test/verification jobs (§5).

## 8. Open questions for the owner

- Should the org layer live in `database.db` (where the daemon already is) or in
  a dedicated sibling? The code currently claims both. Consolidating onto
  `database.db` is the smaller diff and matches the daemon; a dedicated file is
  cleaner but requires a one-time migration of the 11 seeded seats.
- Should `IdentityGate` be wired (a behaviour change at cron boot) or removed?
  Wiring it can start blocking jobs that run today, which is a policy decision,
  not a refactor.

---

## 9. Addendum — iter-650/651 findings and the open Slice-2 decision

### Closed by iter-650 (verified live)
The DB-path split is fixed and **proven**: after deploying a clean `origin/main`
build and restarting the daemon, `operant doctor` and `operant org check` both
report **9 seat(s)** (baseline: 11 vs 0). The nine seats are the cast —
`premiere`(org-lead), `chief-of-staff`, `compass`, `crew-chief`, `dispatcher`,
`dp-the-program`, `governor`, `hrmaster`, `identity-warden`. The old
`employees` table in `database.db` still holds the 11 stale rows from the split;
it is now unwritten by every process but not yet dropped (see §10).

### Closed by iter-651 (deploy-procedure defects)
Three real defects in the step-7 deploy block, each of which caused a bad
deploy this session: shared-tree builds are not `origin/main` under fleet
conditions; the `/usr/local/bin` `cp` ran unprivileged and failed EACCES
before its own `sudo mv`; and nothing checked that a failed build had not
silently left a stale artifact that still passes `--version`.

### NOT closed — Slice 2 needs an owner decision (security-relevant)

`iter-636` claimed "one canonical glob matcher". **That claim is false on tip.**
Two runtime matchers remain, and both gate tool *permissions*:

| Site | Dialect | Gates | Callers |
|---|---|---|---|
| `crates/operant-runtime/src/agent/loop_support.rs:20` | `*` only (first-star split) | MCP tool-group exposure per turn | `filter_tool_specs_for_turn:64` |
| `crates/operant-runtime/src/hooks/builtin/webhook_audit.rs:149` | `*` only (multi-segment split) | tool allowlist | `:216` |
| canonical `crates/operant-core/src/context/lcm.rs:49` | `*`, `?`, `[a-z]` | (already used by core allowlists) | agent/mod.rs:776, tools/kernel/mod.rs:188 |

**The consolidation is a permission WIDENING, not a refactor.** Today a
pattern like `mcp_[ab]*` matches *nothing* in the MCP filter (the `[` is
literal and no star-split applies); under `lcm::glob_match` it matches
`mcp_a…`/`mcp_b…`. Patterns that currently *deny by accident* would start
allowing. Same for `?`: `mcp_?avigate` changes meaning silently.

**Owner decision required — three options:**
- **A. Delegate to `lcm::glob_match` (accept the widening).** Smallest diff,
  one matcher tree-wide. Changes MCP/webhook tool-group semantics. Requires
  a migration note for anyone who wrote `[`-bearing patterns expecting literal.
- **B. Keep the `*`-only dialect, consolidate onto one `*`-only helper.**
  Preserves every existing permission exactly; still removes the duplication
  that made `iter-636`'s claim false. Loses `?`/`[a-z]` for MCP patterns.
- **C. Leave both, fix the doc claim.** Smallest risk; the duplication and the
  dialect split both persist, but nothing changes for users.

**Recommendation: B.** It removes the real problem (two divergent
implementations of the same concept, and a false claim in the changelog)
without silently widening any permission. Option A is defensible only if you
want full fnmatch semantics on MCP patterns and accept the migration.

`filter_by_allowed_tools` (`loop_support.rs:85`) is deliberately excluded: it
is exact-match by design and widening it is a separate behavior change.
