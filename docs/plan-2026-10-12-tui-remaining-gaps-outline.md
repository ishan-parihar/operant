# TUI remaining implementation gaps — 2026-10-12 outline (post-P4-2)

Ground truth as of `origin/main` at `d2e420d0` (iter-777). Sources: the four
live complaints, the v1/v2 jcode-parity plans, the P5 scrollback design doc,
`command_sweep.py` (fresh run: 44/106 consumed, 62 fall-through), the corpus
harness, BUGS.md, and live tmux verification on deployed binaries.

Supersedes `plan-2026-10-11-tui-remaining-gaps-outline.md` (pre-P4-2 state).

## The four live complaints — ALL CLOSED

| # | Complaint | Closed by |
|---|-----------|-----------|
| 1 | Legacy notification overlays | **iter-776/777** — banner + error modal + countdown machinery deleted; jcode status-notice (3s TTL) + inline `SystemMessageStyle::Error` cards; error single-sourced (no dup assistant row). Live-verified: 0 banner rows, 1 error card, composer restored |
| 2 | Duplicated streaming output | **iter-758** — SSE reasoning-triad alias collision (our parser dropped every reasoning-bearing chunk); remaining duplication is faithful render of provider-repeated deltas, by design |
| 3 | Terminal-level scrollback w/ select-copy | **iter-753–767** — inline viewport, watermark emission, preview-band suppression, `/cls` collapse+divider, P5-4 goldens; all live-verified + corpus-pinned |
| 4a | Thinking default-ON | **iter-749/758** — default + omp/laguna reasoning renders live |
| 4b | Broken slash commands | **iters 765–770** — tier 1 status (765/766), tier 2 dialogs (769), tier 3 agent-flows via suspended shell (770): 44/106 consumed. Remainder below |

## Remaining gaps, dependency/cost order

### 1. Command backlog — the 62 fall-through commands (sweep: 44/106)
The tracker is `tests/tui_scenarios/command_sweep.py`. The remaining 62 split
into four honest tiers by real value:

- **(a) Wire-only aliases (~10, one small iteration):** `/split`,
  `/split-view` (route to `splitview`), `/accounts` (→ `/account`),
  `/colors` (→ `/color`), `/subscription` (→ `/subscribe`), `/zzz`+`/zz`
  (→ `/z` if a surface exists), `/dictation` (→ `/dictate`). Same
  intercept-and-route pattern as tier 1; sweep is the acceptance gate.
- **(b) jcode surfaces with no operant counterpart — deliberate port-decisions
  (~25):** `/account /auth /remote /subscribe /support /swarm /selfdev
  /workspace /transcript /transfer /transport /wrapped /z /poke /goals …`
  Each needs a *decision*, not code: port the surface, or unregister the
  command. Unregistering is the cheap correct move for surfaces operant will
  never have (jcode account/payment UX). An unregister sweep is itself one
  iteration.
- **(c) Real features with existing seams (~12):** `/compact` (re-check —
  sweep says fall-through but adapters.rs has a ShellCommand path; likely a
  routing miss), `/commit /commit-and-push /git` (git integration), `/fix
  /improve /judge /refactor` (the registry ShellCommand handlers from
  iter-770 may not be reachable — the sweep still lists them as broken;
  verify which dispatch layer the sweep exercises), `/test`, `/cache`.
- **(d) Unknown-invented names (~15):** `/autojudge /autoreview
  /commit-push-release /cut-release /overnight /screenshot-mode
  /update-sim` — registry noise. Audit each against jcode; unregister the
  ones with no provenance.

Recommended order: (a) then (b)+(d) as one unregister/port-decision sweep,
then (c) only after verifying why iter-770's handlers don't consume.

### 2. P4-6 — split-timing audit (small, audit-only)
- Slow-frame repro of the tool-split stream seam; `tool-split-stream`
  scenario already pins the deterministic case and survived the purge
  re-prove. Remaining work: one slow-frame live repro to confirm no
  nondeterministic seam exists; fix only if a real bug surfaces.

### 3. BUGS.md: D-1 documentation debt (tiny)
- The D-1 live-data-loss entry's "FIXED iter-570/571" correction note is
  buried in prose. Fold into the next docs iteration: move the correction
  into the section header state so a grep for open-critical items can't
  misread it.

### 4. Deploy residue
- None. `~/.local/bin`, `~/.cargo/bin`, and the gateway all run iter-777
  (md5-verified). `/usr/local/bin/operant` was deleted outright (the stale
  copy that needed sudo) — root installs now require the user's sudo, but no
  root-owned operant binary exists to go stale.

## Not gaps (verified — leave alone)
- The corpus: 0-drift post-purge; `tool-block` restored to deterministic
  (its wall-clock flakiness WAS the banner countdown bar); frozen-clock
  seam deleted from `_run.py`.
- Faithful-append render policy; scroll-mode emission watermark; `/cls`
  divider; selection pane-scoping (headless A/B proved the 442-char "leak"
  was banner/animation jitter between runs, and the drag clamp was already
  in-band from iter-672).
- Prompt-entry fade pin + settled-color goldens (iter-730): stable across
  four consecutive full-corpus verifies.
- The v6 outline (`plan-2026-10-09-remaining-gaps-outline-v6.md`) is CLOSED
  (iter-775) — its rows all landed; do not re-derive work from it.

## Fleet notes
- The shared tree carries a peer's paste-burst WIP (~225 dirty files,
  shared-tree `main` parked at `8e72d80a`). Their track; untouched.
- `fleet-hold-notif-purge-wip` was deleted from origin after its content was
  extracted-and-superseded (it was a pre-765 regression snapshot, not a
  purge — adopting it would have reverted iters 765–772).
