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
| Gateway HITL timeout | 120 s oneshot, then "(no reply received within 120s — timeout)" | `gateway_runner.rs` pending_uq receiver (~7440) |
| Telegram callback data cap | 64 bytes; index-based `choice:<idx>` / `approval:<a>` / `skill:<idx>` today | `gateway/telegram.rs` `send_choice_prompt`, `handle_callback_update` |
| TUI callback surfaces | approval buttons (Approve/Deny), choice buttons, skill picker | `gateway/telegram.rs` inline_keyboard sites (~649/741/800/871) |
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
4. Layout must be **lossless below 72 cols for interactive nodes**: a test drives
   `WidgetNode::measure(width)` and asserts `ui_request_input` rows never
   ellipsize command/file args at 60 and 55 cols (the red-team §A3 safety hole).

### M2 — Two tools, not nine (`operant-core`, `tools/builtin.rs` + `tools/ui_tool.rs`)

Exactly two new tools (red-team §D3; 55→57 registered; ~96→~98 schemas —
still under the deferral budget, and both new tools are declared
`#[serde(default)] deferred = true`-eligible in `ToolSettings` so an operator
can shrink the wire cost with `operant tools defer ui_render_surface`):

| Tool | Args | Returns |
|---|---|---|
| `ui_render_surface` | `{ surface_id, intent, provenance, body: <widget-json or TOON rows> }` | `ToolResult` carrying the sealed `SurfaceOp` set + `to_plain_fallback()` |
| `ui_request_input` | `{ surface_id, kind: approval|choice|form, question, choices?, form_schema?, provenance }` | resolved action payload (suspends the turn — see M3) |

`ui_render_surface` never suspends: it writes its IR into the transcript as an
emitted row (via the same AgentEvent channel the TUI already drains). It is the
"show a table / chart / diff / metrics" surface.
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
- **Durability gap, accepted and documented:** clarify's oneshot dies on restart
  (in-memory `PENDING_USER_QUESTIONS`, `gateway_runner.rs` ~7340). This plan
  inherits that limit for v1 — an in-flight `ui_request_input` that loses its
  oneshot returns the timeout string, same as clarify does today. Closing it
  (persisted `PendingSurfaceAction` + CAS) is **out of scope for v1** and filed
  as a BUGS.md row (RT-598) owned by this plan's successor; the audit's §A1
  scenario is therefore DOCUMENTED, not fixed, here. Reason: the 120 s oneshot
  with timeout covers the common think-time (a user taps within 2 min), and a
  durable-queue + resume-the-ReAct-loop system is a much larger change than the
  surface IR itself.

### M4 — Governance gate (`operant-core`, `src/ui/gate.rs`)

One function, consulted inside the tool interceptor seam (tools.rs `ToolInterceptEvent`,
registerable via `ToolRegistry::register_tool_interceptor` — already used by WASM
observers, gap G6):

```
fn validate_surface(op: &SurfaceOp, caps: &SurfaceCapabilities, ctx: &UiContext)
  -> Result<SurfaceOp, UiRefusal>
```

Checks, in order (audit-fail-closed: every failure refuses with a typed reason):
1. **Exists & enabled** — config `[ui] generative_enabled = true` (new section,
   default true, mirroring `ToolSettings` conventions; added to
   `operant.example.toml` in the same change per AGENTS.md rule).
2. **Capability match** — `SurfaceCapabilities { inline_widgets, native_buttons,
   modals, images, max_image_bytes }` injected per-turn from the transport
   (TUI = full; Telegram/Slack = images+native buttons; SMS/WhatsApp = text
   fallback only). NOT a binary TUI-vs-gateway fork (red-team §A2); the struct is
   built from the calling transport.
3. **Secret scrub** — walk every string leaf through `redact_sensitive_text`
   BEFORE the op is emitted (red-team §A4: the pixels leave the box via
   Telegram/Slack; the scrubber already exists and is the one used by
   `agent/message_safety.rs` + `gateway/telegram.rs`).
4. **Group-channel policy** — on `is_group_chat` incoming messages, `CodeDiff`
   and raw workspace-table nodes are auto-replaced with their
   `to_plain_fallback()` (text) — the PNG preview stays DM-only (red-team §A4).
5. **Audit row** — `tracing` event `ui.rendered` / `ui.refused` (tool_name,
   surface_id, initiator, refusal_reason). The existing `turn_attribution.rs`
   carries the initiator (TUI operator / gateway user / cron / autonomous).

### M5 — TUI renderer (`operant-cli`, new `tui/operant_ui/ui_surface.rs`)

- Read-only nodes (`Table`, `BarChart`, `Progress`, `CodeDiff`, `StatusTimeline`,
  `Badge`, `Text`, `Divider`, `Card`, `Tabs`, `List`, `KeyValueGrid`) map to
  `ratatui::Widget`s rendered INSIDE the existing scrollback path
  (`operant_ui::ui_prepare::display_messages` / `app/scroll_emit.rs`), not a new
  overlay. Re-layout on `Event::Resize` (the existing viewport already does this
  in `ui_viewport.rs`).
- Interactive nodes (`Button`, `ChoicePicker`, `TextField`, `Confirm`, `Modal`)
  reuse the **existing overlay stack**: `modal_layout` (`overlays/layout.rs:134`)
  + `ask_user_dialog.rs`'s frame primitives. A new `ui_surface_dialog` mirrors
  `ask_user_dialog` (question header + focus ring over fields). Key bindings
  follow the repo's `keybindings/report.rs` registry (Tab/Shift-Tab fields,
  Enter submit, Esc cancel — NOT bare-Esc=Deny; the red-team §A3 collision
  guard). `TurnState::WaitingForApproval` already exists for the suspend
  display; no new enum variant.
- The `[v] view raw JSON` toggle is a `key_input_dialog` variant (already a
  dialog), reachable from every `ui_request_input` card below 72 cols.

### M6 — Gateway PNG snapshotter + native actions (`operant-core/src/gateway`)

- `SurfaceSnapshotRenderer` trait (`async fn render_png(&self, surface,
  RenderOptions) -> Result<Vec<u8>>`) with ONE impl: `CellBufferRenderer` built
  on the `image` crate (M-mine). The `TestBackend` half of the red-team Option B
  is the buffer SOURCE; the `image` crate is the PIXEL TARGET. No resvg, no
  cosmic-text, no fontdb.
- **Embedded font**: one permissive 5x7 bitmap font via `include_bytes!` (a
  ~3 KB binary in `gateway/`); `operant doctor` gains a check that the font
  bytes are present and non-empty (red-team §A7 — blank `□□□` boxes in
  minimal containers). No system-font dependency, ever.
- **Telegram**: upload once via `sendPhoto` (new method on
  `TelegramAdapter`, sibling of the existing `send_choice_prompt`) + caption =
  `to_plain_fallback()`. Native buttons: `InlineKeyboardMarkup` with
  `callback_data = op:<12-byte-id>:<16-byte-HMAC>` (32 B ≤ 64 B cap). The
  existing `handle_callback_update` gains a fourth family (`op:`) that:
  (a) verifies `HMAC(gateway_secret, surface_id || action_id || user_id ||
  chat_id || message_id)` (red-team §A5) using the `hmac`+`sha2` crates ALREADY
  in the tree, (b) checks `sender_id == expected_external_user_id`, (c)
  synthesizes the equivalent inbound message (same pattern as
  `approval_message_from_callback` / `choice_message_from_callback`), (d) edits
  the prompt to strip the keyboard on success.
- **Slack**: OUTBOUND-ONLY today (no Socket Mode; `slack.rs` `start()` logs and
  returns). v1 covers Telegram only; Slack `files.uploadV2` + Block Kit is a
  stretch goal gated on the gateway receiving Slack inbound at all. Recorded so
  no reader assumes parity.
- **Streaming**: text status while streaming (the existing typing indicator +
  streamed partial text the gateway already sends), ONE PNG upload on seal /
  HITL pause, ONE `editMessageMedia`/`editMessageText` on resolve. Never
  re-upload on a debounce (red-team §A6 — 429 storms). The
  `OutgoingMessage` struct (`gateway/types.rs`) gains one `#[serde(default)]
  attachments: Vec<SurfaceAttachment>` field; every existing construction site
  (21 in `gateway/mod.rs`, plus adapters) keeps compiling via `..Default::default()`
  semantics (audit: only `mod.rs`'s 21 + 8 adapter sites use
  `OutgoingMessage::new` / struct literals — mechanical).

## Gates / success criteria

(fixed before the run)

| # | Gate | Mutation-proof |
|---|---|---|
| G1 | `cargo test -p operant-core ui::` green: (a) DLP scrub leaves no `sk-`/`ghp_`/`BEGIN PRIVATE KEY`/`postgres://` in any leaf after render (feed known secrets, assert masked); (b) `SemanticTone` is the only color path (compile-time: no `Color` field exists); (c) `AgentSynthesis` renders the `[unverified]` badge; (d) `measure()` at 55/60 cols never truncates `ui_request_input` args | break (a) → test red |
| G2 | `cargo test -p operant-cli --bin operant tui::operant_ui::ui_surface` green: 120x40 and 55x24 snapshots render a `ui_request_input` with zero arg truncation; `[v]` raw toggle shows full JSON | break measure() → red |
| G3 | `cargo test -p operant-core gateway::` green: (a) forged/wrong-user `op:` callback rejected by HMAC (mutate one byte of expected user id → rejected); (b) callback_data ≤ 64 B; (c) 1 upload + 1 edit per turn under simulated streaming (count `sendPhoto` calls); (d) PNG byte-length > threshold (catches missing-font blank output) | break HMAC verify → red |
| G4 | `bash scripts/check-gates-present.sh` still lists every new test (the self-test gate that catches "gate script silently emptied") | delete a test → red |
| G5 | `./scripts/check.sh check -p operant-core --lib` + `cargo clippy -p operant-core --all-targets -- -D warnings` + `cargo fmt --all --check` clean | n/a |
| G6 | `rg -c 'name = "resvg"|name = "tiny-skia"|name = "cosmic-text"' Cargo.lock` prints `0` (no rasterizer crate added) | add dep → red |
| G7 | `operant.example.toml` gains `[ui] generative_enabled = true` and `operant config set ui.generative_enabled false` flips the gate (kill-switch, OpenBot parity) | flip → G1 gate refuses |

## Cost

- New crates: 0 (`image`, `png`, `hmac`, `sha2`, `unicode-width` already in tree).
- New tools on the wire: +2 (deferrable).
- New DB tables: 0 (clarify oneshot reused; durable queue deferred to successor,
  RT-598).
- New config section: 1 (`[ui]`, one boolean, default true).
- Estimated new LOC: ~1,200 (IR 300 + gate 200 + TUI renderer 250 + gateway
  rasterizer 250 + tool glue 200), excluding tests.
- Rasterizer deps avoided: resvg, tiny-skia, usvg, cosmic-text, fontdb,
  ab_glyph, fontdue, imageproc (8 crates + system fontconfig).

## Tranches (each = one iteration = one gate)

- **T1** — `src/ui/mod.rs` IR + `ui/gate.rs` + `tools/ui_tool.rs` two tools +
  config `[ui]` + example.toml + G1/G5/G6. (operant-core)
- **T2** — `tui/operant_ui/ui_surface.rs` + dialog reuse + `[v]` toggle + G2.
  (operant-cli)
- **T3** — `gateway/` `CellBufferRenderer` + embedded font + `sendPhoto` +
  `op:` callback family + `OutgoingMessage` attachments field + doctor font
  check + G3. (operant-core)
- **T4** — BUGS.md RT-598 (durable HITL queue) filed; Slack upload stretch;
  `docs/library/ui.md` library page + INDEX row + `genome lint` green.
- NOT in v1: WASM custom widgets / Draft-Published registry (red-team §D2 cut;
  the plugins crate's existing `manifest.toml` + `ToolRegistry` capability seam
  is enough for a plugin to return a `WidgetNode` in its tool result), resvg
  card renderer, `ui_render_surface` TOON streaming hydration at 30 fps (the
  composer's ghost-text engine is unrelated; TOON rows stream as the tool result
  arrives, same as every other tool).

## Resume here

Nothing landed yet. Start at T1; `docs/audit/2026-11-18-generative-ui-surface-ir-governance-and-rasterizer-survey.md`
holds the seam-by-seam evidence table the tranches cite.