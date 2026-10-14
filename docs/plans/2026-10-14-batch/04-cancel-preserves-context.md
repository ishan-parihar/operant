# 04 — Esc / `/cancel` stop the stream and leave the agent context consistent

## Requirement (user, 2026-10-14)

"`/cancel`/Esc must only stop the current streaming without modifying the active
agentic-context."

After a cancel, the agent's stored conversation must be exactly what happened up to the
moment of the cancel: the user prompt, every completed assistant message and tool result,
and the partial answer that was visible on screen. Nothing may be deleted. Nothing may be
added except the minimum the provider APIs require (a result for each tool call that never
finished). The transcript on screen keeps every row, from every turn.

## Evidence: three defects today

1. **The task is hard-aborted.** `App::cancel_streaming_turn` (`tui/app/prompt.rs:17-33`),
   which Esc (`key_handling.rs:1172`) and `/cancel` (`commands.rs:798`) both reach, calls
   `agent_task_handle.abort()`. The core stores the conversation incrementally:
   - the user message at turn start (`agent/turn_context.rs:150-160`);
   - the assistant message with its `tool_calls` (`agent/run.rs:1333`);
   - one `Message::tool` per result, as each finishes (`run.rs:2075`).

   An abort between those writes leaves an assistant `tool_calls` message with missing
   results in the store. The next request is then either rejected by the provider or
   silently "repaired" by `message_safety` (dropping calls), which is exactly the context
   mutation the user forbids. The graceful path already exists and is unused by the TUI:
   - `interrupt_flag` (`agent/mod.rs:512`; `builders.rs:491` exposes it);
   - `race_interrupt` / `call_interruptible` cancel the in-flight HTTP stream
     (`agent/events.rs:55-108`);
   - the loop checks it at `run.rs:638, 785, 877, 1008`.
2. **The partial answer never reaches the agent.** The TUI flushes the streamed text into
   its *display* list (`flush_streamed_assistant_message`), but the core never commits a
   partial assistant message. The model's context and the screen diverge: the next turn
   the model doesn't know what it had already said.
3. **Every tool row in the session is cleared.** `tool_use_blocks.clear()`
   (`prompt.rs:32`, and the Ctrl+C streaming branch at `key_handling.rs:1213-1223`) removes
   the rows of all earlier turns, not just the cancelled one.

   Also: **Ctrl+C while streaming doesn't stop the agent at all.** It sets
   `is_streaming = false` and clears rows, but neither aborts nor interrupts, so the agent
   keeps running and spending invisibly.

Also relevant: tools don't observe the interrupt (no cancellation reaches `execute_tools`),
so a long `shell`/`process` call keeps running after a cancel.

## Design

**One cancel path, graceful first.**

1. The TUI triggers the agent's `InterruptFlag`. It never aborts the task.
2. The core, on observing the interrupt at any await point (stream, tool batch, between
   iterations), runs `commit_interrupted_tail` and returns `Error::Interrupted`.
   `commit_interrupted_tail`:
   - commits the partial assistant text accumulated by `process_stream` as
     `Message::assistant(partial)` (verbatim; no "[interrupted]" text). Partial tool-call
     deltas whose arguments never completed are dropped, because they were never
     executed;
   - for every `tool_call` id in the last assistant message that has no `Message::tool`
     result, appends `Message::tool(id, "Cancelled by the user before this call
     finished.")`. This is required by the API pairing rule, and it is truthful;
   - persists those messages to the DB (`database.save_message_full`, same as `run.rs:1368`)
     and LCM-ingests them, so a resume sees the same history.
   - It does **not** append `"Operation interrupted."`
     (`message_safety::close_interrupted_tool_sequence`, `:280`). Role alternation for
     strict providers is handled at request-build time by the existing message-safety
     pass, not by mutating the store.
3. **Tools are interruptible.** The tool batch future races the interrupt flag (reuse
   `race_interrupt`). Dropped tool futures must kill child processes: verify
   `kill_on_drop(true)` on every `tokio::process::Command` in the tool crates, and add it
   where missing. Results that completed before the interrupt are kept; the rest get the
   synthetic "Cancelled…" result above.
4. **Watchdog.** If the agent task hasn't returned 3 s after the interrupt (for example a
   tool ignoring cancellation), the TUI aborts the task as a last resort. It then calls a
   new `agent.repair_after_abort().await`, which runs the same `commit_interrupted_tail`
   logic on the stored conversation (pairing results only; the partial text is supplied by
   the TUI, which still has it).
5. **The TUI keeps all rows.**
   - Replace `tool_use_blocks.clear()` with: every block whose status `is_pending()`
     becomes `ToolStatus::Cancelled` (new variant; it renders `⊘`, dim, with the preview
     "cancelled"; add it to the `✗/✓/○` mapping in
     `operant_model/adapter.rs::display_message_from_tool_block`).
   - Blocks of earlier turns are untouched.
   - The notice stays "Turn stopped." (iter-805).
6. **Ctrl+C while streaming** calls the same `cancel_streaming_turn`.
7. **No interrupt during the boundary write.** The store must never be observed half
   written. `commit_interrupted_tail` runs while holding `conversation.write()` once, for
   all of its appends.

## Steps

1. **Core (operant-core):**
   - `Error::Interrupted` variant (if the existing `interrupt::Interrupted` type fits, map
     it); `run` returns it on every interrupt path instead of
     `Error::Agent("Interrupted by user")` (`run.rs:663-667`), and stops emitting
     `AgentEvent::Error` for it. The TUI already shows "Turn stopped."
   - `commit_interrupted_tail(&self, partial: Option<&str>)` in `agent/events.rs`, next to
     `add_message`.
   - Make `process_stream` return its partial text on interrupt (today the partial is
     discarded with the stream). Store it on the turn context, so the run loop's interrupt
     branch can pass it in.
   - Interrupt-race the tool batch, and add `kill_on_drop` audits.
   - `pub async fn repair_after_abort(&self, partial: Option<String>)`.
2. **TUI (operant-cli):**
   - `App` gets `agent_interrupt: Option<operant_core::interrupt::InterruptFlag>`, set
     wherever `agent_task_handle` is set (`adapter_types/tui_app.rs` submit paths at
     `:745-760` and `submit_user_message` `:791-815`).
   - `cancel_streaming_turn` triggers it, marks pending blocks Cancelled, starts the
     3 s watchdog (store `cancel_deadline: Option<Instant>` and check it in the loop next to
     `drain_run_complete`), and **does not** clear blocks or abort.
   - `drain_run_complete`: treat `Error::Interrupted` as success (no error card). The
     `TryRecvError::Closed` arm (task aborted by the watchdog) calls
     `repair_after_abort` through `run_pending_agent_actions`.
   - Unify the Ctrl+C streaming branch with `cancel_streaming_turn`.
3. **Gateway:** its `/stop` path should use the same `commit_interrupted_tail`. Check
   `gateway_runner.rs` for `abort()` on agent tasks and route those through the interrupt
   flag too.

## Tests (RED first)

- Core, with a mock client that streams "Hello wor", then blocks: trigger the interrupt.
  The conversation ends with `assistant("Hello wor")`, `run` returns `Error::Interrupted`,
  and no `Error` event is emitted.
- Core, with an assistant that issues 3 tool calls, where the second tool blocks: interrupt
  during the batch. The store has 3 tool results (one real, two "Cancelled…"), the next
  request passes `message_safety` validation **without repairs**
  (`message_safety::repairs_needed(...) == 0`), and the blocked tool's child process is
  gone (spawn `sleep 30` in a test tool and check the pid is gone).
- Core: interrupt between iterations. The store is unchanged apart from completed
  messages, and no "Operation interrupted." is appended.
- TUI: two turns, the second cancelled mid-tool. The first turn's tool rows are still in
  `display_messages()`, and the second turn's pending row is `Cancelled`.
- TUI: Ctrl+C while streaming triggers the interrupt flag (assert on the flag).
- TUI: the watchdog. With a mock agent that ignores the interrupt, after the deadline
  `repair_after_abort` is called and the agent handle is gone.

## Acceptance

Live: start a turn that runs `sleep 20` through the shell tool, press Esc. Within about
1 s the spinner stops, the tool row shows `⊘ cancelled`, and `ps` shows no `sleep`. Then
ask "what were you doing?": the model refers to the cancelled step, which proves the
context kept it. Repeat with Esc during a streamed answer: the next turn's request
(debug log) contains the partial assistant text.
