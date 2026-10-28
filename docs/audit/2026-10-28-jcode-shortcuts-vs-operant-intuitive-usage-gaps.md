# Audit: jcode shortcut surface vs operant (intuitive-usage gaps)

Status: closed
Date frozen: 2026-10-28
Owner: agent-jcode (operator session, "Sisyphus")
Method: read jcode's authoritative tables (crates/jcode-tui/src/tui/
keybind.rs — every `load_*` family — plus crates/jcode-base/src/config/
default_file.rs, the default_config_file_contents block) and diffed
against operant's live arms (crates/operant-cli/src/tui/app/
key_handling.rs, grep-verified per chord).

## Verdict up front

The core jcode muscle-memory set is ALREADY at parity in operant:
model cycle (Alt+P; jcode: Ctrl+Tab), scroll bookmark (Alt+G; jcode:
Ctrl+G), retry (Alt+R/F5), command palette (Ctrl+K), external editor
(Ctrl+G — jcode puts editor elsewhere), fine scroll ladder (Ctrl+Up/Down
±3, PageUp/Down ±10, Alt+Up/Down ±20; jcode: Ctrl+K/J + Alt+U/D),
input-history search (Ctrl+R, shipped iter-1403), effort cycle
(Shift+Tab), Enter chords (iter-1426). No dead-key class remains in the
composer path — the 2026-10-27 sweep (iter-790 catch-all + iter-1426)
closed that family.

## Gaps found (ranked by intuitive-usage value)

1. **Prompt-jump keys (jcode Ctrl+[ / Ctrl+]) — HIGHEST VALUE.** jcode
   jumps between USER PROMPTS (scroll_prompt_up/down), the "where was
   my last question" navigation. Operant scrolls by rows and approximates
   message boundaries (Alt+Up/Down = ±20 rows) but has no prompt-
   anchored jump. In a long session the operator's mental model is
   prompts, not row counts. operant HAS the anchor data (rewind marks
   record user turns; `App.messages` carries roles) — the binding is the
   missing part, not the plumbing.

2. **Model-cycle chord parity (Ctrl+Tab / Ctrl+Shift+Tab).** jcode's
   model_switch_next/prev defaults are Ctrl+Tab / Ctrl+Shift+Tab;
   operant's is Alt+P only (no prev direction, no Tab family). Alt+P is
   documented in the help overlay, so this is a parity nicety, not a
   defect — but Ctrl+Tab is the chord a jcode user presses first.

3. **Voice input / dictation chords — N/A for now.** jcode binds
   voice_input = ctrl+space and dictation externally. Operant's TTS is
   Kokoro (output), and no voice-input pipeline exists; adding the chord
   would be a dead key, which this repo treats as the worst outcome.
   Skip until a voice-input feature exists.

4. **open_resume (jcode Alt+R) — CONFLICTED, correctly divergent.**
   jcode's Alt+R opens the /resume picker; operant's Alt+R retries the
   failed turn (OMP semantics, deliberately shipped 2026-10-25 after the
   "undesired re-run" report). operant's /resume is a registered
   command; a resume-picker chord would need a different chord (e.g.
   Alt+B is free? — no: Ctrl+B is branching). Deliberate divergence,
   documented here so nobody "fixes" it into a regression.

5. **Speed tier (Alt+Up/Alt+Down in jcode) — CONFLICTED.** operant uses
   Alt+Up/Down for ±20-row transcript scroll (part of its scroll
   ladder). operant has no speed-tier concept (its perf tiers are
   auto-detected, not user-cycled). No port.

6. **Workspace navigation (Alt+H/J/K/L) — N/A.** That is jcode's
   window-manager surface (Niri-style); operant is one window.

## Recommendation

Ship gap 1 (prompt-jump keys) as the next TUI iteration: Ctrl+[ /
Ctrl+] jump to the previous / next user-prompt boundary in the
transcript view, with the status row naming the jump (the same
discoverability discipline as the scroll bookmark). Gap 2 (Ctrl+Tab
model cycle alias) can ride along as a one-arm alias to the existing
`cycle_configured_model`. Gaps 3-6: close with "deliberate divergence /
not applicable" as recorded above.

Blocking note: key_handling.rs is under active peer refactor (their
session_hub iter-B touches key_handling.rs per feed F0130) — the
implementation must wait for their landing, same as the advisor commit.

---

## RESOLUTION — 2026-10-28, iter-1461 (prompt-jump chords SHIPPED)

Gap 1 is **closed and deployed** (`feat(iter-1461)`, binary in
`~/.local/bin/operant`, three-way md5-verified against the running
gateway process). Gap 2 (Ctrl+Tab alias) remains open by design — it is
the lowest-value item in this audit (Alt+P already cycles models) and is
tracked in TODO.md, not here.

### What the implementation had to solve that the audit did not foresee

The audit's recommendation ("Ctrl+[ / Ctrl+]") read as the chord to
bind. It is not, on its own, and this is the correction the audit owes a
reader:

1. **Ctrl+[ is byte 0x1b — the Esc key.** crossterm maps codepoint 27 to
   `KeyCode::Esc`, so under the kitty keyboard protocol (which operant
   pushes at boot since iter-1426) the pressed chord arrives as
   **Esc+CONTROL**, never as Char('['). In legacy/tmux modes it arrives
   as Char('[')+CONTROL, and in the oldest mode it is indistinguishable
   from plain Esc. Ctrl+] is even less portable: in the legacy raw-byte
   path crossterm maps 0x1D to `Char('5')+CONTROL`, which is an entirely
   different chord.

   Therefore the shipped primaries are **Alt+[ / Alt+]** — which are
   also jcode's own defaults (`ScrollKeys.prompt_up/prompt_down`), not
   fallbacks — with Char('[')+CONTROL / Char(']')+CONTROL / Esc+CONTROL
   dispatched as the same chord under the terminals that report them.
   The catalogue advertises the Alt chords so `/keys` and the help
   overlay name only chords that work everywhere.

2. **Three live Esc consumers had to be guarded.** Each took Esc with
   ANY modifiers, so under kitty the jump would have been swallowed by
   whichever ran first: the streaming-cancel arm (`Esc if is_streaming`
   — the jump would have cancelled the running turn), the double-Esc
   rewind arm (would have armed the 500 ms double-tap clock), and the
   transcript-focus arm (would have silently switched focus). All three
   now gate on `KeyModifiers::NONE`; plain Esc keeps every existing
   meaning, asserted in `streaming_esc_still_cancels_but_ctrl_esc_does_not`
   and `transcript_focus_esc_returns_focus_but_ctrl_esc_jumps`.

3. **Position in the match matters.** The chord arms sit above the
   generic text-entry arm, whose CONTROL-only guard lets ALT-modified
   chars through: without the earlier position, Alt+[ would type a
   literal `[` into the composer instead of jumping.

4. **Deliberate divergence from jcode's fallback ladder.** jcode also
   maps Ctrl+K/Ctrl+J to prompt up/down. Both are already taken here
   (the command palette, and the iter-120 newline contract), so the port
   drops those two aliases rather than reassign a live chord. jcode's
   Ctrl+5..9 recency-rank jumps were not ported either: the repurposing
   risk (Ctrl+5..9 are prime editor/picker chords) outweighs the value,
   and the prompt-jump pair plus the status breadcrumb covers the
   navigate-to-a-question job.

### Verification record (per the repo's honesty rules)

- 10 regression tests in `crates/operant-cli/src/tui/app/tests.rs`
  (`prompt_jump_tests`), 7 of them mutation-proved red-then-restored:
  the Alt+[ dispatch arm no-op (6 fail), the Esc+CONTROL alias no-op
  (2 fail), the streaming-Esc guard removed (1 fail), the
  auto-follow-first-press branch removed (5 fail), the resize-anchor
  clear removed (1 fail), the transcript-focus guard removed (1 fail).
- Full bin sweep 1860 passed / 0 failed; clippy gate `seen=0 new=0 stale=0`.
- Live check: `strings` on the running gateway's `/proc/<pid>/exe`
  carries the breadcrumb and the empty-state notice.

### The one thing the audit got wrong, restated

The audit framed this as "prompt-jump keys (jcode Ctrl+[/], HIGHEST
value)". The jcode parity grade holds, but the *chord* it named is the
one portable-unavailable form of the feature. An agent re-implementing
from the audit's recommendation alone would ship Ctrl+[ / Ctrl+], find
that Ctrl+[ is reported as Esc+CONTROL, and — unless it also guarded
the three Esc consumers — introduce a regression that cancels a running
turn whenever the operator tries to scroll. That is why the shipped
chords are Alt+[ / Alt+] with the ctrl forms as aliases, and why the
Esc guards (not the chords) are the load-bearing part of iter-1461.
