# TUI remaining implementation gaps — 2026-10-13 outline (post-iter-792)

Ground truth as of `origin/main` = `ca7d39b8 fix(iter-792)`. Sources: the
2026-10-13 mandate (final-message duplication / tool-call duplicates /
text selection / exhaustive legacy sweep), live tmux repros on deployed
binaries, event-log captures, corpus + suite gates, and direct contrast
against the vendored jcode source (`../parent-projects/jcode/`).

## The 2026-10-13 glitch list — status

| Glitch | Root cause | State |
|---|---|---|
| Final-message "duplication" | The model's **reasoning draft** rendered as body text: display shim `config()` was a `LazyLock` of hardcoded defaults that never read TOML (mode frozen at `Full`), CLI `show_reasoning` default `Some(false)` overrode the core `true`, and Off/Current had no live behavior | **Closed — iter-792.** Off/Current/Full honored (config + `/thinking-display`), draft renders dim+italic `rgb(100,100,100)` vs body `rgb(200,200,195)` (live-verified), Current = ephemeral per jcode |
| Tool-call duplicates | Same mechanism: the draft contained a tool-results table, then the real answer repeated it. Tool ROWS (`✓ tool` chips) never double-commit (find-by-id dedup, suite-pinned; live read-file turn shows each chip once) | **Closed by iter-792** (draft now visually distinct / suppressible). If a real double-chip repro surfaces, reopen with an event log |
| Text selection | App-mode drag paint: in-band clamp shipped (iter-672) + corpus goldens PASS. Terminal-scroll mode: mouse capture deliberately released — selection is NATIVE terminal select-and-copy over emitted history (the P5 design goal). Boot-dialog Down-swallow documented in the scenario notes (a click that closes a dialog is correct) | **Closed — verified across all three surfaces** |
| Shredded streaming (from 10-12 round) | Live partials in `display_messages` froze growing frames into scrollback | **Closed — iter-784/789/790** |

## Verified-closed surfaces (leave alone)

- Streaming emission (P5-1/2/3), `/cls` divider, viewport floor, watermark.
- Tool/message ordering: live-slot splice (iter-789); `tool-split-stream` golden.
- Command honesty: not-wired catch-all, sweep 106/106 (iter-790).
- Thinking shading: sentinel at storage time (iter-780) + live contract (iter-792).
- Selection: in-band paint clamp + native scroll-mode selection + copy reflow.
- Notification purge: banners dead, single error card (iter-776/777).

## Remaining gaps (exhaustive sweep, jcode-contrast)

### A. Reasoning-mode polish (small, follow-up tier)
1. **`Current` mode lacks jcode's in-place anchor**: jcode slices the live
   trace out of the stream and anchors it as a display-only message that
   stays readable until the next user prompt (then removed). Our Current
   mode simply never persists — the trace vanishes at Done with no
   in-transcript residue. jcode's `TurnReasoningTrace` + GC-at-next-prompt
   is the full contract; port if Current-mode users miss the tail.
2. **`/thinking-display current` summary footer**: jcode renders
   `▸ thought for Xs`; our `turn_elapsed` exists but no collapsed affordance
   renders. Cheap: one dim row at trace collapse.
3. **`Off` mode and the agent-side `show_reasoning`**: core config still
   carries `agent.show_reasoning` (gateway path) — unify so gateway-mode
   reasoning display follows the same mode enum eventually.

### B. jcode surfaces not yet ported (audit-tier)
4. **`reasoning_block_start` byte-offset model** (jcode reasoning_region.rs):
   jcode reasons INTO `streaming_text` with markup at delta time; we keep a
   separate `streaming_thinking` buffer. Equivalent output today; the
   divergence matters only if the interleaved reasoning+text-in-one-buffer
   rendering (reasoning mid-answer) is wanted. jcode handles
   mid-answer-thoughts; ours appends thinking above text per iteration.
   Cost: medium. Value: only for models that interleave.
5. **`get_cached_message_lines` renderer cache**: jcode caches rendered
   lines per (message, width, diff-mode); our `ui_prepare` re-renders
   markdown every frame. Perf tier — measure before porting.
6. **Turn metadata chrome** (`TurnMetadata` in jcode W5): model name,
   agent mode, interrupted flags render in jcode's chrome; ours partially
   (Overview/Runtime boxes). Audit what's missing at the frame level.

### C. Known tracked debts (not new)
7. P4-6 split-timing slow-frame audit — no live defect found; stays parked.
8. BUGS.md D-1 doc-debt row.
9. `/usr/local/bin` freshness relies on `~/.cargo/bin` shadowing — user
   PATH state, not a repo gap.

## Verification state at time of writing

- Suite: `1751 passed / 0 failed` (grant_seats has a known parallel-DB
  flake — passes in isolation; separate track).
- Corpus: `verify PASSED: 0 drift` (incl. thinking-block goldens re-pinned
  under the corrected default).
- Live: rivers/mountains repros on the deployed iter-792 binary — shaded
  thinking, single answer, clean error cards.
