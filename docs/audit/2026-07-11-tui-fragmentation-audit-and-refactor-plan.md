# TUI Fragmentation Audit & Refactor Plan

**Date**: 2026-07-11
**Scope**: Full TUI surface of `crates/operant-cli/src/tui/` (~44,500 LOC across 49 modules) + TUI debugging infrastructure + reconciliation against the 2026-07-06 audits (`BACKEND_TUI_AUDIT.md`, `SETUP_TUI_AUDIT.md`) and the 2026-07-11 internal plan (`docs/superpowers/plans/2026-07-11-tui-debugging-and-refactor-plan.md`).
**Method**: 5 parallel read-only audit sweeps (app core, adapter/config, dialogs, render pipeline, debug infra) + build/test/lint ground truth.
**Supersedes**: continues the 2026-07-11 plan (its Tasks 1–3 shipped as iter-219/220/221; Task 4 was reversed by iter-222; Task 5 shipped earlier as iter-208).

---

## 0. Verification Baseline (ground truth, this machine)

| Check | Result |
|---|---|
| `cargo check --workspace` | ✅ clean |
| `cargo test --workspace` | ✅ **1,608 passed, 0 failed, 6 ignored** |
| `cargo clippy --workspace --all-targets --all-features` | ⚠️ **498 warnings** (TODO.md claims "~82 pre-existing" — 6× drift) |
| Build environment | This session runs **on the build machine itself** (`ishanp@CachyOS`). Remote agents use `skills/remote-build-ssh` (cloudflared tunnel). |
| `skills/workspace-lint` | Copied into repo (iter-202) but **inactive** — no `workspace-lint.yaml` at root, so it enforces nothing. |

The TUI is not "broken at the compiler level" — every bug in this report is a runtime/architecture defect that compiles and passes the current (thin) test suite. That is precisely why the debug infrastructure comes first in the plan: the current suite cannot see these bugs.

---

## 1. Executive Summary

Since the 2026-07-06 audits, **iters 114–222 resolved nearly every P0/P1**: the event bridge is deleted, all four fake-data stubs (McpManager, FileHistory, TaskStore, UserQuestionEvent) are gone, the duplicate Config wrapper is deleted, `Done.message`/`Usage` bugs are fixed, and settings.json is demoted to visual prefs. The Message/Role type separation was **deliberately kept and documented** (iter-222, `adapter_types.rs:250-284`) — the code confirms it is the coherent "built-FROM-core" pattern, not lossy parallel types. That decision stands; do not relitigate it.

What remains is a different class of problem — **structural fragmentation**, concentrated in four places:

1. **The debug infrastructure is a skeleton, not a loop.** The headless simulator drives the *real* `App::run` loop (the strongest design decision in the codebase) but can only assert on a hardcoded whitelist of 10 booleans, cannot see rendered screen text (the `TestBackend` buffer is discarded), cannot mock agent events (it spawns a *real network agent*), and the event bus publishes only 4 of its 16 event variants. An autonomous agent cannot yet observe→assert→diagnose the TUI.
2. **`app.rs` is a 7,815-line god-module**: 147-field `App` struct, a 1,553-line `handle_key_event` with ~35 sequential dialog gates, and two hand-maintained parallel lists (`close_secondary_views`: 27 entries, `any_modal_open`: 36 entries) that must be edited in sync for every new dialog.
3. **27 dialogs implement 7 different patterns** with 12 copies of list-navigation, 4 filter implementations, no shared close contract, and keybinding inconsistencies a user feels daily (`j`/`k` works in 6 dialogs, not in 5 others; `q` closes 4, not the rest).
4. **The render pipeline re-renders the entire transcript from scratch every ~50ms frame during streaming** (caches are bypassed exactly when streaming), deep-clones the full line vector every idle frame, and carries ≥6 independent text-wrap implementations — one byte-based (real CJK/emoji bugs) — plus a dead legacy rendering subtree.

Everything above is fixable incrementally with green builds at every step. Estimated total: **~25–35 iterations**, net **negative LOC**.

---

## 2. Reconciliation: 2026-07-06 Findings → Today

| Prior finding | Status today | Evidence |
|---|---|---|
| Bridge (`bridge.rs`) translates & drops events | ✅ FIXED iter-114 | file deleted; `App.agent_event_rx` consumes `AgentEvent` directly (`app.rs:1147`, `6170`) |
| Stub McpManager returns `vec![]`/Disconnected | ✅ FIXED iter-208 | stub deleted; `/mcp` wired to core manager (`adapter_types.rs:2283`) |
| Stub FileHistory / dead turn-diff | ✅ FIXED iter-209 | module + feature cut (`adapter_types.rs:236-244`) |
| `Done.message` dropped; `Usage.total_tokens` dropped | ✅ FIXED iter-210 | `app.rs:6369-6415`, `6443-6463` + regression tests |
| Config wrapper duplicates AppConfig, hardcodes 8192/0.8 | ✅ FIXED iter-215 | `App.config: AppConfig` (`app.rs:859`); settings screen reads real values (`settings_screen.rs:272-274`) |
| settings.json overrides operant.toml provider/model | ✅ FIXED iter-220/221 | provider/model removed from Settings; TOML sole store (`app.rs:1933-1969`) |
| Duplicate Message/Role types | 📌 INTENTIONALLY KEPT iter-222 | documented seam `adapter_types.rs:250-284`; ContentBlocks built FROM AgentEvents — coherent |
| 6 config sources, config.yaml precedence heuristic | ❌ **STILL OPEN** | `config.yaml` still merged on every run with `model == "gpt-4"` heuristic (`main.rs:1139-1166`, `1143-1145`) |
| `IterationComplete` dropped ("iter N" pill dead) | ❌ STILL OPEN (now explicit no-op) | `app.rs:6488-6494` |
| SessionRecord vs DatabaseSession duplication | ⚠️ PARTIALLY OPEN | wired to real DB but `updated_at: Utc::now()`, `messages: vec![]`, `total_cost: 0.0` hardcoded with a **misleading comment** (`adapter_types.rs:873-882`) |

**New residual bugs found by this audit** (not in any prior doc):

| # | Bug | Location | User impact |
|---|---|---|---|
| R1 | `/resume` browser shows fake "just now" timestamps, 0 messages, $0 cost | `adapter_types.rs:879-881` | trust — session list looks broken |
| R2 | Resumed sessions drop reasoning + tool history (`Vec<(String,String)>`) | `adapter_types.rs:892-914` | resumed transcripts lossy |
| R3 | `/stats` cost uses hardcoded $3/$15-per-Mtok; authoritative `AgentEvent::Cost` only debug-logged | `adapter_types.rs:225`, `app.rs:6465-6486` | wrong money numbers |
| R4 | `ask_user_dialog` word-wrap is **byte-based** — CJK/emoji questions wrap wrong | `ask_user_dialog.rs:430` | visible layout corruption |
| R5 | `status_height` counted in `chars()` not display width — wide-char status overflows | `render.rs:407` | clipped status row |
| R6 | "↓ N new messages" box sized by `.len()` (bytes) — arrow overcounts | `render.rs:1152` | oversized indicator |
| R7 | `bypass_permissions_dialog` is **unreachable in production** — `show()` called only from tests; `--dangerously-skip-permissions` wiring absent | `bypass_permissions_dialog.rs:175+`, `app.rs:1511` | dead feature shipping as live code |
| R8 | `settings_screen` list nav clamps while all 12 siblings wrap | `settings_screen.rs:197` | inconsistent feel |
| R9 | ~13 slash commands are stubs that only set a status string (`/reload`, `/replay`, `/queue`, `/steer`, `/background`, `/browser`, `/mouse`, `/billing`, `/update`, …) yet appear in `/help` | `app.rs:2540-2710` | trust — help advertises non-features |
| R10 | 4 byte-identical duplicate slash arms (`output-style`==`verbose`, `theme`==`skin`, `stats`==`cost`, `changes`≈`rollback`) | `app.rs:2152-2688` | maintenance drift risk |
| R11 | Dead fields with doc comments describing behavior that doesn't exist: `queued_messages`, `pending_auto_submit`, `history_search`, write-only `input_history`/`history_index` | `app.rs:875-876, 928, 1105-1108` | misleads every future reader/agent |
| R12 | "bridge badge" hardcoded `Disconnected` (unwired remote-session display) | `render.rs:265, 2608` | dead pixel real estate |
| R13 | Dead dialog code: `ToolPermissionDialog` + `ElicitationField` + `render_tool_permission_dialog` (broken signature) + `render_mcp_approval_dialog_frame` | `dialogs.rs:756-816, 1268` | dead weight |
| R14 | Dead legacy render subtree: `render_message` dispatcher family only reachable from tests; `render_code_block` references a **feature flag that doesn't exist** | `messages/mod.rs:76, 844, 1108, 1321` | dead weight + stale docs |
| R15 | `markdown_enhanced.rs` doc claims italic/strikethrough support that was never implemented | `markdown_enhanced.rs:1` | stale docs |

---

## 3. Findings by Area

### 3.1 Debug infrastructure (the gate for everything else)

**What exists and is right**: `operant tui debug simulate --keys … [--assert …] [--output …]` constructs a real `App` on `TestBackend::new(120,40)` and runs the **real** `App::run` loop (`adapter_types.rs:2562-2668`, `app.rs:6852-6867`) — zero simulator/production drift. The `TuiEventBus` (16 typed variants, ring of 1000, JSON-serializable, atomic-gated) and F12 overlay are cleanly designed (iter-206).

**The 11 gaps** (ranked):

| # | Gap | Severity |
|---|---|---|
| G1 | **Rendered screen invisible**: `run_headless` returns `(Vec<TuiEvent>, App)` and discards the `TestBackend` buffer — no `--assert-screen`, no snapshots | BLOCKER |
| G2 | **Agent events cannot be mocked**: simulator spawns a real network agent (`adapter_types.rs:2578-2594`) — non-deterministic, needs credentials, cannot reproduce streaming/tool/error UI states offline | BLOCKER |
| G3 | **Event bus 75% dead**: only `Key`, `AgentEvent`, `FrameRendered`, `Error` are ever published (3 call sites in app.rs); `OverlayOpened/Closed`, `SlashCommand`, `PermissionRequest`, `UserQuestion`, `Mouse`, `Resize`, `Paste`, `ModelFetch`, `SessionList/Load`, `VoiceEvent` never fire | HIGH |
| G4 | **Assertions = 10 hardcoded booleans** (`cmd_tui_debug.rs:1079-1100`); no strings, numbers, nesting; extending requires recompiling | HIGH |
| G5 | No live App-state JSON dump (`App` not `Serialize`; `dump_on_exit` dumps events only) | HIGH |
| G6 | Permission/user-question/voice dialogs un-drivable headlessly (can't inject their events) | HIGH |
| G7 | `record_error` has one non-test caller; App-internal error paths never publish | MED |
| G8 | No scenario/script format; unknown `<tokens>` silently degrade to literal chars (`cmd_tui_debug.rs:997-1000`) | MED |
| G9 | No timeout/max-frame guard — hung stream = hung simulator (`app.rs:6519`) | MED |
| G10 | Terminal size hardcoded 120×40 — layout bugs at other sizes unreproducible | MED |
| G11 | Entirely undocumented (zero mentions in README/TODO/CHANGELOG) | MED |

**Test coverage today**: exactly **1 of ~27** modal surfaces has end-to-end simulated-key coverage (`help_overlay`, via `test_interactive_multi_step_simulation`). Zero coverage: mouse handler (370 lines), all 7 channel drains in `run()`, streaming multi-iteration flush, and every dialog listed in §3.3.

### 3.2 `app.rs` monolith (7,815 lines)

- **App struct: 147 fields** (`app.rs:857-1259`), of which 36 are per-dialog state structs each with its own `visible` bool, plus a redundant `show_help` shadowing `help_overlay.visible`.
- **`handle_key_event`: ~1,553 lines** (`3411-4963`) — one flat priority chain of ~24 inline `.visible` gates + 11 delegated handlers + a ~40-arm fallback match. `global_search` is checked **twice** (3436 and 4362 — the second is dead).
- **Adding one dialog requires editing ≥5 hand-synced sites**: the field, the key gate, `close_secondary_views` (27 `.close()` calls, `2788-2815`), `any_modal_open` (36 OR-branches, `2817-2854`), and the render dispatch. These two parallel lists are the single largest source of "new dialog breaks old dialog" bugs.
- **Other god-methods**: `intercept_slash_command_with_args_impl` ~647 lines (63 arms/89 commands), `run` ~446, `handle_mouse_event` ~370, `handle_agent_event` ~337, `new` ~272.
- **Verbatim copy-paste**: Ctrl+P/N pair ×4, Home/End/PgUp/PgDn block ×3, streaming-cancel clear block ×2, tool-output truncation ×2, spinner-seed block ×3, voice `spawn_blocking` block ×4.

### 3.3 Dialog fragmentation (27 modals, 7 patterns)

- **Patterns in the wild**: (1) shared `DialogSelect` widget (3 users — the only real abstraction, but its key mapping is still copy-pasted 3× in app.rs); (2) struct + inline match + Frame render (~12 modules); (3) struct + App-method handler + Buffer render (3); (4) module-level handler fn (3); (5) overlays.rs family with inconsistent `open()` conventions; (6) multi-mode enum-driven (5); (7) `Option<T>`-driven (permission dialog only).
- **Duplication**: list-nav wrap `(idx+1) % count` ×12 (settings_screen clamps instead — R8); Esc-close via four different method names (`close`/`dismiss`/`cancel`/`back`); 4 independent filter implementations; **zero** use of `ratatui::widgets::Scrollbar` — 11 hand-rolled offset trackers; `centered_rect` duplicated byte-identically (`overlays.rs:27` vs `dialogs.rs:326`); `compute_modal_layout` exists but is **private**, so every dialog eyeballs its own header/body/footer split.
- **Keybinding inconsistency matrix** (user-facing): `j/k` in 6 of 11 list dialogs; `Ctrl+P/N` only in the DialogSelect trio + model_picker; `q`-to-close in 4; `Home/End`/`PgUp/PgDn` coverage arbitrary; Tab means field-advance in 3 dialogs but pane-switch in journey_view; Backspace means back-to-list in skills_view but delete-char everywhere else.
- **Dead**: R7 (bypass dialog unreachable), R13 (ToolPermissionDialog subtree, dead Frame-variant renderer).

### 3.4 Render pipeline

- **No dirty tracking**: full frame rebuilt ≥20×/s unconditionally (`app.rs:6834`), plus full-buffer OSC8 URL scan per frame (`app.rs:6837`).
- **Streaming worst case**: caches consulted only when `cacheable = !streaming && !has_running_tool_blocks` (`render.rs:1360`) → during streaming, `build_items()` re-renders the **entire transcript incl. syntect on every frame**, and `render_markdown(&app.streaming_text)` re-parses the whole growing buffer each frame (`render.rs:1530`). The comment claiming the completed-cache "is valid even during streaming" (`render.rs:367`) contradicts the code.
- **Idle waste**: cache hit **deep-clones the whole line vector** per frame (`render.rs:1379/1468`); `VirtualList::set_items` re-collects `search_index` strings + clears `height_cache` per frame (`virtual_list.rs:76-78`); row maps + selectable-text cell walk rebuilt per frame; visible diff viewer state **cloned whole** per frame (`render.rs:530`).
- **Z-order = source order** of ~35 sequential `if visible` blocks with comment-enforced priorities and one early-return hack for the error modal (`render.rs:673-690`). Nothing prevents two overlays painting simultaneously.
- **Text handling**: ≥6 wrap/truncate implementations; `markdown.rs::word_wrap` ≈ `dialogs.rs::word_wrap` near-identical; `ask_user_dialog` byte-based (R4); `truncate_middle` char-count-based; no grapheme segmentation anywhere (ZWJ emoji mis-measure in every wrapper). Two full syntect grammar bundles loaded (`markdown.rs:18-22`, `diff_viewer.rs:27-28`).
- **Dead subtree**: R14 (`render_message` family, `render_code_block` + phantom feature flag, never-read `RenderContext.highlight`); `markdown_enhanced.rs` is a table-only helper mislabeled as a renderer (R15).
- **`prompt_input.rs` (5,511 lines)** is seven modules in one file: vim state machine ~1,450 lines, autocomplete/filesystem walk ~330, kill ring ~100, paste ~65, 50-method state impl, render ~456, **tests ~1,613 (29% of file)**. Clean section boundaries already exist — mechanical split.

### 3.5 Config & backend residuals

- **6 config sources persist** (config.yaml, .env, env vars, operant.toml, settings.json, auth.json). config.yaml is still deep-merged on every run behind a `model == "gpt-4"` heuristic (`main.rs:1143-1145`) — the last surviving BLOCKER from SETUP_TUI_AUDIT §A1. One more `"gpt-4"` fallback at `adapter_types.rs:1834`.
- R1/R2/R3 session + cost residuals (§2 table).
- Dead: `DEFAULT_MAX_TOKENS` (`adapter_types.rs:206`), `Settings.max_output_tokens` (`:86`).

### 3.6 Workspace & process hygiene

- Clippy at **498** (claimed ~82). Dead-code warnings cluster exactly where this audit found dead code (adapter_types, gateway_runner, main.rs).
- `workspace-lint` skill present but unconfigured (no `workspace-lint.yaml`).
- Debug tooling undocumented (G11) — an autonomous worker must read source to discover the simulator's key-token vocabulary and assertion whitelist.
- Root carries 4 audit docs (`BACKEND_TUI_AUDIT.md`, `SETUP_TUI_AUDIT.md`, `UX_AUDIT_REPORT.md`, `BUGS.md`); `BUGS.md` (2026-06-19) is now almost entirely stale — most "Critical" items no longer reproduce.

---

## 4. Refactor Plan

**Ordering principle**: build the observation loop first (Phase A), then use it to lock in behavior while collapsing structure (B–D), then config + hygiene (E–F). Every step keeps `cargo check`/`test` green and commits per iteration, per the repo's iteration protocol. Iteration numbers continue from iter-223.

> **For agentic workers:** implement task-by-task with checkboxes; verify each step with `./scripts/check.sh` + the headless simulator; commit `feat|fix|refactor(iter-N): …` and push after each iteration. On remote workers, build via `skills/remote-build-ssh`.

### Phase A — Debug infrastructure: close the loop (iter-223 → ~228) — DO FIRST

- [ ] **A1 (iter-223): Screen-buffer observability.** `run_headless` returns the final `TestBackend` buffer; add `--assert-screen "contains:…"` / `not-contains:` and `--dump-screen <path>` (plain text rows) to `simulate`. This single change makes every later refactor verifiable. (G1)
- [ ] **A2 (iter-224): Mock agent script.** `--agent-script <json>`: a `Vec<AgentEvent>` (serde) injected through the existing `agent_event_rx` channel instead of `create_runtime_agent` — deterministic offline reproduction of streaming/tool/permission/done/error UI states. Also inject `PermissionRequest`/`UserQuestionRequest` via their channels. (G2, G6)
- [ ] **A3 (iter-225): Light up the event bus.** Publish the 12 dead variants at their natural choke points — `OverlayOpened/Closed` inside the (Phase B) dialog registry open/close, `SlashCommand` in `intercept_slash_command_with_args_impl` (one site), `Mouse`/`Resize`/`Paste` in `run()`'s event dispatch, `PermissionRequest`/`UserQuestion`/`ModelFetch`/`SessionList/Load`/`VoiceEvent` at their channel drains. Route App error paths through `record_error`. (G3, G7)
- [ ] **A4 (iter-226): Generic assertions.** Replace the 10-boolean whitelist with a small state-snapshot JSON (`App::debug_snapshot()` — dialog visibilities, `is_streaming`, message/token counts, active model/provider, status line) and dot-path assertions (`--assert 'snapshot.model_picker.visible == true'`, `contains` for strings). Also `--size WxH` and `--max-frames/--timeout-ms` guards; error on unknown `<key>` tokens. (G4, G5, G8, G9, G10)
- [ ] **A5 (iter-227): Scenario regression pack.** `tests/tui_scenarios/` — one keys+assert scenario per modal surface (27 dialogs) + streaming/tool-call/permission flows using A2 mocks. This is the safety net for Phases B–D. Target: every dialog opens, navigates, closes headlessly.
- [ ] **A6 (iter-228): Document.** README section + `docs/` page for `operant tui debug simulate` (key vocabulary, assertion grammar, agent-script format, event-log schema); update TODO.md. (G11)

### Phase B — Dialog unification (iter-229 → ~235)

- [ ] **B1: `Modal` trait + registry.** `trait Modal { fn visible(&self) -> bool; fn close(&mut self); fn handle_key(&mut self, key) -> ModalOutcome; fn render(&self, …); fn z(&self) -> u8; }`. A single ordered registry on `App` replaces: the ~35-gate priority chain in `handle_key_event`, `close_secondary_views`, `any_modal_open`, and render.rs's 35 sequential `if visible` blocks. One loop each. Migrate incrementally — pattern-2 dialogs first (they're already uniform), enum-mode dialogs last.
- [ ] **B2: Shared interaction helpers.** One `ListNav` (wrap, j/k, Ctrl+P/N, Home/End, PgUp/PgDn) replacing the 12 copies; one filter helper replacing the 4; make `compute_modal_layout` public and adopt it; adopt `ratatui::widgets::Scrollbar` or one shared `ensure_visible`; delete `dialogs.rs::centered_rect`. Standardize keys: Esc closes everything, `j/k` + arrows everywhere list-like, `q` closes read-only views only. Fix R8 (settings clamp→wrap).
- [ ] **B3: Kill dead dialogs.** Delete `ToolPermissionDialog`/`ElicitationField`/`render_tool_permission_dialog`/`render_mcp_approval_dialog_frame` (R13). Decide bypass dialog (R7): wire `--dangerously-skip-permissions` to it, or delete it — do not keep shipping unreachable UI. Remove `show_help` shadow bool and the dead second `global_search` gate (`app.rs:4362`).
- [ ] **B4: Slash command table hygiene.** Collapse the 4 duplicate arms (R10) to aliases in one place; either wire or remove the ~13 stub commands so `/help` only advertises working features (R9 — this was UX_AUDIT P1-13); drop stale `help_command_category` mappings.

### Phase C — Render pipeline (iter-236 → ~241)

- [ ] **C1: Fix the streaming re-render.** Use `COMPLETED_MSG_CACHE` during streaming (append live content to cached committed lines instead of full `build_items()`); cache `render_markdown(streaming_text)` keyed on content length. Verify with A1 screen snapshots + `FrameRendered.render_ms` from the bus.
- [ ] **C2: Kill per-frame allocation churn.** `Rc<Vec<RenderedLineItem>>` from caches (no deep clone); persist `VirtualList.search_index`/`height_cache` across frames keyed on `transcript_version`; stop cloning `diff_viewer` per frame; add a `needs_redraw` flag so idle frames skip line construction entirely.
- [ ] **C3: One text_util module.** Single width-aware `word_wrap`/`truncate_{start,middle,end}` used by markdown, dialogs, ask_user, render.rs. Fixes R4/R5/R6 as a side effect of consolidation. Consider `unicode-segmentation` for grapheme correctness.
- [ ] **C4: Dead code + assets.** Delete the `render_message` legacy family, `render_code_block`, `RenderContext.highlight` (R14); rename `markdown_enhanced.rs` → `tables.rs`, fix its doc (R15); share one syntect `SyntaxSet`/`ThemeSet` between markdown and diff_viewer.
- [ ] **C5: Split `prompt_input.rs`** along its existing section boundaries: `vim.rs` (~1,450), `autocomplete.rs` (~330), `kill_ring.rs`, `paste.rs`, `tests.rs` (~1,613). Core widget lands at ~1,000 lines. Pure file moves, zero behavior change — scenario pack proves it.

### Phase D — App decomposition + backend residuals (iter-242 → ~247)

- [ ] **D1: Shrink `handle_key_event`** to: F12 gate → modal-registry loop → focus machine → prompt fallback (~200 lines). Extract the copy-pasted blocks (streaming-cancel, voice spawn_blocking, tool-output truncation, spinner-seed) into helpers.
- [ ] **D2: Delete dead App fields** (R11): `queued_messages`, `pending_auto_submit`, `history_search`, write-only `input_history`/`history_index` — or implement the documented queue behavior if it's still wanted (decide; don't keep lying doc comments).
- [ ] **D3: Fix session residuals** (R1, R2): map `DatabaseSession.updated_at`/message-count/cost through `SessionRecord` honestly; preserve reasoning + tool blocks on `load_session` (return real messages, not `(role, content)` tuples). Fix the misleading comment either way.
- [ ] **D4: Fix cost truth** (R3): surface `AgentEvent::Cost` into `CostTracker` instead of hardcoded per-token rates; wire `IterationComplete` to the "iter N" pill or delete the pill.
- [ ] **D5: Split app.rs** (mechanical, last): `app/state.rs` (struct + new), `app/keys.rs`, `app/slash.rs`, `app/agent_events.rs`, `app/mouse.rs`, `app/run_loop.rs`, `app/tests/`. Target: no file >2,000 lines.

### Phase E — Config consolidation (iter-248 → ~250)

- [ ] **E1: Deprecate `config.yaml`.** Load with a one-time deprecation warning + `operant config migrate` writing equivalent operant.toml; remove the `"gpt-4"` precedence heuristic (`main.rs:1143`) and the stray fallback (`adapter_types.rs:1834`). Target end-state: operant.toml + .env (+ auth.json for credentials), as SETUP_TUI_AUDIT §A1 prescribed.
- [ ] **E2: Purge config dead code**: `DEFAULT_MAX_TOKENS`, `Settings.max_output_tokens`; wire or remove the `Disconnected` bridge badge (R12).

### Phase F — Hygiene & ledger (iter-251+)

- [ ] **F1: Clippy burn-down** to `-D warnings` clean (498 → 0; Phases B–D delete most dead-code offenders for free), then add clippy to the pre-commit verify path so it can't drift again.
- [ ] **F2: Activate workspace-lint**: author `workspace-lint.yaml` (audits → `docs/audit/`, plans → `docs/superpowers/plans/`, no stray root files) and run it in the iteration protocol.
- [ ] **F3: Ledger truth**: archive stale `BUGS.md` (mark superseded by this doc), fold the 4 root audit docs' still-open items here, update TODO.md Implemented/Pending, CHANGELOG on user-facing changes.

### Sequencing at a glance

```
A (debug loop) ──► B (dialogs) ──► D (app decomposition)
       │                └──► C (render) ─┘
       └── A5 scenario pack gates every B/C/D merge
E, F — parallel-safe after A
```

### Effort & impact summary

| Phase | Iterations | Net LOC | Risk | Unblocks |
|---|---|---|---|---|
| A Debug loop | ~6 | +1,500 | Low (additive) | everything |
| B Dialogs | ~7 | **−1,500** | Med (mitigated by A5) | D1 |
| C Render | ~6 | **−1,200** | Med (perf-visible) | smooth streaming |
| D App/backend | ~6 | **−800** + moves | Med | maintainability |
| E Config | ~3 | −300 | Low | setup trust |
| F Hygiene | ~3 | −warnings | Low | CI gate |

---

## 5. What NOT to do

- **Do not merge TUI `Message` into `core::client::Message`** — iter-222's documented seam is correct; the rendering ContentBlocks have no wire-format equivalent.
- **Do not rewrite the simulator** — it drives the real `App::run`; extend it (Phase A), never fork it.
- **Do not big-bang the dialog migration** — the `Modal` trait adopts dialogs one at a time; both dispatch paths coexist during Phase B.
- **Do not delete `input`/`cursor_pos` legacy mirrors until tests migrate** to `prompt_input` directly (they're test-facing).
