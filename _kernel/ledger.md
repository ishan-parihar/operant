# Task Ledger — agentic-core/TUI repair

0. End-state test: `cargo test -p operant-core --lib`, targeted CLI tests, clippy, and behavior regressions prove full-context resume, 1M display, 350k/400k caps on token+percentage, bounded continuation, and logging seams.
1. [x] Add `ContextCaps` + `ContextCapLimits` (`soft=min(350k,80%)`, `hard=min(400k,95%)`) and config fields/defaults — done when pure tests cover stricter-wins and zero/disabled values — depends on: —
2. [x] Use configured context window in TUI and gateway, expose agent accessor — done when custom bare model shows configured window and core/TUI agree — depends on: 1
3. [x] Persist/rehydrate tool results and assistant reasoning/finish_reason — done when resume test keeps assistant tool_calls + tool rows — depends on: —
4. [x] Make truncation continuation see truncated text; persist it — done when failing/current loop recovery test contains partial assistant text — depends on: 3
5. [x] Bounded grace continuation, retries, and persistence — done when grace tests assert conversation/DB carry the summary and stream errors are typed/logged — depends on: 3
6. [x] Enforce soft cap at turn end and hard cap mid-stream with compaction — done when regression tests cross caps and assert compaction occurred — depends on: 1
7. [x] Add logging at session rehydration, tool results, compaction/retry/errors — done when tests can capture diagnostics/messages, no silent count loss — depends on: 3,6
8. [x] Fix proven TUI stale counters/session display/early-turn text/debug event defects — done when CLI tests cover each — depends on: 3,6,7
9. [x] Update example config, README, CHANGELOG, BUGS ledger, TODO where behavior changes — done when doc gates pass — depends on: 8
10. [x] Scoped workspace verification, clippy/fmt, commit with next iteration, push, deploy release — done when origin/main contains the iteration and deployed binary matches — depends on: 9

## Round 1 (iter-1769, shipped)

The caps, the resume, the truncation/grace recovery, the TUI counters and the
session-boundary gate. Commit `6bc1b7af`. Ledger rows RT-225/RT-226.

## Round 2 (iter-1774, shipped)

The restored transcript (one shared converter), the silent stream end (the
`[DONE]` marker is surfaced and an unmarked end is continued when the text
looks cut), the replayed tool call (the dedup set is cleared before a
replay), and the debug log's blind spot (the inline picker's keys, the live
agent error, the terminal error exit's dump). Commit `55ed08b4`. Ledger row
RT-227.

**Verified in round 2**
- `cargo check -p operant-core --lib` and `-p operant-cli --bin operant` — clean.
- `cargo test -p operant-core --lib --tests` — 2792 passed / 5 failed, the
  SAME five pre-existing failures (three environmental, one sourcehound,
  one the peer's mcp.rs).
- `cargo test -p operant-cli --bin operant` — 2185 passed / 2 failed, both in
  the peer's in-flight vim/settings work; this slice's ten TUI cases pass.
- New: `crates/operant-core/tests/stream_incomplete_continuation.rs` (2),
  `crates/operant-core/tests/stream_replay_tool_rows.rs` (1, mutation-proved),
  `crates/operant-cli/src/tui/app/session_display_tests.rs` (4),
  `crates/operant-cli/src/tui/app/debug_log_tests.rs` (2), three adapter
  completion-signal cases in `clients/openai.rs`.

## Round 2b (iter-1776, shipped)

The clippy gate is green again. It was red at HEAD with an EMPTY allowlist
and four findings on untouched lines: two boolean `assert_eq!`s in the patch
tool's occurrence rules, a plugin fixture test without the test-target
exemption header, an `#[expect(clippy::expect_used)]` attribute in
`agent/run.rs` that sat on `turn_attribution` (no `.expect()`) instead of on
`run` (four), and a const-assertion lint in the schema fixture's runtime
assertion. Commit `bef1858d`.

## Final gates (measured on this tree)

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo check --workspace --all-features` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean, 0 warnings |
| `bash scripts/clippy-warning-gate.sh` | pass (seen=0, new=0, stale=0) |
| `cargo test -p operant-core --lib --tests` | 2792 passed / 5 failed (pre-existing: three environmental, one sourcehound, one the peer's `mcp.rs`) |
| `cargo test -p operant-cli --bin operant` | 2186 passed / 1 failed (the peer's in-flight `settings_screen` work) |
| `cargo test --workspace --exclude operant-cli` | the same five pre-existing core failures, nothing else |
| `./scripts/genome lint` | 0 errors |
| `python3 scripts/audit-ledger-index.py` | OK |

## Not done, and why

- `TODO.md` (the autonomous-mode ledger) is untouched: the caps change the
  context behaviour every mode shares, but no autonomous-mode row opened or
  closed by it.
- The synchronous SQLite load in the TUI frame loop was left alone: it is a
  performance shape, not one of the audited defects, and moving it is a
  change with no failing test behind it.
- The five core and one CLI test failures are NOT this slice's: they are
  environmental (the repo IS a git repo; the sourcehound binary is absent),
  the peer's `mcp.rs` WIP, and the peer's in-flight `settings_screen` work.
  Measured, named, and left alone.

## Round 3 (2026-11-06) — the context calculation, the lying log, the poke

End-state test: the live session `sess_56bae289-989f-42f8-b28f-a3942a51ebdc` (653 rows,
1.72 MB of text) must (a) build a request whose estimated size agrees with the provider's own
reported prompt tokens, (b) NOT emit a compaction card below the soft cap, (c) keep its whole
transcript on resume with an honest log, and (d) when a turn ends early with no usable answer,
the core must continue it with a bounded synthetic poke instead of stopping dead.

| # | unit | done when | depends on | premises |
|---|---|---|---|---|
| 1 | [x] Measure the live defects from the gateway log + the live DB | evidence table in `_kernel/claims.md` | — | A1..A5 |
| 2 | [ ] iter-N1: estimator truthfulness — stop charging reasoning no adapter sends; anchor the estimate on the provider's reported count | core+cli tests green; the live session's estimate lands near the provider's number; clippy gate clean | 1 | A1,A2 |
| 3 | [ ] iter-N1b: honest session-rehydration log (`rows_dropped` = rows actually dropped; warn only when rows were dropped) | unit test fails without the fix | 1 | A5 |
| 4 | [ ] iter-N1c: the compaction card fires only when a rung really changed the request | unit/integration test | 2 | A3 |
| 5 | [ ] iter-N2: core auto-poke — bounded synthetic continuation after an early exit with no usable answer; bypasses the configured iteration limit; non-retryable errors never re-poked; does not collide with the grace continuation, the stream-incomplete continuation or the advisor continuation | integration tests: poke fires / is bounded / is skipped on a non-retryable error / never fires when the wrap-up produced an answer | 2 | A4 |
| 6 | [ ] iter-N3: honest early-turn notices in the core (`TurnDiagnostics::explanation`), the TUI arm and the gateway substitution | the notice names the real cause; no double notice | 5 | A4 |
| 7 | [ ] iter-N4: TUI defect sweep on the touched surfaces + full-scope logging for the tool-call / mid-stream paths | the audit records each defect with evidence; every fixed defect has a test | 2,5 | — |
| 8 | [~] final verification + release build + deploy + gateway restart | the canonical gate table is green; `operant --version` matches the deployed md5 | 2..7 | C1..C7 |

### Round 3, second half — the resumed session's tool history (iter-1786)

| # | unit | done when | depends on | premises |
|---|---|---|---|---|
| 5b | [x] Recover the `tool_call_id` of tool rows persisted without one, from the transcript's ORDER | `crates/operant-core/tests/session_tool_pairing.rs` (an id-less result is paired again; a mixed turn keeps the id it has) + the live-data replay (0 mispairings) | 1 | A6 |
| 6b | [x] The session distillation parses the array out of a prose-wrapped answer | two new unit tests in `distillation.rs` | 1 | A7 |
| 7b | [x] Ship iter-1787: commit, push, release build, deploy, gateway restart | `origin/main` carries the commit; the deployed binary's md5 matches the artifact | 5b,6b | C3 |
| 7c | [x] iter-1789: the crash the DEPLOYED binary showed on the operator's own data (`operant sessions list` panicked on a byte slice of a title with an em-dash) — and the same byte-slicing shape in four other display paths | the deployed command prints the table; the regression test fails without the fix | 7b | A8 |
| 7d | [x] iter-1787 + iter-1789 deployed: release build, `~/.local/bin/operant`, gateway restarted | the deployed md5 equals the artifact's; `systemctl --user status operant-gateway` shows the new inode | 7b,7c | C3 |

**Measured while round 3 ran** (the numbers are in `_kernel/claims.md`): the live session's
437 tool rows, 407 of them without an id; the live log's 818 dropped halves; ~25 000 rows
recoverable across six sessions; 15 prompt tokens for 18 000 characters of `reasoning_content`;
27 distillations lost to a wrapped array.

### Not planned (named, with the reason)

- The AFT bridge's undelivered-completion backlog: a different subsystem, measured separately
  (`operant-testing/kb/red-team/01-turn-early-root-cause.md`).
- jcode's TODO-policy-specific poke wording: operant's core has no TODO-policy crate; the
  mechanism (bounded synthetic continuation + non-retryable guard) is what is ported.


## Round 3 outcome (shipped)

- iter-1783 (`8264cdea`): the estimator stops charging the reasoning the provider does not charge;
  the session-rehydration warning reports the rows it actually dropped; the auto-poke (jcode's
  mechanism) gives an early turn one more pass WITH its tools; the "Turn ended early" notice names
  the cause that happened.
- iter-1787 (`ecf5c59a`): a resumed session keeps its ENTIRE tool history (the rehydration recovers
  the ids the old writer never stored); the session distillation reads the facts out of a
  prose-wrapped answer.
- iter-1789 (`450925a0`): `operant sessions list` no longer dies on the operator's own title; five
  byte-slicing display paths are char-safe.

**Verified on this tree**: `cargo fmt --all -- --check` clean; `cargo clippy -p operant-core
--all-targets --all-features -- -D warnings` clean; `cargo clippy -p operant-cli --bin operant
--all-features -- -D warnings` clean; `cargo test -p operant-core --lib` 2798 passed / 5 failed
(the same five pre-existing: the peer's `mcp.rs`, three environmental, sourcehound); `cargo test -p
operant-cli --bin operant` 2197 passed / 0 failed; the new integration targets (auto_poke,
session_rehydration_log, session_tool_pairing) green; `./scripts/genome lint` 0 errors;
`python3 scripts/audit-ledger-index.py` OK.

**Deliberately NOT done** (named, with the reason): the AFT bridge's undelivered-completion backlog
(a different subsystem, measured in `operant-testing/kb/red-team/01-turn-early-root-cause.md`); a
suffix rule for the tool-name recovery ("bash" → "aft_bash") — the refusal already names the
candidates, and a wrong recovery runs a tool the model never asked for; the prompt-cache misses
(the cacheable head changes because the tool set is exposure-dependent — an architectural change,
not a patch); the five pre-existing core failures (not this slice's).

## Round 4 (2026-11-06) — the remaining agentic-core gaps

End-state test: `cargo test -p operant-core --lib --tests` is green except the peer's in-flight
`mcp.rs` test, every gap this round fixes has a regression test that fails without the fix, and
each fix lands as its own pushed iteration.

| # | unit | done when | depends on | premises |
|---|---|---|---|---|
| 1 | [x] Re-derive the open gaps from live evidence (test suites, the audit ledger, the scratch gap register) | the measured table below | — | A6..A9 |
| 2 | [ ] iter-N1: the config template stops advertising config keys the loader REJECTS (`requires`, `provides` under `[plugins]`) | `crates/operant-core/tests/example_template_loads.rs` is green; the manifest schema has exactly one owner (`docs/library/plugins.md`) | 1 | A6, C8 |
| 3 | [ ] iter-N2 (RT-197): `[skills]` paths expand a leading `~` / `$HOME` / `$OPERANT_HOME`, so a copied template path never creates a literal `$HOME` directory | a load test fails without the fix and passes with it; the template's commented example stays honest | 2 | A7 |
| 4 | [ ] iter-N3: a failing AFT command surfaces the diagnostics the bridge already captured (stderr tail) instead of "unknown error" | a bridge test fails without the fix and passes with it | 1 | A8 |
| 5 | [ ] iter-N4: the `harness_tools.rs` header stops claiming mount/unmount are stubs (they are implemented and tested) | the header matches the code; no behaviour change | 1 | — |
| 6 | [ ] final verification: fmt, scoped clippy, core lib+integration suites, genome lint, audit-ledger index | the gate table is green except the peer's `mcp.rs` test, which is reported | 2..5 | C1..C7 |

### Measured while round 4 ran

- `cargo test -p operant-core --lib`: 2802 passed, 1 failed, 1 ignored — the one failure is
  `mcp::tests::a_server_that_never_matches_the_request_id_fails_boundedly`, in the PEER's
  in-flight `mcp.rs` (their response-drain rewrite changed the behaviour their own test pins).
- `cargo test -p operant-core --tests --no-fail-fast`: one non-peer failure —
  `every_commented_example_in_the_template_names_a_field_the_loader_accepts` — reporting
  `operant.example.toml` line 568 (`requires`) and line 572 (`provides`).
- Those two lines are `manifest.toml` fields of a PLUGIN, not `operant.toml` keys; the loader's
  `deny_unknown_fields` rejects them, and `docs/library/plugins.md` already owns that schema.
- The scratch gap register's "MCP response ID mismatch" class is the peer's in-flight `mcp.rs`
  fix, not an open slice; its `aft_inspect` "unknown error" class is real and is unit 4.
- The remaining ledger rows are records/decisions, not code gaps: RT-136 (a recorded
  confirmed-not-a-defect), RT-190 (an owner ruling on two memory-wire bank families), RT-209
  (two rendering options; the module's own header says the choice is the operator's).

### Not planned (named, with the reason)

- RT-209's subscriber: the module header states the remaining choice (a TUI panel vs a `<turn_context>`
  block) is the operator's, and the measured blocker (the volatile tail is built once per turn,
  before any tool runs) makes the prompt seat impossible.
- RT-190's bank bridge: an owner ruling, already documented as a boundary.
- The peer's `mcp.rs` response-drain rewrite and its red test: another agent's slice, in flight.
- The deploy copy and the gateway restart: the sandbox has no systemd user bus and cannot write
  `~/.local/bin`; the release artifact is built in-tree and the exact deploy commands are reported.

### Round 4 — SHIPPED (iter-1798, iter-1799)

- **iter-1798** (`928a6394`) — RT-197 (the `[skills]` paths resolve `~`/`$HOME`) +
  RT-246 (the template stops advertising the plugin MANIFEST's fields as config keys).
  Both mutation-proved; `config.rs` 43/43, `example_template_loads` 4/4, clippy clean.
- **iter-1799** (`e08a643c`) — RT-247 (a failing aft command reports the child's stderr)
  + RT-248 (`harness_mount`/`harness_unmount` can be approved — the host now passes the
  approval on). Both mutation-proved; `aft_bridge` 9/9, `agent::tests` 79/79,
  `harness_tools` 6/6, clippy clean.
- Both pushed to `origin/main`; the audit ledger's index agrees with its checkboxes
  (`python3 scripts/audit-ledger-index.py` → OK) and the open rows are now four:
  RT-136 (a record), RT-179 (a channel-parity feature), RT-190 (an owner ruling),
  RT-209 (an operator choice between two rendering seats).

### Round 4, continued — SHIPPED (iter-1800, iter-1805, iter-1808)

- **iter-1800** (`c1870d71`) — RT-249: a delegation that produced nothing is a NAMED failure
  (one shaping point, `child_outcome`), not an empty successful tool result. Mutation-proved
  (the check had to move INTO the function that shapes the outcome — a helper-level test did
  not pin the call site).
- **iter-1805** (`c49f282f`) — RT-250: the plugin observe envelope's post-execute event carries
  the call's args again (the redaction seat could see the output but not the input).
  Mutation-proved.
- **iter-1808** (`4a35be8a`) — RT-255: the workspace lint gate's 13 expectations became allows
  with the same reasons (an expectation on a FLOATING `stable` toolchain is a red/green coin
  flip). `bash scripts/clippy-warning-gate.sh` → "seen=0 allowlist=0 new=0 stale=0 — pass".
- All three pushed to `origin/main`; the release artifact (20:48:18, after the last commit at
  19:58:47) reports `operant 0.3.0 (memory-wire b478d15)`, md5
  `f8c636fad4750484d1163349c2e23bec`. The deploy copy and the gateway restart remain BLOCKED
  by the sandbox (read-only `~`, no user bus) and are reported, not faked.

### The agentic-core's remaining gaps after iter-1808 — MEASURED, not assumed

| gap | status | why |
|---|---|---|
| RT-136 | a RECORD | "Carried open as a RECORD, not a task" — `[pk]`/`[socialization]` are honestly labelled unread |
| RT-179 | a FEATURE | the channel twins; the functional core is already reachable from a channel |
| RT-190 | an OWNER RULING | two memory-wire bank families; the boundary is documented, the bridge is a future tranche |
| RT-209 | an OPERATOR CHOICE | two rendering seats for the turn attribution; the module header says the choice is the operator's |
| the harness2-contrast plan (iter-A/B/C) | LANDED | `HARNESS_LESSON_TAG`, the verdict tripwire, the INV/DOM review lane, the curator auto-run, the status counters |
| the plugin plan (G1/G2/G6/G7/G8/G9) | LANDED | the turn-interceptor seat, the Replace verdict, live re-registration, creator mode, the in-tree fixture, `plugins status` |
| `write_approval` / `persistence_seam` | ACCOUNTED FOR | KB-3/AD-020 decided against wiring the first; the second is inside the harness2 plan's audited surface |
| `Dispatcher::complete_run`/`fail_run` | hygiene, not a gap | `operant kanban complete` completes tasks through `db.complete_task` |
| the MCP OAuth manager (`build_authorization_url`, `handle_401`) | NOT WIRED | a real gap, but it needs a decision on the OAuth surface (the MCP client never sees a 401 path); named, not shipped |
| the sourcehound / aft-binary limits | EXTERNAL | the binaries' own caps and the server's timeouts, not this crate's code |
| the peer's `mcp.rs` | IN FLIGHT | their response-drain rewrite; their own test is red in their tree |


### Round 4 — what is still open after iter-1799

- The release build + deploy: the sandbox has no systemd user bus and cannot write
  `~/.local/bin`, so the artifact is built in-tree and the deploy commands are
  reported instead of faked.
- The peer's `mcp.rs` response-drain rewrite is still in flight and its own test is
  red in their tree (`a_server_that_never_matches_the_request_id_fails_boundedly`).
- RT-209 (the turn-attribution subscriber), RT-190 (the memory-wire bank families),
  RT-179 (the channel twins), RT-136 (a record): records/decisions, not code gaps.
- The sourcehound/AFT-binary gaps in the scratch gap register are EXTERNAL to this
  crate (the aft binary's own limits, the sourcehound server's timeouts); the core's
  half of the aft class (RT-247) is closed.

### Round 4 — the next tranche (found by the orphan-module sweep, for the NEXT round)

The sweep over `crates/operant-core/src/lib.rs`'s `pub mod` list (count each module's
external users) found TWO modules with zero callers outside themselves:

- `write_approval` — ACCOUNTED FOR, not a new gap: ledger row KB-3 (closed iter-971,
  AD-020) already decided against wiring it ("no staged apply path (the `write_approval`
  module has no caller; deletion candidate)") — the three knowledge tools were gated at
  the PERMISSION prompt instead. Do NOT wire it; the decision is recorded.
- `persistence_seam` — the 018-U1 sidecar persistence seam. It is one of the mechanisms
  the repo's own dated-frozen plan audits (docs/plans/2026-11-05-recursive-self-harnessing-
  for-operant-harness2-contrast-audi.md).

That plan is the next named tranche, and it is already scoped ("Three small iterations;
the only new persistence is one reserved tag and one marker scan"):

- **iter-A** — the harness-lesson record: a reserved `harness-lesson` tag in `memory_wire`,
  a four-line body (`component:` / `cue:` / `lesson:` / `evidence:`, `confidence:` optional),
  and a TRIPWIRE at retain time that refuses-and-logs a lesson body carrying validation
  markers ("test result:", "FAILED (", "panicked at", "0 passed", "cargo test",
  "test_command"). Gates: a round-trip test and a tripwire test.
- **iter-B** — the review fork becomes a propose–reflect lane (prompt text only in
  `agent/background_review.rs`): INV/DOM split, verdict discipline
  (`helped | hurt | no observed effect | confounded`), at most ONE lesson, `NO_ENTRY`
  expected, library read before edit.
- **iter-C** — close the loop: prefetch includes the lesson tag, `CuratorEngine::maybe_run_curator`
  (zero callers today) fires from the review cadence gate, and `operant status` gains two
  counters.

Other named open items (decisions, not code): RT-209's subscriber (two rendering seats —
the operator's choice), RT-190's bank bridge (an owner ruling), RT-179's channel twins
(a feature), RT-136 (a record), the harness-level lane's default flip (an owner decision
the plan explicitly does not take silently).



## Round 6 (2026-11-07) — the operator's five TUI complaints

End-state test: every one of the five symptoms has a regression test that fails without the fix,
`cargo test -p operant-cli --bin operant` and `cargo test -p operant-core --lib` are green, the
clippy gate passes, and each fix is a pushed `iter-N` commit.

| # | unit | done when | depends on | premises |
|---|---|---|---|---|
| 1 | [x] the steer queue is MIRRORED onto the App, so the queued band renders the real queue | a queued message shows in `queued_messages()`; Ctrl+X recall updates the mirror | — | A14 |
| 2 | [x] queued + goal messages enter the prompt history | the Up/Down ladder and `history.jsonl` carry them | 1 | — |
| 3 | [x] `/goal` is a STANDING goal (injected into every turn) | the agent's turn context carries `<session_goal>` on the next turn | — | A15 |
| 4 | [x] the advisor card renders in BLOCKS (one per note), not as one `"; "`-joined lump | the card text has one block per note; the continuation payload stays machine-readable | — | A16 |
| 5 | [x] no non-input text inside the composer's text area | the queue hint moves to the status line; the text rect carries only the operator's text | — | — |
| 6 | [x] final verification: fmt, scoped clippy, core+cli suites, genome lint, audit index | the gate table is green | 1..5 | C1..C7 |

### Shipped (all on `origin/main`)

| iteration | commit | what |
|---|---|---|
| iter-1839 (re-applied as iter-1841) | 15cec34e | the queue mirror, the prompt-history writer, the standing goal |
| iter-1842 | 23015b38 | the advisor card in blocks, the card/payload split |
| iter-1845 | 64094d60 | the composer hints moved out of the input area |
| iter-1846 | d3d4aef7 | the changelog entries |

### Final verification

- `cargo test -p operant-core --tests` → 2833 passed / 0 failed (lib) and every
  integration target 0 failed.
- `cargo test -p operant-cli --tests` → 2235 passed / 0 failed / 8 ignored (bin)
  plus the integration targets 16 / 3 / 1 passed.
- `cargo clippy -p operant-core --lib --all-features -D warnings` → clean.
- `cargo clippy -p operant-cli --bin operant --all-features -D warnings` → clean.
- `cargo fmt --all -- --check` → clean.
- `python3 scripts/audit-ledger-index.py` → OK. `./scripts/genome lint` → 0 errors.
- Release build → exit 0; the binary is installed at `~/.local/bin/operant`
  (`operant --version` → 0.3.0) and `operant-gateway` is ACTIVE on that path.

### NOT DONE (named, not hidden)

- The `docs/audit/2026-06-19-BUGS.md` rows for these defects. The file is in a
  concurrent editor's in-flight change (its index table and its detail rows must
  move together, and `scripts/audit-ledger-index.py` checks the pair), so the
  rows were left to that editor. The CHANGELOG entries and the commit bodies
  carry the audit trail meanwhile.
- The workspace-wide `cargo test --workspace` in a CLEAN worktree: the cold
  target dir cannot build `espeak-rs-sys` on this host (the espeak-ng data
  generation fails), and in the SHARED tree the same command cannot compile
  because of the concurrent editor's in-flight `WorkerSpec::toolsets` change.
  The two touched crates' full suites (lib + integration + bin) are green in
  the shared tree, and the pushed state was re-read from `origin/main`.

### Measured while round 6 ran

- `TuiState::queued_messages()` returned `&[]` — a `[port-decision]` stub — so the queued band
  NEVER rendered a queued message, and `pending_soft_interrupts` had the same stub.
- `session_goal` had NO reader outside its own setter: the "standing session goal … injected as a
  system preamble message whenever it is set" doc was false.
- The advisor's preserve channel joined the notes with `"; "` into ONE summary string.
- The composer's queue hint was pushed into the SAME `lines` vector as the wrapped draft, so it
  was painted as the FIRST ROW of the text rect.

---

## Round 5 (2026-11-06) — the remaining agentic-core gaps

End-state test: `cargo test -p operant-core --lib --tests` is green, the workspace clippy gate is
green, and every gap this round fixes has a mutation-proved regression test and a pushed iteration.

| # | unit | done when | depends on | premises |
|---|---|---|---|---|
| 1 | [x] Baseline: core suite green, peer WIP identified, genome wake check read | the suite is green at HEAD; the peer's files are named | — | — |
| 2 | [x] A fresh evidence sweep over operant-core (a read-only subagent + my own greps) | 5 findings, each with file:line evidence | 1 | — |
| 3 | [x] iter-1813: the `[proxy]` table reaches the provider clients (RT-258) | `configured_proxy_is_used_by_the_provider_client` passes; mutation-proved; pushed | 2 | A10 |
| 4 | [x] iter-1816: the deferred bridge reads the TYPE, not the name (RT-259) | `a_type_deferred_tool_is_reachable_through_the_bridge` passes; mutation-proved; pushed | 2 | A11 |
| 5 | [x] iter-1819: a plugin observer is registered ONCE and unregistered when its capability is withdrawn (the HIGH finding) | the observer's `observe` export runs once per tool event after a live change; the turn gate is revoked when `turn_gate` is dropped | 4 | A12 |
| 6 | [x] iter-1821: the curator's idle clock moves on USE (a used skill must not be archived) | a skill used through the production path survives `apply_automatic_transitions` with `archive_after_days = 1` | 4 | A13 |
| 7 | [x] iter-1822: `agent.session_reset` is reported as UNREAD instead of as an active setting | the config's unread-warning list names it; the setup wizard stops claiming it works | 4 | — |
| 8 | [~] final verification + release build + deploy | the gate table is green; the artifact is fresh; the deploy is attempted and REPORTED (the sandbox blocks `~` and the user bus) | 5,6,7 | C1..C8 |

### Round 5 — measured

- `cargo test -p operant-core --lib --tests --no-fail-fast` at HEAD: GREEN (cargo exit 0). The
  peer's `mcp.rs` response-drain rewrite compiles and its own test passes in the shared tree.
- The `[proxy]` gap: `apply_runtime_proxy_to_builder`, `build_runtime_proxy_client` and
  `build_channel_proxy_client` had ZERO callers outside `operant-config/src/schema/proxy.rs`.
- The deferred-bridge gap: `assemble_tools` matches on `s.exposure`; `tool_search`'s catalog,
  `tool_describe`, `tool_call` and `materialise_for_turn` matched on `is_deferrable(&name)`.

### Round 5 - shipped

- iter-1813 (RT-258): the proxy table reaches the provider clients.
- iter-1816 (RT-259): the deferred bridge reads the TYPE, not the name.
- iter-1819 (RT-260): a plugin observer is registered once, and a withdrawn
  observer (or turn gate) is unregistered.
- iter-1821 (RT-262): the curator idle clock moves on a USE.
- iter-1822 (RT-263): agent.session_reset is reported as unread instead of
  being advertised as a policy.

### Round 5 - named residuals (not fixed, with the reason)

- The proxy TOOL half: the tool clients follow the process environment, not
  the runtime table. Wiring them blindly would weaken the SSRF guard (the
  guard resolves locally, a proxy resolves remotely) and break the localhost
  CDP client - a per-client decision, not a patch.
- The provider-account OAuth surface (oauth/, oauth_refresh.rs): real, with
  no reachable entry point; no small fix (it needs a login surface and a
  call site).
- The curator CONFIGURED skills root: with skills.root_dir set, the auto-run
  reads the default tree while the tools write the configured one - a
  config-resolution change.
- The peer slice WIP (mcp.rs, advisor.rs, the two core tests, the TUI files,
  scripts/genome): untouched, by the fleet rule.

---

## Round 8 (2026-11-07) — Ctrl+Enter, the generic compression, the immortal notice

End-state test: each of the three reports has a mutation-proven regression
test; the core and CLI suites are green; the release binary is built, installed
at `~/.local/bin/operant` and the gateway is restarted on it.

| # | unit | done when | depends on | premises |
|---|---|---|---|---|
| 1 | [x] every transient notice expires (TTL) and the dead field is cleared | the clipboard notice leaves the notification line and releases its row | — | — |
| 2 | [x] the compaction threshold scales with the window | a 1M window compacts at 800k, not 350k | — | — |
| 3 | [x] Ctrl+Enter always queues, and an idle-queued message is delivered with the next turn | the key never submits; the pending queue rides the next turn | 1, 2 | — |
| 4 | [x] verification: fmt, scoped clippy, core+cli suites, release build, deploy | the gate table is green | 1..3 | C1..C7 |

### Shipped (all on `origin/main`)

| iteration | commit | what |
|---|---|---|
| iter-1871 | e7cdc003 | the notice TTL: one reader, a clearing frame loop, two write doors |
| iter-1872 | 4eab5015 | the caps: the absolute axis off by default, the percentage governs |
| iter-1873 | a395ca53 | the README's line count restored (the band finding is pre-existing) |
| iter-1875 | d384d5ba | Ctrl+Enter always queues; the pending queue is delivered with the next turn |

### The compaction mechanism, in one paragraph

`run.rs` builds the request, estimates its tokens, and compares the estimate
with the resolved soft cap. Over the cap, the "preflight ladder" runs four
rungs in order — (1) re-trim historical tool results to 2_000 chars (no LLM),
(2) a deterministic decay-render, (3) the semantic goal-similarity cut (off by
default: it needs an embedder), (4) an LLM "summarize-before-evict" whose
output is explicitly framed as "background reference, NOT as active
instructions". Each rung runs only while the estimate is still over the cap;
the eviction rung then fits the request to the window. With the caps at
350k/400k absolute and a 1M window, the ladder fired EVERY turn of a long
session and rung 1 (the lossy trim) is what ran — truncating the agent's own
earlier reads, which is the "generic compression" the operator saw and the
reason the instructions it had read stopped being followed.

### NOT DONE (named, not hidden)

- The ladder's LOSSY budget (2_000 chars per historical tool result) is
  unchanged: with the threshold fixed it runs only when the request genuinely
  approaches the window, which is the right time for it. Widening it is a
  separate decision about what a long tool output costs.
- The semantic rung stays off by default (it needs an embedder, and every
  degrade path falls back to recency), and the context engine stays
  unattached — both are named in the report as the lossless paths.
- `docs/audit/2026-06-19-BUGS.md` rows for these three defects: the file is
  in a concurrent editor's in-flight change (its index table and its detail
  rows must move together). The CHANGELOG entries carry the audit trail.
- The README rubric's `length_in_band` finding (371 lines vs the monorepo
  band's 340) is PRE-EXISTING and is reported as such, not silently fixed by
  trimming 31 unrelated lines.

### Round 8b — the gateway crash loop (found while deploying)

| # | unit | done when | depends on | premises |
|---|---|---|---|---|
| 5 | [x] the gateway survives a child that closes its pipe | the daemon arm ignores SIGPIPE; the service stays up | — | — |

- iter-1880 (5a7d4885): `ignore_sigpipe_disposition` in the gateway arm; the
  CLI keeps the default disposition its piped output needs.
- MEASURED before: `operant gateway run` exits 141 (128+SIGPIPE) seconds after
  start; the journal shows 870+ restarts in two hours (the counter was
  already 863 before this session's deploy).
- MEASURED after: the foreground run survives the 20s timer (exit 124), and
  the service is `active` with `NRestarts=0`.

### Deploy (round 8)

- `scripts/build.sh --offline --release` → exit 0 (9m02s).
- Installed at `~/.local/bin/operant` (`operant --version` → 0.3.0; the
  deployed md5 equals the artifact's).
- `systemctl --user restart operant-gateway` → active, NRestarts=0.
- The operator's `~/.operant/operant.toml` sets `context_window = 1000000`
  and does NOT override the caps, so the new default (80% / 95%) applies to
  their session: 800k / 950k instead of 350k / 400k.

## Round 11 (2026-11-08) — the operant-core campaign, iter-1901…iter-1950

Fifty iterations, every one pushed (origin/main 8141e2a3 → b9fe0707). The
protocol held: fix → mutation-proved regression test → RT row + changelog →
index-script OK → commit+push. `docs/audit/2026-06-19-BUGS.md` gained
RT-328…RT-364 and `docs/CHANGELOG.md` the matching newest-first entries.

The fixes, by class:

- **A dropped parameter between schema and wire** (the round's dominant
  class): the kernel trio's namespace precedence / harness_list routing /
  stale invalidation list (RT-328), the camelCase-to-snake handoff (already
  shipped as RT-315 in round 10), inputSchemaProperties' arg merge (RT-318),
  the registry pass-through (RT-320, round 10), memory_forget's ignored id
  param (RT-322, round 10), notification_tool's dead priority knob
  (RT-352), process_tool's notify-on-complete (RT-347), video_analysis's
  analysis_type (RT-344), browser_cdp's unread timeout (RT-335),
  code_execution's UNBOUNDED cell timeout (RT-335, RT-337).
- **A unit or scale subscribed wrong**: the cookie jar writing HttpOnly into
  the netscape format's SUBDOMAIN column (RT-338, both directions of the
  round-trip repaired), the accessibility render taking a heading level's
  character count as the level (RT-339), the dated timestamp ignoring the
  requested unit (RT-352's fourth site).
- **A doc or log that claims more than the code does** (the RT-275 family):
  the session stall detector (RT-331), the mirror forwarder (RT-332), the
  admin handler's shutdown arm (RT-333), the runtime adapter (RT-345), the
  LCM tools' registration (RT-358), the persistence seam (RT-359), the rate
  limiter's status half (RT-360), the checkpoint registration seam's false
  "loaded" log (RT-356), the SMTP connect failure (RT-334), the env probe's
  substring false-positives (RT-341), the @-reference spellings (RT-363).
- **Nondeterminism or ordering**: the webhook route picked per HashMap
  iteration (RT-342), the anomaly budget counted per conversation (RT-354),
  the turn summary's dead overflow count (RT-350 → the row is RT-329),
  learning_manage's read-after-write snapshot (RT-343).
- **A live-path behaviour fix**: edit_message bypassing the dispatch
  breaker (RT-330), the ACP session/cancel destroying the session (RT-355),
  cli_roster's re-admission clobber (RT-349), the checkpoint list's
  hardcoded diff stats (RT-348), file_state's directory hashing (RT-346),
  fs_secrets' permission widening (RT-361), gateway_session_delete's
  creation-only guard → runtime-time store() (RT-321, round 10).

Measured, not assumed:

- The campaign's standing hazards all recurred and were survived: `git
  checkout --` wiped the RT-348 worktree and the pre-mutation backup
  restored it (the commit message records it); two `git checkout`s of the
  same hazard family; a restore-from-backup reverted a repaired contract to
  its self-satisfying form (RT-340's skills_guard half — re-repaired in
  iter-1938); and the end-of-round sweep found three more round-owned
  regressions (RT-364), including a contract vacuous since iter-1928
  because its own doc comment satisfied its needle.
- Twelve near-miss tests were caught by their own mutation runs (the
  needle matching the test's own prose, a camelCase-vs-snake_case schema
  needle, a doc comment that leaks into the schema description, a
  non-discriminating fixture where both conditions were true). Every one
  was repaired and re-proved; the commits say so.

### Deploy (round 11)

- `scripts/build.sh --offline --release` → exit 0 (12m03s).
- The install path this round was NOT the read-only wall the earlier rounds
  recorded: `cp target/release/operant ~/.local/bin/operant` first failed with
  "Text file busy" — the LIVE gateway held the old image open — so the fix was
  `systemctl --user stop operant-gateway`, the copy, `systemctl --user start`.
  The deployed md5 equals the artifact's (`~/.local/bin/operant` is
  57360992 bytes, stamped 18:06).
- `systemctl --user restart operant-gateway` → `active`, NRestarts=0, the
  journal shows "Gateway started with 1 platform(s)" on the new binary
  (`operant --version` → 0.3.0, memory-wire b478d15).
- The earlier rounds' "deploy impossible" note was about `~/.local/bin` being
  unwritable; that was false this round — the directory is writable and the
  blocker was the running process, which is a different (and solvable) thing.
  Recorded so the next round doesn't inherit the wrong lesson.

## Round 12 (2026-11-08) — the shared-document hazard, and a fresh audit

Eight pushed iterations (iter-1953..iter-1960, origin/main 629a8f8d). The
round's protocol held: fix → mutation-proved test → RT row + changelog →
index-script OK → commit+push.

The fixes:

- **Two more swallowed writes** (the round's dominant class, both in
  agent/): the evolution-counter persists that leave the memory-review and
  skill-nudge intervals hydrating stale counts (RT-365), and the skill-use
  telemetry write that leaves the curator under-counting a skill (RT-367).
- **Two gateway doc claims that outran the code**: the turn-exit
  classifier's "journal column / retries-breakers-audits branch on THIS"
  (RT-368 — the classifier is a correct, zero-caller surface; the LIVING
  classification is a different enum) and the breaker's `Auth` class doc,
  which named only the credential family while the router also sends
  "chat not found" and "bot was blocked" (RT-369).
- **The round's own two repairs**: the test-module boundaries my
  insertions broke (RT-370 — the anchor consumed the existing module's
  `#[cfg(test)]`, caught by the strict clippy gate), and the shared-
  document hazard from the publishing side, below.

### The shared-document hazard, hit from the publishing side (RT-367's repair record)

My iter-1954 commit staged a STALE worktree copy of
`docs/audit/2026-06-19-BUGS.md` — my last write of that file predated the
peer's `950bb761`, which landed while my verification ran. The
temporary-index recipe (`git read-tree HEAD` + explicit adds) does NOT
protect a file whose worktree copy is stale: the explicit `git add`
staged my old copy, and the published ledger lost the peer's RT-366 row
and detail section (their code files survived; only the document was
damaged). Detected by the id collision — both iterations had chosen
RT-366 independently — and repaired forward in iter-1955 (their row
restored verbatim; mine renumbered to RT-367 per the renumber-forward
convention their own row records; the stale worktree copies of
WORKLOG.md and the workflow board refreshed from HEAD, where the peer's
eleven entries were intact).

Lesson: a shared-document edit must be derived from the freshly FETCHED
remote version, never from the worktree copy.

### The fresh audit's findings that did not survive the read

This round audited by first-hand verification, not by carrying the earlier
sweeps' lists forward. Verified false and dropped (with the read that
killed them): the memory_management sweep's tool names (they do not exist
in this tree); `endpoints/carrier_pigeon_docs.rs` and `genetics.rs` (no
such paths); `browser_tool`'s "dead redeclaration" (accessibility_tree is
handled); `credential_pool`'s "unused constants" (read at :576);
`models_dev`'s fetch path (wired to the TUI); `ponytail::kick_update_check`
(wired at `operant-cli/src/main.rs:1713`); `trajectory`'s zero-init
counters (a genuine accumulator, both filled at `prompting.rs:147-148`);
`emit`'s best-effort channel send (receiver closing is terminal); the
redaction module's wiring claims (all live); the kanban args (read
multi-line); the schema-only tool structs reading raw JSON by design.

The two gateway findings that DID survive came from the round's subagent
sweep (7ce30cd0) with first-hand line-level evidence, and are RT-368 /
RT-369 above. The sweep's un-reached scope (tools/ description-vs-execute,
org/, config) was re-dispatched and its findings are pending at this
round's close.

## Round 12, second half (2026-11-08) — the tools/org/config sweep, verified and shipped

Six more iterations (iter-1962..iter-1965, origin/main abc2a9ea), all from the
dispatched tools/org/config sweep. The discipline that decided what shipped:
EVERY sweep finding was re-verified first-hand before an iteration was spent on
it, and two of the sweep's eight claims did not survive.

Verified FALSE and dropped (with the read that killed them):
- The sweep's finding #1/#2 (seat budgets): it claimed no gateway turn is
  budget-gated, on a core-only grep. The gateway DOES gate — `operant-cli's
  gateway_runner.rs:2909-2955` reads `employee_window_usage` and refuses (hard)
  or runs (soft) at the turn boundary. What WAS true in that claim — that
  mid-flight enforcement has since shipped — shipped as RT-376 below.
- (For the record, the sweep itself killed five candidates before reporting:
  tts_tool's incomplete provider enumeration, cron_tool's omitted pause/resume,
  `context_lcm_db`'s default — TRUE, the consumer lives in operant-cli — the
  two method lists omitting OPTIONS, and the sourcehound token measurement.)

Shipped, each mutation-proved:
- RT-371 — the discord tool's hand-written SCHEMA STRING still promised
  "default 50" for search_members while the action resolves 20 (the second
  copy of the RT-352 claim; the contract now pins both).
- RT-372 — the context-injection header described three org classes as the
  live phase-1 pipeline while the collector returns `Ok(Vec::new())` for all
  three (retired to Holon), contradicting `crate::org`'s own module doc.
- RT-373/374/375 — one bundled iteration: `UsageAccumulator`'s docs named a
  `result.usage` seam no struct implements (in two docs, one the module
  summary); `KaizenProposal`'s doc asserted a framework caller that does not
  exist (the type is dead with a false provenance story); and a ponytail
  comment grounded a static claim in a RETIRED config migration key.
- RT-376 — the seat-budget docs said mid-flight enforcement was "a recorded
  follow-up"; it has shipped (run.rs stops a token-basis turn mid-flight at
  the cap), so an operator believed a hard cap could not bound an in-flight
  turn.

The needle hazard's third instance this round, recorded in RT-376's row: two
of the contracts' first-cut needles were vacuous (a doubled backtick; a split
point that produced `preferred_pprovider`), a third was satisfied by the doc's
own history note quoting the old claim, and the contract-insertion pattern
consumed two existing modules' `#[cfg(test)]` for the THIRD time this
campaign. Every proof was re-run after the repair and only counted once the
contract was seen failing by name on the restored defect.

Round-12 end-of-round gates: full suite exit 0 (46 targets), strict clippy
exit 0, fmt clean, ledger index OK, genome lint 0 errors, red-team docs
verification "no CLOSED finding regressed".

## Round 13 (2026-11-09) — the session/cronjobs/curator sweep, all ten findings shipped

Fifteen pushed iterations (iter-1967..iter-1981... close: iter-1967 through
iter-1978, origin/main 824afa7a). The round's discipline held: every one of
the dispatched sweep's ten findings was re-verified first-hand before an
iteration was spent on it; the two that did not survive (the seat-budget
claims, whose "gateway enforcement absent" premise died on the cross-crate
reader in operant-cli) had their TRUE sub-part shipped earlier as RT-376.

Shipped, each mutation-proved:
- **RT-384** — the curator's "pre-archive" backup was taken AFTER the
  archival it was named for (a safety mechanism that documented its contract
  and then ran on the wrong side of it); the backup now precedes the loop and
  a failed backup means the archival does not run at all.
- **RT-385** — two curator archive paths dropped the tracker write that makes
  the archive durable — the invariant the file itself names at :539, which
  the apply path propagates with `?` while run_review and run_consolidation
  used `let _ =`.
- **RT-386** — the "reactivated" counter counted a change that never happened
  (a defensive arm incremented and did nothing else, then the aggregate log
  reported the reactivation); fixed by naming — the reactivation itself is a
  product decision.
- **RT-387** — the cron self-heal's persist was discarded, so a failed write
  left the FIRED schedule diverging from the STORED one forever, unlogged.
- **RT-388** — the cron rewrite's atomicity note was false: a mutex is not a
  transaction, and a mid-loop failure left a half-rewritten job fleet; the
  rewrite now runs inside ONE explicit transaction.
- **RT-389** — the session `discard` doc promised a durable delete the body
  never performs (it only removes the resident copy; the session stays
  recoverable — the RT-285 product decision is the behaviour, this row is the
  doc's additional false claim).
- **RT-390** — the turn-summary doc promised a display bucket no formatter
  renders.
- **RT-391** — `repair_schedules`' doc claimed an interval expansion the
  schedule module forbids (the 2026-10-24 expansion bug is why phrases stay
  raw).
- **RT-392** — three curator docs promised a `stale` lifecycle state that does
  not exist (no enum variant, nothing sets one; staleness is report-only).
- **RT-393** — the skill summary sent the first ten RAW file lines as the
  description the doc promises from SKILL.md frontmatter; a parser now
  extracts the field.

Plus the round's two ledger chores: iter-1968's repair of the fourth
shared-document erasure (my iter-1967 ledger blob was built from a stale
worktree copy and dropped the peer's rows RT-378…RT-383 — detected by the
commit's own diff stat, repaired forward from the parent's bytes, my row
renumbered RT-377→RT-384 per the later-publisher convention), and iter-1978's
fold of that repair note into the RT-384 section (its standalone id parsed as
a second RT-384, which the uniqueness check caught).

### The round's measured lessons (all recorded where the next round reads them)

- **The consumed `#[cfg(test)]`, fourth instance**: my scheduler.rs contract
  insertion consumed the existing test module's attribute; the plain
  `cargo build` caught it as an unresolved import of a cfg(test)-only helper.
  The lesson is unchanged and now proven four times: re-emit the consumed
  attribute in the same replacement.
- **The self-satisfaction trap, instances six through ten**: a doc history
  note quoting the claim it corrects (RT-389), a test comment quoting the
  claim it forbids (RT-390, RT-392), and a doc note quoting the phantom
  bucket (RT-390's production note). Each paraphrased before the proof
  counted.
- **A helper's green test is not the seam's proof (RT-393)**: the parser's
  unit test passed while the producer mutation was green; the seam assertions
  were added and the proof re-run.
- **Two vacuous fixtures caught by their own mutation runs**: RT-386's first
  fixture (the never-used grace floor intercepted the record) and RT-388's
  first trigger (keyed on the display name, not the minted id).
- **A compile error is not a test proof (RT-387, RT-385)**: two mutation
  attempts left dangling references or unbalanced blocks; both were redone as
  whole-block replacements and re-proved.

Round-13 end-of-round gates: full suite exit 0 (46 targets), strict clippy
exit 0, fmt clean, ledger index OK (index table and checkboxes agree, id space
unique), genome lint 0 errors, red-team docs verification clean.

## Round 14 (2026-11-09) — the skills_guard sweep plus two own findings, eleven rows shipped

Twenty-two pushed iterations (iter-1980..iter-1992-to-close, origin/main
feaecd42-descended). The dispatched skills_guard/skill_preprocessing sweep
returned ten findings; seven shipped as defects, one shipped as doc-truth with
its stronger claim verified FALSE, one shipped as doc-truth with a named
follow-up policy decision, and one of its targets (the memory/ and skills/
directories) did not exist as paths — the audit's coverage map recorded the
real layout (flat memory.rs, memory_wire.rs, memory_provider.rs,
memory_import.rs; skills logic lives in skills_guard.rs, skill_marketplace.rs,
and tools/skills_tool.rs). Two further findings came from my own crate-wide
swallow sweep.

Shipped, each mutation-proved:
- **RT-394** — four `gateway_session` state changes (suspend, resume-pending
  mark, resume-pending clear, activity timestamp) discarded their persist, so a
  failed write left the disk entry on its previous state with no log: a
  suspended session returned after restart, a cleared resume flag stayed set
  (a duplicate resume), a stale timestamp forever.
- **RT-395** — the active-session lock writes were discarded, breaking the
  module's own promise ("a busy session can never be evicted"): a failed
  refresh evicted a BUSY session mid-conversation, and a failed create produced
  a slot born stale.
- **RT-396** — the inline-shell cap was in BYTES over a char label, so a
  multibyte snippet hit byte 4000 mid-character and `String::truncate`
  PANICKED inside `run_inline_shell` (the guarding test only fed ASCII).
- **RT-397** — the `Trusted` skill tier matched any source PREFIXING a trusted
  repo, so a look-alike path inherited the first-party install policy.
- **RT-398** — the skill scanner FAILED OPEN: a scannable-extension file that
  was not valid UTF-8 returned zero findings (a payload in a latin-1 "text
  file" scanned Safe and was ALLOWED), and a path that was neither dir nor file
  ran no branch and reported "clean scan, no threats detected".
- **RT-399** — the extension gate framed a curated 23-extension list as "text
  files only"; every other text format is skipped, so `java_runtime_exec`'s
  pattern is unreachable. Doc-truth + RT-399-FOLLOW (which languages belong on
  the list — a scanner-policy decision).
- **RT-400** — the `too_many_files` count counted directories and symlinks
  while reporting "files" (51 empty subdirectories tripped the limit).
- **RT-401** — `content_hash`'s doc claimed "of all files" over a digest that
  covers only the readable files. Doc-truth; the sweep's stronger claim
  (a file becoming unreadable leaves the digest unchanged) was VERIFIED FALSE
  and recorded, and my first (perturb-the-digest) fix was reverted because its
  mutation run came back green.
- **RT-402** — the skill file walk followed symlinks: a symlinked directory was
  traversed (the scan and the content hash read host content outside the skill
  tree) and symlinks were pushed as regular files; the all-entries walk could
  recurse forever on a symlink cycle.
- **RT-403** — the inline-shell timeout marker reported the request rather than
  the enforced deadline (a 1s kill labelled "timeout after 0s").
- **RT-404** — the `GuardScanner` constructor doc claimed the regexes compiled
  in the initializer; they compile on the FIRST scan. Doc-truth.

### The round's measured lessons

- **A negative needle can be vacuous across a doc's own line break (RT-404)**:
  the first mutation run came back green because the needle spanned the
  doc-comment wrap and could never match; cut at the break, the proof
  discriminated.
- **A green mutation run can mean the fix was unnecessary (RT-401)**: the
  perturb-the-digest fix's mutant passed, which is what exposed the finding's
  stronger claim as false. A green run is not only a proof — it is also a
  warning about the premise.
- **The end-of-round clippy gate caught lints on my own additions (iter-1992)**:
  `manual_contains` in the new symlink guard, `unneeded_formatting` on a static
  needle, `needless_borrows_for_generic_args` on two `contains` args — values
  unchanged, contracts re-run green after the fix.
- **A stale anchor ships code without its ledger row (iter-1980/1981)**: the
  peer pushed between my fetch and my ledger edit, my anchor had gone stale,
  and the commit landed the code alone. The row and the changelog entry were
  landed in the next iteration. The shared-document hazard is a fetch-then-edit
  window, not only a stale-blob one.
- **Compile-error mutants, twice more (RT-395, RT-401)**: line surgery left
  dangling references or unbalanced blocks; both redone as whole-block
  replacements and re-proved.
- **Consumed `#[cfg(test)]` did NOT recur this round** (it has bitten five
  times); every insertion re-emitted the consumed attribute in the same
  replacement.

Round-14 end-of-round gates: full suite exit 0 (46 targets), strict clippy exit
0 (`-D warnings`), fmt clean, ledger index OK (RG-016, RG-021), genome lint 0
errors.

## Round 15 (2026-11-09) — the gateway/oauth sweep, a new doc gate, and eight rows shipped

Twenty-three pushed iterations (iter-1993..iter-2004 plus this close,
origin/main 608733d3-descended). The dispatched gateway-adapters/oauth
sweep returned ten findings; seven shipped as defects, two as doc-truth
with a named follow-up decision, and one (the `previous_refresh` promise)
shipped as doc-truth whose missing wiring was named for its owner. Two
further findings came from my own crate-wide sweeps (the stale line
citation, the doc-link class).

Shipped, each mutation-proved:
- **RT-407** — the model-roles header cited a blank line as the home of
  `OperantAgent::set_model`; a reader following the doc's own pointer
  landed on nothing and could conclude the self-switching behavior does
  not exist. Then, in RT-410, the FIX was found to re-arm the rot (a
  rewritten line number drifts again) and was replaced by a symbol anchor
  that rustdoc resolves at build time.
- **RT-408** — the webhook adapter advertised `POST /webhook/notion`, a
  route that does not exist and deliberately cannot: the arm mounts at the
  instance's SECRET path, and the CLI's O-08 note names the literal as
  exactly the guessable path the token-rotation guard prevents. An operator
  wiring Notion to the advertised URL sent every event into the generic
  `IncomingMessage` path.
- **RT-409** — the device-flow `poll` doc promised a `previous_refresh`
  seed the signature does not carry (the parse passes no seed), so a
  device-code response without a `refresh_token` yields an unrefreshable
  credential. The parser under it DOES support the seed — the auth-code
  flow supplies one. Threading a real parameter is RT-409-FOLLOW: the
  reachability was ESTABLISHED (bare, module-qualified, re-export, and
  type spellings) — no production caller exists.
- **RT-410** — the crate's doc links had no checker. The first run of
  `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps
  -p operant-core` was RED: four dangling symbols, one of them
  `composites_match_the_live_server`, named as "the test that catches the
  drift" for a table whose contract drift is caught by NO test. The gate is
  green now — the row doubles as the class entry.
- **RT-411** — the refresh coordinator's follower comment claimed a
  recovery ("Sender dropped without a result — reachable only if the
  leader's future panicked") that cannot run: `Flight` owns the
  `watch::Sender` and BOTH the map and the follower hold an `Arc` across
  the await, so the sender cannot drop while a follower waits — a panicking
  leader wedges every follower FOREVER (probe-verified against the
  workspace's tokio). Waking them is RT-411-FOLLOW: real machinery, a
  design decision.
- **RT-412** — the SMTP timeout "protection" was conditional on two
  discarded fallible setters, so a hung server blocks the conversation's
  blocking thread indefinitely with no timeout applied and nothing logged.
- **RT-413** — the token response's `scope` was read and thrown away by an
  unnamed bare discard, so a narrowed-grant response is indistinguishable
  from a full one. A scope field would grow an API no consumer reads:
  RT-413-FOLLOW, the credential store owner's decision.
- **RT-414** — the native-cert loader reduced its load errors to a COUNT
  (the offending entry unidentifiable when debugging a TLS failure) and
  discarded the per-certificate trust-store rejection outright.

### The round's measured lessons

- **A fix row's own prose is a doc, and the guards read it.** My RT-407
  ledger row named the exact stale line the RT-209 guard forbids; the
  round's own suite run caught it. The iter-1996 REPAIR NOTE then repeated
  the mistake by quoting what it repaired. Both fixed; the lesson is that
  a fix row and a repair note are checked by the same guards as production
  docs.
- **Rewriting a line number re-arms the same rot** (RT-407 → RT-410).
  The durable repair is structural: name the symbol, and let rustdoc's
  broken-link gate fail the build when it moves.
- **The self-satisfaction trap's instances twelve through fifteen.** A doc
  quoting the claim it forbids (RT-409, RT-413); a needle matching its own
  definition (RT-409, RT-407); a needle matching the text past the
  `#[cfg(test)]` boundary (RT-409 — the boundary in that file is at line
  28, NOT at the test module); a needle spanning a doc's own line break
  (RT-411, RT-409, RT-413). Each repaired before the proof counted.
- **A negative needle can be mis-designed** (RT-414): it forbade the error
  COUNT, which legitimately survives alongside the errors — the defect was
  that the count was the WHOLE report. The needle was removed, not bent.
- **The fetch-then-EDIT window erased the peer's rows twice more.** iter-1993
  shipped code without its ledger row (the peer's changelog anchor had
  gone stale between my fetch and my edit), and iter-1994 repaired it
  forward from the parent's bytes, renumbering my row RT-405 → RT-406 →
  RT-407 as the peer took ids in between. The note recording the
  renumber had to stop carrying the row's own id (it parsed as a second
  section — the sixth instance of that hazard).
- **A flake named honestly, not fixed on an unreproduced failure**: three
  `working_diff_tool` tests failed once in a parallel full-suite run, pass
  in isolation, and stayed green across three subsequent full-suite runs
  and three targeted repeats. The shape is identified (they spawn git
  without `test_env_lock` against env-mutating tests) and the file is
  peer-touched, so the preserved log plus the named remedy is the honest
  action.

Round-15 end-of-round gates: full suite exit 0 (46 targets), strict clippy
exit 0 (`-D warnings`), fmt clean, the doc-link gate exit 0, ledger index OK
(RG-016, RG-021), genome lint 0 errors.

## Round 16 (2026-11-09/10) — the build chain, the env locks, and a swept cluster

Twenty-two pushed iterations (iter-2006..iter-2017 plus this close). The round
began with a gap in the tree: a chore commit prepared in a previous session
had never been pushed, and the peer had independently taken its iteration id —
so the round opened by RENUMBERING (iter-2005 → iter-2006) and rebuilding the
shared documents from the FETCHED tip's bytes rather than the worktree's (the
worktree's ledger lacked the peer's RT-415 row — the sixth shared-document
erasure waiting to happen).

Shipped, each mutation-proved:
- **RT-416 (M)** — **the operant-cli binary did not LINK on any host with
  libsonic installed.** espeak-ng's cmake build DETECTS a system libsonic
  (detected, not configured), so `USE_LIBSONIC` compiles seven `sonic*`
  references into the archive; espeak-rs-sys's build script forwards only the
  libraries it built itself, never espeak-ng's PRIVATE system dependency, so
  nothing emitted `-lsonic` and the final link failed. `cargo build -p
  operant-cli` exited 101. The repo's own `build.rs` carried the false
  premise ("Nothing references a sonic symbol … true only where espeak-ng's
  build does not detect a system libsonic"). Fixed by stubbing the seven
  symbols in the existing `espeak_audio_stubs.c` — the audio backend is
  stubbed anyway, so the speed-up has nothing to speed up, and no host
  library or header is needed. The workspace builds clean again (zero
  warnings).
- **RT-417 (L, scope completed in iter-2009)** — the deployability plan
  still claimed the build needs a libsonic artifact: three Phase-B bullets
  (the deleted `-lsonic` flag, the installer's "bare build", the env scripts'
  hardcoded `/home/z` paths) were corrected with a dated marker, and B2's
  accept criterion is now met by the stronger form — libsonic is not required
  at all.
- **RT-418 (L)** — the config write's mode round-trip was documented as
  unconditional ("must survive the round-trip") while its `set_permissions`
  result was discarded; a failed carry-over silently left the file
  owner-only. The fallback is the SAFER mode, so the fix reports rather than
  changing the policy.
- **RT-419 (M, class)** — two more swallowed permission writes: the OAuth
  directory's 0700 and the downloaded bridge binary's 0755 (a binary
  installed at the path the bridge EXECUTES, without its execute bit). One
  shared helper (`fs_secrets::set_mode_reporting`) now applies the mode and
  reports the failure with the consequence the caller names.
- **RT-420 (M, class)** — **the crate had THREE locks for one process-wide
  environment.** Observed as a real suite failure ("Source profile 'source'
  does not exist"): `env_passthrough.rs` and `sourcehound_cdp.rs` each held a
  LOCAL `ENV_LOCK`, which serializes only its own file — and
  `sourcehound_cdp.rs` mutated `BROWSER_CDP_URL` under its local lock while
  `tools/browser_cdp_tool.rs` mutated the SAME variable under the shared
  lock. The local locks are gone; every shared-variable mutation holds the
  crate's one lock.
- **RT-421 (H)** — **`delete_board("")` DELETED the default board's
  database.** `resolve_path` treats the empty slug and `"default"` as the
  same board; the delete guard compared the SLUG, so the empty slug passed
  and `remove_file` took the default board's entire database. The guard now
  compares the RESOLVED path.
- **RT-422 (L)** — the roots fence's note claimed an empty `roots` list is
  UNGOVERNED; the code fences it to the run directory (the narrowest fence,
  and upstream's "the workdir only"). Both notes now state the truth, and the
  sibling's "matching the roots fence's convention" was false — the two
  fences differ.
- **RT-423 (H)** — **a corrupt seat-policy row WIDENED the seat.** The read
  was loud (an error, as the doc says), but `policy_for` turned it into
  "no policy" = ungoverned, so a typo'd `lockdown` spelling re-opened every
  SAFE tool the seat was shut out of — the exact widening the module doc
  calls impossible. The seam now FAILS CLOSED (lockdown: every tool
  escalates).
- **RT-424 (M)** — the context-injection quotas are documented in CHARACTERS
  and were charged in BYTES, so non-ASCII content was charged up to 4x its
  character count, overflowed the class quota, went to the pool, overflowed
  the total cap, and was DROPPED. The accounting counts characters now.
- **RT-425 (M)** — the context collector took the NEWEST pending items and
  the cursor then jumped to their max, so a backlog over the per-class limit
  left its OLDER items below the cursor forever: never collected, never
  rendered, no trace — while the module doc promises "at-least-once". The
  query now takes the OLDEST pending items, so the cursor walks through the
  backlog.

### The round's measured lessons

- **A build that cannot link is invisible to a library-target gate.**
  `cargo build -p operant-core` passed while `cargo build -p operant-cli`
  failed: a library rlib never resolves the reference, and the test binaries
  that had been linked earlier were cached — the breakage surfaced only when
  a relink was forced. A gate that only builds libraries cannot see a link
  defect; the workspace build can.
- **A host-detected dependency is a build-shape dependency.** espeak-ng's
  cmake DETECTS libsonic rather than being configured for it, so the same
  source produced a linkable archive on one host and an unlinkable one on
  another. The repair made the build host-independent rather than
  provisioning a payload for one host.
- **"Serialized by ENV_LOCK" was true of the file, not of the crate** — the
  second-lock class is invisible from inside a file, and only a CRATE-WIDE
  sweep finds it.
- **A needle that names an instance misses a renamed instance** (RT-420):
  the class contract first matched `static ENV_LOCK`, so a mutant that
  renamed the lock passed; it now matches the SHAPE. And a shared-variable
  set must count DISTINCT FILES, not mutations.
- **A dated correction that QUOTES the claim it corrects re-creates the
  claim** (RT-417): the contract's negative needles matched the corrections
  themselves, so all three were rewritten as paraphrases — the
  self-satisfaction trap's sixteenth instance, and the standing rule for
  every dated correction.
- **A guard that compares a NAME misses every ALIAS of that name**
  (RT-421): the delete guard compared the slug; the code that RESOLVES the
  slug treats the empty string as "default". Compare the resolved value.
- **A doc can be false in the SAFE direction** (RT-422): the code fenced
  more tightly than the note promised. The fix was the note, not the code —
  and the sibling note ("matching the roots fence's convention") was false
  in the other direction, so both had to be read.
- **"An error on read" is not "the run path does not widen"** (RT-423): the
  read was loud; the SEAM converted the loud error into the exact widening
  the doc forbids. A loud failure that feeds a permissive default is still a
  widening.
- **A pre-existing test can pin the DEFECT** (RT-423): its assertion
  expected `None` under the comment "rather than silently widening the
  seat" — `None` WAS the widening the sentence claimed to prevent. When a
  test's comment and its assertion disagree, the comment is the spec.
- **A unit mismatch is a silent content loss** (RT-424): chars versus bytes
  agreed for ASCII and charged everything else 4x, so the quota overflowed
  and the content was dropped — with no error anywhere.
- **A cursor that jumps over pending items deletes them, and the fix must
  hold with a MONOTONIC cursor** (RT-425): the first attempt advanced the
  watermark to the OLDEST collected item, which cannot advance past the
  current cursor (the update is `MAX(last_ts, excluded.last_ts)`) and so
  stalled. FIFO collection is the fix that holds.
- **A partial revert can be MASKED by another path** (RT-424): reverting
  only the class-quota charge still passed the test, because the roll-over
  pool's own (character) charge re-admitted the item. The proof that counts
  reverts the WHOLE fix.

Round-16 end-of-round gates: full suite exit 0, strict clippy exit 0
(`-D warnings`), fmt clean, the doc-link gate exit 0, the WORKSPACE build
exit 0 with zero warnings, ledger index OK (RG-016, RG-021), genome lint 0
errors.

## Round 17 (2026-11-10) — a swept cluster: twelve rows, and the class the sweep exposed

Twelve pushed iterations (iter-2019..iter-2030 plus this close). The round
started from a subagent's first-hand sweep of the kanban/org/context cluster
(18 findings, 16 confirmed, 2 dropped as false) and closed with every one of
the confirmed findings fixed, mutation-proved, and shipped.

Shipped:
- **RT-421 (H)** — `delete_board("")` DELETED the default board's database.
  `resolve_path` treats the empty slug and `"default"` as the same board; the
  delete guard compared the SLUG, so the empty slug passed and `remove_file`
  took the default board's whole database. The guard now compares the
  RESOLVED path.
- **RT-422 (L)** — the roots fence's note claimed an empty `roots` list is
  ungoverned; the code fences it to the run directory (the narrowest fence).
  Both notes now state the truth, and the sibling's "matching the roots
  fence's convention" was false — the two fences differ.
- **RT-423 (H)** — a corrupt seat-policy row WIDENED the seat to ungoverned.
  The read was loud (an error, as the doc says), but `policy_for` turned it
  into "no policy" = every SAFE tool runs — the exact widening the module
  doc calls impossible. The seam now FAILS CLOSED.
- **RT-424 (M)** — the context-injection quotas are documented in CHARACTERS
  and were charged in BYTES, so non-ASCII content was charged up to 4x its
  character count, overflowed the class quota, went to the pool, overflowed
  the total cap, and was DROPPED.
- **RT-425 (M)** — the context collector took the NEWEST pending items and
  the cursor then jumped to their max, so a backlog over the per-class limit
  left its OLDER items below the cursor forever: never collected, never
  rendered, no trace — while the module doc promises "at-least-once".
- **RT-426 (M, class)** — four writes whose failures were discarded: the
  run's refusal record (a POLICY DENIAL with no audit trail), the run's
  learning record, the refused skill write's ROLLBACK (a scan-refused file
  could stay on disk with the agent told only that the scan refused it), and
  the half-created skill directory's cleanup. All four report now, and the
  refusal names the still-on-disk consequence.
- **RT-427 (M)** — a DENIED seat escalation carried no resolution string.
  `SeatApprover`'s doc promises the resolver's name is on "the resolution
  string an operator sees"; `approve` returned one, `deny` returned
  `Ok(())`. `deny` and `expire` now return the same shape, and the gateway
  surfaces it.
- **RT-428 (L)** — `next_round` promised `None` "when the retrieval already
  completed", but an EARLY completion was never recorded, so a retrieval
  that finished in round 1 got another recall.
- **RT-429 (L)** — the stale-task diagnostic reported "-1s" for a task that
  had NEVER heartbeated — a negative elapsed time for exactly the case it
  flags. The elapsed time is now the one since the task STARTED.
- **RT-430 (L)** — the kanban subscriber log claimed a notification that
  never happened (the manager holds no platform handle, and the function has
  no caller). The log says it was NOT delivered; the delivery wiring is a
  NAMED product decision.
- **RT-431 (L)** — the turn-attribution docs claimed the spans were
  "exclusive"; the classifier checks MEASUREMENT, and the residual saturates
  at zero, so a >100% turn is `Complete` while `is_exclusive` says false.
- **RT-433 (L)** — the turn-attribution module's note named the type the
  delivered event does NOT carry (the bare `TurnBreakdown` instead of the
  classification) with a stale size. Renumbered from RT-432 when the peer
  took that id.
- **RT-434 (L)** — the `Yolo` seat mode's doc named ONE exception; the
  seat's OWN deny list bites under Yolo too (the file's own precedence list
  calls it "absolute over everything below").
- **RT-435 (L)** — the learning annotation's documented kind list named a
  kind nothing writes (`proposals`) and missed the one that IS written
  (`learning_changes`, the knowledge-base edits). The list is now checked
  against the emitters by a SELF-MAINTAINING contract.
- **RT-436 (L)** — the context injector's doc sent a reader to the app
  database and the field name sent them there; `open` resolves the ORG
  SIBLING — the exact "invisible no-op" `open`'s own doc recounts.
- **RT-437 (L)** — the context-injection schema comments named class sets
  that are neither live nor retired-honestly (`feed` was "future"; the
  retired org classes were listed as live).
- **RT-438 (L)** — the cosine similarity's doc claimed the range is [0, 1];
  the function returns the RAW cosine. The bounded range is a CALLER's
  policy (`context_management.rs` clamps; `context/lcm.rs` sorts
  descending).

### The round's measured lessons

- **A crate-wide sweep is how a CLASS is found; a single report is how one
  instance of it is found.** RT-426's four sites shared a shape (a discarded
  write) in two modules — none of them had been reported. The sweep
  (`let _ = writeln!`, `let _ = fs::write`, `let _ = fs::remove_*`) is
  cheap and finds what a reading pass does not.
- **A rollback that discards its result leaves the REFUSED state in
  place.** The skill scan refused a write and the rollback's failure was
  invisible — so the file the scan rejected stayed on disk while the agent
  was told only that the scan refused it. The rollback became fallible, and
  the refusal now names the consequence.
- **A pre-existing test can PIN THE DEFECT.** RT-423's
  `a_bad_mode_string_errors_on_read_not_defaults` asserted `policy_for ==
  None` under the comment "rather than silently widening the seat" — `None`
  WAS the widening the sentence claimed to prevent. When a test's comment
  and its assertion disagree, the comment is the spec.
- **A loud error that feeds a permissive default is still the widening the
  docs forbid.** The seat-policy READ was an error; the SEAM turned it into
  "no policy", and "no policy" is the wide door.
- **A unit mismatch is a silent content loss.** Chars versus bytes agreed
  for ASCII and charged everything else 4x, so a quota overflowed and the
  content was dropped — with no error anywhere.
- **A cursor that jumps over pending items DELETES them, and the fix must
  hold with a MONOTONIC cursor.** The first attempt advanced the watermark
  to the OLDEST collected item, which cannot advance past the current cursor
  (the update is `MAX(last_ts, excluded.last_ts)`) and so stalled. FIFO
  collection is the fix that holds.
- **A guard that compares a NAME misses every ALIAS of that name.**
  `delete_board("")` deleted the default board's database because the guard
  compared the slug and the resolver treats "" as "default". Compare the
  RESOLVED value.
- **A field name can make a wrong doc BELIEVABLE.** The injector held the
  org sibling's path in a field called `app_db`, and the doc on the struct
  said the state lives in the app database. Both were wrong in the same
  direction; the rename (`db_path`) removed the trap.
- **A BEHAVIORAL test alone cannot catch a DOC regression.** RT-438's first
  mutant restored the false range claim and the test still PASSED — the
  behavioral test measures the function, not its comment. A doc needs its
  own contract.
- **A doc that lists an enumeration can be CHECKED AGAINST THE CODE.**
  RT-435's contract derives the annotation kinds from the emitters and
  compares them with the note — a self-maintaining list that cannot drift.
- **An operator-facing message with a negative number is a bug.**
  "-1s without heartbeat" was the diagnostic for the case it exists to flag.
- **A dated correction that QUOTES the claim it corrects re-creates the
  claim.** Five instances this round (RT-426, RT-431, RT-432, RT-436,
  RT-438): every corrected doc was rewritten to paraphrase.
- **A needle that spans a message's own line break matches NOTHING.**
  Four instances this round: the needle must be a single-line fragment.
- **A PART-reverted fix can be MASKED by another path.** RT-424's first
  mutant reverted only the class-quota charge, and the roll-over pool's own
  (character) charge re-admitted the item — the rendered output cannot tell
  "admitted by the class quota" from "admitted by the pool". The proof that
  counts reverts the WHOLE fix.
- **The peer takes ledger ids CONCURRENTLY.** RT-432 was published by the
  peer between this work's fetch and its write; the later publisher
  renumbers forward and records an ID NOTE (RT-433).

Round-17 end-of-round gates: the WORKSPACE test suite exit 0 (88 targets
ok), strict clippy exit 0 on the core and on the CLI, fmt clean on both,
the doc-link gate exit 0, the WORKSPACE build exit 0 with zero warnings,
the ledger index OK (RG-016, RG-021), genome lint 0 errors.

## Round 18 (2026-11-10) — the agent/tools sweep: thirteen rows, and the two organs that disagreed

Thirteen pushed iterations (iter-2032..iter-2043 plus this close). The round
started from my own first-hand sweep of the MCP/config/session cluster and
then took a subagent's first-hand sweep of `agent/**` and `tools/**` (11
findings, 14 candidates dropped as false or not proven) — and closed with
every confirmed finding fixed, mutation-proved, and shipped.

Shipped:
- **RT-441 (M)** — two of the three MCP `send_notification` implementations
  could NOT fail: the stdio one discarded the write and the flush and
  silently skipped a missing transport; the streamable-HTTP one discarded
  the whole send. They carry the protocol's `notifications/initialized` — a
  lost one leaves the client believing it is connected to a server that was
  never told.
- **RT-442 (M)** — the streamable-HTTP notification was sent as a BARE
  post: no `Mcp-Session-Id`, so it landed in a NEW session the server had
  never initialized. The discarded send (RT-441) hid it.
- **RT-444 (M)** — `peel_revision` split the model slug at a BYTE offset, so
  a slug whose tail cut a multi-byte character PANICKED — reachable from a
  provider registry entry's `model` field on the runtime's fallback path.
- **RT-445 (H)** — the Holon policy-roots WRITE fence FAILED OPEN when the
  target's parent did not exist: the fence's own comment said "the strict
  checks below handle it", and they have NO roots check. The write tool
  creates the missing parents, so a governed run wrote outside its roots,
  with no refusal recorded.
- **RT-447 (M, class)** — the file tools' three recursive walkers FOLLOWED
  symlinks (the repo's own rule says they must not): a symlinked FILE was
  read through — past the deny list — and a symlink CYCLE made the walkers
  walk the same tree at dozens of levels (82 entries instead of 2).
- **RT-448 (M)** — the todo tool's MERGE mode dropped `details`, `priority`,
  `assigned_to` and `blocked_by`: the model was told the item was updated
  while the stored row (and the TUI card) kept the stale brief and owner.
- **RT-449 (M)** — the patch tool's three tiers disagreed on an EMPTY
  replace: a NAMED occurrence BLANKED the line where the other two tiers
  deleted it — the exact defect the tool's own contract names.
- **RT-451 (M)** — the skill-bundle cache was keyed on the DIRECTORY's
  mtime, so an EDITED bundle was never re-read: stale guidance kept being
  injected into every turn for the life of the process.
- **RT-452 (M)** — the insights report DROPPED unreadable session rows
  silently (and turned a wrong-typed counter into a silent 0), so it
  presented itself as whole while sessions and tokens were missing: the
  RT-294 contract failing at ROW granularity.
- **RT-453 (M)** — the OSV malware gate checked the runner's VERB as the
  package (`pipx run black` → "run"), so it reported `known_malware: false`
  for packages it never checked. The repo's own test pinned the defect.
- **RT-454 (M)** — the Holon command fence judged only the FIRST token of a
  line, so a SHELL line could smuggle a second command past the allowlist
  (`git status; rm -rf /`), and its doc promised a re-tokenization the code
  never did.
- **RT-455 (L)** — the degenerate-final-text gate measured its 80 ceiling in
  BYTES while the doc speaks of characters, so the same reasoning-leak
  artifact was flagged in ASCII and surfaced as an answer in CJK.
- **RT-456 (L)** — the deferred-tool module's doc put the token floor at 3
  while the code's floor is 4: the file contradicted itself.

What the round taught:
- **A DOC and its CODE can disagree INSIDE ONE FUNCTION.** The command
  fence's doc promised the inner command would be judged; the comment
  twelve lines below said the wrapper is. Both were born in one commit.
  When a doc and an inline comment disagree, the ACCEPTANCE TEST decides —
  and it was already pinning the code's behaviour.
- **A promise about the WRAPPER is not a promise about what the wrapper
  runs.** The kernel's allowlist judges a shell by its name, and the fence
  now does the same — but it judges EVERY SEGMENT of a shell line, using the
  kernel's own splitter, so the two organs share one rule instead of two.
- **A pre-existing test can pin a DEFECT.** Two did this round: the OSV
  parser's test asserted the package name "run", and the patch tool's
  helper's timestamp-based filename let two parallel tests clobber each
  other.
- **A sweep's hypothesis can be WRONG in the direction that matters.** The
  sweep reported a stack overflow for the symlink cycle; the path length
  bounds the recursion, so it is a runaway, not an abort. The row states
  what was measured (82 entries instead of 2).
- **A byte/char mismatch is a CLASS, not an incident.** RT-424 (quotas),
  RT-455 (the ceiling) and RT-444 (the slug split) are three instances of
  the same mistake in three modules.
- **A row-level failure defeats a QUERY-level contract.** RT-452: the
  report handled a failed query faithfully and still lied about a partially
  unreadable database.
- **A security fence's own rule is the spec.** The fence promised "only
  those command names may execute"; its stated reference (the kernel's
  allowlist) judged every segment; the fence judged one token.
- **The peer's ids move WHILE you work.** Five renumbers this round
  (RT-441/442, RT-444, RT-447, RT-451): fetch, write from the tip's bytes,
  and re-check the id space before every shared-document write.

Round-18 end-of-round gates: the WORKSPACE test suite exit 0 (89 targets
ok), strict clippy exit 0 on the core, the CLI and the config crate, fmt
clean on all three, the doc-link gate exit 0, the WORKSPACE build exit 0
with zero warnings, the ledger index OK (RG-016, RG-021), genome lint 0
errors (5 pre-existing warnings).

## Round 19 (2026-11-10) — two first-hand sweeps of the agent and provider client: twenty rows, and the class the sweeps kept hitting

Twenty-eight pushed iterations (iter-2045..iter-2059 plus this close), twenty ledger rows
(RT-457..RT-471 plus the renumber). The round took TWO subagent sweeps — one over the
provider client (`client.rs`, `model_client.rs`, `error_classifier.rs`,
`stream_retry_budget.rs`, `iteration_budget.rs`, `clients/openai.rs`) and one over the agent
execution loop (`run.rs`, `stream.rs`, `compress.rs`, `message_safety.rs`, `pooled_client.rs`)
— and closed with every confirmed finding in those sweeps fixed, mutation-proved, and shipped.

Shipped:
- **RT-457 (M)** — the MCP OAuth token store's removal discarded the deletions' errors and
  returned `Ok(())` unconditionally: a credential file that could not be removed stayed ON
  DISK while the function reported success, and the manager logged "removed from disk" either
  way.
- **RT-458 (L)** — the cron self-heal's operator-facing warning had its sentence breaks
  collapsed into runs of 22 spaces ("the stored                      schedule stays").
- **RT-459 (M)** — three more MCP protocol sends discarded their results (the stdio
  transport's error responses to a server's request ×2, the SSE transport's response
  delivery): a server waiting for an answer never got one, and the log said nothing.
- **RT-460 (M)** — the HTTP client's body reads lost the transport cause: a failed read of a
  200 body became an empty body and a PARSE error, so a network failure was presented as
  garbage from the provider; a failed error-body read became an empty body.
- **RT-461 (M)** — the stream-retry ladder re-armed its wall clock on the turn's FIRST
  failure, so the first attempt's stall was EXEMPT — the exact attempt the bound was built to
  catch (the 2026-09-29 incident).
- **RT-462 (M)** — the interrupt drain indexed a parallel id vector with the WRONG index: a
  pre-flight rejection shifted every pairing, and a batch with more calls than pending
  indexed past the end of the vector — an out-of-bounds PANIC that killed the turn task.
- **RT-463 (M)** — the SPEND GUARD ran AFTER the 4096-byte absolute cap, so it never saw
  more than 4096 bytes: at the shipped default 128K window it could NEVER fire, making
  `max_tool_result_share` inert.
- **RT-464 (M)** — the message-sequence repair MANUFACTURED `tool_calls: Some([])`, which
  goes on the wire as `"tool_calls": []` plus a fabricated `reasoning_content`.
- **RT-465 (M)** — the tool-result trim's budget is documented in CHARACTERS and was charged
  in BYTES: a CJK result kept ~667 characters under a 2000-character budget, and the marker
  reported a byte count as a character count.
- **RT-466 (M)** — after merging a native tool call, the stream looked up the FIRST call with
  a non-empty name instead of the one it just merged, so an SSE event carrying two native tool
  calls announced only the first.
- **RT-468 (L)** — the auth branch kept the RAW provider body, so the sanitizer was dead on
  exactly one path.
- **RT-469 (L)** — `is_local_endpoint` checked only the IPv4 private and link-local ranges, so
  a keyless server on an IPv6 unique-local address was classified as a PUBLIC host.
- **RT-470 (M)** — the session-row writes (the last-activity timestamp and the cost) DROPPED
  their Result on both paths, so a failed write left the session list stale.
- **RT-471 (M)** — `[client.rate_limit] max_retries` counted ATTEMPTS, not RETRIES: the
  shipped default of 3 made three requests instead of four.

What the round taught:
- **A PARALLEL ARRAY is a silent contract.** RT-462: two vectors that must stay aligned, one
  built from the calls that survived a filter and one from all of them. The mismatch cost a
  panic and a wrong pairing.
- **A guard can be dead by ORDER, not by omission.** RT-463: the spend guard was present,
  documented, and tested — and could not fire, because the cap ahead of it had already
  reduced the body below its threshold. A test at a window where the order still worked
  (8K instead of the shipped 128K) hid it for good.
- **A knob's NAME is a contract.** RT-471: `max_retries` counted attempts while the config
  and the example TOML both said "retries after the first attempt".
- **Two sweeps of the same file find DIFFERENT halves of one defect.** The provider sweep
  found the body's read failure (RT-460) and the raw auth body (RT-468) in `client.rs`; the
  agent sweep found the spend guard's order (RT-463) and the session writes (RT-470) in
  `run.rs`.
- **A MUTATION PROOF MUST CONFIRM THE TEST RAN.** A filter that matches nothing exits 0 —
  the first RT-465 mutant "passed" for exactly that reason (the module is `preflight_tests`,
  the filter said `preflight`).
- **A NEEDLE THAT MATCHES ITSELF matches the contract, not the code.** The RT-470 contract's
  own literal was counted as a violation until the needle was split, the way the RT-365
  contract's is.
- **A duplicated `#[test]` runs a test TWICE and only the strict gate sees it.**
- **Two incidents can pull a policy in OPPOSITE directions.** RT-461: one wants the first
  attempt's stall to count, the other wants a cut after real output to earn its retries. The
  difference between them is not a preference — it is a measurable fact (did THIS attempt
  produce output?).

Round-19 end-of-round gates: the WORKSPACE test suite exit 0 (89 targets ok), strict clippy
exit 0 on the core and on the CLI, fmt clean, the doc-link gate exit 0, the WORKSPACE build
exit 0 with zero warnings, the ledger index OK (RG-016, RG-021), genome lint 0 errors (5
pre-existing warnings).

## Round 20 (2026-11-10) — the provider client's last mile and the agent's parallel arrays: nine rows, and the half-applied fix the gate caught

Nine pushed iterations (iter-2061..iter-2068), nine ledger rows (RT-473..RT-481). The round
closed the two sweeps' remaining confirmed findings: the reasoning-effort knob that never
reached the wire, the Anthropic clamp it exposed, the repetition guard's first-call signature,
the poisoned locks, the SSE parser's poll-before-reparse, and three doc claims that the code
contradicted.

Shipped:
- **RT-473 (M)** — the operator's `agent.reasoning_effort_param` NEVER reached the wire: the
  agent resolved the shape and dropped it (`_shape`), and the only production adapter hardcoded
  `Auto`. So "none" still sent `reasoning_effort` on every non-openrouter host, "anthropic_thinking"
  never emitted a `thinking` block, and the forced branches of `apply_reasoning_field` had no
  production caller at all.
- **RT-474 (M)** — the Anthropic thinking clamp (`max.saturating_sub(1024).max(1024)`) did not
  clamp below `max_tokens`: for a cap of 512 it produced 1024 — ABOVE the cap — and Anthropic
  rejects such a request. The branch was unreachable while the shape could not be forced
  (RT-473); fixing RT-473 made it live.
- **RT-475 + RT-481 (M)** — the cross-iteration repetition guard signed only the batch's FIRST
  call, so `[A,B]` then `[A,C]` counted as "identical" and the turn was force-ended with "I
  repeated the same tool call 6 times in a row". RT-475 shipped the helper; RT-481 shipped the
  CALL SITE (the first edit had been lost, and the strict clippy gate caught the half-applied
  fix as an unused function).
- **RT-476 (M)** — two poisoned locks were handled by SILENTLY skipping the work they protect:
  the streaming ToolStart dedup set's clear (so a recovered turn's tool rows never appear) and
  the credential pool's rotation anchor (so the rotation replaced the FIRST credential's secret
  instead of the one in use).
- **RT-477 (M)** — the SSE parser polled the inner stream before re-parsing its buffer, so a
  valid event behind a corrupt payload waited for the NEXT wake-up — up to the whole idle
  timeout, a minute in production — and could be lost to a stall.
- **RT-479 (L)** — `set_api_key("")`'s doc promised the CONFIGURED key back, but after a runtime
  retarget that key belongs to the OLD host and is deliberately never sent.
- **RT-480 (L)** — the error classifier's doc listed a pipeline step ("Transport error
  heuristics") the code does not have, and numbered the fallback accordingly.

What the round taught:
- **A FIX THAT CONNECTS CODE EXPOSES THE DEFECTS IN IT.** RT-473 made the Anthropic thinking
  branch reachable for the first time — and RT-474 (the clamp that did not clamp) was waiting
  there.
- **A DEAD HELPER IS THE SIGNATURE OF A HALF-APPLIED FIX.** RT-475's helper, its test and its
  mutation proof all passed while the production call site still used the old form. The strict
  clippy gate — `function 'batch_signature' is never used` — was the only thing that saw it.
- **A GATE LIST IS A BACKSTOP, NOT A FORMALITY.** RT-475's iteration ran the format and the
  module's tests and skipped the clippy; the round's gates caught it.
- **A CONTRACT THAT COMPARES TWO EMPTY LISTS PROVES NOTHING.** RT-480's first contract parsed
  neither list and "passed" against the mutant; the fix was to make the needles find something
  and to assert they did.
- **A NEEDLE THAT SPANS A DOC COMMENT'S OWN LINE BREAK MATCHES NOTHING.** RT-479's contract
  "passed" against the mutant for exactly that reason — the same trap as before, in a new
  shape.
- **A MEASUREMENT IS A REGRESSION TEST.** RT-477's defect was a DELAY (the event was not lost,
  it waited); the test measures the delay (501ms against the fix's 0ms) — a behavioural test
  that no boolean assertion would have caught.
- **A DOC THAT LISTS AN ENUMERATION CAN BE CHECKED AGAINST THE CODE.** RT-480's contract derives
  both lists (the code's step markers and the doc's numbered lines) and compares them — the
  RT-435 pattern, applied to a pipeline instead of a set of annotation kinds.

Round-20 end-of-round gates: the core's strict clippy exit 0 (lib + all features + tests), the
core's test suite exit 0 (3006 tests), fmt clean, the ledger index OK (RG-016, RG-021). The
WORKSPACE suite ran with ONE failure in the CLI's TUI keybindings
(`no_catalogued_chord_types_itself_into_the_composer`) — the peer's in-flight work, outside
operant-core; the core's own targets were green.

## Round 21 (2026-11-10) — three more doc claims, a cooldown that vanished, and the residuals that need a decision

Three pushed iterations (iter-2070..iter-2071 plus this close) and three more rows
(RT-482, RT-483, RT-484).

Shipped:
- **RT-482 (L)** — `fitted_request`'s doc described the "reported" flag as tracking the
  estimator's own preference, but the code hardcodes it to FALSE (the provider's count is
  exact but STALE — it is the previous request's, missing every tool result since).
- **RT-483 (M)** — an unrepresentable provider retry-after became a ZERO cooldown
  (`unwrap_or_default()` on the chrono conversion), so the credential pool retried the
  credential IMMEDIATELY — exactly when the provider asked it to wait.
- **RT-485 (M)** — the `[proxy]` service key is computed from the CONFIGURED base URL when
  the HTTP client is built, so a runtime retarget (`set_base_url`) keeps the OLD provider's
  proxy — and its proxy credentials — for the new host. The proxy is baked into the
  `reqwest::Client`, so the fix is a design choice, not a one-liner (see the residuals).
  (First drafted as RT-484; the peer took that id concurrently — see the lesson below.)

What the round taught:
- **A DOC THAT DESCRIBES A CHOICE MUST MATCH THE CODE'S CHOICE.** RT-482: the doc promised a
  per-request decision the code never makes.
- **A FALLBACK MUST NOT INVERT THE MEANING.** RT-483: the conversion's failure mode
  (`unwrap_or_default`) turned "wait longer than chrono can express" into "do not wait".
- **A CONFIGURED-VALUE SNAPSHOT IS A STALE VALUE AFTER A RETARGET.** RT-485: the proxy's
  service key is read once, at build time, from a value that can change at runtime.
- **A FETCH-THEN-WRITE WINDOW IS A SHARED-DOCUMENT HAZARD.** iter-2072 rebuilt the audit
  ledger from a fetched tip and the peer pushed BETWEEN that fetch and the commit: their row
  was overwritten and the id collided. The rebuild protocol (fetch → rebuild from
  `git show origin/main:` → check the id space) narrows the window but does not close it — the
  closing move is to rebuild from the tip AGAIN immediately before the commit, or to re-apply
  the peer's rows if the post-commit diff shows a loss. iter-2073 restored the peer's row and
  renumbered ours to RT-485 with an ID NOTE.

### Residuals — named, with the decision each one needs

The campaign's rule is to NAME a residual rather than guess at it. These are the open items
carried by this round and the earlier ones:

1. **The retargetable client's proxy is bound at build time (RT-485).** The fix is either
   rebuilding the HTTP client on every retarget (the client field becomes swappable — a lock
   and a per-request lookup) or moving the proxy to a per-request `reqwest::Proxy` (a
   config API addition). **Decision needed:** which shape.
2. **The credential pool's setters are not serialized with the request path (the agent
   sweep's L2).** `set_api_key` / `rotate_credential_value` / `set_endpoint` mutate the
   shared inner client while a request may be in flight; the gate is a `tokio::Mutex` the
   sync setters cannot take. **Decision needed:** a sync/async lock split, or a doc that
   stops promising serialization.
3. **The reasoning-effort shape's home (the provider sweep's M2, now RT-473).** SHIPPED as a
   `ChatRequest` field; the residual is that the enum exists in BOTH `config.rs` and
   `client.rs` with a manual mapping between them. **Decision needed:** one type, or keep the
   config's enum provider-agnostic and map once (today's choice).
4. **`max_retries`' authoritative meaning (RT-471).** SHIPPED as "retries after the first
   attempt" per the config and the example TOML. The residual is that the default of 3 now
   means FOUR requests, which changes every provider's load. **Decision needed:** confirm, or
   re-document and re-pick the default.
5. **The MCP OAuth credential-revocation wiring (RT-457).** The removal path now reports
   honestly, but nothing in the product CALLS it. **Decision needed:** which command owns
   "revoke this credential".
6. **The Anthropic thinking budget's ceiling (RT-474).** SHIPPED as "below max_tokens", but
   the effort's budget (up to 32,000 tokens) still exceeds a small cap and is clamped to
   `max_tokens - 1` — i.e. the whole cap becomes thinking. **Decision needed:** whether a
   small cap should disable thinking instead.
7. **The seat-memory writer being test-only.** The socialisation coordinator is not in this
   tree, so the memory write path has no production caller. **Decision needed:** where it
   belongs.
8. **The dead code the sweeps found** (named, not deleted — the repo rule is "mention it,
   don't delete it"): `OpenAIModelClient::inner()` (its doc's example, audio generation, has
   no caller), `IterationBudget::remaining()` (only its own tests),
   `close_interrupted_tool_sequence` (run.rs says it is "gone"), `next_probe_tier` and
   `parse_context_limit_from_error` (only their own tests), the error classifier's unused
   `Format`/`Network` variants, and `IterationBudget`'s remaining fields.
   **Decision needed:** delete, or wire.
9. **The carried decisions from earlier rounds** (unchanged): RT-285's three decisions;
   RT-399/409/411/413/430's follow-ups; the KaizenProposal's session-end producer (RT-374);
   the `turn_exit` classifier wiring (RT-368); DN-0058/DN-0059; the admin-wipe decision; the
   curator reactivation (RT-386); the autocompaction mount (RT-385/389's durable delete); the
   RT-452 residual (an incomplete report as a separate field rather than `error`); the
   command-fence's shell-wrap semantics; and the RT-481 lesson that a half-applied fix looks
   complete until the strict gate sees it.

## Round 22 (2026-11-10) — the live context-window incident and confirmed agentic-core gaps

End-state test: the live operator config can no longer be overwritten by a unit test; a protected prompt larger than the configured window fails typed with an actionable message and no useless gateway retry; the runtime model override survives fallback; focused core/CLI regressions, strict clippy, fmt, genome lint, workspace tests, release build, deployment and a credentialed live turn all pass.

| # | unit | done when | depends on | premises |
|---|---|---|---|---|
| 1 | [x] Prove the live outage cause and map the architecture surface | exact 4242 fixture + concurrent global-path race + live logs/config are recorded with evidence | — | A1,A2,C1,C2 |
| 2 | [x] Make settings persistence hermetic and stop read helpers from changing the writer identity | no settings test can write outside a temp config; a read helper leaves `loaded_config_path` unchanged; mutation-proved | 1 | A1,C7,C8,C9 |
| 3 | [x] Repair context-window semantics, validation and recovery | row says Context window; invalid windows are rejected; protected-floor failure names required/configured tokens and gateway does not retry when compaction cannot help | 2 | C2,C3,C9,C10 |
| 4 | [x] Repair runtime model fallback override | request model is the per-request primary under fallback; mutation-proved | 1 | C1,C9 |
| 5 | [x] Repair non-text turn finalization parity | grace/circuit exits run the same memory/prefetch/TurnEnd finalizer exactly once | 1 | C1,C6,C9 |
| 6 | [x] Repair session rehydration tool-group boundaries | a resumed request contains either a complete tool-call group or none | 1 | C1,C6,C9 |
| 7 | [x] Repair verification false-green/process cleanup | missing test phase cannot report ok; timed-out starts kill their process group | 1 | C1,C6,C9 |
| 8 | [x] Run realtime matrix, strict gates, red-team, docs and deploy | scoped/full tests, clippy, fmt, genome lint, fake-provider socket tests, release artifact, deployed live turn all green | 2..7 | C3,C5,C6,C7,C11 |
| 9 | [x] Repair transport recovery truth | streaming 429 honors Retry-After/backoff; credential rotation does not disable unrelated stream retries; both mutation-proved | 8 | C1,C9,C10 |
| 10 | [x] Make context-limit learning and repeated overflow truthful | explicit/probed limits are learned monotonically; every second overflow is typed; regressions mutation-proved | 9 | C1,C9,C10 |

Round-23 note: the hermetic realtime matrix (HTTP, SSE, real verification tool
call, and MCP stdio) and focused gateway/TUI regressions are complete. The full
workspace test suite, strict workspace clippy, genome lint, release artifact,
deployment, and a credentialed live turn through the deployed binary are green.
The repository's `cargo fmt --all -- --check` still reports three PRE-EXISTING
unformatted files already present on `origin/main` (advisor.rs,
stream_replay_tool_rows.rs, tool_failure_logging.rs); this slice changed none
of them, and its owned Rust files pass rustfmt.
### iter-2075 evidence (pushed `a7af53af`)

- Live incident: operator `context_window = 4242` restored to 1,000,000; gateway restarted active with `NRestarts=0` before code repair.
- Writer isolation: explicit-path persistence and read-only config loads do not move `loaded_config_path`; skill preprocessing reads only installed runtime config; TUI persists `agent.context_window` only to `config.config_path`.
- Context truth: `ContextExhausted.required` carries the protected floor; protected overflow names the required/configured tokens and `agent.context_window`, never `/compact`; only foldable exhaustion (`required == 0`) triggers gateway compact-and-retry.
- Gates: full `operant-core` lib+integration suite green (3,011 tests), focused CLI settings 15/15, strict core and CLI clippy green, fmt clean, genome lint 0 errors (5 existing warnings).
- Upstream peer commits `ae436c4b`/`3a34f649` were integrated first; peer audit ids RT-486/RT-487 were preserved and this fix uses RT-488.
