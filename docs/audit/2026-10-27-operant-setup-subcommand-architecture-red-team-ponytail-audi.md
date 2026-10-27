# operant setup + subcommand architecture red-team (ponytail-audit), agent self-configuration, and slash-command dead-end sweep

Dated-frozen: audited `2026-10-27`; findings mark verified vs asserted.

Scope: the `operant setup` menu + wizard (`crates/operant-cli/src/cmd_setup.rs`,
`prompt_helpers.rs`), the CLI subcommand surface as an agent-operable
configuration plane, the TUI slash-command surface (99 registered rows,
98 locally dispatchable), and two operator-reported defects (slash-command
Enter mid-turn, Ctrl+R history-search parity vs jcode).

Method: every claim below was probed live on the current tree's debug binary
and, where relevant, the three deployed binaries (`target/release`,
`~/.local/bin`, `/usr/local/bin`), via `tui debug simulate` (headless real
event handlers, TestBackend), `command_sweep.py` (98-command consumption
sweep), and direct CLI invocation. The full command sweep reports
**consumed: 98/98, zero fall-through** on the current tree.

## Findings

### A. `operant setup` + subcommand architecture (ponytail-audit)

Ranked biggest cut first. Tags: `delete:` dead code, `yagni:` needless
flexibility, `shrink:` same behavior fewer lines, `native:` platform does it.

1. **[verified] `delete:` the dead `#[cfg(any())]` `/effort` row in
   REGISTERED_COMMANDS** (`operant_app/app.rs:213-215`). The `#[cfg(any())]`
   attribute compiles the row OUT, so the comment "[port-decision] keybind
   registry not ported (EFFORT_HELP); re-activate at cutover" sits above a
   second, LIVE `/effort` row at :218-221. Pure dead weight + a stale
   comment that reads as live guidance. Replacement: delete both the row and
   the comment; the live row below is the real one.

2. **[verified] `yagni:` `setup --json` is accepted, parsed, and never
   read** (`main.rs:510-512` `json: bool` → `handle_setup_command(… _json)`).
   The handler's `_json` parameter is discarded; `--non-interactive` already
   prints human-formatted status only. An agent wanting JSON already has
   `operant status --json` / `operant doctor --json`. Replacement: drop the
   flag (a flag that parses but does nothing is a lie an agent will trust).

3. **[verified] `shrink:` `setup --quick` is a no-op flag.**
   `handle_setup_command` computes `effective_quick = quick || !reconfigure`
   — i.e. quick is ALREADY the default and `--quick` cannot change any
   outcome (it can only re-assert the default). Only `--reconfigure` has
   an effect. Replacement: delete the flag, keep `--reconfigure`.

4. **[verified] `yagni:` `setup --reset` backups then re-prompts with
   dialoguer.** The reset path is a human-only confirm. For agents this is
   fine (they should not factory-reset), so the interactivity is defensible;
   recorded, not cut.

5. **[verified] `shrink:` section-dispatch duplication.** `run_setup_wizard`
   (`cmd_setup.rs:319-399`) hand-enumerates six `Some("provider"|"terminal"|
   "tts"|"browser"|"gateway"|"agent"|"governance")` arms, and `Some(other)`
   bails with "Unknown setup section". The clap side documents the same list
   in a doc comment. A table would remove the drift surface, but at 7 rows
   the duplication is near the floor; recorded, not cut.

6. **[verified] `delete:` no dead subcommands found in the TUI registry.**
   The 2026-10-14 dead-command sweep (plan 02) plus today's re-run
   (`command_sweep.py`: 98/98 consumed, exit 0) plus the static cross-check
   (99 rows registered; the 7 without a literal intercept arm — /fix,
   /improve, /judge, /refactor, /resumeall, /usage, /version — all resolve
   in `command_registry` handlers) found zero dead rows. The
   `no_registered_tui_command_is_unwired` test pins it.

### B. Agent self-configuration (subcommands as an autonomous plane)

The requirement: an agent operationalizing INSIDE operant must be able to
configure itself without a human. Verified state per surface:

- **[verified] `operant config set <dotted.key> <value>`** — fully
  non-interactive, persists to `~/.operant/operant.toml`, prints the result.
  Probed live: `config set agent.max_iterations 25` wrote and reported; the
  probe was rolled back byte-identical (md5-verified against the
  pre-probe backup). **This is the primary agent self-configuration seam and
  it works.**
- **[verified] `operant config show|path|check|migrate`, `status --json`,
  `doctor --json`, `tools list`, `mcp list`, `auth status|add|remove`** — all
  non-interactive, machine-readable where it matters. `auth add <provider>
  <key> [label]` takes the secret as an ARGUMENT (scriptable, though it
  lands in shell history — `--env`-style stdin would be better; recorded as
  a hardening note, not a blocker).
- **[verified] `operant skills install <src> --force`, `skills list`,
  `skills update`** — non-interactive with `--force` for the security-scan
  confirm. **`skills uninstall` has NO force flag** — it always prompts
  (cmd_skills.rs:965) and is a dead end for agents. Same for `skills reset`
  (:1371) and the publish/snapshot confirm at (:771, :883). Replacement:
  add `--yes` to `uninstall`/`reset` (one line each) or accept that agents
  use `rm -rf` on the skill dir (worse: it bypasses the manager's index).
- **[verified] `operant sessions` opens a dialoguer FuzzySelect** —
  interactive-only. Harmless for agents (they use `sessions --json`
  surfaces elsewhere or `resume <id>`); recorded.
- **[verified] `operant setup`** — the wizard itself is inherently
  interactive (dialoguer Select/Password/FuzzySelect on every step; no
  flags to pre-supply values), and `--non-interactive` degrades to a
  status REPORT only. **For agents, the correct seam is
  `config set` + `auth add` + `.env`, and those all work.** The wizard
  remains the human path. This is a defensible split; the one true gap is
  that `setup --non-interactive` does not say so (it prints a status table
  without naming `config set` as the scriptable alternative).
- **[asserted] The gateway slash-command surface** (59 commands) reaches
  agents on the other side of Telegram/Discord and IS an agent
  self-configuration plane; `effort`/`plan`/`context`/`export` twins landed
  in iters 1379-1386. The TUI's `/thinking-display` family has no gateway
  twin (the gateway `/reasoning [show|hide]` is the display-boolean
  ancestor, not the three-state mode). Recorded as the RT-179 backlog
  shape, not a defect.

### C. Slash-command dead-end sweep (all 99 rows)

- **[verified] command_sweep.py: 98/98 locally dispatchable rows are
  consumed; zero fall through to the model.** The static no-unwired test
  and the sweep agree. No dead ends in the registry itself.
- **[verified] `/usage` and `/version` land in `command_registry` handlers
  that only `push_system_message`** — the message renders in the transcript
  (`system_annotations` after the last message), verified on the final
  frame of the headless run. NOT a dead end (the message shows), though
  `/usage`'s handler text says "Use /stats to view" while `/usage` is
  itself documented as an alias FOR /stats — a cosmetic wrinkle: the alias
  answer could just open the stats dialog directly. Recorded, cosmetic.
- **[verified, FIXED today] Enter was a silent dead key mid-turn for every
  command except /steer+/queue** — the operator's reported blocker. See D.
- **[verified, FIXED today] Ctrl+R history search did not preview the
  selection in the composer and did not step on repeat Ctrl+R.** See E.
- **[verified] `/stats` needs TWO Enters in the headless sim** — first
  Enter consumes the typeahead suggestion (submit path inserts the
  completed command), second opens the dialog. In a live terminal the
  typed-exact path takes the first Enter; the sim's two-step is the
  popup-visible state, not a defect. Recorded so the next audit does not
  re-flag it.
- **[asserted] `handle_command` in gateway_commands.rs keeps a sync arm
  for `/export` added today by the peer** — out of my slice (peer's live
  WIP in the tree); not evaluated beyond noticing it exists and matches
  the /context stub shape.

### D. FIXED (iter-1402, commit f66d5e85): Enter on `/thinking-display off` (and any
registered command with args) while a turn streams

**Root cause, verified by headless repro with a never-ending mock turn:**
`key_handling.rs`'s plain-Enter arm gated on
`!is_streaming || is_steer_command_input(text)`. Any other registered
command fell through the match to `_ => {}` — a silent dead key. The
command sat in the composer; nothing dispatched; no notice. Idle-state
submissions worked (which is why the code paths looked correct in the
sim and in every prior test — the tests never held `is_streaming = true`).

**Fix:** the gate now also admits any input whose first word is a
registered, locally dispatchable command
(`is_registered_command_input` → `locally_dispatchable_command_names`).
The intercept arms were ALREADY mid-stream-safe
(`/clear` defers via pending_clear, `/stop` interrupts, `/new` defers,
`/undo`/`/retry` refuse with a notice, `/compact` refuses with a notice,
display toggles just flip state) — the Enter key was the only blocker.
Plain prompts stay un-submittable mid-turn (queueing is `/queue`'s job)
but now get an honest status notice instead of silence. Unregistered
`/…` text still does not submit (iter-790 catch-all contract preserved).

**Tests:** `registered_command_with_args_submits_while_streaming` (fails
without the fix — mutation-proven), `unregistered_slash_text_still_waits_while_streaming`,
and `plain_prompt_should_not_submit_while_streaming` extended to pin the
notice. Live headless verify: `/thinking-display off<enter>x` with a
never-ending turn now shows `Thinking display: Off` and the `x` lands in
the fresh composer.

### E. FIXED (iter-1403, commit 6625252c): Ctrl+R history-search parity with jcode

Gaps vs `jcode-tui/src/tui/app/prompt_history.rs`, all verified by reading
both implementations and probing operant's live:

1. **Live preview in the composer** (the operator's "previews the prompt
   in the input box"): jcode writes the selected match into the input as
   you navigate; operant showed only the floating panel. FIXED:
   `apply_history_search_preview` installs the selected entry (or the
   pre-search draft when nothing matches) into the composer on every
   query change and Up/Down step, with the same
   `suppress_suggestions_once` popup-suppression the Up/Down recall uses
   (a previewed `/…` entry cannot re-open the typeahead popup — the
   2026-10-25 stranding fix).
2. **Draft restore on Esc**: jcode's `cancel_prompt_history_search`
   restores `original_input`/`original_cursor`. FIXED:
   `HistorySearchOverlay::capture_composer_draft` at open, `restore_draft`
   on Esc.
3. **Ctrl+R again steps older** (readline): FIXED — the open overlay now
   intercepts Ctrl+R as select_prev instead of falling to `_ => {}`.
4. **Empty query**: operant's browse-all (vs jcode's nothing-until-typed)
   is KEPT — it powers the pin workflow — with one guard added: the
   browse-all does not preview into the composer until the user actually
   navigates or types, so opening Ctrl+R over a half-typed draft does not
   visually clobber it.
5. **Kept as-is (operant ahead)**: pin/unpin with persisted
   `history_pins.json`, merged persisted+live search corpus (jcode
   excludes slash commands from history; operant searches them in the
   overlay by design, 2026-10-25).

**Tests:** `history_search_previews_the_selected_match_in_the_composer`
(mutation-proven: fails without the preview wiring),
`history_search_esc_restores_the_pre_search_draft`,
`history_search_ctrl_r_again_steps_older`; all 8 history-search tests
green.

## Not in scope / left alone

- gateway_commands.rs + gateway_runner.rs (the peer's uncommitted WIP was
  in the tree during this audit; untouched).
- The `/usr/local/bin/operant` deploy leg needs sudo (F0093) — unchanged.
- jcode's persisted-prompt-history EXCLUDES `/` and `!` commands;
  operant's `input_history` records commands in the file but filters
  them from the Up/Down ladder (`load_prompts_only`). Both are defensible;
  no change recommended.

## Net

Two operator-reported defects fixed with mutation-proven regression tests;
three dead-weight items flagged for the next cleanup iteration (the
`#[cfg(any())]` row, `--json`, `--quick`); one agent-plane gap flagged
(`skills uninstall --yes`). The setup menu itself is honest — no lying
options found beyond the two no-op flags.

---

## Appendix A — OMP context-injection study (2026-10-27, turn-2 follow-up)

The operator asked for OMP parity-or-better on the three context-injection
surfaces. Sources read in full:
`external-repos/oh-my-pi/packages/tui/src/autocomplete.ts` (value/label
split, `isDirectoryCompletionValue`, `buildCompletionValue`),
`packages/coding-agent/src/utils/file-mentions.ts` (the auto-read engine),
`packages/coding-agent/src/session/messages.ts` (the `fileMention` →
LLM-content conversion),
`packages/coding-agent/src/session/agent-session.ts:8630-8660` (the submit
path), `packages/coding-agent/src/extensibility/skills.ts` +
`prompts/skills/{user-invocation,autoload}.md` (skill templates),
`packages/coding-agent/src/discovery/at-imports.ts` (AGENTS.md `@`-imports —
a DIFFERENT feature: memory-file includes, not prompt mentions).

### What OMP does (file/dir mentions)

- The composer inserts BARE `@path` values (no `@file:`/`@folder:` kind
  prefix — `buildCompletionValue` is `@"path"` for spaces, `@path`
  otherwise). The kind lives only in the popup row's icon column.
- On submit, `extractFileMentions` regex-matches every `@token` at a word
  boundary (emails excluded), `generateFileMentionMessages` resolves each
  EXACT path (no fuzzy guessing), and produces ONE `role: "fileMention"`
  message per turn packing all files.
- Per-file handling: directories → a listing (`name/ (age)` lines, 500-entry
  cap); text → full content (5 MB cap); images → real `ImageContent`
  (25 MB cap, auto-resized); videos → contact-sheet PNG. Over-cap or binary
  → a `{skipped auto-read: too large, N}` notice, file skipped, TURN
  SURVIVES.
- Model-side conversion (messages.ts:1085): text files wrap as
  `<file path="...">\ncontent\n</file>` in a **developer**-role message;
  image files go in a separate **user**-role message with real
  `ImageContent` parts.

### Operant vs OMP — surface by surface

| surface | OMP | operant today | verdict |
|---|---|---|---|
| `@` token grammar | bare `@path` | kinded `@file:`/`@folder:` (+ `@git:`, `@url:`, line ranges) | operant AHEAD (explicit, machine-parseable, zero ambiguity with prose emails) — keep |
| popup row | label/value split (`AutocompleteItem.label`) | was value-as-row (verbose) → NOW fixed: `display` field + `file_ref_display_label` (iter this) | parity |
| directory row | `name/` + icon column | `name/` + description column | parity (since this fix) |
| mid-stream popup | always active | was `is_processing`-gated → NOW fixed | parity |
| auto-read | submit-time, developer-role `<file path>` block | submit-time, appended `--- Attached Context ---` blocks in the user message | parity (different but equivalent; operant's security model is stricter) |
| per-file cap | 5 MB, skip-with-notice, turn survives | WAS: no cap → one huge file refused the WHOLE turn at the 50% budget → NOW: 5 MB cap + skip notice (this iteration) | parity |
| images/videos | real ImageContent + contact sheets | binary notice only | OMP ahead — noted, not ported (vision support is its own project; flagged for the operator) |
| skills | `[IMPORTANT: User invoked …]` + `[Skill directory: …]` template | `build_skill_invocation_message_in` mirrors both parts verbatim | parity |
| dir listing detail | flat `name/ (age)` listing, 500 cap | recursive visible-tree listing with sizes + limit + `...` marker | operant AHEAD (depth) — keep |
| budget guard | none (per-file caps only) | 50% refuse / 25% warn turn budget + cwd confinement + sensitive-path deny-list | operant AHEAD — keep |

### Delivered in this iteration

1. `@` popup mid-stream fix (render gate) — live-verified with
   `scripts/at-verify-midstream.sh`: 5-frame capture, 2 frames show the
   popup WITH directory rows WHILE the spinner runs (frames 0003/0004).
2. Display-label split — bare path rows (OMP `label` parity).
3. Double-separator fix (`@folder:crates//` → `@folder:crates/`).
4. Per-file auto-read cap in `context_references.rs` — an oversized file
   now skips with a notice instead of blocking the whole turn's injection
   (the one place operant was strictly BEHIND OMP).

### Not ported, deliberately

- OMP's developer-role `<file>` wrapper: operant appends to the user
  message; both carry the content. No behavior difference observed to
  justify a migration.
- OMP's image/video auto-read: real vision plumbing (ImageContent parts,
  resizing, contact sheets). Out of scope for a parity iteration; the
  binary notice already points the model at its tools.
- OMP's `at-imports.ts` (AGENTS.md `@path` includes): operant has no
  memory-file include expansion. Noted as a candidate, not a gap —
  operant's skills system covers the load-by-reference use case.
