# Remaining implementation gaps — 2026-10-17

**Written for:** the next agent picking up the 2026-10-16 audit execution.
Ground state: `origin/main` @ `iter-899`, worktree `~/worktrees/audit-exec`
(detached, tree clean at that commit + the in-flight step-5 edit below).

**Landed so far (iters 862–899, all pushed):** 02 test gate · 06A gateway
turn lock · 03 §1–§6 TUI drives agent · 09 surgical config writes (incl. the
setup wizard, owner-directed) · 05A `/model` switches the agent ·
04 one command table · 07 §1′+§2 message watermark + native replay ·
08 memory unification · 06B seat memory banks · 05B model+effort inline
picker (port + legacy overlay deletion) · 10 crate purge phase 1 (−179,411
lines) · G3 clock-organ liveness. 17 of the 20 roadmap items are done.

---

## 0. IMMEDIATE BLOCKER — the in-flight step-5 edit does not compile

`App::sync_overlay_alt_screen` (app/commands.rs) calls `execute!` on
`terminal.backend_mut()`, which requires `B: std::io::Write`; the fn's bound
is only `Backend + SynchronizedUpdate` → 4 × E0599 "method cannot be called
on `&mut B`".

Fix (mirror `run_suspended_shell_command`, tui_app.rs:1225): bound the fn
`B: Backend + SynchronizedUpdate + std::io::Write` and `use std::io::Write;`
inside it. ratatui's `TestBackend` implements `Write`, so the headless corpus
stays runnable. The rest of the edit (field `overlay_alt_active`, the
`full_screen_overlay_open()` classification, the live-loop + sim-loop call
sites, `request_native_replay` on close) is complete and reviewed.

---

## 1. Plan 07 steps 3–7 (native scroll) — roadmap item 18/20

Ordered per the audit's own fix order.

| Step | Work | Acceptance | Notes |
|---|---|---|---|
| **3** | Dynamic live-region height + early paragraph emission; spike ratatui 0.30 inline `resize` first | live region grows/shrinks with the streaming tail; settled paragraphs emit as they cross the boundary instead of waiting for the message to finalise | Run BEFORE step 5's alt-screen work interacts with the viewport |
| **4** | Reasoning collapse line `▸ thought for Xs` (resurrect `reasoning_heading()`, `#[allow(dead_code)]` in `transcript_turn.rs`) + Ctrl+O append semantics | collapsed line renders; Ctrl+O keeps appending rather than replacing | Must keep the H-1 message-watermark invariant (a collapsed reasoning block is still a settled row range) |
| **5** | Full-screen overlays enter/leave the alternate screen (BLOCKED, §0 above) | opening help/pickers/rewind/settings/session-picker/diff-viewer in native mode paints into the alt screen; closing leaves it and requests a replay so no duplicated or stale history survives | The unified INLINE picker (05B) must stay inline — already excluded in `full_screen_overlay_open()`; permission/ask-user/command-palette/mcp-approval/context-menu/vibration rows excluded too |
| **6** | Mouse capture off in native mode + help/README text | native-scroll branch never enables mouse capture (already true by construction: only the alt-screen branch calls `EnableMouseCapture` — VERIFY, don't re-add); help + README document Shift+PgUp / tmux copy-mode | The earlier attempted edit was a no-op and was reverted |
| **7** | Live verification under tmux + kitty/ghostty/alacritty + one non-kitty terminal, then flip `terminal_scroll_mode` default to `true` (config.rs + operant.example.toml + README + CHANGELOG) | quality bar: no flicker (every frame + `insert_before` inside the DEC 2026 bracket); `/quit` leaves the full transcript in scrollback; panic restores cursor/line discipline; wide glyphs measured with one width table; tmux copy-mode contains each line exactly once | **Live-only** — TestBackend makes `insert_before` inert; the headless corpus cannot prove this step |

---

## 2. Plan 05B remainder — roadmap item 19/20

**B2.6 — the session picker wire.** The ported full-screen `SessionPicker`
(tui/operant_ui/session_picker/) renders nothing: `inline_interactive_state()`
returns `None`, and the `[port-decision]` block in operant_ui/mod.rs:3387
documents the excised overlay frame (jcode's `session_picker_overlay`
early-returns the whole frame). The ported seam needs the TuiState overlay
methods restored (`operant_app/tui_fns.rs:18-23` explains why they were
excised) or an operant-native frame routed through the existing
`session_browser`/`pending_branch_list` feeder. The adapter work is mapped:
`SessionRecord` (adapter_types/history.rs) → `SessionInfo`
(operant_app/session_picker_types.rs: ~30 fields: id/parent_id/icons/timestamps
/model/status/needs_catchup/messages_preview/search_index/source/resume_target).

**B2.7 remainder — the connect-dialog family.** `/login`→`operant setup
provider` landed (iter-898, owner direction). Remaining: retire the four
dead-end providers in the connect dialog (H-9 below) OR finish the device
flows — **owner call**, only worth it if the owner uses `github-copilot` /
`openai-codex`.

---

## 3. BUGS.md open items (fix in this order — correctness first)

- **H-5** memory feedback channel (`memory_feedback.rs:289`): turn-2 recall
  returns the default prompt instead of the remembered answer. Root is in the
  recall/prefetch seam, not the engine. One iteration; regression test exists.
- **H-6** pause-gate estop (`pause_gate.rs:115`): a model call got through
  while paused. Safety gate — highest priority after the blocker.
- **H-7** review_discipline noop-round off-by-one (5 tests, `[12,13]` vs
  `[11,12]`): one shared root in the autonomous loop's round accounting.
- **H-2** legacy MEMORY.md loader (`main.rs:3282`, loads 0 of 1): delete the
  surface with plan 08's memory-wire-only world (the loader has no callers
  left — verify before deleting).
- **H-3** overlay paint-order disagreement (`operant_overlays.rs:747` +
  `rewind-flow` scenario). The rewind-flow corpus red is the same rename.
  Decide the true paint order, align table + test + scenario, `prove` the
  golden.
- **S1/S2** OSC-8 pinned-image post-draw writes + wide/ambiguous glyph
  widths — live-only (tmux `capture-pane` + F12 `d` dump). Pair with step 7's
  live sweep; one iteration can clear both.
- **W3-2** `/stop` for a QUEUED session is a no-op — small UX decision
  (queue-entry removal in the gateway dispatch loop). Owner call: fix or
  leave documented.
- **H-8** `/fast` lies in two places. TUI half: delete the arm + the picker's
  fast column (`prove` regeneration of the picker goldens). Gateway half
  (`gateway_commands.rs:1571-1595`, write-only metadata) is the peer's
  gateway line — do not grab from the TUI slice; message the peer.

---

## 4. Deferred (plan 11 re-scope) — roadmap item 20/20

Re-read `docs/audit/2026-10-16-operant-audit/11-batch-redirection.md` and
execute the re-scoped batch-06 / batch-07.2 items: batch-06 P0/P1 and the
LIVE verification legs. These are last by design — they depend on the
quality bar (07 steps 4–7) and the default flip.

Peer-owned surfaces (coordinate via `agent://<name>`, never fix from another
slice, fleet rule 7): the gateway `/fast` twin (H-8), the memory-wire lock
advances (chore, precedent iters 866/897/899), and anything in
`REMAINING-GAPS.md` G1 (owner-held).

---

## 5. Verification debt to clear

1. The live tmux sweep for 07 step 7 (four terminals + copy-mode check).
2. The S1/S2 tmux `capture-pane` conviction-or-clear.
3. `scripts/test-compile-gate.sh` must stay green at every push (added
   iter-863) — it is the gate that keeps the ten-dead-test-target class of
   bug from returning.

## 6. Standing conventions

One iteration = one commit = one push, label from `origin/main` after
`git fetch` (+1, hand-computed). Gates per iteration: `cargo fmt --all` →
`scripts/test-compile-gate.sh` → `cargo test -p operant-cli --bin operant` →
`cargo test -p operant-core --lib` (solo any parallel-DB flake) → corpus
(`env -u OPENAI_BASE_URL -u OPENAI_API_KEY OPERANT_BIN=<fresh copy>` per
scenario) → `scripts/clippy-warning-gate.sh` (report baseline parity, never
`--update` to hide new warnings). Code iterations deploy per AGENTS.md §7
(clean-tree release build → provenance gate → both install paths → gateway
restart → md5 verify). Pre-existing reds to cite, not fix: H-2/H-3/H-5/H-6/H-7
and the palette-race flake class (`accent_role`,
`quantized_palette` — solo-green).
