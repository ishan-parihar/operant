# Plan — close the residual context-lifecycle gaps

**Date:** 2026-11-15 (revision 1)
**Base:** `origin/main` @ `41d217b1`
**Status:** outline. Writing this file executes nothing.

This plan owns the four items the iter-2384 red-team left open, plus the
verification pass that audit never ran. It is the follow-up to
`iter-2365`…`iter-2384`; the ledger record for that work is in
`_kernel/ledger.md`.

---

## Resume here

Nothing is implemented. Five tranches, in this order, one commit each:

| # | Tranche | Gap | Risk |
|---|---|---|---|
| U1 | recap handle stop is real | 19 | low |
| U2 | one owner for the character cut | 18 | medium |
| U3 | script-aware estimator | 17 | medium |
| U4 | continuation latch follows host capability | 16 | high |
| U5 | the verification the audit never ran | — | none |

Read the tranche's section before starting it: each names its files, its `done
when`, and its rollback. Re-verify against the live tip first — this file is
measured on `41d217b1`.

---

## Status against the four open items

| # | Item | State |
|---|---|---|
| 16 | The advisor's continuation latch suppresses a real user turn on hosts that cannot submit | open — measured at `advisor.rs:742` |
| 17 | The shared estimator is `chars/4`, ~4× low on CJK and ~25% low on code | open — measured at `context_management.rs:41` |
| 18 | Five tool-result truncators restate the char/byte contract | open — five sites, listed below |
| 19 | Dropping the recap handle does not stop the engine, contrary to its doc | open — measured at `recap.rs:181` |
| — | Full-workspace gate, a live continuation probe, a CJK fixture corpus | open — the audit's `Not checked` list |

Nothing in this file is implemented. Each tranche names its own `done when`, and
each is meant to land as its own commit so it can be reverted alone.

---

## Headline finding

**The four items are not one kind of work, and the cheapest-looking one is not
cheap.**

- 16 is a **semantic** change: it alters when a review is skipped, so the
  operator can see the difference. It goes last, with the most evidence.
- 17 is **one function body**. `estimate_tokens` has 50 call sites in
  `operant-core`; fixing the body fixes all of them with one proof, which is why
  the fix is not a signature change.
- 18 is **deduplication with a contract**: one of the five (`truncate_head_tail`)
  already carries the correct character-based arithmetic, and the defect class is
  that the other four restate it. RT-465 already bit one of them.
- 19 is **an honest surface**: the doc claims a stop that does not exist.

### The revision corrected a frozen plan

`docs/plans/plan-2026-11-12-rtk-token-efficient-tool-loop-integration.md` records
at :174 and :409 that a vendored BPE tokenizer (`crates/aft-tokenizer`) is
"already vendored" and "strictly better" than the heuristic. **No such crate
exists in this tree** — `find . -name "*tokenizer*"` is empty, no `Cargo.toml`
declares one, and the path has no history. So gap 17 is not the cheap port that
plan implies: either vendor a tokenizer (a dependency, a data file, a build
cost) or fix the estimate without one. This plan prices both and recommends the
second, with the trigger that would reopen it.

---

## U1 — make the recap handle's stop real

**Gap:** 19. **Risk:** low. **Files:** `crates/operant-core/src/recap.rs`.

**Measured:** `spawn_recap_engine` returns `RecapActivity { tx: Some(act_tx) }`
and its doc says "Dropping it stops the engine" (`recap.rs:181`). Dropping the
handle drops the activity sender, so the `Some(()) = act_rx.recv()` arm in the
`tokio::select!` fails and the `else => continue` arm re-arms the loop — the
engine keeps firing recaps on every turn end until the *bus* closes. Dropping
the handle disables cancellation; it does not stop anything.

**Why it matters:** `chat_non_tui` holds the handle as `_recap_activity` and
returns while the engine may be mid-fire. The process then exits under the
engine, so a recap print can land after the command's own output, or the side
call is aborted at teardown with no log. Both are invisible in every other host.

**The decision:** hold the task's `JoinHandle` beside the sender in a struct with
a `Drop` that aborts, so the doc's sentence becomes true rather than being
deleted. The rejected alternative — correct the doc to "dropping disables
cancellation" — is cheaper but leaves no host able to stop the engine, and the
headless exit path is exactly the caller that needs one.

**Done when:** a test spawns the engine, drops the handle, and asserts the task
completes (an abort is observable as the task finishing within a bounded wait)
where before it stayed pending; and `chat_non_tui`'s exit path no longer has a
detached writer.

**Rollback:** revert this commit alone — the handle shape is new, so the previous
bytes restore exactly.

---

## U2 — one place that knows the character contract

**Gap:** 18. **Risk:** medium (behaviour must not change). **Files:**
`crates/operant-core/src/agent/compress.rs`, `.../agent/llm_compressor.rs`,
`.../agent/mod.rs`, `.../context_management.rs`.

**Measured, five sites and what each actually is:**

| Site | Unit | Job |
|---|---|---|
| `context_management.rs:1901` `guard_tool_result` | tokens | per-result share: withhold bulk, keep a preview |
| `context_management.rs:139` `trim_tool_results_to_share` | tokens | aggregate share across results |
| `compress.rs:190` `truncate_head_tail` | **chars** | the primitive: head+tail cut with a stated marker |
| `llm_compressor.rs:631` `prune_tool_results` | chars | keep recent, clip old |
| `agent/mod.rs:1311` `truncate_tool_result_capped` | **bytes** | JSON-aware field stripping, then cut |

**The defect is not that there are five.** They are five different policies —
per-result, aggregate, primitive, recency, JSON-aware — and collapsing them would
lose four. The defect is that `truncate_tool_result_capped` still measures in
**bytes** (`content.len()`, `serialized.len()`) while its sibling already learned
the lesson in RT-465, and each policy restates the cut arithmetic rather than
calling the one primitive that has it right.

**The decision:** make `truncate_head_tail` the single owner of the character
cut, have the other four call it for the cut step while keeping their own
policy, and add a table test that runs a CJK and a code fixture through all five
asserting each respects **its own documented unit** — a character budget is a
character count, not a byte count. Behaviour is preserved for Latin text by
construction; the CJK case is where the byte-based one is wrong today.

**Done when:** the table test passes with the CJK fixture keeping at most `cap`
*characters* through all five sites, and a byte-for-byte output comparison over
an ASCII corpus shows no change from the current behaviour.

**Rollback:** revert this commit alone. If a policy turns out not to fit the
primitive, that policy keeps its own cut and the plan records it as a named
exception rather than forcing the call.

---

## U3 — one estimator, correct on every script

**Gap:** 17. **Risk:** medium (it moves a budgeting number every turn).
**Files:** `crates/operant-core/src/context_management.rs` (one function body).

**Measured:** `estimate_tokens` is `text.chars().count().div_ceil(4)`
(`context_management.rs:41`) with **50 call sites** in `operant-core` — the fold
target, the decay curve, the recap budget, the compression threshold, the turn
accounting. Its own comment says CJK is undercounted; it also says the heuristic
"overestimates for English", which is wrong (English is ~4 chars/token, so
`chars/4` is about right there; the real errors are CJK at ~4× low and code at
~25% low).

**Why the body, not the signature:** 50 callers is the whole argument. The
project skill's own rule — count the implementors, then fix inside the body they
all call — applies directly: a signature change lands at 50 sites and needs 50
re-verifications, while one body change is proved by one test.

**The decision:** a script-aware piecewise estimate in the body — count
characters by class (CJK, other non-ASCII, ASCII) and apply a per-class
chars-per-token ratio — with the constants carried as named pairs beside the
function so they are falsifiable rather than buried. The rejected alternative is
vendoring a real BPE tokenizer: it is exact, and it is a dependency, a data file
and a build cost, for a number whose only job is to decide *which rung* of the
ladder fires. **Trigger that would reopen this choice:** a request that fails
with `context_length_exceeded` while the estimate says it fits, on a route where
the piecewise error bound is the explanation.

**Ground truth and control:** the piecewise constants are calibrated against
published chars-per-token ratios for the three classes, and the control
experiment is the error bound — a fixture corpus (Latin prose, Rust source, CJK
prose, mixed) where the estimate must land within a stated percentage of the
reference ratio. The plan records the bound; a bound that cannot be met reopens
the tokenizer choice rather than being quietly widened.

**Done when:** a fixture corpus over the four classes asserts the estimate is
within the stated bound of the reference ratio for each class, and the existing
budget tests still pass unchanged on ASCII input (no behaviour change where the
old heuristic was already right).

**Rollback:** revert this commit alone — the constants and the body are new, and
the previous heuristic is one line to restore.

---

## U4 — the continuation latch follows the host's real capability

**Gap:** 16. **Risk:** high (it changes when a review is skipped). **Files:**
`crates/operant-core/src/advisor.rs`, `.../agent/builders.rs`,
`crates/operant-cli/src/gateway_runner.rs`, `crates/operant-cli/src/main.rs`,
`crates/operant-cli/src/tui/adapter_types/tui_app.rs`.

**Measured:** on every `should_continue` review the engine does
`continuations_outstanding.fetch_add(1)` (`advisor.rs:742`); at the next turn end
the counter is loaded, decremented, and the delta is **skipped unreviewed**
(`advisor.rs:595-610`) with only a `debug!` line. That is correct on the TUI,
which takes the batch at `tui_app.rs:707` and submits it. It is wrong on the
gateway and in headless: neither has any submission path (grep for
`pending_advisor_continuation` in those two files returns nothing), so the latched
turn is an ordinary operator message and its review is silently dropped — once per
advisor blocker, alternating forever.

**Why the engine cannot fix this alone:** the latch is set on *emission*, before
any host acts. The engine knows `should_continue`; it cannot know whether the
host will submit.

**The decision:** make the capability explicit at attach time —
`attach_advisor` gains a "this host continues runs" parameter, threaded at all
three construction sites (TUI true, gateway false, headless false), and the
engine latches only when the host both wants to continue and can. The rejected
alternatives: (a) a host ack after the fact — truthful but a second message on
the wire for a fact known at construction; (b) leave the counter and drop the
`by_counter` arm on non-submitting hosts — the counter then means nothing and the
tag remains the only signal, which is the fragile half the counter was added to
replace.

**Why this is a named parameter and not a config key:** it is a property of the
host loop, not an operator preference. A knob would let an operator claim a
capability the host does not have.

**Done when:** a gateway-shaped test (no submission) emits a blocker review,
then a fresh user turn, and asserts the fresh turn **is** reviewed; and the TUI
test asserts the continuation turn is still skipped. The two together are the
guard: one proves the latch still works where it should, the other that it stops
firing where it cannot.

**Rollback:** revert this commit alone; the parameter is new at every site, so
the previous bytes compile and behave exactly as before.

---

## U5 — the verification the audit never ran

**Gap:** the `Not checked` list. **Risk:** none (no behaviour change).
**Files:** none — this tranche runs gates and records results.

The iter-2384 report named five things it did not examine. Each becomes a check
with a recorded result, not a claim:

1. **Full workspace** — `cargo test --workspace` (the audit ran core + CLI only).
2. **A live continuation probe** — confirm on the deployed binary that a gateway
   session with `[advisor] enabled = true` produces a card and **no** phantom
   continuation warning, and that the headless host prints a card only.
3. **The CJK corpus as a permanent fixture** — promote U3's fixture corpus from a
   test-local constant to a fixture the budget tests read, so the estimator's
   bound is checked on every run rather than once.
4. **A learned-limit probe** — this host has zero recorded `context_limit:*` rows,
   so the ceiling fix is unit-verified only. Record that as a known-unverified
   path with the condition that would verify it (a real provider overflow).
5. **The interactive TUI** — no interactive run was performed; record it as
   unverified rather than inferred from tests.

**Done when:** each of the five has a recorded result (pass, or
not-verifiable-here with the reason), and the workspace suite's failures are
triaged by owner — this tranche reports a number, it does not assert green.

**Rollback:** nothing to revert; this tranche records evidence only.

---

## Standing rules for every tranche

From the iteration protocol and the last audit's own corrections:

1. **One tranche per commit**, and its `done when` must be checkable at that
   commit — a tranche sharing a commit cannot be reverted alone.
2. **Rebase onto the live tip before the first edit** of each tranche, and
   re-read the target file after the rebase: a ledger written against
   `41d217b1` stales the moment a peer lands.
3. **A fresh `TMPDIR` per suite run.** Several tests key on
   `std::env::temp_dir()` plus the process id, and a reused scratch directory
   fails unrelated checkpoint and config tests. Every count in the audit was
   taken this way; a count taken otherwise is not comparable.
4. **`cargo fmt --all` reflows unrelated files on this tree** (52 pre-existing
   unformatted files at the tip). Revert every file outside the tranche's own
   set, and verify the tranche's files with `cargo fmt --all -- --check`.
5. **Regenerate `config_registry.rs` with its generator, never by hand**, and
   keep `--check` green. Any tranche that changes a settings doc must end with
   the registry regenerated — a hand-edit is overwritten by the next regen.
6. **Triage a red gate by owner before repairing it.** A failure that reproduces
   on the fetched base is a peer's; repairing published work to green your own
   gate destroys what someone else shipped.

---

## What this plan deliberately does not do

- **No change to the two decay constant sets.** `turn_finalizer`'s preflight pair
  (100/20.0) is deliberately more aggressive than the reactive pair (200/30.0),
  and the earlier audit's "one rule, two values" framing was wrong. They are
  named and documented; they stay.
- **No new config key.** U4's capability is a host property, not an operator
  preference.
- **No tokenizer vendoring.** Named as the trigger-reopened alternative in U3, not
  as work.
- **No change to the recall/batch ladder or the four budget knobs** shipped in
  iter-2365…2370.
- **No repair of the peer-owned failures** in the workspace suite (`tools::kernel`,
  which needs a git submodule this worktree lacks). They are attributed, not
  fixed.

---

## Open question

**Does the estimator need to be exact, or only within a stated bound?**

- **Default if unanswered:** the script-aware piecewise estimate (U3 as written)
  — no dependency, and the bound is recorded and testable. A later
  `context_length_exceeded` that the bound explains reopens the tokenizer choice.

---

## What changes when this is executed

Nothing yet. This file is a plan. Executing it lands five commits — U1 the recap
stop, U2 the truncator contract, U3 the estimator, U4 the continuation latch, U5
the verification record — each with its own `done when`, and updates
`genome.toml`'s `core.living_plans` to declare this file living.