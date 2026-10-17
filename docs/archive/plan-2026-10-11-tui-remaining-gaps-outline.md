# TUI remaining implementation gaps — 2026-10-11 outline

Ground truth as of `origin/main` after iter-762 (P5 surface complete).
Sources: the four live complaints, `plan-2026-10-09-tui-jcode-parity-gaps-v2.md`,
`plan-2026-10-10-terminal-scrollback-p5-design.md`, `command_sweep.py` tracker,
and live tmux verification on deployed binaries.

## Complaint status (all four)

| # | Complaint | State |
|---|-----------|-------|
| 1 | "Todos: 0/0 done" | **Closed** — P4-3 todo store wired; band is live |
| 2 | Duplicated streaming output | **Closed** — root cause was ours: SSE reasoning-triad alias collision silently dropped chunks (iter-758 salvage); remaining "duplication" is faithful render of provider-repeated deltas |
| 3 | Terminal-level scrollback w/ select-copy | **Behaviorally complete** — P5-1/2/3 + `/cls` collapse+divider, live-verified; P5-4 goldens remain (below) |
| 4a | Thinking default-ON | **Closed** — P4-4 + omp/laguna reasoning now renders (post-758) |
| 4b | Broken slash commands | **Open** — tiers below (37/106 consumed, ~69 handlers) |

## Remaining gaps, in dependency/cost order

### 1. P4-2 — notification purge (PEER'S ACTIVE WAVE — not ours)
- Shared tree hot (213 dirty files); peer WIP preserved at `8e72d80a` /
  `fleet-hold-notif-purge-wip` on origin.
- After their landing: corpus banner-golden reshapes (~11 goldens) are THEIR
  iteration's work. We re-verify corpus only.

### 2. Command tier 1 — status-message handlers (smallest, daily value)
- `/version` `/usage` `/info` `/model-status`: consume → one-line status
  notice. Data paths already exist in `tui debug` functions.
- Tracker: `python3 tests/tui_scenarios/command_sweep.py`; add each command
  to consumed set + a scenario pin per handler.
- ~4-6 commands, one iteration, no dialogs.

### 3. P5-4 — mode-aware corpus scenarios (scroll-mode goldens)
- Harness already writes a pinned `operant.toml` per run (`-c`).
- Open question: does headless `tui debug simulate` exercise the inline
  viewport? Its terminal backend may make `insert_before` emission inert —
  if so, goldens scope to viewport-floor + preview-gate surfaces only, and
  the emission path stays live-verified (document that in the scenario).
- Add `[tui] terminal_scroll_mode = true` config variants; goldens for:
  live-window collapse after Done, `/cls` clear state, watermark floor.

### 4. Command tier 2 — dialog/panel handlers
- `/login` `/models` `/agents` `/observe`: vendored pickers exist for
  several; wire command → dialog open + corpus pin per dialog.

### 5. P4-5 — selection pane-scoping (complaint #3's copy surface)
- Headless proof: drag paint spans whole frame chrome — 442 chars / 17
  lines for a 5-line convo incl. left rail + info widgets.
- Port jcode `CopySelectionPane::{Chat,SidePane,Input}`; extend
  `selection-highlight` scenario with a chrome-spanning drag asserting
  borders/rail unpainted.

### 6. Command tier 3 — agent-flow handlers (largest; each a project)
- `/improve` `/judge` `/review` `/refactor`: jcode runs real loops.
  Scope each to its own plan before touching.

### 7. P4-6 — residual split-timing audit
- Slow-frame repro of stream/tool-split timing; audit only, fix if a
  real seam bug surfaces.

### 8. Doc hygiene (fold into the next code iteration's commit)
- P5 design doc "Thinking status" section is stale: written pre-758
  ("kilo gateway emits none") — superseded; omp DOES emit the reasoning
  triad and it renders live.
- Corpus 4-drift baseline (journey-view ×2, skills-view ×2) is
  pre-existing peer drift — not ours to pin.

## Deploy residue
- `/usr/local/bin/operant` stale (`82fb8fcd`, 3 iterations behind) —
  awaiting user sudo; gateway runs `~/.local/bin` (current, iter-762).

## Not gaps (verified — leave alone)
- scroll_anchor.rs consumers, thinking render, todo renderers, selection
  copy-reflow: shipped and suite-pinned.
- Prompt-entry fade pin + settled-color goldens (iter-730): stable across
  4 consecutive verifies.
- Faithful-append rendering policy: duplicated deltas render duplicated by
  design; no dedup belongs in the TUI.
