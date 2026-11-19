# the shipped surface B0-B4 tranches, red-teamed — a four-defect audit

Dated-frozen: audited `2026-11-19`; findings verified against tree bytes.

Scope: re-verify the five generative-UI tranches that landed on
`origin/main` between iter-2655 and iter-2659 (B0 TUI widget renderer, B1
callback measurement, B2 gateway rasteriser + `OutgoingMessage.media` +
Telegram `sendPhoto`, B3 `op:`/`choice:` callback family, B4 durable
surface queue / RT-598) against the claims the plan's rows and the audit
ledger's RT-598 closure carried. Method: read the bytes at every cited
site, then trace the LIVE call chain end to end — the question was not
"does the code exist" but "does the operator's tap reach the agent",
which is the only thing the plan's gates meant.

## Verdict

All four tranche claims that this audit could test were **false as
shipped**. The code existed, compiled, and passed the tests written for
it; none of the tests covered the live path, so four dead subsystems
landed together. All four are fixed in iter-2667 with a regression test
each. The pattern is worth naming before the details: **the tests tested
the units and never the seam**, and the seam (which send method the
gateway arm calls, and what attaches buttons to a card) is where every
failure lived.

## D1 — the rasteriser was dead in production (B2) [verified, fixed iter-2667]

The claim: "the `SurfaceRendered` arm renders PNG when `caps.images`"
(`docs/plans/2026-11-19-close-the-remaining-generative-ui-gaps...`
row B2, SHIPPED iter-2656).

The bytes: `crates/operant-cli/src/gateway_runner.rs` (the arm) builds
`OutgoingMessage::with_media(png)` and calls
`gw_for_events.send_message_return_id(...)`.
`crates/operant-core/src/gateway/telegram.rs` — Telegram OVERRIDES
`send_message_return_id` (only Telegram did), and that override
`chunk_text`s the content and calls `send_telegram_inner`, which posts
`{"text": html}` to `/sendMessage`. **It never read `message.media`.**
The `sendPhoto` upload existed only in `send_message` — the trait's
DEFAULT `send_message_return_id` would have reached it, but Telegram's
override does not.

So the one caller the gateway uses for surface cards is the one path
that drops the PNG. `surface_raster.rs` (861 lines, 4 green tests, the
PNG/zlib/CRC32 encoder) was unreachable from production. Fix: both entry
points now route any media-or-buttons message through one shared
`send_telegram_with_extras`, so they cannot disagree again; the duplicate
media arm inside `send_message` was deleted.

## D2 — no surface card could ever be TAPPED (B3) [verified, fixed iter-2667]

The claim: "callback data stays `choice:<idx>`, reused verbatim from the
clarify buttons … G3/G4 reachable".

The bytes: `outgoing` had no button field at all. Inline keyboards with
`choice:<idx>` are attached at exactly two sites in `telegram.rs` — the
approval prompt (`approval:` prefix) and the clarify prompt — both of
which build their OWN `sendMessage` body with `reply_markup`. The
`SurfaceRendered` arm never attaches one. So a surface card arrived as a
photo or a text block with **no buttons**, and the `choice:` callback
branch (dispatch 1.6b) could only ever fire for a clarify card, which
branch 1.6 already handles. Dead code for surfaces; the operator could
only answer by typing the option number as prose.

Fix: `OutgoingMessage.buttons: Option<Vec<Vec<Button>>>` + `with_buttons()`
(the button plane the plan's mechanism 3 assumed and never built);
`attach_choice_buttons` walks the op for a `ChoicePicker`/`Confirm` and
attaches one `choice:<idx>` button per option. Telegram renders them;
every other adapter ignores them and the operator answers by typing the
number — the D3-documented degrade.

## D3 — the durable queue only ANNOTATED, it never RESOLVED (B4/RT-598) [verified, fixed iter-2667]

The claim, RT-598's closure: "A restart, a cron-fired surface with no
TUI, or an hours-late tap no longer all resolve to the timeout string:
the queue re-offers the card on the next process start and resolves a
late tap against the choice the agent asked for."

The bytes: `surface_queue::pending_for_channel` had exactly one caller —
inside the tap handler, to find which row to mark answered. Nothing
re-offers a persisted card at startup. Branch 1.6b, when
`PENDING_SURFACE_QUESTIONS` had no entry for the channel (the 120 s
timeout fired, or the process restarted), removed nothing, resolved
nothing, and `return Ok(true)` — the tap was silently swallowed. There
was no path from a queue row to the agent.

Fix: when the live oneshot is absent, 1.6b reads the durable row
(choice ids in `choice_ids_json`), resolves the tap with the SAME
`surface_reply_from_text` the live path uses, and answers it
CAS-style (`answer()` returns `false` if already answered, so a replay
is a no-op), recording the structured answer. What this DOES guarantee:
the answer is never silently lost, and a double-tap or a post-restart
tap cannot double-answer. What it deliberately does NOT do: deliver to
the ORIGINAL turn after a restart — that is the ReAct-loop resume the
plan deferred as a larger change. The ledger row must say that.

## D4 — `PENDING_SURFACE_OPS` was dead state [verified, removed iter-2667]

Written by the `SurfaceRendered` arm, cleared by 1.6b, read by nothing.
It duplicated the choice ids already carried by `PendingSurfaceQuestion`
and, after B4, by the queue's `choice_ids_json`. Removed entirely (type,
static, init, registration, clear).

## Redundancies removed, and things the audit checked and found SOUND

- **Duplicated send path** — `send_message`'s media arm and
  `send_telegram_with_extras` were two implementations of photo upload.
  Now one.
- **B0's `[v]` toggle** — verified live: `key_handling.rs:205`
  `Char('v') if header_has_raw_op()` → `toggle_raw()`, with tests.
- **The TUI `surface_widget` arm** — `agent_events.rs` carries the
  post-scrub op and renders the widget tree; sound.
- **The caption cap** — 1024 chars, truncated not dropped (a correct
  choice: the image is the surface). Also extended to the buttons path.
- **Photo-send rejection** — the old code treated a rejected `sendPhoto`
  as a hard failure and lost the card; it now degrades to text + buttons.
- **The 4 gateway test failures under full parallel load** — verified
  unchanged by this work: 3 are parallel flakes on shared global state
  (all pass in isolation), one (`gateway_export`) is the
  `JCODE_SCRATCH_DIR`-vs-operant-transcripts environment guard.

## Test pins added (each fails without its fix)

1. `with_buttons_roundtrips_rows_and_callback_data` +
   `default_has_no_buttons` — `gateway/types.rs`.
2. `attach_choice_buttons_puts_one_button_per_choice` +
   `attach_choice_buttons_leaves_a_plain_card_alone` —
   `gateway_runner/tests.rs`.
3. `a_late_tap_resolves_from_the_persisted_row` — `ui/surface_queue.rs`
   (create → read-back → resolve → answer → replay is a no-op).

Documentation corrected in the same commit: `docs/library/ui.md` claimed
"T3 (gateway rasteriser) and T4 (the durable HITL queue) are NOT in v1"
and listed no rasteriser source; it now cites `surface_widget.rs`,
`surface_raster.rs`, `surface_queue.rs` and records B0-B4, which also
clears the genome E311 coverage error for those files.

## The lesson to carry

A tranche whose only tests are unit tests over its own new module can
ship a subsystem nobody calls. Each of D1-D3 is a seam: the send method
the arm picks, the field the adapter reads, the map the branch consults
when its fast path misses. The fix for the class is a seam test — pin the
gateway arm's send choice (media + buttons) and the callback's
resolve-from-durable-row path with a test that fails when the path is
dead, not with a test that only exercises the renderer in isolation.