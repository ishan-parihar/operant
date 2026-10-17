# TUI live audit — first-hand repro of the 2026-10-09 bug reports + fix plans

Audit method: the deployed global executable (`operant 0.2.1`, md5
`3263bd9a6f0be7cb4dac6473f89b6d90` — iter-718) driven through tmux
first-hand, every claim reproduced on the live binary, then articulated
against code. Companion to `plan-2026-10-09-remaining-gaps-outline-v2.md`
(peer org-track); this file owns the TUI track.

## Verdict table (complaint → root cause → plan row)

| # | User report | First-hand repro | Root cause (code anchor) | Plan |
|---|---|---|---|---|
| 1 | Legacy notification overlays still present | 💡 tips rotating in chat top ("Ctrl+G to bookmark…", "```mermaid…", "Ctrl+Shift+J/K…"); errors append raw to the status row | tips rotation (`adapter_types/tips.rs`), banner+notification surfaces, 40-entry legacy overlay registry (`render/operant_overlays.rs:218`) | P1-7 |
| 2 | Scroll is single-page inbuilt, not terminal-level; PgUp/PgDn "not working" | PgUp fires (Ctrl+G banner shows) but blanks the transcript when offset > content; jcode has NO alternate screen — native scrollback | inner-viewport scroll: unclamped `scroll_offset` (`app/key_handling.rs:1737-1760`); alt-screen render model (`adapter_types/tui_app.rs:425`) | P1-5 clamp; P2-8 architecture |
| 3 | Tool calls split the streaming mid-render | "I'll read the first 5 lines of AG" / `✓ aft_read · 33 tok` / "ENTS.md and summarize." — sentence split by tool row, leading-space artifact on the continuation | ToolStart flush boundary + `tool_rows_after` insertion mid-wrap (`operant_model/adapter.rs:76+`, `app/agent_events.rs:53-73`) | P0-2 |
| 4 | Selection grabs TUI elements + redundant spacing; want text-editor-like | copy mode works (cursor, `↑19` indicator, keyboard nav) but extraction is raw cells | `render/selection.rs:28-44` extracts every glyph across full row width; empty cells → spaces; no trim, no reflow, no message-row filter | P1-6 |
| 5 | Agent errors during tool testing | `web_extract` → "Tool error: URL safety check failed: IO error: failed to lookup address information: Name or service not known" | fail-closed SSRF guard surfaces raw resolver text (`operant-core/src/security.rs:73-102`); host DNS genuinely failed for that domain | P3-10 |
| 6 | Not seeing native sourcehound | `web_search` → answered "ratatui 0.30.2" natively (69 tok); `web_extract` on docs.rs → "Docs.rs"; 97 tools incl. web_crawl/extract/scrape/fetch/browser/browser_cdp/browser_dialog | integration IS live; user config still prefers retired `igs` (`~/.operant/operant.toml [tools.web] preferred_provider`); model chained raw `http_request` (50k tok → 26% context) because fetch results are untrimmed | P3-10 + P0-4 |

## New defects found during the audit (not in the original six)

- **N-1 (P0-1). Paste-burst swallows Enter — silent message loss.** tmux
  (and any SSH/multiplexer that batches keystrokes) delivers a typed
  string inside the 50 ms poll window; `try_detect_paste_burst`
  (`app/event_coalesce.rs:68-85`) absorbs the trailing Enter as paste
  data → text stays in the composer, nothing submits, no error. Reproduced:
  `tmux send-keys 'msg' Enter` never submits; per-char typing at 80 ms
  submits every time. This is very plausibly the "stuck / message never
  sends" experience. Fix: burst-detect must never absorb a bare
  `KeyCode::Enter` press — chars only; Enter terminates the burst and
  submits per normal paste semantics (or: require ≥2 chars + shrink the
  window). Unit test: seed the event queue with chars+Enter inside one
  poll, assert `EventOutcome::Submit` still fires.
- **N-2. Display-cache staleness on the submit path (idle orb occludes a
  sent message).** `display_messages_slice` is keyed on
  `transcript_version` (`app/tui_state_impl.rs:104-118`); neither the
  live submit block (`adapter_types/tui_app.rs:683-697`) nor
  `submit_user_message` (:722-748) bumps it — the user message is
  invisible until the first agent event arrives, and with a slow/stalled
  first token the decorative idle animation keeps painting
  (`operant_ui/mod.rs:4181-4214`, gated on
  `has_started_conversation` reading that stale cache). Fix: call
  `invalidate_transcript()` (and `on_new_message()`) in both submit
  paths. Unit test: push a user message via the live submit path and
  assert `display_messages()` sees it without any agent event.
- **N-3. Duplicate tool rows.** `✓ web_search · 69 tok` rendered twice
  for one call (live capture). Same surface as P0-2 — diagnose together
  (double `ToolStart`, double `Done` replay, or the adapter's
  `tool_rows_after` inserting both the streamed flush and the block).
- **N-4. Raw web-fetch results charged into context.** `http_request`
  returned 50k tokens of HTML → context 500 → 51k (26%) in one turn.
  `max_tool_result_chars` is a schema-only dead knob (per the peer
  outline v2 audit). Fix belongs with P0-4.

## Fix plans (waves; each its own iteration: gates → commit → push → deploy)

### P0 wave (user-visible correctness, small diffs)

**P0-2/N-3 — split-stream tool rows + duplicate rows.** Reorder so a
mid-sentence `ToolStart` never interleaves a tool row into a wrapped
word: flush the iteration's streamed text as one block, render the tool
rows after the block (per-iteration grouping), dedupe by `tool_call_id`
in `display_messages`. Regression: corpus scenario streaming text →
ToolStart mid-sentence → `contains:` both halves of the sentence on
contiguous lines + tool row after; assert no duplicate tool-id rows.
Anchor: `operant_model/adapter.rs`, `app/agent_events.rs:53-73`.

**P0-4/N-4 — tool-result hygiene.** Wire `max_tool_result_chars` at the
result-ingestion seam (truncate + `[+N bytes truncated]`), and for
HTML-bearing results strip to text before they enter context. Evidence:
26% context burn in one fetch. Anchor: tool result path in
`operant-core/src/agent/` + the dead knob in config.

**P0-1 — paste-burst Enter.** As N-1 above.

### P1 wave

**P1-5 — scroll clamp.** Clamp `scroll_offset` to
`content_height − viewport_height` at the key handler (and treat
at-top as no-op) so PgUp never blanks the transcript. Keep the ladder
(±3 Ctrl, ±10 PgUp/PgDn, ±20 Alt, Ctrl+Shift+J/K ±1 already live).
Unit: short transcript + PageUp → assert no blank render + offset clamped.

**P1-6 — selection semantics.** In `render/selection.rs`: (a) `trim_end`
each extracted row; (b) skip rows that belong to tool blocks/chrome —
select/copy message text only; (c) on copy, reflow wrapped continuation
lines into logical lines. Keep the jcode blend/visual as-is (golden-
pinned). Unit: extraction over a mixed transcript returns trimmed,
reflowed text without tool rows.

**P1-7 — overlay/tips frankenstein.** Sweep the 40-entry overlay
registry (`render/operant_overlays.rs:218`) for dead entries (same
method as the iter-717 renderer deletion); gate the rotating 💡 tips
behind `display.show_tips` (default off); route error/status notices to
the single status row (already mostly true). Corpus re-prove: banner
goldens (`notification-banner`, `help-overlay`, tips surfaces) reshape
intentionally.

### P2 wave (architecture)

**P2-8 — terminal-level scrollback (jcode model).** jcode renders
WITHOUT `EnterAlternateScreen`; past frames live in the terminal's
native scrollback and the terminal's own PgUp/PgDn/wheel scrolls them;
the in-app ladder scrolls only the live viewport. Port decision wave:
survey jcode's frame loop for its exact primary-screen mechanism
(ratatui `Viewport::Inline`-equivalent vs print-and-scroll), then port.
This is multi-iteration: design doc first, corpus re-baseline after
(goldens reshape: alt-screen-specific surfaces), live tmux smoke =
scrollback populated + terminal PgUp works. Only after P1-5 (clamp) so
the two scroll systems compose sanely.

### P3 wave (hygiene)

**P3-10 — config + error-text hygiene.** Migrate
`preferred_provider = "igs"` → `"sourcehound"` in the live user config
(works today via the iter-716 alias; migrate + note). Wrap resolver
failures at the tool-error layer with a human hint ("DNS resolution
failed — check network/VPN; the domain may not resolve from this host")
instead of raw getaddrinfo text; keep fail-closed SSRF semantics.

### Carried ledger (pre-existing, unchanged)

`last_msg_area` field deletion (repoint 4 mouse.rs readers at
`last_selectable_area`), right-click context-menu corpus scenario
(`<right,x,y>` — mouse coverage gap), multibyte selection unit test,
`tui/latex.rs` orphan wire-or-delete, jcode Up/Down history-exhaustion
scroll fallthrough, `/cls` registered-no-arm.

## Non-findings (verified working, no action)

- Native sourcehound: `web_search` and `web_extract` both succeeded live
  on the deployed binary; `web_crawl`/`web_scrape`/`web_fetch`/`browser`/
  `browser_cdp`/`browser_dialog` all registered (97-tool registry);
  `sourcehound` + `cloakctl` binaries present on PATH. The ratatui.dev
  failure was a host-DNS condition (unresolvable machine-wide), not the
  integration.
- The chat TUI end-to-end: submit → stream → reply → tool calls all work
  with normal typing cadence (verified across `operant chat` and
  `operant run --query` initial-query paths).
- Ctrl+T copy mode enters, cursor-navigates, shows the `↑N` scroll
  indicator (its *copy semantics* are the gap — P1-6).
- Auth header post-iter-718: filled circles for the configured session
  (`● openai(key) ● custom`), no unconfigured fallback list.
