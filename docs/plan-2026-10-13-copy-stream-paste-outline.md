# Copy fidelity, stream death, paste loss: fix outline (2026-10-13, evening)

Source: the user's report after the iter-795 (local) deploy: "selection still not
like jcode; text breaks at line-breaks and copies UI", plus a transcript where every
tool turn dies with `stream died … after 0 retries over 60s (error decoding response body)`.
Investigated against `origin/main` @ `89e8290a` + local commit `748fbd7a`.

## 0. The fixes were never under test (ship hygiene, blocking)

- `~/.local/bin/operant` was **overwritten at 19:31:05** by a peer deploy from
  `origin/main` (md5 `4c3dae31…`). My 19:16 deploy (`1bacba91…`) was never pushed,
  so origin builds don't contain it. Both of the user's TUI processes (started 19:31 and
  19:47) run the peer binary. `~/.cargo/bin/operant` still has mine.
- Peers also took the `iter-795` and `iter-796` labels. Local `748fbd7a` must be
  relabeled to the next free label before pushing (check `origin/main` after `git fetch`).

**Do:** relabel (amend the commit and the CHANGELOG entries) → push to `origin/main` → release build → deploy
both paths → restart gateway → md5 check. After that, peer deploys carry the fixes.
Every later item here ships the same way, pushed, never local-only.

## A. Stream death at 60s with zero retries (P0, agentic core)

**A1. A total request timeout kills healthy streams.** `[client] timeout_secs = 60`
(user TOML) is applied as reqwest's **total** timeout twice: client-wide in
`OpenAIClient::new` (`client.rs:99`, `Client::builder().timeout(..)`) and per request
in `chat_streaming` (`client.rs:393`, `.timeout(self.effective_timeout(model))`).
In reqwest a total timeout runs until the body finishes. Any turn that streams for more than 60s
(reasoning plus tool calls on `small-stack`) is cut at exactly 60s and surfaces as
`error decoding response body`. This matches the "over 60s" in both of the user's errors.
Turns without tools finish under 60s and succeed, which is what we saw live.
`small-stack` has no `reasoning_timeouts` floor, so nothing raises the limit.

- Fix: a dedicated streaming `reqwest::Client` with `connect_timeout(..)` plus
  `read_timeout(effective_timeout)`. `ClientBuilder::read_timeout` exists in 0.12.28.
  It is per-read and resets after every chunk, so it detects a stall without capping
  stream length. Non-streaming `chat` keeps the total timeout. Note that a per-request
  `.timeout()` can only override the client-wide value, never unset it, so the streaming
  path needs its own client.
- Rename the error: a read-timeout stall reports `stream stalled for Ns` instead of the
  misleading decode error.

**A2. The retry budget is spent by healthy streaming time.** `StreamRetryBudget` (45s)
is created at turn start (`agent/run.rs:480`) and re-armed only after a fully parsed
response (`run.rs:1076`, iter-791). A stream that streams for 60s and is then cut has
already "spent" 60s > 45s, so `decide_stream_retry` returns `GiveUp(WallClock)` with
`attempts: 0`. That is the user's "0 retries".

- Fix: the budget measures time spent **stalled/failing**, not time spent streaming.
  Re-arm it whenever a request is issued and whenever the stream yields a chunk
  (process_stream progress hook), so a drop after productive streaming always earns
  its retries. The wall-clock bound still stops dead-but-accepting providers.

**Verify:** a local mock SSE server test (pattern exists in `client.rs` tests at ~:1765)
that drips chunks for 3× the timeout with gaps under the timeout must complete. A gap
over the timeout must produce a retry (`attempts >= 1`), not `GiveUp(WallClock)` with 0.
Live check: a 5-tool turn on the user's kilo `small-stack` config completes.

## B. Copy = clean document text, like jcode (P0)

Dumped from a drawn frame (wrapped vs raw lines). User prompts, assistant paragraphs and
bullets already have intact logical **raw** lines. The defects are in how we copy
them, plus chrome baked into some raw lines:

| Surface | Raw line today | Problem |
|---|---|---|
| soft-wrapped paragraph / prompt | one logical line ✓ | edge lines copied from **wrapped** rows → breaks at soft wraps |
| error/system card | pre-wrapped fragments (`render_system_message` → `render_plaintext_lines(wrap_width)`) | hard breaks at terminal width (user's paste) |
| tool row | `  ✓ bash $ ls · 2 tok` | UI chrome copied |
| code block | `┌─ rust`, `│ fn main() {`, `└─` | frame and gutter copied; copy offset is 0 |
| table | `Tool          │ Status`, `──┼──` | box drawing, not a document table |
| blockquote | `│ quoted line` | gutter copied |
| header / boot chrome | no line map | copied via the wrapped fallback |

**B1. Port jcode's raw-line copy for the whole selection.** jcode
`ui/copy_selection.rs::copy_selection_text_from_raw_lines_base` + `raw_selection_point`
maps **both endpoints** through `wrapped_line_map` + `wrapped_copy_offset` to
`(raw_line, column)` and slices logical lines. Our port marked it "not ported"
(`operant_ui/mod.rs`, `copy_selection_text`). `PreparedChatFrame` already exposes
frame-global `raw_plain_line` / `wrapped_line_map` / `total_raw_lines`. Replace
`render::selection::selection_copy_text`'s edge/interior split with this path.
The wrapped fallback applies only when an endpoint has no map.

**B2. UI rows never copy.** Skip rows owned by `tool`, `meta`, `usage`, `swarm`,
`background_task` messages (via `message_boundaries`), and unmapped header/chrome rows.
Exception: when the selection lies entirely inside one such row, copy that row, since
the user explicitly selected it.

**B3. Chrome-free raw text for markdown structures.** Add `raw_copy_lines:
Arc<Vec<Option<String>>>` beside `raw_plain_lines` in `PreparedMessages`, filled where
the body renders these rows. When present, it overrides the copied text for that raw line:
- code block → fence lines ` ```rust ` / ` ``` ` and code without the `│ ` gutter;
- table → markdown pipe rows (`| Tool | Status |`, `|---|---|`);
- blockquote → `> quoted line`;
- headings stay plain text (the rendered text is already clean).
An edge that lands inside such a row takes the whole row's copy text. Columns inside
box-drawing chrome have no meaningful text slice.

**B4 — DROPPED during execution.** Copying an assistant message's markdown
source pastes `**bold**` and backticks literally into Word/Docs, which is the
manual editing the user wants gone. Drag copy stays rendered plain text (jcode
semantics). B3 instead emits plain text: code without fences, quotes without
markers, tables as tab-separated cells (paste as real cells). Original item:
**B4. Whole-message selection copies the source.** When a selection covers an
assistant message from its first to its last row, emit `DisplayMessage.content`
verbatim (the model's markdown, exact and document-ready). This is cheap, exact, and
the most common "copy the answer" case.

**B5. System/error cards copy unbroken.** In the `"system"` arm of
`render_message_into`, record raw overrides mapping each pre-wrapped display line back
to its authored source line (the `"error"` arm already does this). Alternative: stop
pre-wrapping plaintext and let `wrap_lines` wrap. That's a smaller diff, but it changes how
indented help text looks. **Recommend the raw-override route** (no visual change).

**Verify (all against a drawn frame, helper `screen_rows` + `last_chat_frame`):**
soft-wrapped paragraph selected across a wrap → one line; prompt → no `1›`/`│`;
table → pipe table; code → fenced block; error card → unbroken sentence; tool row
inside a larger selection → absent; whole assistant message → exact `content`.
Live: drag across a real transcript, paste into a file, read it.

## C. Large pastes are sent as their placeholder (P0, data loss; found while verifying)

`prompt_input::handle_paste` replaces any paste of 3+ lines or over 150 chars with
`[Pasted ~N lines #K]` and stores the content in `PromptInput.paste_contents`.
**Nothing ever reads `paste_contents`**: there is no expansion on submit. The model receives
the placeholder (observed live: the model replied about "Pasted ~1 lines #1").

- Fix: expand `[Pasted ~N lines #K]` tokens from `paste_contents` at submit (and
  when recalling history), then clear the map. jcode does the same expansion on send.
- Verify: test that pastes 200 chars and submits; the submitted text equals the pasted text.

## D. Found during execution (live smoke test, step A binary)

- **Guardrail halt reads as a user interrupt + failed turn.** `process_tool`'s
  `GuardrailDecision::Halt` (`agent/stream.rs:542`) triggers the USER interrupt
  flag, so the loop emits `Error { "Interrupted by user" }` and returns
  `Err("Interrupted by user")`: two error cards plus "Prompt restored — the turn
  failed", for a halt the agent chose. Needs its own stop reason (a final
  content message, turn ends Ok).
- **Guardrail warnings glue into the answer.** `⚠ Tool 'x' has failed N times…`
  is emitted as raw `Content` with no separator, so it runs into the model's
  sentence and into the next warning. These should be system annotations, not
  answer text.
- **Failed tools render `✓`.** `web_search` failed (missing tool group) and
  every failed `process` call showed `✓ process`.

## E. Carry-overs

- Live-verify the iter-795 (local) fixes once item 0 lands: rows render once while
  tools run; Current reasoning hides at the next prompt.
- BUGS.md K-3 (scroll-mode watermark) and K-4 (`--no-default-features` build) stay open.

## Order + gates

0 → A (A1+A2, one iter) → C (one iter, small) → B1+B2 (one iter) → B5 → B3+B4 (one iter).
Each one: `cargo fmt` (revert unrelated churn), `cargo test -p operant-cli --bin operant`,
`cargo test -p operant-core` (A, scoped to client/agent), corpus `_run.py verify`,
operant-cli clippy diff, then push, release build, deploy both paths, gateway restart, md5 check.
