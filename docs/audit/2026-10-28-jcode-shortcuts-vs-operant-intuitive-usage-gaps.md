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
