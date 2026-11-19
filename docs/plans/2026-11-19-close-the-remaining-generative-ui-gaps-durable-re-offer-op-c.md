# Plan — close the remaining generative-UI gaps: durable re-offer, the `op:` callback seam, the G6 kill test, and the split-brain owner

**Date:** 2026-11-19 (revision 1)
**Base:** `origin/main` @ `150a6f55`
**Status:** outline. Writing this file executes nothing.
**Predecessor:** `docs/plans/2026-11-19-close-the-remaining-generative-ui-gaps-the-gateway-rasterise.md`
(B0-B4, landed iter-2655..iter-2659, then red-teamed and partially fixed in
iter-2667/iter-2668 — see
`docs/audit/2026-11-19-generative-ui-b0-b4-red-team-four-dead-subsystems.md`).

This plan owns the GENERATIVE-UI surface gaps the red-team audit left open
AFTER its fixes. It does not re-open the four defects the audit closed (D1
rasteriser send path, D2 button plane, D3 tap resolution, D4 dead state) —
those are pinned by tests and landed. It closes the gaps the audit found and
DEFERRED, plus the three plan rows whose text no longer matches the shipped
code.

**Boundary with the broader ledger plan.** `docs/plans/2026-11-19-executing-the-remaining-implementation-gaps-post-2668.md`
(peer, iter-2669) sequences the open RT-* ledger rows (RT-136, RT-209,
RT-275, RT-285, RT-293, RT-543, ...). This plan does NOT touch those rows and
does not schedule ledger hygiene; it is the surface-specific follow-up to the
surface audit, and the two plans meet only at RT-598 (this plan's C1/C3/C4
close the surface half of that row; the peer's plan does not carry it).

---

## Resume here

Nothing is implemented. Five tranches, in this order, one commit each:

| # | Tranche | Gap | Risk |
|---|---|---|---|
| C1 | the durable re-offer at startup | the queue is write-and-tap-only; nothing re-offers a card after a restart | medium |
| C2 | the `op:` callback seam (or its retirement) | B1's opaque handle and G3's forged-callback refusal have no code; B3 shipped `choice:<idx>` instead | low |
| C3 | the G6 kill test | the plan's own gate requires a process-kill seam test; none exists | low |
| C4 | the split-brain owner | D4: the TUI and the gateway each hold a pending map; a durable queue means both could resolve one surface | high |
| C5 | the doc-truth pass | the B3/B4 rows and the audit's own claims vs the shipped bytes | none |

Read the tranche's section before starting it: each names its files, its `done
when`, and its rollback. Re-verify every claim against the live tip first —
this file is measured on `150a6f55`.

---

## Why these five and not others

The predecessor plan's own "Explicitly out of scope" (resvg, real fonts, TUI
form cards, approval surfaces, TOON, Slack Block Kit) stays out of scope — none
of it is a gap, it is a decision. What remains is what the audit measured and
left open:

1. **The queue does not replay (C1).** `surface_queue::pending_for_channel` has
   exactly one caller, inside the tap handler. Nothing re-offers a persisted
   card at startup, so the audit's own corrected RT-598 row is still half a
   promise: "the tap resolves durably" is true, "the card is re-offered after a
   restart" is not. C1 is the smallest change that makes it true.
2. **The `op:` callback family does not exist (C2).** B1 measured four payload
   shapes and chose `op:{opaque 8-byte handle}`; no code implements it. B3
   shipped the clarify-style `choice:<idx>` on surface cards instead — which
   works and is simpler, but it means B1's measurement is dead, G3's
   forged-`op:`-callback refusal has nothing to refuse, and the plan's B3 row
   ("the `op:` callback family") describes code that was never written. C2
   decides: either build the `op:` seam as measured, or retire the row and
   record `choice:<idx>` as the shape that won. The plan's own rule
   ("the successor's shape wins") applies.
3. **The G6 gate has no test (C3).** G6 says "RT-598 closes only with a test
   that kills the process mid-prompt and shows the answer still resolving
   exactly once after restart." The queue's tests are unit tests over the
   table; no test kills a process. C3 adds the seam test G6 names.
4. **The split-brain has no owner (C4).** The audit's D4 (predecessor plan's
   numbering) — the TUI and the gateway each hold a `PENDING_SURFACE_QUESTIONS`
   map, and the durable queue means both could resolve the same surface. The
   audit documented this as "a higher layer's job"; it is the last dangerous
   open item and the one a restart actually triggers.
5. **The docs drift from the bytes (C5).** B3's row claims the `op:` family
   shipped; B4's row claims G6 reachable; the audit doc's own "what this does
   NOT do" is right but the ledger and plan rows were only half-corrected. C5
   is a no-code pass that makes every claim match the tree.

---

## Status against the open items

| # | Item | State |
|---|---|---|
| C1 | a persisted card is re-offered to the operator after a restart | open — `pending_for_channel` is called only in the tap handler (`gateway_runner.rs` ~8643) |
| C2 | the `op:{opaque handle}` callback family exists, or is retired with the reason | open — 0 hits for `op:` in `gateway/`; B1's measurement at plan lines 118-140 is unused |
| C3 | G6's kill-the-process-mid-prompt test | open — no `process::exit`/SIGKILL seam test anywhere in `surface_queue.rs` or `gateway_runner/tests.rs` |
| C4 | one owner resolves a surface when both hosts are live | open — the gateway holds `PENDING_SURFACE_QUESTIONS`, the TUI holds its own oneshot in `ask_user_dialog` (`reply_tx: oneshot::Sender<String>`, `ask_user_dialog.rs:53`), and the durable queue sits between them with no cross-host CAS |
| C5 | every claim about B0-B4 matches the shipped bytes | open — B3 row says `op:` family, B4 row says G6 reachable; both false as written |

Nothing in this file is implemented. Each tranche names its own `done when`,
and each is meant to land as its own commit so it can be reverted alone.

---

## C1 — the durable re-offer at startup

**Gap:** a card that was in flight when the process died is in the queue, but
nothing re-offers it. The operator restarts, the card is gone from the chat,
and the queue row sits there until the next tap (which can only come from a
card that no longer exists).

**Change:** at gateway startup (beside the surface receiver's registration),
drain `pending_for_channel` for each channel the gateway is about to serve and
re-send the card. The row carries `op_json` + `choice_ids_json`; re-rendering
uses the same `SurfaceRendered` path (PNG or text + `attach_choice_buttons`),
so the re-offered card is byte-identical to the original. Do NOT reset
`created_at`; the row stays the same row.

**The cap:** re-offer only rows whose `created_at` is within
`SURFACE_REPLY_TIMEOUT` (300 s) — a card older than the tool's own timeout is
not re-offered; it is answered "dismissed" exactly as the live timeout does.
This keeps the re-offer from resurrecting week-old cards.

**Done when:** a test registers a card, drops the in-memory map (simulating
restart), runs the re-offer, and asserts the operator receives the same card
with buttons, AND a card older than 300 s is NOT re-offered. Files:
`gateway_runner.rs` (startup drain), `surface_queue.rs` (an
`offered_at`/`expired` helper if needed).

**Rollback:** remove the startup drain; the queue stays write-and-tap-only
(today's behavior). Low blast radius.

---

## C2 — the `op:` callback seam, or its retirement

**Gap:** B1 measured four payload shapes (plan lines 118-140), chose
`op:{opaque 8-byte handle}`, and the cost note says it forces a pending-row
table. B4 built that table. B3 then shipped `choice:<idx>` — the clarify
shape — which needs no handle and no MAC. So the measurement, the choice, and
the code disagree, and G3 (forged-`op:`-callback refusal) has nothing to
refuse.

**The decision this tranche forces:** build the `op:` seam as measured, or
retire it. The plan's rule: "the successor's shape wins."

- **Build it:** add an `op:` arm beside `choice:` in
  `handle_callback_update`, keyed by the opaque handle; the row is the pending
  queue row (already keyed by `surface_id`); the MAC is verified before any
  state is touched; a forged `op:` is indistinguishable from an unknown
  surface. This makes B1's measurement live and G3 testable.
- **Retire it:** delete the `op:` rows from the plan (B1's measurement table
  and the B3 row's `op:` claim), record `choice:<idx>` as the shape that won
  with the reason (the clarify-style index needs no new table and no MAC), and
  move G3's intent onto the `choice:` path (a forged `choice:999` on a card
  with 2 options is refused by bounds, not by MAC).

**Recommendation:** retire. The index shape is already live, already tested
(`attach_choice_buttons_*`), and the opaque handle adds a MAC-verification
seam for a threat (a forged tap naming a surface the sender could not see)
that the index already prevents by construction (the index is bounded by the
card the operator is looking at). Building `op:` buys nothing and costs the
verification seam. But this is the plan owner's call, not the implementer's:
C2 lands whichever way the owner rules, and the ruling is recorded in the
amendment.

**Done when:** the plan's B1/B3 rows and G3 match the chosen shape, and a
test pins the choice (either a forged-`op:`-refused test, or a
forged-`choice:`-refused test). Files: plan doc + `telegram.rs` (if building)
or plan doc only (if retiring).

**Rollback:** the plan doc edit is the rollback.

---

## C3 — the G6 kill test

**Gap:** G6 says RT-598 closes only with a test that kills the process
mid-prompt and shows the answer resolving exactly once after restart. No such
test exists; the queue's tests are unit tests over the table.

**Change:** a test that (a) registers a card through the real queue, (b) kills
the process (a child `operant` invocation that registers then `process::exit(1)`
mid-prompt, or the in-process equivalent: drop the oneshot map and simulate
restart), (c) re-runs the re-offer (C1) + a tap, and (d) asserts the answer
resolves EXACTLY once — a second tap is a no-op (the CAS). The "kill" can be
an in-process simulation if a real subprocess is too heavy; what G6 actually
pins is "the answer survives the oneshot dying and resolves once," and that is
testable without a real SIGKILL.

**Done when:** the test exists and fails if the CAS is removed. Files:
`surface_queue.rs` tests or `gateway_runner/tests.rs`.

**Rollback:** delete the test. No production code.

---

## C4 — the split-brain owner

**Gap:** the gateway resolves surfaces through `PENDING_SURFACE_QUESTIONS`
(its own map + the durable queue), while the TUI resolves through its own
oneshot in `ask_user_dialog` (`reply_tx`, `ask_user_dialog.rs:53`) — two
independent answer paths over the same surface. The durable queue means both
could resolve the same surface. The audit documented this as "a higher
layer's job" and moved on; it is the one remaining item a restart actually
triggers.

**The decision:** who owns a surface when both hosts are live? Three options:

1. **The queue is the authority (D4 of the predecessor plan).** The row is
   written once; the resolver CASes it (`answer()` already returns false if
   already answered); the second host loses cleanly. The in-memory maps become
   pure fast paths over the row.
2. **The host that rendered the card owns it.** The `SurfaceRendered` event
   carries the host; the other host's tap is routed to the owner.
3. **First-answer-wins with a visible loser.** Both hosts try; the CAS decides;
   the loser says "already answered" and the operator sees which host won.

**Recommendation:** option 1, because `answer()` already implements the CAS
and the queue is already the durable record — the change is small (the TUI's
`ask_user_dialog` and the gateway's 1.6b both resolve through the queue
instead of their private maps) and it is the shape the predecessor plan's D4
named. Option 3 is the fallback if the TUI cannot reach the queue (it runs in
a different process; the queue is a SQLite file, so it CAN — the TUI just
needs to open it).

**Done when:** a test where both a TUI-side and a gateway-side resolver race
the same surface_id, and exactly one wins with the other getting
"already answered." Files: `gateway_runner.rs` (1.6b already CASes), the TUI's
`channel_drains.rs`/`ask_user_dialog` (route through the queue), tests.

**Rollback:** keep the two private maps; the split-brain stays documented as
open. This is the highest-risk tranche because it touches the TUI's answer
path.

---

## C5 — the doc-truth pass

**Gap:** B3's row says "the `op:` callback family" shipped (it did not — see
C2); B4's row says "G6 reachable" (it is not — see C3); the audit doc's own
"what this does NOT do" is right, but the plan and ledger rows were only
half-corrected (iter-2668 corrected RT-598 but not the plan rows).

**Change:** a no-code pass that makes every claim about B0-B4 match the
shipped bytes: the plan's B3 row, the B4 row, the audit doc's verdict, and the
ledger's RT-598 row. This is the same discipline iter-2648/2651 applied to the
predecessor's false "SHIPPED" rows — re-derive from the tree, never copy.

**Done when:** every row in the predecessor plan and the audit doc cites a
file:line that exists and says what the code does. Files: the plan docs and
the ledger only.

**Rollback:** none (docs).

---

## Order and dependencies

C5 is independent (docs only) and can land any time. C1 and C3 are coupled
(C3's kill test needs C1's re-offer to exist). C2 is independent of C1/C3/C4
(it touches the callback dispatcher and the plan doc). C4 is the largest and
touches the TUI answer path; it should land last, after C1's re-offer and C3's
kill test have pinned the queue's behavior.

Suggested order: **C2 (decide the shape) → C1 (re-offer) → C3 (kill test) →
C4 (split-brain owner) → C5 (doc-truth)**.

## Gates

Carried from the predecessor, restated for what remains:

- **G6** (the one this plan closes) — a process-kill test shows the answer
  resolving exactly once after restart.
- **G3** — a forged callback is refused before state is touched; under C2's
  retirement this becomes a forged `choice:<idx>`-out-of-bounds test.
- **G1/G2/G5** — already green (iter-2656), unaffected.
- **New G7** — the re-offered card is byte-identical to the original (same
  `op_json` → same PNG/caption), and an expired row is NOT re-offered.

## What this plan does NOT do

Carried unchanged from the predecessor: resvg/tiny-skia/usvg, a real font
stack, TUI form cards, `ui_request_input(kind: approval)`, TOON hydration,
Slack Block Kit. Plus what the audit added: the four defects it fixed
(D1-D4) stay fixed and pinned; this plan does not reopen them.