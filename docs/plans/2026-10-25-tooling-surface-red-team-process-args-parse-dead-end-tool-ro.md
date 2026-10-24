# tooling-surface red-team: process args-parse dead end, tool-row wall, run debuggability

Dated-frozen: published `2026-10-25`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Context

The user ran an in-TUI "test all tooling surface" agentic turn and pasted the TUI's
rendering of it: most tools passed, but the `process` tool failed ~12+ times in a row
(the turn ended on the tool-fail-streak guardrail), the end of the stream showed a wall
of redundant `✗ process · 33 tok` rows, and the user asked to audit the tooling
infrastructure, find what happened, and check the run's logs. Ask: investigation + a
plan to fix (this document). The fixes below are separate iterations.

## Evidence (all measured on this machine, 2026-10-25)

- Session `sess_62831bf4-edb6-42a6-b9a4-bed5e420029a` (~/.operant/database.db):
  assistant row 57788 carries ONE tool_calls array with **64 parallel `process`
  calls**; python json.loads fails on all 64 with "Invalid control character" — the
  arguments are sequential slices of the model's own final answer text (raw newlines
  inside a JSON string). 32 of them were executed (32 `role='tool'` results carrying
  "Arguments could not be parsed"); the guardrail ladder warned at 8–11 and halted at
  12. Row 57717 earlier in the session holds 13 parallel calls (legit batch, parsed fine).
- gateway.log (the user's `[logging] log_file` target — the TUI run IS logged):
  `2026-10-24T22:16:31.999Z WARN Post-result guardrail ladder … Failures { tool:
  "process", count: 8..11 }` then `Guardrail ladder exhausted — halting the turn`.
  "Unrepairable tool arguments … tool: process" appears **130 times across Oct 20,
  Oct 23, Oct 24** — the pathology is recurring and it is always `process` (the one
  tool whose args naturally hold multi-line text).
- The tok numbers on the rows are result sizes, exactly: 33 tok = the bare
  parse-failure message, 71 tok = parse failure + `[Tool loop warning: ⚠ … failed N
  times]`, 85 tok = parse failure + the hard-stop message. The TUI rendered
  faithfully; the wall is real calls, not a renderer duplication.
- `~/.operant/operant.toml` pins `[memory] provider = "builtin"` →
  `load_memory_manager` treats builtin as the legacy no-op → no provider attaches →
  every memory_* tool answers "No memory backend attached in this session."

## Root causes

- **RC1 — the promised control-char repair was never implemented.**
  `repair_tool_call_arguments` (`crates/operant-core/src/agent/message_safety.rs:60`)
  Pass 0's comment says "try strict=False (accept control chars in strings)" — Python
  `json.loads(strict=False)` semantics — but the code calls plain strict
  `serde_json::from_str`, which REJECTS raw control characters. The pass can never do
  what its comment and its "control chars cleaned" warning claim. Everything the
  model writes with a raw newline inside a JSON string arg (multi-line shell
  commands; answer-dumps) is Unrepairable today.
- **RC2 — the parse-failure result carries zero diagnostic.**
  `stream.rs:862-875`: the synthetic ToolResult says "Arguments could not be parsed
  and were NOT executed. Reply with corrected arguments…" with no reason; the warn!
  logs `args_len` only (the S4 privacy rule — correct for args, but the serde error
  is a parser diagnostic, not model output). The model cannot self-correct from
  nothing; the storm's 12 identical retries prove it.
- **RC3 — the fail-streak gate only observes results, so a parallel batch drains.**
  The guardrail streak is fed post-execution (`observe_guardrail_results`); within one
  64-call batch, pre-flight parse failures `continue` before any streak consult, all
  64 are dispatched, and 32 doomed calls completed before the ladder halted at 12.
  A pre-flight consult would have refused calls 13..64 with synthetic skips.
- **RC4 — the tool-row collapse is dead at the default.**
  `adapter.rs:217-219` runs `collapse_tool_runs` only when `!app.show_tool_calls`;
  `show_tool_calls` defaults TRUE (config.rs:950, operant.example.toml:212), so the
  default user gets every call as its own row — the "redundant wall". Separately,
  a failed row's one-line header shows `✗ {name} · {tok} tok` with no error text
  (the concise error summary machinery `tool_display::concise_tool_error_summary`
  exists but is not applied to the header).
- **RC5 — memory tools dead by user config.** `[memory] provider = "builtin"` pins
  the legacy no-op. Nothing warns that this disables memory_store/recall/search.
- **RC6 — a default TUI run logs to a sink.** `log_file` defaults None and the rich
  TUI routes tracing to a discard sink (main.rs:3549 test name). Debuggable runs
  currently REQUIRE a hand-configured `[logging] log_file` (the user has one; fresh
  installs do not).

## Fix plan (each numbered item is one iteration)

1. **core: implement the control-char escaping pass (RC1).** In
   `repair_tool_call_arguments` Pass 0, before the strict parse: walk the string
   tracking in_string/escape_next state (the scanner pattern already exists at
   :86-102) and escape raw control characters (< 0x20) inside string literals as
   `\n` `\r` `\t` `\uXXXX`. Then parse strictly; on success re-serialize (the
   existing reserialize path). Apply the same arm to the history-repair path
   (`repair_message_sequence`) if it shares the gap, so restored sessions heal.
   Regression tests: the exact storm specimen (answer slice with raw newlines →
   Fixed, parses to the intended string) and the multi-line shell-command specimen
   ({"command": "cd /tmp\nls -la"} raw → Fixed → spawns correctly).
2. **core: diagnostic in the parse-failure result (RC2).** The synthetic ToolResult
   gains the serde error's first line plus a ≤160-char preview of the raw args, and
   an explicit hint when the args look like prose ("your arguments are not JSON; if
   you meant to answer the user, reply in plain text"). The warn! logs the parse
   error string (still not the args). Tests: the error text contains the serde
   reason and the truncated preview.
3. **core: pre-flight fail-streak consult (RC3).** In the phase-1 pre-flight loop
   (`stream.rs:700+`), consult the tool_guardrails fail-streak before parse: once a
   tool's streak is hot (warn+ rung), later same-tool calls in the SAME batch get a
   synthetic "skipped — failing repeatedly this turn" result without executing.
   Test: a synthetic 64-identical-shape-call batch executes ≤ N and the rest are
   skips.
4. **TUI: readable failure wall (RC4).** Fold CONSECUTIVE identical tool rows (same
   name, same ✗/✓ inference, same one-line content) into one row with a ×N marker
   even when show_tool_calls=true — the existing `collapse_tool_runs` stays for the
   hidden mode. And render the concise error summary in a failed row's header
   (`✗ process · invalid arguments: <reason> · 33 tok`). Verify with `operant tui
   debug simulate --assert-screen 'count:✗ process==1'` against a recorded storm
   shape.
5. **config/docs: default TUI log file (RC6) + debuggability runbook.** When
   `[logging] log_file` is unset, default the interactive TUI to
   `<operant_home>/logs/operant.log` (rotation already exists: 32 MB × 3
   generations). Update `operant.example.toml`, and add a RUNBOOK row documenting
   the debugging surfaces: `[logging] log_file`, `RUST_LOG`, `OPERANT_TUI_DEBUG=1`
   + `OPERANT_TUI_EVENT_LOG=<path>` (ring dumps on clean /quit), and
   `operant tui debug simulate --frame-per-event --assert-screen`.
6. **user config + doctor (RC5).** Tell the operator to delete the
   `[memory] provider = "builtin"` pin (memory-wire is the default and the only
   engine); add a doctor row that warns when provider=builtin because memory_* tools
   stay dead.
7. **(triage of the remaining report failures — no code in this plan unless an item
   reproduces.)** TTS: all providers failed on missing deps (kokoro needs
   espeak-ng-data, edge needs the edge-tts package, piper/neutts/kittentts absent) —
   a doctor probe for TTS provider deps is the candidate follow-up. `aft_search`
   "search lanes not ready" is cold-start index warming, not a bug.
   `file_state` "Unknown operation 'read'" is the model guessing an op — the error
   already lists the valid ops. `skill_view` failure was not captured in the
   transcript I could query — reproduce before fixing. browser_cdp /
   accessibility_tree are ALREADY FIXED by the peer's iter-1246.

## Gates / success criteria

- `cargo test -p operant-core --lib -- message_safety` green with the two new
  specimens (raw-newline args repair).
- Storm replay: the 64-call batch fixture executes ≤ the streak threshold and the
  parse-failure result names the reason (items 2+3 verified together).
- TUI: `tui debug simulate` on a recorded storm shape asserts ONE folded row.
- A fresh `HERMES_HOME=/tmp/fresh operant` TUI run writes `<home>/logs/operant.log`.
- Scoped builds only (`-p operant-core --lib` / `-p operant-cli --bin operant`);
  full workspace verify before each push; every item ships its regression test.

## Cost

- Items 1-3: one operant-core iteration each, small diffs in two files
  (message_safety.rs, stream.rs) + tests.
- Item 4: one operant-cli TUI iteration (adapter.rs / ui_messages.rs) + a sim
  scenario.
- Item 5: config default + example.toml + RUNBOOK (docs-weight iteration).
- Item 6: doctor row + operator config note.

## Non-goals / skipped

- The provider-side generation of the 64-call storm (omp gateway → poolside emitting
  the answer as tool calls) — not fixable in this repo; the harness defenses in
  items 1-3 make it survivable and legible.
- Flipping the `show_tool_calls` default — the fold in item 4 removes the wall
  without changing what the surface shows by default.
- Retrying the whole surface test in-TUI to confirm every tool — the report's other
  failures are either environment deps (TTS), already fixed by the peer (browser),
  or unreproduced (skill_view); each has a named next step above.

## Risks

- Item 1 changes what executes: an answer-dump repaired into `{"command": "<prose>"}`
  will RUN through `sh -c` and fail as "command not found" — harmless, and it gives
  the model real feedback, but it is a behavior change from "call not executed".
  `process` spawn rides the same approval policy as before; no new capability.
- Item 3's batch short-circuit could skip a legitimate long batch if the streak is
  hot from earlier identical failures — the threshold (warn rung, ≥8) makes that
  unlikely; the synthetic skip text tells the model why.
- Item 5 turns on disk writes for fresh installs by default (bounded by the existing
  rotation cap).
