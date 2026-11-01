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
