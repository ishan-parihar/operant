# Claim Register — agentic-core/TUI repair (round 1)

Format: `<claim> — [V|D|R|G] — <source> — unit N`
[V] verified in this session · [D] derived from a verified fact · [R] reasoned · [G] guess

## Round 6 (2026-11-07) — the operator's five TUI complaints

- The TUI's queued-message band never rendered anything: `TuiState::queued_messages` returned `&[]` behind a `[port-decision]` stub — [V] — read `crates/operant-cli/src/tui/app/tui_state_impl.rs` before the patch; the stub's own comment said "wire when a snapshot of the drained queue is mirrored onto App" — unit 1
- A queued message is now visible in the band, the recall takes it out of the band, and the band follows the agent's drain — [V] — `tui::app::steer_queue_tests` (5 cases, green) — unit 1
- The band's tests are mutation-sensitive: restoring the `&[]` stub fails 3 of the 5 by name — [V] — the mutation run in this session ("left: 0, right: 2" on the queue length) — unit 1
- A queued message and the `/goal` text reach BOTH prompt-history stores (the in-session ladder and `history.jsonl`) — [V] — `a_queued_message_enters_the_prompt_history`, `the_goal_message_enters_the_prompt_history` (green) — unit 2
- The history sandbox needs a writable temp dir: `TMPDIR` here points at a read-only mount, and the history writer ignores write errors — [V] — the first run failed with an empty file; `touch /var/tmp/ishanp-agentic/probe-write` → "Read-only file system", `/tmp` writable — unit 2
- `/goal` was one-shot: `session_goal` had NO reader outside its own setter — [V] — grep over `crates/` found 7 hits, none a read of the value (the setter, the init, two clears, the display, the test) — unit 3
- The goal now rides every turn as `<session_goal>` in the volatile turn-context tail, and leaves the request when cleared — [V] — `agent::tests::the_standing_goal_rides_every_turn_until_cleared` (green) — unit 3
- That test is mutation-sensitive: replacing the injected block with `None` fails it by name ("the goal rides turn one — left: 0, right: 1") — [V] — the mutation run in this session — unit 3
- The advisor's preserve channel joined the notes with `"; "` into ONE string — [V] — read `crates/operant-core/src/agent/builders.rs` before the patch — unit 4
- The card now renders one block per note, and the continuation payload (marker + `<advisory>` XML) is split off at the marker and never painted in the transcript — [V] — `advisor::tests::the_card_renders_one_block_per_note`, `advisor::tests::the_delivery_summary_carries_the_card_then_the_payload`, `tui::app::advisor_card_tests` (2 cases, green) — unit 4
- Those tests are mutation-sensitive: restoring the `"; "` join fails both core cases by name — [V] — the mutation run in this session — unit 4
- The composer's hint was painted as the FIRST ROW of the text rect (the hint Line was pushed into the same `lines` vector as the wrapped draft) — [V] — read `crates/operant-cli/src/tui/operant_ui/ui_input.rs` before the patch — unit 5
- The input area now carries only the operator's text; the hint rides the status row above the composer — [V] — `composer_hint_tests::the_input_area_carries_no_non_input_text` (green) — unit 5
- That test is mutation-sensitive: pushing the hint back into the composer's `lines` fails it by name — [V] — the mutation run in this session — unit 5
- The suites are green with a WRITABLE TMPDIR: `cargo test -p operant-core --lib` → 2819 passed / 0 failed; `cargo test -p operant-cli --bin operant` → 2230 passed / 0 failed / 8 ignored — [V] — the runs in this session — unit 6
- With the sandbox's read-only TMPDIR the same suites report ~187 failures, all `ReadOnlyFilesystem` — [V] — the first full CLI run (`.tmpwork/cli-suite-after.log`); every failing test's message names a read-only filesystem — unit 6
- The clippy gate is green for both touched crates — [V] — `cargo clippy -p operant-core --lib --all-features -D warnings` and `cargo clippy -p operant-cli --bin operant --all-features -D warnings`, no warnings — unit 6
- The four iterations are on `origin/main` — [V] — `git log origin/main` after each push: iter-1839 (re-applied as iter-1841), iter-1842, iter-1845, iter-1846 — unit 6
- The ledger rows were NOT added: `docs/audit/2026-06-19-BUGS.md` is in a concurrent editor's in-flight change, and its index table and checkbox list must move together — [V] — `git status` shows it dirty with 12 hunks around the index table (lines 76-82) — unit 6

- The soft cap (350k / 80%) and the hard cap (400k / 95%) resolve to the SMALLER of the absolute and percentage axes for the active window — [V] — `crates/operant-core/src/context_management.rs` `ContextCaps::resolve` + the unit tests `context_caps_take_the_stricter_of_absolute_and_percentage` — unit 6
- Crossing the soft cap compacts the history at the turn boundary — [V] — `crates/operant-core/tests/context_caps.rs::soft_cap_compacts_the_history_at_the_turn_boundary` (1 compaction event) — unit 6
- A history UNDER the soft cap is not compacted — [V] — `crates/operant-core/tests/context_caps.rs::history_under_the_soft_cap_is_not_compacted` (0 compaction events) — unit 6
- A turn that grows past the hard cap mid-turn compacts the request list before the next provider call — [V] — `crates/operant-core/tests/context_caps.rs::hard_cap_compacts_the_request_mid_turn` (the second request is under the cap) — unit 6
- The two cap tests are mutation-sensitive — [V] — the mutations were applied and reverted in this session: disabling `enforce_hard_cap` fails the hard-cap case (5593 >= 4774); disabling the soft-cap bust trigger fails the soft-cap case (0 vs 1) — unit 6
- The hard-cap compaction rewrites the REQUEST list, not the stored transcript — [D] — `compact_to_hard_cap` mutates the local `Vec<Message>`; the session store is untouched — unit 6
- A resumed session keeps its tool rows, tool_call_ids and assistant reasoning — [V] — `row_to_message` maps tool/system rows and carries `tool_call_id`/`tool_name`/`reasoning`; `crates/operant-core/tests/session_transcript.rs` and the rehydration cases — unit 3
- The transcript carries one row per message — [V] — `crates/operant-core/tests/session_transcript.rs::a_turn_writes_one_user_row_and_one_assistant_row` — unit 3
- The gateway no longer writes duplicate user/assistant rows — [V] — `crates/operant-cli/src/gateway_runner.rs` no longer calls `save_message` for the turn's user/assistant rows; the core owns them — unit 3
- A truncated answer keeps its first half — [V] — `crates/operant-core/tests/loop_recovery_paths.rs::truncated_stop_continues_with_prompt_and_refunds_the_iteration` (both halves in the result; the partial in the continuation request) — unit 4
- The wrap-up summary is continued when the provider cuts it off — [V] — `crates/operant-core/tests/grace_output_gate.rs::truncated_grace_summary_is_continued_and_joined` — unit 5
- The wrap-up summary is in the session transcript — [V] — `crates/operant-core/tests/grace_output_gate.rs::the_grace_summary_is_persisted_in_the_session` — unit 5
- A failed wrap-up reports through the turn diagnostics and the error event — [V] — `attempt_grace_call`'s Err arm now emits both; the streaming path no longer escapes through `?` — unit 5
- The context overlay shows the provider-measured prompt occupancy — [V] — `crates/operant-cli/src/tui/app/session_counter_tests.rs::the_context_overlay_shows_the_prompt_occupancy_not_the_cumulative_spend` — unit 8
- The session totals are cumulative input/output — [V] — `…::the_session_totals_are_cumulative_input_and_output` — unit 8
- A session boundary resets the per-session counters — [V] — `…::a_session_boundary_resets_the_counters` — unit 8
- A session boundary waits for the agent task — [V] — `…::a_session_boundary_waits_for_the_agent_task` — unit 8
- The configured `[agent] context_window` is the denominator the TUI reports — [V] — `App::refresh_context_window_size` reads the config; `crates/operant-cli/src/tui/app/providers.rs` — unit 2
- The session rehydration reports what it dropped — [V] — `SessionStore::load_from_db` logs the kept/dropped/unconvertible counts — unit 7
- The clippy delta of this change is ZERO — [V] — the gate's four findings are on lines this diff does not touch (three files/lines outside the change) and no `.expect()`/`.unwrap()` was added outside the exempt test targets — unit 10
- The five `operant-core --lib` failures that are not this change are environmental or peer WIP — [V] — the failures were read one by one: the repo IS a git repo (three cases), the sourcehound binary is absent (one), `crates/operant-core/src/mcp.rs` is the peer's uncommitted file (one) — unit 10
- The restored-session display still drops reasoning and tool activity — [V] — the two lossy rebuild paths in `crates/operant-cli/src/tui/adapter_types/tui_app.rs` were read and neither converts tool/reasoning rows — unit 8 (OPEN)
- The inline picker still does not publish its keys to the debug bus — [V] — the inline picker consumes the key before the debug publication in `crates/operant-cli/src/tui/app/key_handling.rs` — unit 7 (OPEN)

## Round 2 additions

- A restored session keeps its reasoning and its tool rows — [V] — `session_display_tests.rs` (4 cases) — unit 8
- The converter is ONE for both rebuild paths — [V] — both call sites now call `session_display::apply_conversation_to_display` (app/mod.rs, adapter_types/tui_app.rs) — unit 8
- The `[DONE]` marker is now surfaced by the SSE parser — [V] — `ChatStreamResponse::done_flag` + the three adapter cases — unit 7
- A stream with no completion signal is continued only when the text looks cut — [V] — `stream_incomplete_continuation.rs` (2 cases) — unit 4
- A replayed stream re-announces its tool calls — [V] — `stream_replay_tool_rows.rs` + the mutation check (2 without the clear, 3 with it) — unit 4
- The inline picker's keys and the live agent errors reach the debug bus — [V] — `debug_log_tests.rs` (2 cases) — unit 7
- The CLI suite's two failures are the peer's in-flight work — [V] — `tui::app::tests::test_alt_v_does_not_toggle_vim_mode` and `tui::settings_screen::tests::every_settings_entry_has_an_applier` are in the peer's modified files — unit 10

## Round 2b additions (iter-1776)

- The clippy gate passes — [V] — `bash scripts/clippy-warning-gate.sh` reports `seen=0 allowlist=0 new=0 stale=0 — pass` — unit 10
- The gate was red at HEAD with an EMPTY allowlist and four findings — [V] — the first gate run of this session reported exactly those four — unit 10
- The `#[expect(clippy::expect_used)]` in `agent/run.rs` sat on `turn_attribution` (no `.expect()`) instead of on `run` (four) — [V] — the attribute's position in the file and the gate's two findings on that file — unit 10
- The dead `KeyAction::ToggleVimMode` variant landed in the concurrent iter-1775 — [V] — `git show HEAD:…/keybindings.rs | grep -c ToggleVimMode` is 0 — unit 10

## Round 2c additions (iter-1777)

- A tool that ran and failed is logged with its name, the call id, the duration and the error — [V] — `crates/operant-core/tests/tool_failure_logging.rs` (captures the tracing output and asserts all four) — unit 7
- The canonical workspace clippy gate is clean — [V] — `cargo clippy --workspace --all-targets --all-features -- -D warnings` exits 0 with no warnings — unit 10

## Round 3 (2026-11-06) — the context calculation, the lying log, the early turn

Format: `<claim> — [V|D|R|G] — <source> — unit N`

- The live session `sess_56bae289-989f-42f8-b28f-a3942a51ebdc` holds 652 active rows: 953 148 B of
  content, 339 933 B of tool-call JSON, 428 693 B of reasoning — [V] — `sqlite3 ~/.operant/database.db`
  `select role,length(content),length(tool_calls),length(reasoning) …` — unit 1
- The chars/4 heuristic over that transcript is 432 740 tokens — [D] — recomputed from the DB bytes
  with the shipped estimator's formula (4 + content/4 + tool_calls/4 + reasoning/4 per message) — unit 1
- The agent's own ladder saw 418 711 tokens for it, over the 350 000 soft cap, on a 1 000 000
  window — [V] — `~/.operant/logs/gateway.log` 20:41:31 "Preflight ladder: over threshold — running
  ordered rungs, estimated: 418711, threshold: 350000, budget: 1000000" — unit 1
- The ladder fires on EVERY turn of that session — [V] — the same log line at 19:54:12, 19:58:36,
  20:00:42, 20:41:31, 23:33:53, 23:41:39, 23:44:12 — unit 1
- The compaction rewrites the request only, so the next turn rebuilds the same over-cap request and
  the ladder fires again — [D] — `build_messages` mutates a local `Vec<Message>`; `self.conversation`
  is never rewritten by the ladder — unit 1
- NO adapter sends the assistant reasoning back to the provider — [V] — `client.rs::Message::to_value`
  inserts role/content/tool_calls/name/tool_call_id/extra_content and never `reasoning`;
  `anthropic.rs::build_assistant_content` emits only `text` and `tool_use` blocks — unit 1
- The estimator nevertheless charges the reasoning — [V] — `context_management.rs::estimate_message_tokens`
  adds `estimate_tokens(&msg.reasoning)` — unit 1
- 428 693 B of reasoning is therefore ~107 000 phantom tokens on every request of that session —
  [D] — 428 693 / 4 — unit 1
- The provider's own count for that session's request is ~200 000 (the TUI footer) while the
  heuristic says 341 122 after compaction — [V] — the operator's report + the CompactionCompleted
  payload `341122` — unit 1
- The session-rehydration warning fires on EVERY load and reports the kept count as the dropped
  count — [V] — `gateway.log` "rows_total: 643, rows_kept: 643, rows_dropped: 643";
  `session/mod.rs`: `let dropped = total.saturating_sub(start);` where `start = total - kept` — unit 1
- The 20:25 turn ended early because the TURN WALL-CLOCK limit (1200 s) was hit, the wrap-up then
  emitted a malformed tool call, and the S1 quality gate threw the 13-char result away — [V] —
  `gateway.log` 20:22:15 "Turn wall-clock limit reached — attempting grace call, turn_elapsed: 1293";
  20:25:58 "Failed to parse tool_call content, content_len: 555"; "Grace summary rejected by the
  quality gate — nothing persisted, chars: 13"; "Turn ended: reason=grace_call … response_len=0" — unit 1
- The operator-facing notice for that exit says the wrap-up "ran out of room mid-summary", which is
  not what happened — [D] — the notice arm `tui/app/agent_events.rs:389-393` is keyed on
  `TurnExitReason::GraceCall` alone; the grace call produced a DEGENERATE output, not a truncated one — unit 1
- The turn was NOT out of iterations (api_calls 14/90, budget 15/90) — [V] — the same
  "Turn ended: reason=grace_call" line — unit 1
- jcode's poke is a bounded synthetic continuation guarded by a non-retryable-error classifier —
  [V] — `external-repos/jcode/crates/jcode-todo-policy/src/lib.rs:653` `build_auto_poke_message`,
  `crates/jcode-tui/src/tui/app/commands_auto_poke_errors.rs` `is_non_retryable_auto_poke_error` — unit 1
- operant's core has no synthetic-continuation producer today — [V] — grep for `auto_poke|synthetic
  continuation` over `crates/operant-core/src/agent/` returns only the advisor's own continuation
  channel (`advisor.rs::CONTINUATION_TAG`), which the TUI drains as the NEXT turn's query — unit 1

## Round 3, second half — the resumed session, the distillation, the poke

- 407 of the 437 tool rows in the live session `sess_56bae289…` carry NO `tool_call_id` — [V] —
  `select role, tool_call_id, tool_name … from messages where session_id=… and active=1` on
  `~/.operant/database.db` (the 30 rows that DO carry one were written after iter-1769 landed) — unit 5
- An id-less tool row is DROPPED by the rehydration's orphan repair, and its assistant turn loses
  its calls too — [V] — `agent/message_safety.rs::drop_orphaned_tool_messages` ("a tool message with
  no id at all can never be matched, so it is dropped too") + direction 2 (the request whose result
  was evicted) — unit 5
- That is the "818 dropped halves" in the live log — [D] — 407 dropped tool rows + the calls
  stripped from their assistant turns + the assistant rows left empty ≈ the logged 818 — unit 5
- The transcript's ORDER carries the pairing: an assistant turn's tool results follow it, one per
  call — [V] — MEASURED on the live database: 193/193 assistant turns in `sess_56bae289…` and 627/628
  in `emp-cron_fa1e44c` have exactly as many consecutive tool rows as they have calls — unit 5
- The recovery pairs ~25 000 tool rows across six live sessions with 0 mispairings — [V] — the
  shipped algorithm replayed over the live rows: 8 741 + 7 790 + 5 609 + 1 262 + 875 + 803
  recovered, and 0 rows ended up with an id no assistant turn requested — unit 5
- The provider does NOT charge the assistant's reasoning against the window — [V] — a live probe
  against `https://aroute.ishanparihar.com/v1`: an 18 000-character `reasoning_content` added 15
  prompt tokens to the next request (4 027 total, 3 904 of them cached) — unit 2
- The provider's tokenizer on that corpus is ~4.5 chars/token (18 000 chars → 4 012 tokens) — [V] —
  the same probe — unit 2
- The "Turn ended early — the wrap-up ran out of room mid-summary" notice was a MISDIAGNOSIS — [V] —
  `gateway.log` 20:22:15 "Turn wall-clock limit reached"; 20:25:58 "Failed to parse tool_call
  content, content_len: 555"; "Grace summary rejected by the quality gate — nothing persisted,
  chars: 13" — unit 3
- The turn was NOT out of iterations when it died — [V] — the same log line: "api_calls=14/90
  budget=15/90" — unit 3
- The session distillation failed 27 times on a response that did not start with JSON — [V] —
  `gateway.log.1`: "Invalid distillation JSON: expected value at line 1 column 1" ×27 — unit 6
- jcode's poke is bounded and guarded against non-retryable errors — [V] —
  `external-repos/jcode/crates/jcode-todo-policy/src/lib.rs:653` + `crates/jcode-tui/src/tui/app/
  commands_auto_poke_errors.rs` — unit 3

### Live verification of iter-1787 (the deployed gateway, 2026-11-06 06:00 UTC)

- The running gateway rehydrates the cron session and recovers 705 tool-call ids — [V] —
  `~/.operant/logs/gateway.log`: "session rehydration recovered the tool-call ids of rows written
  without one, session: \"emp-cron_fa1e44c\", tool_ids_recovered: 705" — the same count the
  offline replay of the shipped algorithm predicted — unit 5b
- The tool history is BACK: the turn's dropped halves fell from 1434 to 3 — [V] — the same log:
  "Dropped tool-call halves orphaned by context eviction, repairs: 3" (the previous hourly turns
  all reported 1434) — unit 5b
- A second session: 119 ids recovered, 1 dropped half — [V] — the same log — unit 5b
- The rehydration warning now reports the rows it ACTUALLY dropped — [V] — the same log:
  "rows_total: 3176, rows_kept: 2000, rows_dropped: 1176" (3176 − 2000 = 1176; the old build
  reported 2000) — unit 3
- The sequence repair is non-destructive — [V] — `agent/message_safety.rs::repair_message_sequence`
  INSERTS a synthetic assistant bridge for a tool→user transition and MERGES the others; it never
  drops a tool row — unit 5b
- The deployed `operant sessions list` no longer panics on the operator's own title — [V] — the
  deployed binary printed the table (and rendered the em-dash title "Dispatcher status check —…")
  where the previous build panicked with "end byte index 25 is not a char boundary" — unit 7c
- The clippy gate is green on the shipped tree — [V] — `bash scripts/clippy-warning-gate.sh`:
  "seen=0 allowlist=0 new=0 stale=0 — pass" — unit 7d

## Round 4 (2026-11-06) — the remaining agentic-core gaps

Format: `<claim> — [V|D|R|G] — <source> — unit N`

- The core lib suite is green except for ONE failure, and that failure is in the peer's in-flight
  `mcp.rs` — [V] — `cargo test -p operant-core --lib`: "2802 passed; 1 failed; 1 ignored", the
  failure being `mcp::tests::a_server_that_never_matches_the_request_id_fails_boundedly` — unit 1
- The core integration suite has ONE non-peer failure, and it is the config template's own gate —
  [V] — `cargo test -p operant-core --tests --no-fail-fast`:
  `every_commented_example_in_the_template_names_a_field_the_loader_accepts`, reporting
  `operant.example.toml` lines 568 (`requires`) and 572 (`provides`) — unit 1
- The two flagged template lines are PLUGIN MANIFEST fields, not `operant.toml` keys — [V] —
  the template's own line 566 reads "A plugin's manifest.toml can declare:", and
  `docs/library/plugins.md` lines 22-23/55-64 document `capabilities` / `requires` / `provides` —
  unit 1
- The config loader rejects them because the settings structs use `deny_unknown_fields` — [V] —
  `crates/operant-core/tests/example_template_loads.rs` (module doc, lines 7-9) and the live
  loader error "unknown field `requires`" — unit 1
- `docs/library/plugins.md` already owns the manifest schema and the requires/provides semantics
  (transitive skip, service-name composition) — [V] — the same file, lines 55-64 — unit 1
- The `[skills]` config paths are never expanded — [V] — `config.rs` expands only
  `config.holon` (`expand_holon_path`, line 2605); `SkillsSettings::root_dir` (line 1400) is a bare
  `PathBuf` and no other expansion touches it — unit 3
- A failing AFT command with no `message`/`error` field surfaces the literal "unknown error" while
  the bridge holds the child's stderr in `stderr_tail` — [V] — `aft_bridge.rs::call_inner`
  (lines 849-861) and the `stderr_tail` ring (line 515) — unit 4
- `harness_mount` / `harness_unmount` are implemented and tested, not stubs — [V] —
  `crates/operant-core/src/tools/harness_tools.rs` lines 127-340 (real mount/unmount) and the four
  tests at lines 426-490; only the module header (lines 11-15) still says "stubs" — unit 5
- The remaining open ledger rows are records or operator decisions, not code gaps — [V] —
  RT-136 ("Carried open as a RECORD, not a task"), RT-190 ("The row stays OPEN as a future tranche"
  after an owner ruling), RT-209 (the module header: "the choice is the operator's") — unit 1
- This session cannot reach the systemd user bus, so the deploy copy and the gateway restart are
  impossible here — [V] — `systemctl --user is-system-running` → "Failed to connect to user scope
  bus via local transport: No data available"; the sandbox's writable root is the workspace only —
  unit 6

## Round 4 — SHIPPED (iter-1798, iter-1799, iter-1800)

- The `[skills]` paths had NO expansion, so a copied `$HOME/...` value reached the filesystem
  verbatim — [V] — `config.rs`: `SkillsSettings::root_dir` was a bare `PathBuf`; the only
  expansion (`expand_holon_path`) was applied to `[holon]` only — unit 3- The template's plugin-manifest example was a CONFIG example the loader rejects — [V] —
  `example_template_loads.rs` was RED at HEAD naming lines 568 (`requires`) and 572 (`provides`);
  the loader answers "unknown field `requires`" (deny_unknown_fields) — unit 2
- A failing aft command threw away the diagnostics the bridge had already captured — [V] —
  `aft_bridge.rs`: the error path read only the response's `message`/`error` and fell back to
  "unknown error", while the stderr ring (plan 07.1) held the child's last 64 lines — unit 4
- The self-extension tools could never run — [V] — `harness_tools.rs` gates on
  `ToolContext::metadata["approval"]`, which NOTHING set: the tools are not permission-gated
  (never prompted for) and `tool_context()` carried only `session_id` / `unattended`; the
  operator's `command_allowlist` DID reach the agent's persistent allowlist — unit 5
- A delegation that produced nothing reached the parent as an EMPTY successful tool result — [V]
  — `run_child` returned `message.content` verbatim, and a turn can end with no answer (the
  iter-1783 live incident ended one with `response_len=0`) — unit 5b
- Each fix is pinned by a test that FAILS without it — [V] — six mutation checks were run and
  reverted in this session: the skills expansion (two call sites), the aft error fallback, the
  approval flag, and the empty-child check — units 3, 4, 5, 5b
- The peer's in-flight `mcp.rs` is the only remaining core-suite failure — [V] —
  `cargo test -p operant-core --lib --tests --no-fail-fast`: the single failure is
  `mcp::tests::a_server_that_never_matches_the_request_id_fails_boundedly`, in the peer's
  uncommitted file — unit 6
- The audit ledger's open rows are now four, and none of them is a code gap — [V] —
  `python3 scripts/audit-ledger-index.py` → "OK: the index table and the checkboxes agree"; the
  open checkboxes are RT-136 (a record), RT-179 (a channel-parity feature), RT-190 (an owner
  ruling) and RT-209 (an operator choice between two rendering seats) — unit 6
- The sandbox's cargo cache overlay is what makes a build possible at all late in the session —
  [V] — `~/.cargo/registry/cache` is read-only and empty, so cargo refused to create it
  ("failed to create directory …/registry/cache/index.crates.io-…"); a writable CARGO_HOME
  overlay (`.tmpwork/cargo-home`, symlinked to the read-only registry + a real cache dir) makes
  the gates run again — unit 6

## Round 4 continued — SHIPPED (iter-1805, iter-1808)

- The plugin observe envelope's POST-EXECUTE event carried NO args — [V] —
  `ToolRegistry::execute` built it with `args: Value::Null` while the documented envelope is
  (point, tool, args, result, config); the `replace` verdict (gap G2's redaction seat) therefore
  saw the output but not the input — unit 5c
- The lint gate's 13 expectations were unfulfilled on stable 1.98.0 — [V] —
  `cargo clippy --workspace --all-targets --all-features` reports 13
  `unfulfilled_lint_expectations` and ZERO `double_must_use` / `deprecated` diagnostics; an
  expectation FAILS exactly when the lint it expects does not fire — unit 6
- The same tree with the 13 attributes as `#[allow]` is lint-clean and the gate is GREEN — [V] —
  `bash scripts/clippy-warning-gate.sh` → "seen=0 allowlist=0 new=0 stale=0 — pass" (exit 0) —
  unit 6
- The repo's own dated-frozen plans are ALREADY LANDED — [V] — the harness2-contrast plan's
  iter-A (`HARNESS_LESSON_TAG` + `VERDICT_MARKERS` + the `memory_store` tripwire), iter-B (the
  INV/DOM + verdict-discipline review prompt) and iter-C (`maybe_run_curator` called from
  `agent/prompting.rs:807`, `operant status`'s harness-lesson and curator counters), and the
  plugin plan's G1 (the turn-interceptor seat), G2 (the Replace verdict), G6/G7/G8/G9 (live
  re-registration, creator mode, the in-tree fixture, `plugins status`) — unit 1
- The two core modules with no consumer are ACCOUNTED FOR, not gaps — [V] — `write_approval` is
  the deletion candidate ledger row KB-3/AD-020 already decided against; `persistence_seam` is
  part of the harness2 plan's audited surface — unit 1
- The kanban dispatch lifecycle works through another path — [V] — `operant kanban complete` calls
  `db.complete_task(...)`; the Dispatcher's `complete_run`/`fail_run` are a redundant second API
  with no caller (hygiene, not a gap) — unit 1


## Round 5 (2026-11-06)

- The `[proxy]` table reached no client: `apply_runtime_proxy_to_builder` / `build_runtime_proxy_client`
  / `build_channel_proxy_client` had zero callers outside `operant-config/src/schema/proxy.rs` — [V] —
  grep over `crates/` (unit 2) — unit 3
- The provider clients called `.no_proxy()` and nothing else: `client.rs` (both builders) and
  `agent/clients/anthropic.rs` — [V] — read of the two files before the fix — unit 3
- `scope = "environment"` never reaches a service: `ProxyConfig::should_apply_to_service` returns
  false for that scope — [V] — `operant-config/src/schema/proxy.rs:217-236` — unit 3
- The deferred bridge read the NAME and the tools array read the TYPE: `tool_search.rs:379/548/634`
  and `tool_search/deferred.rs:131` vs `tool_search.rs:131` — [V] — read of the four sites — unit 4
- The sourcehound composite guard read the NAME: `every_composite_is_deferrable` asserted
  `is_deferrable(spec.tool_name)` while the type is what production reads — [V] —
  `sourcehound_composites.rs` — unit 4
- The mutation check: restoring the name filter in the catalog makes the new test fail by name
  ("the type-deferred tool must be in the catalog: []", left 0, right 1) — [V] — the mutation run
  in this session — unit 4
- A plugin observer is registered into the UNNAMED interceptor list at boot and into the NAMED one
  on a live change, and `execute()` concatenates both — [V] — read of tools.rs:517/522/937 and
  plugin_tools.rs:211/515 — unit 5 (FIXED iter-1819)
- `bump_use` / `bump_view` (the only writers of a skill's `last_used` on a use) had no
  production caller — [V] — grep over crates/ plus read of skill_usage.rs:159-171,
  skills_tool.rs:2325 and curator/mod.rs:250/454 — unit 6 (FIXED iter-1821)
- `agent.session_reset` is parsed, offered by the setup wizard and read by nothing — [V] —
  grep over crates/ plus read of config.rs:666 and cmd_setup.rs:1203/216 — unit 7
  (FIXED iter-1822)

- Every round-5 fix is mutation-proved: the provider proxy (the proxy
  listener recorded 0 connections), the deferred bridge (the catalog came back
  empty), the plugin observer (2 registrations instead of 1), the withdrawn
  observer and turn gate (1 instead of 0), the curator use (the skill was
  archived), and the unread session_reset (no warning) — [V] — the mutation
  runs in this session — units 3-7
- The core suite is green at the round-5 tip: cargo test -p operant-core --lib
  --tests --no-fail-fast returned cargo exit 0 — [V] — the chain log in
  .tmpwork/r5-verify-1821.log — unit 8
- The CLI suite is green: cargo test -p operant-cli --bin operant gave 2215
  passed / 0 failed / 8 ignored — [V] — the same chain log — unit 8
- The workspace lint gate passes: bash scripts/clippy-warning-gate.sh printed
  seen=0 allowlist=0 new=0 stale=0 and pass — [V] — the same chain log — unit 8

## Round 6b (2026-11-07) — the agentic-core gap campaign (6 shipped iterations)

- The operator's configured `[skills] root_dir` was honoured by the tools and the agent's skill manager but ignored by seven other callers (the curator's auto-run, the bundle loader, the marketplace install/updates, the status panel, the dump) — [V] — `crates/operant-core/src/skills.rs` (the new `configured_skills_dir`), the seven call sites read before the patch — unit 1
- One rule now resolves the skills tree (`crate::skills::configured_skills_dir()`: the runtime config's root, the platform default when empty) and the CLI callers use the config they hold — [V] — the core's own bundle loader, the CLI's cmd_skills/cmd_dump/cmd_status — unit 1
- The bundle loader read the platform DEFAULT under a comment that claimed "config-aware" — [V] — `agent/skill_bundle.rs` before the patch — unit 1
- The curator's "never archive a skill a cron job calls" rule protected NOTHING: it read `<skills>/.curator/cron.db` while every cron job writes `<db dir>/operant_cron.db`, the automatic path had no cron guard at all, and a failed read was replaced with an EMPTY set — [V] — `curator/mod.rs` (run_review + apply_automatic_transitions) and `cmd_curator.rs` read before the patch — unit 2
- The curator's archive was also not DURABLE: the tracker/state write errors were dropped (`let _ =`), so SKILL.md could sit in `.archive` while usage.json said Active — [V] — `curator/mod.rs:469-471, 521-522` before the patch — unit 2
- `run_review` saved `state.json` BEFORE setting `last_run_summary`, so the file on disk carried the PREVIOUS run's summary and `operant status` (a file reader) disagreed with `operant curator status` (a live reader) — [V] — `curator/mod.rs:316` vs `:320` before the patch — unit 3
- `${OPERANT_SESSION_ID}` was documented as substituted, and the only production call passed `None` — [V] — `agent/skill_preprocessing.rs` (the substitution function is correct; the caller hardcoded None) — unit 4
- Seventeen `[providers.models.*]` knobs and six `[observability]` knobs are read by NOTHING, and each carried a doc comment promising a behaviour — [V] — a per-field grep over crates/ (0 reader hits for all seventeen; only `backend` is read in `[observability]`) — unit 5
- `operant doctor` hard-ERRORed on a provider profile's out-of-range temperature — a knob that cannot change any request — [V] — `doctor.rs:219-232` before the patch — unit 5
- The concurrent slice's `iter-1840` commit was built on a stale tree and REMOVED my already-pushed iter-1839 work; iter-1841 re-applied it (nothing rewritten) — [V] — `git show c819e2f7 --stat` (122 deletions in operant-config/src/schema/core.rs) — unit 6

## Round 7 (2026-11-07) — the third sweep and the dropped transcript writes

- The rewind's FORK transcript write dropped its error (`let _ =`) while the sibling session-row write in the SAME function surfaced its failure (`RewindError::SaveFailed`) — so a failed fork produced a session whose history silently lost messages — [V] — `crates/operant-core/src/agent/rewind.rs` read before the patch — unit 1
- The interrupted-tail commit (the cancelled tool rows appended after an abort) dropped its write error too, and the function cannot return a `Result` (it runs on the abort path) — [V] — `crates/operant-core/src/agent/events.rs` — unit 1
- The rewind now propagates the write error; the tail-commit logs it with the session id and the row's role — [V] — the source-grep contract test `the_rewind_does_not_drop_the_transcript_write_error` (the needle built by concatenation, so the test's own source cannot satisfy it) — unit 1
- Every config field with no reader outside config.rs is now either warned at load or documented as a park — [V] — a re-run of the field sweep: 34 unread fields, all of them covered (the four legacy web keys by `WebToolSettings::warn_legacy_keys`, `route_python_to_kernel` by its deprecated-alias doc) — unit 2
- The third audit sweep (agent/, session/, gateway/, org/, context/, cost/, provider_catalog, cronjobs/scheduler.rs) is still running; its findings are for the next round — [G] — the subagent is still active — unit 3

## Round 8 (2026-11-07) — Ctrl+Enter, the generic compression, the immortal notice

- The notification's WRITER stamped a TTL; the READERS ignored it — [V] — `status_notice()` read `status_message` raw and `has_notification()` asked only whether the field was set; nothing clears the field — unit 1
- A notice that served its 3-second TTL stayed on screen and kept its row — [V] — `tui::app::notice_ttl_tests` (4 cases, green) — unit 1
- Those tests are mutation-sensitive: the raw read fails two of them by name — [V] — the mutation run in this session — unit 1
- The compaction caps defaulted to an ABSOLUTE 350k/400k and the resolver takes the SMALLER axis — [V] — `ContextCaps::resolve` (`(abs, pct) => abs.min(pct)`) — unit 2
- On a 1M-token window the soft cap was therefore 350_000 — 35% of the window — and the ladder fired every turn — [V] — the operator's own session in `~/.operant/logs/gateway.log`: "Preflight ladder: over threshold — running ordered rungs, estimated: 361332, threshold: 350000, budget: 1000000" — unit 2
- The ladder's first rung re-trims every historical tool result to 2_000 chars — LOSSY and content-agnostic — [V] — `fast_trim_tool_results` + `PreflightConfig::default().tool_result_retrim_chars = 2_000` — unit 2
- The percentage axis now governs: a 1M window compacts at 800k, a 128k window is unchanged at 102k — [V] — `context_caps_default_to_the_window_percentage` (green) — unit 2
- The cap tests are mutation-sensitive: the 350k default back fails three by name — [V] — the mutation run in this session — unit 2
- Ctrl+Enter only queued while a turn was streaming; while idle it was a second Submit — [V] — the key arm and the test `ctrl_enter_submits_when_idle` that pinned it — unit 3
- The agent's steer drain runs BETWEEN iterations, so an idle-queued message had no boundary to wait for — [V] — the drain sits at the end of the run loop's iteration body (`run.rs`) — unit 3
- Ctrl+Enter now always queues, and the submit path delivers the pending queue with the next turn — [V] — `ctrl_enter_queues_when_idle`, `an_idle_queued_message_is_taken_for_the_next_turn`, `queued_messages_keep_their_order` (green) — unit 3
- The queue tests are mutation-sensitive: the idle-submit arm back fails two by name — [V] — the mutation run in this session — unit 3
- The CLI suite is green after the three changes — [V] — `cargo test -p operant-cli --bin operant` → 2241 passed / 0 failed; `cargo test -p operant-core --lib` → 2837 passed / 0 failed — unit 4
- The README's `length_in_band` rubric finding is PRE-EXISTING (371 lines vs the monorepo band's 340) — [V] — `score_readme.py --archetype auto` on the pre-change README gives the same count — unit 4

## Round 8b (2026-11-07) — the gateway crash loop (found while deploying)

- The gateway was restarting 870+ times in two hours, dying seconds after each start — [V] — `journalctl --user -u operant-gateway`: "Scheduled restart job, restart counter is at 863", "code=killed, signal=PIPE" — unit 1
- The crash predates this session's deploy (the counter was already 863 before it) — [V] — the journal line at 17:38:17, before the 17:38 deploy — unit 1
- The gateway dies of SIGPIPE when a child closes its pipe — [V] — `operant gateway run` exits 141 (128+SIGPIPE) in the foreground with stdout redirected to a FILE — unit 1
- The broken pipe is the AFT bridge's CHILD, not the journal's stdout — [V] — the gateway writes NDJSON to the child's stdin (`aft_bridge::send_request_once`) and the child prints help and exits (the bridge's own log records the help text being parsed as NDJSON) — unit 1
- The CLI's default SIGPIPE disposition is what turns the write into the daemon's death — [V] — `restore_default_sigpipe_disposition` sets SIG_DFL process-wide before main's dispatch — unit 1
- The gateway arm now ignores SIGPIPE, so the write returns EPIPE — [V] — `ignore_sigpipe_disposition` in the gateway arm, pinned by `tests::the_gateway_ignores_sigpipe` — unit 1

## Round 11 (2026-11-08) — what this round's evidence does and does not support

Supported (each claim is backed by a named, mutation-proved test):

- Every RT-328…RT-364 defect in `docs/audit/2026-06-19-BUGS.md` was
  verified first-hand in this workspace before it was fixed — the subagent
  sweeps' claims were each re-read at the file and line level, and four
  sweep items were dropped as FALSE after that check (`browser_tool`'s
  "dead redeclaration" — `accessibility_tree` is handled; `credential_pool`'s
  "unused constants" — `EXHAUSTED_TTL_SOLE_CREDENTIAL_SECONDS` is read;
  `gemini`'s "resurrection comment" — `extra_content` is merged into the
  request; `ponytail::check`'s "unwired" — `kick_update_check` is called at
  `operant-cli/src/main.rs:1713`).
- Two sweep items were dropped as UNVERIFIABLE in this tree (the
  `endpoints/carrier_pigeon_docs.rs` "brick-the-webhook" boilerplate and the
  `genetics.rs` `stuck_absent` probe — neither path exists in this
  workspace) and are named in the round ledger rather than fixed or
  silently dropped.
- The final suite state is green: `cargo test -p operant-core --lib
  --tests --no-fail-fast` exit 0, `cargo fmt -p operant-core -- --check`
  clean, `scripts/clippy-warning-gate.sh` clean, `scripts/audit-ledger-
  index.py` OK, `./scripts/genome lint` 0 errors (5 pre-existing TOC
  warnings on large docs, none of them this round's).

NOT supported (named, with the reason):

- Append coupon/documentation/process regressions that materialised during
  the round (the shared tree holds peer WIP) are not resolvable here — the
  peer's WIP is never committed, and any outstanding finding outside
  crates/operant-core belongs to its owner.
- The round's earlier claim (carried from prior rounds) that deployment is
  impossible on this host turned out to be a stale lesson, NOT a fact: the
  first `cp` failed with "Text file busy" (the running gateway held the old
  image), and stopping the service, copying, and starting it again deployed
  the new binary cleanly (md5 match, `active`, NRestarts=0). The read-only
  `~/.local/bin` claim from earlier rounds is retracted for this host state —
  [V] — `ls -la` shows the directory writable and the copy succeeded.
- The six named-not-shipped surfaces (RT-331 stall detector, RT-332 mirror
  forwarder, RT-333 admin handler, RT-345 runtime adapter, RT-358 LCM
  tools, RT-359 persistence seam, RT-360 rate-limiter status) each need a
  product decision; the round's change is the honest doc plus a contract
  that pins the doc.

## Round 12 (2026-11-08) — the shared-document hazard, verified

- A stale worktree copy of a shared document is erased peer work — [V] — my iter-1954 commit dropped the peer's RT-366 row and section while their code survived, detected by the id collision's grep and repaired forward in iter-1955 from commit `950bb761` — unit 1
- The temporary-index recipe does not protect a stale worktree copy — [V] — `git read-tree HEAD` + explicit adds stages the WORKTREE version of any file you `git add`; only the files you omit are protected — unit 1
- The strict clippy gate catches a consumed `#[cfg(test)]` — [V] — iter-1956/iter-1958's insertions left two existing test modules un-gated and clippy's `-D warnings` run flagged the dead helpers and unused import in the same run — unit 1
- The evolution-counter and telemetry swallows left named state wrong — [V] — the counters hydrate the review intervals (their own comments say so) and the telemetry is read by the curator; both contracts fail when the swallow is restored — unit 1
- The turn-exit classifier is unwired while a different enum is live — [V] — `classify_turn_error`'s only references are its tests, and `agent::TurnExitReason` (six variants, no kill precedence) is what `turn_finalizer` feeds — unit 1

NOT supported (named, with the reason):

- The sweep claims for memory_management tool names, the two endpoint paths, browser_tool, credential_pool, models_dev, ponytail, trajectory, kanban, and the schema-only tool structs were all verified false first-hand this round and are recorded as dropped rather than fixed.
- The tools/org/config re-sweep was still running at this round's close; its findings are not claimed here.

## Round 12, second half (2026-11-08) — the sweep, verified

- A sweep finding is a hypothesis, and a core-only grep cannot see a cross-crate
  reader — [V] — the "no gateway turn is budget-gated" claim died on
  `operant-cli/gateway_runner.rs:2909-2955`; its true sub-claim shipped as
  RT-376 — unit 1
- A tool's schema can be built TWO ways, and a retraction that fixes one copy
  leaves the other — [V] — RT-352 fixed the struct doc while the hand-written
  `json!` schema string still promised the old default (RT-371) — unit 1
- Two module docs can contradict, each looking authoritative — [V] — the
  context-injection header described three live sources the collector returns
  empty for, while `crate::org`'s own doc recorded the retirement (RT-372) — unit 1
- A doc's history note quoting the old claim satisfies the "must not contain"
  needle — [V] — the worklog RT-373 contract passed on its own mutant until the
  note was reworded to paraphrase — unit 1

## Round 13 (2026-11-09) — the sweep, verified and shipped

- A safety mechanism can document its contract and then run on the wrong side
  of it — [V] — the curator's "pre-archive" backup was created after the
  archival loop had already moved the skills, so it could not roll back the
  archive it was named for (RT-384) — unit 1
- A file can name an invariant at one site and violate it at two others —
  [V] — apply_automatic_transitions propagates the tracker write with `?`
  while run_review and run_consolidation discarded it, under the comment that
  describes the dropped write's consequence (RT-385) — unit 1
- A mutex is not a transaction — [V] — the cron rewrite's "no partial
  rewrites" note survived until an explicit transaction made it true; the
  non-transactional mutant left job-1 already rewritten (RT-388) — unit 1
- The campaign's self-satisfaction trap keeps finding new prose: instances
  six through ten this round — [V] — doc history notes and test comments
  quoting the claims they forbid, each caught by the mutation run coming back
  green and repaired by paraphrase before the proof counted — unit 1
