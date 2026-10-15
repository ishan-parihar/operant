# 2026-10-14 batch: TUI and agent-core plans (index)

Written for an implementing agent. Each plan stands alone. It gives the evidence (with
`file:line`), the root cause, the design, ordered steps, the tests to write first, and the
acceptance criteria. Line numbers were taken from `origin/main` @ `642fecd4` (iter-811);
re-locate symbols with grep if they have drifted.

| # | Plan | Size | Depends on |
|---|------|------|------------|
| 01 | [Duplicate tool rows that heal on scroll](01-duplicate-tool-rows.md) | M (diagnose first) | — |
| 02 | [Remove dead and jcode-only commands](02-command-removal.md) | S | — |
| 03 | [Wire reasoning effort to providers](03-reasoning-effort.md) | M | — |
| 04 | [Esc / `/cancel` stop streaming only, context intact](04-cancel-preserves-context.md) | M | — |
| 05 | [Rewind selector (oh-my-pi style), core-backed](05-rewind-selector.md) | L | 04 |
| 06 | [Context management à la magic-context, memory via memory-wire](06-context-management.md) | XL (phased) | 04 |
| 07 | [Per-instance memory footprint, shared services](07-memory-footprint.md) | L (phased) | — |
| 08 | [Thinking and tool-call show/hide toggles](08-visibility-toggles.md) | S | 10 (keymap table) |
| 09 | [Native terminal scrollback as the default](09-native-scroll.md) | L | 01, 08 |
| 10 | [Shortcut and overlay coherence](10-shortcut-overlay-coherence.md) | M | 02 |
| 11 | [Own integrations only: auto-sync, purge alternatives, one memory model](11-integrations-and-purge.md) | L (phased) | — (06 P4 uses its banks) |

Recommended order: **11A → 02 → 10 → 08 → 03 → 04 → 01 → 05 → 09 → 11B/C/E → 07 → 06 → 11D**.
11A (memory-wire sync on every build) comes first because every later build should
compile current upstream.
02 and 10 are cheap and remove most of the confusion. 04 must land before 05 and 06,
because both rely on the stored conversation staying consistent. 01 is needed before 09,
because native scrollback freezes whatever is emitted, so a duplicate would become
permanent.

## Working conventions (mandatory)

- Work in a git worktree, never in the shared tree: `~/worktrees/<name>` from `origin/main`.
  Peer agents commit to `main` concurrently, and the shared tree has peer uncommitted edits.
- One iteration = one commit = one push. Label it `iter-N`, where N is taken from
  `git fetch && git log origin/main --oneline | grep -o 'iter-[0-9]*' | head -1` + 1.
  After pushing, verify `git log origin/main --oneline | grep -c "iter-N"` == 1. If a peer
  took the label, rebase and relabel.
- Environment: `source scratchpad/env.sh` (it sets `LOCAL_DIR`, sources `scripts/dev-env.sh`
  and `AR=/usr/bin/ar`), or run `export AR=/usr/bin/ar` before cargo.
- Once plan 11 phase A lands, build through `scripts/build.sh` (it syncs memory-wire first).
- Gates, every iteration:
  1. `cargo fmt --all`, then revert unrelated churn (`git diff --stat`). Known churn files:
     `operant-cli/src/cmd_context.rs`, `operant-config/src/secrets.rs`,
     `operant-core/src/recap.rs`, `operant-core/src/client.rs` (`dedup_reasoning_keys` hunk).
  2. `cargo test -p operant-cli --bin operant` (default features; `--no-default-features`
     is broken, BUGS K-4).
  3. `cargo test -p operant-core --lib`. The only accepted failures are K-1
     (`tools::kernel` `ping_roundtrip`, `harness_apply_and_rollback_roundtrip`).
  4. Screen corpus: copy the debug binary, then
     `cd crates/operant-cli/tests/tui_scenarios && OPERANT_BIN=<copy> python3 _run.py verify`.
     Don't edit sources or check out other commits while it runs, because it reads
     baselines from the tree. Known flake: `agents-menu`; rerun it before calling it a
     regression. Re-prove a golden only for an intended visual change, and say so in the
     commit message.
  5. Clippy parity: the count of warnings before equals the count after, same command on
     both trees.
- Tests come first: write the failing test and confirm it is RED on the old code. A test
  that passes before the fix proves nothing.
- Docs, in the same commit: `CHANGELOG.md` (Unreleased), `README.md` if user-facing,
  `operant.example.toml` when a config field changes, `TODO.md` (Implemented/Pending), and
  `BUGS.md` for anything found and not fixed.
- Deploy (only when the user asks): release build; back up `operant.prev-HHMM` in
  `~/.local/bin` and `~/.cargo/bin`; `cp` to `.new`, then `mv`; run
  `systemctl --user restart operant-gateway`; check the md5 of `/proc/<pid>/exe`.
- Never `pkill -f` with a pattern that matches your own shell command.

## Cross-cutting test infrastructure (build once, used by 01, 04, 08, 09)

The headless simulator applies a whole `--agent-script` before the first frame is drawn
(`tui/adapter_types/tui_app.rs:880-935`: events are buffered, and `drain_agent_events`
drains them all in one frame). Mid-stream states (reserved tool rows, live tails, boundary
flushes) are never rendered in the corpus. On top of that, `prepare_messages` and
`prepare_body_cached` bypass every cache under `cfg!(test)`
(`operant_ui/ui_prepare.rs:681`, `:1140`), so **no test exercises the render caches during
streaming**.

Add `--frame-per-event` to `tui debug simulate`: while simulating, `drain_agent_events`
handles at most one `AgentEvent` per loop iteration, so each event gets its own painted
frame. With `--capture-frames`, that yields the mid-stream frames. Plan 01 builds its
oracle on top of this.

## Decisions taken in these plans (reversible; listed for review)

These are judgment calls made so the work isn't blocked. Each one is marked in its plan.
Change any of them before implementation starts.

1. **Shift+Tab** cycles reasoning effort (oh-my-pi parity) instead of permission mode.
   The current cycle reaches *Bypass permissions* in two keypresses. Permission mode moves
   to a new `/permissions` picker (plan 10).
2. **Ctrl+T** toggles thinking (oh-my-pi). Copy mode moves from Ctrl+T to **Alt+Y**, which
   is jcode's copy-mode key. Alt+Y yank-pop is dropped; Ctrl+Y yank stays (plan 08/10).
3. **Ctrl+Shift+O** shows or hides tool activity (oh-my-pi), with **Alt+O** as the
   fallback for terminals without kitty keyboard reporting (plan 08).
4. **Rewind forks** the session: the old path is kept as a parent-linked session, and
   nothing is deleted (plan 05). The truncate-in-place alternative is rejected.
5. **Native scrollback becomes the default** (`terminal_scroll_mode = true`), once plan 09's
   acceptance criteria pass. Full-screen overlays temporarily switch to the alternate screen.
6. **The magic-context engine sits behind `[context] engine = "classic" | "magic"`**,
   default `classic`, until the soak test passes (plan 06).
7. **The shared-services daemon is the existing `operant gateway` process** (plan 07 phase 2),
   not a new binary. The full thin-client mode (jcode-style) is phase 3, and is optional.

## Resolved with the user (2026-10-14)

- **memory-wire:** in-process only, pulled from upstream `main` on every build
  (plan 11 phase A). No HTTP/daemon backend and no alternative memory providers.
- **Native scroll:** must be fully functional, like oh-my-pi. On resize, history is always
  replayed at the new width, so there's no option to set (plan 09).
- **No alternative web-search, browser or memory providers:** sourcehound and memory-wire
  only (plan 11 phase C).

## Still open (not blocking)

- Plan 07 phase 3 (thin client, one server process for all sessions): decide after phase
  2's measurements.

## Execution status (2026-10-15)

Landed iterations live in `docs/CHANGELOG.md` under `[Unreleased]`. This index
is the plan→iteration map:

| Plan | Status | Iterations |
|---|---|---|
| 11A build entry + provenance | **done** | 815 |
| P2 sim infra (frame-per-event, count asserts) | **done** | 816 |
| 02 dead commands | **done** | 817 |
| 10 shortcut/overlay coherence | **done** | 820 |
| 08 visibility toggles | **done** | 821 |
| 03 reasoning effort (end-to-end) | **done** | 829 |
| 04 cancel preserves context | **done** | 832 (live Esc-during-tool red-team pending provider) |
| 01 duplicate tool rows | **oracles + S1b/S2/S3/S4 code done** (836); S1/S2 live verdicts pending provider | 836 |
| 05 rewind (core + selector) | **done** (live 3-turn proof pending provider) | 829, 838, 839 |
| 09 native scroll (step 1, K-3) | **done**; steps 2-7 pending live verification | 839 |
| 11B binary updater | **done** | 841 |
| 11C1 web providers purge | **done** | 842 |
| 11C2 browser purge | **done** | 843 |
| 11C3 memory-wire only | **done** | 846 |
| 07.1 footprint (drains, reap, budgets, measurement, debug surface) | **done** (lazy MCP folded into 07.2) | 848, 850, 851 |
| 11E core seam (bank_for) | **done**; seat wiring blocked on the org/ rebuild | 856 |
| 07.2 shared services | pending (gateway zone in flight) | — |
| 06 context management P0-P5 | pending (prompting/compress in flight) | — |
| 11D crate audit | pending (last by order) | — |

Live-provider red-team items (S1/S2 glyph verdicts, Esc-during-sleep-20,
3-turn rewind, native-scroll live) run when the model provider stabilizes;
the harness for each is documented in session note #10 and the batch
corpus covers every headless-verifiable claim.
