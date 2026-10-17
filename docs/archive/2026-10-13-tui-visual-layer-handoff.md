# HANDOFF — TUI visual-layer duplication + jcode parity (2026-10-13)

**For:** senior AI-agent developer taking over the TUI visual layer.
**Ground state:** `origin/main` tip ≈ `2c5a14c9` (2026-10-13). Deployed binary:
`~/.local/bin/operant` + `~/.cargo/bin/operant` = my `fix(iter-792)`
(`ca7d39b8`); gateway runs `~/.local/bin`. A duplicate label collision exists
(`feat(iter-792)` vs `fix(iter-792)`) — append-only, both stay, next free is
iter-794.

## THE LIVE COMPLAINT (2026-10-13, user transcript, verbatim shape)

```
› Test 5 tool calls so that I can test your agentic-core
[tool table rows 3-5: env_probe / debug_system / memory_search]
Protocol spot-check (per the three probes that lie)        ← PARTIAL, CUT OFF at col ~76
The user asked me to test 5 tool calls...                  ← RAW THINKING, UNSHADED
...Let me follow the protocol and re-probe live.            ← MORE THINKING
All 5 tool calls completed successfully. Here's the test report:
[FULL tool table AGAIN — rows 1-5]
Protocol spot-check (per the three probes that lie)        ← AGAIN, this time complete
Protocol spot-check (per the three probes that lie)        ← THIRD TIME, alone on a line
```

**Decode:** the "duplicates" are the model's *reasoning draft* interleaved
with the final answer, plus the answer itself rendered at multiple
wrap-widths. The thinking block narrates the table first, then the content
block emits the real table. Symptoms to treat as ONE defect family, not
three.

## WHAT IS ALREADY DONE (verify before touching — do not re-fix)

| Iter | Fix | Files |
|---|---|---|
| iter-780 | sentinel-wrap at storage time — committed + streaming thinking dim+italic | `operant_model/adapter.rs`, `operant_render_core/reasoning.rs` |
| iter-784 | live partials render ONLY in the mutable Streaming section — stops shredded fragments in scrollback | `operant_ui/ui_prepare.rs` (`has_streaming` gate at ~:850, `prepare_streaming_cached` ~:2358) |
| iter-789 | thinking-before-tool ordering: live tail spliced into the ToolStart-reserved slot | `operant_model/adapter.rs` (`live_tail_rows`, splice in `display_messages` ~:150) |
| iter-792 (mine, ca7d39b8) | reasoning display honors config: Off/Current/Full contract; `[display] show_reasoning` bridged via RwLock override cell; `/thinking-display` flips live; `Off` skips accumulation; `Current` drops the block at flush (jcode `anchor_current_reasoning_block` semantics) | `operant_app/config_shim.rs` (cell + resolver ~:325), `main.rs` (~:2820 bridge), `app/commands.rs` (/thinking-display arm ~:1325), `app/agent_events.rs` (Off gate ~:56, Done arm ~:300), `app/messaging.rs` (flush gate ~:20), `config.rs` (CLI default → None) |
| iter-790 | not-wired command catch-all (both dispatch loops) — sweep 106/106 | `app/commands.rs` tails + `cmd_tui_debug.rs` sim |
| iter-776/777 | notification purge + single-source error card | `app/messaging.rs` (`push_notice`), dead `notifications.rs` surfaces |

**Gates at ship time:** suite 1751/0 (`AR=/usr/bin/ar cargo test -p
operant-cli --bin operant --no-default-features` for the TUI-only suite;
default-features build too), corpus `verify PASSED: 0 drift`
(`cd crates/operant-cli/tests/tui_scenarios && OPERANT_BIN=<fresh build>
python3 _run.py verify`).

## WHAT REMAINS — the actual job

### 1. The interleave in the user's paste is NOT fully explained yet
The 2026-10-13 paste shows the thinking draft + answer rendering TOGETHER in
scrollback in an order that suggests: thinking rows emitted to native
scrollback at ONE wrap width, answer rows at ANOTHER, interleaved by the
runtime-card redraws. My live repro (rivers, event-logged at
`/tmp/duplog2.txt` — tmpreaper may have eaten it; regenerate with
`OPERANT_TUI_DEBUG=1 OPERANT_TUI_EVENT_LOG=<path>`, dumps on clean `/quit`)
proved the *data path* clean: Reasoning → Content → Done, once each. So the
defect family that remains is RENDER-side:
- **Suspect A — scrollback emission of committed thinking in Full mode.**
  `emit_settled_transcript_rows` (`app/scroll_emit.rs`) emits the settled
  committed prefix each frame. With `show_reasoning = true` (the user's
  TOML, line 27 of `~/.operant/operant.toml`), thinking COMMITS as a
  `ContentBlock::Thinking` message — it is settled transcript, so it
  emits. If the user's terminal then ALSO shows the live-tail render of
  the same content (in-flight frames), you get the same prose at two
  widths. Trace: add a row-hash to `committed_prefix_rows()` and log
  emitted-row hashes; a repeated hash = double-emission, not interleave.
- **Suspect B — per-iteration flush residue.** `agent_events.rs:46-58`
  flushes on new-thinking-after-text. If Content arrives AFTER the final
  Thinking flush (laguna's triad), the tail's wrap differs from the
  committed one. Check `flush_streamed_assistant_message` call sites
  against the event log order.
- **Repro recipe:** tmux 110x45, `OPENAI_BASE_URL=http://localhost:20129/v1`
  (omp gateway, routes `small-stack` → poolside/laguna-s-2.1:free — the
  ONLY local provider that emits the reasoning triad; it is FLAKY
  mid-session: streams die, render as single error cards — that's the
  iter-777 path, not a bug). Config: copy `~/.operant/operant.toml` to
  `/tmp/inline2.toml`, merge `[tui] terminal_scroll_mode = true` into the
  EXISTING `[tui]` section (duplicate table = parse error — this bit me
  twice). `/tmp` is tmpreaper-swept: recreate before every repro run.

### 2. jcode parity — remaining surfaces (full sweep in the outline)
Read `docs/plan-2026-10-13-tui-remaining-gaps-outline.md` §A/§B. Ranked:
- **Current-mode in-place anchor** (jcode `TurnReasoningTrace`,
  `reasoning_region.rs:150-200`): trace stays readable until next user
  prompt, then GC'd. Ours just vanishes at Done. Port = the real fix for
  "thinking disappears too fast" complaints that will follow iter-792.
- **`▸ thought for Xs` affordance** — jcode `render_reasoning_message`
  consumers; ours has `reasoning_heading()` in `transcript_turn.rs:56`
  marked `#[allow(dead_code)]` — resurrect it.
- **Renderer line cache** (jcode `get_cached_message_lines`,
  `ui_prepare.rs:1600`): we re-render markdown every frame; measure first.
- **`build_transcript_turns`** (`transcript_turn.rs`) is DEAD CODE — no
  live consumers (only docs/tests reference it). Either wire it or delete
  it; it's the second renderer that confuses every audit.

### 3. Select-and-copy jcode parity
- App-mode (alt-screen): drag paint clamped in-band (iter-672); corpus
  `selection-highlight.scenario.json` pins it (read its `notes` — the
  boot-dialog Down-swallow is DOCUMENTED, correct behavior).
- Terminal-scroll mode: mouse capture is deliberately OFF — selection is
  NATIVE terminal selection. If the user wants jcode's app-level paint in
  scroll mode, that's a DESIGN DECISION, not a bug — take it to the owner.
- jcode reference: `crates/jcode-tui/src/tui/ui.rs:2448` (expand/collapse
  badge hit-testing), `app/navigation.rs:261`.

## REFERENCE MAP (all paths from repo root unless noted)

**Core render pipeline (ours):**
- `crates/operant-cli/src/tui/operant_model/adapter.rs` — `display_messages()`,
  tool-row anchoring, live-tail splice, `live_tail_rows()`; THE flatten layer
- `crates/operant-cli/src/tui/operant_ui/ui_prepare.rs` — frame assembly,
  `has_streaming` gate (~:850), `prepare_streaming_cached` (~:2358),
  `prepare_body`, dispatch table (~:1660 mirrors jcode)
- `crates/operant-cli/src/tui/operant_ui/ui_messages.rs` —
  `render_reasoning_message` (:314), assistant renderer
- `crates/operant-cli/src/tui/operant_app/markdown.rs` — render entry,
  reads config shim (~:96)
- `crates/operant-cli/src/tui/operant_render_core/reasoning.rs` —
  `reasoning_line_markup` / `reasoning_partial_markup` / `REASONING_SENTINEL`
- `crates/operant-cli/src/tui/app/agent_events.rs` — event → state machine
  (Thinking/Reasoning/Content/Done/ToolStart/ToolComplete arms)
- `crates/operant-cli/src/tui/app/messaging.rs` —
  `flush_streamed_assistant_message` (pub(crate)), `push_notice`,
  `clear_view_terminal_style`
- `crates/operant-cli/src/tui/app/scroll_emit.rs` — scrollback emission,
  `committed_prefix_rows()`, watermark, `/cls` divider
- `crates/operant-cli/src/tui/app/mouse.rs` — drag paint, in-band clamp
- `crates/operant-cli/src/tui/operant_app/config_shim.rs` — display config
  snapshot + reasoning override cell (RwLock, ~:325-350)
- `crates/operant-cli/src/tui/app/mod.rs` — `App` struct (fields:
  `streaming_text`, `streaming_thinking`, `tool_use_blocks`, `messages`)

**jcode reference (vendored):**
- `../parent-projects/jcode/crates/jcode-tui/src/tui/app/input/reasoning_region.rs`
  — THE reasoning lifecycle (open/append/close/anchor/GC; byte-offset
  invariants documented in module header, issues #632/#633/#635)
- `../parent-projects/jcode/crates/jcode-tui/src/tui/ui_messages.rs` —
  `render_reasoning_message` (:295)
- `../parent-projects/jcode/crates/jcode-tui/src/tui/ui_prepare.rs` —
  role dispatch + `get_cached_message_lines` (:1600+)
- `../parent-projects/jcode/crates/jcode-config-types/src/lib.rs:244` —
  `ReasoningDisplayMode` enum (Off/Full/Current; **default Off**)
- `../parent-projects/jcode/crates/jcode-tui/src/tui/app/input.rs:3731` —
  `collapse_reasoning_for_commit`

**Tests + corpus:**
- `crates/operant-cli/tests/tui_scenarios/` — `_run.py` (harness;
  `OPERANT_BIN` env, per-scenario `config` TOML fragment, `prove` writes
  goldens), `command_sweep.py`, `*.scenario.json`, `baselines/`
- Scenarios that pin this defect family: `thinking-block-reasoning-attr`,
  `thinking-block-streaming`, `tool-split-stream`, `tool-block`,
  `selection-highlight`, `terminal-scroll-mode`, `terminal-scroll-cls`
- Regression tests: `operant_model/adapter.rs` tests module
  (`reasoning_mode_contract_current_and_off` — single-test sequential,
  the override cell is process-global, parallel threads race);
  `app/tests.rs` (`thinking_display_defaults_on_and_the_command_toggles_it`
  — resets the cell at its end, keep that reset)

**Event-capture debug tooling:**
- `OPERANT_TUI_DEBUG=1 OPERANT_TUI_EVENT_LOG=<path>` — ring dump on clean
  `/quit` (DebugHub::dump_on_exit). Format: JSON array, `kind:
  agent_event`, `summary: 'Content { text: "..." }'` (Debug-format, not
  JSON payloads — parse with regex)
- `tui debug simulate --keys '<k1><enter><k2>' --mouse '<l,x,y><d,x,y>'`
  — headless; NOTE the sim's TestBackend makes `insert_before` inert
  (scrollback emission untestable headless — live-verify only)

**Docs:**
- `docs/plan-2026-10-13-tui-remaining-gaps-outline.md` — the exhaustive
  gap sweep this handoff summarizes
- `docs/plan-2026-10-10-terminal-scrollback-p5-design.md` — scrollback
  design (thinking-status note corrected iter-771)
- `docs/plan-2026-10-09-tui-jcode-parity-gaps-v2.md` — P4 wave specs
- `AGENTS.md` — THE protocol file (read top-to-bottom before any change)
- `BUGS.md` — open-issues ledger (D-1 doc-debt row)

## PROCESS DISCIPLINE (non-negotiable, learned the hard way)

1. **One iteration = one fix = one push.** Label from `origin/main` AFTER
   `git fetch`, never local history (duplicate `iter-792` on main right
   now proves the race is real). `git log origin/main --oneline | grep -c
   "iter-N"` MUST print 1 after your push.
2. **The shared tree is HOT** — peers work in it. Never `git add -A`;
   never commit from the shared tree; ship from a dedicated worktree
   (`git worktree add --detach ~/worktrees/<name> origin/main`) via
   python unique-anchor patches (origin-side text can differ by fmt —
   extract anchors AFTER `cargo fmt`).
3. Gates per iteration: `cargo fmt --all` → scoped build + suite → corpus
   (`OPERANT_BIN=<fresh> python3 _run.py verify`; the 4-drift baseline
   [journey-view ×2, skills-view ×2] is pre-existing peer drift — never
   yours) → commit explicit paths → push → label check → release build →
   deploy BOTH `~/.local/bin/operant` and `~/.cargo/bin/operant` (cp to
   `.new`, then mv — running daemons hold old inodes) → `systemctl --user
   restart operant-gateway` → `md5sum /proc/$(systemctl --user show -p
   MainPID --value operant-gateway)/exe` == artifact.
4. **Always `AR=/usr/bin/ar`** and `source scripts/dev-env.sh` first — a
   foreign CLI shadowed binutils' ar twice.
5. Rebuild the debug binary after EVERY edit before running tests — a
   stale cached binary produced a false regression reading once this
   session (§1053→§1055 pattern).
6. The suite has ONE known parallel-DB flake:
   `gateway_commands::tests::grant_seats_ungoverned_seat_and_mints_ttl_grant`
   — passes 3/3 in isolation; rerun before concluding regression.
7. omp (the local model gateway) is flaky: mid-stream deaths render as
   single error cards (correct behavior). Retry via `continue`. Provider
   probes: `curl http://localhost:20129/v1/models` returns
   auth-required for anonymous — that's ALIVE.

## RUST DISCIPLINE

Apply `/home/ishanp/.agents/skills/rust-best-practices/` (Apollo GraphQL
handbook) on every change — see AGENTS.md §"Rust Code Standards" which
mirrors it. Read the relevant chapters (in `references/chapter_*.md`) in the
same turn you write the code. The repo's mechanical enforcement:
`bash scripts/clippy-warning-gate.sh` (fails on NEW warnings only;
`-D clippy::unwrap_used -D clippy::expect_used`; `--update` ONLY to prune
warnings you fixed). Production code: no `.unwrap()`/`.expect()` — use
`#[expect(clippy::…, reason = "…")]` + `python3
scripts/expect-annotate.py crates/` for justified sites.
