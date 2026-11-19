# Surface UI — the governed IR, its gate, and the two agent tools

Owns: what the agent emits when it wants to show structured content
instead of prose — the `WidgetNode` IR, the governance gate that validates
every op, the `ui_render_surface` / `ui_request_input` tools, and which
host actually renders a surface.
Not for: rendering the ordinary transcript ([tui-rendering.md](tui-rendering.md)),
the session's own panels ([tui.md](tui.md)), or injecting content into a
turn ([context-injection.md](context-injection.md)).

Source: `crates/operant-core/src/ui/mod.rs` (the IR types and nothing
else), `crates/operant-core/src/ui/gate.rs` (validation and scrubbing),
`crates/operant-core/src/ui/request.rs` (the host-request channel),
`crates/operant-core/src/ui/plain.rs` (the width-bounded plain-text
fallback), `crates/operant-core/src/ui/surface_queue.rs` (the durable HITL
queue, B4), `crates/operant-core/src/gateway/surface_raster.rs` (the
cell-grid PNG rasteriser, B2), `crates/operant-core/src/gateway/types.rs`
(`OutgoingMessage.media` + the inline-keyboard button plane, B2/B3),
`crates/operant-core/src/tools/ui_tool.rs` (the two tools),
`crates/operant-cli/src/tui/surface_widget.rs` (the TUI widget renderer,
B0), `crates/operant-cli/src/tui/adapter_types/tui_app.rs` (the only
production host registration).

Plan: `docs/plans/2026-11-18-composable-generative-ui-surface-ir-for-the-tui-and-the-gate.md`
and its successor `docs/plans/2026-11-19-close-the-remaining-generative-ui-gaps-the-gateway-rasterise.md`
(B0-B4). Landed: T1a (IR + gate), T1b (the two tools), T2
(`ui_request_input` reachable in the TUI), iter-2640 (the gateway receiver,
turn-scoped capabilities), B0 (the TUI widget renderer + the `[v]` raw
toggle, `tui/surface_widget.rs`), B2 (the gateway rasteriser,
`gateway/surface_raster.rs`, PNG with no font stack) and `OutgoingMessage.media`,
B3 (the `choice:<idx>` button plane on surface cards + the callback
resolution), and B4 (the durable HITL queue, `ui/surface_queue.rs`, which
closed RT-598).

## Why this exists

The alternative to an IR is the model hand-writing Ratatui, Slack or
Telegram markup, which is unvalidated, unrenderable on a second transport,
and unauditable. A governed document is the only shape that can be checked
BEFORE a host sees it: one gate, one scrub, one refusal vocabulary.

## The three invariants (enforced in code, not hoped for)

1. **`SemanticTone` is the only colour surface.** No variant carries a
   `Color`, a hex string, or any other raw colour field. The TUI maps tones
   onto the ANSI palette; a rasteriser maps them onto high-contrast RGB.
   Adding a colour field to a variant breaks the second renderer silently,
   which is why the type forbids it.
2. **A tree is bounded twice**: `MAX_SURFACE_DEPTH = 8` (root is depth 1)
   and `MAX_SURFACE_NODES = 200` (counts every node, so a 200-wide fan-out
   is refused like a 200-deep chain). Checked by
   `check_depth_and_count` in the gate AND again by the tool that parses the
   model's JSON, because the serialized form is handed to providers as part
   of a tool schema.
3. **`Provenance::AgentSynthesis` always renders an `[unverified]` badge.**
   The consumer appends it from the op's provenance; no node can opt out,
   because the badge is added at render time rather than carried as a field
   the model could omit.

## The gate, in order

`gate::validate_surface(op, caps, config)` — and the ORDER is the contract:

1. **Kill switch** — `!config.ui.generative_enabled` refuses with
   `UiRefusal::Disabled` before anything else is looked at.
2. **Depth and node bounds** — `check_depth_and_count`.
3. **Capability match** — `check_capabilities` compares each interactive
   node against the live `SurfaceCapabilities`; a read-only node degrades to
   text everywhere, an interactive one needs `native_buttons` or
   `inline_widgets`, and `Modal` additionally needs `modals`. The set comes
   from THE TURN (`capabilities_for` reads the `transport` key from the
   agent's `ToolContext`, resolving through `SurfaceCapabilities::for_transport`,
   which fails CLOSED to `text_only` when the key is absent). It is NOT
   hardcoded to `tui()`: until iter-2640 it was, so a gateway turn was
   validated against the TUI's full set and the capability gate was
   decorative — it answered "can this transport draw a table?" with "yes"
   on a transport that draws plain text.
4. **Secret scrub** — every string leaf of the op goes through
   `redact_sensitive_text`, so a token pasted into a table cell never
   reaches a renderer. `scrub` is `pub` because the tool ALSO scrubs the
   serialized fallback text, which a leaf walk cannot see.

A refusal is a typed `UiRefusal` whose `Display` names the rule and the
limit ("surface has 412 widgets; the limit is 200"), never a bare enum
variant: the reader is a model that has to fix its own call.

## The two tools

**`ui_render_surface`** builds a `SurfaceOp` from the model's widget JSON
(or the table shorthand: `columns` + row objects), runs the gate, emits
`AgentEvent::SurfaceRendered` via `try_send`, and returns the plain-text
fallback to the model. It NEVER suspends — no oneshot, no await.

**`ui_request_input`** builds a `CreateSurface` op, runs the same gate, then
sends a `SurfaceRequest` over a process-global channel and AWAITS the
host's answer, returning it as structured JSON (`{"kind": "choice", ...}`)
so the model can branch without parsing prose.

### Which host answers — read this before touching a timeout

There are THREE paths, and each bounds a surface prompt differently:

- **TUI** — `tui/adapter_types/tui_app.rs` registers the sender and
  `drain_surface_requests` drains into `ask_user_dialog`, which carries NO
  timeout of its own. So on this path `SURFACE_REPLY_TIMEOUT` (300 s) is the
  ONLY bound, and it is what stops a prompt the operator walked away from
  wedging the turn forever.
- **Gateway** — `gateway_runner.rs` registers a receiver beside the `clarify`
  one and drains it on a 120 s timer, answering "dismissed" when it fires. So
  on this path the 300 s is the OUTER BACKSTOP it was originally documented
  as. It was NOT before iter-2640: the gateway registered no receiver at all,
  so `try_send_surface_request` returned `None` and the tool failed fast
  without ever reaching the await.
- **`operant run`** — hostless by design. `try_send_surface_request` returns
  `None` and `ui_request_input` FAILS FAST with "no interactive surface is
  attached ... ask the user in plain text" rather than ever reaching the
  await.

`tests/surface_host_receivers.rs` (iter-2639, strengthened in iter-2640) pins
this set: every host that registers must also DRAIN, so the next host cannot
repeat the gateway-registered-but-never-drained pattern.

(For contrast: the 120 s `clarify` / `PENDING_USER_QUESTIONS` path is a
SEPARATE mechanism with its own receiver and its own pending map. A surface
answers with structured JSON, so it cannot share `clarify`'s
`oneshot::Sender<String>`.)

### Unattended runs

`ui_request_input` refuses before reaching a host when
`context.metadata["unattended"] == "true"`, so a cron run never waits on a
human that is not there — the same rule `clarify` uses.

## What is NOT in v1

WASM custom widgets, a Draft-Published widget registry (the plugins crate's
`manifest.toml` + `ToolRegistry` capability seam is enough for a plugin to
return a `WidgetNode`), a resvg card renderer (the rasteriser is the
cell-grid `surface_raster.rs` with a built-in bitmap font, no font stack),
TOON hydration at 30 fps, TUI form cards (`kind: form` is refused TUI-side),
and `ui_request_input(kind: approval)` (that routes through the existing
permission dialog, not a surface card).