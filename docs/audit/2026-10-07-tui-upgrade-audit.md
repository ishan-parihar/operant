# TUI Upgrade Audit & Refactor Plan

**Date**: 2026-10-07
**Baseline**: `origin/main` @ `0d85fcbe` (iter-666 — still **red**: 2× E0308 in `operant-runtime`, inherited from iter-662/663 and untouched by 664–666); deployed binary = iter-661 `e9720182`
**Method**: five investigation tracks — rebrand inventory (full grep census), voice/boot artifact mapping, selection-engine comparison (operant vs jcode source), transcript-ordering root-cause trace, UI-UX contrast vs jcode + parity-plan ledger (`docs/JCODE-VISUAL-PARITY-PLAN.md`)
**Input**: user report of six problem areas; `parent-projects/jcode` @ `0a9dc7805` as behavior reference

## 0. Supersedes / reconciles — prior TUI documents

This audit is the single authority for the TUI upgrade (2026-10-07). Prior docs map as follows:

| Prior document | Status |
|---|---|
| `docs/audit/2026-07-11-tui-fragmentation-audit-and-refactor-plan.md` | **superseded** — fragmentation concerns (tui/mod.rs split) already executed; its open items are subsumed by W5 here |
| `docs/audits/TUI_AUDIT_REPORT.md` | **superseded** — input-box/interaction gaps; remaining live items live in §3 and W5 |
| `docs/audits/BACKEND_TUI_AUDIT.md` | **superseded for ordering** — its message-flow concerns are closed by the W1 arrival-anchor fix; seam survey folded into §3.4 |
| `docs/audits/UX_AUDIT_REPORT.md` | **superseded** — P0/P1 UX findings folded into §3 and the W5 ledger |
| `docs/superpowers/plans/2026-07-11-tui-debugging-and-refactor-plan.md` | **executed & stale** — debugging infrastructure shipped (`cmd_tui_debug`, scenario corpus); nothing open |
| `docs/TUI_AUDIT_AND_REFACTOR_PLAN.md` | **superseded** — pre-jcode-port audit; all findings either landed or re-measured here |
| `docs/PLAN-TUI-OVERHAUL.md` + `-v1.md` | **superseded** — jcode-parity overhaul plans; parity ledger now measured in `JCODE-VISUAL-PARITY-PLAN.md` and sequenced here |
| `docs/TUI-PORT-NEXT-OUTLINE.md` | **superseded** — sequencing only; subsumed by §7 |
| `docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md` | **subsumed as reference** — LOC budgets quoted in §3 |
| `docs/JCODE-VISUAL-PARITY-PLAN.md` | **still authoritative** — for the measured parity ledger (§3 quotes its Tier 1/2 tables); NOT for sequencing or the six fixes |
| `docs/ROADMAP-TUI-FLEET.md` | **partially live** — Waves 3–6 fleet coordination still apply; wave content flows from this audit |


Every finding below carries file:line evidence or is labeled `[CANDIDATE — verify during execution]`.

---

## Executive summary

| # | Area | Verdict | Effort |
|---|---|---|---|
| 1 | jcode→operant rebrand | **mechanical, large**: 3,017 source hits, 7 dirs, 43 identifier forms, 179 user-visible strings, 13 env vars | 2–3 iterations |
| 2 | Fresh-start animation | donut torus is a bit-identical jcode port (`jcode_anim/`, 937 LOC); originality replacement needed | 1–2 iterations |
| 3 | UI-UX bugs | 4 Tier-1 parity omissions, stale-state class, 3 drifting goldens, corpus coverage gaps | 3–5 iterations |
| 4 | Voice-mode overlay | fully mapped, safe to purge (226-LOC notice + wiring + recorder) | 1 iteration |
| 5 | Text selection | operant port is 40% of jcode's engine (530 vs 1,332 LOC); known live bugs | 2–3 iterations |
| 6 | Tool-call/final-message order | **root-caused**: side-registry splice in the W1 adapter, not the core loop | 1–2 iterations |

---

## 1. jcode → operant rebrand (global)

Census (source hits, excluding generated baselines): **3,017**. Generated test artifacts add 111 golden/baseline hits (excluded from rename map — regenerated, not edited).

### (a) Directory renames — 7 dirs, 174 path-level refs

| current → proposed | anchor | refs |
|---|---|---|
| `jcode_anim/` → `operant_anim/` | `tui/mod.rs:61` | 6 public fns |
| `jcode_app/` → `operant_app/` | `tui/mod.rs:69` | ~978 imports; 30+ public items |
| `jcode_markdown/` → `operant_markdown/` | `tui/mod.rs:77` | ~32 |
| `jcode_model/` → `operant_model/` | `tui/mod.rs:82` | ~125 |
| `jcode_render/` → `operant_render/` | `tui/mod.rs:87` | ~43 |
| `jcode_render_core/` → `operant_render_core/` | `tui/mod.rs:95` | ~52 |
| `jcode_ui/` → `operant_ui/` | `tui/mod.rs:103` | ~1,626 — single biggest churn |

### (b) Identifiers — 1,976 hits, 43 distinct forms

Renaming with the dirs: the 7 module paths, `pub mod` declarations, `jcode_dir()` → `operant_dir()` (`jcode_app/storage.rs:13` — this one produces the on-disk `~/.jcode` and `jcode-*.log` names, so it is also a filesystem-paths fix).

### (c) User-visible strings — 179 hits, rebrand-critical

Highest-visibility (quoted verbatim in full inventory at `/tmp/audit/jcode_inventory_report.md`; copy into the execution iteration):
- version banner `"jcode {}"` (header/welcome)
- `"Exit jcode"` confirm dialog
- provider catalog `"Jcode Subscription"`
- `~/.jcode` storage path, `jcode-*.log` naming
- welcome/onboarding strings referencing jcode

**Goldens break here**: user-visible strings appear in scenario baselines (the 111 hits). The rebrand iteration MUST regenerate the corpus (`python3 _run.py` from the main tree), not hand-edit goldens.

### (d) Env vars — 13

`JCODE_HOME`, `JCODE_RUNTIME_PROVIDER`, `JCODE_RUNTIME_RELEASE_SEMVER`, `JCODE_RUNTIME_GIT_TAG`, `JCODE_RUNTIME_GIT_HASH`, `JCODE_RUNTIME_GIT_DATE`, `JCODE_TRACE`, `JCODE_PERF_TIER`, `JCODE_GLYPH_SAFE_MODE`, `JCODE_LOG_JSON`, `JCODE_TEST_SESSION`, `JCODE_SSH_REMOTE`, `JCODE_THEME`.

**Decision needed**: rename env vars outright (breaks any user scripts) or accept `OPERANT_*` with `JCODE_*` read-fallback for one release. Recommend: rename outright, note in CHANGELOG — internal tool, no external consumers known.

### (e) Do NOT rename
- Historical references in `docs/` (the port's provenance: "jcode @ `0a9dc7805`" citations) and commit messages.
- `operant-core` hits (9, outside TUI) — separate, flag-only.

### Execution notes
- Mechanical rename only (sed-verify via `grep -ri jcode` = 0 outside docs/goldens). No behavior change, no golden *content* change beyond regenerated banners.
- `git mv` the 7 dirs, then `ast_edit`/LSP rename for identifiers, then string rewrite pass, then `grep -ri jcode crates/` must return only intentional hits.
- Ordering dependency: do the rebrand **after** the ordering/selection/purge fixes land (waves below), so behavior diffs don't double-touch renamed files.

---

## 2. Fresh-start animation — originality replacement

**Current state** (all verified):
- `tui/jcode_anim/mod.rs` (937 LOC): donut torus, gyroscope, black_hole, orbit_rings samplers. Precomputed LUT tables, **bit-for-bit identical** to jcode's reference (`assert_bit_identical("donut", …)` test at `mod.rs:924`). This is the a1k0n/donut.c torus as shipped in jcode.
- `jcode_ui/ui_animations.rs`: `IDLE_VARIANTS = [donut, orbit_rings]` (gyroscope/black_hole available), drawn at `chunks[9]` — 14 rows below the input area.
- First-run onboarding (`jcode_app/tui_state.rs:531`): "gray telemetry header, **prominent donut**, welcome text, and the login prompt" — the donut is the fresh-start centerpiece.
- Welcome screen also renders the OPERANT wordmark (`banner.rs`, iter-392) + rotating tips (`adapter_types/tips.rs`).

**Originality plan**:
1. Design an operant-original idle animation (same seam: `sample_*` fns, same 14-row slot, same LUT discipline for 60fps without CPU burn). Ideas that keep the "living" quality without copying the torus: rotating hex/gear lattice, signal-wave oscilloscope, particle constellation with gravity, ASCII "orbit" of the Operant mark. User should pick the direction — this is a taste decision.
2. Replace `IDLE_VARIANTS` content and the onboarding centerpiece; delete the jcode donut/gyroscope/black_hole/orbit_rings samplers (or keep one as easter-egg if user wants — ask).
3. New goldens for the onboarding scenario; `jcode_anim/` dir rename folds into the rebrand wave (§1).
4. The LUT bit-identical test goes with the old animation; write an equivalent determinism test (same elapsed → same frame) for the new one.

---

## 3. UI-UX bugs — contrast vs jcode + agentic-core integration

### 3.1 Tier-1 parity omissions (look broken, not plainer) — from the measured ledger

| # | Item | jcode | operant | Status |
|---|---|---|---|---|
| 1 | Composer glyph mode switch, 4 states | `ui_input.rs:415-426` | 1 constant `PROMPT_POINTER` | **missing** |
| 2 | 10-band chrome stack | `ui.rs:3182-3215` | ad-hoc `reserved: u16 = 1` (`render/mod.rs:136`) | **missing** |
| 3 | User bubble bg `(35,40,50)` + rainbow ordinal | `ui_prepare.rs:369-416` | no bubble, no ordinal | **missing** |
| 4 | Code fence `┌─ lang`/`│ `/`└─` | `markdown_render_full.rs:475-499` | table borders only | **missing** |
| 5 | Selection 58% accent blend | `selection_highlight.rs:10-20` | `SELECTION_BLEND = 0.58` | done (iter-566) |

### 3.2 Tier-2 (recognizable but visibly plainer) — verify against `JCODE-VISUAL-PARITY-PLAN.md` §2 during execution: command palette overlay, cursor centering, notice taxonomy, status-line compaction, tool-row 3-state icons; Ctrl+L collapsed transcript confirmed missing (`adapter_types/types.rs:71` — state field exists, implementation does not).

### 3.3 Palette bypass — 84 hardcoded color literals in 17 files
`debug/debug_hub.rs` 16, `stats_dialog/render.rs` 9, `bridge_state.rs` 9, `messages/commands.rs` 8, `messages/tools.rs` 7, `messages/markdown_enhanced.rs` 7, `debug/overlay.rs` 6, `messages/mod.rs` 5, `prompt_input/render.rs` 4, `render/selection.rs` 3, +8 files 1–2 each. `Role::Notice` has **zero readers** — ported color nothing paints. Cheapest real parity win; no jcode source needed.

### 3.4 Integration-seam bug class (stale state)
- One-frame-stale publish pattern existed at `render/mod.rs` (`last_selectable_area` published pre-draw) — **fixed in iter-661**; audit all other publishes for the same class (state set before the draw that consumes it). [CANDIDATE — systematic pass during W5]
- Known drift: `ask-user-dialog`, `bypass-permissions-dialog`, `footer-bar` goldens drift at clean base — peer iters 657–660 changed rendered output without rebaselining. **Coordinate rebaseline with that peer**; do not silently absorb.
- Corpus coverage gaps: no goldens exist for most Tier-2 surfaces (command palette, onboarding, tool-row states) — each W5 fix adds its scenario as it lands.

### 3.5 Known flakes (not TUI-visual bugs, excluded from this plan)
3 suite flakes pass 10/10 isolated (e.g. `accent_role_reaches_the_frame`, `grant_days_override_cannot_defeat_the_standing_rule`) — full-suite load only.

---

## 4. Voice-mode overlay — purge

**Scope = the TUI voice-mode surface only.** Operant-core's Kokoro TTS is a design preference and is NOT touched. Gateway `/voice` commands (platform voice output) are NOT the TUI overlay — out of scope.

Touchpoint map (verified):
| Artifact | Location |
|---|---|
| Notice overlay (the visible artifact) | `tui/voice_mode_notice.rs` (226 LOC) |
| Render call | `adapter_types/tui_app.rs:371` |
| Dialog routing/priority | `app/dialog_routing.rs:116-117`, `app/enums.rs:138` + `:209` (`DialogId::VoiceModeNotice = 320`) |
| Key handling | `app/key_handling.rs:1007-1008` (Esc dismiss) |
| Dialog list entry | `app/commands.rs:58` |
| Recorder machinery | `adapter_types/voice.rs` (`VoiceRecorder` wrapping core voice), `drain_voice_events` (`app/channel_drains.rs`), `/voice` command (`app/commands.rs:764`), `operant tui voice` debug subcommand (`cmd_tui_debug.rs`), tips mention (`adapter_types/tips.rs`) |

Purge = delete notice module + recorder wiring + command + debug subcommand; run dead-code pass (`aft_inspect`/clippy) for orphaned helpers; scenario corpus entries if any reference voice. Config `VoiceConfig` reads in TUI go too; keep core `voice` module untouched.

---

## 5. Text selection — parity with jcode

**Architecture delta**: jcode's selection stack = 1,332 LOC (`app/copy_selection.rs` 681 state machine + `ui/copy_selection.rs` 359 render + `ui/selection_highlight.rs` 84 + `app/helpers/clipboard_helper.rs` 208). Operant's port = 530 LOC (`render/selection.rs` — buffer-scrape row cache + post-render blend + context menu) + `app/mouse.rs` drag/copy-mode handling. **40% of the engine.**

**Correction after source read (was [CANDIDATE] in the first draft)**: jcode DOES copy on mouse release in normal mode (preserving highlight) — `copy_current_selection_preserving_highlight` on Up. Operant's copy-on-release was already jcode-shaped; the real bug was the `auto_copy_enabled` setting defaulting to **false**, so release silently copied nothing.

**Landed in iter-672 (W2)**:
- Selection state moves to scroll-stable content space: `(col, content_line)` anchored via `last_resolved_chat_scroll`, projected per frame by the blend pass — a selection survives viewport scroll (the screen-row model was the actual "very buggy" root: scroll shifted content under a fixed screen coordinate).
- Keyboard copy mode (jcode's full key set): h/l/j/k/arrows, SHIFT-extend, Home/End, PageUp/PageDown, g/G, A select-all-visible, Ctrl+A viewport-context copy (±4 lines), Enter/Y copy.
- Edge autoscroll in NORMAL drags (was copy-mode only).
- Release ALWAYS copies (setting + settings row removed; highlight preserved until next click).
- Two latent tuple-order bugs fixed: `copy_selection_range` read `(col,row)` as `(row,col)`; `copy_selection_status` counted columns as lines.
- Clipboard: `tui/clipboard.rs` 5-mechanism chain retained; jcode's non-blocking 150ms poll noted as a follow-up if the chain ever blocks a headless write (documented in iter-661: scenarios must not end with `<release>`).

**Still open for W2 follow-up** (verify during corpus re-prove): double/triple-click row-cache reach after the projection change; `selection-highlight` golden regeneration (content-space coords change the pinned numbers).

---

## 6. Tool-call / final-message ordering — root cause

**The core is FIFO-correct; the W1 port's adapter splices tool calls back into the stream and gets it wrong.**

Evidence chain (all verified):
- Core emits strictly ordered `User → Assistant(tool calls) → ToolComplete×N → Done`:
  - `operant-core/src/agent/stream.rs:120-149` (ToolStart mid-stream)
  - `agent/run.rs:1293-1294` (assistant appended before tools)
  - `agent/run.rs:1964-1997` (results appended + ToolComplete same iteration)
  - `agent/run.rs:1403-1407` (Done only when `tool_calls.is_empty()`)
  - `agent/events.rs:11-15` (plain FIFO `mpsc::send`)
  - `operant-cli/src/tui/app/channel_drains.rs:284-296` (`while let Ok` drain preserves order)
- **The break**: `tui/jcode_model/adapter.rs:display_messages` (lines 75-140) models tool calls as a side registry (`App::tool_use_blocks`) spliced back by message index, instead of real messages in the stream. Supporting seam:
  - `adapter_types/types.rs:1-6` — `Role` enum has no `Tool` variant
  - `app/messaging.rs:47-55` — `add_message` handles User/Assistant/System only
  - `app/agent_events.rs:121` — `ToolStart` creates the side-registry block
  - `app/agent_events.rs:167-184` — `ToolComplete` updates status/preview, never pushes a message
  - `app/agent_events.rs:112` — `turn_index` written once at ToolStart, never re-stamped

User-visible symptom: the final assistant message and its tool calls/results interleave wrong (tool blocks appear after the message they produced, or the answer renders above its results), and multi-turn sessions drift as indices desync.

**Fix spec**: port jcode's model — tool calls/results are real transcript entries in arrival order (add `Role::Tool` to the enum, push at `ToolStart`/`ToolComplete` like jcode does, keep in-progress live status for streaming UX), delete the side registry and the splice. Preserve the live "running" indicator jcode shows while a tool executes.

---

## 7. Sequenced execution plan

| Wave | Content | Iterations | Dependency |
|---|---|---|---|
| **W1** | Order fix (§6) + voice purge (§4) | 1–2 | none; order fix is the highest-impact UX bug, purge is contained |
| **W2** | Selection parity (§5) | 2–3 | none; extends iter-661 seam + scenario |
| **W3** | Global rebrand (§1) incl. env vars + corpus regeneration | 2–3 | after W1/W2 (avoid double-touch churn); banner strings land in goldens |
| **W4** | Fresh-start animation (§2) | 1–2 | after W3 (builds as `operant_anim`) |
| **W5** | Tier-1/2 parity + palette literals + stale-state sweep + corpus coverage (§3) | 3–5 | overlaps freely; coordinate the 3-drift-golden rebaseline with the owning peer |

**Execution constraints** (fleet):
- origin/main tip iter-663 is **red** (2× E0308 `operant-runtime`, entered with peer's W1.8c/W1.10). Rule 7: do not fix a peer's in-flight subsystem. W1's ordering fix touches `operant-cli` TUI files — no overlap with `operant-runtime` — but every iteration must re-verify base health and prove delta-adds-zero-errors.
- Corpus runs execute from the main tree only (goldens contain absolute checkout paths).
- Full-suite verification: `cargo test -p operant-cli --bin operant --no-default-features` (1726 at last green) + scenario `prove`/`verify` ×3 determinism.
- Deploy after every pushed iteration per R&D protocol (worktree build at tip; tip must be compiling).

---

## 8. Open decisions for the user

1. **Rebrand env vars**: hard rename vs one-release fallback (§1d). Recommend hard rename.
2. **Animation direction** (§2): pick the replacement motif; keep any jcode variant as easter-egg?
3. **Selection copy trigger**: jcode parity likely means explicit copy (keybinding/menu) instead of copy-on-every-release — confirm desired behavior (§5).
4. **Voice purge boundary**: confirm gateway `/voice` (platform-side voice output) stays — it is separate from the TUI overlay (§4).
