# Operant task ledger

Autonomous mode (`operant autonomous`, `operant run --autonomous`) reads this
file (`[autonomous] todo_path`): it works the `## Pending` list, runs
`[autonomous] test_command`, commits to `[autonomous] git_branch`, and records
progress in `autonomous-status.toml`. Only the `## Implemented` and
`## Pending` sections are parsed. Anything that needs an owner decision belongs
in `## Review`, which autonomous mode never touches.

Working a `## Pending` row, one row per iteration:

- Write the test the row names first and watch it fail. Then make it pass, and run `cargo fmt --all`, `cargo check --workspace`, `cargo test --workspace` and `bash scripts/clippy-warning-gate.sh`.
- Mark the row's BUGS.md entry `[x]` with the iteration id. Move the row to `## Implemented` with the same id.
- If a gate fails twice on the row, the fix changes a Holon contract (kernel fields, notify semantics, the driver), or it needs an owner decision, move the row to `## Review` with the output or the reason. Do not guess.

## Implemented
- [x] Prompt-jump chords: Alt+[ / Alt+] navigate the transcript by QUESTION (iter-1461) — the scroll ladder stepped rows and pages with no way back to the prompt you care about. The status row names the landed prompt and both directions; positions come from the frame the renderer last drew, not a second mirror. Ctrl+[ / Ctrl+] are also dispatched on the terminals that report them, but CAUTION: `Ctrl+[` is byte `0x1b` (Esc), so under the kitty protocol it arrives as `Esc+CONTROL` — three always-live Esc arms (streaming-cancel, double-Esc-rewind, transcript-focus) had to gain `KeyModifiers::NONE` guards or the jump is swallowed. Do not widen those arms back; the guard pair is pinned by tests.
- [x] Session hub: preview pane renders, Tab/PageUp/PageDown/Home/End route by pane, and rewind marks seed on session switch (iter-1476, audit 2026-10-29 D1/D2/D4/D5) — four operator-reported live defects. D1: the preview drew nothing (one row per stored message now: role label + flattened text + tool-call labels; loaded on a blocking thread over its own channel with the row id inside the message so a late load is dropped, not painted). D2: there was no pane-focus field, so Tab/PageUp had nothing to route — `PaneFocus` with Tab toggle, h/l, and the four step keys + PageUp/PageDown/K/J + Home/End acting on the FOCUSED pane. The stuck “Loading session…” notice (which timed out with the switch already applied, so it read as a hang) is replaced by an honest “No transcript cached — press r to load” + an `r` arm in both panes. D5 is the rewind half: `user_turns` only ever grows (never re-seeded), so a session switched INTO had no marks or a stranger's — `set_session_id` now re-derives the loaded conversation's marks. **Residual:** the hub still opens its own `Database::init` per open (303 ms measured for `list_sessions_rich(500)` on a 105 MB DB) and the selector's sibling-branch / prompt-text-filter / tail-cap items (plan 2-5) are open.
- [x] Rebuild `/settings` over `operant.toml` (iter-1176, RT-171): the screen's OWN save path was discarding what you typed. `settings_screen/state.rs::apply_and_save` offered 19 editable entries and matched FIVE, so toggling `notifications`, `verbose`, `vim`, `reduce_motion`, `show_cwd`, `fileInjectionEnabled` and 8 more rendered the new value, stored it in `pending_changes`, and dropped it in `_ => {}` — while still rewriting the whole `settings.json`. Five of the snapshot's fields were CONSTANTS (`verbose: false`, `cursor_blink_enabled: false`, `disable_claude_mds: false`, `file_injection_enabled: false`, `OutputFormat::default()`), so saving ANY setting reset all five to defaults. All 19 keys now apply, every snapshot field is carried from `settings`, and two regression tests pin it — one derives its key list from `all_entries` so a newly offered entry with no applier fails the build. **Residual, still open:** the screen still persists to the LEGACY `settings.json` rather than `~/.operant/operant.toml`, and 4 of the 19 entries write `AppConfig` which `apply_and_save` never saves — so `max_tokens` and `compact_threshold` still do not survive a restart. The toml_edit rebuild (tabs per config section, comments kept, live-apply, retire the settings.json screen) is the remaining half

- TUI rows render once while tools run (iter-797)
- Synchronous, viewport-independent drag copy (iter-797, iter-801)
- Reasoning display: jcode Current semantics by default (iter-797)
- Streams outlive `timeout_secs`; stream drops always earn retries (iter-799)
- Large pastes reach the model, not their placeholder (iter-800)
- Copy is clean document text: logical lines, no gutters, no tool rows (iter-801)
- Copy carries no chrome: system cards rejoin; code, quote, table as text (iter-802)
- Tool-loop guardrails warn the model and stop the turn cleanly (iter-805)
- Selection highlights only the text a copy carries (iter-806)
- oh-my-pi shortcuts, conflict-checked; `[tui] external_editor` (iter-807)
- Essential commands wired (`/compact`, `/cancel`, `/transcript`, `/changelog`, `/refresh-model-list`, `/auth`, `/goals`); jcode-only commands removed (iter-808)
- 35 more dead commands purged; wiring pinned by `no_registered_tui_command_is_unwired` (iter-817)
- Help/keys overlays generated from the keybinding registry; `/permissions` picker; Shift+Tab no longer cycles permission mode (iter-820)
- Visibility toggles: Ctrl+T thinking, Ctrl+Shift+O/Alt+O tools; copy mode on Alt+Y (iter-821)
- Reasoning effort end-to-end: config → request shape (per-provider) → rejection fallback → Shift+Tab/picker (iter-829)
- Cancel preserves context: typed `Error::Interrupted`, tail committed with synthetic tool results, interruptible tools, 3s watchdog (iter-832)
- Fullscreen `/rewind` selector over the real transcript; double-Esc opens it; forks are parent-linked sessions (iter-838)
- Web search and browser are sourcehound-only; memory is memory-wire-only (MEMORY.md/USER.md imported once) — alternates accepted-with-warning then ignored (iter-842/843/846)
- Subprocess hygiene: child stderr drains (no 64KB-pipe freezes), warmups reaped (no zombies), tool-output retention budgeted at 4MB (iter-848/850)
- `operant debug memory` + `docs/MEMORY_BUDGET.md` footprint surfaces (iter-851)
- Memory banks: `bank_for(identity, scope)` — seat banks (`emp:*`) isolated from the personal bank (iter-856)
- Terminal-scroll watermark keyed on message boundaries, not rows (BUGS K-3, iter-887)
- Async memory writes carry their bank from enqueue; a scope change after queueing does not move the write (BUGS GW-4, KB-7, iter-962)
- Skill writes are screened and rolled back when refused; skill names and descriptions are escaped in the system prefix; a Holon run's learning delete or edit is an annotation (BUGS KB-4, KB-5, KB-9, iter-962)
- Observe posture denies `holon_skill_invoke`, which writes the org ledger (BUGS OS-4, iter-963)
- Every turn starts unseated: an unbound chat no longer keeps the previous chat's employee seat (BUGS GW-3, iter-965; the test covers the seat accessor, not the consult in `stream.rs`)
- `/resume` retargets the agent: the picked session becomes the agent's session and the transcript is rebuilt from it, through the branch-switch path (BUGS TUI-2, iter-966)
- A new agent never replaces an existing definition, dangling symlinks included; the error names the file (BUGS TUI-3, iter-964)
- Agent descriptions round-trip through save and reload: a multi-line description is written as one JSON-quoted YAML scalar (BUGS TUI-4, iter-964)
- Any key other than y or n cancels the `/rewind <n>` confirmation (BUGS TUI-5, iter-964)
- `chair notify` exits nonzero when no chair was reached (BUGS OS-5, the exit-code half, iter-963)
- Holon bodies are fenced from the supervisor's secrets and their slot (BUGS OS-1, OS-2 implicit path, OS-6, OS-7, OS-8, OS-9, OS-10; AD-006, iter-967). Residual: an explicit read of the operator's chair key by a same-uid body waits on the O-11 runner isolation.
- Org mode refuses a Holon run whose `policy.json` is missing (BUGS OS-3; AD-007, iter-967)
- Holon chair notices are retried until delivered, and driver placeholders must be plain tokens (BUGS OS-5 retry half, OS-11; AD-008; Holon commit `992b08f`, local to that repo)
- Gateway budgets meter per turn into a seat-spend ledger charged to the start seat, and a hard cap fails closed when its usage cannot be read (BUGS GW-1, GW-8; AD-009, iter-968). The ledger starts empty on upgrade.
- A chat's conversation id is kept on its session row: `/new`, a seat change and `/resume` pick a transcript that survives a restart, and the continuation summary is keyed by it (BUGS GW-5, GW-6, GW-7; AD-010, iter-968)
- Cron and socialization runs set their seat as the memory scope, and `/session` no longer names the premiere for an unbound chat (BUGS GW-2, GW-9; AD-010, iter-968)
- The TUI's approval prompt answers its key before any overlay (BUGS TUI-1; AD-011, iter-969)
- `/rename` saves the confirmed name, and a bare `/rename` starts once the list loads (BUGS TUI-8; AD-012, iter-969)
- A delegation whose turn is cleared is marked abandoned, and the HUD shows a running one first (BUGS TUI-9; AD-013, iter-969)
- A shell-out restores the terminal before its result is reported, and frees the mouse for the child (BUGS TUI-6, TUI-7, TUI-10; AD-014, iter-969)
- The Ctrl+B fork tree is read off the event loop, and a database error is shown (BUGS TUI-11 branch half; AD-015, iter-969)
- The TUI's command descriptions match their handlers, and `/mouse` reports the real capture state (BUGS TUI-12, TUI-13, iter-969)
- The TTS engine's ONNX Runtime panic is a tool error, and the release profile unwinds (BUGS CR-1, CR-3; AD-016, iter-970)
- ONNX Runtime is pinned to 1.24.4 in every script and AGENTS.md, and `install.sh` refuses a library without API 24 (BUGS CR-2; AD-017, iter-970) — **SUPERSEDED by iter-2513**: Kokoro is gone and nothing links `libonnxruntime` any more, so this pin, the `install.sh` refusal and the `dev-env.sh` rpath were all removed with it. The CR-1/CR-3 panic-to-tool-error conversion went too: `ptts` returns `Result` and has no panic-on-load hazard to catch.
- [x] The local TTS default is Phonon (`gradium/phonon-31m-en`, GGUF q8_0) instead of Kokoro (iter-2513, RT-588): 41M vs 82M params, streaming on CPU at ~20x real time, and the ONNX Runtime dependency — a hard `libonnxruntime.so.1` NEEDED entry that needed an rpath and a library copy to survive an install — is gone entirely. GGUF over the ONNX export because `ptts` runs it directly on its `xn` CPU backend. Checkpoint is revision-pinned and downloaded into `~/.cache/phonon/` on first use, never a crate dependency.
- `operant doctor` reports a tool available only through its configuration, flags a second `operant` on PATH, and the config loads once (BUGS CR-4, CR-5, CR-6; AD-018, iter-970)
- Enter submits registered slash commands mid-stream, `/thinking-display off` included, and a plain prompt gets an honest "turn is running" notice instead of a dead key (2026-10-27 red-team audit; iter-1402)
- Ctrl+R history search previews into the composer live, restores the draft on Esc, and steps older on repeat Ctrl+R (jcode parity; iter-1403)
- The `@` file-ref typeahead renders while a turn streams, rows show the bare path (no `@file:`/`@folder:` grammar), a directory never gains a doubled separator, and an oversized `@file:` skips alone with a notice instead of refusing the whole turn's injection (operator report 2026-10-27; OMP file-mentions parity; iter-1418)

- [x] **Ctrl+Tab model-cycle alias (jcode-shortcuts audit gap 2 — iter-1476).** One arm aliasing `cycle_configured_model`; no new cycle state, per this row's own instruction. The reverse direction (jcode's ctrl+shift+tab) stays un-scheduled because crossterm folds shift into BackTab and a reverse cycle is a feature, not an alias. Mutation-proven: removing the arm leaves the model unmoved.
- [x] **Unify the gateway's `agent.show_reasoning` with the TUI reasoning display mode (iter-1473).** `/profile` printed the raw `[agent] show_reasoning` bool, which cannot express the TUI's three-state contract (Off / Current / Full) — so a `/thinking-display full` session with the bool false reported `false` and contradicted the screen. Now resolved through the SAME shim the TUI boot bridges (`config_shim::config().display.reasoning_display()`, set on both the TUI and `gateway run` paths), so the channel reports the mode in force rather than a second opinion about it.

## Pending

Work the plan `docs/plans/2026-11-21-close-the-residual-implementation-gaps-the-pub-fn-queue-the-.md` top-down. One row per iteration.

- **T4 (do first, docs-only) — close the two stale `TODO.md` residuals.** Measured at iter-2777: (a) the session-hub `Database::init` per open does not exist — `grep -rn 'Database::init' crates/operant-cli/src/tui/session_hub/` hits only `tests.rs`, and `open()` resets state and calls `rebuild_visible()` with data arriving over the cached-then-refresh mpsc; the "plan 2-5" selector items (`sibling-branch`, `prompt-text-filter`, `tail-cap`) are absent from the tree and the 2026-11-14 hub plan records all seven operator items closed (iter-2317..2351). (b) the settings screen DOES persist config keys to `operant.toml` via `persist_config_key_at` (iter-2645) — the config-key half is closed; the `settings.json` half is a DELIBERATE boundary (the TUI's own visual state must not leak into a shared `operant.toml`, per `settings_screen/keys.rs:127` and `state.rs:355`) and belongs in `## Review` as a design question. Rewrite both residuals to say what is measured, split the conflated half, and move the `settings.json` question to `## Review`.
- **T1 (long tail, 8-10 iterations) — the pub-fn queue: 100 NEW candidates.** `python3 scripts/audit-pub-fn-surface.py` at `265b5ccb`: 2115 pub fns, 103 candidates, 3 accounted, **100 new**. A candidate is NOT a verdict — the acid test is: comment out the `pub`, `cargo check -p operant-core --lib`, restore. Passes -> the `pub` is dead, delete it and add a RED-proven needle to `crates/operant-core/tests/dead_pub_fns_stay_dead.rs`. Fails -> it stays with a recorded reason. Work top-down by test-only caller count (highest first: `roster_path_in` 15, `add_session_tag` 12, `plan_span` 10, `canonical_name` 9). Exclusions the script's own header names: `#[cfg(test)]`-only helpers whose `pub` is load-bearing for a sibling test module, `pub use` re-export shims, doc-example callers, and the 142 multiply-defined names the script skips.
- **T2 (2-3 iterations) — the keybinding dispatcher.** `crates/operant-cli/src/tui/keybindings/registry.rs` exists and IS consumed (`tui/app/providers.rs`, `tui/settings_screen/tabs.rs`), but `tui/app/key_handling.rs` never calls into it (`grep -c 'keybindings::' key_handling.rs` = 1, a comment). So `[keybindings]` overrides in `operant.toml` are parsed by the catalogue and IGNORED by the dispatch. Take the resolve-then-dispatch shape: the dispatcher asks the registry for the action bound to `(key, modifiers, context)` and runs it; the if-chain becomes the registry's `Default` impl rather than a second copy. Dispatch-then-annotate was rejected because it can only warn, so it documents the gap instead of closing it. Acceptance: a test that sets an override, runs the dispatch, and asserts the overridden action fired. NOT in scope: new bindings, chord changes, the `vim.rs`/`word_nav.rs` split.
- **T3 (1 iteration) — the lint-suppression ledger.** `python3 scripts/audit-lint-suppressions.py`: 112 suppressions across 52 files, 122 baseline entries, 3 "unexplained", 13 conversion candidates. The 3 unexplained are SCANNER false positives — each carries an inline `reason` the regex does not read, and the script emits a Python `SyntaxWarning` for an invalid `\[` escape in that regex, so it has been silently not matching. Fix the scanner to read the reason (catches the next one too) OR add a baseline entry per site. The 13 `allow`->`expect` conversions: do `agent/run.rs:1391` and `:2582` first (`unused_assignments` on a `mut` binding is a smell the `allow` masks); the `operant-harness` `dead_code` ones are lower value. `.ci/` allowlist is untouched.
- **T5 (1 iteration, survey only) — the 195 unreached config keys.** `audit-config-keys.py --check` reports 77 keys and "-195 reach all four surfaces", with 6 inherited gaps: `browser`, `checkpoints`, `database_path`, `moa`, `skills`, `terminal_backend`, each missing one or more of note/heal/template/doctor. Do NOT fix them here — the audit says they are inherited. Enumerate them into one table here with the missing-surface matrix, marking which have a `doctor` gap (can silently misconfigure an operator) vs a `template` gap (documentation only), so a later plan can pick one surface.

- `[keybindings]` user overrides in `operant.toml` (`/keys` now shows the effective registry bindings + conflicts — iter-820; overrides remain). The blocker is the DISPATCH: `tui/app/key_handling.rs` still runs the if-chain, and the registry is the catalogue, not the dispatcher — the keybindings module calls migrating it "a separate, riskier change". Until then the table is read-and-ignored with a warning, and `operant tui keybindings` prints the bindings in effect instead of opening a dead file (iter-1133, RT-146). Re-scoped 2026-10-29: the remaining-gaps plan examined this and deliberately did NOT schedule it — it is a dispatcher REFACTOR, not a feature, it delivers no capability the operator asked for, and `key_handling.rs` is the repo's most contended file (the peer's live WIP has sat in it for days).

## Review

Decided on 2026-10-19; each choice and its reason are in the record named. The owner can overturn any of them with a new record.

- **NEW (iter-2777, from T4's measurement): does the TUI's OWN visual state belong in `~/.operant/settings.json`, or in `[tui]` in `operant.toml`?** Today it is `settings.json` on purpose -- `settings_screen/keys.rs:127` and `state.rs:355` both record that theme, cursor blink and reduce-motion must not leak into a SHARED `operant.toml` (a per-machine visual preference in a file that gets committed is a footgun). But it is also the last surface the settings screen writes outside the one-config-system rule, and a second file is a second thing to back up, migrate and explain. Options: (a) keep `settings.json`, document it as the deliberate per-machine visual store and close the question; (b) move the visual keys to `[tui]` with a one-time migration from `settings.json` and a `doctor` note on the old path. No code changes until the owner picks.

- Autonomous publish: AD-027. The loop stages and commits only the paths its tick changed.
- Seat default for new org chats: AD-028. New chats stay unbound. The comment that calls an unbound session ungoverned is still open for review.
- Context management: AD-029. The synchronous summarize rung stays; agent-driven reduction and a background historian are not built.
- BUGS.md R42-8 (Telegram tokens in gateway.log): AD-030. The filter is covered by a test; the historic log is not rewritten; rotation is the owner's call.
- TUI-11, git-diff half: AD-026 (closed). KB-6 retention: closed by iter-971 (AD-019).

### Residual-gaps survey, 2026-11-21 — the owner-gated rows

Plan: `docs/plans/2026-11-21-close-the-residual-implementation-gaps-the-pub-fn-queue-the-.md`
(T1..T5, executable, peer-published iter-2778). The claimable tranches live on the board
`docs/kb/workflows/residual-gaps-2026-11-21.md` (T4, T1a-T1e, T2, T3, T5 — claim with
`./scripts/genome claim <id>`).

**The rows below are owner-gated, so they sit here and never in `## Pending`.** Each is
also a `blocked` row on that board (R1/R2/R3/T3a/T3b), with its evidence.

- [ ] **R1 — RT-543 `operant acp` disposition.** Reviewer (context-pack round 4, iter-2772)
      recommends (a) deprecate `--socket`, keep stdio. Campaign prior: (a). Not docs-only
      (edits `cmd_acp.rs`): needs crate-scoped tests plus a release deploy.
- [ ] **R2 — RT-280 resume/stop flags.** Reviewer recommends **(c) delete both flags and
      their 3 writers**. This **REVERSES the campaign prior (a)** — the reviewer's ground is
      that (a) needs a `max_age_seconds` the repository cannot supply. Read it as a
      disagreement, not a confirmation. Needs a schema-compatibility decision so existing
      SQLite rows still deserialize.
- [ ] **R3 — RT-190 cross-surface memory.** Reviewer recommends (c) accept two families and
      close the row; campaign prior (c). Docs/ledger-only when ruled.
- [ ] **T3a — `plugins-wasm`: is the surface worth its advisories?** The 15 wasmtime
      advisories are triaged+allowlisted (iter-2763, reason per entry); what is unruled is the
      feature. extism 1.41.0 pins wasmtime 49.0.3 and is its own latest release, so they are
      permanently unbumpable while it is on. Keep on (allowlist keeps this reason) or default off.
- [ ] **T3b — `/reset` durability (ASKS Q6a).** `reset_slot_for_session` clears
      seat/model/effort/posture/charter and never touches the store, so a "wiped" history
      returns on restart. Not a false claim (RT-389 corrected the doc) — a missing capability.

Evidence, per-ruling confidence, and the one measurement that would change each answer:
`research/audits/context-pack-2026-11-20-blocker-rulings/VERIFICATION.md` Part 1.

**Checked and NOT gaps — do not re-open:** `config_registry.rs` is generated+gated (409
keys); `operant-api` and `operant-harness` are live dependencies, not parallel surfaces;
there are no `TODO:`/`FIXME:` markers in `crates/` (the 2 matches are test fixtures); 72 TUI
modules / 349 files and 7 crates are accurate; the 15 wasmtime advisories are allowlisted;
the `libsqlite3-sys` bump is triaged (RT-616).
