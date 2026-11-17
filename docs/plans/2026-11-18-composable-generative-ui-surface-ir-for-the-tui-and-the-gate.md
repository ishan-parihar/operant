# composable-generative-UI-surface-IR-for-the-TUI-and-the-gateway-image-preview

Dated-frozen: published `2026-11-18`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**STATUS: PLANNED (not started).** Nothing below has been built. Every file, symbol
and count in this document was read from the working tree on 2026-11-18
(`git rev-parse --short HEAD` at write time: derive it, per RG-008 — do not copy a
number from this page). Where a claim is an inference from the CopilotKit/OpenBot
write-up the user supplied, it is marked [source claim]; where it is a verified read
of this tree, it carries its file path.

Reference material (read-only, NOT modified): the user-supplied OpenBot/CopilotKit
architecture digest of `CopilotKit/OpenBot` (three-tier A2UI surface system: 18
audited primitives, compiled gallery tools, sandboxed playground; governance gates;
HITL suspend/resume) and its red-team audit (durable checkpoints, capability-based
routing, DLP scrubber, HMAC callbacks, single PNG rasterizer).

Predecessors / neighbours:
- `docs/library/gateway.md` — the 7-platform adapter registry (telegram, discord,
  slack, whatsapp, email_smtp, sms_twilio, webhooks).
- `docs/library/tui.md` + `docs/library/tui-rendering.md` — the Ratatui render
  pipeline (one renderer: `display_messages` in
  `crates/operant-cli/src/tui/operant_model/adapter.rs`; frame assembly in
  `crates/operant-cli/src/tui/operant_ui/ui_prepare.rs`).
- `docs/library/tools.md` — the ToolRegistry, tool_search deferral, the
  `DeferredNames` / `disabled_names` machinery.
- `docs/plans/2026-10-24-omp-style-prompt-composer-upgrade-typing-system-mention-file.md`
  — the prompt composer's @-mention grammar (the IR must not disturb it).
- `docs/audit/2026-11-18-generative-ui-surface-ir-governance-and-rasterizer-survey.md`
  — the coverage audit this plan is grounded in (dated-frozen, audit).

## What this plan is

A governed **Surface IR** (`SurfaceOp` / `WidgetNode` / `Provenance` / `SemanticTone`)
plumbed through THREE existing seams of this tree, adding exactly TWO tools to the
agent's surface, ONE image backend to the gateway, and a render path into the TUI.
No new workspace crate. No second database. No parallel config system.

```
  ┌─ operant-core (the agent) ──────────────────────────────────┐
  │  ui_render_surface  (Tier 1, config-gated, TOON/JSON rows)  │
  │  ui_request_input   (Tier 2, HITL: approval/choice/form)    │
  │        │ writes SurfaceOp IR into ToolResult.content        │
  │        ▼                                                    │
  │  ToolRegistry interceptor seam (tools.rs:561) validates     │
  │  + scrubs (redaction.rs) + audit-writes BEFORE dispatch     │
  └────────┬───────────────────────────────────┬───────────────┘
           │                                   │
  ┌────────▼─────────┐              ┌──────────▼──────────┐
  │ operant-cli TUI  │              │ operant-core gateway │
  │  WidgetNode →    │              │  WidgetNode → PNG    │
  │  ratatui::Widget │              │  (image crate,      │
  │  (lossless       │              │   embedded font,     │
  │  <72-col wrap)   │              │   no resvg/tiny-skia)│
  │  receipts in     │              │  + native buttons    │
  │  scrollback      │              │  + HMAC callback     │
  └──────────────────┘              └──────────────────────┘
```

## Measured ground truth (verified reads, 2026-11-18)

Each row carries the command or file that derives it. Do NOT re-copy these numbers
into code or docs without re-deriving (RG-008 / RG-010 / RG-011).

| Fact | Value | Evidence |
|---|---|---|
| Workspace crates | 7: api, os, config, core, cli, plugins, harness | `Cargo.toml` `members` |
| Version | 0.3.0 | `Cargo.toml` `[workspace.package]` |
| Registered builtin tools | 55 `registry.register` calls in `builtin.rs` | `rg -c 'registry.register' crates/operant-core/src/tools/builtin.rs` → 55 |
| Live tool schemas per request | ~96 tools / 18,732 tok (comment, measured 2026-11-12) | `crates/operant-core/src/config.rs` `ToolSettings::deferred_tools` doc |
| Tool deferral | per-tool deferral behind `tool_search` bridge | `tools/tool_search.rs`, `ToolRegistry::deferred_names` (tools.rs:586) |
| Secret scrubber | `redaction.rs` `redact_sensitive_text` / `redact_sensitive_text_if_enabled` / `redact_api_message_content` | `crates/operant-core/src/redaction.rs` |
| DB file families | sessions→`database.db`, cron→`operant_cron.db`, kanban→`operant_kanban.db` | `migrations.rs` header + `config.rs` `database_path` |
| TUI entry | alt-screen by default; `terminal_scroll_mode` config (default false) | `tui/app/mod.rs` (`enter_alt_screen`, `terminal_scroll_mode`), `ui_viewport.rs:422` |
| Turn state enum | `Idle/Sending/Thinking/Streaming/RunningTool/WaitingForApproval/WaitingForNetwork/Compacting` | `tui/app/enums.rs` `TurnState` (line ~186) |
| Existing HITL | `clarify` tool → `user_question.rs` oneshot → TUI `ask_user_dialog` / gateway choice prompt | `crates/operant-core/src/user_question.rs`, `tools/clarify_tool.rs`, `gateway_runner.rs` PENDING_USER_QUESTIONS |
| Gateway HITL timeout | 120 s oneshot, then "(no reply received within 120s — timeout)" | `gateway_runner.rs` pending_uq receiver (~4027) |
| Telegram callback data cap | 64 bytes; index-based `choice:<idx>` / `approval:<a>` / `skill:<idx>` today | `gateway/telegram.rs` `send_choice_prompt`, `handle_callback_update` |
| TUI callback surfaces | approval buttons (Approve/Deny), choice buttons, skill picker (TELEGRAM, not TUI — corrected iter-2612) | `gateway/telegram.rs` inline_keyboard sites (~649/741/800/871) |
| Image crate | `image = 0.25` (default-features=false, features jpeg/png/gif/webp) ALREADY a dep of operant-core AND operant-cli | `Cargo.toml` `[workspace.dependencies]`; `Cargo.lock` |
| PNG crate | `png 0.18.1` in lock (transitive via image) | `Cargo.lock` |
| No resvg/tiny-skia/usvg/cosmic-text/fontdb/ab_glyph | 0 entries | `rg -c '^name = "..."$' Cargo.lock` per crate |
| HMAC/SHA2 in tree | `hmac 0.12`, `sha2 0.10`, `ring 0.17` already workspace deps | `Cargo.toml` |
| Unicode width | `unicode-width 0.2` already workspace dep | `Cargo.toml` |
| Existing MCP serve wire cost | 94 tools = 75,016 B ≈ 18,754 tok/request | `docs/plans/2026-10-28-native-capability-extension-...md` (cited) |

### Consequence for the rasterizer choice

The red-team's "one PNG engine" ruling is correct, and this tree makes the choice
for us: **the `image` crate is already a direct dependency of both `operant-core`
and `operant-cli`.** Adding `resvg`/`tiny-skia`/`cosmic-text`/`fontdb` would pull
5+ new crates and a system-font dependency into a tree that deliberately avoids
them (they are all absent from the lock). The lean path is:

- **Primary (TUI-parity): a `TestBackend`-buffer → PNG renderer built on the
  already-linked `image` crate** — walk the cell buffer, paint glyphs onto an
  `image::RgbaImage`. No font shaping: render the box-drawing/label glyphs as
  embedded 5x7 or 8x8 bitmap font (include_bytes!, tiny), fill fg/bg per cell.
  Deterministic, no system fonts, no new deps.
- **NOT Option A (resvg/blitz SVG card renderer)** — the red-team's "keep one
  engine" wins; between the two, the TestBackend path is the only one whose
  dependencies are ALREADY in this tree. Record the deviation from the
  user-digest recommendation here, in the same iteration that lands it.

## Mechanism

### M1 — Surface IR types (`operant-core`, new module `src/ui/mod.rs`)

One file, `#![deny(missing_docs)]`-clean, `serde`-derived. No `anyhow` (library
crate; typed `Error` variants per `error.rs` convention).

```rust
// src/ui/mod.rs  (sketch — final field sets land with the tests that pin them)
pub enum SurfaceOp { CreateSurface{..}, UpdateComponents{..}, UpdateDataModel{..}, SealSurface{..} }
pub enum WidgetNode { Row{..}, Column{..}, Card{..}, Tabs{..}, List{..}, Divider,
                      Text{..}, Badge{..}, KeyValueGrid{..}, Table{..}, BarChart{..},
                      Progress{..}, CodeDiff{..}, StatusTimeline{..},
                      Button{..}, ChoicePicker{..}, TextField{..}, Confirm{..}, Modal }
pub enum Provenance { ToolVerified{tool_name, source_uri:Option<String>}, UserSupplied, AgentSynthesis }
pub enum SemanticTone { Default, Muted, Accent, Success, Warning, Danger }
```

Rules (enforced by unit tests, not comments):
1. Every string leaf implements `to_plain_fallback() -> String` (used when an
   image upload fails; also the TUI scrollback text).
2. `SemanticTone` is the ONLY color surface; no `Color`/`hex` field exists on any
   node. The TUI maps tones→ANSI palette; the PNG maps tones→high-contrast RGB.
3. `Provenance::AgentSynthesis` on any node makes the renderer append a
   `[unverified]` badge — unit test asserts it cannot be suppressed.
4. The recursive `WidgetNode` tree is depth-bounded (`MAX_SURFACE_DEPTH = 8`)
   and node-count-bounded (`MAX_SURFACE_NODES = 200`) at deserialization and
   again in `validate_surface`; an over-deep/over-wide op is refused with a
   typed reason (bounds the per-tool JSON Schema the providers see).
5. Layout must be **lossless below 72 cols for interactive nodes**: a test drives
   `WidgetNode::measure(width)` and asserts `ui_request_input` rows never
   ellipsize command/file args at 60 and 55 cols (the red-team §A3 safety hole).

### M2 — Two tools, not nine (`operant-core`, `tools/builtin.rs` + `tools/ui_tool.rs`)

Exactly two new tools (red-team §D3; 55→57 registered; ~96→~98 schemas —
deferrable behind `tool_search` so the wire cost is operator-controlled: they
ship eager by default and an operator shrinks the budget with `operant tools
defer ui_render_surface` (the `ToolSettings::deferred_tools` state file the
registry reads at `tools.rs:586` — NOT a per-tool serde flag; the plan's
earlier `#[serde(default)] deferred = true` phrasing was wrong and is
corrected here per review F3). The recursive 19-variant `WidgetNode` schema is
deep; `validate_surface` bounds its serialized depth (see M1) so the per-tool
schema cannot blow provider recursion limits):

| Tool | Args | Returns |
|---|---|---|
| `ui_render_surface` | `{ surface_id, intent, provenance, body: <widget-json or TOON rows> }` | `ToolResult` carrying the sealed `SurfaceOp` set + `to_plain_fallback()` |
| `ui_request_input` | `{ surface_id, kind: approval|choice|form, question, choices?, form_schema?, provenance }` | resolved action payload (suspends the turn — see M3) |

`ui_render_surface` never suspends: it writes its IR into the transcript as an
emitted row (via the same AgentEvent channel the TUI already drains). It is the
"show a table / chart / diff / metrics" surface. The emission contract is
explicit (review F18): the op rides a dedicated `AgentEvent` surface variant
(plus a `database.db` session-transcript block so a reloaded session re-renders
the card instead of dumping raw JSON into a text stream); `gateway_runner.rs`
collects emitted SurfaceOps for `OutgoingMessage.attachments`.
`ui_request_input` suspends (M3). The two cover the OpenBot gallery split without
adding `ui_show_table` / `ui_show_metrics` / `ui_show_chart` / `ui_show_diff` /
`ui_show_kanban` / `ui_ask_approval` / `ui_ask_choice` / `ui_ask_form` as separate
tools — one discriminated enum each, matching the red-team merge.

TOON: accepted alongside JSON for `Table` rows / `UpdateDataModel` payloads.
Note: the user-digest claims "TOON in the ecosystem"; this tree has NO `toon`
crate in `Cargo.toml`/`Cargo.lock` — the `toon` mentions are test fixture
strings (`agent/evidence/recall.rs` test marker). So TOON support is a SMALL
hand-rolled parser in `ui_tool.rs` (rows = `k:v, k:v` lines), NOT a new crate
dependency. This is a deviation from the digest, recorded here because the
digest's assumption is false for this tree.

### M3 — Durable HITL (`operant-core` + `operant-cli`, reusing clarify's seam)

The red-team §A1 ruling (never hold a Tokio future across human think-time) maps
onto machinery that ALREADY EXISTS here: the `clarify` tool's oneshot →
`user_question` channel → gateway 120 s timeout → TUI `ask_user_dialog`. Do NOT
invent a parallel `PendingSurfaceAction` table.

- `ui_request_input` reuses `try_send_user_question(question, choices, session)`
  for the TUI + choice case. The TUI `ask_user_dialog` already renders a question
  + numbered options and returns the selected index (`ask_user_dialog.rs`
  `confirm()` / `select_by_number()`). Extend the dialog (one new field: the
  surface's `to_plain_fallback()` header) rather than adding a second dialog.
  Measured key precedence (`key_handling.rs`): the permission arm (~158) runs
  FIRST, the ask-user arm (~199) second — so a choice card opened while a
  permission dialog is up waits its turn, and Esc on the ask-user dialog
  dismisses the question instead of denying an approval. `kind: approval` must
  NOT reuse this path:
  it routes through `approval.rs`'s `ApprovalMode` + the permission dialog
  (`permission_request` / `handle_permission_key`), which owns the y/Y/p/n
  semantics. `kind: form` is NOT supported on the TUI in v1 (the dialog has no
  field widgets): form cards are a gateway-only stretch; the TUI refuses a
  form op with `to_plain_fallback()` text and a `ui_request_input(kind:
  choice)` follow-up. Telegram forms degrade to a conversational question
  (send_choice_prompt-style numbered options; free-text answers via the next
  plain message) — never a fake form.
- **Durability gap, accepted and documented:** clarify's oneshot dies on restart
  (in-memory `PENDING_USER_QUESTIONS`, `gateway_runner.rs` ~2181/4027). This plan
  inherits that limit for v1. The honest failure (review F11): on a process
  restart the oneshot map AND the Tokio turn are gone — no timeout string is
  returned to the model; the turn is orphaned mid-tool-call and outstanding
  Telegram buttons fail on click. The 120 s timeout string only fires while the
  process stays up. Closing it (persisted `PendingSurfaceAction` + CAS) is
  **out of scope for v1** and filed as a BUGS.md row (RT-598) owned by this
  plan's successor; the audit's §A1 scenario is therefore DOCUMENTED, not fixed,
  here. Reason: the 120 s oneshot with timeout covers the common think-time (a
  user taps within 2 min), and a durable-queue + resume-the-ReAct-loop system
  is a much larger change than the surface IR itself.

### M4 — Governance gate (`operant-core`, `src/ui/gate.rs`)

One function, called from the surface EMISSION and the `ui_request_input`
dispatch path — NOT from the ToolRegistry interceptor seam (review F8): a
pre-execute interceptor runs before `SurfaceOp` exists, and a post-execute
interceptor runs after `ui_render_surface` already emitted its AgentEvent and
after `ui_request_input` already suspended and displayed the card. The seam
call is: `ui_tool.rs` builds the op, calls `validate_surface`, then emits:

```
fn validate_surface(op: &SurfaceOp, caps: &SurfaceCapabilities, ctx: &UiContext)
  -> Result<SurfaceOp, UiRefusal>
```

Checks, in order (audit-fail-closed: every failure refuses with a typed reason):
1. **Exists & enabled** — config `[ui] generative_enabled = true` (new section,
   default true, mirroring `ToolSettings` conventions; added to
   `operant.example.toml` in the same change per AGENTS.md rule).
2. **Capability match** — `SurfaceCapabilities { inline_widgets, native_buttons,
   modals, images, max_image_bytes }` built per-turn from the transport and
   injected through `<turn_context>` on the latest user message (B-LAW prompt-
   cache rule: the frozen system prefix must not carry per-turn volatile
   state). TUI = full; Telegram = images+native buttons (the ONLY image+button
   adapter); Discord/Slack/WhatsApp/Email/SMS = text fallback only (Slack is
   OUTBOUND-ONLY — see M6; Discord receives but has no inline keyboard in this
   tree); SMS = text only. `is_group_chat || is_channel_post` incoming ⇒
   `caps.images = false` (the PNG preview stays DM-only by CAPABILITY, not by
   rewriting nodes inside the op — review F9).
3. **Secret scrub** — walk every string leaf through `redact_sensitive_text`
   BEFORE layout (red-team §A4). Two measured limits the plan acknowledges
   (review F8): the scrubber is regex MASKING, not fail-closed DLP refusal, and
   a leaf-wise walk misses secrets split across sibling cells (half a token in
   two `Table` cells) — so the gate ALSO scrubs the serialized fallback text,
   and `[v] view raw JSON` shows the post-scrub op, never the pre-scrub one.
4. **Group-channel policy** — on `is_group_chat || is_channel_post`, images are
   refused by capability (2), so no op-level node rewriting is needed; the
   `to_plain_fallback()` text is what ships.
5. **Audit row** — `tracing` event `ui.rendered` / `ui.refused` (tool_name,
   surface_id, initiator, refusal_reason). The initiator (TUI operator /
   gateway user / cron / autonomous) rides the op's `Provenance` field set by
   the calling seam — `turn_attribution.rs` exists in the tree but is not
   load-bearing for v1.

### M5 — TUI renderer (`operant-cli`, new `tui/operant_ui/ui_surface.rs`)

- Read-only nodes (`Table`, `BarChart`, `Progress`, `CodeDiff`, `StatusTimeline`,
  `Badge`, `Text`, `Divider`, `Card`, `Tabs`, `List`, `KeyValueGrid`) map to
  `ratatui::Widget`s rendered INSIDE the existing scrollback path
  (`operant_model/adapter.rs::display_messages` feeding
  `operant_ui/ui_prepare.rs`, then `app/scroll_emit.rs` in terminal_scroll_mode
  or the alt-screen viewport otherwise), not a new overlay. Re-layout on
  `Event::Resize` (the existing viewport already does this in
  `ui_viewport.rs`); `terminal_scroll_mode = true` keeps the inline viewport —
  surfaces re-render at the new width on the next frame but the scrollback
  emitter's append-only watermark does NOT retroactively re-wrap (documented
  limit, not a bug).
- Interactive nodes (`Button`, `ChoicePicker`, `TextField`, `Confirm`, `Modal`)
  reuse the **existing overlay stack**: `modal_layout` (`overlays/layout.rs:134`)
  + `ask_user_dialog.rs`'s frame primitives. A new `ui_surface_dialog` mirrors
  `ask_user_dialog` (question header + focus ring over fields). Key bindings
  follow the repo's `keybindings/report.rs` registry (Tab/Shift-Tab fields,
  Enter submit, Esc cancel — NOT bare-Esc=Deny; the red-team §A3 collision
  guard). The suspend state: `ask_user_dialog` does NOT set
  `TurnState::WaitingForApproval` today (measured: `channel_drains.rs` opens the
  dialog without touching turn state, and `tui_state_impl.rs:481` renders
  WaitingForApproval as `RunningTool("permission")` status text — reusing it
  would show approval chrome for a choice card). So v1 adds ONE new variant,
  `TurnState::WaitingForSurface` (enums.rs), rendered as
  `RunningTool("surface")`, dispatched by the dialog's own key block (the
  `ui_surface_dialog` arms in `key_handling.rs`, placed after the permission
  arm so approvals keep precedence). This corrects the plan's earlier "no new
  enum variant" claim (review F19: verified — WaitingForApproval IS approval-
  specific).
- The `[v] view raw JSON` toggle is a `key_input_dialog` variant (already a
  dialog), reachable from every `ui_request_input` card below 72 cols.

### M6 — Gateway PNG snapshotter + native actions (`operant-core/src/gateway`)

- **Crate-direction resolution (review F4, the plan's hardest correction):**
  `ratatui` is a dependency of operant-cli ONLY (`rg 'ratatui' crates/operant-core/Cargo.toml`
  → empty; `crates/operant-cli/Cargo.toml` → present). operant-core CANNOT
  reuse T2's `ratatui::Widget` renderer. v1 injects the layout from the CLI
  side: `gateway_runner.rs` (operant-cli) owns a `SurfaceLayoutFn` callback
  (a `fn(&WidgetNode, width) -> Vec<StyledCell>`) built from the TUI's widget
  mapper; the gateway core consumes the resulting cell grid, never ratatui.
  The `SurfaceSnapshotRenderer` trait (`async fn render_png(&self, surface,
  RenderOptions) -> Result<Vec<u8>>`) and `CellBufferRenderer` (cell grid →
  `image::RgbaImage`) live in operant-core; the `TestBackend` buffer is the
  cell SOURCE and the `image` crate is the PIXEL TARGET. No resvg, no
  cosmic-text, no fontdb.
- **Embedded font**: one permissive 5x7 bitmap font via `include_bytes!` (a
  ~3 KB binary in `gateway/`); `operant doctor` gains a check that the font
  bytes are present and non-empty. Coverage is HONEST (review F13): ~95–256
  glyphs, so box-drawing (`─│┌┐└┘├┤┬┴┼`) and block elements (`█▓▒░`) MUST be
  rendered GEOMETRICALLY (lines/rects drawn as pixels, never glyph lookups),
  and any non-ASCII text (CJK, emoji) falls back to `□` per cell with a
  one-line note in the caption. `unicode-width 0.2` (already in tree) drives
  cell width. `operant doctor`'s font check is a PRESENCE check only — the
  real assertion is G3(d)'s rendered-glyph test.
- **Telegram**: upload once via `sendPhoto` (new method on `TelegramAdapter`,
  sibling of the existing `send_choice_prompt`; verified ABSENT today —
  `rg sendPhoto crates/operant-core/src/gateway/telegram.rs` → 0 hits) +
  caption = `to_plain_fallback()`. PNGs are integer-upscaled 2× or 3× before
  upload so Telegram's JPEG recompression cannot blur 1-px 5x7 strokes;
  `sendDocument` is the fallback when upscaling is insufficient. Native
  buttons: `InlineKeyboardMarkup` with
  `callback_data = op:<b64url(12B-id)>:<b64url(16B-HMAC)>` — 43 B ≤ 64 B cap
  (review F7: raw 32 B is impossible; Telegram callback_data is UTF-8 text).
  The HMAC binds `surface_id || action_id || expected_user_id || chat_id`
  — NOT `message_id`, which Telegram assigns only AFTER sendPhoto returns
  (review F7, causal fix): the pending entry records `message_id` post-send,
  and `handle_callback_update` verifies `callback.message.message_id`
  against it. The `hmac`+`sha2` crates are ALREADY in the tree (0.12.1/0.10.9).
  The signing key is `[gateway] webhooks_secret` (ALREADY a config field,
  `config.rs:1973`, consumed by the webhook adapter — reused, not a second
  secret), provisioned under the operant home via the one-resolver rule.
- **Slack**: OUTBOUND-ONLY today (no Socket Mode; `slack.rs` `start()` logs and
  returns). v1 covers Telegram only; Slack `files.uploadV2` + Block Kit is a
  stretch goal gated on the gateway receiving Slack inbound at all. Recorded so
  no reader assumes parity.
- **Streaming**: text status while streaming (the existing typing indicator +
  streamed partial text the gateway already sends), ONE PNG upload on seal /
  HITL pause, ONE `editMessageMedia`/`editMessageText` on resolve. Never
  re-upload on a debounce (red-team §A6 — 429 storms). The
  `OutgoingMessage` struct (`gateway/types.rs`) gains one `#[serde(default)]
  attachments: Vec<SurfaceAttachment>` field. Compilation impact, measured
  (review F16 corrected the plan here): there are NO struct literals outside
  `types.rs` itself (2 hits, both the definition) — every one of the 79
  construction sites across the workspace uses `OutgoingMessage::new(..)`
  (`rg -c 'OutgoingMessage::new' crates/` → 79) — so adding the field with a
  `Default` impl on the struct plus one line in `new()` keeps ALL of them
  compiling mechanically. `#[serde(default)]` alone would NOT have (it affects
  serde only, not Rust initializers).

## Gates / success criteria

(fixed before the run)

| # | Gate | Mutation-proof |
|---|---|---|
| G1 | `cargo test -p operant-core ui::` green: (a) DLP scrub leaves no `sk-`/`ghp_`/`BEGIN PRIVATE KEY`/`postgres://` in any leaf after render (feed known secrets, assert masked); (b) `SemanticTone` is the only color path (compile-time: no `Color` field exists); (c) `AgentSynthesis` renders the `[unverified]` badge; (d) `measure()` at 55/60 cols never truncates `ui_request_input` args | break (a) → test red |
| G2 | `cargo test -p operant-cli --bin operant tui::operant_ui::ui_surface` green: 120x40 and 55x24 snapshots render a `ui_request_input` with zero arg truncation; `[v]` raw toggle shows full JSON | break measure() → red |
| G3 | `cargo test -p operant-core gateway::` green: (a) forged/wrong-user `op:` callback rejected by HMAC (mutate one byte of expected user id → rejected); (b) callback_data ≤ 64 B (UTF-8 bytes, base64url); (c) 1 upload + 1 edit per turn under simulated streaming (count `sendPhoto` calls); (d) PNG renders every box-drawing/block cell as the correct GEOMETRIC pattern (decode the PNG, assert border pixels present at expected coordinates) and every non-ASCII cell as the □ fallback — catches both the missing-font blank output and the tofu-box regression | break HMAC verify → red |
| G4 | `bash scripts/check-gates-present.sh` still lists every new test (the self-test gate that catches "gate script silently emptied") | delete a test → red |
| G5 | `./scripts/check.sh check -p operant-core --lib` + `cargo clippy -p operant-core --all-targets -- -D warnings` + `cargo fmt --all --check` clean | n/a |
| G6 | `rg -c 'name = "resvg"|name = "tiny-skia"|name = "cosmic-text"' Cargo.lock` prints `0` (no rasterizer crate added) | add dep → red |
| G7 | `operant.example.toml` gains `[ui] generative_enabled = true` and `operant config set ui.generative_enabled false` flips the gate (kill-switch, OpenBot parity) | flip → G1 gate refuses |

## Cost

- New crates: 0 (`image`, `png`, `hmac`, `sha2`, `unicode-width` already in tree).
- New tools on the wire: +2 (deferrable).
- New DB tables: 0 (clarify oneshot reused; durable queue deferred to successor,
  RT-598).
- New config section: 1 (`[ui]`, one boolean, default true) + the existing
  `[gateway] webhooks_secret` reused as the callback HMAC key (no new secret).
- Estimated new LOC: ~1,200 (IR 300 + gate 200 + TUI renderer 250 + gateway
  rasterizer 250 + tool glue 200), excluding tests.
- Rasterizer deps avoided: resvg, tiny-skia, usvg, cosmic-text, fontdb,
  ab_glyph, fontdue, imageproc (8 crates + system fontconfig).

## Tranches (each = one iteration = one gate)

Sliced VERTICALLY where the review (F14/F17) showed horizontal slicing does not
ship: each tranche must leave the tree compiling and net-positive.

- **T1** — `src/ui/mod.rs` IR + `ui/gate.rs` + `tools/ui_tool.rs`
  (`ui_render_surface` read-only only) + config `[ui]` + example.toml +
  `AgentEvent` surface variant + `display_messages` scrollback render of
  read-only nodes + G1/G5/G6. Ship together: the IR without a renderer is
  dead weight; the renderer without the IR is unbuildable. (operant-core +
  the cli-side render arm.)
- **T2** — `ui_request_input` (choice only) + `ask_user_dialog` header field +
  `TurnState::WaitingForSurface` + `ui_surface_dialog` (choice) + `[v]` toggle
  + G2. (operant-cli; the tool lands in the same iteration it becomes
  reachable.)
- **T3** — `gateway/` `CellBufferRenderer` + embedded font + geometric
  box-drawing + `sendPhoto` + `op:` callback family (b64url HMAC over
  surface_id/action_id/user_id/chat_id, message_id verified post-send) +
  `OutgoingMessage` attachments + doctor font check + G3. (operant-core +
  `gateway_runner.rs` layout injection.)
- **T4** — BUGS.md RT-598 (durable HITL queue) filed; Slack upload stretch;
  `docs/library/ui.md` library page + INDEX row + `genome lint` green.
- NOT in v1: WASM custom widgets / Draft-Published registry (red-team §D2 cut;
  the plugins crate's existing `manifest.toml` + `ToolRegistry` capability seam
  is enough for a plugin to return a `WidgetNode` in its tool result), resvg
  card renderer, `ui_render_surface` TOON streaming hydration at 30 fps (the
  composer's ghost-text engine is unrelated; TOON rows stream as the tool result
  arrives, same as every other tool), TUI form cards (`kind: form` refused
  TUI-side, gateway-only stretch), `ui_request_input(kind: approval)` (routes
  through the existing permission dialog, not a surface card).

## Resume here

Nothing landed yet. Start at T1; `docs/audit/2026-11-18-generative-ui-surface-ir-governance-and-rasterizer-survey.md`
holds the seam-by-seam evidence table the tranches cite.