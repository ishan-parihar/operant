# ponytail-audit / red-team of the codex-architecture-deepen implementation

Dated-frozen: `2026-11-01`. Scope as requested: the whole *implementation* of the
deepen rows, not just a diff. Two lenses:

1. **ponytail-audit**, over-engineering, dead code, things to delete.
2. **correctness red-team**, bugs in what shipped.

Both are reported. Everything found was fixed in this file's accompanying commits,
except the four items listed under "NOT fixed here", each with the reason.

## Verdict: the port is honest, and its biggest defect is that row 4 was never wired

Rows 1, 2, 3, 5 shipped the behaviour they claimed. Row 4 did not, in the most
consequential way a port can fail: **it shipped as a dead module.** That single
finding is the reason this audit exists.

---

## CRITICAL: the row-4 engine was an orphan (found and fixed)

**Finding.** `iter-1600` "native patch engine with occurrence counting and
line-ending fidelity" added `crates/operant-core/src/tools/patch/mod.rs` (21 KB,
`OccurrenceRule`, `SourceFile`, `LineEnding`, 18 tests). It was declared
`pub mod patch;` and **never called by anything.** `builtin.rs:146` still
registered the old `PatchTool`, whose `execute` still did
`content.replace(&args.find, &args.replace)` on every occurrence and reported
`replacements: N` with `success: true`.

Measured on the shipping binary before the fix:

```
find "foo" on a file with three "foo" -> {"replacements":3} success=true
file after: qux bar qux baz qux
```

**Why this is the worst possible shape of bug.** The commit body claimed
"a_two_occurrence_find_is_refused_with_the_count_named" and recorded a mutation
guard proving it fails without the gate. That test exists and passes, against
code nothing calls. Every mutation guard in row 4 was green for the same reason
a test of unreachable code is green: it never ran.

**Fix.** The registered tool now consults `patch::OccurrenceRule::resolve` on
both the exact and the fuzzy tier, gained an `occurrence: Option<usize>` argument,
and applies the refusal before any write. A refused patch leaves the file
byte-identical.

**Tests.** `test_patch_multiple_replacements` previously *asserted the defect*
("replace all three, success=true") and now asserts the refusal. Four new tests
pin the refusal, the occurrence index, an out-of-range index, the unique-match
success case, and the fuzzy tier.

**Mutation guards, each observed failing by name before restore:**

| mutation | tests that failed |
|---|---|
| occurrence gate forced to `Unique` (exact tier) | 3, incl. `test_patch_multiple_replacements` |
| occurrence gate forced to `Unique` (fuzzy tier) | 4, incl. `test_patch_fuzzy_ambiguous_is_refused_too` |
| writer forced back to unconditional `"\n"` | 1, `test_patch_fuzzy_preserves_crlf` |

---

## HIGH: row 1's unwrap only descended one level (found and fixed)

`interpreter_script` returns the remainder of a wrapper segment as one flat
string, and that string was then rated as a *command*. So a wrapper sitting in
front of an interpreter rated `None`, which is exactly the hole iter-1595 closed,
reopened by a different shape. Measured before the fix:

```
env VAR=1 sh -c 'rm -rf /'      -> None     (should be Unbounded)
sudo -u root sh -c 'rm -rf /'   -> None     (should be Unbounded)
timeout 5 sh -c 'rm -rf /'      -> None     (should be Unbounded)
bash -c 'sh -c "rm -rf /"'      -> None     (should be Unbounded)
```

All four now rate `Unbounded`. `segments_full` became recursive with a
`MAX_INTERPRETER_DEPTH = 8` cap.

**Guard:** flattening the recursion back to one level fails
`a_wrapper_chain_is_followed_to_the_command_beneath` and
`nested_interpreters_are_followed_all_the_way_down` by name.

---

## HIGH: a bound I added almost shipped as a security hole (found and fixed during this audit)

`classify()` runs on the approval path and the recursion re-tokenizes at every
hop, so its work is superlinear in the input length with **no input cap at all**.
The obvious fix, fall back to the raw `segments()` split past some length, is
wrong: that split's head verb is `bash`, which `rate_segment` does not model, so
`bash -c 'rm -rf /'` would rate `None`. I wrote exactly that first, and the test
I wrote for it caught it (`left: None`).

The bound now emits the fail-closed sentinel instead and rates `Scoped`.

**Guard:** reverting the bound to the surface split fails
`a_command_past_the_unwrap_bound_fails_closed_at_scoped` with `left: None`.

---

## MEDIUM: row 1's `-c` flag handling (found and fixed)

`bash -c -x` consumed `-x` as the *script*, so the real command was never
consulted and the whole thing rated `None`. Now the token after `-c` is taken as
the script only when it is not another flag; otherwise the segment fails closed at
`Scoped`. Pinned by `an_unfollowable_interpreter_flag_fails_closed_at_scoped_not_none`.

---

## MEDIUM: row 4's D2 dead branch and CRLF fidelity (both found and fixed)

1. **D2's dead branch was still there.** The plan said delete
   `patch_tool.rs:95-99`, "received a StepC with one body...". It is still in the
   tree in the live tool: an `if replace.is_empty()` whose body was a comment
   saying "already handled", reached only when a condition that did nothing
   either way was true. Removed now, with a comment naming exactly what it was.

2. **A delete could not delete.** An empty `replace` removed the text but left the
   line behind, so `patch` could blank a line and never remove one. The
   pre-existing `test_patch_delete_content` pinned the resulting `"\n"` as
   correct behaviour, and I corrected the test, because a delete that leaves the
   line is not a delete. Pinned by `test_patch_delete_a_line_with_an_empty_replacement`.

3. **CRLF was destroyed on write.** The writer joined with `"\n"`
   unconditionally, so any fuzzy patch on a CRLF file converted the WHOLE FILE to
   LF: the concrete defect D2's `LineEnding` model exists to fix, which never
   reached the live tool because the module was orphaned. Three tests pin CRLF,
   lone-CR and mixed-ending fidelity at both tiers.

---

## LOW / over-engineering (ponytail-audit lens)

`delete: tools/patch/ is now 21 KB of mostly-dead code. Reusable: SeekSequence tiers, LineEnding, OccurrenceRule. Dead after this fix: the whole test module measures the engine in isolation while the tool now duplicates only the CRLF *writer* rule. [crates/operant-core/src/tools/patch/mod.rs]`

The honest ponytail call is that row 4 should have replaced the tool, not
sit beside it. I did not do that here: it is a larger refactor than a red-team
should perform unattended, and the smaller fix restores the *behaviour* the port
promised. Recorded as a follow-up, not left as a surprise.

`yagni: PatchTool still carries two parallel replace paths (exact-with-index vs fuzzy) that now share one gate. Shrink candidate: fold the fuzzy tier onto the ported engine's tier ladder. [crates/operant-core/src/tools/patch_tool.rs]`

`shrink: blast_radius's segments() allocates a Vec<char> and a String per char for its two-char lookahead. Byte scan + one lookahead index is the shorter form. Not changed: it is pre-existing, it is correct, and it is not on the hot path this audit is about. [crates/operant-core/src/blast_radius.rs]`

---

## NOT fixed here, and why

1. **Deep same-verb nesting rates `None` at depths 2..7.** `bash -c 'bash -c
   "...'` degrades because each hop strips one quote layer. This pre-dates this
   audit (the pre-recursion shape rates identically, measured), and the fix is a
   real shell tokenizer, a much larger row than a red-team should open. The
   bound is what stops it being unbounded work. Recorded in
   `nested_interpreters_are_followed_all_the_way_down`, which asserts not-None
   rather than a rating I cannot back.
2. **The patch engine is still a second implementation.** See the ponytail row
   above. Needs its own iteration.
3. **`profile::tests::test_use_profile`** fails under full-suite parallel load
   and passes in isolation. Not mine (no profile.rs in this diff), same family as
   the known RT-203 loopback flake.
4. **Deploy.** No release build was cut for this iteration.

---

## The gate

`cargo fmt --all`, `cargo test -p operant-core --lib blast_radius` (27 passed),
`cargo test -p operant-core --lib patch` (68 passed), `cargo clippy -p
operant-core --lib` clean for both touched files, `cargo test -p operant-core
--lib` = 2735 passed / 1 failed (the pre-existing profile flake above).

Five mutation guards observed failing BY NAME before restore:
row-1 recursion flattened, exact-tier gate forced open, fuzzy-tier gate forced
open, CRLF writer forced to `"\n"`, bound forced to fail open.

---

# Second pass (2026-11-01, later): the audit of the audit

The first pass above audited rows 1 and 4 and pronounced them fixed. The second
pass re-ran the red-team against the *fixed* code, on the theory that a fix
applied in a hurry is where the next defect lives. It found three, two of them
security-relevant, in the very code the first pass had certified.

**The lesson, stated once:** "the mutation guard is green" and "the behaviour is
correct" are not the same claim. Every defect below sat behind guards that were
still passing.

## Row 1: a destructive command still rated `None` (found and fixed)

**Finding.** `trap 'rm -rf /' EXIT` rated `BlastRadius::None`. So did
`env trap 'rm -rf /' EXIT`.

The first pass made the unwrap recursive and kept its guards green, but the
recursion is only half of the path. `tokens()` parses and STRIPS the quoting, so
`trap 'rm -rf /' EXIT` became three tokens `["trap", "rm -rf /", "EXIT"]` with
the handler already collapsed into one. `rejoin_tail` then re-quoted that token
to keep it a single token through the next `segments()` call, producing
`'rm -rf /' EXIT`. The recursive segmenter re-tokenized that and took the HEAD
TOKEN as the command, but the head token was the whole phrase `rm -rf /`,
which `verb_kind` does not model. One unrated segment, and the whole command
was rated `None` and slipped the gate.

Note the shape of this: it is a *quote round-trip* defect, not a missing pattern.
The first pass's indirection recursion is correct; what was wrong is that
between the two hops, the handler lost the quoting that made it the argument.

**Fix.** `rejoin_tail` marks an already-unquoted first token so it is emitted
VERBATIM instead of re-quoted. `rejoin_tail`'s other job, re-quoting later tokens
so they survive a round-trip, is unchanged.

**How it was found.** A 24-command probe over the interpreter family
(`env`, `sudo`, `timeout`, `doas`, `nice`, `nohup`, `setsid`, `stdbuf`,
`ionice`, `bash -c -x`, `bash --posix -c`, glued `-c<script>`, absolute
`/bin/sh`, and three-level `bash -c 'sh -c "..."'` nesting). Exactly four rated
`None`. Two are pre-existing and by design (a `cat` read, which this tool does
not rate, and `xargs`, whose command arrives on stdin). One is a pre-existing gap
in `VCS_HARD_SUBS` (`git push --force`). The fourth was this defect. The probe
was not kept as a test: it is a scouting instrument, and its three non-defects
would be false guards.

## Row 4: two defects the first pass's guards did not catch (found and fixed)

**D3, the tool lied.** `occurrence: 1` over three matches rewrote one place and
answered `"replacements": 3`. The first pass added the `Chosen` arm that rewrites
only the named occurrence and never revisited the count, which was still
`exact_count`. The model reads that number to reason about what it did; a
number that describes a different operation than the one performed is a
correctness defect in the tool's contract, not a cosmetic one.

**D4, the gate did not gate.** The fuzzy tier checked `OccurrenceRule::resolve`
against the match count and then handed the work to `fuzzy_replace` - which had
no `occurrence` parameter and rewrote every matched block. Measured on the
shipping tree: `occurrence: 1` over two fuzzy matches rewrote BOTH. This is the
exact failure the occurrence gate was added to prevent, alive on the tier that
was supposed to be the safer one, and the first pass's guard for it was green
because the guard tested the gate, not the write.

Two counts, not one, is the actual fix: the GATE must see how many places
matched at all (so a legal "edit the second one" stays selectable), and the tool
must report how many it actually changed. Collapsing them makes a named
occurrence unselectable; conflating them is what let D4 through.

**Fix for both.** `fuzzy_replace` takes the index and honours it, threading the
original lines of non-target blocks through unchanged; the exact tier tracks
`applied` separately from the match count and reports `applied`.

## Row 3: a fourth orphan, and the most expensive kind

**Finding.** `crates/operant-core/src/turn_attribution.rs` (409 LOC,
`TurnSpan`, `TurnBreakdown`, `TurnAttribution`) is exported (`lib.rs:130`) and
called by nothing. Its own module docs call it "the RT-199 fix", which is not
true: RT-199 was in fact closed separately, by
`agent/mod.rs:754`'s `tool_durations_by_call` map that the observer emit now
reads instead of the hardcoded `Duration::from_millis(0)`.

**Why this one costs more than rows 4 and 7.** Rows 4 and 7 were orphans that
cost the operator a *missing improvement*. This one is an orphan that its own
documentation claims is already live, so the next agent reads the docs, believes
the RT-199 class is handled, and does not check. The plan for row 3
(`2026-10-31-codex-architecture-deepen-execution-outline.md:177`) specifies the
missing half precisely: the four-span exclusive breakdown must be built from
`turn_start.elapsed()`, the per-call LLM durations, `tool_duration_for`, and the
estop hold, and rendered on the volatile `<turn_context>` tail next to the
`<context_budget>` block (`run.rs:2983`). The types are all there; the emission
site is not.

**NOT fixed here.** It is a real one-iteration row, not a red-team fix, and it
touches the agent loop's request path, which is where a careless edit breaks a
prompt-cache invariant. Recorded so the next agent knows the row is OPEN, not
done.

## Rows 2, 5, 6, 7: measured, clean

- **Row 2 (process hardening) is correct and live.** `hardening.rs` runs first
  in `main()` (before config, before any daemon spawns, before `.env` is read),
  nothing in the workspace resets `RLIMIT_CORE`, there is exactly one `main()`
  so every entry point including `operant gateway run` is covered, and the
  refusal policy is a deliberate, documented divergence from codex (log and
  carry on, because refusing to start over a non-security failure is worse).
  Verified on the RUNNING gateway process, not just in source:
  `Max core file size 0 0 bytes` in `/proc/<pid>/limits`, pid 1697588.
- **Row 5 (mermaid) and Row 6 (hook decisions) were BLOCKED pending operator
  answers and were correctly never shipped.** They are holds, not defects. Row
  6 in particular asks whether an operator-configurable shell command should be
  able to rewrite a tool call's arguments and inject a developer message; that is
  a security posture, and it should not be decided by an agent mid-audit.
- **Row 7** was fixed in iter-1616 (the orphan itself), and the second pass
  confirmed the two channel surfaces that had re-implemented the filter inline
  are correct as written.

## The correction this pass makes to the first pass

The first pass ended with a five-guard list that read like a certificate. Five
guards were green over code whose fuzzy tier ignored its own gate and whose
count described an operation that had not happened. The discipline that would
have caught it: after every mutation-guard run, **read the production call path
end to end** and ask what each guard actually exercises. The row-4 fuzzy guard
exercised the gate; the write never went through the gate.

## The gate, second pass

`cargo test -p operant-core --lib blast_radius` = 29 passed (27 + 2 new).
`cargo test -p operant-core --lib patch_tool` = 34 passed (31 + 3 new).
`cargo fmt --all` clean. Clippy clean on both touched files.

Three mutation guards observed failing BY NAME before restore in this pass:
dropping the `first_is_a_quoted_phrase` guard (fails both row-1 tests),
restoring `exact_count` in the exact tier's tuple (fails D3), making the fuzzy
target check unconditional (fails D4). Each restored to green.

The remaining known-red item is unchanged: `profile::tests::test_use_profile`
fails under full-suite parallel load and passes in isolation (the RT-203 family),
with no profile.rs in this diff.
