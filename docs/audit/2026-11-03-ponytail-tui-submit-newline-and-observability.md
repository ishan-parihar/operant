# ponytail-audit — the TUI submit/newline surface and the self-observability surface

Scope (the operator's objective areas, 2026-11-02):

- `crates/operant-cli/src/cmd_tui_debug.rs` — the key-grammar parser (P0-2's fix)
- `crates/operant-cli/src/tui/app/messaging.rs` — composer restore (P0-1's fix)
- `crates/operant-cli/src/tui/app/tests.rs` — 4 regression tests for the leak fix
- `crates/operant-cli/src/cmd_tui_debug.rs` — 2 regression tests for the key grammar
- `crates/operant-core/src/trajectory.rs` (182 lines) — the recorder G-1 targets
- `crates/operant-core/src/observer.rs` (542) — the telemetry seam G-2 targets
- `crates/operant-core/src/advisor.rs` (690) — G-4's subject
- `crates/operant-core/src/agent/error_classifier.rs` (1599) — G-3's subject

Over-engineering and complexity only. Correctness, security and performance are out of
scope by the skill's contract. Every claim below was measured, not recalled.

---

## Verdict per file

| file | verdict | one-line reason |
|---|---|---|
| `cmd_tui_debug.rs` (my +107) | **Ship** | one closed lookup table, no new abstraction |
| `app/messaging.rs` (my +42) | **Ship** | one bounded 6-line helper + one call |
| `app/tests.rs` (my +108) | **Ship** | 4 behaviours, no harness |
| `trajectory.rs` | **Ship** | 2 structs, 11 fns, zero indirection |
| `observer.rs` | **Lean** | the *emission* side is the bloat, not the trait |
| `advisor.rs` | **Lean** | justified as-is; its surface is missing, not excessive |
| `error_classifier.rs` | **Ship, do not touch** | a 24-class match table is a match table |

---

## `shrink` findings

**`chord_key_code` duplicates `parse_key_sequence`'s bare-name table.** My fix adds a
16-arm `match` alongside the parser's own 18-entry table, and the two lists overlap on
15 names (`enter`, `tab`, `up`, `down`, `backspace`, `del`…). Two tables naming keys is
the classic setup for drift: adding `f13` support to one and not the other makes
`<f13>` and `<ctrl+f13>` disagree. **Kept anyway**, deliberately: extracting a shared
`bare_key_code()` means restructuring the parser's `match lower.as_str()` arm, which is
a 60-line change inside a file a peer is actively editing (`mouse.rs` passthrough, and
the same crate), for a saving of ~15 lines. Risk-adjusted that is a bad trade in this
session. **Filed as a follow-up:** fold `chord_key_code` into `parse_key_sequence`'s own
table by hoisting the name→`KeyCode` step out of the `match`, so there is exactly one
place a key name is spelled. [cmd_tui_debug.rs:1166]

**`observer.rs`'s `ObserverMetric` has 6 of 7 variants with no production emission.**
`ActiveSessions`, `QueueDepth`, `TokensUsed`, `HandRunDuration`, `HandFindingsCount`
and `HandSuccessRate` are constructed only in `operant-api`'s test module. They are not
dead code in the "never referenced" sense — they are *never constructed anywhere that
isn't a test*. Six variants × a variant-shaped API is a design that anticipates a
consumer that does not exist. **Do not delete them** (the trait is public and deleting
variants is a breaking change), but do not *build* G-2 as "wire up the sink" either: a
sink for metrics nobody emits is theatre. The plan already says build both halves or
neither. [observer.rs:133]

**`restore_failed_input_to_composer`'s new call is one line.** No helper class, no
retry, no flag. The helper it calls is 6 lines with a doc comment that is 60% of its
length, which is correct for this repo — the comment is the *bounding argument* for
why removing a transcript row is safe to do by content match. [messaging.rs:328]

## `delete` findings

Nothing in the audited surface is deletable on over-engineering grounds. Checked and
measured, not assumed:

- **No dead `ObserverEvent` variant.** All 27 are referenced; the emitted-from-production
  set (`AgentStart`, `AgentEnd`, `LlmRequest`, `LlmResponse`, `ToolCall`, `TurnComplete`,
  `Error`) is a strict subset, so the unused ones are *unemitted*, not unreferenced. Same
  class as `observer.rs`'s finding above — a design awaiting its consumer, not rot.
- **`trajectory.rs` is the lean end of the spectrum, not the fat one.** 2 structs, 11
  functions, no trait, no builder, no `Deref`. G-1 asks it to record *more*, which is
  unusual for this skill's output and correct here: the recorder is the one observability
  surface that is complete enough to be worth deepening.
- **`advisor.rs`'s two public functions both have callers** (`review_should_continue`,
  `spawn_advisor`). Its 690 lines are guards plus quarantine patterns, each of which
  exists because of a named failure in the module's own doc header.

## `yagni` findings

No speculative abstraction was added. The fix is a `match` and a `Vec::rposition`.
`drop_transcript_user_row_matching` is not parameterised by "which field to match on",
not generic over message type, and not a trait method — one caller, one shape.

One thing that is *not* YAGNI but looks like it: the `error.rs`-shaped `error` field G-1
asks to add to `TrajectoryStep`. It could be "just the bool" and still satisfy the
proposal's spreadsheet. But the proposal's own incident (an `aft_apply_patch` that
failed 12 times) is exactly what a `bool` cannot tell you — the patch-format failure
and the not-ready-index failure are both `false`. The field earns its place.

## `native` / `stdlib` findings

Two findings, both about my own code:

**`chord_key_code` should not exist as a function.** The numbers it dispatches on are
already available as `KeyModifiers` bit flags and `KeyCode` variants; the "name → variant"
step is the only thing the table adds. The `native` tag applies: there is no stdlib type
that parses a key name, so the table is unavoidable — but it is one table, and it should
be the *same one* the parser already has. See the `shrink` finding.

**No reinvented iteration.** `drop_transcript_user_row_matching` uses `rposition`
(reverse, one match, tail-first) rather than a hand-rolled loop, and the tests use
`iter().filter().count()` for the user-row count rather than a manual counter.
Both lookups were deliberately *not* extracted into a `fn user_row_count()` helper:
each has exactly one caller.

## What the audit says about the *plan*

Three of the six plan rows are thin, and the audit is the evidence:

| row | real work | verdict |
|---|---|---|
| G-1 | 3 `serde(default)` fields + 3 populate sites | **do it** |
| G-2 | 1 sink body + 2 emission sites | **do both halves**, see above |
| G-3 | 1 counter map + 1 subcommand | thin but load-bearing |
| G-4 | 1 persist call + 1 read back | keep in plan, sequence last |
| G-5 | ~100 lines | thin; ship or drop |
| G-6 | probe + hint row | the only row with real design content |

**The one row worth more than its line count is G-6.** It is the production half of the
operator's Shift+Enter report, and it is the only row that requires reading another
codebase's protocol and mirroring it. Everything else on the list is plumbing between
two places that already hold the number.

---

## Net

This session's own delta: **+256 lines of production and test code** across 3 files,
fixing 2 operator-reported defects, each mutation-proved RED. No new file, no new
module, no new dependency, no new trait, no new config key.

---

## The one negative result, recorded because a green gate that catches nothing is worse
## than no gate

Two corpus scenarios were added (iter-1679). One is mutation-proof; the other is not,
and I only found that by doing the mutation:

| scenario | fix reverted | result |
|---|---|---|
| `shift-enter-newline` (`ab<shift+enter>cd`) | `chord_key_code` branch deleted | **RED** — fails on `not-contains:shift+enter` |
| `failed-turn-no-duplicate-prompt` (`hello world<enter>` + `error-modal.json`) | `drop_transcript_user_row_matching` call deleted | **STILL GREEN** |

The second one's mechanism is worth knowing: the headless mock path
(`else if is_mock`, `adapter_types/tui_app.rs:1099`) pushes the user message itself,
without going through `submit_user_message`. So the transcript row that the fix removes
is never created by the mock, and deleting the removal cannot change its output. The
scenario asserts `count:hello world==1`, which is true before and after.

**Consequence and what it says about the harness:** any defect reachable only through
the live submit path is invisible to the mock corpus. The four unit tests in
`app/tests.rs` cover this one because they call `take_input` + `add_message` in the same
order production does. If more of these accumulate, the fix is a live-path scenario
runner (drive a real provider error, not a mock), not more unit tests.

The scenario is kept, with its notes rewritten to state the negative result, because it
still pins the restore-once outcome deterministically and cheaply.
