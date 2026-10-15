# Re-audit brief: 2026-10-14 batch — remaining gaps after the agentic-os pivot

**Date:** 2026-10-15 · **Author:** the batch-execution agent (iters 815–857)
**Audience:** red-team audit agents tasked with re-auditing the project direction
**Inputs to read first, in order:**
1. `docs/plans/2026-10-14-batch/00-INDEX.md` — the batch's 12 plans, resolved decisions, and (appended iter-857) the execution-status map.
2. The per-plan files `01…11` in the same directory (the authoritative acceptance criteria this batch executed against).
3. `docs/agentic-os/ORG-LAYER-DISPOSITION.md` — the owner-direction document that **supersedes part of the batch plans** (see §2).
4. `docs/agentic-os/AUDIT-2026-10-15.md` — the agentic-os audit (items O-01…O-29) the concurrent W-plan stream executed against.
5. `docs/CHANGELOG.md` `[Unreleased]` — the iteration-by-iteration record.
6. Session note #10 (batch resumption state) and `docs/MEMORY_BUDGET.md` (07.1's measured record).

**Why this brief exists:** the batch was planned 2026-10-14 against an org/ layer
that the agentic-os disposition has since largely **retired**. Roughly half the
remaining phases (11E, parts of 06) were scoped against modules that no longer
exist or whose mandate changed. A re-audit must not ask "were the batch plans
executed correctly" alone — it must ask **which parts of the remaining plans are
still the right work at all**.

---

## 1. Execution scorecard (what landed, with proof pointers)

| Plan | Status | Iterations | Notes |
|---|---|---|---|
| 11A build entry + provenance | **done** | 815 | `scripts/build.sh` the one entry point; rev baked into `--version`/doctor |
| P2 sim infra | **done** | 816 | `--frame-per-event`, `count:` asserts; corpus 0-drift |
| 02 dead commands | **done** | 817 | 35 purged; `no_registered_tui_command_is_unwired` pins the class |
| 10 shortcut/overlay coherence | **done** | 820 | registry-generated overlays; `/permissions` picker; Shift+Tab inert |
| 08 visibility toggles | **done** | 821 | Ctrl+T thinking, Ctrl+Shift+O/Alt+O tools, Alt+Y copy |
| 03 reasoning effort | **done** | 829 | config→shape table→rejection fallback→TUI; 8 shape tests |
| 04 cancel preserves context | **done** (live red-team pending) | 832 | typed `Error::Interrupted`, tail-commit, interruptible tools, 3s watchdog |
| 01 duplicate tool rows | **code done, S1/S2 verdicts live-only** | 836 | prep-oracle + per-event frames landed; verdict needs the provider |
| 05 rewind | **done** (live proof pending) | 829/838/839 | core marks/fork + fullscreen selector; corpus goldens |
| 09 native scroll | **step 1 (K-3) done**; steps 2–7 live-gated | 839 | message-final watermark; native-scroll corpus goldens |
| 11B binary updater | **done** | 841 | shared sourcehound+aft path; live 1.6→1.7 update proven |
| 11C1/2/3 purges | **done** | 842/843/846 | sourcehound-only web+browser; memory-wire-only + one-time importer |
| 07.1 footprint | **done** | 848/850/851 | stderr drains, warmup reap, 4MB tool-output budget, `debug memory`, measured table |
| 11E core seam | **done** | 856 | `bank_for`/`scoped()` + isolation tests (see §3.3 — the *remaining* scope changed) |
| Docs closeout | **done** | 857 | README/TODO/INDEX current for everything landed |
| 07.2 shared services | **not started** | — | see §3.2 |
| 06 P0–P5 | **not started** | — | see §3.1 |
| 11D crate audit | **not started** | — | see §3.4 |

Release binary deployed through iter-817 (dual-path install, gateway restarted,
md5-proven). Later iterations await the next release build window.

## 2. The ground that moved: the agentic-os pivot (the primary re-audit subject)

The concurrent W-plan stream (iters ~837–855, subjects `W1…W8`, audit items
O-01…O-29) landed the Holon integration:

- **New crate `operant-os`** (api, archive, chair, contract, enforcement,
  events, organs, policy, run_env, run, supervisor, tools) — the body-side
  runtime for Holon runs. `docs/agentic-os/BOUNDARY.md` fixes the mode line.
- **`org/` shrank from 26 files to 7** per `ORG-LAYER-DISPOSITION.md`:
  kept/rewired — `seat_policy` (KEEP, the enforcement core), `seat_budgets`
  (REWIRE: caps from `policy.json`), `seat_authority` (personal mode only),
  `context_injection` (channel-sourced aspects only), `worklog`'s accumulator;
  **retired** — `authority`, `employee_db`, `hierarchy_edges`,
  `pending_requests`, `synthesis`, `worklog_db`, `write_barrier`,
  `decisions_db`, `cast`, `department_db`, `notice*`, `dm_thread`,
  `socialization`, `identity_gate`, `schema`.
- **The gateway is now an organ host**: W4 put the supervisor and clock in the
  gateway daemon; W5 routes the Chair's channel; W6/W7 made Holon runs annotate
  learning (`org::holon_learning_sink`) and **never write personal memory
  stores** (their O-22 invariant — audit-note the overlap with my iter-846).
- **Known broken at tip (committed)**: `gateway_commands.rs`'s test fixture
  `seed_p3`/`P3Stores` (7 references) still constructs `GrantDb`,
  `EmployeeDb`, `HierarchyEdgesDb`, `PendingRequestDb` from the retired
  modules — the cli **test target does not compile**. This is a disposition
  mismatch, not in-flight WIP: the fixture must be rewired to the kept modules
  or Holon's read API. The lib and bin targets are green. This debt blocks
  execution (not compilation of) several committed batch tests — see §4.

**Re-audit question A (the central one):** the batch plans 06/11E/07.2 were
written for the pre-Holon org layer. For each remaining phase, decide:
*still-valid as written*, *re-scope to the disposition*, or *superseded —
delete the plan section*. The per-phase analysis below gives the evidence for
each.

## 3. Remaining gaps, per phase

### 3.1 Plan 06 — context management (`06-context-management.md`, P0…P5)

Not started. **Files it touches are the W-plan's active zone** — `agent/prompting.rs`
(O-17 refusals, O-21 usage sidecar), `agent/compress.rs`, `org/mod.rs`,
`agent/builders.rs` were all mid-flight during the last three checks.

Drift impact per sub-phase:

- **P0 (volatile blocks → tail; cache telemetry)** — the request-assembly
  invariant (byte-stable prefix) now coexists with the W-plan's `usage.json`
  sidecar and refusals merge in the same function. The plan's "no user message
  at index 0" RED test must be re-derived against the current `build_messages`.
  *Audit: is the cache-stability goal still the priority, or does the Holon
  run contract's `result.usage` path supersede the in-band telemetry half?*
- **P1 (BustPermission scheduler, ContextExhausted at 95%)** — unchanged by
  the pivot (agent-loop internals); still valid as written.
- **P2 (context_tags, §N§ prefixes, ctx_reduce/ctx_expand tools)** — the
  tags table is `database.rs` (uncontended), but the §N§ prefix scheme
  overlaps `context_injection.rs`, which the disposition **rewired**
  (self/dept/global sources → Holon messages/memory). *Audit: should §N§
  tagging apply to channel-sourced context only, or does Holon's brief/message
  flow carry the identity now?*
- **P3/P4 (background historian → memory-wire)** — in org mode, memory is
  **Holon's** (O-22: runs never write personal stores; learnings ride
  `result` annotations). *Audit: is the historian a personal-mode-only organ,
  like `seat_authority`? The disposition pattern says yes; the plan said
  nothing (it predates the boundary).*
- **P5 (soak, dogfood, default flip)** — unchanged.

### 3.2 Plan 07.2 — shared services (`07-memory-footprint.md` phase 2)

Not started. **Design reaffirm needed:** the plan hosts MCP/aft pools in the
gateway daemon — the W-plan has since moved the supervisor and clock into
that same daemon (W4). This *strengthens* the design (the daemon is already
multi-tenant) but the socket protocol now shares a process with organs.
`docs/MEMORY_BUDGET.md` carries the measured baseline (gateway 26.7 MB idle;
TUI 24–25.5 MB; arena knob measured nil) and the acceptance gate
(5×TUI ≤ 5×40 MB + daemon). The lazy-MCP-children idea from plan 07.1 was
**folded into this phase by measurement** (idle already at target; the
intermediate layer would double the surface the pool deletes).

*Audit: confirm the services.sock design against the organ-hosting gateway,
or fold 07.2 into the agentic-os phase-3 "thin client" decision the
disposition defers.*

### 3.3 Plan 11E — one memory model (`11-integrations-and-purge.md` §E)

**Half landed, half re-scoped by the pivot.** Landed (iter-856, uncontested):
`MemoryScope {Personal, Seat}` + `bank_for()` (sanitized `emp:<identity>`
banks) + `MemoryWireProvider::scoped()` + real-store isolation tests — the
solo-never-sees-`emp:*` invariant is pinned.

What changed: the plan's remaining items were
(a) *org/synthesis stops opening its own SqliteStore* — **moot**: `synthesis.rs`
is retired outright by the disposition (Holon memory promotion replaces it);
(b) *seat memory files → emp: banks* — still valid for **personal-mode
operator seats** (the disposition keeps `seat_policy_db` in personal mode),
but the wiring target is now the kept module set, not the retired org store;
(c) *charter postscript* — verify against the Holon charter flow (Holon
`employees`/`grades` own org-mode identity now).

*Audit: re-scope 11E's remainder to "personal-mode seat memory banks via the
landed `scoped()` seam"; confirm org-mode memory is exclusively Holon's
(O-22) so no `emp:` bank is ever consulted in a Holon run.*

### 3.4 Plan 11D — unused-crate audit

Not started (last by design). Grew by one crate (`operant-os`). K-4
(`--no-default-features` builds) remains open in BUGS.md. Method is unchanged:
fix the builds, prove each dep via build+gateway smoke, `docs/CRATES.md`,
then delete with per-deletion proof. *Run only after 06/07.2/11E settle so
deletions don't race their dependencies.*

### 3.5 Deferred test executions (committed, not yet run)

All compiled into the bin target; blocked by the §2 `seed_p3` fixture debt:
- `tool_output_budget_evicts_oldest_first` (iter-850) — 4MB eviction budget.
- The rewind-selector TUI tests (iter-838 series) — open/filter/Enter gates.
- The LIVE-class tests below need the provider instead.

Delta-zero was proven per fleet rule 7 (error attribution across three build
passes). **First action in the next green window: run these, then the full
bin suite.**

### 3.6 LIVE red-team items (provider-gated)

The omp model gateway flapped across three error classes during the session
(403 dead free-tier route, 503 upstream pressure, 400 lost model mapping);
every live attempt reached the machinery and failed upstream. Harness is
ready and documented (session note #10; tmux socket `live04`; model =
api-key:anthropic route via `/model`):

- **LIVE 01** — 5-tool turn with `--frame-per-event`-style capture +
  F12 `d` buffer dump → S1 (OSC-8 residue) / S2 (wide-glyph) verdicts. The
  headless oracle (iter-836) reports 0 mismatches; these verdicts are the
  plan's final word.
- **LIVE 04** — Esc during a live `sleep 20` shell call: graceful cancel,
  Cancelled rows, next turn proves the partial is in context.
- **LIVE 05** — 3-turn live rewind: context actually rewound at the provider,
  fork visible in `/sessions`.
- **LIVE 09 steps 2–7** — dynamic live-region height, replay on
  clear/rewind/resize, alt-screen overlays, native selection, default flip.

## 4. External gates (both re-verified 2026-10-15, multiple probes)

1. **The org/ gateway debt** (§2): cli test target red at tip from the
   committed `seed_p3` fixture. Every committed batch test rides its fix.
2. **Model provider**: the local omp gateway's upstream instability (three
   error classes). All LIVE items gate on it.

## 5. Re-audit checklist (concrete questions, in priority order)

1. **Direction:** with the disposition executed, is the remaining batch
   (06/07.2/11E-remainder/11D) still the correct next work, or does the
   agentic-os IMPLEMENTATION-PLAN's phases G–I supersede parts? Name the
   superseded plan sections explicitly.
2. **Boundary:** O-22 (Holon runs never write personal stores) vs 11E's
   `emp:` banks vs 06.P4's historian→memory-wire — one coherent memory
   boundary statement should fall out of the audit.
3. **06.P0 cache stability** vs the usage/refusals sidecar cohabitation in
   `prompting.rs` — re-derive the RED tests against the current assembly.
4. **07.2** reaffirm or fold into the thin-client decision.
5. **The `seed_p3` fixture** — prescribe the rewire (kept modules vs Holon
   read API) and who executes it (it sits in the W-plan's zone).
6. **Deferred tests** — run-order and ownership once the fixture is fixed.
7. **LIVE items** — confirm the provider recovery criterion (a stable route
   for ≥3 consecutive probe turns) and whether any S1/S2 verdict can be
   obtained headlessly after all (the oracle says no; challenge it).
8. **Ledger hygiene:** duplicate iteration labels exist (iter-838 ×2,
   iter-850 ×2 — documented in commit bodies, append-only per protocol);
   confirm no renumbering is attempted.

## 6. Pointers

- Batch plans: `docs/plans/2026-10-14-batch/00-INDEX.md` (+ per-plan files; the
  index carries the plan→iteration map appended at iter-857).
- Agentic-os: `docs/agentic-os/{BOUNDARY,ORG-LAYER-DISPOSITION,IMPLEMENTATION-PLAN,AUDIT-2026-10-15}.md`.
- Record: `docs/CHANGELOG.md` `[Unreleased]`; `docs/MEMORY_BUDGET.md`.
- Resumption: session note #10 (blockers, queue order, tmux/provider specifics).
