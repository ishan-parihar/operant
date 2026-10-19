# remaining implementation gaps after iter-941 (red-team survey)

Dated-frozen: published `2026-10-19`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Supersedes `docs/plans/2026-10-18-runtime-model-catalog-and-remaining-gaps.md` where the
Corrections section below says so. The 2026-10-18 plan stays as written (dated-frozen).

## Mechanism

Survey basis: `origin/main` = `HEAD` = `fdf72239` (iter-941), checked after `git fetch`.
Sources read: `docs/audit/2026-06-19-BUGS.md` open sections, `TODO.md`, `docs/kb/WORKLOG.md` (tail), the
2026-10-18 plan, `AGENTS.md` genome rules, and the code.

Method and limits (so the numbers can be checked):

- **Lint gate.** `scripts/clippy-warning-gate.sh` passes `-D clippy::unwrap_used -D clippy::expect_used`.
  The most recent saved gate JSON (`--all-targets --all-features`) holds 162 `dead_code`
  warnings and no `unwrap_used` or `expect_used` diagnostics. A standalone probe crate with
  one known `unwrap()` and one `expect()` fires under the same flags, so zero hits means the
  sites carry `#[expect(..., reason)]`, not that the lints are off.
- **Config fields.** A python scan over every struct named `*Config`, `*Settings`, or `*Section`
  in files named `*config*`/`*settings*` found 701 fields. 136 have no name reference outside
  their definition file and tests. The full list is in the task output at
  `/var/tmp/ishanp-agentic/claude-1000/-home-ishanp-Documents-github-my-projects-agentic-harness-operant/1384b891-f519-4a8c-bf0a-90f22bb746e7/tasks/bin138s3z.output`.
  The check is by name, so it can miss a dead field whose name appears elsewhere. Six sampled
  hits were read by hand, and all six were dead (see RT-1).
- **Orphan files.** A name scan found 7 candidates. Five are `include!`-d (the
  `markdown_tests/cases/*.rs` set and `vendor/style/palette_literals.rs`). One is a true orphan:
  `crates/operant-providers/src/glm.rs` (RT-7).
- **Not run in this survey:** `cargo test` (last full run: iter-941, three runs, 7361 passed,
  0 failed, 23 ignored each), the live TUI, any provider endpoint, `cargo deny`, and the corpus
  validator (its 21 pre-existing errors are recorded in BUGS.md, not re-measured).

## Corrections to the 2026-10-18 plan (verified 2026-10-19)

- **P0.2 (nested models.dev schema): landed.** `crates/operant-core/src/models_dev.rs:209` reads
  `/limit/context`.
- **P1 (one live fetcher): effectively done, one wrapper left.** `fetch_models_for_provider` has no
  definition. `fetch_endpoint_models` (`crates/operant-cli/src/tui/app/providers.rs:12`) is a thin
  wrapper that calls `model_catalog::resolve`, so there is one fetcher in fact. The P1 gate
  (`grep` finds no definitions) still fails on that wrapper. It is cosmetic: inline or rename it.
- **P2 (static catalog): done for the gate string.** No `models: &[` in `provider.rs`; no
  `default_model` in `provider.rs`.
- **P3.1 (insights cost table): done.** `insights.rs` reads `crate::models_dev::cached_model_prices()`.
  No literal table in that file. P3.2 is unverified: its `compress.rs` path does not exist
  (`ugrep` found no such file), so the file has moved.
- **P4.2 ("Not started", BustPermission): stale.** `bust_permission` is at
  `crates/operant-core/src/context_management.rs:579`. It has a production caller:
  `crates/operant-core/src/agent/run.rs:2402` (via `compute_bust_permission`, `run.rs:2378`, used
  at `run.rs:2532`). BUGS.md closed it at iter-929. The P4.2 text is wrong.
- **P6.8 (`expect(dead_code)` blankets): largely closed.** No inner `#![allow(dead_code)]` remains
  under `crates/operant-cli/src/tui/`. Eight item-level sites remain in seven Rust files:
  `tui/mod.rs` (2), `rustle.rs`, `operant_model/adapter.rs`, `image_render.rs`, `copy_targets.rs`,
  `banner.rs`, `app/redraw_reason.rs`.
- **P6.9 (`:20129` pin): gone.** `~/.operant/.env` contains no `:20129`, and neither does
  `~/.operant/operant.toml`. `OPENAI_BASE_URL` is set to an `https://` host with no port. The owner's
  `/connect` re-run is therefore probably done. Confirm by reading the gateway's endpoint in the
  next session (`gateway_runner.rs:817` re-reads it per message).
- **Context-engine flag name (WORKLOG 2026-10-19T01:20): wrong name recorded.** The flag the code
  reads is `agent.context_engine`, with the value `"lcm"` (`crates/operant-cli/src/cmd_context.rs:78`
  and `:220`). It is not `[context] engine = "magic"`. The conclusion holds: plan 06 P2/P3 stay
  deferred. But `agent.context_engine` is not in `operant.example.toml`, which is itself a gap (RT-1).
- **P6.7 (`/usr/local/bin/operant` stale): not re-checked.** Still owner-run (sudo).
- **TODO.md "Pending", `[keybindings]` overrides: still not built.** No reader in
  `operant-config`, `operant-core/src/config.rs`, or `operant.example.toml` (RT-3).
- **S1, S2: unchanged.** Live-only, owner-run.

## Findings (new in this survey, not in the ledger)

Severity is a judgment: **M** = user-visible or safety-relevant, **L** = hygiene or drift.

- **RT-1 (M) — 136 of 701 config fields are read nowhere.** A user who sets them gets no effect.
  Six sampled and read by hand, all dead, each with one hit (the field and its default):
  `DisplayConfig.show_cost` (`operant-cli/src/config.rs:641`), `persistent_output` (`:638`),
  `bell_on_complete` (`:634`), `inline_diffs` (`:640`), `AuxiliaryConfig.title_generation` (`:572`),
  `TerminalConfig.docker_image` (`:300`). Of the 60 fields the scan printed, none appears in
  `operant.example.toml`; the other 76 are not checked against it. The groups: `TerminalConfig` container and backend fields (`docker_*`,
  `singularity_image`, `modal_image`, `daytona_image`, `vercel_runtime`, `container_*`,
  `persistent_shell`); `BrowserConfig` (`inactivity_timeout`, `record_sessions`, `dialog_policy`,
  and four others); `CheckpointsConfig` (`auto_prune`, `delete_orphans`, `min_interval_hours`);
  `BedrockGuardrailConfig`; `OpenRouterConfig` (`response_cache`, `min_coding_score`); `DisplayConfig`
  (the set above, plus `resume_display`, `busy_input_mode`, `tui_status_indicator`,
  `user_message_preview`, `interim_assistant_messages`, `final_response_markdown`).
- **RT-2 (M) — unknown keys are accepted silently.** `operant-cli/src/config.rs:1070` notes that a
  struct has no `deny_unknown_fields`, so retired `tirith_*` keys still load. The side effect: a
  typo in a real key is also silent, and RT-1's dead fields never warn. `warn_legacy_keys`
  (`operant-core/src/config.rs:1541`, `:1623`, `:1775`) already warn for retired keys, and can take unknown ones.
- **RT-3 (M) — `[keybindings]` user overrides are not loaded.** The registry, `/keys` report, and
  conflict check exist (`tui/keybindings.rs`, `tui/keybindings/defaults.rs`,
  `tui/keybindings/report.rs`). Nothing reads a `[keybindings]` table. This is the TODO.md Pending item.
- **RT-4 (M) — a log-file open failure panics at startup.** `operant-cli/src/main.rs:580-581`:
  `OpenOptions::new()...open(logging.log_file.as_ref().expect("log file should exist")).expect("failed to open log file")`.
  The `open` can fail at runtime (permissions, a missing directory, a full disk). The enclosing
  `init_logging` (`main.rs:551`) carries
  `#[expect(clippy::expect_used, reason = "invariant guaranteed by surrounding validation")]`
  (`main.rs:547-550`). That reason does not hold for the `open` call. Not reproduced. By reading only.
- **RT-5 (M) — the expect reason is boilerplate, and the gate does not check it.** The string
  `invariant guaranteed by surrounding validation` appears on 97 `#[expect]` annotations across
  20+ files. RT-4 and `operant-cli/src/cmd_backup.rs:42-49` (`SystemTime::now().duration_since(UNIX_EPOCH).unwrap()`,
  which fails only on a clock set before 1970) both use it. For those two, a fallible path
  (`map_err` or `unwrap_or_default`) is the honest fix. The gate passes on any reason string.
- **RT-6 (M, latent) — `unsafe { std::env::set_var }` in production.** `operant-cli/src/config.rs:1869`
  and `operant-cli/src/cmd_auth.rs:274`. Edition 2024 makes `set_var` unsafe, so the `unsafe` blocks
  are required. The safety argument is a comment. The comment at `cmd_auth.rs:274` admits the site
  "could be reached from a long-lived runtime". Tokio worker threads exist in the process. The
  claim to check: both sites run before any worker thread starts, or they run with no concurrent
  env readers.
- **RT-7 (L) — orphan source file.** `crates/operant-providers/src/glm.rs` is not compiled
  (`crates/operant-providers/src/lib.rs:29`: "glm.rs excluded — not compiled in upstream (dead code
  with known issues)"). It is still in the tree.
- **RT-8 (L) — stray untracked directory.** `xd:/` at the repo root holds one file,
  `xd:/report_issue` (220 B, dated 2026-10-16 15:10). It is a note about a todo-tool bug
  (`op=done` with a one-item list marks every open item done). It is not repo content.
- **RT-9 (L) — `Cargo.lock` is dirty in the shared tree.** Six `+` and six `-` lines moving `windows-sys`
  0.52 and 0.59 to 0.61.2. The change is the memory-wire lock sync that
  `scripts/build.sh` applies. AGENTS §7 builds in clean worktrees, so the dirty file does not reach
  the deployed binary, but it does sit in `git status`.
- **RT-10 (M) — the supply-chain gate is configured and never runs.** `deny.toml` exists (4742 B),
  and `cargo-audit` is installed (`~/.cargo/bin/cargo-audit`). No file under `.github/workflows/`
  or `scripts/` invokes `cargo deny` or `cargo audit`.
- **RT-11 (M) — the corpus validator is red and not gated.** `_validate.py` shows 21 pre-existing
  ledger errors at HEAD (BUGS.md, "Found by the iter-910 dead-code purge").
- **RT-12 (L) — ignored tests with no owner.** `#[ignore]` on seven attributes in source
  (`info_widget_stability_tests.rs` at :174, :284, :333, :388; `info_widget_git.rs:573`;
  `info_widget_tests.rs:1837`; `substitution_cost.rs:404`, which already has a reason).
  `cargo test` reports 23 ignored per run. Each needs a reason or a re-enable.
- **RT-13 (L) — two keys for one setting.** `[display] show_reasoning` (`config.rs:635`) and
  `[agent] show_reasoning`. The gateway reads `ctx.config.agent.show_reasoning`
  (`gateway_commands.rs:1424`, `:1492`, `:1942`). The TUI bridges `[display]` into it
  (`config.rs:1915`, `main.rs:2904`). TODO.md Pending "Unify" is still open.
- **RT-14 (L) — the legacy settings file is still read.** `tui/hooks_config_menu.rs:275` reads
  `Settings::config_dir().join("settings.json")`. TODO.md Pending: "retire the legacy settings.json screen".
- **RT-15 (L) — `#[allow]` where `#[expect]` is the practice.** Counts across the tree (test
  modules included): `allow(unused_imports)` 73, `allow(clippy::too_many_arguments)` 22,
  `allow(clippy::cast_possible_truncation)` 17. The practice guide prefers `#[expect]`.
- **RT-16 (L) — genome warnings.** `./scripts/genome lint` returns 0 errors and 8 warnings: W010
  (no table of contents) on `AGENTS.md`, `docs/audit/2026-06-19-BUGS.md`, `README.md`, `docs/CHANGELOG.md` (eight warnings in total).
- **RT-17 (L) — remotes.** `origin` = `HEAD`. The `github` remote is 13 commits behind
  (R40-22, known).

Owner-only items (unchanged, not findings):

- P6.7: `sudo cp ~/.local/bin/operant /usr/local/bin/operant.new && sudo mv /usr/local/bin/operant.new /usr/local/bin/operant`.
- S1 and S2: F12 `d` buffer dump in the owner's terminal (docs/audit/2026-06-19-BUGS.md).
- Plan 06 P2/P3 (historian, `ctx_reduce`): deferred until P0 and P1 are measured.
- R42-8 (Telegram tokens in gateway.log): declined by the owner. A token-shape scan found none
  in the current log. Not re-litigated.
- D-5b and D-6: accepted limits (docs/audit/2026-06-19-BUGS.md).

## Gates / success criteria

Fixed before the run. One iteration per phase, one commit, pushed, per AGENTS.md. Each phase
ends with `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
the touched crate's tests, and `./scripts/genome lint` at 0 errors. Code phases also get a
clean-worktree release build with the provenance gate.

- **G0 (this iteration, docs only):** this plan, a BUGS.md section with one row per RT item, and
  a WORKLOG entry that corrects the context-flag name. Gate: `genome lint` 0 errors.
- **G1 (RT-4, RT-6, RT-7, RT-5 for the two sites above):** `init_logging` returns `Result`, or falls
  back to stderr with a notice. The `set_var` sites carry a written, checked claim about threads. `glm.rs`
  is deleted (or restored with a reason). Gate: a test that a log path under a read-only directory
  does not panic.
- **G2 (RT-2):** unknown keys produce one warning line per key at load. Gate: a fixture TOML with a
  typo key loads and warns; the existing retired-key test still passes.
- **G3 (RT-1):** each of the 136 fields is either wired (has a reader and a test) or deleted (field,
  default, and any `operant.example.toml` line). Gate: the scan reports 0 unread fields, and
  `operant.example.toml` documents every field kept.
- **G4 (RT-3):** `[keybindings]` in `operant.toml` overrides the registry, and `/keys` shows the
  override. Gate: a test with one override; a test with a conflicting override that `/keys` reports.
- **G5 (RT-5, the rest):** no `#[expect]` carries the boilerplate reason. The gate rejects that string
  (a grep in `scripts/`). Gate: the scan finds 0.
- **G6 (RT-10, RT-11, RT-12):** `cargo deny check` runs in the local gate script, and its findings are
  fixed or listed in `deny.toml` with a reason. The corpus validator is green, or each of its 21
  errors is listed with a reason. Each ignored test has a reason in its attribute or is re-enabled.
- **G7 (RT-13, RT-14):** one `show_reasoning` key feeds both surfaces. The hooks menu stops reading
  `settings.json`, or the read is documented as import-only.
- **G8 (RT-8, RT-9, RT-15, RT-16):** `xd:/` removed, `Cargo.lock` committed or reverted deliberately,
  `allow` → `expect` for the 73 `unused_imports` sites, TOC warnings cleared.

## Work breakdown

Ordered by risk first, then by how much the owner has to decide. Where a choice is open, the default is
stated, and the iteration records it.

### Phase 0 — ledger (G0)

- **0.1** Write this plan (done) and the BUGS.md section (RT-1…RT-17 as `- [ ]` rows).
- **0.2** Append a WORKLOG entry: the context flag is `agent.context_engine = "lcm"`, not
  `[context] engine = "magic"`; the deferral conclusion stands.

### Phase 1 — safety and correctness (G1, G2)

- **1.1** RT-4: `init_logging` returns `Result`; the caller prints the error and exits 1. Default: exit
  with the path and the OS error, because a silent stderr fallback hides a misconfigured log path.
- **1.2** RT-6: grep both `set_var` sites, read each call path to `main`, and record for each whether a
  worker thread can exist. If one can, move the call before the runtime starts. Default: move it.
- **1.3** RT-7: delete `glm.rs`. The lib.rs comment already says it is not compiled upstream.
- **1.4** RT-2: unknown-key warning in the same loader that runs `warn_legacy_keys`. Default: warn,
  do not fail. A fail would break every existing config with a stale key.

### Phase 2 — dead config (G3)

- **2.1** Group the 136 fields by owner (the groups in RT-1). For each group, the default is
  **delete** unless a reader can be wired inside the same iteration. Backends that are not implemented
  (`docker_*`, `singularity_*`, `modal_*`, `daytona_*`, `vercel_runtime`, `container_*`) are deleted.
- **2.2** Display toggles (`show_cost`, `bell_on_complete`, `inline_diffs`, `persistent_output`,
  `final_response_markdown`) are wired if their feature exists in the TUI. Default: wire the first
  three, delete the rest, and record the list.
- **2.3** Add `agent.context_engine` to `operant.example.toml`, with the value `"lcm"` and its meaning.

### Phase 3 — user keybindings (G4)

- **3.1** RT-3: load a `[keybindings]` table, apply it over the defaults, and report conflicts through
  the existing `/keys` report. No new file format.

### Phase 4 — gates and hygiene (G5, G6, G8)

- **4.1** RT-5: replace the boilerplate reason with a specific one, or with a fallible path, at each
  site. Start with the two confirmed sites. The rest get a checked list.
- **4.2** RT-10: add `cargo deny check` to the local gate, and run it once now. Default: keep
  `deny.toml` as written and add exceptions with reasons, not policy changes.
- **4.3** RT-11: fix the 21 validator errors, or list each with a reason. Default: fix; they are ledger
  rows that drifted.
- **4.4** RT-12: each of the nine `#[ignore]` sites gets `#[ignore = "reason"]` or is re-enabled.
- **4.5** RT-8, RT-9, RT-16: remove `xd:/`; commit the `Cargo.lock` lock advance as its own `chore`
  commit, or revert it before the next build. Default: commit it, because the memory-wire pin moved on
  purpose. TOC warnings cleared.
- **4.6** RT-15: `allow(unused_imports)` → `expect(unused_imports)` where the import is really unused,
  else remove the import.

### Phase 5 — one setting, one file (G7)

- **5.1** RT-13: `[agent] show_reasoning` becomes the one key. `[display] show_reasoning` is read as a
  deprecated alias with a warning, which Phase 1.4 supports.
- **5.2** RT-14: the hooks menu reads its own config only. Import from `~/.claude/settings.json` stays as
  an explicit import action.
- **5.3** Cosmetic: inline or rename the `fetch_endpoint_models` wrapper (Corrections, P1).

### Phase 6 — dead code and blankets (carried, design per item)

- **6.1** The 52 allowlisted files and 114 kept names from iter-941. Each needs a design decision, so
  this phase is item by item, not a sweep.
- **6.2** The eight item-level `dead_code` expects (Corrections, P6.8).
- **6.3** BUGS.md "Event and notification variants never built" (six enums, listed there).

### Phase 7 — owner (not started by the agent)

- **7.1** P6.7 (`sudo`) and the confirmation that the gateway runs the new endpoint (Corrections, P6.9).
- **7.2** S1 and S2 (live terminal, F12 `d`).
- **7.3** Plan 06 P2/P3 go or no-go, after P0 and P1 are measured.
- **7.4** Live Anthropic and Google model listing, and the models.dev live endpoint (plan 2026-10-18
  P0.1). These need a key or network.

## Cost

- Phases 0–5: about 12 iterations, one per phase or sub-phase. Phase 2 is the largest: 136 fields, most
  of them deletions with one line each in `operant.example.toml` and tests.
- Phase 6 is open-ended, because each item is a design decision.
- Phase 7 needs the owner. The agent does not start it.
- Nothing here adds a crate dependency or a generated artifact. `deny.toml` already exists.
  `cargo-deny` is not on this machine's PATH (`cargo-audit` is), so Phase 4.2 installs it first.
