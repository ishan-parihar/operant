# recursive self-harnessing for operant — harness2 contrast audit and ponytail upgrade plan

Dated-frozen: published `2026-11-05`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Context

Audited `../external-repos/harness2` (Google's recursive agent-harnessing framework,
~13k LOC Python, arXiv 2610.14224) against operant's self-improvement surface. Read in
full: `core/{spec,tree,library,ledger,recursion,selection,context,proposer,prompts,schedule}.py`.
The operant side was verified from source (Explore sweep + direct reads of
`background_review.rs`, `turn_finalizer.rs`, `harness_slots.rs`, `harness_tools.rs`,
`persistence_seam.rs`), not from AGENTS.md claims.

## What harness2 does — the method, not the benchmark

Harness² is a **score-blind recursive harnessing loop**. The model is fixed; the
surrounding harness improves. Two levels:

**Task-level recursion — Propose · Probe · Compose** (per task):

1. **k base rollouts** of the current harness on the task.
2. **Propose**: 2k parallel editor sessions — k **INV** ("how the agent WORKS":
   procedure, tool use, completion) + k **DOM** ("what a PRACTITIONER knows": domain
   knowledge). Each session receives a short pointer mission plus an **evidence tree on
   disk** (`context/{task.md,rollouts/,deliverables/,harness/,library/}` — full
   trajectories, never prompt-budget-truncated), and edits the harness **files in place**
   (`context/harness/`); the edited workspace is harvested (`proposer.py::call`).
3. **Probe**: roll every candidate harness on the SAME task → behavioral contrast
   (deliverables + trajectories). `RunView` "Never carries a score" (`spec.py:178`).
4. **Compose**: k composer sessions build finals from the candidate pool + contrasts.
   Invalid proposal → ONE retry with the error fed back → **inherit the current harness**
   (`recursion.py::_run_stage`). The loop never dies on a bad proposal.
5. **Select**: blind, anonymized choice between final *deliverables* (`selection.py`) —
   A/B/C ids assigned by seeded shuffle, harnesses hidden.
6. **Reflect** (`prompts.py::experience`): per-edit verdict rows
   (`helped | hurt | no observed effect | confounded`) citing the trajectory moment, then
   **at most ONE** transferable lesson (`cue / lesson / evidence / confidence`) — and
   adoption is explicitly NOT proof an edit worked ("The adopted candidate is not the
   only evidence"). `NO_ENTRY` is a first-class outcome.

**Harnessing-level recursion — the Library** (`library.py`, threaded in
`schedule.py::run_domain`): per-domain store carrying the **full adopted tree** (never a
summary) + `EXPERIENCE.md`; `INDEX.md` rebuilt from disk; read before each task,
appended after; same-domain entries only, **retrieval by construction**.

**Load-bearing invariants (the transferable part):**

1. **Score-blind discipline** — grader/test verdicts never enter improvement context;
   mechanical tripwires at every boundary (`context.assert_no_grader_verdicts` scans
   file names AND contents; `assert_no_leak_paths` for answer-key globs). This is
   harness2's #1 documented failure mode defense: an improver that sees the grader
   overfits to the grader.
2. **Evidence as files, prompt = pointer** — nothing truncated to fit a prompt.
3. **INV/DOM dim separation** — each lane is told "a parallel candidate owns the other
   dimension" (`prompts.py:370,407`).
4. **Cheapest-effective-component ladder** (INV template): tool error → bounded
   fallback; mechanical repetition → script; triggered workflow → skill; ignored
   invariant → defensive loop lever (only with repeated evidence); ordinary sequencing →
   concise always-on guidance.
5. **Omitted-is-inherited** edit contract; validate → retry-once → fallback = inherit.
   Degrade, don't die.
6. **Always-on cap + autotrigger repair** (`tree.py::autotrigger/validate`) — skills get
   a trigger line in always-on text within a char cap.
7. **Lesson attribution discipline** — every lesson names its component and cites
   evidence; `confounded` is preferred over invented attribution; "too generic is
   useless, too specific never fires again".

## Contrast — operant already has the hard half

Verified status of each harness2 mechanism in operant:

| harness2 mechanism | operant status (verified) |
|---|---|
| Editable harness surface | ✅ EXISTS, spread over 4 seams: skills (`tools/skills_tool.rs:1707` skill_manage), prompt sections (`harness_slots.rs:50` PromptSlot, rendered in the frozen prefix at `agent/events.rs:962`), prompt addendums + subagent specs (`tools/kernel/harness_tools.rs`, the continual-harness store, kinds `prompt`/`subagent`), memory (`memory_wire.rs`) |
| Write paths + validation + rollback | ✅ skills_guard security scan on every skill write; `kernel_refine` all-or-nothing with byte-exact snapshot rollback; frontmatter validation; Holon runs diverted to the org learning sink |
| A reflection loop | ✅ background review fork (`agent/background_review.rs`, spawned at `agent/prompting.rs:384`): replays last 40 messages, tool whitelist exactly `memory_store`/`memory_search`/`skill_manage`/`skill_view` (+ one `kernel_refine` when `harness_auto_learn` is on), cadence-gated (`creation_nudge_interval=10` iterations, `memory_nudge_interval=10` turns) — **running by default** |
| Score-blind tripwire | ❌ MISSING — nothing scans lesson/skill text for test/grader verdict contamination |
| First-class experience record | ❌ MISSING — memory_wire stores opaque tagged text; no `cue/lesson/evidence/confidence` record type |
| Per-edit verdict discipline | ❌ MISSING — review prompts ask for updates, not contrast-attributed verdicts; no `confounded`/`NO_ENTRY` concept |
| Library retrieval before editing | ⚠️ PARTIAL — BM25-fused recall exists (`recall_with_weights` → prefetch), but lessons are not a distinct retrievable class, so nothing fires when the same *situation shape* recurs |
| Lifecycle curation of agent-authored components | ✅ code exists (`curator/mod.rs`), ❌ **zero runtime callers** (`maybe_run_curator` at `curator/mod.rs:363` is never invoked; `--background` prints "not yet supported") |
| Probe via candidate rollouts | ❌ N/A without benchmarks — but operant's **real turns are the probe**; the contrast signal is the transcript the review fork already replays |
| Blind selection between candidate outputs | ❌ N/A — there are no candidate rollouts to select between; the existing approval/guard path IS selection |
| Benchmark machinery (adapters, channels, sweeps, budget-identity ledger, runmark cache, parallel "agg" mode) | ❌ N/A — operant's task stream is real usage, not a benchmark |

**The ponytail verdict.** Operant does not need harness2's engine transplanted. Its
editable surface, write paths, validation, rollback, and the reflection fork all exist
and run by default. What is missing is the *discipline* harness2 wraps around the same
shape: a first-class lesson record, verdict-attributed reflection, a contamination
tripwire, retrieval that fires on situation shape, and one dead hook wired. Three small
iterations; the only new persistence is one reserved tag and one marker scan.

## Mechanism

One design claim runs through all three iterations: **operant's probe is its real
turns.** Harness2 rolls candidate harnesses on the same task and contrasts the
executions; operant cannot spawn synthetic rollouts, so the next real turns under the
edited harness are the probe, and the contrast evidence is the transcript the review
fork already replays. That is weaker than same-task A/B (no candidate isolation), and
the verdict discipline below is chosen for exactly that: `confounded` will dominate,
and per harness2 "a confounded row is a useful record; an invented attribution poisons
the library".

### iter-A — the harness-lesson record (`memory_wire`)

- New reserved tag `harness-lesson`. A lesson is ONE tagged memory whose body follows
  the harness2 four-line format: `component:` (the skill/slot/store entry it concerns),
  `cue:` (the shape of situation it fires on), `lesson:` (the operation, stated so it
  can be applied and checked), `evidence:` (what was observed, where). `confidence:` optional.
- **Tripwire at retain time** (the port of `context.py::assert_no_grader_verdicts`):
  refuse-and-log a lesson body carrying validation/verdict markers — a small const list
  (`"test result:"`, `"FAILED ("`, `"panicked at"`, `"0 passed"`, `"cargo test"`,
  `"test_command"`) scanned on the lesson text only. Mechanical backstop; the review
  prompt (iter-B) carries the primary instruction not to quote validation output.
- Files: `crates/operant-core/src/memory_wire.rs` (tag const + retain-path scan),
  `crates/operant-core/src/tools/memory_tools.rs` (nothing if `retain_tagged` already
  carries tags — check at implementation; the shape is enforced by prompt, not parsed).
- Gates: round-trip test (retain a lesson, recall it by component query); tripwire test
  (a body containing `test result: FAILED` is refused, not stored).

### iter-B — the review fork becomes a propose–reflect lane (prompt-level only)

Changes to `SKILL_REVIEW_PROMPT` / `build_review_prompt` in
`agent/background_review.rs` — zero new machinery:

1. **INV/DOM split**: two dimensions in one review pass (two prompts is one more fork
   per threshold — not needed at k=1): INV = how the agent works (procedure, tool use,
   completion), DOM = what the practitioner knows (domain knowledge). Each with the
   cheapest-effective-component ladder, mapping to operant's actual components:
   ordinary sequencing → concise always-on guidance (prompt addendum via `kernel_refine`
   when auto-learn is on) / triggered workflow → skill via `skill_manage` / mechanical
   repetition → script inside a skill / tool error → bounded fallback guidance.
   A line states the other dimension is owned elsewhere, harness2-style.
2. **Verdict discipline**: every proposed edit must state what was observed and a
   verdict `helped | hurt | no observed effect | confounded`, citing the transcript
   moment; `confounded` is preferred whenever the record cannot isolate the edit; **at
   most ONE** durable lesson per review; `NO_ENTRY` is expected most reviews ("skip
   entirely when nothing durable emerged" — already in the auto-learn lane text, now
   becomes the general rule).
3. **Library read before edit**: the fork's whitelist already includes `memory_search`
   — prompt it to retrieve prior `harness-lesson` memories for the components it is
   about to touch before proposing anything (the harness2 `context/library/` step).
4. The fork never sees validation output by instruction (iter-A's scan is the backstop).

Files: `agent/background_review.rs` (prompt text only), possibly
`agent/prompting.rs` (no whitelist change needed — `memory_search` is already there).

Gates: prompt-content tests (both dimensions present, NO_ENTRY clause present,
retrieval instruction present); the frozen-prefix byte-stability test stays green (all
of this is review-fork/volatile-tail content, never the frozen prefix).

### iter-C — close the loop: retrieval, curator auto-run, readout

1. **Cue-firing on the live path**: memory prefetch (`recall_with_weights` → pre-turn
   injection) includes the `harness-lesson` tag in its recall so lessons surface when
   their situation shape recurs. Rides the existing prefetch machinery; no new
   scheduler. If the prefetch path needs an explicit opt-in:
   `memory.prefetch_include_lessons: bool = true` in `operant.example.toml` same-change.
2. **Curator auto-run**: call `CuratorEngine::maybe_run_curator` (`curator/mod.rs:363`,
   built for this, zero callers today) from the same cadence gate that fires the review
   fork. The curator then owns the lifecycle (archive-after-idle, staleness,
   consolidation) of the skills the review fork creates — the harness2 library's
   garbage collection, which harness2 never had.
3. **Effectiveness readout**: the `operant status` MetricsSnapshot gains two counters —
   harness-lesson count and curator archive count. The smallest honest efficacy signal;
   NOT a benchmark.
4. One `kernel_refine` note: the prompt/subagent addendum lane is opt-in OFF
   (`tools.kernel.harness_auto_learn`, default false). With iters A–C the loop is real
   at the skill/memory level; whether the harness-level lane (prompt addendums) flips
   ON by default is an owner decision — recommend flipping after the lesson discipline
   has run for a period, not silently in this plan.

Files: memory prefetch path, `agent/turn_finalizer.rs` or the cadence gate site,
`cmd_status.rs`/metrics, `operant.example.toml` if the flag lands.

Gates: prefetch-inclusion test; curator fires under the gate when its interval has
elapsed; status JSON carries both counters.

### Explicitly NOT ported (skip list)

- **Benchmark adapters, channels (claude_cli/opencode), sweeps, Namespace/runmark
  caching, report/demo** — operant has no benchmark; its task stream is real usage.
- **Parallel "agg" mode, k>1, the budget-identity Ledger** — k=1 sequential on a 2-CPU
  personal box; R=4k/P=3k accounting is meaningless there, and worklog rows already
  record what happened.
- **The SurfaceSpec/Tree model** (`tree.py`) — each operant seam already validates
  (skills_guard, kernel_refine rollback, frontmatter rules); a second abstraction over
  the same guarantee is overhead with no new behavior.
- **Blind anonymized selection** between candidate outputs — no candidate rollouts
  exist to select between; the existing write-approval/guard path IS selection.
- **Harness-tree snapshots in the library** — the curator's backups plus git history
  are the audit trail; lesson + component pointer suffices.
- **Always-on cap + autotrigger repair** — the frozen-prefix machinery and the skill
  router's 200-line cap already enforce the equivalent.
- **KaizenProposal wiring** (`org/worklog.rs:215`) — designed for this but worklog rows
  exist only on org/seat runs (`agent/run.rs:2016`); wiring it on the main path would
  be speculative. The `harness-lesson` tag IS the kaizen of the main path. If the org
  lane wants it later, its doc already names the producer contract ("a framework task
  at session end").

## Gates / success criteria

(fixed before the run)

1. Every iteration: scoped `check`/`test`/clippy-gate green; regression test cited in
   the commit body (Iron Rule: commit + push per iteration, `docs(iter-N)` numbering
   from `origin/main`).
2. iter-A: retain/recall round-trip + tripwire tests green.
3. iter-B: prompt-content tests green; frozen-prefix byte-stability test stays green.
4. iter-C: prefetch, curator-gate, and status-counter tests green;
   `operant.example.toml` updated same-change if the flag lands.
5. Efficacy readout (measurement, not a gate): after a period of real use,
   `operant status` shows harness-lesson count > 0 and curator archives agent-created
   skills; autonomous-mode repeat-failure rate (`FailureRecord.attempts`,
   `autonomous.rs:804`) trends down. All read from existing state files — no new harness.

## Cost

- Three iterations, scoped to `operant-core` (+ `operant.example.toml` in iter-C if the
  flag lands). No new crates, no new dependencies, no TUI, no gateway, no schema
  migration (one reserved tag on an existing store).
- New-code estimate: ~200–300 LOC total (tag const + marker scan + status counters +
  tests); the bulk of iter-B is prompt text.
- Runtime cost: unchanged cadence (the review fork already fires per threshold); one
  more memory tag; one cadence-gated curator call.
- Risks: (1) lesson noise polluting recall — mitigated by the NO_ENTRY-expected
  discipline, the curator's archive lifecycle, and the confidence field; (2) the probe
  is weaker than same-task A/B, so verdicts skew `confounded` — accepted by design,
  per harness2's own attribution rule; (3) test-verdict contamination — the iter-A
  tripwire is the mechanical backstop for harness2's #1 documented failure mode.
