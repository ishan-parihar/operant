# Close the pub-fn surface queue one acid-proven group at a time

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Why this plan exists

`python3 scripts/audit-pub-fn-surface.py --check` is **the last red gate in the
battery**. Measured at `origin/main` = `73088cab` (iter-2751), in a clean throwaway
worktree, after the doc/ledger/genome debt was closed by peers:

| gate | exit | verdict |
|---|---|---|
| `bash scripts/check-doc-claims.sh` | 0 | green |
| `python3 scripts/audit-ledger-index.py` | 0 | green (RT-554/RT-555/RT-614 collision resolved by iter-2707) |
| `bash scripts/supply-chain-gate.sh` | 0 | green |
| `bash scripts/clippy-warning-gate.sh` | 0 | green (iter-2744 closed the `tests/` exemption hole) |
| `./scripts/genome lint` | 0 | green (iter-2750 closed E311 + the W006/W010 warnings) |
| `python3 scripts/audit-pub-fn-surface.py --check` | 1 | **130 NEW candidates** |

`cargo test -p operant-core --lib` is **not** red any more either: 3689 passed / 5 failed
at `6a90b5c4`, and all 5 are `tools::kernel::tests::*` failing only because a fresh
worktree has no `kernel-sidecar/.venv` and no `vendor/prime-agent` submodule (AGENTS.md
fleet rule 6). The iter-2702 "suite is red at tip" premise is stale; the 27-failure
steady state the plan `2026-11-20-close-the-test-pollution-gap-*` describes no longer
exists.

So the remaining red is one gate with one question: which of the 130 `pub fn`s in
`operant-core` have no production caller, and for each, is it dead (delete), reachable
(a caller the census missed -- fix the census), or a compat shim (account for it)?

## The honest framing, stated first

**A candidate is NOT a verdict.** The auditor says so in its own docstring and prints it
after every run. Its census counts identifier occurrences; a function called through a
trait object, a macro, a different name, or a doc example reads as a candidate while
being live. The **acid test** is the only proof:

```bash
# comment out the `pub`, then:
cargo check -p operant-core --lib    # a clean compile with only `dead_code` == dead
```

This plan's method is: **acid-test in small coherent groups, one iteration per group,
and never delete on a scan alone.** Each group ships with the count before/after and the
mutation that proves the deletion was correct.

**Why not delete all 130 at once.** Two of the 130 are `ACCOUNTED_FOR` compat shims
(kept so a retired key's warning still fires); a blanket delete would silently drop the
operator's deprecation surface. And the census is a heuristic: iter-2709 deleted three
candidates after the census had been *fixed* twice for exactly this class of error (the
first draft reported 382 candidates; 3 were live and only the acid test proved it).

## The measured landscape at `73088cab`

```
pub fns in operant-core (production code) : 2141
names defined more than once (skipped)    : 142
candidates (0 production callers)         : 132
  accounted for                           : 2
  NEW                                     : 130
```

**43 of the 130 have `tests=0`** -- no test names them either. That is the strongest
prior for dead (a live function usually has at least one test), and it is where this plan
starts. The other 87 have tests and are a different question (a tested-but-unwired
function is a real design gap, not a dead one -- it is RT-275's territory, not this
plan's).

### The `tests=0` subset, by file (43)

| file | n | candidates |
|---|---|---|
| `skills_guard.rs` | 3 | `determine_trust`, `format_report`, `structural_check` |
| `trajectory.rs` | 2 | `set_success`, `to_json_compact` |
| `tools.rs` | 2 | `enable_tool`, `is_direct` |
| `oauth/store.rs` | 2 | `with_default_path`, `with_lifetime` |
| `database.rs` | 2 | `child_sessions`, `session_parent` |
| `agent/clients/cache_monitor.rs` | 2 | `warm_len`, `with_prefix_hash` |
| `user_question.rs` | 1 | `has_user_question_sender` |
| `tools/sourcehound.rs` | 1 | `scrape_url` |
| `tools/session_search_tool.rs` | 1 | `register_session_search_tool` |
| `tools/harness_tools.rs` | 1 | `with_builders` |
| `sourcehound_cdp.rs` | 1 | `resolve_cdp_ws_url` |
| `skill_usage.rs` | 1 | `all_records_mut` |
| `parser.rs` | 1 | `is_in_tool_call` |
| `org/seat_authority.rs` | 1 | `approver_of` |
| `oauth_refresh.rs` | 1 | `with_auth_store_path` |
| `model_roles.rs` | 1 | `single_model` |
| `memory.rs` | 1 | `resolve_memory_provider` |
| `mcp_oauth.rs` | 1 | `tokens_file_mtime_ns` |
| `kanban/mod.rs` | 1 | `from_app_config` |
| `historian.rs` | 1 | `historian_system_prompt` |
| `harness_adapters.rs` | 1 | `with_pool_factory` |
| `gateway/whatsapp.rs` | 1 | `with_verify_token` |
| `gateway/webhook.rs` | 1 | `with_holon` |
| `gateway_session.rs` | 1 | `is_session_expired` |
| `gateway/mod.rs` | 1 | `with_delivery_ledger_capacity` |
| `gateway/dispatch_breaker.rs` | 1 | `from_str_key` |
| `estop.rs` | 1 | `sentinel_path` |
| `daemon_pool.rs` | 1 | `spawn_anonymous` |
| `credential_pool.rs` | 1 | `refresh_oauth_async` |
| `context_window.rs` | 1 | `output_ceiling` |
| `config_heal.rs` | 1 | `render_parked` |
| `agent/retry_policy.rs` | 1 | `is_announced` |
| `agent/insights.rs` | 1 | `format_terminal` |
| `agent/clients/anthropic.rs` | 1 | `with_base_url` |
| `agent/builders.rs` | 1 | `list_subagents` |
| `accessibility.rs` | 1 | `has_text` |

### One acid test already run, as the method's calibration

At `73088cab`, `all_records_mut` (`skill_usage.rs`) was acid-tested by hand: removing
`pub` compiles clean (`cargo check -p operant-core --lib` exits 0 with one
`dead_code` warning). **It is dead.** This is the shape every candidate must be
subjected to before deletion, and the reason the plan groups them rather than batching.

## The items, in acid-cost order

### T1 -- the single-candidate files (11 candidates across 11 files)

The cheapest group: one file, one candidate, one acid test. Deleting one function from a
file that has exactly one is a self-contained change with an obvious blast radius.

`output_ceiling` (`context_window.rs`), `sentinel_path` (`estop.rs`), `render_parked`
(`config_heal.rs`), `is_announced` (`agent/retry_policy.rs`), `format_terminal`
(`agent/insights.rs`), `list_subagents` (`agent/builders.rs`), `single_model`
(`model_roles.rs`), `resolve_memory_provider` (`memory.rs`), `historian_system_prompt`
(`historian.rs`), `has_text` (`accessibility.rs`), `spawn_anonymous` (`daemon_pool.rs`).
(`parser.rs`'s `is_in_tool_call` is the 12th single-candidate file but sits in T5 because
`parser.rs` is a high-traffic file on the input path and deserves its own acid test.)

**Acid-test each one individually** (comment out `pub`, `cargo check -p operant-core
--lib`). Any that compiles clean is deleted. Any that does NOT compile has a caller the
census missed -- that candidate is **recorded in `EXCLUDED_SHAPES` with the call site**,
not deleted and not left in the queue.

**Guard:** the auditor's `--check` count drops by exactly the number deleted (11 minus
however many turn out to be live), and `cargo test -p operant-core --lib` stays at the
same pass count.

### T2 -- the `tools.rs` pair and the `oauth/store.rs` pair (4 candidates)

Two files, two candidates each. `tools.rs` is the tool registry -- the highest-traffic
file in the crate -- so `enable_tool` / `is_direct` are the two candidates most likely to
have a caller the census missed (a trait method or a `matches!` arm). Acid-test first;
expect at least one to be live.

`oauth/store.rs`'s `with_default_path` / `with_lifetime` are builder methods on a store
type; builders are the documented false-positive shape in the auditor's own `EXCLUDED_SHAPES`
(`with_token` was a false positive in the first draft). **Assume these are live until the
acid test says otherwise.**

**Guard:** same as T1. If a builder turns out to be dead, delete it; if live, record the
call site in `EXCLUDED_SHAPES`.

### T3 -- the three `skills_guard.rs` candidates (3)

`determine_trust`, `format_report`, `structural_check`. A security-scanning module whose
public surface has no production caller is a real finding (the module's own doc may claim
a `skills_guard` integration that does not exist -- the same class as the
"False claim" rows in the ledger). Acid-test each; if all three are dead, the module's
public surface is entirely unreachable and its doc header must be corrected in the same
change (the iter-2727 / iter-2734 lesson: a doc that claims a live consumer that does not
exist is more expensive than the dead code).

**Guard:** `cargo test -p operant-core --lib` (the module's own tests keep passing), plus
a doc correction if the header claims a caller.

### T4 -- the `gateway/*` candidates (5)

`with_verify_token` (`gateway/whatsapp.rs`), `with_holon` (`gateway/webhook.rs`),
`is_session_expired` (`gateway_session.rs`), `with_delivery_ledger_capacity`
(`gateway/mod.rs`), `from_str_key` (`gateway/dispatch_breaker.rs`).

The gateway is the most peer-contended area in the tree (seven platforms, three live
process-architecture plans). **Check `git log origin/main --oneline -- <file>` and the
workflow board before touching any of these** -- a candidate may be a peer's in-flight
work, and fleet rule 7 says prove your delta adds zero rather than deleting their slice.
If a peer owns the file, skip it and record why in this plan's commit.

**Guard:** the acid test plus `cargo check -p operant-cli --bin operant` (the gateway
lives in the CLI's compile unit too).

### T5 -- the remaining `tests=0` candidates (19)

The leftovers across `database.rs` (`child_sessions`, `session_parent`), `trajectory.rs`
(`set_success`, `to_json_compact`), `cache_monitor.rs` (`warm_len`,
`with_prefix_hash`), `user_question.rs` (`has_user_question_sender`), `sourcehound.rs`
(`scrape_url`), `session_search_tool.rs` (`register_session_search_tool`),
`harness_tools.rs` (`with_builders`), `sourcehound_cdp.rs` (`resolve_cdp_ws_url`),
`skill_usage.rs` (`all_records_mut` -- already acid-proven dead), `seat_authority.rs`
(`approver_of`), `oauth_refresh.rs` (`with_auth_store_path`), `mcp_oauth.rs`
(`tokens_file_mtime_ns`), `kanban/mod.rs` (`from_app_config`),
`credential_pool.rs` (`refresh_oauth_async`), `harness_adapters.rs`
(`with_pool_factory`).

Same method, same guard. This group is the long tail; it can split across two iterations
if the acid tests turn up more live candidates than dead ones.

### T6 -- the 87 tested candidates (NOT this plan's)

The other 87 candidates have tests (`tests>0`). A tested-but-unwired function is a
different question: it is either a real feature gap (RT-275's territory) or a
per-crate-public API (an `operant-core` function that only `operant-cli` calls -- which
the census does count, since it scans the whole workspace, so these are genuinely
uncalled). **They are out of scope here.** This plan closes the `tests=0` half (43) and
leaves the 87 for a successor, because mixing "dead, delete it" with "tested, decide its
disposition" in one tranche is how a delete-the-wrong-thing incident happens.

## What this plan does NOT do

- **No operator ruling required.** Every item is a code change with a falsifier.
- **No design-preference change.** Phonon, memory-wire, sourcehound, the 7 gateway
  platforms, tiered eviction -- untouched.
- **No new surfaces.** The fixes delete or account for existing surface.
- **Not the 87 tested candidates** (see T6).
- **Not RT-190 / RT-280 / RT-543.** All three are owner-gated ledger rows; RT-543's
  sharing half is pinned (iter-2746) and only its DISPOSITION half waits on the operator.
- **Not the 5 `tools::kernel::*` test failures.** They pass in the main tree (fleet rule
  6); a fresh worktree lacking the submodule and `.venv` is not a defect.
- **Not W014 (asks Q4).** An operator decision on ~120 lines of input-path terminal
  probing. Surface it; do not withdraw it on the operator's behalf.

## Execution order

```
1. T1  (11 single-candidate files)      -> verify: --check count drops by the deleted count
2. T2  (tools.rs + oauth/store.rs)      -> verify: builders either deleted or EXCLUDED_SHAPES
3. T3  (skills_guard.rs)                -> verify: module doc corrected if surface is dead
4. T4  (gateway/*, peer-checked)        -> verify: cargo check -p operant-cli too
5. T5  (remaining tests=0 tail)         -> verify: --check count + lib suite pass count
```

T1 first: one file per acid test, the cheapest possible proof that the method works.
T4 is deliberately after T1-T3 because it needs the peer-ownership check, not because it
is harder. T5 last as the long tail.

## Definition of done

- `python3 scripts/audit-pub-fn-surface.py --check` exits 0, or the remaining
  candidates are all `ACCOUNTED_FOR` with a recorded reason.
- Every deletion is acid-proven (comment out `pub`, `cargo check -p operant-core --lib`
  compiles clean) and cited in its commit body with the test that would have caught the
  function being live.
- `cargo test -p operant-core --lib` pass count unchanged (3689 at base, modulo the
  kernel-sidecar 5).
- `bash scripts/clippy-warning-gate.sh` stays green (no new warnings from the deletions).
- `./scripts/genome lint` stays green (0 errors).
- Each item is one iteration: commit + push + deploy, label re-derived after
  `git fetch origin` with a `git rev-list --left-right --count origin/main...HEAD` check.

## Cost

Five iterations, all deletions or `EXCLUDED_SHAPES` additions in `operant-core` -- **no
new code, no new dependencies**. T1 is the bulk of the win (11 of 43); T2-T5 split the
rest. Each iteration is a `cargo check -p operant-core --lib` (~3 min cold, seconds warm)
plus the lib suite.
