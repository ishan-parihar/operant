# codex portability remaining gaps — execute the five shippable rows

Dated-frozen: published `2026-10-31`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**Supersedes nothing. Scopes the residue of** `docs/audit/2026-10-30-codex-portability-ledger/`
**after two facts landed:**

1. **iter-1510 shipped Row 4 of `01-tool-surface.md`** (relevance ranking, RT-200 closed).
2. **iter-1523 re-measured all 30 rows** (`docs/audit/2026-10-30-ponytail-codex-portability-execution.md`)
   and **refuted Rows 02-1 and 02-2 with measurement, and under-priced Rows 01-1 and 03-2**.
   The ledger's 9-import shortlist is therefore **5 shippable rows, not 9** — and two of the
   five are cheaper than the ledger claimed.

This plan is the build list for those five. It is not a re-derivation: every row below names
the `file:line` measurement that sizes it, and every row that is *not* here carries its
disqualifying measurement in the audit so nobody re-litigates it.

---

## What is already done (do not re-do — these are verified shipped, not "in progress")

| Ledger row | Shipped | Guard |
|---|---|---|
| 02-safety Row 3 — phantom sandbox comments | **iter-1504** (`ba37ce26`, RT-198) | `crates/operant-core/tests/no_phantom_isolation_layer.rs` |
| 01-tool Row 4 — relevance ranking | **iter-1510** (`260ceeb2`, RT-200) | 5 tests in `tool_search::tests` |
| 04-obs Row 1 — tool-call duration was `0` | **iter-1506** (`aeb926de`, RT-199) | `crates/operant-core/tests/tool_call_duration_is_real.rs` |
| 01-tool Row 3 — dispatch trace, **duration half** | **iter-1506**, same commit | same guard |

The genome board `codex-portability-imports` rows 001 and 002 still read `open`/`in-progress`
for the first two of those; they are stale and **the first task of this plan is to correct the
board**, not the code.

---

## The five rows to build, in order

### 1. Row 01-1 — `ToolExposure` as a type-level decision (re-graded S, not M)

**Measurement that sizes it:**
- `crates/operant-core/src/tools.rs:592-600` — `get_schemas_all` iterates `tools.values()`
  **holding `t`** and calls `t.schema().with_intent()`. The registry association is in hand;
  `.with_exposure()` is the same shape.
- `crates/operant-core/src/schema.rs:154-158` — `ToolSchema` is 3 fields; a 4th is a struct
  change, not a redesign. `assemble_tools` (`tool_search.rs:107-119`) keeps its signature.
- Today the decision is `is_deferrable(name)` (`tool_search.rs:80-82`), a **string-prefix +
  table-scan** that runs at six sites (`:115`, `:358`, `:527`, `:613`, `deferred.rs:131`, the test).
- The scan cost is real and uncached: `is_sourcehound_composite` → `composite_specs()`, which
  **allocated a 35-element `Vec` per call** — fixed separately in iter-1523 (RT-202, now a
  `LazyLock`), so this row's *performance* motive is partly spent and its *structural* motive
  (the deferral decision lives on a name, not on the type that owns the tool) stands.

**Effort:** S (~42 added, ~14 deleted, 0 deps).
**Acceptance:** `ToolExposure` enum exists; `McpNamespacedTool` + sourcehound composites +
the CLI roster return `Deferred`; `assemble_tools` partitions by the flag; **and a tool that
merely happens to be named `mcp_*` but is not one of those families is no longer deferred by
accident** — that is the behaviour this row buys, and the test must force it (a name collision
fixture, not a smoke test).
**Mutation guard:** revert `assemble_tools` to `is_deferrable` and the test must fail by name.

### 2. Row 03-2 / 01-2 — the model's own context budget, **as a tail line, not a tool** (re-graded ~12 lines)

**Measurement that re-grades it:** both ledger docs recorded M because "`messages` is not a
parameter of `execute_tools`". That is true of the **tool** form and irrelevant to the **tail**
form. `estimated_tokens` (`crates/operant-core/src/agent/run.rs:2721`) and `effective_budget`
(`run.rs:2717`) are **in scope** at the `<turn_context>` attach site (`run.rs:2965-2978`),
verified by tracing the block: the one that declares them opens before `:2696` and closes at
`:2997`, and the attach site is nested inside it (depth 4), so both are readable at that exact
line. So the signal the model needs is already
computed, one block away from a mechanism that already delivers volatile per-turn text to the
model without touching the frozen prefix or the prompt cache.

**Effort:** S (~12 lines, 0 deps). **Do not build the 148-LOC codex tool and do not touch
`ToolContext`** — that is the shape the ledger's own "needs plumbing" caveat steered toward,
and the measurement says the plumbing is unnecessary.

**The two values to read, both already in scope at the attach line (verified, not assumed):**
- `estimated_tokens` (`run.rs:2721`) and `effective_budget` (`run.rs:2717`) — declared in the
  same block that closes at `:2997`, with the attach site nested inside it.
- `ladder_fired` (`run.rs:2723`) — the preflight ladder's own boolean, already read at
  `:2923` and `:2988`, i.e. two lines *after* the attach site. It is the "did operant evict
  this turn" signal and no plumbing creates it.

**Do NOT read `EvictionOutcome` (`context_management.rs:802`) for this.** It exists and is
accurate, but the agent path calls `manage_context` (`context_management.rs`, the wrapper that
discards the outcome and returns only the list) at `agent/compress.rs:380,386,411` — never
`manage_context_reporting`. `EvictionOutcome` is reachable only by changing that call, which
is plumbing this row does not need. `ladder_fired` is the same information, already computed.

**Acceptance:** the built `<turn_context>` block carries the remaining-budget figure and
whether the ladder evicted this turn; a test asserts the frozen system prefix stays
byte-identical across two turns with different budgets (`agent/mod.rs:2069` already pins that
shape for MoA guidance — copy it).
**Mutation guard:** strip the budget line from the tail and the test must fail.
**OUTCOME — iter-1545, SHIPPED as specified.** One block on the volatile
`<turn_context>`, reading `estimated_tokens`, `effective_budget` and `ladder_fired` — all three
already in scope at the attach line, so ~12 lines total and no plumbing. It charges itself
before quoting its numbers (built once to measure, size added back to `used`, built again for
the record): the first draft charged `turn_context_tokens` instead and the compiler warned it
dead, because every read of that binding happens earlier in the function. Guard
`the_tail_reports_the_context_budget_and_compaction_state`; mutation-proven RED by deleting the
block. operant-core 2685/0, clippy clean.

### 3. Row 01-3 — dispatch trace **payload** (extend the existing variant; ~8 lines)

iter-1506 fixed the duration constant. What is still missing versus codex's shape is the
payload size. `observer.rs:295` already carries `ObserverEvent::ToolCallStart { tool,
arguments }`, and at the result site `result.content` is in hand (`agent/run.rs:2207-2211`), so
`output_bytes` is `result.content.len()`.

**Effort:** S (~8 lines, 0 deps).
**Acceptance:** the emit carries input/output byte counts; `ConsoleObserver` renders them;
a test proves a large tool result reports its real size rather than 0.
**Mutation guard:** hardcode `0` for either field (the RT-199 defect's own shape) and the
test must fail by name — this is the same mutation that proved iter-1506, so reusing it
guards against a regression of the same class.

**OUTCOME (2026-10-31): DONE, with a correction to the brief.**

Implemented by `Sisyphus` in `79f2bd5c` and repaired by `agent-omarchy` in `2cd1a0e2`.
Shipped at the 4 sites the brief assumed, not 2:

- `crates/operant-core/src/agent/stream.rs` — `record_tool_input_bytes` parks the rendered
  args' length at the `ToolCallStart` emit (`:915`), `tool_input_bytes_for` reads it back by
  call id, `clear_tool_durations` clears it too (a long-lived agent would otherwise grow one
  entry per call).
- `crates/operant-core/src/observer.rs:49-61` — the two `usize` fields on the variant.
- `crates/operant-core/src/observer.rs:305-318` — the `ConsoleObserver` arm binds them into
  the `tracing::debug!` line (the brief's "renders them").
- `crates/operant-core/src/agent/run.rs:2211-2212` — the completed-call emit.

**The brief under-specified one site.** `79f2bd5c` wrote the fields at the run.rs emit and in
the observer.rs *test* only, never on the variant itself — so the tree was RED with two
`E0559` errors and nothing in the workspace could compile or test. `2cd1a0e2` is that repair
(+11 lines). Recorded on the feed as F0212.

**Guard:** `crates/operant-core/tests/tool_call_duration_is_real.rs` (2 new tests)
`the_observer_sees_the_real_byte_count_in_both_directions` and
`neither_byte_count_is_a_hardcoded_zero`. 5/5 pass. Mutation-proven with the RT-199 shape:
revert run.rs to `input_bytes: 0` and both fail by name; the fixture's input and output
lengths differ on purpose so a swapped-field report fails too.

**Also corrected:** I measured this row while a parallel attempt of my own was mid-flight in
this tree. Comparing it against `79f2bd5c` showed byte-identical design decisions, so mine
was discarded instead of pushed as a duplicate. Per the plan's own standing rule
("never take a peer's claimed row"), I shipped only the repair their commit was missing —
the 11 lines in `observer.rs`. Nothing of mine survives in the tree or in `git stash list`;
that entry was restored into the working tree and dropped after checksum-verifying all 8
peer files byte-identical against it.

### 4. Row 04-2 — MCP schema byte budget (one constant + truncation)

**Verified absent at plan time, with the trap named:** the byte cap does not exist, but a naive
grep is misleading in BOTH directions. `rg -n "MAX_SCHEMA\|schema_max_bytes\|SCHEMA_BYTES"
crates/` returns 6 hits — every one of them `MAX_SCHEMA_RETRIES` (`tools.rs:102`,
`tools/delegation_output_schema.rs:21`, `tools/sub_agent_tool.rs:44,599,604,606`), which is a
**schema-validation retry count, not a byte budget**. So the honest search is for the cap's
*purpose*: a bound on how many bytes of one tool's schema reach the model. That is the deferred
materialisation path in `tools/tool_search.rs` (`deferred_schemas` at `:353`, materialisation
(`deferred.rs::materialise_for_turn` at `:125`, the inject loop `materialise` at `:142`, and
`tool_search.rs:353`'s `deferred_schemas` feeding it), and it has no bound of any kind today.
The cap belongs at the inject loop, where one schema's `parameters` size is known.

**Effort:** S (~22 lines, 0 deps). Take codex's *cap design*, not its `McpHandlerCache` (its
weak-reference cache solves a problem operant does not have — its MCP manager is per-process).
**Acceptance:** a constant cap, a clear truncation marker, and the byte count is measured so
the operator can see what was cut (the `metadata_metrics` half of the row).
**Mutation guard:** raise the constant above the fixture schema and the truncation test must fail.

### 5. Row 03-1 — token-budget compaction without a model call (config flag + branch)

Verified: operant's three compaction mechanisms are genuinely separate
(`session/autocompact.rs:3-20` names them) and the 4,000-token floor is real
(`COMPACTION_FLOOR_TOKENS` at `autocompact.rs:167`, rationale at `:29-33`). The missing piece is a **mode that evicts hard and never summarises**,
used when the history is already a summary of a summary. Existing knob naming sits at
`config.rs:629-638` (`context_compression`, `..._threshold`, `..._summary_max_chars`).

**Effort:** S (~35 lines, 0 deps).
**Acceptance:** the new mode installs a fresh window with **zero model calls** (assert the
client is never invoked — that is the whole value), and it is refused below the floor rather
than compounding loss.
**Mutation guard:** make the mode fall back to the LLM summariser and the "no model call"
assertion must fail.

---

## Rows that are NOT in this plan, with the measurement that disqualifies them

Recorded here as well as in the audit, so the plan is self-contained:

| Row | Why not |
|---|---|
| 02-1 prefix-rule policy file | **38 of 60** `rx()` patterns in `approval.rs` are not expressible as a token prefix (char classes 17, alternation 16, gaps 8, backtick capture 3). A TOML file would sit **beside** the compiled table — two sources of truth for the safety boundary. |
| 02-2 `match`/`not_match` | operant's patterns are `&'static str` literals checked by `hardline_regex_patterns_all_compile` (`approval.rs:1294`); a typo is a **compile error**, so the silent-non-match failure mode does not exist here. |
| 02-7 tool-level hooks | needs the Row-01-1 exposure type first, and has no consumer yet. Becomes real only if an operator asks for per-tool hooks. |
| 03-4 turn-diff tracking | no consumer; blocked on a `ToolContext` enrichment nothing needs. |
| 01-9 lifecycle notifications | no consumer; superseded by Rows 01-3 once the payload lands. |
| 01-2/03-2 **as a tool** | see row 2 above — the tail form is strictly cheaper and needs no plumbing. |

---

## Gates / success criteria

(fixed before the run)

Per row, one iteration, each with a **mutation-proven** guard: the mutation must break the
behaviour the test claims, and restoring it must be green. A guard that passes under its own
mutation is the vacuous-fixture failure already recorded once in this ledger
(`01-tool-surface.md` Row 4's `mcp_github_issue_create` fixture passing with every score
forced to 0) — the fixture must be designed so the wrong implementation **disagrees** with the
right one, not merely differ by accident.

Repo gates, unchanged: `cargo fmt --all`; `./scripts/check.sh check -p <crate>`; the touched
crate's tests; `bash scripts/clippy-warning-gate.sh`; release build + deploy to both install
paths; `git log origin/main -1` confirms the push.

**Known red gate, carried into every verification step:** RT-203 (opened iter-1523) is an
unidentified `operant-core --lib` parallel-load flake. It is **delta-proven not ours** (the
touched module is 11/11 across 20 runs, the suite is 2678/0 serially) but it will occasionally
fail a parallel verification run and must not be read as this plan's regression. If the gate
goes red on an unrelated test name, capture the name and continue; do not "fix" a test this
plan does not own.

**Label discipline:** read the next iteration number from `origin/main` after `git fetch`,
and check `git rev-list --left-right --count origin/main...HEAD` before every push. Two label
collisions already happened in this lane (iter-1510, iter-1523); both were cosmetic and left
append-only. History is never rewritten to renumber.

## Cost

| Row | Added | Deleted | Deps |
|---|---|---|---|
| 01-1 `ToolExposure` | ~42 | ~14 | 0 |
| 03-2/01-2 budget tail | ~12 | 0 | 0 |
| 01-3 trace payload | ~8 | 0 | 0 |
| 04-2 MCP byte cap | ~22 | 0 | 0 |
| 03-1 compaction mode | ~35 | 0 | 0 |
| **total** | **~119** | **~14** | **0** |

Five iterations, one deploy each. `bm25` and `starlark` stay out, as both the ledger and the
audit require. Nothing here adds a dependency, a crate, or a new subsystem.
