# generative-UI surface IR, governance and rasterizer — coverage survey

Dated-frozen: audited `2026-11-18`; findings mark verified vs asserted.

Scope: does `operant` (7-crate Rust workspace at
`/home/ishanp/Documents/github/my-projects/agentic-harness/operant`, v0.3.0)
have the seams the CopilotKit/OpenBot "composable generative UI" proposal needs,
and what does the tree actually look like where that proposal would land?

Method: every claim below was verified by reading source bytes in the working
tree (rg/sed/head, cited per row). Claims that rest on the user-supplied OpenBot
digest or on inference rather than a read are marked **[asserted]**. Numbers are
derived, not copied (RG-008/RG-010/RG-011).

## Findings

### 1. The tree has NO existing generative-UI / surface-IR machinery [verified]

- `rg -l 'generative_ui|generative-ui|a2ui|SurfaceOp|WidgetNode'` over `crates/`
  → 0 hits in code. The concept does not exist; this is greenfield.
- The closest existing thing is the `clarify` tool → `user_question.rs` oneshot
  (TUI ask-dialog / gateway choice prompt) and the tool-permission approval flow
  (`approval.rs` + gateway `approval:` / `choice:` / `skill:` inline keyboards).
  Those are the reuse anchors the plan builds on.

### 2. Tool budget is real and tight [verified]

- 55 `registry.register` calls in `tools/builtin.rs`; live schemas ~96 tools /
  ~18.7k tokens per request (`config.rs` `ToolSettings::deferred_tools` doc,
  measured 2026-11-12). `tool_search` deferral exists (`deferred_names`,
  `tools/tool_search.rs`) and is the operator's only lever to shrink the wire
  without disabling tools.
- Consequence: the red-team's "merge 9 tools → 2" ruling is correct and
  load-bearing here. 9 new always-on tools would push the per-request schema
  bill past the deferral budget for no benefit.

### 3. DLP scrubber exists and is the one to reuse [verified]

- `crates/operant-core/src/redaction.rs` exports `redact_sensitive_text`,
  `redact_sensitive_text_if_enabled`, `redact_api_message_content`,
  `mask_secret`, `redact_id`, `redact_chat_id`. Patterns cover `sk-`/`ghp_`/
  `AIza` prefixes, env assignments, bearer headers, private-key blocks,
  connection strings, JWTs, Telegram tokens, URL userinfo, query params, JSON
  fields. Runtime toggle `OPERANT_REDACT_SECRETS` (default on).
- Already consumed by `agent/message_safety.rs` and `gateway/telegram.rs` — a
  surface gate that calls it is not a second scrubber.

### 4. HITL suspend/resume machinery exists (clarify) but is in-memory [verified]

- `user_question.rs` `try_send_user_question` → oneshot → TUI `ask_user_dialog`
  / gateway `PENDING_USER_QUESTIONS` (120 s timeout). The red-team §A1
  durability concern is REAL for this tree: the pending map is a process-global
  `OnceLock<Mutex<HashMap>>` in `gateway_runner.rs` (~7340), not a DB row.
- A v1 surface IR can reuse it; a durable `PendingSurfaceAction` + CAS is a
  genuinely larger change (owned as RT-598 successor work, not this plan).

### 5. TUI render path is single-renderer and resize-aware [verified]

- One flatten layer (`operant_model/adapter.rs::display_messages`), one frame
  assembly (`operant_ui/ui_prepare.rs`), viewport re-layout on resize
  (`ui_viewport.rs`), alt-screen default with `terminal_scroll_mode` config.
- `TurnState` (tui/app/enums.rs) has `WaitingForApproval` already; no new
  suspend variant needed.
- Overlay stack: `overlays/layout.rs::modal_layout`, `ask_user_dialog.rs`,
  `key_input_dialog`, `keybindings/report.rs` — a new interactive surface is a
  dialog, not a new keybinding universe.

### 6. Gateway image + callback capabilities [verified]

- Telegram adapter: `send_choice_prompt` (inline keyboard, `choice:<idx>`),
  `handle_callback_update` (approval/choice/skill families), `editMessageText`,
  4096-char splitting, `telegram_prompt_outcome` error surfacing. NO
  `sendPhoto`/`sendDocument`/media upload exists yet.
- Slack adapter: `chat.postMessage` only; `start()` logs and returns (outbound
  only, RT-86). No Socket Mode, no `files.uploadV2`, no Block Kit actions.
- HMAC machinery in tree: `hmac 0.12`, `sha2 0.10`, `ring 0.17` are workspace
  deps; the webhook adapter already does Slack `v0:` HMAC verification
  (`gateway/webhook.rs::verify_request_signature`).

### 7. Rasterizer dependency reality [verified]

- `image = 0.25` is a direct dependency of BOTH `operant-core` and
  `operant-cli` (`Cargo.toml` workspace deps; `Cargo.lock` shows
  `operant-cli -> image`, `operant-core -> image`; `png 0.18.1` transitive).
- `resvg`, `tiny-skia`, `usvg`, `cosmic-text`, `fontdb`, `ab_glyph`, `fontdue`,
  `imageproc` are ALL ABSENT from `Cargo.lock` (0 entries each).
- The user-digest's Option A (resvg/tiny-skia SVG card renderer) would add 5+ new
  crates + a font stack to a tree that deliberately avoids them. The
  TestBackend-buffer → `image::RgbaImage` path (red-team Option B) is the only
  option whose dependencies are already linked. This is the single biggest
  divergence from the digest and the plan records it.

### 8. "TOON in the ecosystem" is false for this tree [verified]

- No `toon` crate in `Cargo.toml` or `Cargo.lock`. The only `toon` strings are
  test fixtures (`agent/evidence/recall.rs` test marker `["toon", "cap"]`).
- The digest's token-savings claim rests on TOON being available; here it would
  be a small hand-rolled row parser, not a crate feature. Recorded as a
  deviation, not silently inherited.

### 9. Config surface conventions [verified]

- `AppConfig` (`config.rs`) is the ONE config system; sections are struct
  fields with serde defaults; `operant.example.toml` is updated in the same
  change per AGENTS.md rule. A new `[ui]` section with one boolean follows the
  `ToolSettings` / `TuiSettings` precedent exactly. No parallel config crate.

### 10. DB conventions [verified]

- One DB family per file (`migrations.rs` header): sessions→`database.db`,
  cron→`operant_cron.db`, kanban→`operant_kanban.db`. A new table would be a
  new family; the plan avoids one by reusing the clarify oneshot (v1) and
  filing the durable queue as successor work.

## What the survey does NOT cover

- CopilotKit/OpenBot internals: not readable from this environment; the digest
  is taken as [asserted]. The plan re-grounds every OpenBot claim against this
  tree before acting on it.
- Slack inbound (Socket Mode), Discord components, WhatsApp interactive
  messages: the tree's adapters are outbound-only or absent; the plan scopes
  gateway v1 to Telegram and records the rest as stretch.
- The prompt composer (`prompt_input/`) and ghost text: unrelated to the
  surface IR; the plan only asserts the IR must not disturb the @-mention
  grammar, verified by the composer's existing tests.

## Verdict

Every seam the proposal needs exists in some form and can be reused:
scrubber (redaction.rs), HITL (user_question.rs + ask_user_dialog), render
(TUI single-renderer + gateway adapters), governance (ToolRegistry
interceptor seam), image encoding (already-linked `image` crate). The plan
(`docs/plans/2026-11-18-composable-generative-ui-surface-ir-for-the-tui-and-the-gate.md`)
is therefore grounded: 2 tools, 0 new crates, 0 new DB tables, 1 config
boolean, Telegram-only gateway v1, TestBackend-buffer PNG (not resvg), and
the durable-HITL gap documented rather than hidden.