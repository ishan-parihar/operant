# What stands after iter-1379 — the open ledger rows, and the order they close

Dated-frozen: published `2026-10-27`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section.

**Status:** dated-frozen WORKLIST. **It amends, and does not replace,**
`docs/plans/2026-10-27-remaining-implementation-gaps-ranked-execution-plan-for-the-.md`
(the peer's plan — still the record of what the 2026-10-25 audit ranked AT AUDIT
TIME; frozen history, cite it and do not edit it). Where the two disagree, this file
carries the newer verified state; the ranked plan keeps its original ranking.

**Authority:** `docs/audit/2026-06-19-BUGS.md`'s "What is OPEN right now" table.
**Measurements, not impressions:** every row below was re-verified against the
live tree and (where relevant) the deployed binary TODAY. Several rows moved
under the earlier plan, so a row's checkbox state was not treated as its state.

### The measured landscape (2026-10-27, after iter-1379)

`grep -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md` = **7 open checkbox rows**:
RT-135, RT-136, RT-179, RT-190, RT-195, RT-196, plus RT-179's peer-side work.
Two rows are NOT checkboxes (prose `### S-2` / `### D-5b` / `### D-6`), so a
checklist count misses them — the same trap the ledger itself documents.

Plus the non-checkbox openings that the plan must name:
- **S2** (prose row, "Duplicate tool rows mid-turn"): WIDE/emoji glyph class unproven.
- **D-5b / D-6**: accepted limits, by decision. No work.
- **S-1** (skills referential integrity): a dangling citation in the OPERATOR's
  skills pool, not this repo — verified this session, see Tier 2.2.

### What already landed (so this plan does not repeat it)

- **iter-1372** — Tier 0's three description lies (aft_search, process list,
  aft_checkpoint) closed with pin tests; RT-181's ladder-half pin added;
  RT-180's dead scenario unblocked.
- **iter-1373** — RT-190 ruled as option (c) and documented; RT-194's scope
  correction recorded.
- **iter-1379** (mine, `ac92db08`) — RT-179's first two functional-core twins
  (`/effort`, `/plan`) wired, deployed md5-verified.
- **iter-1379** (peer, `698b2e63`) — RT-194 SHIPPED as the native registration
  path plus the `is_deferrable` extension (850 new lines,
  `crates/operant-core/src/tools/sourcehound_composites.rs`). The residual is
  recorded in the ledger row: the 23 are deferred, not a curated always-on
  set, so discovery rides `tool_search`.

## Mechanism

Each item is ONE iteration: change → scoped test → crate battery → clippy gate
(seen=0/new=0) → commit with the fresh `origin/main` number → push →
deploy (code-adjacent only). Docs-only items skip the build.

**Ordering rule:** rows whose state is wrong get corrected BEFORE any code is
scheduled against them. A stale row that says "not implemented" invites the
next agent to re-implement shipped work — that is the RT-181 trap, and it cost
a full audit cycle.

## Gates / success criteria

(fixed before the run)

1. Every row scheduled here has its state RE-VERIFIED against the tree in the
   same iteration that changes it — no scheduling from a checkbox alone.
2. No peer-touched file is edited: `tools.rs`, `tools/builtin.rs`,
   `tools/tool_search.rs`, `tools/sourcehound_composites.rs`,
   `tools/sourcehound*.rs`, `feed.md`, `PAGE-INDEX.md`, `genome.toml`,
   `README.md`, `Cargo.lock`, `docs/library/`. `genome claim` before starting.
3. Every code change carries a regression test that fails without it, cited in
   the commit body, mutation-proven where the change is a single call site.
4. Live claims are verified on the DEPLOYED binary, never inherited from a
   source read.
5. Push then deploy; confirm `git log origin/main -1` shows the commit and the
   running PID's `/proc/PID/exe` md5-matches the deployed path.

## Cost

Small. Closing the ledger is documentation, tests, and one small feature
(RT-196's smoke pass). The only genuinely multi-iteration item is RT-179's
remaining functional core, and its mechanism is already built.

---

# Tier 1 — ledger corrections first (zero risk, prevents wasted work)

### 1.1 RT-196 (a) — the 22-tool smoke pass, and its misdiagnosis corrections

**Verified state:** the two description fixes (RT-195(a), RT-196(a)/(b)) LANDED
in iter-1372 with contract tests `aft_checkpoint_description_names_the_tracked_files_limit`
and `process_description_states_its_registry_scope_not_the_os_process_table`.
What remains open is the pass itself.

**Work:** a second smoke pass over the 22 tools the 2026-10-25 "Complete Report"
claims were all exercised. Side-effect tools need an explicit consent pass first —
they must not run destructive operations to prove they load.

**Accept:** every one of the 22 either exercised-and-recorded or named as
consent-gated with the reason. The row's count becomes a measured one.

### 1.2 S2 — the WIDE (emoji) glyph probe

**Verified state:** AMBIGUOUS was cleared live at iter-1139; WIDE is still
unprobed. The ledger's own caution (line ~1246) is the honest limit: the check
observes the TERMINAL's grid, not the app's buffer — it proves the operator sees
the right thing, not that the two agree cell-for-cell.

**Work:** one live wide-terminal probe with an emoji present, then CLOSE the row
or SPLIT it honestly into "the terminal renders it" and "the buffer agrees",
with whichever half survives. The ledger already records the AMBIGUOUS precedent
and what that provenance buys.

**Accept:** either a closed row naming the probe, or a split row whose each half
has its own evidence.

### 1.3 RT-195 residual — `SOURCEHOUND_GROUP_HINT` names a path that does not exist

**Verified state (line 6133):** still open. The hint names
`~/.config/sourcehound-mcp/settings.yml`; the live layout is
`~/.config/sourcehound/settings.yml` and the `-mcp` dir only appears under a
redirected `SOURCEHOUND_CONFIG_DIR`.

**Work:** a one-line hint correction plus a test asserting the hint names a path
that exists on this machine's default layout, or names the redirect explicitly.
The only operator-visible consequence is a wasted trip looking for a setting file.

**Accept:** the hint string is either the live path or carries the redirect
condition, and a test pins it.

### 1.4 RT-135 / RT-136 — confirmed non-work

**Verified state:** RT-135 is the owner-accepted `cron run <id>` stub (AGENTS.md
records the acceptance). RT-136 is a confirmed not-a-defect; `[pk]` and
`[socialization]` are labelled in-code and reported by
`unused_setting_warnings`.

**Work:** none. Both rows already carry their ruling. Recorded here so the next
plan does not schedule them again.

### 1.5 D-5b / D-6 — accepted limits

**Work:** none. Accepted by decision.

---

# Tier 2 — architecture decisions (an owner ruling, or a re-read of the live code)

### 2.1 RT-190 — memory bank topology

**Verified state:** RULED option (c), documented iter-1373. The ruling is
recorded in the ledger row; the bridge (b) stays a future tranche if
cross-surface recall turns out to matter; (a) is deliberately not first because
it trades the whole in-process advantage for a shared store.

**Work:** none this tranche. The operator's confusion cost was the actual harm,
and it is closed. If the operator rules the bridge, that is a new tranche with
its own plan.

### 2.2 S-1 — the dangling `emotional-memory.md` citation

**Verified state (this session, primary source):** the citation is real and
lives in the OPERATOR's skills pool, NOT this repo.
`~/.agents/skills/human-conversation/references/emotional-authenticity.md:11`
routes to `references/emotional-memory.md`, which exists nowhere. The skill's own
`_build/plan.md:228` names the missing leaf as a build artifact still to be
authored. So this is a plan-in-progress pointer inside a skill, not a repo defect.

**Decision, not a fix:** either author the capability in the pool (it is a real
capability the routing promises — cross-turn emotional continuity), or delete the
two routing lines. Repo-internal work is zero either way.

**Why it is not a code task here:** editing `~/.agents/skills` from this slice
would change a shared pool every other agent reads, for a WARN-grade
dangling-hint. The honest move is to record the finding and let the skill's owner
decide.

### 2.3 S-2 — the `~/.operant/skills` default pool carries OLD-MODEL copies

**Verified state:** OPEN, medium. The configured root (`[skills] root_dir`) is
`~/.operant/skills`, a separate store from `~/.agents/skills`; at least
`agent-interface` exists in both with the pool copy still pre-migration, so
`operant skills audit` reports stray nested `SKILL.md` files on every run.

**This is a decision, not a patch:** are the two stores one store or two? If
one, the fix is a sync/migration; if two, the fix is to stop auditing the pool
tree as if it were the migrated one. Drift here is the same shape as the
meta-skill-creator B1 divergence the ledger already records.

**Recommend:** decide "one store", because two divergent copies of one skill is
what produces the audit noise, and the noise is what trains operators to ignore
the audit. Confirm with the operator before migrating anything.

### 2.4 RT-194 residual — deferred composites ride `tool_search`

**Verified state:** SHIPPED by the peer (iter-1379). The residual is recorded:
the 23 are deferred, so discovery needs one `tool_search` call.

**Work:** none scheduled. If a future request wants the high-traffic composites
always-on, that is a curated-subset decision with the token budget as the input,
and it is a new plan.

---

# Tier 3 — multi-iteration: RT-179's remaining functional core

**Verified state:** the blocker (RT-175's mechanism) closed at iter-1298;
`run_session_scoped_command` is the seam, and iter-1379 proved it works for
`/effort` (an agent-cell setter) and `/plan` (a permission-receiver posture).
Rendering commands (`/theme`, `/vim`, `/mouse`) legitimately have no twin.

**What is left, in value order, each its own iteration:**

| # | Command | The seam it needs | Note |
|---|---|---|---|
| 3.1 | `/context` | `compact_history_now` already exists (`/compress` rides it); a status read needs the live message/token estimate the agent holds | `/compress` covers the mutation half; what is missing is the STATUS half |
| 3.2 | `/memory` | `MemoryWireProvider` `recall_scored` (iter-1301 put the score on the wire) over `run_session_scoped_command` | The TUI twin is a panel; the channel twin is a bounded text list |
| 3.3 | `/skill`-invoke | `SkillManager` is on the agent (`set_skill_manager`, iter-175's `/reload-skills` sets it) | Invoke-by-name from a channel |
| 3.4 | `/export` | the transcript the session store holds | Read-only; no turn lock needed, so it is the cheapest of the set |
| 3.5 | `/search` | depends on which search — resolve the ambiguity first | Do not schedule until the intended target is named |

**Explicitly NOT scheduled:** the TUI shell-outs (`/cron`, `/kanban`,
`/curator`, `/org`) — the gateway already answers "use the CLI" honestly, and a
channel twin of a subprocess launcher is a worse experience than the redirect.

**Acceptance for the tranche:** each command either does real work or refuses
honestly naming the surface that does. The regression battery in
`gateway_runner/tests.rs` is the seam's guard; the registry-invariant test
(`registry_names_and_aliases_are_unique`) catches dead entries.

---

# Suggested execution order

```
Tier 1 → 1.1 (22-tool smoke pass, consent first)
         1.2 (S2 live wide probe → close or split)
         1.3 (SOURCEHOUND_GROUP_HINT one-liner)
         1.4/1.5 — none, recorded as ruled
Tier 2 → 2.1 none (ruled) · 2.2 none (pool-owned) · 2.3 needs owner ruling
         2.4 none (peer-shipped, residual recorded)
Tier 3 → 3.1 /context status, then 3.4 /export (cheapest), 3.2 /memory,
         3.3 /skill-invoke, 3.5 /search (last — needs the ambiguity resolved)
```

**What this plan deliberately does NOT do:** it does not edit the operator's
`~/.agents/skills` pool, does not touch the peer's sourcehound work or its
residual, does not migrate `~/.operant/skills` without the operator's ruling,
and does not re-run any row whose state it already verified this session.
