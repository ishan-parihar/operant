# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **iter-712 — P0 delegation governance (the standing rule's first slice):**
  employee-seat delegation now consults the governance layer and nowhere
  else — `DelegationPosture` (forbidden/bounded/independent) + the pure
  `can_delegate` in `org/authority.rs` apply the §2.3.1 tiers to WORK
  (same-dept allowed; cross-dept on a live grant; unregistered target
  fails closed — the registry is the only identity source);
  `SeatPolicy.delegation` column via the `ensure_column` probe (bad
  spelling errors on read, never defaults; pre-712 files migrate in
  place); `SeatAuthority::consult_delegation` companion with an optional
  employee registry (Bounded fails closed without it); `stream.rs`
  consults for every `delegate` call from a governed seat — no posture row
  = ungoverned = today's byte-for-byte.
- **iter-711 — feed class live: channel/group posts reach the org's
  seats (gap 5 phase 2's feed sources):** `ContextClass::Feed` joins the
  four aspect classes — captured at route time, before the admin gate
  (`MessageHandler::record_feed`, no-op default so `handle`'s contract is
  untouched), rendered per-seat under `feed_quota` with
  `feed_seat_map` routing (unmapped chats default to `premiere`).
  Telegram `channel_post` updates now parse (author = channel title) and
  are feed-ONLY: recorded, never turned, never answered into the
  channel. Group messages record as feed AND keep their existing
  turn behavior. Regression pins: channel-post parse, feed capture →
  seat isolation → watermark advance, and the
  capture-before-the-gate routing contract.
- **iter-697 — Wave-4 ordered preflight ladder:** `build_messages` now
  runs explicit ordered rungs when the estimate exceeds the 80% preflight
  threshold — TOC/trim (`fast_trim_tool_results`, its first production
  caller) → deterministic decay → LLM summarize-before-evict
  (`preflight_llm_summarize`: same reactive guards — `should_compress` +
  anti-thrash cooldown — with NO deterministic fallback) → evict — each
  gated on still-over-threshold. Wrap-up rung appends final-call copy when
  the ladder fired, riding the built list only so it cannot accumulate
  across turns. 2 ladder property tests.
- **iter-698 — PromptCacheGuard + `HERMES_TURN_TIMEOUT`:** the frozen-prefix
  invariant becomes checked — `prompt_cache_guard` snapshots the head
  system run when the ladder fires and verifies it byte-identical after
  the evict rung (warn in release, debug_assert in tests), so a future rung
  edit cannot silently tax every prompt-cache hit in a session. The turn
  wall-clock ceiling gains the `HERMES_TURN_TIMEOUT` env override (seconds;
  malformed or non-positive input keeps the 20-minute default), the
  sibling of `HERMES_REQUEST_TIMEOUT` (one call) and `HERMES_MAX_ITERATIONS`
  (iteration count). 3 guard tests.
- **iter-695 — socialization phase 2: session outcomes post to the board:**
  the senior's close-out notice is the only board write outside the CLI,
  gated by the same §2.3.1 consult (identity fail-closed, consult before
  write, refusal = skipped post). `resolve_actor_scope`/`live_grants_for`
  moved to `org/authority.rs` as the canonical consult companions shared
  by the CLI seams and the scheduler-side writer. Phase 3 holds by
  construction: outcomes ride the phase-2 notice + worklog into `org
  synthesize`'s org-bank digest.
- **iter-692 — gap 8: chief-of-staff synthesis (Slice 9) + decision→charter
  amendment (Slice 10):** `operant org synthesize [--window-hours N]
  [--dry-run]` composes the org digest over a window (recent notices,
  proposed/accepted decisions, worklog), retains it to the **org memory
  bank** (bank `org` in the same memory_wire.sqlite), and posts it as a
  chief-of-staff broadcast notice. Amendments: `org decision propose
  --amend-seat <SEAT> --amend-charter <TEXT>` stores the pair
  (both-or-neither); `org decision accept` applies it BEFORE the status
  transition (a failed amendment leaves the decision proposed);
  `EmployeeDb::amend_charter` is the only sanctioned charter write besides
  the cast seeder; `org audit <seat>` shows the charter posture. Pre-692
  decision files migrate via a PRAGMA-probe ALTER.
- **iter-688 (record_output port) — output-side successful-repeat guard:**
  `ToolGuardrailTracker::observe_output` (openhuman parity): identical
  narration+batch signature — captured in run.rs before `tool_calls` moves
  into `execute_tools` — warns at 4 (`OUTPUT_REPEAT_WARN`), arms a
  skip-next-call backstop at 5 (`OUTPUT_REPEAT_SKIP`); failed/exempt batches
  reset the streak; synthetic skip results excluded via
  `stream.rs::observe_iteration_output`. Warn/skip, never halt (iter-682
  deviation). 7 ladder tests.
- **iter-689 — SwitchModel steer variant:** `/model <name>` parses to
  `SteeringCommand::SwitchModel` (strict prefix, case-preserved arg); the
  drain arm retargets via the interior-cell `set_model` with the iteration
  budget refunded, mirroring the fallback chain.
- **iter-690 — notice-board READ side wired:** `operant org notice inbox
  --for-employee <id>` (`--pending-only`, `--limit`, `--json`) and the seat
  prompt's pending-notices block in `bind_seat_run` — ack-is-the-watermark:
  pending `ack_required` notices re-render each seat prompt until acked,
  fail-open if the seat is not a registered employee.
- **iter-691 — startup reaper CLOSE + turn-exit journal columns:** pending
  turn-state rows flip terminal at detection time (close no longer gated on
  channel-notice delivery — kills the "still in-flight after restart"
  loop); `.turn_state` gains `exit_code`/`exit_reason` on every terminal;
  `TurnExitReason` classifier with trinity-#904 kill-marker precedence (a
  signal death is never auth).
- **fix(channels) — telegram `strip_tool_call_tags` underscore alias:** the
  iter-664 sanitize relocation dropped the `<tool_call>` tag form from the open-tag array and close-tag match, leaking raw tool-call JSON
  into user-visible Telegram replies; pre-existing, surfaced when the gate
  grew `-p operant-cli` (feature unification compiles the telegram tests).
- **iter-688 — gap 7: authority predicates enforced + `org budget`/`org
  cast`/`org audit`:** `org notice post` consults `can_post_to` per
  recipient (§2.3.1) — employee senders gated on scope/dept/grants,
  unregistered senders refused fail-closed, `user`/`system` keep the
  operator-root surface; `org decision accept` takes `--as <actor>` and
  consults `can_accept_decision` (§2.3.3) — closing the iter-642 finding
  that ratification was structurally open. New read surfaces: `org cast`
  (nine-seat topology + registry state) and `org audit <seat>` (grants,
  decisions, budget posture + metered window spend). `operant budget`
  folds into `org budget`; the top-level namespace is gone.
- **iter-684/685 — guardrail exemption threading (openhuman
  `is_repeat_call_exempt`):** `AgentConfig.guardrail_exempt_tools` seeds
  the live `ToolGuardrailTracker` in both `OperantAgent` constructors
  (`add_exempt_tools`, the mutable complement to `with_exempt_tools`);
  `FacadeConstruction.guardrail_exempt_tools` threads caller exemptions
  through `build_from_config` — the facade-side per-tool activation beyond
  CLI exclusions. `is_guardrail_exempt()` exposes the state. iter-685
  seeded the new field across 18 test/example/facade fixture literals.
  Config-schema wiring remains a pending row.

- **iter-682 — Wave-2 ladder completion: per-tool failure rungs, hard-reject
  halt, successful-repeat recurrence ledger.** `ToolGuardrailTracker`
  gains the remaining openhuman `no_progress/` semantics: a per-tool
  consecutive-failure ladder (any error class — warn at 8, `Halt` at 12 with
  root-cause `failure_copy`, reset by that tool's next success; S2's
  3-strike timeout mask stays on top); a hard-reject halt at the second
  consecutive `Blocked by security policy` result (shared prefix constant
  so the classifier and stream.rs's blocked arm cannot drift; seat-policy
  denials deliberately excluded — a minted grant can make those succeed);
  and a run-wide `(tool, result-hash)` recurrence ledger that catches
  A, B(reset), A cycles the consecutive-identical-result rung can never
  see, at the same 5/6 warn/arm escalation (deliberate deviation: operant
  warns/skips, never halts a turn for repeated successes).
  `GuardrailDecision::Halt(String)` is a new variant; `observe_guardrail_results`
  becomes async two-phase (lock dropped before await), surfaces the Warn
  verdicts it previously discarded, and on Halt triggers the interrupt
  flag (same machinery as steer request-stop) with the summary as final
  content. 12-test adversarial port of openhuman's `mod_tests.rs` ladder
  semantics lands as `tool_guardrails::ladder_tests`.

- **iter-672 — W2 selection: scroll-stable content-space selection points
  + jcode-parity keyboard copy mode.** `selection_anchor`/`selection_focus`
  become `(col, content_line)` — the scroll-stable rendered-line index — so a
  selection stays glued to its text across viewport scrolls instead of sliding
  off the screen row it was made on (jcode's `CopySelectionPoint` model,
  projected through `last_resolved_chat_scroll` exactly like jcode's
  `copy_point_from_screen`). Ctrl+T copy mode gains the full jcode keyboard
  set: h/l/j/k + arrows (SHIFT extends from the anchor, plain moves collapse
  the selection onto the cursor), Home/End line edges, PageUp/PageDown,
  g/G viewport top/bottom, plain A select-all-visible, Ctrl+A copies the
  cursor ± 4 lines of context (jcode's `COPY_VIEWPORT_CONTEXT_LINES`) and
  exits, Enter/Y copy. Edge autoscroll now drives NORMAL drags too, not just
  copy mode (jcode autoscrolls on any edge drag).

### Fixed

- **iter-672 — selection release semantics and two pre-existing count bugs.**
  A finalized drag selection now ALWAYS copies on release — the
  `auto_copy_enabled` setting (default **false**, which is exactly why
  copy-on-highlight appeared broken) is gone along with its settings-screen
  row: jcode has no opt-out and neither does operant now. The highlight stays
  visible until the next click (already jcode behavior, now documented).
  `copy_selection_range` read the `(col, row)` tuple as `(row, col)` — the
  jcode-adapter range pointed at the wrong cell; and `copy_selection_status`
  counted COLUMNS as "selected lines" — both fixed with the content-space
  semantics.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- **iter-707 — Clean-sweep 2: onboarding welcome-takeover phantom excised.**
  The ported draw carried a constant-false gate for a first-run welcome
  takeover (`onboarding_welcome_active` → early-return that suppresses all
  chat chrome), plus its ~140-line support stratum: the two trait methods,
  `OnboardingWelcomeKind` (4 variants), `LoginImportPrompt`, `ImportSummaryPill`,
  `TelemetryChoice`, `LoginImportRow`, a `#[cfg(any())]` test draw hook, and
  the onboarding margins branch. The upstream onboarding module
  (tui/onboarding.rs) was never vendored and no implementor ever opted in —
  the gate guarded a phantom. `onboarding_preview_mode` and
  `suggestion_prompts` stay (live empty-state seam). Behavior identical:
  constant-false branches removed. Corpus verify 0 drift; suite green.

- **iter-703 — Clean-sweep 1: live credential status fixes the stuck
  "/login to add provider" header.** Root cause of the 2026-10-09 visual-audit
  complaint 2: the jcode persistent header is auth-driven, and operant's
  adapter mapped a `has_credentials` boolean snapshotted once at `App::new`
  (auth-store + ANTHROPIC/OPENAI env only) onto the provider matrix — so any
  session that acquired credentials after boot (omp/custom base URLs,
  mid-session /login) reported NotConfigured forever and the header never
  transitioned. `credentials_live()` now recomputes per read (activation flag
  OR live auth-store OR env); `auth_status()` re-derives the provider from the
  configured model when `active_provider` is unset, and maps every
  OpenAI-compatible profile (custom-openai, omp, free-mode upstreams, groq/
  cerebras catalog) onto `openai_compatible_any`. The header gains a `custom`
  circle row for that slot when configured (unconfigured sessions — and every
  corpus golden — render byte-identically; corpus verify 0 drift). The /model
  gate, Ctrl+A model-picker gate, and `auth_method` telemetry read the live
  predicate. Tests: store-gain refreshes matrix without App recreation,
  env-isolated via AUTH_ENV_LOCK (real on-disk store cleared per test — the
  dev machine carries real credentials).

- **iter-694 — W5 Tier-2: Ctrl+L terminal-style clear (jcode parity).** The
  confirmed Tier-2 gap from JCODE-VISUAL-PARITY-PLAN.md item 17: the
  layout-side collapse was already ported (operant_ui renders a zero-height
  messages chunk when `terminal_clear_collapsed()` holds), but nothing could
  set the state. App now captures the `transcript_version` at Ctrl+L
  (`terminal_clear_version`); the state is live only while that version still
  matches and the same idle conditions jcode derives hold (not scrolled up,
  not streaming, no streaming text) — so any new output, streaming, or
  scroll-up immediately restores the full layout, no reset points to hunt.
  `clear_view_terminal_style()` snaps the scroll to the bottom; the chord
  binds next to the paste handlers. Nothing is deleted (contrast `/clear`).
  Also found while wiring: `/cls` is registered but has no execution arm —
  pre-existing, left for its owner. New corpus scenario `terminal-clear`
  (golden: cleared frame contains none of the transcript text). Label note:
  origin/main carries two iter-691-labeled commits (W5a palette sweep + the
  concurrent reaper line), and iters 692-693 (chief-of-staff gap-8 slice + its docs)
  were taken mid-prove — append-only history, all stay.
- **iter-691 — W5 palette/dead-code sweep, sub-wave 1: the transcript's last
  hardcoded grays route through the palette, and a dead helpers stratum goes.**
  `messages/tools.rs` paints tool-row summaries with `theme::dim_color()`
  (the `Dim` role) instead of a private `TRANSCRIPT_MUTED` const — jcode paints
  the same spans the same way, and the role follows the user's palette config.
  The const itself, `TRANSCRIPT_TEXT`, `TRANSCRIPT_SUBTLE`, `GOAL_ACCENT`,
  `GOAL_BODY`, and the user-prompt truncation trio were read only by
  unreachable code and are deleted. `messages/helpers.rs` keeps its live fold
  machinery and loses seven never-called rendering primitives (user-text
  rendering, indenting, block styling, attachment chips) that rustc had been
  flagging; the four generic result renderers in `tools.rs`
  (`render_file_read_result`, `render_file_op_result`,
  `render_tool_result_success`, `render_tool_result_error`) were unreachable
  (the live tool-block renderer in `render/tools.rs` paints every result the
  transcript shows) and go together with `TOOL_RESULT_MAX_LINES` and their
  test. `render_markdown` has no production caller, so its re-export is now
  `#[cfg(test)]` pending the markdown wire-or-delete sweep. Verbatim-parity
  literals (swarm gallery, overscroll pink, latex marker — jcode-identical) and
  data ladders (heatmap, memory-age tints, diff red/green, usage severity ramp)
  are deliberately kept hard-coded. Full suite 1721/0; corpus verify 0 drift.

- **iter-687 — W4 animation: the original radar-pulse `signal` replaces the
  four vendored samplers.** `sample_donut`, `sample_gyroscope`, `sample_black_hole`
  and `sample_orbit_rings` (with their angle-table LUT machinery and their
  jcode-parity bit-identical tests) are deleted; the idle animation is now
  `sample_signal`, original operant code: a beam sweeps the unit disk leaving a
  rational-falloff afterglow, three staggered rings expand from the center, and
  a bright core pulses at the origin. Determinism is pinned by a
  bit-identical-across-calls test at four sizes × eight elapsed values
  (determinism replaces parity as the guarantee — the code is no longer a
  jcode port); `beam_head_leads_the_trail` and `sweeps_over_time` pin the
  physics. Exactly two transcendentals per subpixel (`sqrt` + `atan2`), no
  LUTs, vs the donut's ~142k `cos`/`sin` per frame. `IDLE_VARIANTS` becomes
  `["signal"]`, the `idle_donut_*` identifiers rename to `idle_animation_*`,
  the `three_rings`/`gyroscope` disabled-name aliases are deleted with their
  samplers, and the corpus pin `OPERANT_DISABLED_ANIMATIONS` follows the new
  name. `shape_char_3x3` and `hsv_to_rgb` stay vendored verbatim (the blit
  path is sampler-agnostic).

- **iter-686 — W3 rebrand: the vendored TUI layer stops saying jcode.** The
  seven vendored module trees rename on disk (`tui/jcode_{anim,app,markdown,
  model,render,render_core,ui}` → `tui/operant_*`), every `jcode_`/`JCODE_`
  identifier and env var with them (`JCODE_HOME` → `OPERANT_HOME`, etc.),
  and the user-visible surface with it: the onboarding header wordmark, the
  `/feedback`, `/subscription`, `/subscribe`, `/log`, `/selfdev` command
  descriptions, the login provider's display name ("Jcode Subscription" →
  "Operant Subscription"), logger stderr prefixes, two subscription-overlay
  lines, and the flicker-notice log hint. The TUI state root moves from
  `~/.jcode` to `~/.operant` (`OPERANT_HOME` override unchanged) and
  `binary_stem()` reports `operant`, so builds/logs/session markers land in
  the canonical operant state dir; existing `~/.jcode` state migrates with a
  one-time `cp -a ~/.jcode/. ~/.operant/` (leaf names are disjoint from the
  gateway's files). `AuthStatus.jcode` renames to `subscription` (serde shape
  changes only in-process). Provenance comments ("ported verbatim from jcode
  @ 0a9dc7805", "jcode parity") and the vendored file headers stay by design;
  machine-facing identifiers with state-compatibility risk (`LoginProvider
  id: "jcode"`, `SessionSource::Jcode`, `LoginProviderAuthStateKey::Jcode`)
  are deliberately kept. Corpus goldens regenerated for the rebrand; three
  goldens that had drifted with a peer's concurrent renderer edits (ask-user-
  dialog, bypass-permissions-dialog, footer-bar) ride along — absorbing peer
  drift is what a full golden regen does.

- **iter-671 — W1 of the TUI upgrade**: tool rows splice at their arrival
  anchor (`after_index` — the message count when the call started, immutable)
  instead of after their turn's last assistant message, so a tool that ran
  between two assistant messages renders between them (the "tool calls and
  final message messed up" bug); transcript turns attach tool blocks via a
  message-index→ordinal map. The legacy voice-mode surface is fully purged
  (~530 LOC): notice overlay + recorder + PTT key handling + `/voice` command
  + `tui voice` subcommand + keybinding catalogue entry (Alt+V deliberately
  uncatalogued — the registry must not advertise what nothing implements).
  Core voice/TTS and gateway `/voice` untouched.

- **iter-669 — Wave-2 guardrail harvest: ping-pong and no-progress rungs
  live on the one engine.** `ToolGuardrailTracker` absorbed the three
  patterns from the runtime's dead `loop_detector.rs` (its only consumer
  was the deleted Loop B engine; thresholds verbatim): **ping-pong** — two
  distinct tools alternating 4 complete cycles warn, 5 skip pre-execution
  (20-name sliding window); **no-progress** — a tool returning the
  identical result 5× warns, 6× arms a next-call skip regardless of
  arguments (the varied-args backstop the identical-args rung could never
  catch), fed by a new post-execution `observe_result` hook in
  `execute_tools` (synthetic guardrail skips excluded — identical by
  construction); and the **exemption port** — `with_exempt_tools`
  bypasses every repeat/loop rung and survives `reset` (the W1.8c-dropped
  `tool_call_dedup_exempt`). Skip/warn copy is pattern-aware and every
  skip variant keeps the `Guardrail: ` prefix so R35 never counts guardrail
  skips. `loop_detector.rs` deleted. Gate re-baseline: 5441 passed /
  0 failed (−15 runtime tests, +13 ported pattern tests).

### Fixed

- **Unblock main: `PathBuf` inline format capture in the cron seat-memory
  prompt** (`operant-core/src/cronjobs/scheduler.rs`): `{path}` →
  `path.display()` — committed in 5464764d, broke every `cargo check/test
  -p operant-core` build.

### Removed

- **iter-664 — the dead channels dispatch orchestrator (audit-r2 plan §2,
  Option A)**: `start_channels` (the only pub entry), the dispatch chain
  (`run_message_dispatch_loop` → `dispatch_worker` →
  `process_channel_message`), and every sibling consumed only by that chain —
  `commands`, `consts`, `dispatch`, `factory`, `health`, `history`,
  `identity`, `memory_ctx`, `media_pipeline`, `prompts`, `routing`,
  `runtime_types`, `sanitize`, `supervision`, `startup`, and the
  orchestrator-level `tests.rs` (~167 tests) — plus the dead
  `orchestrator::deliver_announcement` registry variant and
  `ChannelNotifyObserver`. Audit trail: the shipped CLI gateway never
  called any of it (live paths are `gateway_runner.rs` + `operant-gateway`;
  LTO stripped the cluster from every deployed binary since iter-628), the
  only external `channels::orchestrator::` consumer is `acp_server`, and no
  production caller of `register_delivery_fn` exists. The one live helper
  the cluster exported, `strip_tool_call_tags`, moved verbatim to
  `telegram::helpers`. `acp_server` and the feature-gated `mqtt` listener
  remain. Five-crate gate re-baselined: 5443 passed / 0 failed across 44
  binaries.

### Added

- **Mouse injection for `operant tui debug simulate` (`--mouse`)**: the headless
  simulator can replay mouse events after the key sequence —
  `<action,x,y>` tokens (`left`, `right`, `middle`, `drag`, `up`/`release`,
  `scroll_up`, `scroll_down`; 0-based viewport coordinates). This un-parks the
  `selection-highlight` surface: `apply_selection_highlight` is mouse-written
  and style-only, so no scenario could reach it before. The corpus gains a
  `selection-highlight` scenario (120x40 + 80x24) whose style golden pins the
  58% accent background blend and the 32% white foreground blend. Five
  defects were fixed on the way, two of them live-UI bugs: (1) `render_app`
  published `last_selectable_area` *before* `jcode_ui::draw`, one frame stale
  and a zero-rect on the first frame — the first mouse click after startup was
  silently dropped; (2) a token-advance off-by-one in the `--mouse` parser
  hung any sequence of two or more tokens; (3) the simulation exit check
  ignored the mouse queue, exiting at frame one before any mouse event was
  consumed; (4) no closing draw on the simulation exit path — `terminal.draw`
  runs before the event pump, so the last event's state never reached the
  dumped buffer; (5) `App::debug_snapshot()` now exposes `selection_anchor`,
  `selection_focus`, `selection_text` plus comma-free row fields, so drag state
  is assertable from scenarios.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- **W1.8c — channel dispatch runs on the reconciled facade** (the last
  `run_tool_call_loop` call site): `process_channel_message` now constructs a
  `ReconciledAgent` per message via `FacadeConstruction` (provider route,
  system prompt, `temperature`, the channel's `max_tool_iterations`, the
  Loop-B-parity tool allowlist with non-CLI exclusions and the pacing
  `loop_detection_enabled` switch), pins a per-conversation core session id
  (re-minted on `/new` and on rollback), hydrates the fresh session transcript
  from the orchestrator's sanitized prior turns, and runs the turn through
  `run_facade_turn` with the old loop's scope chain at the call site
  (`Box::pin` keeps the large turn future off the polling stack — a 2MB
  tokio worker overflowed inside `Regex::new` in context-reference
  preprocessing before this). Cross-crate fixes the migration surfaced:
  (1) prompt-mode providers now receive tool results as user-role
  `[Tool results]` text (`operant-api` `Provider::chat` default) — core's
  role-`tool` JSON was invisible to text-only providers and re-triggered
  calls until the loop detector tripped; (2) the facade's session store
  enables `PRAGMA busy_timeout=500` and `BEGIN IMMEDIATE` on its write
  transactions — concurrent facades (channel turns, cron, sub-agents) died
  instantly with `SQLITE_BUSY_SNAPSHOT` ("database is locked"); (3) the
  guardrail duplicate-skip result is success-shaped guidance (`Guardrail: …`,
  no `[` prefix — the anomaly heuristic treated a bracket prefix as failed
  JSON and refunded iterations re-asking the model); (4) channels substitute
  core's empty degenerate grace summaries with a stopped-early notice naming
  the configured iteration budget, and strip raw tool-call/result JSON
  artifact lines from final replies; (5) test facades point at per-context
  temp data dirs instead of the operator's live `~/.operant` (39 contexts
  contended on one SQLite file). Channels suite: 1045/0.

- `org grant give` / `org grant revoke` (F1, ORGANISM-ARCHITECTURE §6): the
  CLI no longer writes the `authority_grants` ledger directly — every mint
  and revocation goes through the `SeatApprover`, the same grantor-scope
  ceiling, TTL shape, and vacant/unmanaged-seat fail-closed the gateway's
  `/grant` and `/revoke` get. `GrantDb::insert`/`GrantDb::revoke` are now
  `pub(crate)`: outside `operant-core` the approver is the only write path,
  enforced by the compiler rather than by convention. The flags keep their
  shape but become assertions about the approver-derived grant —
  `--scope` must equal the derived scope, `--target-dept` (when given) the
  derived pin, and `--expires-at` is honored as a whole-day TTL override
  (mirroring `/grant`'s `--days`); a request the approver would not mint is
  refused before any row is written, with the refusal naming why.
### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- **iter-637 — one canonical required-field guard in the org stores
  (consolidation)**: `org::require_non_blank(value, message)` now owns the
  emptiness rule (`trim().is_empty()`) exactly once. The four copies
  (`authority::require_reason`, `pending_requests::require_non_blank`,
  `decisions_db::{require_mutation_reason, require_text}`) became thin
  wrappers that keep their domain message verbatim, so every error string
  the 268 org tests pin is byte-identical.

- **iter-636 — one glob matcher across every allowlist (consolidation)**:
  the two hand-rolled glob subsets are deleted and both delegate to the
  canonical `context::lcm::glob_match` — the matcher the org seat policy
  already enforced with. Three matchers meant a pattern written with a
  character class (`content.[0-9]`) matched in the org layer and silently
  never matched in the agent command allowlist and the kernel import
  allowlist. One deliberate behavior delta on kernel imports, in the
  fail-closed direction: `skills.*` now requires the literal dot (it no
  longer matches the bare name `skills`); character classes now work as
  written. Also drops `check_genome`'s unused `config` parameter (the
  genome checks read the org DB directly, not the config).

### Added

- **iter-633 — mid-flight seat-budget enforcement (Wave-4 §5)**: a HARD
  token cap now stops a runaway turn AT the iteration boundary instead of
  after it — a turn that spends the whole window in one go is exactly the
  burst the cap exists to prevent. Design notes: the envelope
  (`SeatBudgetEnvelope{cap_tokens, used_at_turn_start}`) rides a
  `tokio::task_local!` set ONLY by the gateway runner inside the per-turn
  task, so it is per-turn by construction — one seat's breach can never
  abort a concurrent seat's turn. The agent-wide `InterruptFlag` is
  explicitly NOT used: it wraps one `Arc<AtomicBool>` shared by every
  session's handle (the `/stop` `turn_inflight` entries all point at it),
  so tripping it on one seat's breach would abort every concurrent turn.
  On breach: a 🛑 meta-notice (surfaced as its own message by the existing
  meta-notice path), a `TurnDiagnostics{BudgetExhausted}` log, and the
  existing grace call for a legible partial. Ungoverned callers (CLI `run`,
  chat, TUI, autonomous) set no envelope and keep the byte-identical legacy
  loop. Token basis only — usd-basis seats stay turn-start-enforced (per-call
  catalog cost inside the loop is a recorded follow-up). Two regression
  tests prove the boundary fires (grace exit + notice) and that the
  ungoverned path is unchanged (normal text response, no notice).

- **iter-632 — Wave-4 metering wire (production bug fix)**: real model
  usage now reaches the budget accumulator. Before this,
  `SessionStore::update_tokens` had ZERO production callers — the
  `gateway_sessions.total_tokens`/`estimated_cost_usd` columns the Wave-4
  rollup sums stayed permanently 0, `employee_window_usage` always
  returned (0, 0.0), the hard cap at turn start could never trip on real
  spend, and `<budget_state>` always claimed the full cap remained.
  The wire: the gateway event receiver folds every `AgentEvent::Usage`
  (tokens) and `AgentEvent::Cost` (models_dev-catalog USD, emitted per
  model call on both streaming and non-streaming paths) into a
  per-thread `TurnUsageMap` — same lifecycle as the S5 `ExitReasonMap` —
  and the turn-end block drains it ONCE into the session accumulator
  after `agent.run()` returns but before the outcome arms, so error and
  early-stop turns are metered too and no entry leaks into the next turn.
  Fail-open on telemetry (genome invariant): a metering write never fails
  a turn. Regression test: `wave4_turn_usage_drains_into_the_budget_accumulator`
  proves the drain makes real spend visible to `employee_window_usage`.

- Wave 5 — onboarding governance (ORGANISM-ARCHITECTURE §4):
  `operant cron create` is now the onboarding TRANSACTION — new flags
  `--seat-mode <yolo|standard|scoped|lockdown>`, `--allow <glob>`
  (repeatable), `--deny <glob>` (repeatable). The automaton's employee row
  is provisioned via the registry's own backfill and its policy row is
  seated in the same step (mode defaults to `[genome].unrestricted_default`
  — the documented posture — unless named). Cross-file sqlite cannot be
  one transaction, so the honest shape is job-first-then-provision with a
  DELETE rollback: the operator gets a governed automaton or no automaton,
  never an ungoverned one left behind by a failed register.
  The unified doctor (F2) grows a `genome` section — the owner's no-new-
  commands flag surface: registry census; UNGOVERNED CRON AUTOMATON count
  (the ratified D-2 default made visible, with the exact seating
  commands); budget-window typos named per seat (the fail-open value the
  resolver silently absorbs); pre-Wave-2 session rows pending their
  premiere backfill; cast-not-seeded guidance. Both `operant doctor` and
  GET /api/doctor see it (drift pin extended to five categories).

- Wave 4 — budgets as policy (ORGANISM-ARCHITECTURE §5):
  `[genome].budget` sets the org-wide default (basis tokens|usd, window
  daily|weekly|monthly UTC, cap, mode hard|soft); per-seat overrides live
  in the new `seat_budgets` table with per-field precedence — a row with
  `cap = 0` is the deliberate release valve ("this seat runs free even
  though the org caps"). Ungoverned when no row and cap 0 (the shipped
  default): nothing computed, nothing injected, byte-identical legacy.
  Metering READS the existing per-session accumulator
  (`employee_window_usage`: SUM(total_tokens)/SUM(estimated_cost_usd) over
  the employee's sessions in the window) — no second counter. Every turn
  injects a `<budget_state>` block with the remainder so the agent
  self-economizes before the cap; HARD mode refuses turns that start over
  the cap with a clear notice naming the seat, usage, cap, and the
  escalation path; SOFT warns and continues. Enforcement is turn-boundary
  v1 — mid-flight enforcement is a recorded follow-up, not this contract.
  Typo'd window values fail OPEN to daily (turns keep running; doctor can
  flag the value) — a silent refusal would be worse.

- Wave 3 — rolling compaction handoff (ORGANISM-ARCHITECTURE §3): every
  completed gateway turn upserts ONE rolling summary row per
  (employee, session) — new roll = previous roll + the turn's doings,
  never an aggregate; head-bounded at 8 KB (tail-kept, UTF-8-boundary
  safe). A continuation (restart / session switch / /resume — a real
  transcript reload, not a live conversation) begins from the roll inside
  a `<continuation_summary>` block; TTL is read-time against
  `[genome].session_summary_ttl_minutes` (owner range 15–30, default 30),
  so an expired roll means a fresh start and config changes apply without
  migration. Deterministic v1 roll (prior roll + final turn text) — no
  extra LLM call; LLM-compressed rolls remain a follow-up knob. Summary
  writes fail soft (warn + continue): a failed write degrades to raw
  transcript continuation, never a lost turn.
  F1 follow-through: the authority-tools battery moved from
  `tests/org_authority_tools.rs` to in-crate
  `src/tools/authority_tools_tests.rs` under cfg(test) — its 54 tests
  exercise `issue_grant` at hand-built grants and precise timestamps,
  which only the sealed `pub(crate)` seam reaches; integration-test crates
  are external and must mint through the approver.

- Wave 2 — sessions bind employees (ORGANISM-ARCHITECTURE §2):
  `gateway_sessions` gains `employee_id` (additive migration; legacy rows
  backfill to `premiere` on load — owner ruling: the default conversation
  talks to the premiere). The genome consult resolves the SEAT through the
  bound employee before the session id, so chat sessions run under their
  employee's policy/grants instead of an ungoverned `gw_<hash>` seat; cron
  runs are unchanged (their session id already IS the employee id). The
  employee's charter (org system prompt) rides the frozen prefix inside an
  explicit `<employee_charter>` block — appended after the base prompt,
  byte-stable while the binding holds, so the prompt-cache discipline is
  preserved. New `/session [employee]` gateway command (alias `/use`):
  no args reports the bound employee; with an id it validates against the
  registry (unknown ids list the known cast) and rebinds at the next turn
  start — binding is a conversation act, not an authority act; every tool
  consult still gates under the bound employee's genome row at run time.
  `Gateway::get_persistent_sessions()` exposes the persistent store the
  binding lives on (survives restarts); `SessionStore::entry_for_source`
  gives commands the entry under its real key (group/thread shapes
  included).

- The cold-start cast (ORGANISM-ARCHITECTURE §1, wave 1): `org/cast.rs`
  declares the organism's nine manifest seats — `premiere` (executive,
  org-lead), `chief-of-staff` (executive, reports to premiere — the
  same-department report that makes `premiere` compute as org-lead),
  `governor` (meta-governance, reports to premiere), `identity-warden`
  (identity, reports to governor), `compass` (strategy, reports to
  governor), `crew-chief` (crew, reports to governor), `hrmaster` and
  `dispatcher` (crew, report to crew-chief), and `dp-the-program`
  (execution, reports to premiere). The gateway's org-store init seeds
  them idempotently on first run: employee rows via INSERT-OR-IGNORE,
  reporting edges only for seats with no edge, and premiere's standing
  `Org`-scope grant minted once through `SeatApprover::mint_for` (the
  approver seam — no direct `authority_grants` writes, even at seed
  time). A topology that fails to compute premiere as org-lead aborts
  the seed loudly instead of minting. The seeder writes no
  `seat_policies` rows: seeded seats stay ungoverned until an operator
  policies them. Default cron specs are recorded in the manifest; the
  cron store has no register-without-executing path, so actual
  registration belongs to the execution wave.
- `employees.system_prompt` — the charter column (a session executing as
  an employee runs under its charter). Added via the org layer's
  additive `ensure_columns` reconciliation, so existing databases gain
  it on next open; cron-backfilled rows keep `NULL`.


### Removed

- `operant-runtime::doctor` (1,347 lines) — the duplicate engine. Only the
  gateway's `GET /api/doctor` consumed it; its uncalled `run`/`run_models`/
  `run_traces` entry points went with it. Its checks live on in
  `operant-core::doctor`, adapted to `AppConfig` where possible. Named
  gaps (fields only `schema::Config` carries, or state nothing writes —
  porting would be faking it): channels-configured, delegate-agent
  provider validity, `gateway.port`, `memory.embedding_model` hint
  targets, config-file presence (`config_path`), the runtime
  `workspace_dir` and its SOUL.md/AGENTS.md presence checks (the AppConfig
  world's data root is `operant_home()`; the workspace concept did not
  carry over), and daemon heartbeat freshness (`daemon_state.json` is
  written only by `operant-runtime::daemon::run`, which has no callers).
- `[genome].queued_cron_jobs_resolve_grants` — declared in wave 2 but the
  scheduler sweep it described never gained a consumer (iter-600). Pending
  escalations resolve by a human verdict or the 60s interactive lapse only.
  Breaking for configs that set it (the key shipped commented; if you
  uncommented it, delete the line): `[genome]` keeps `deny_unknown_fields`
  because a silence-tolerant typo on a security knob failing open to the
  `yolo` seating default is the worse failure mode.

The permission genome — per-employee authority for the organism (iters
573–586, wave fleet execution). A seat's policy is data (`seat_policies`:
mode yolo/standard/scoped/lockdown + allow/deny globs), the precedence is a
pure function (`decide()`: blocklist absolute, seat-deny absolute, allow,
yolo, standing grant, then mode defaults), and the run path consults it
behind an Option so ungoverned seats are byte-identical to before.
Escalations persist to a durable `pending_requests` queue; unattended
(cron) runs enqueue and deny the same run, and a senior's approval mints a
TTL'd grant via the existing grant ledger so the next tick runs without
re-asking. Reporting lines are data (`hierarchy_edges`). The scheduled-run
write barrier is live: every completed cron run leaves exactly one
attributable worklog row, and a barrier write failure fails the run.
`[genome]` config block lands with `grant_ttl_days = 7` default.
Also in this wave: BUGS.md D-5 (two MemoryManagers over one MEMORY.md)
fixed by hoisting one `(MemoryManager, provider)` pair for both agents;
D-2 and D-3 resolved (see BUGS.md); K-2 filed (pre-existing order-dependent
`loop_request_timeout` pair).

### Added

- `operant-core/src/doctor.rs` — ONE doctor engine on `AppConfig`
  (docs/ORGANISM-ARCHITECTURE.md §6 F2). Two divergent engines existed:
  `operant-runtime::doctor` ran on `schema::Config` for the gateway's
  `GET /api/doctor`, `operant doctor` ran its own checks on `AppConfig` —
  same intent, drifting check lists. The surviving engine runs the union:
  config semantics on the `[providers]` section (default provider, API key,
  model, temperature range, fallback chain, model/embedding routes), data
  root (exists/writable/disk) at `operant_home()`, environment (git, shell,
  HOME, curl), and CLI tool discovery (`operant_tools`). A drift-pin test
  pins the category order so the CLI and gateway paths cannot silently
  diverge again.
- `org/seat_policy.rs` + `org/seat_policy_db.rs` — seat modes, the
  precedence engine, the sqlite store, and the `SeatPolicySource` seam.
- `org/hierarchy_edges.rs` — reporting lines as an edges table feeding
  `Hierarchy::new`; cycle/dangling defects are reported, not repaired.
- `org/pending_requests.rs` — durable escalation queue with atomic
  pending→resolved transitions and an `expired_as_of` TTL sweep.
- `org/seat_authority.rs` — `SeatAuthority` (policy + grants + requests
  in one consult) and `SeatApprover` (approve→`issue_grant` with TTL tiers,
  deny, expire; `approver_of` routing with operator-literal fail-closed).
- Agent builders: `with_seat_authority`, `with_unattended`.
- `[genome]` config block: `unrestricted_default`, `grant_ttl_days`,
  `queued_cron_jobs_resolve_grants`.
- `tests/governance_escalation.rs` (4 end-to-end),
  `tests/seat_policy_run_path.rs` (5), 6+8+8 unit tests across the new
  stores, 2 new `cron_session_isolation` barrier tests.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- `operant doctor` renders the shared engine's Diagnostics section after
  its install-level checks; engine errors count as issues (they gate the
  exit code), warnings are advisories, and `--json` gains a `diagnostics`
  array — the same list `GET /api/doctor` serves through a thin boundary
  adapter (`schema::Config.providers` → `AppConfig.providers`).
- `GET /api/doctor` now serves the shared engine (doc comment corrected:
  the route was always `get(...)`, not POST).

- `run_agent_job` mounts `WriteBarrier::apply` (opt-in via
  `with_write_barrier`; the gateway constructs it over the app db, so it is
  live in production) — one worklog row per completed run, failure
  propagates to the run's status.
- `start_gateway` builds the memory pair once and threads it to both
  agents (D-5).
- The permission dispatcher: policy-escalated requests deny under YOLO and
  no-active-channel (ungoverned requests keep auto-`AllowSession`);
  60s timeout records `Expired`.
- `/approve` mints first, then answers; `/deny` resolves the queue row.

### Removed

- **W1.10 — Loop B engine deleted**: `run_tool_call_loop`
  (`loop_/tool_loop.rs`), the `agent_turn` legacy wrapper (`loop_/turn.rs`),
  and their 35 unit tests, fixtures, and the five `*_loop_c`
  direct-drive parity tests go away with zero remaining production callers
  (channel dispatch has run on the reconciled facade since iter-662;
  `loop_detector.rs` was already harvested into `core::tool_guardrails` in
  Wave 1). `agent_parity` keeps the core↔Loop C pairs (10 tests).
  `ChannelRuntimeContext` drops the fields only the engine consumed —
  `multimodal`, `tool_call_dedup_exempt`, `activated_tools` (the
  context copy; the tool-registry's own `ActivatedToolSet` handle stays),
  and `max_tool_result_chars` — from the struct, startup wiring, and all 39
  test contexts. Re-baseline: runtime lib 1654/0, parity 10/10; five-crate
  gate 5610 passed / 0 failed across 44 binaries.

## [0.2.1] - 2026-09-30

Foundation for post-turn features (reflection / advisor / dreaming).
A `TurnEnd` event (turn id, iterations, tool counts/durations, capped result
summary) is emitted once per completed turn at the turn-end chokepoint onto a
`TurnEndBus` broadcast seam; with no subscribers attached the emit site costs
a field read. Deterministic tool-loop anomaly detectors (empty result,
malformed output, error signal) re-enter the loop through the existing
continuation shape, capped at 3 retries per turn; the dead `evaluate_response`
self-critique now executes on the response path with its score exposed.
Batches with >1 file mutation or any approval-gated call run sequentially
instead of sharing the 8-worker pool; all provider stream tasks are bound to
their stream handle (`AbortOnDrop`) so early drops stop the task.

Parity work against the `jcode` reference agent (iters 347-409). The TUI
gained roughly 3,000 lines; 1,146 lines of long-dead code were deleted.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- **Memory backend is now memory-wire (in-process).** `MemoryWireProvider`
  calls the `memory_wire` crate's sync `retain`/`recall` API directly with a
  `catch_unwind` boundary (a panic degrades to a memory miss, not a dead
  turn). `provider = "memory-wire"` is the default; `"agentmemory"` still
  resolves to it. Upstream tracks branch `main`; advance with
  `scripts/sync-vendors.sh`, pin `rev =` at release.
- **Web + browser backends are now sourcehound (MCP subprocess).**
  `web_search`/`web_scrape`/`web_extract`/`web_crawl` and the `sourcehound`
  browser provider (`cloakctl.navigate`/`read`/`act`, raw CDP via
  `cloakctl.cdp`) run over the `sourcehound` binary's stdio MCP server.
  `browser.provider` and `preferred_provider` default to `"sourcehound"`.
  In-process was measured unviable (their `[patch.crates-io]` evaporates for
  consumers; unconditional render stack).

### Removed

- **agentmemory integration deleted.** `AgentMemoryProvider`, the `:3111`
  `npx` auto-spawn, and the deferred agentmemory MCP registration are gone.
  `memory.agentmemory_*` config keys no longer exist; old configs carrying
  them fail parse under `deny_unknown_fields`.
- **IGS + Obscura implementations deleted.** `tools/igs.rs`, the IGS binary
  surface, `ObscuraProvider`, and `obscura_cdp.rs` (renamed to
  `sourcehound_cdp.rs`, re-homed onto `cloakctl.cdp`) are gone. `tools.*`
  and `browser.provider` values `igs`/`obscura` no longer exist; the retired
  names fall back to lightpanda (browser) and sourcehound (search).

### Fixed

- **`/steer` was a guaranteed no-op.** Submitting it required `Enter` while not
  streaming, but the queue path rejected exactly that state — the two halves
  were mutually exclusive, so the command had never worked despite being
  documented. The core loop's steering was fully implemented and unreachable.
- **A compaction cut could orphan a `tool_use`/`tool_result` pair.** Eviction
  keeps a head and a recency tail, so a cut could land between a request and its
  answer; providers reject either half alone, and an unanswered request leaves
  the model waiting so it reissues the call and burns a turn. A shape-independent
  repair now runs at the single chokepoint both eviction paths pass through.
- **`/keys` advertised two chords that never worked.** `Ctrl+H`/`Ctrl+L`
  ("previous/next history") had no dispatch arm anywhere, and cannot be bound
  anyway — `Ctrl+h` is ASCII 8 (backspace) and `Ctrl+l` is readline's
  clear-screen. The claims were removed and both chords added to the terminal
  conflict list that `/hotkeys` uses to explain unavailability.
- **`operant-core`'s `simple_agent` example did not compile** (a config field
  added in iter-357 was missing from its struct literal). Note that
  `cargo check --workspace` does NOT catch this — only `cargo test --workspace`
  builds example targets.
- **`operant chat` never exited on a non-TTY stdin.** `read_line` reports a byte
  count and `0` means EOF, but the loop discarded it, so EOF was indistinguishable
  from a blank line and the `continue` spun forever — 33 MB of `You: ` in 20 s
  with stdin on `/dev/null`. `operant run --query` handled the same condition
  correctly, so the fallback existed and was simply not wired to `chat`/`tui`.
- **`operant <cmd> | head` aborted with a core dump.** Rust's runtime sets
  `SIGPIPE` to `SIG_IGN` before `main`, so the first write to a closed pipe
  returns `EPIPE`, which `println!` treats as a broken invariant and panics on;
  the release profile's `panic = "abort"` then turns that into SIGABRT plus a
  core file. Restoring the default disposition makes a closed pipe behave like
  every other Unix tool (exit 141, silent), matching `yes | head -1`.
- **`embedded-web` embedded nothing, on any machine, ever.** The build script and
  the `include_dir!` both pointed at `<repo-root>/web/dist`, a path that has never
  existed in this layout; the Vite project writes to `crates/operant-cli/src/dashboard`.
  Because the build script only sets its cfg when the dist exists, the `include_dir!`
  was never even compiled, so the feature silently degraded to the filesystem
  fallback while passing `--all-features`. A feature that reads as wired and
  reaches nothing.
- **`/terminal-setup` was a stub that hid the problem.** It printed "No manual
  setup needed" while a real interoperability gap sat behind it: without
  kitty-protocol parsing, most terminals cannot send a distinguishable Shift+Enter.
  The command now reports the terminal's actual capabilities.
- **vim `j` and `k` were advertised and dead.** `/keys` listed `VimMotionDown`/
  `VimMotionUp` for the two most fundamental vertical motions; `vim_normal` had
  no arm for either, the `apply_vim_command` referenced in a comment did not
  exist, and every `motion_*` helper in the file was horizontal.
- **Sixel was claimed from a multiplexer's `$TERM`.** Detection inferred
  graphics capability from tmux's own terminfo rather than the outer terminal, so
  it was wrong in both directions — the common `$TERM` never matched, and
  `tmux-256color` claimed Sixel unconditionally. Pinned graphics re-emit every
  frame, so a wrong claim turned a one-shot glitch into a permanently mangled
  image.
- **`operant doctor` always exited 0**, printing "Found 4 issue(s) to address" and
  then reporting success, so `operant doctor && operant chat` walked straight
  into the failure doctor had just described. A diagnostic that cannot fail is a
  diagnostic nobody can gate on.
- **The default theme's accent leaked past the palette in 14 places.** Banner,
  prompt input, spinner, model-picker highlight and four dialogs hardcoded
  `Rgb(255,191,0)` — the *default* theme's accent — so all 7 other themes showed
  them amber.
- **Four `/keys` entries named the right chord with the wrong action.** The chord
  was dispatched, just not as advertised: Ctrl+A was catalogued "move to start" but
  opened the model picker, Ctrl+B "word left" opened the session branch browser,
  and Ctrl+P was wrong twice. A wrong-action key is worse than a dead one — a dead
  key is visibly dead.
- **Three advertised prompt bindings did nothing** (Ctrl+E, Ctrl+F, Ctrl+N).
  `/keys` listed them, the dispatcher had no arms.
- **A test double shipped in the production library.** `VirtualClock` was `pub`
  with no `#[cfg(test)]`, dragging `Mutex::lock().expect()` in with it.
- **The build scripts only worked on one machine.** `dev-env.sh` and
  `provision-build-deps.sh` hardcoded `/home/z/my-project/local`, so on any other
  host every export resolved to a nonexistent directory. Both now derive every
  path from one `LOCAL_DIR` defaulting to the repo's own `local/` — which is where
  `operant-core/build.rs` already searched. `install.sh` now preflights and names
  the missing package instead of failing 200 lines into a linker error.
- **A dangling symlink was committed.** `provision-build-deps.sh` created
  `local/lib/libsonic.so` with an unconditional `ln -sf`, which succeeds even when
  the target is absent; the resulting dead link was then committed, past
  `.gitignore`. It was never load-bearing — the build resolves from the committed
  `libsonic.a` alone.
- **The first error a new user saw named no remedy.** `operant run --query` on a
  fresh machine printed an internal-sounding pool error with no hint that
  `operant setup` was the fix. `operant doctor` got this right; the path a new user
  actually takes did not.
- **`operant doctor` advised a command that cannot work.** It probed a
  `tinker-atropos` submodule that exists in neither the repository nor
  `.gitmodules`, and recommended `git submodule update --init --recursive`.
- **A turn could never time out.** `reset_on_success()` was called from the `Ok`
  arm of the response match — once per agent *iteration*, not once per turn — so
  a turn alternating "stream dropped" with "answered fine" had its retry budget
  refilled between failures and never exhausted it. A flaky upstream held a
  gateway turn open for 9 minutes against a 120s per-request ceiling that bounds
  a single call, not a turn. A monotonic per-turn `turn_retry_failures` counter
  (cap 12, deliberately not the per-call `max_retries` default of 3) and a
  20-minute wall-clock limit now terminate such a turn; the per-call budget
  still refills per iteration.
- **Model output that can name credentials was logged.** The tool-call parser
  logged 80-100 chars of model-emitted text verbatim on both the parse-failure
  and repair paths, and that text is routinely shell fragments — so shell
  fragments, which name credentials, reached the server log. Worse, the
  parser's `ParserEvent::Error` was injected into the *conversation*, handing the
  raw fragment back to the model and persisting it in session history. The
  parser now logs `content_len` plus a preview cut before the first `$`, `=`, or
  quote; the parser error is a fixed re-issue sentence with no model output; and
  the safety layer logs lengths only.
- **A turn lost to a restart was detected and then silently discarded.**
  `check_interrupted_turns()` logged a WARN naming the channel and returned
  unit, so the caller threw the `(channel_id, timestamp)` pairs away: a run that
  started, died 5.2s later, and rebooted showed the user 18 minutes of silence
  with no notice. The boot path now delivers the notice through
  `send_channel_message`, and the state file records the platform so a
  non-Telegram turn is not announced on Telegram (pre-existing files without the
  field fall back to telegram, the only adapter that recorded turn state).
- **Channel feature flags selected nothing.** The gateway's dependency line
hardcoded 21 `channel-*` features, so cargo's feature union switched them all
on no matter what was requested — dropping channels from the CLI defaults
produced a byte-identical binary. Each CLI `channel-*` now forwards to both
`operant-channels` and `operant-gateway`, the hardcoded list is down to the
9 the gateway's own source structurally requires, and a manifest-contract test
locks the wiring. Default builds resolve the same 23 channels as before.
- **The prompt-injection detector ran nowhere.** `PromptGuard` (override /
role-confusion / JSON-injection / exfiltration / jailbreak patterns) compiled
and re-exported with zero production call sites. It now scans inbound text at
the runtime turn entry point and records the verdict; the default stays
`Warn`, so turns proceed exactly as before.
- **Five `web_search_tool` tests flaked on a leaked proxy, not on wiremock.**
`proxy_config` tests mutated the process-global proxy without restoring it,
and the mock-backed search tests read it back — routing loopback URLs to a
dead port. A snapshot-restore guard plus a shared reader/mutator mutex fixed
it (0 failures in 13 parallel runs after 4-of-7 failing).
- **`lifeos_enabled` configured nothing and is gone.** The key gated no code —
no LifeOS tool module or cargo feature exists — so it was removed from the
schema and the example together. Existing configs carrying it now fail parse
under `deny_unknown_fields` instead of being silently ignored.

### Added

- **Turn lifecycle state machine** — a 9-state `TurnState` (idle, sending,
  connecting, thinking, streaming, running-tool, awaiting-approval,
  waiting-for-network, compacting) replaces the single `is_streaming` bool for
  display, plus per-turn wall-clock, tokens/sec, and input/output/cache deltas
  in the footer.
- **Tool-call grouping** — concurrent calls group with an `N/M done` meter and
  per-sub-call status, and a call waiting on the tool-pool semaphore now
  renders as queued rather than running.
- **Real markdown rendering** — `pulldown-cmark` drives lists (nested, ordered,
  task), italic, strikethrough, horizontal rules and footnotes. Previously
  `- item` printed literally and the renderer claimed italic support it did not
  have.
- **One clipboard path** with a five-mechanism fallback chain
  (arboard → wl-copy → xclip → xsel → OSC 52). Writes previously failed
  silently over plain SSH. Drag-select copy mode added.
- **Eight live themes** including a deuteranopia-safe palette, `reduce_motion`
  now honoured by the animation path, and Kitty/iTerm2/Sixel image rendering
  wired to paste.
- **Truecolor detection** with perceptual xterm-256 quantization, and
  grapheme-cluster + cell-width measurement (a CJK ideograph is one cluster but
  two cells, so cluster counting alone is still wrong).
- **Mermaid diagrams** render as images through the existing image stack, with
  text and raw-source fallbacks, on a background worker.
- **Provider factory registry** — a provider is now one trait impl plus one
  line, with a structural `RuntimeKey` (not a display string) and a typed
  `FailoverDecision` distinguishing a rate limit from a broken model.
- **OAuth PKCE + device-code login** with an account pool that stores only keys
  and order (never tokens) and a per-account single-flight refresh coordinator,
  so N concurrent requests cannot each spend the same single-use rotating
  refresh token.
- **Destructive-command gate** hooked into the existing approval path —
  structural blast-radius classification, a justification gate that measures
  substance rather than length, and system-path denial. Defense-in-depth, not a
  sandbox.
- **Cache-miss detection** — a prefix tracker over the stable prefix (system
  prompt + tools + skills), with proven and inferred verdicts reported
  separately. A detector reporting a hit it cannot know is worse than none.
- **Tool-name recovery** — a hallucinated or near-miss tool name is recovered by
  bounded Levenshtein, and the model is told the recovery happened. It refuses
  on an equidistant tie, because arbitrarily choosing is how you get a
  confidently wrong action.
- **Swarm workers** — real headless workers running their own agent loop, with a
  spawn guard that checks depth and a *global* breadth cap before spawning and
  releases its count on every exit path.
- **Keybinding registry wired** — `/keys` and `/hotkeys` read the catalogue and
  count real usage; input stash (`Ctrl+S`), burst undo (`Ctrl+Z`) and queued
  message recall.

- **A deployability plan** (`docs/DEPLOYABILITY-PLAN.md`) recording, from verified
  evidence rather than inference, what actually blocks shipping: two crashes on
  non-interactive paths, a native build chain that only resolved on one machine,
  a release that has never been cut, and a `main` branch that had been continuously
  red. The headline is that feature work is essentially done and deployability is
  not — nothing is wrong with what is *in* the tree; everything around it was.
- **A usage overlay** (F8) and **background task rows** showing live delegation
  state, both fed by data that already existed but was never surfaced.
- **`/terminal-setup` reports real terminal capabilities** instead of claiming
  there is nothing to do.
- **Behavioural keybinding tests** across the prompt, global and vim contexts, and
  a tokenizer-based reachability pin covering all 69 vim bindings.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- **`operant acp` gained the real protocol surface** — `initialize`, `session/new`,
  `session/prompt`, `session/cancel`, and streaming `session/update` notifications.
  The four original methods are gone; the envelope validation (including the
  notification-vs-explicit-null-`id` distinction) is kept.
- **Pinned graphics survive redraw.** Pasted images and Mermaid rasters were
  written to stdout once and forgotten, so both vanished on the first scroll — any
  redraw destroyed them permanently. They are now laid out in the frame and
  re-emitted after ratatui's flush, positioned with an explicit cursor move
  (Kitty escape sequences carry no coordinates), reserved in the cell grid so they
  cannot overlap the usage overlay or task rows.
- **LaTeX fences render** via a background worker (`dvipng` when present, a
  Unicode typesetter otherwise), so the render loop never blocks on a subprocess.
  Pending results report as pending rather than appearing blank.
- **The release pipeline can publish.** `release.yml` gates on the whole Build
  workflow's conclusion, so one unbuildable cross target (Android needs an NDK,
  FreeBSD needs a sysroot) silently blocked every release while the native
  binaries sat complete and unpublishable. The cross job is now explicitly
  best-effort; the native job still gates, because those three artifacts *are*
  the release.
- **The release build no longer depends on a third-party submodule.**
  `submodules: recursive` meant every release required `vendor/prime-agent` to be
  reachable, for a `[pk]` component that ships `enabled = false`. Verified
  (`cargo build --workspace` completes with the submodule uninitialised) rather
  than assumed. `test.yml` still initialises it — the kernel tests genuinely need
  it at run time.
- **284 foreground colour sites moved onto the theme palette** (accessor calls
  236 → 530), so the bulk of the named `Color::*` variants are themed rather than
  hardcoded. Five sites are intentionally left alone where the colour encodes a
  syntax or tool-category role the palette does not model.
- **Keybinding drift is guarded behaviourally, not just structurally.** Binding
  catalogues are now checked by pressing the chord and asserting the resulting
  state, because a source-level grep passes for a binding that is dispatched to
  the *wrong action* — which is the failure mode that actually bit `/keys`.
- **Oversized tool results are withheld, never silently truncated.** A result
  that would exceed `agent.max_tool_result_share` (default 5% of the context
  window) is replaced by a marker stating the kept head, the token count, the
  per-token price and the tokens removed.
- **1146 net lines of dead TUI deleted** — `voice_capture.rs`, `state.rs` and
  `terminal.rs` removed outright, `messages/cache.rs` superseded. Three sibling
  files had carried `#![allow(dead_code)] // wired in Phase 2I` with zero
  consumers.

### Known limitations

- **MCP turn-triggered materialisation is net-negative on short turns.** On a
  realistic 40-tool catalog a 4-token generic turn materialises 24 of them, so
  it *increases* the visible schema. Kept as a convenience (a relevant tool
  without a discovery turn), not a token optimisation; the number is pinned in
  a test.
- **Hybrid retrieval scoring is unproven and deliberately unwired.** Measured
  recall got *worse* (R@3 2/5 → 1/5). A mutation-proven gate fails the build if
  anything outside `retrieval.rs` calls it.
- **Themes still do not reach the whole TUI.** iter-414 migrated every
  foreground `Color::{White,Yellow,Cyan,Red,Green}` to a palette accessor
  (accessor calls 236 → 530), and later work removed the accent leak that kept
  14 sites amber on every non-default theme. Still unmigrated: ~89 explicit
  `Color::Rgb`/`Color::Indexed` literals (~80 distinct values), plus `DarkGray`
  and `Black` sites. These are deliberately NOT queued as a mechanical sweep:
  `DarkGray`/`Black` are fixed ANSI palette slots and therefore theme-invariant by
  construction, the default theme's `muted()` is a dim *amber*, and the transcript
  uses a deliberate cool-grey scheme — so a name-based mapping would repaint the
  UI. Each remaining site is a per-theme appearance decision, not a refactor.
- **`operant acp` implements only part of the Agent Client Protocol.** It now
  handles `initialize`, `session/new`, `session/prompt` and `session/cancel`,
  and streams `session/update` notifications with content blocks — previously it
  was a hand-rolled JSON-RPC with four unrelated methods that shared only a name
  with ACP. Dispatch was widened (`DispatchOutcome` + a `SessionRegistry`) because
  ACP's methods carry session state and `session/update` is a notification, which
  a single `RpcResponse` return could not express. Still missing versus the real
  protocol: `session/load`, `authenticate`, and a proper permission
  request/response round-trip.
- **There is no SDK server and no harness-api-server.** The `operant-harness`
  crate exposes no API surface; `operant status --json` and
  `architecture dump --live` are the only machine-readable surfaces today.


## [0.2.0] - 2026-09-27

### Security

- **`code_execution` and `patch` ignored the approval blocklist.** The command
  extractor read `args["command"]` for tools whose arguments are `code`/`path`,
  so the payload fell through to the literal tool name and matched nothing.
  `code_execution` runs unsandboxed, which made the approval blocklist its last
  real mitigation — and it was inert. Both tools now have their own extraction
  arm.
- **Credential-pool keys were written to `config.toml` in plaintext.** The
  primary `api_key` was encrypted; the pool's `api_keys` vector was not.
- **The full parent environment — including every API key — reached every
  spawned process.** The terminal backend passed `std::env::vars()` through, and
  an empty `env_vars` inherited everything implicitly. A secret-pattern scrub now
  runs at all six spawn sites. Deliberately a denylist, not an allowlist: an
  allowlist would strip `PATH`/`HOME` and break arbitrary user commands.

### Added

- **Lossless Context Management (LCM) engine** (`agent.context_engine = "lcm"`) — hermes-lcm parity: an append-only SQLite DAG keeps every message verbatim (FTS5-indexed) while the fresh-tail (D0) window stays in context. Opt-in; the built-in `compact` engine remains the default.
- **LCM rollups (P1)** — on-demand day/week/month LLM summaries (`operant context rollup`), stored idempotently in `lcm_rollups`; over-budget contexts inject stored rollups into compaction (`context_lcm_rollups_inject`, default on) so the model reads condensed history while the DAG stays lossless.
- **LCM maintenance** — on-demand sweep (`operant context rollup-maintenance`) and a config-gated background scheduler (`context_lcm_rollup_interval_minutes`, 0 = off) that build missing rollups for all sessions; a bad LLM pass is logged and never aborts.
- **LCM agent tools (P2)** — `lcm_recall` (verbatim FTS recall), `lcm_stats` (engine diagnostics), `lcm_assert` (durable, conflict-preserving fact store with active-state resolution and contradiction reporting), and `lcm_recall_round` (multi-round evidence-gated retrieval with cumulative exact evidence and search leads).
- **LCM adaptive auto-recall (P3)** — one bounded retrieval round per assemble against the latest user message injects a system "pre-answer evidence" block (`context_lcm_auto_recall`, default on).
- **AFT tool bridge** — optional native integration that surfaces the AFT code-toolkit (`aft_read`/`aft_write`/etc.) to the agent when the `aft` binary is available, with natural fallback to operant-native tools.
- **Harness kernel surfaces** — `operant status --json` exposes `harness.metrics` (mount ok/pending/failed counters, best-effort boot) and `architecture dump --live` attaches the same `HarnessMetrics` block, so operators can see mount outcomes and alert on PENDING churn.
- **Configurable prompt-cache TTL** — `client.prompt_cache_ttl = "5m" | "1h"` (OpenRouter path honors it; Anthropic clients mark breakpoints with the ttl field).
- **WhatsApp markdown** — outbound WhatsApp messages optionally convert `*bold*`/`_italic_`/`~strike~`/fences when `parse_markdown` is set (shared `asterisk_dialect` with Slack mrkdwn).
- **Pinned agentmemory spawns** — every runtime `npx @agentmemory/*` spawn (auto-spawn + MCP entries) uses the pinned `DEFAULT_AGENTMEMORY_VERSION` (0.9.29), overridable via `[memory] agentmemory_version` / `AGENTMEMORY_VERSION`. A cold boot no longer fetches whatever npm serves at that moment.
- **Mount-cap enforcement** — `max_active_providers` is enforced per mount (config default 64) with a cheap `provider_count()` check, preventing OOM from runaway mounts at large pool counts.

### Fixed

- **Piping any long-output command aborted with a core dump** (SIGPIPE
  disposition). `operant doctor | head` now exits 0.
- **`operant chat` looped forever on a non-TTY stdin**; EOF is now terminal.
- **`operant doctor` always exited 0**, printing failures and reporting success,
  so nothing could gate on it. It can now fail — and advisories ("install git",
  "not logged in") are explicitly *not* counted as failures, so a healthy install
  with unconfigured optional integrations still exits 0.
- **The build could not succeed on a machine that had never built operant.**
  `dev-env.sh` pointed at another developer's home directory and died sourcing a
  `~/.cargo/env` that does not exist outside rustup installs; the dependency
  provisioner was Debian-only and failed outright without `dpkg-deb`.
  Both now detect what the host actually has.
- **`/keys` advertised four chords that did something else**, and three prompt
  bindings that were documented but never provided.
- **web_search per-provider timeout** — every candidate engine is now bounded by `search_timeout_secs` (`run_provider_chain`), so a hung provider (e.g. a stalled igs subprocess whose own timeout can reach 60s) fails over to the next engine instead of killing the whole search at the agent-loop tool timeout. Timeouts/errors/empty results all fall through and the timeout is surfaced in the error.
- **agentmemory default alignment** — schema `MemoryConfig::default()` backend was `sqlite` while the core AppConfig default, docs, and `operant.example.toml` said `agentmemory`; the schema default is now `agentmemory` so the daemon/gateway path matches the CLI path (the injected MCP server stays deferred, and `ensure_backend` degrades gracefully with a warning when Node.js/npx is unavailable).
- **memory tool guidance** — `memory_store`/`memory_search`/`memory_recall` descriptions now state they target the builtin MEMORY.md store and point to `memory_save`/`memory_smart_search` for the agentmemory backend, so agents route to the correct memory surface.
- **`OPERANT_CONFIG_DIR` honored by every load path** — `default_config_paths()` (operant-core) now resolves `<OPERANT_CONFIG_DIR>/operant.toml` before the XDG/`~/.operant` fallbacks, matching the schema layer. Previously `operant run` (and the `config_manage` tool) silently ignored the env var and loaded the real `~/.operant/operant.toml`, so supposedly-isolated runs could mutate the user's real config.
- **Agentic self-management guidance** — `SKILLS_GUIDANCE` gains a Self-Management Protocol: consult the `operant` self-skill and `operant <cmd> --help` for CLI syntax, never read the Rust source to discover flags, trust command output (no re-runs), prefer the validating CLI over hand-edited TOML, and restore the baseline after management tasks.
- **Deployment-audit closeout (R39 series)** — doctor probes the configured endpoint rather than every provider's public default (custom gateways no longer report false `✗ (invalid API key)`); the clippy gate script survives deny violations and prints the offending site; background review no longer poisons the write-origin for the process lifetime (task-local scope); memory stats read the real session count from `database.db`; the doctor key scan can no longer send a base URL as a Bearer token.
- **Secret files created 0600 at open** — the memory master-key file and channel session stores are written with `OpenOptions::mode(0o600)` in one `open(2)` instead of a post-write `set_permissions` that left a umask-default window.
- **Dead Slack `_signing_secret` removed** — R14-4 withdrawn as a misread: Slack signature verification lives in the webhook adapter (verified against `webhooks_secret`), and the Socket-Mode adapter has no signed requests to verify.
- **Tagged-release pipeline repaired** — release workflow reads `docs/CHANGELOG.md` (was a nonexistent root path) and the dead `tdg-rust` clone step is gone from build; a `v*` tag push now produces notes and artifacts instead of exiting 1.

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Best-effort cross-target builds no longer veto a release, and the release no
  longer depends on a third-party submodule being reachable.
- A 7.6 MB platform-specific `libclang.deb` is no longer committed to the
  repository; the provisioner fetches it on demand.
- The installed binary is self-contained. It previously carried no rpath, so
  `~/.local/bin/operant` exited 127 with "libonnxruntime.so.1: cannot open
  shared object file" unless the shell that ran it had sourced `dev-env.sh` —
  a build artifact with a dependency on the caller's environment. The build now
  embeds a RUNPATH to `~/.local/lib/operant`, which `install.sh` populates —
  deliberately not the build tree, since an rpath into a checkout or a tmpfs
  dies the moment that directory is removed. (Static linking is not available:
  `ort-sys` cannot link the prebuilt ONNX Runtime that way.)
- **Version 0.2.0** — workspace bumped from 0.1.4 so `operant --version` distinguishes freshly built binaries from stale installs (stale builds reject configs containing newer fields).

### Verified

- Live agentic-loop E2E across the full LCM tool surface (6/6 PASS) and cross-process durability (4/4 PASS): facts saved to the global assertion scope and DAG marker in one process are recalled verbatim by a fresh process.
- Fresh-install cold-start: bundled skills seeded (29), 84 tools registered, igs 1.0.3 + obscura 0.2.0 (stealth) provisioned, AFT v0.50.0 auto-downloaded; agentmemory path (memory_save → server, smart_search recall, session/start + observe hooks) and builtin path (store/search/recall) both PASS.

## [0.1.4] - 2026-07-19

### Added

- Autonomous coding mode through `operant autonomous` and the `operant run --autonomous` compatibility alias
- Shared `[autonomous]` runtime configuration for autonomous polling interval, TODO path, status report path, validation command, git target, commit message, command timeout, and repeated-failure pause threshold
- Repo-root `TODO.md` task ledger with `Implemented` and `Pending` sections for autonomous workspace planning
- Repo-local `autonomous-status.toml` status reports that capture autonomous state, validation results, failure summaries, and last push targets
- Disposable-repo autonomous validation coverage that exercises the full tick loop without a live model call
- Long-term memory injection into agent system prompts via `<long_term_memory>` context built from durable `MEMORY.md` facts
- Async state distillation that extracts durable session facts into repo-local `MEMORY.md` after completed agent runs
- Workspace context-file auto-loading for `AGENTS.md`, `CLAUDE.md`, `.operant.md`, `HERMES.md`, and `.cursorrules` with prompt-injection scanning and truncation
- `delegate_to_sub_agent` tool for opt-in isolated child-agent delegation from the parent ReAct loop
- Headless TUI simulator (`operant tui debug simulate`) that drives the real TUI loop against a test backend, with `--dump-screen`/`--assert-screen` (rendered-screen assertions), `--agent-script` (deterministic offline mock agent events), dot-path state assertions over the App debug snapshot, and `--size`/`--max-frames` guards; documented in `docs/tui-debugging.md`
- `--dangerously-skip-permissions` flag that shows a confirmation dialog at startup and, on accept, runs the session in permission-bypass mode

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- TUI slash commands `/steer`, `/queue`, `/reload-mcp`, `/reload`, `/reload-skills`, and `/mouse` now perform their real actions instead of printing placeholder status (steer injects mid-turn guidance, `/reload-mcp` reconnects MCP servers live, `/reload-skills` rescans the skills directory, `/mouse` reports live capture state)
- README, `AGENTS.md`, and `CLAUDE.md` now document the autonomous workflow, the role of `TODO.md` as the workspace task source of truth, and the disposable operator workflow for validating autonomous mode safely
- Repeated autonomous failure pauses now persist across process restarts until `TODO.md` or git state changes, using `autonomous-status.toml` as the durable state store
- CLI, TUI, and autonomous sessions now reload persisted long-term memory from the current workspace before constructing agents
- The TUI workspace now uses the desktop split at 120 columns and gracefully collapses secondary panels into popups below 65 columns or 20 rows
- TUI agent reasoning now renders with quote rails, and tool activity now renders as compact blocks for clearer tool-call scanning

### Fixed

- Autonomous command execution now runs in blocking isolation with strict exit-status checks so failed validation cannot fall through to git push
- Autonomous status tracking no longer dirties workspace fingerprints or staged commits with the runtime status file itself
- TUI layout rows now preserve the primary conversation area in cramped terminals instead of letting fixed chrome starve the workspace body
- The effort picker now registers as an open modal, so background keys no longer leak through while it is visible
- `/resume` session browser now shows real last-activity timestamps and message counts instead of placeholder "just now" / 0 values
- CJK/emoji text in the ask-user dialog, status row, and new-messages indicator now wraps and sizes by display width instead of byte length
- Live TUI cost display now uses the real model-aware per-request cost from `AgentEvent::Cost` instead of a flat-rate estimate; the `/resume` session browser now shows the real accumulated cost per session (persisted via a new `Database::update_session_cost`) instead of always `$0.00` (R3 — non-streaming request path)
- Streaming mode now reports real token usage and cost too, closing the R3 gap: OpenAI-compatible providers get `stream_options.include_usage` on streamed requests and a `usage` field on the final chunk; the native Anthropic client now parses `message_start`/`message_delta` usage events. Both merge in `process_stream` and emit `AgentEvent::Usage`/`AgentEvent::Cost` the same way the non-streaming path already did
- `/tasks` now actually works as the documented alias for `/agents` in the TUI instead of printing a "not yet wired" error
- Stabilized a pre-existing parallel-test flake in `osc8`'s URL-detection tests (was racing on an unsynchronized process-global env var)
- Stabilized a matching flake in `discord_tool`'s no-token test, which was missing the `#[serial_test::serial]` attribute its sibling tests already use
- `CredentialPool::refresh_async`/`refresh_oauth_async` no longer hold a lock across the network call that refreshes each OAuth credential, which previously blocked any writer for the whole refresh loop's duration
- `operant logs --follow` (and `operant logs`) could spin at 100% CPU forever if the log file hit a read error, instead of stopping — `Lines::filter_map(Result::ok)` skips errors and can loop on a repeating error per `std::io` docs; switched to `map_while(Result::ok)`, which stops at the first error
- The context-window warning subsystem was entirely dead: `check_token_warnings()` had full 80%/95%/100% threshold logic and its doc comment said to call it after updating `token_count`, but nothing ever did — users got no warning before hitting a full context window. Wired the call into the `AgentEvent::Usage` handler; also fixed a related latent bug the wiring would otherwise have shipped live — the threshold tracker only ever escalated, so after `/clear` or `/compact` shrank usage back down, warnings would never fire again for the rest of the session. Now resets once usage drops back below the last-shown threshold
- Pasted images (`Ctrl+V` clipboard paste) showed a thumbnail row implying they'd be sent, but silently vanished on submit — the core client's request path has no multi-part/image content support yet, and nothing drained `pending_images` or told the user. `App::drop_pending_images_with_notice()` now clears them on send and pushes a warning notification instead of a silent no-op; the underlying `PastedImage`/`clear_images` plumbing is left in place for a future dedicated session to wire up real image attachment

### Removed

- 96 `cargo clippy --fix`-applied style/idiom warnings in `operant-core` (124→28) across 39 files, unblocked by fixing the one invalid clippy suggestion (`PathBuf == &str`, not a valid comparison) that had been causing the whole-crate fix to silently roll back every prior session — behavior-preserving only
- 19 more manually-judged `operant-core` warnings (28→9): 3 dead struct fields, a duplicated attribute, a doc-indent nit, and a redundant always-`Caution` branch in `skills_guard.rs`'s verdict logic (see Fixed)
- 9 manually-judged `operant-cli` warnings (136→127): an orphaned doc comment for a deleted function, a merged if/else arm, a `loop`→`while let` simplification, a manual counter→`.enumerate()`, a manual `strip_prefix`, and 2 enum variants renamed to CamelCase
- ~110 `dead_code` warnings in `operant-cli` (127→55): unused functions, methods, fields, constants, enums, and structs removed across 24 files (adapter_types.rs, app.rs, dialogs.rs, diff_viewer.rs, and 20 others), each individually verified to have zero references anywhere in the workspace, including test-only usage that a binary-only clippy scan can't see. Deliberately-parked or actually-live features (device auth flow, event-bus variants pre-published for a future registry, the model picker's real entry points) were verified and kept, not deleted
- Duplicated wraparound list-selection arithmetic across 12 TUI overlay files (`agents_view`, `effort_picker`, `hooks_config_menu`, `mcp_view`, `memory_file_selector`, `model_picker`, `plugins_hub`, `session_branching`, `session_browser`, `skills_view`, `tasks_overlay`, `theme_screen`) — each `select_prev`/`select_next` reimplemented the same "decrement, wrap to `count - 1` at zero" / "increment, wrap to `0` at `count`" logic; replaced with shared `cycle_prev`/`cycle_next` helpers in `overlays.rs` (B2), net -116 lines. Files with genuinely different selection semantics (`ask_user_dialog`'s custom-input-row wrap, `bypass_permissions_dialog`/`settings_screen`'s non-wrapping toggle/clamp) were left untouched

- Dead TUI code with no reachable callers: the legacy `ToolPermissionDialog` cluster, the `render_message` renderer family, the `RenderContext.highlight` field, and five unused `App` fields (~1,400 lines total)
- Legacy `config.yaml`/`config.local.yaml` loading from `CliConfig::load()` — `operant.toml` is now the sole file-based config source; `.env` loading and `HERMES_*` env overrides are unaffected. Also removed the now-dead `deep_merge`/`expand_env_vars_in_value` YAML-merge helpers and the redundant `"gpt-4"` model precedence heuristic in `main.rs` (env-based `HERMES_MODEL` override already covers it)
- ~125 style/idiom clippy warnings in `operant-cli`'s bin target (derived-impl opportunities, redundant closures, unnecessary borrows, `to_string` in `format!` args, simplifiable `map_or`, manual `div_ceil`, and similar), via `cargo clippy --fix` — behavior-preserving only

Full Changelog: [v0.1.3...v0.1.4](https://github.com/ishan-parihar/operant/compare/v0.1.3...v0.1.4)

## [0.1.3] - 2026-04-20

### Added

- Prompt history in the TUI input box, with `Up` / `Down` navigation that replays recent prompts and restores the current draft when you leave history browsing
- New README screenshots for the landing screen and workspace chat flow in `assets/main.png` and `assets/chat.png`
- Project-context sections in `AGENTS.md` and `CLAUDE.md` so future coding agents can immediately understand the current config, TUI, and release workflow expectations

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Conversation rendering now follows the newest assistant output by default while still allowing manual scrolling with `Up`, `Down`, `PageUp`, `PageDown`, `Home`, and `End`
- Prompt mode keeps chat scrolling available through paging keys, so long replies remain readable even while the input box is focused
- Workspace UI now labels active assistant output as `responding` when `stream = false` instead of incorrectly showing `streaming`
- README now documents the prompt history keys, conversation scrolling behavior, screenshots, and release-driven documentation sync expectations

### Fixed

- Streaming provider tool-call deltas now tolerate missing `index` fields, which prevented some NVIDIA NIM tool runs from completing in `stream = true` mode
- Streaming tool-call parsing now merges incremental argument chunks and strips split `<tool_call>` XML tags from visible conversation output more reliably
- Non-streaming mode now parses XML tool calls embedded in assistant content instead of leaving tool execution text stranded in the reasoning pane
- Final assistant replies and tool outputs now land in the conversation pane more consistently instead of leaving the workspace stuck on older chat content
- Non-Windows CI and tarpaulin coverage now pass the join-error TUI test by using terminal-free run-result assertions instead of any real TTY-backed terminal setup

Full Changelog: [v0.1.2...v0.1.3](https://github.com/eikarna/operant-rs/compare/v0.1.2...v0.1.3)

## [0.1.2] - 2026-04-20

### Added

- Workspace follow-up prompting now returns to prompt mode automatically after both completed and failed runs, so a user can continue the same session without clearing history
- Regression coverage for Windows key handling, landing prompt bootstrap, follow-up prompting after errors, and activity-pane failure rendering

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Runtime and operational errors in the rich TUI are now summarized in the footer while their detailed text is rendered in the `Activity` pane
- Activity entries now render as compact single-line log rows so failures stay visible in narrow panel heights
- README now documents the TOML configuration model, `operant.example.toml`, and the current ratatui-based interactive workflow

### Fixed

- Windows and PowerShell landing screen now paints an explicit dark canvas instead of inheriting a gray terminal background
- Landing prompt entry now accepts immediate typing on the first screen while still preventing duplicated characters from key-release events
- Landing status/footer no longer duplicates `idle` or `run failed`
- Current chat sessions can accept a new prompt after runtime errors without conflicting with agent self-healing logic

Full Changelog: [v0.1.1...v0.1.2](https://github.com/eikarna/operant-rs/compare/v0.1.1...v0.1.2)

## [0.1.1] - 2026-04-19

### Added

- Shared TOML-backed `AppConfig` runtime configuration across `operant-cli` and `operant-core`
- Config discovery with precedence `defaults < operant.toml/.operant.toml/config.toml < env vars < CLI flags`
- Config sections for client, agent behavior, TUI, MCP, skills, gateway, and tool/runtime defaults
- Responsive ratatui application architecture with landing and workspace views split across dedicated state, action, form, render, and app modules
- In-TUI panels for Session, MCP, Skills, and Behavior management, including modal forms for MCP server creation, skill creation, and behavior editing
- Example-config parse coverage to keep `operant.example.toml` synchronized with the Rust config schema
- GitHub release workflow that extracts matching release notes from `CHANGELOG.md` and publishes tagged build artifacts to GitHub Releases

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Replaced ad hoc CLI-only config parsing with shared core config loading and runtime installation
- Moved runtime-tunable defaults and provider/tool endpoints out of scattered literals in `client`, `agent`, `gateway`, `web_tools`, `http_tool`, `terminal_tool`, and `code_execution`
- Reworked rich `run` and `chat` flows to launch the new TUI instead of the previous single-screen live monitor
- Updated build workflow artifact naming so tag builds can be promoted directly into release assets
- Bumped crate versioning to `0.1.1`

### Fixed

- Reasoning, MCP, skills, and behavior state now render as dedicated TUI surfaces instead of raw merged text
- Missing or invalid TOML configuration files now fail with user-facing diagnostics instead of silently falling back

## [0.1.0] - 2026-04-17

### Added

- ReAct orchestration loop with streaming-first architecture
- Tolerant XML parser for tool call detection with early execution
- OpenAI API client with SSE streaming support
- Dynamic JSON Schema generation from Rust structs (`schemars`)
- Tool registry with 17 built-in tools:
  - `file_read`, `file_write`, `terminal`, `code_execution`
  - `web_search` (DuckDuckGo Lite scraper), `web_fetch`, `http_request`
  - `datetime` with timezone offsets and advanced formatting
  - `memory_store`, `memory_search`, `memory_profile`
  - `todo` (in-memory task list), `clarify` (agent-to-user questioning)
  - `patch` (find-and-replace file patching)
- MCP protocol client with HTTP and stdio transports
- Persistent file-backed memory (`MEMORY.md` / `USER.md`) matching Python agent format
- Skills system with YAML front matter parsing
- Gateway adapters for Telegram, Discord, and Slack
- Context window management with compression
- RL trajectory export
- Cross-platform utilities (`platform.rs`) for shell detection, config dirs, file permissions
- CLI with `run`, `chat`, `tools`, and `test` subcommands
- 99 unit and integration tests
- CI/CD pipelines: lint (rustfmt + clippy + docs), build (3 native + 6 cross-compiled targets), test (3 platforms + coverage)

### Changed

- **iter-716 — Clean-sweep 6: integration-truth fixes — `tool_backend` stops
  mis-advertising the live web_search chain.** The introspection tool claimed
  `tavily, exa, searxng, ddg` with `tavily` current — but the live chain is
  `sourcehound → tavily → exa → ddg → searxng` with `sourcehound` the config
  default and DDG the key-free fallback. Map + description now name sourcehound
  and default to it (static claim of the config default, noted in source).
  Stale `igs`-alias comment in web_tools.rs corrected (the config default is
  `"sourcehound"`, not "the old string"); the alias itself stays — pre-re-home
  configs depend on it reaching the key-free engine. NOT deleted (audit
  candidates re-verified as live): operant-tools `WebSearchTool` + routing feed
  the runtime agent registry consumed by the ACP orchestrator; the obscura/
  lightpanda names in browser_provider are intentional retired-name fallbacks
  with a pinned warning test. Suite: tool_backend 3/3 on clean origin/main
  (the shared tree is red from a peer in-flight core change — collect_map_nodes
  — unrelated to this delta).

- **iter-714 — Clean-sweep 5: composer text-area selection (jcode parity).**
  Shift+Left/Right select characters in the composer; Shift+Up/Down extend
  across visual rows (vim Visual mode keeps its existing Shift+arrow path);
  the selection renders reverse-video through the vendored wrap renderer
  (ui_input::wrap_input_text splits each wrapped segment into before/selected/
  after spans); Ctrl+C copies the composer selection and suppresses exit-confirm
  while one is live; any other edit key collapses it. The vendored surface
  gains `TuiState::input_selection()` (default None — no other implementor
  affected). Multibyte-safe via char-boundary clamping. Suite 1745/0 incl.
  3 selection tests (chords, backwards anchor, reverse-video render with
  surrounding chars intact).

- **iter-710 — Clean-sweep 4: Ctrl+Up/Ctrl+Down fine transcript scroll
  (jcode parity).** Completes the keyboard scroll ladder: ±3 on Ctrl+arrows,
  ±10 on PageUp/PageDown, ±20 on Alt+arrows, mouse wheel otherwise. Without
  the fine rung a mouseless session (tmux without mouse-mode, plain tty —
  wheel events arrive as nothing) could only jump in 10-line quanta; part of
  the "chat window is pinned" complaint (2026-10-09 visual audit). Both
  chords pause tail-follow; reaching the bottom resumes it. Suite 1742/0
  incl. chord-step regression test.

- **iter-709 — Clean-sweep 3: `last_msg_area` republished — the dead right-click
  context menu lives again.** The iter-648 chrome cutover deleted the
  dispatch-row writer that fed `last_msg_area`; every consumer (right-click
  message menu bounds, Ctrl+Shift+M cursor-open menu, menu clamping) gated on
  a zero rect and could never fire — production-dead since the cutover,
  flagged by the 2026-10-09 visual audit. The render pass now publishes it
  from the vendored `message_area()` alongside the other three hit-test cells
  (same post-draw discipline as the iter-661 fix). Pinned by a default-zero +
  setter-semantics unit test. Corpus verify 0 drift; suite 1741/0.

- Switched TLS backend from `native-tls` (OpenSSL) to `rustls-tls` (pure Rust) for cross-compilation support

### Dependencies

- Rust MSRV: 1.86
- `tokio` 1.36, `reqwest` 0.12 (rustls), `serde` 1.0, `clap` 4.5
- `schemars` 0.8, `tracing` 0.1, `anyhow` 1.0, `thiserror` 1.0

[0.1.0]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.0
[0.1.1]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.1
[0.1.2]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.2
[0.1.3]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.3
