# close the residual implementation gaps: the pub-fn queue, the keybinding dispatcher, and the stale ledger residuals

Dated-frozen: published `2026-11-21`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Why this plan exists

The previous plan (`2026-11-20-close-the-supply-chain-gate-the-pub-fn-tail-and-the-residual`)
closed the supply-chain gate and zeroed the clippy gate. What it explicitly did **not**
close is the residual: the acid-test pub-fn queue, the keybinding dispatcher, and the
ledger rows that are stale rather than open. This plan does that work, and it exists
because a survey run on 2026-11-20 at `265b5ccb` (iter-2777) found the following,
each measured rather than assumed:

| survey | result at `265b5ccb` | what it means |
|---|---|---|
| `scripts/audit-pub-fn-surface.py` | **100 NEW candidates** (was 26 remaining after the peer campaign) | the queue REFILLED. 2115 pub fns, 103 candidates, 3 accounted, 100 new |
| `scripts/audit-lint-suppressions.py` | 112 suppressions, 122 baseline entries, 3 "unexplained" | 3 are inline-reasoned `allow`s the scanner does not read, 13 are `allow`-where-`expect`-would-work |
| `scripts/audit-config-keys.py --check` | 77 keys, 6 inherited gaps, **stale baseline entries** | 195 keys never reach a second surface |
| `scripts/audit-config-surface.py --check` | 1 candidate, 1 accounted, **0 new** | clean |
| `scripts/genome doctor` | 1 finding (peer's generated-header rotation) | not content loss |
| `scripts/genome lint` | 0 errors, 1 warning (W014) | W014 is owner-gated |
| `crates/operant-cli/src/tui/keybindings/` | registry exists, is consumed by settings/providers, **but `key_handling.rs` never consults it** | the Pending row's diagnosis holds |
| `TODO.md` session-hub residual | claims `Database::init` per open + "plan 2-5" selector items | **both halves are stale** (see T4) |

## What is NOT in this plan (read this first, it is the cheapest way to avoid duplicate work)

- **The `libsqlite3-sys` major bump.** RT-616 already triaged it with the blast radius
  measured (99 `rusqlite::` sites, 3 crates). It is scheduled on its own merit.
- **The 15 wasmtime advisories.** Locked by `extism 1.41.0`, its own latest release;
  reachability measured in iter-2763 (p1-only features, no threads/gc paths in play,
  the Windows-only and aarch64-only entries are platform-gated out). Already allowlisted
  with a reason per entry.
- **W014 / the four ASKS rows** (`Q4` terminal enhanced-keys, `Q5` RT-179, `Q6`
  RT-285/293, and the round-4 RT-190/280/543 rulings). These are owner product
  decisions, not code repairs. They sit in `## Review`, which autonomous mode never
  touches. Do not re-open them.
- **Any TUI feature audit.** `crates/operant-cli/tests/tui_scenarios/command_sweep.py`
  already exists and enumerates every registered slash command through the headless
  simulator; it needs a debug build to run. Running it is a verification step, not a
  gap.
- **The `#![deny(missing_docs)]` or `#![warn(missing_docs)]` lints.** Not present,
  not requested, and a lint that fails the build on every new field is a different
  project's decision.

## The gaps, and what closes each

### T1 — the pub-fn queue (100 candidates, ~8 iterations)

**PEER COORDINATION, checked 2026-11-20 at 05:34 while this plan was being written.**
A concurrent agent holds an UNCOMMITTED `residual-gaps-2778` workflow board in the
shared main tree whose T1 rows are the same pub-fn triage (batched by `tests=1`,
`tests=2-3`, `database.rs`'s 11 candidates, `gateway_session.rs` + `credential_pool.rs`,
and the remaining ~20 single-candidate files). That agent is ALSO mid-edit on
`crates/operant-core/src/database.rs` (+72 lines: RT-554 claim-time interrupted-session
marking and its tests). **Therefore `database.rs` is NOT this plan's T1 territory** --
its candidates (`add_session_tag`, `get_session_tags`, `insert_compartment`,
`is_compression_locked`, `get_recent_sessions`) belong to the board already in that
file. Work this plan's T1 on the OTHER files only, and re-check the peer board before
starting: `cat docs/kb/workflows/residual-gaps-2778.md` in the main tree.

**Measured.** `scripts/audit-pub-fn-surface.py` reports 2115 production pub fns in
`operant-core`, 103 with zero production callers, of which 3 are already accounted for
and **100 are new**. The top of the queue by test-only caller count:

```
roster_path_in              tests=15   tools/cli_roster.rs
add_session_tag             tests=12   database.rs
plan_span                   tests=10   historian.rs
canonical_name              tests=9    tools/tool_search/name_recovery.rs
validate_span               tests=8    historian.rs
parse_compartment           tests=7    historian.rs
new_turn                    tests=7    tools/checkpoint_tool.rs
get_session_tags            tests=7    database.rs
clear_active_memory_provider_for_tests  tests=7  tools/memory_tools.rs
turn_interceptor_count      tests=6    tools.rs
```

**The rule that keeps this honest: the acid test, not the count.** A candidate is a
`pub fn` with no production caller. The verdict is NOT the audit script -- it is:

1. comment out the `pub` (making it private),
2. `cargo check -p operant-core --lib`,
3. restore.

If the check passes, the `pub` is dead and the deletion is safe. If it fails, the
function is used from outside the crate's public surface or the removal breaks an
adjacent test, and it stays with a note. **The script's 103 is a queue, not a
verdict** -- exactly as its own header says.

**Three exclusions the header already names, restated because they are the
easy way to get this wrong:**

- `#[cfg(test)]`-only helpers that are `pub` so a sibling test module can reach them.
  The `pub` is load-bearing even though production never calls them. Deleting them
  breaks the test target and the gate (which compiles `--all-targets`).
- Functions re-exported through a `pub use` shim, or called from a doc example.
- Names defined more than once (142 of them) -- the script skips them because it cannot
  attribute a caller to one definition.

**Order:** work top-down by test count. High test count with zero production callers
is the strongest signal (many tests exercise it, nothing ships it). Each deletion is
one iteration: acid-test, delete, run `cargo check -p operant-core --lib` and
`cargo test -p operant-core --lib`, add the name to
`crates/operant-core/tests/dead_pub_fns_stay_dead.rs` (RED-prove it first), commit,
push.

**Falsifier:** the queue must reach 0 NEW. If an acid test fails on a candidate, it is
recorded as "kept, reason" and removed from the queue -- the count going down without
a deletion is a legitimate outcome, but the reason must be in the commit body.

### T2 — the keybinding dispatcher (~2-3 iterations)

**Measured.** `crates/operant-cli/src/tui/keybindings/` exists (`defaults.rs`,
`registry.rs`, `report.rs`, `tests.rs`). `registry.rs` is consumed by
`tui/app/providers.rs` and `tui/settings_screen/tabs.rs` -- so the catalogue is real
and drives the settings surface. But `tui/app/key_handling.rs` (the dispatcher, the
repo's most contended file) never calls into it: `grep -c 'keybindings::' key_handling.rs`
returns 1, and that one hit is a comment.

So `[keybindings]` overrides in `operant.toml` are read by the catalogue and ignored
by the dispatch. `operant tui keybindings` prints the bindings in effect (iter-1133),
which is honest, but the override the operator writes does nothing.

**The gap is the dispatch seam, not the parser.** The registry parses and validates;
what is missing is a single choke point where the raw key event is resolved against
the effective binding before the if-chain runs. Two shapes were considered:

1. **Resolve-then-dispatch.** `key_handling.rs` asks the registry for the action
   bound to `(key, modifiers, context)` and runs that action; the if-chain becomes the
   registry's `Default` impl, not a second copy.
2. **Dispatch-then-annotate.** The if-chain stays; after it runs, a post-pass checks
   whether the operator's overrides would have bound a different action and warns.

(1) is correct and is the one this plan takes. (2) cannot actually override anything --
it can only warn -- so it does not close the gap, it documents it.

**Why this is scheduled at all, when the prior campaign refused it.** The 2026-10-29
remaining-gaps plan declined it as "a dispatcher refactor, not a feature". That
judgment still stands, but it was made when `key_handling.rs` was "the peer's live WIP
for days". It is no longer contended. The refactor is bounded (one function, one
registry lookup) and the tests already exist in `keybindings/tests.rs` -- the work is
wiring, not writing.

**Scope boundary:** `[keybindings]` overrides and the registry's `BindingContext`
gating. NOT: new bindings, chord changes, or the `vim.rs` vs `word_nav.rs` motion
split (those are design decisions).

**Falsifier:** `operant tui keybindings` output must equal the dispatch behaviour for
every binding the operator can set. A test that sets an override, runs the dispatch,
and asserts the overridden action fired is the acceptance test.

### T3 — the lint-suppression ledger (1 iteration)

**Measured.** 112 suppressions across 52 files; the baseline holds 122 entries; the
scanner flags 3 as "unexplained" and 13 as `allow`-where-`expect`-would-work.

- The 3 "unexplained" are **false positives in the scanner**: each carries an inline
  `reason` and a `#`-anchored doc comment the scanner's regex does not read
  (`commands.rs:366` on `build_command_map` -- mine from iter-2776; `settings_screen.rs:109`
  on `SettingStore::store`; `subagent_navigator.rs:42` on `SubagentEntry::parent`).
  **Fix the scanner**, not the code: the header comment says "read back from the `#\[`",
  and the escaping there is a Python `SyntaxWarning` (`"[\" is an invalid escape
  sequence`") which means the regex has been silently not matching. Add a baseline
  entry per site OR teach the scanner to read the reason. Either closes it; the second
  is better because it also catches the next one.
- The 13 conversion candidates (`allow` -> `expect`) are the interesting half: an
  `allow` that never expires hides a lint that stopped firing. `agent/run.rs:1391`
  and `:2582` (`unused_assignments`) are the two worth doing first -- an `unused
  assignment` on a `mut` binding is a code smell the `allow` is masking. The rest
  (`dead_code` on `operant-harness` rows and example binaries) are lower value.

**Falsifier:** `audit-lint-suppressions.py` reports 0 unexplained and the conversion
count drops. The allowlist in `.ci/` is untouched.

### T4 — close the stale ledger residuals (1 iteration, docs-only)

**Measured.** Two `TODO.md` residuals name work that has already landed:

1. **Session-hub `Database::init` per open + "plan 2-5" selector items.** Measured at
   `265b5ccb`: `grep -rn 'Database::init' crates/operant-cli/src/tui/session_hub/`
   returns one hit, and it is in `tests.rs`. The hub's `open()` resets state and calls
   `rebuild_visible()`; data arrives through the cached-then-refresh mpsc pattern
   described in the module header. There is no per-open `Database::init` to remove. The
   "plan 2-5" selector items (`sibling-branch`, `prompt-text-filter`, `tail-cap`) do
   not exist in the hub: `grep -rn 'tail_cap\|prompt-text-filter'` over `tui/` returns
   nothing, and the 2026-11-14 session-hub plan records all seven operator items
   **closed** (iter-2317 through iter-2351, "139 session-hub tests pass"). The residual
   was written against an older plan numbering and never re-derived.

2. **Settings screen persisting to legacy `settings.json`.** Measured: the screen now
   calls `operant_core::config::persist_config_key_at(path, dotted, value)` per key,
   which writes `operant.toml` through `toml_edit` under a `ConfigWriteLock`
   (added iter-2645). The config-key half of the residual is closed. **The
   `settings.json` half is still true and still open**: `settings_screen/keys.rs:127`
   and `state.rs:355` both state that the TUI's OWN visual state (theme, cursor blink,
   reduce-motion) lives in `~/.operant/settings.json` and must not leak into a shared
   `operant.toml`. That is a deliberate boundary, not a defect -- but the TODO row
   conflates the two halves and should be split.

**The action:** rewrite both residuals to say what is measured (half closed, half
deliberate), move the settings.json half into `## Review` as a design question
(does the TUI's visual state belong in `settings.json`, or in `[tui]` with a
documented migration?), and drop the rest.

**Falsifier:** a fresh read of `TODO.md` names no residual that `grep` contradicts.

### T5 — the 195 unreached config keys (1 iteration, survey only)

**Measured.** `audit-config-keys.py --check` reports 77 keys in the current change and
"-195 of them reach all four surfaces", with 6 inherited gaps:
`browser`, `checkpoints`, `database_path`, `moa`, `skills`, `terminal_backend`, each
missing one or more of `note` / `heal` / `template` / `doctor`.

The audit's own summary says these are **inherited, not this change's to fix**. This
plan does not fix them. It does the cheaper thing: enumerate them into one table in
`TODO.md` under `## Pending` so a future agent can pick a surface and work it, and
record which of the six have a `doctor` gap (those are the ones that can silently
misconfigure an operator) versus a `template` gap (documentation only).

**Falsifier:** the six keys appear in `TODO.md` with their missing-surface matrix.

## Gates / success criteria

(fixed before the run)

1. `scripts/audit-pub-fn-surface.py` reports **0 NEW** candidates.
2. `crates/operant-core/tests/dead_pub_fns_stay_dead.rs` gains a needle per T1
   deletion, RED-proven before the deletion lands.
3. `bash scripts/clippy-warning-gate.sh` reports `seen=0 allowlist=0 new=0 stale=0`.
4. `bash scripts/supply-chain-gate.sh` reports `new=0`.
5. `python3 scripts/audit-lint-suppressions.py` reports **0 unexplained** and the
   conversion count has dropped by at least the two `agent/run.rs` sites.
6. `cargo check --workspace --all-targets` exits 0.
7. `python3 scripts/audit-ledger-index.py` reports OK.
8. `./scripts/genome lint` reports 0 errors (W014 may remain).
9. `./scripts/genome doctor` reports 0 findings, or each remaining finding names its
   cause as a generated header.
10. Every T1 deletion has a matching test, and `TODO.md` names no residual that a
    fresh `grep` contradicts.

## Cost

| task | rough cost | why |
|---|---|---|
| T1 pub-fn queue | 8-10 iterations | ~100 candidates at ~2 min acid test + 1 min verify + commit overhead |
| T2 keybinding dispatcher | 2-3 iterations | one function, one registry lookup; tests exist |
| T3 lint-suppression ledger | 1 iteration | scanner regex fix + 2 conversions |
| T4 stale residuals | 1 iteration | docs only |
| T5 config-key survey | 1 iteration | enumeration only, no fix |

**Total: 13-16 iterations.** Nothing here needs an operator ruling. The four
`## Review` items stay there.

## Order and dependency

T4 first (it is docs-only and it stops the next agent re-deriving what is already
closed). Then T1, which is the long tail and benefits from starting early. T2 and T3
are independent and can be interleaved. T5 is last because it is a survey whose output
feeds a later plan.

## What this plan deliberately does not do

- **No operator ruling.** The four ASKS rows and the three RT-* checkboxes stay open.
- **No design-preference change.** Phonon, memory-wire, sourcehound, the 7 gateway
  platforms, tiered eviction, `plugins-wasm` default-on -- untouched.
- **No `libsqlite3-sys` bump, no wasmtime patch.** Both already triaged with the
  reason recorded.
- **No new lints.** `#![deny(missing_docs)]` and friends are not this plan's.