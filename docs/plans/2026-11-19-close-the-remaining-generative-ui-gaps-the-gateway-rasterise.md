# close the remaining generative-UI gaps: the gateway rasteriser, the callback seam, and the durable HITL queue

Dated-frozen: published `2026-11-19`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a `Resume here` section).

**STATUS: PLANNED (outline, not started).** Nothing below has been built. Every
claim was read from `origin/main` @ `452b9d26` on 2026-11-19; the survey it rests
on is `docs/audit/2026-11-18-generative-ui-surface-ir-governance-and-rasterizer-survey.md`
(dated-frozen, verified reads), and the predecessor plan is
`docs/plans/2026-11-18-composable-generative-ui-surface-ir-for-the-tui-and-the-gate.md`
(declared living: T1/T2/T3-gateway-half landed, T3-rasteriser and T4 open).

**AMENDED iter-2648.** The first draft listed the TUI side as shipped and
scoped this plan to the gateway. Both were wrong: `agent_events.rs:647` renders
a surface as plain text, no `ratatui::Widget` impl for a `WidgetNode` exists,
`confirm`/`text` cards are unhandled on BOTH hosts, and the predecessor's G2
(`[v]` raw toggle) has no implementation at all. Tranche B0 and gate G0 are
added for that; D1-D4 are unchanged.

This is an OUTLINE, not a build order to be followed blindly. Its job is to fix
the four decisions below BEFORE code, because three of them are cases where the
predecessor plan's own text cannot ship as written.

## Contents

- [What is actually left](#what-is-actually-left)
- [The four decisions this outline exists to make](#the-four-decisions-this-outline-exists-to-make)
- [Mechanism](#mechanism)
- [Gates / success criteria](#gates--success-criteria)
- [Tranches](#tranches)
- [Cost](#cost)
- [Explicitly out of scope](#explicitly-out-of-scope)

## What is actually left

Verified against `origin/main`, not against the predecessor's prose:

| item | state | evidence |
|---|---|---|
| `ui/` IR + gate + two tools | SHIPPED | `crates/operant-core/src/ui/{mod,gate,plain,request}.rs` |
| TUI `ui_request_input` reachable (choice) | SHIPPED | `tui/app/channel_drains.rs:416` |
| **TUI widget renderer** | **ABSENT** (corrected iter-2648) | `tui/app/agent_events.rs:647` calls `plain::render_plain`, a `String` |
| **`confirm` / `text` card kinds on BOTH hosts** | **ABSENT** (corrected iter-2648) | only a `ChoicePicker` arm at `channel_drains.rs:416` and `gateway_runner.rs:7731` (line corrected iter-2650; it read 7714, which is a lock clone) |
| **`[v]` raw-JSON toggle (predecessor G2)** | **ABSENT** (corrected iter-2648) | 4 `Char('v')` bindings in `keybindings/defaults.rs`, all vim; no `v` arm for the dialog |
| Gateway registers + drains a surface receiver | SHIPPED (iter-2640) | `gateway_runner.rs:5978` (set_surface_request_sender; line corrected iter-2650, it read 5957 which is a session-bus clone) + receiver task |
| Turn-scoped capabilities (fail closed to `text_only`) | SHIPPED (iter-2640) | `SurfaceCapabilities::for_transport` in `tools/ui_tool.rs` |
| `docs/library/ui.md` + INDEX row | SHIPPED (iter-2641) | `docs/library/ui.md` |
| **Gateway rasteriser** | **ABSENT** | no `gateway/surface_raster.rs`, no `cell_buffer.rs` |
| **Font for rasterisation** | **ABSENT** | zero `.ttf`/`.otf`/`.woff` in the tree |
| **`OutgoingMessage` attachment field** | **ABSENT** | `gateway/types.rs:246` has no media field |
| **`sendPhoto` on any adapter** | **ABSENT** | 0 hits across `gateway/` |
| **`op:` callback family** | **ABSENT** | `telegram.rs:1339/1360/1377` handle `approval:`, `choice:`, `skill:` only |
| **Durable HITL queue (RT-598)** | **OPEN** | ledger row RT-598, owner "the plan's successor" |

So the remaining work is four things: a renderer, a photo path, a callback
family, and the durability RT-598 records. The first three are T3; the fourth
is T4.

**CORRECTION (iter-2648, verified against `origin/main` @ `452b9d26`).**
The first draft called the TUI side shipped. It is not, and the gap is not
cosmetic: `tui/app/agent_events.rs:647` renders a `SurfaceRendered` by calling
`plain::render_plain(...)`, which returns a `String` — so every `WidgetNode`
(`Card`, `Table`, `BarChart`, `Progress`, `CodeDiff`, `StatusTimeline`, …)
reaches the TUI as monospaced text at `FALLBACK_MODEL_WIDTH = 100`. There is no
`ratatui::Widget` impl for a `WidgetNode` anywhere under `crates/operant-cli/
src/tui/`. `SurfaceCapabilities::tui()` sets `inline_widgets: true`
(`ui/mod.rs:106`) and `check_capabilities` reads it (`ui/mod.rs:1004`), so the
GATE claims widgets the BUILD cannot draw — the capability describes the
transport, not this tree. Three sub-gaps follow, and all three are still open:

1. **The widget renderer itself** — the largest single item, and a prerequisite
   for the rasteriser being worth writing: the gateway PNG would mirror a tree
   the TUI cannot yet draw.
2. **Interactive kinds are `choice` only on BOTH hosts.**
   `channel_drains.rs:416` and `gateway_runner.rs:7731` each match
   `ChoicePicker` and fall through `_ => Vec::new()`, so a `confirm` or `text`
   card shows the question with no field. `ui_tool.rs`'s own description
   advertises "choice / confirm / text field", so the contract outruns both
   hosts. (Not a hang: the dialog still resolves on a typed line.)
3. **The `[v]` raw-JSON toggle does not exist.** The predecessor's G2 asserts
   it. `rg "KeyCode::Char\('v'\)"` finds 4 bindings, all vim visual modes; the
   dialog has no `v` arm. `gate.rs:24` already promises the raw view "shows the
   post-scrub op", so this is a promised behaviour with no implementation — and
   G2 cannot pass until it exists.

So the TUI tranche (call it **B0**) runs before B2: the rasteriser's cell grid
and the `[v]` toggle both assume a widget path that is not there yet. This does
not change D1-D4, which remain gateway-scoped; it adds one tranche ahead of B2
and one gate (G2) that the first draft had no way to satisfy.

## The four decisions this outline exists to make

### D1 — The `op:` callback payload does not fit. Measured, not asserted.

The predecessor plan specifies `op:` as "b64url HMAC over
surface_id/action_id/user_id/chat_id, message_id verified post-send".
Telegram caps `callback_data` at **64 bytes** (`telegram.rs:785` says so, and
the `choice:<idx>` / `skill:<idx>` families were built around it). Measured
against realistic values:

```
op:surf_01H8X2K4M9QRSTUVWXY/act_yes/477281923/-1002345678901/XjGPjPnL4kk
  -> 72 bytes   (OVER the cap by 8)
```

The obvious trim — dropping `chat_id` — does not work: `chat_id` is part of
what the HMAC is supposed to authenticate, so removing it re-opens the
cross-chat replay the field exists to prevent. **This tranche must resolve the
constraint before writing a callback**, and the resolution has to be argued,
not chosen. The candidates, in the order they should be tested:

**RESOLVED (iter-2651, B1).** Measured on 2026-11-19 (HMAC-SHA256 over the four
fields, realistic ids `surf_01H8X2K4M9QRSTUVWXY` / `477281923` /
`[REDACTED:card]`):

| Candidate | Wire bytes | Fits 64? |
|---|---|---|
| raw MAC over all four fields | 104 | no |
| 8-byte MAC over the same four | 72 | **no (OVER by 8)** |
| `op:{sid}:{action_idx}:{mac64}` | 41 | yes |
| `op:{opaque 8-byte handle}` | 14 | yes |

**Chosen: `op:{opaque 8-byte handle}`** keyed by a server-side row
(`pending_surface_action`). It wins on every axis that matters here: it fits
with room to spare, it does not drop `chat_id` (keeping the replay property), it
is strictly simpler than the index shape (which still needs the row to map
index → action), and it is the same shape across the channels that can carry a
`callback_data` and those that cannot (Discord/Slack). Dropping to a truncated
MAC over the four fields does NOT fix D1 and was never going to: the cap is the
four id fields, not the MAC.

**Cost this forces:** the opaque handle implies a pending-row table under the
migrations conventions. That is the same schema B4/RT-598 needs, so **the
"0 new DB tables" invariant breaks here (B3), not in B2** — one migration,
shared by the callback and durability tranches. The invariant violation is
recorded rather than hidden.

The invariant to pin stands: **the callback cannot name a surface the sender
could not already see.** Under the opaque handle, the tap can only reference a
row that was written when the surface was offered, so it cannot.

### D2 — The rasteriser must not add a font stack. The audit already ruled.

The audit (§7) measured `resvg`, `tiny-skia`, `usvg`, `cosmic-text`, `fontdb`,
`ab_glyph`, `fontdue`, `imageproc` as **all absent** from `Cargo.lock`, and
ruled that the SVG path "would add 5+ new crates + a font stack to a tree that
deliberately avoids them." It endorses the alternative: **`TestBackend` buffer
to `image::RgbaImage`**, which needs no new dependency, because `image = 0.25`
is already a direct dep of both `operant-core` and `operant-cli`.

The consequence to state plainly: with no font crate, the rasteriser is a
**cell-grid** renderer — each terminal cell becomes an NxN block of pixels,
colours from the existing `SemanticTone` to RGB mapping, and glyphs are drawn
as a built-in bitmap font or not at all. That is a deliberate quality ceiling,
not a shortcut. If the operator later wants real text in the PNG, that is a
SEPARATE plan with a font dependency decision in it, not an amendment here.

### D3 — The 64-byte cap is Telegram's, not the protocol's. Design for the weakest adapter.

`choice:` and `skill:` are Telegram-only because inline keyboards are
Telegram-only. The `op:` family must not inherit that assumption: Slack
(RT-86, outbound-only, no Block Kit) and the other four adapters have no
callback mechanism at all. So the honest v1 scope is **Telegram is the only
adapter that can answer a surface card**, and the gateway's surface receiver
must degrade to the existing text fallback elsewhere rather than pretending a
button exists. The plan's predecessor already states this shape for the
rasteriser ("gateway-only stretch"); it has to be stated for callbacks too.

### D4 — Durability is a schema decision, not a timeout tweak.

RT-598 says an in-flight `ui_request_input` does not survive a restart, and
that closing it means "persisted queue + ReAct-loop resume + TUI-vs-gateway
split-brain serialization". The split-brain half is the dangerous one: today
the TUI and the gateway each hold their own pending map, and a durable queue
means **both** could resolve the same surface. The single owner has to be
chosen before the first row is written. This plan's position: the durable row
is the authority, the in-memory oneshot becomes a fast path over it, and the
resolver CASes the row so the second host loses cleanly instead of both
answering.

## Mechanism

Four seams, each already existing (this is the plan's main safety property —
none of it requires a new subsystem):

1. **Cell-buffer raster** — `operant-core/src/gateway/` gains a renderer that
   takes a `WidgetNode` tree plus `SurfaceCapabilities` and produces an
   `image::RgbaImage`. It walks the SAME widget vocabulary as
   `ui/plain.rs::render_plain`, so the PNG and the plain fallback cannot
   disagree about what the surface contains. No new crate (D2).
2. **Photo delivery** — `OutgoingMessage` gains an optional media field
   (`gateway/types.rs`), and the Telegram adapter grows a `sendPhoto` that
   multipart-uploads the PNG. The other five adapters must COMPILE against the
   field and ignore it, degrading to the text fallback, so this is an
   interface change with five no-op arms.
3. **Callback family** — `telegram.rs`'s dispatcher gains one more
   `starts_with("op:")` arm beside the existing three, verifying the MAC
   BEFORE touching any state and resolving through the pending map. The
   payload shape is D1's outcome.
4. **Durable HITL queue** — one table, one owner, CAS resolution (D4), with
   the existing in-memory maps kept as the hot path.

## Gates / success criteria

(fixed before the run — these are the plan's own tests, not aspirations)

- **G1** a surface renders to a PNG that is byte-reproducible for the same op
  and differs when any `SemanticTone` changes; the PNG's dimensions are
  derived from the tree's measured depth/node count, not a constant.
- **G2** the rasteriser and `plain.rs` are proven to agree: for every widget
  kind, the text the plain renderer emits is a substring of what the PNG
  encodes (pinned by walking the IR, not by eyeballing an image).
- **G3** a forged `op:` callback is refused before any state is touched, and
  the refusal is indistinguishable from an unknown surface (no oracle).
- **G4** a callback that fits the 64-byte cap is proven to fit **by
  measurement in the test**, not by a comment claiming it does. This is the
  gate D1 exists to enforce.
- **G5** an adapter without callback support degrades to text and says so;
  no adapter silently drops the card (the iter-2640 rule).
- **G6** RT-598 closes only with a test that kills the process mid-prompt and
  shows the answer still resolving exactly once after restart.
- **G0** (added iter-2648) the TUI draws a `WidgetNode` as a WIDGET, not as
  text: a `Card` snapshot shows a bordered box, a `Table` shows aligned
  columns, and a `BarChart` shows bars — asserted against a `TestBackend`
  frame at 120x40 AND 55x24, with zero argument truncation (the predecessor's
  G2, which no draft could satisfy because no widget path existed). The `[v]`
  toggle shows the POST-SCRUB op as JSON: feed a card carrying `sk-…`, press
  `v`, and assert the rendered JSON contains the mask, not the key.

Every gate gets a mutation proof in the same iteration that lands it. A green
run is not evidence.

## Tranches

Ordered by dependency, each leaving the tree compiling and net-positive.

- **B0 — the TUI widget renderer + the `[v]` toggle (added iter-2648; G0).**
  A `render_widget(&WidgetNode, area, buf)` covering all **19** variants in a new
  `crates/operant-cli/src/tui/surface_widget.rs`; `agent_events.rs:647` switches
  from `render_plain` to it, keeping `plain.rs` as the scrollback/text
  fallback; `confirm` and `text` arms added beside the `ChoicePicker` arm at
  `channel_drains.rs:416` (`Confirm` maps to `["Yes","No"]`; `TextField` maps
  to `options = None`, which `open_with_header` already treats as
  `in_custom_input`); `[v]` toggles the raw post-scrub op JSON. Lands before
  B2 because the rasteriser mirrors a tree the TUI must already be able to
  draw. Watch the render-layer invariants: ONE renderer, no live partials in
  settled rows (AGENTS.md "TUI visual layer").

  **The count is 19, not 17 (corrected iter-2650).** Counted from the enum, not
  from the predecessor's prose: `Row, Column, Card, Tabs, List, Divider, Text,
  Badge, KeyValueGrid, Table, BarChart, Progress, CodeDiff, StatusTimeline,
  Button, ChoicePicker, TextField, Confirm, Modal`. iter-2648 wrote "17",
  which would have had a renderer silently drop two variants and still pass a
  visual review. `ui/plain.rs` already matches all 19, so the fallback B0 keeps
  is sound -- the error was in the plan, not the code. A future count change
  must be re-derived the same way (walk the enum), never copied.
- **B1 — decide D1 and write it down.** No code. Amend the predecessor plan
  with the measured 64-byte result and the chosen payload shape, with the
  measurement in the amendment so the next reader cannot re-derive the wrong
  answer. Cheapest tranche and it unblocks B3.
- **B2 — cell-buffer raster + `OutgoingMessage` media field + Telegram
  `sendPhoto`.** G1, G2, G5. No callback yet: a surface that renders but has
  no buttons is still a strict improvement over a text wall.
- **B3 — the `op:` callback family** over B1's payload shape. G3, G4. Lands
  only after B1; writing it first is how the cap gets rediscovered in code.
- **B4 — RT-598 durable queue.** G6. Largest tranche; depends on B2/B3 only
  for the surfaces it must persist, not for their internals.

## Cost

- **Zero new crates.** Verified: `image 0.25` (both crates), `png 0.18.1`
  (transitive), `hmac 0.12`, `sha2 0.10`, `ring 0.17` are all already
  present; the eight font/SVG crates the audit lists are absent and stay
  absent (D2).
- **One config surface, reused.** The raster width/scale belongs under the
  existing `[ui]` section next to `generative_enabled`; no new file.
- **One DB table**, added by B4 only.
- **Tool count unchanged.** No new tools: the renderer is gateway-internal,
  reached from the `SurfaceRendered` event the tool already emits.
- **No change to `ui_tool.rs` schemas**, so the per-request tool-token bill
  does not move — the audit's finding 2 (96 tools / ~18.7k tokens) is a real
  constraint this plan is careful not to worsen.

## Explicitly out of scope

Carried from the predecessor's own "NOT in v1" list, unchanged, plus what this
plan adds:

- WASM custom widgets and a Draft-Published widget registry (the plugins
  crate's `manifest.toml` + `ToolRegistry` capability seam already suffices).
- resvg / tiny-skia / usvg card renderer, and any real font stack (D2 — a
  font decision is its own plan).
- TUI form cards (`kind: form` stays refused TUI-side).
- `ui_request_input(kind: approval)` (routes through the existing permission
  dialog).
- TOON streaming hydration at 30 fps.
- Slack Block Kit actions / `files.uploadV2` (Slack stays outbound-only,
  RT-86; this plan does not touch it).

## Resume here

Nothing landed. Start at B1, which is a measurement and a plan amendment, not
code. If B1's measurement contradicts the predecessor's stated `op:` shape,
the amendment records the contradiction and the successor's shape wins — that
is the whole point of putting it first.

**Read order: B0 first if the TUI is the target, B1 first if the gateway is.**
They are independent (B0 touches `operant-cli/src/tui/`, B1 is a measurement),
so either can lead — but B2 must not start before whichever of B0/B1 it
depends on: B2 mirrors a widget tree B0 has to exist for it to mirror.
