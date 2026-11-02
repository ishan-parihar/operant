# ponytail-audit — the double-Esc rewind selector (iters 1633/1637/1657)

Scope: `crates/operant-cli/src/tui/rewind_selector.rs` (852 lines),
`rewind_marks.rs` (361), the `key_handling.rs` seam, and the tests the three
commits added. Over-engineering and complexity only; correctness, security and
performance are out of scope by the skill's own contract.

## What the three commits actually cost

| commit | files | +lines | -lines |
|---|---|---|---|
| d14b29dc (iter-1633) | tests.rs, rewind_selector.rs | 147 | 12 |
| 33552dfd (iter-1637) | key_handling.rs, tests.rs, rewind_selector.rs | 199 | 7 |
| 16168920 (iter-1657) | rewind.rs test only | 128 | 0 |

Net +455. Audited line by line against the five tags.

## Findings

`shrink` `filtered_indices` is a pass-through to `matching_indices` and is kept
only for its name and test surface. One production caller
(`turn_state.rs:149`); 14 test sites. Not worth the churn of renaming 15 sites.
[rewind_selector.rs:223]

`shrink` the `step` body is 60% comment (29 of 48 lines). The comment is the
*incident record* — it names the exact clamping arithmetic that made one Down
press jump to the bottom. This repo's convention (`//` explains why and cites
the incident) says keep it. The 21 code lines are the same 8 lines of logic omp
ships plus the Rust-side `Option`/iterator plumbing. [rewind_selector.rs:169]

`delete` nothing. No new file, no new module, no new trait, no factory with one
product, no dead flag, no config nobody sets. All three commits added code to
two existing files and one test file.

`yagni` nothing. Zero new abstractions; the two index helpers have two and one
production callers respectively.

`native`/`stdlib` nothing. The filter is `to_lowercase` + `contains`, which is
the standard library doing it.

`delete` zero `unwrap()`/`expect()`/`panic!` in the selector — the clippy gate
is satisfied genuinely, not by an allowlist escape.

## Verdict

`net: -0 lines, -0 deps possible.`

The two `shrink` findings are documentation of the shape, not actionable cuts:
renaming `filtered_indices` touches 15 call sites for a shorter name, and
trimming the `step` comment would delete the reason the next agent does not
reintroduce the position-translation bug. 160 of the 199 added lines in
iter-1637 are the mutation-proven tests that caught the Down-jump.

**Lean already. Ship.**

## Correction to iter-1633's reachability claim (supersedes its commit message)

`d14b29dc`'s message says a compaction "folds a prompt [and] desyncs the mark
list", and that a desync lets "Enter refuse". iter-1657 measured the real
compaction path and that wording is **overstated**: no production path ever
shrinks either list independently, so the mark list and the display list stay
index-aligned.

| path | effect on row count |
|---|---|
| `micro_compact` (`agent/events.rs:316`, `agent/run.rs:2761`) | replaces each folded exchange with one `[compacted exchange ...]` marker IN PLACE |
| `evict_to_budget` (`agent/run.rs:2976`) | operates on the wire copy `messages`, never written back to `self.conversation` |
| `conv.clear()` (`agent/events.rs:385`) | `/new` — whole-session wipe; re-arms both lists together |
| `App.messages.clear()` (`tui/app/mod.rs:1560`) | session switch — rebuilds from the new session |

So the desync that DOES occur is **text-absence**, not positional: the folded
turn's mark keeps the original prompt while the display row becomes the marker,
which is what sets `compacted: true` (`rewind_selector.rs:107`) and makes Enter
refuse. iter-1657's fixture reproduces that real mechanism (padded prompts
recorded exactly as `record_user_turn` stores them), not a synthetic one.

The fix and its tests stand; only the narrative about how an operator reaches
it is corrected here. Append-only: this audit file carries the correction
rather than rewriting the published commit.
