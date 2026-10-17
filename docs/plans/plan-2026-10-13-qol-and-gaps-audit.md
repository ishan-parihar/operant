# Remaining gaps audit + QoL plan (2026-10-13, night)

Ground: `origin/main` @ `5b6ba621` (iter-802), deployed `1fad1fcd`.
Evidence comes from code reads, headless `tui debug simulate` runs (isolated `HOME`), a
sweep of all 106 registered slash commands, and the user's paste after iter-802.

## P0-A. Agentic core: guardrail output leaks into answers ("leaking tool calls")

The user's paste: the final answer is followed by `⚠ Tool 'process' has failed 8…11 times…`,
then the same for `Tool ''`, then `Stopping: \`\` has failed 12 times…`, then **two** error
cards (`Error: Interrupted by user`, `Error: Agent error: Interrupted by user`).

1. **Warnings are UI text, and the model never sees them.** `process_tool` → `GuardrailDecision::Warn`
   emits `AgentEvent::Content { pattern_warning_message(..) }` (`agent/stream.rs:~520`).
   Content is answer text, so it glues onto the model's sentence. Nothing appends it to `messages`,
   so the model is never warned, which is why it kept calling `process` until the halt.
   **Fix:** append the warning to that tool result's model-visible content, and emit a separate
   UI notice event (e.g. `AgentEvent::GuardrailNotice`), shown as a dim system annotation,
   never as answer text.
2. **The halt is reported as a user interrupt.** `GuardrailDecision::Halt` calls
   `interrupt_flag.trigger()` (`stream.rs:542`), so the loop emits `Error("Interrupted by user")`,
   returns `Err`, and the TUI restores the prompt with "the turn failed". **Fix:** a distinct
   stop reason. The turn ends `Ok`, with the halt summary as one system notice, no error cards
   and no prompt restore.
3. **Unnamed tool calls.** The `Tool ''` ladder means calls with an empty function name reach
   execution. **Fix:** reject them at stream parse with one model-visible error
   ("tool call had no name"), and key the guardrail as `(unnamed)`.
4. **Duplicate error cards.** The core emits `AgentEvent::Error` and also returns `Err`.
   `channel_drains.rs:310` converts the `Err` into a second `Error` event. **Fix:** one surface
   per turn failure (skip the drain re-emit when the turn already surfaced an error).
5. **The Error arm discards the partial answer.** `AgentEvent::Error` clears `streaming_text` and
   `streaming_thinking` without flushing them. **Fix:** flush first, so the partial reply stays
   readable above the error card.
6. **Failed tools render `✓`.** The adapter drops `ToolStatus::Error` (documented as
   "named-unmapped"), and the icon is inferred from output text
   (`tool_output_looks_failed`). **Fix:** carry the status into the tool row
   (the adapter prefixes `✗`/an error marker, or adds a `DisplayMessage` field), so a failure
   always reads `✗`.

**Verify:** core tests for warn reaching the model's message list, halt ending `Ok` with one
notice, and empty-name rejection. TUI tests for: no Content from guardrails, one error card per
failure, the partial kept on Error, and `✗` on `ToolStatus::Error`. Live: a tool-loop turn.

## P0-B. Selection highlight paints the UI

The copy path is fixed (iter-801/802), but the **highlight** still covers whole rows:
gutters, `1›`, padding, tool rows, chrome. Two painters run:
- jcode's line-level `highlight_line_selection` (`operant_ui/ui_viewport.rs:964`, ported) paints
  from `wrapped_copy_offset` to the **text** width only, which is correct. But our
  `copy_selection_range()` (`app/tui_state_impl.rs:999`) hands it **screen** columns, not text
  columns.
- Our post-render pass `render/selection.rs::apply_selection_highlight` then paints every cell
  from the left edge to the right edge on top.

**Fix:**
1. Delete the cell-painting loop. Keep `selection_text` refresh (it already comes from
   `selection_copy_text`).
2. `copy_selection_range()` converts each point with `chat_text_col`.
3. jcode's painter skips the rows copy drops (the same `ui_row` predicate as
   `selection_copy_text`, shared as one function).

Result: only the characters that will be copied are highlighted.

**Copy follow-ups:**
- System and error notice rows count as UI rows. The user's paste included both error cards; add
  `system`/`error` to `UI_ROLES`. They still copy when they are the whole selection.
- Live in-flight tool rows (Streaming section) aren't covered by `message_index_at_line`.
  Classify them from `live_tool_rows`.
- **Table cells that wrap** become extra TSV rows (`\t\t已存储并成功读回…`). Merge a continuation row
  (a row whose first cell is empty) into the previous row's cells.

**Verify:** a drawn-frame test asserting highlighted cells equal the copied text's cells (gutters,
tool rows and padding unhighlighted). Corpus `selection-highlight` re-proved.

## P1. QoL from oh-my-pi (`packages/tui/src/app-keybindings.ts`, `config/settings-ui.ts`)

| oh-my-pi | operant today (simulated) | Plan |
|---|---|---|
| `Ctrl+R` history search | **exists**, but lists oldest-first and preselects the oldest; **two renderers** (jcode inline bar + legacy modal `overlays/history_search.rs`) both paint once a query is typed | newest-first, most-recent preselected, dedup; one renderer (keep the inline bar); assert typed keys never reach other handlers |
| `↑/↓` prompt history | **works** on an empty composer (loads `~/.operant/history.jsonl`) | add a discoverability hint (placeholder or footer) and multi-line cursor rules (first/last line) |
| Settings panel: ~270 schema-driven settings in 9 tabs, env-override notes, conditional visibility | `/settings` is a leftover port of another tool's settings: 13 items, writes `settings.json`, "Max Tokens" edits `context_window`, most toggles do nothing | **rebuild** over `operant.toml` with `toml_edit` (already a dep of `operant-config`, keeps comments). Tabs from config sections (Model, Agent, Display/TUI, Tools, Memory, Gateway); live-apply where the runtime supports it (reasoning mode, theme, show_tool_calls, timeouts), otherwise a "restart to apply" badge; delete the legacy screen |
| User keybinding overrides | `load_keybindings_from_config` is a stub that returns the defaults | `[keybindings]` table in TOML; `/keys` shows effective bindings and conflicts |
| `Ctrl+G` external editor | missing (`Ctrl+G` = scroll bookmark) | `$VISUAL`/`$EDITOR` on the composer text; **key decision needed** |
| `F5`/`Alt+R` retry failed turn | "reply `continue`" only | retry = resubmit the restored prompt |
| `Shift+Tab` thinking-level cycle, `Ctrl+P` model cycle | Shift+Tab unbound in the composer; `Ctrl+P` = global search; `Ctrl+A` = model picker | Shift+Tab cycles reasoning effort; model cycle on `Alt+P`, **key decision needed** |
| `Ctrl+O` expand tool output | no key (mouse badge only) | toggle expand on the latest tool row |
| `Alt+Shift+C` copy prompt / `Ctrl+Shift+V` raw paste | `/copy` exists | add keys if wanted |

## P1. Slash commands: 58 of 106 registered commands are "not wired"

The sweep on `1fad1fcd` printed `/x is registered but not wired` for: `/account /accounts /active
/alignment /auth /autojudge /autoreview /cache /cancel /catchup /changelog /color /colors /commit
/commit-and-push /commit-push-release /compact /cut-release /debug-visual /dictate /dictation
/feedback /git /goals /overnight /poke /rebuild /record /refresh-model-list /remote /reset
/resume-all /screenshot /screenshot-mode /selfdev /split /split-view /splitview /ssh /subagent
/subagent-model /subscribe /subscription /support /swarm /telemetry /test /transcript /transfer
/transport /unsave /update-sim /workspace /wrapped /z /zstatus /zz /zzz`.

They all appear in `/help` and autocomplete. **Plan:** wire the essential ones (`/compact`,
`/cancel`, `/reset`, `/resume-all`, `/transcript`, `/changelog`, `/refresh-model-list`, `/goals`).
Unregister the jcode-only ones (accounts/subscriptions, `/z*`, dictation, screenshot, selfdev,
split-view, telemetry, …) so nothing advertised is dead. **Decision needed** on the middle group
(`/commit*`, `/git`, `/test`, `/swarm`, `/subagent*`, `/ssh`, `/remote`).

## P2. Carry-overs and debts

- **Autonomous mode is dead.** `TODO.md` was deleted in iter-101 (`1f0eda9b`). Every tick logs
  "TODO.md not found; skipping". CLAUDE.md still calls it the ledger. Restore the ledger or
  retire the mode and the doc rule.
- Peer outline (`plan-2026-10-13-tui-remaining-gaps-outline.md`): A1 done (iter-797). Still open:
  A2 `▸ thought for Xs` row, A3 gateway `show_reasoning` → same mode enum, B5 renderer line
  cache (measure first), B6 turn-metadata chrome.
- BUGS.md open: D-4 (duplicate LCM workers), D-6, D-5b, K-1 (kernel tests), K-2, K-3
  (scroll-mode watermark), K-4 (`--no-default-features` build), S6 (detector Block verdict is a
  no-op), S-2 (stale skill copies), **R42-8 (two live Telegram bot tokens unredacted in
  gateway.log, previously declined; still a credential exposure)**.
- Gateway test `test_send_channel_message_no_enabled_platforms_errors` is order-dependent (failed once in a
  full run, passes alone 3/3).

## Order

1. P0-A (core output hygiene: items 1–4 in one iter, 5–6 in a second)
2. P0-B (highlight + copy follow-ups)
3. P1 history search fixes → keybinding overrides → retry/editor/cycle keys (after key decisions)
4. P1 settings panel rebuild (largest item; own iters per tab)
5. P1 command triage (after the decision)
6. P2

Each iter: tests (+ RED check), corpus verify, clippy diff, push to `origin/main`, release build,
deploy both paths, gateway restart, md5 check.
