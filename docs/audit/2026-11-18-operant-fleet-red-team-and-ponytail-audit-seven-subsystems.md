# Red-team and quality audit — operant, seven subsystems (2026-11-18)

Dated-frozen audit. Reproduce against `7b2d2530` (fixes landed in iter-2642 /
iter-2643; re-run the listed commands on any later tip). Findings are marked
**verified** (reproduced against live source by this session) or **asserted**
(worker-reported, cited symbol checked, behaviour not exercised).

## Scope

Nine read-only workers, one per subsystem, each pinned to `large-stack` (not
the coordinator's GPT-6.2 model), returning one acceptance-test packet per
subsystem. The coordinator then checked every headline claim at its source.

| subsystem | packet |
|---|---|
| agent loop, retries, context budget | `runtime` |
| memory, persistence, migrations, resume | `state` |
| gateway adapters, session socket, concurrency | `gateway` |
| tool registry, MCP/aft subprocesses, voice | `tools` |
| TUI, generative UI, CLI, dashboard | `interfaces` |
| plugin host, skills, hooks | `extensions` |
| org, cron, kanban, autonomous | `organization` |
| config, setup, profile writers | `configuration` |
| build/install/release, Rust gates, docs map | `delivery` |

Assumed load throughout: one operator, several concurrent local turns and
attached clients. Not exercised: any live provider, any real socket, any
release build. `cargo fmt --check`, the workspace clippy gate and the doc/gate
scripts were run; the clippy gate result is finding 1.

## Must fix (correctness, safety, data)

1. **The workspace lint gate is red on the live tip — 39 warnings against an
   empty allowlist.** `bash scripts/clippy-warning-gate.sh` (measured at
   `7b2d2530`, 1m32s) prints `seen=39 allowlist=0 new=39` and exits 1. `.ci/
   clippy-allowlist.txt` is 0 bytes. `.github/workflows/ci.yml` pins
   `cargo clippy … -- -D warnings`, so any run would be red. Attribution:
   every `ui_tool.rs` / `ui/plain.rs` / `ui/request.rs` warning belongs to the
   Surface IR peer (open PR/branch, not fixable here); the remaining ~35 are
   pre-existing on main. Fix: land the peer's UI work, then
   `scripts/clippy-warning-gate.sh --update` once to rebaseline, and never
   again to hide a new one.

2. **The audit ledger's index disagrees with its own checkboxes.**
   `scripts/audit-ledger-index.py` FAILs on main: `RT-555` is used by two index
   rows and two detail sections, `RT-553`/`RT-554` are index rows with no
   detail section, `RT-554` closed in the checkboxes but not in the index. All
   four are a peer's in-flight rows (fleet rule 7: do not repair a peer's
   subsystem mid-flight). Recorded, not touched.

3. **`tracker.count().unwrap_or(0)` / `acquire(...).unwrap_or(true)`: the
   socket session cap fails OPEN and silently.** Verified
   `crates/operant-cli/src/gateway_runner.rs:3187`, `:3256-3260`. The channel
   path makes the identical decision and warns `the gateway session cap could
   not be enforced` (`operant-core/src/gateway/mod.rs:801-812`, RT-521); the
   socket path emits nothing. Trigger: `session_tracker_dir` a regular file, or
   an unreadable home. Consequence: unbounded concurrent turns on the socket
   surface while channels refuse correctly. Fix: copy the single `warn!` from
   `gateway/mod.rs:801-812` into both fallbacks; behaviour unchanged.

4. **Every Prompt pays a synchronous `read_dir` + `stat` sweep on a runtime
   worker.** Verified `session_server.rs:273-288`: only `LoadHistory` is routed
   to `spawn_blocking`; `handle` for Prompt/Queue/Cancel runs inline over
   `tracker.count()` (`active_sessions.rs:125`). The doc comment at
   `session_server.rs:100` ("one slow client cannot block another") is false for
   this path. Fix: widen the `matches!` to `Prompt | Queue | Cancel`.

5. **AFT: `bash_cache` never drains and `bash_waiters` leaks on timeout.**
   Verified `aft_bridge.rs:658-668` (insert, never removed on the normal path),
   `:971-976` (`get` + clone, no remove), `:997-1015` (timeout returns without
   removing the waiter). Both maps grow for the bridge's lifetime. Note:
   `:1019` says "one blocking directory pass off the runtime thread" above a
   synchronous `tracker.count()` — a comment corrected on one call site only.

6. **Hooks: `install_shell_hooks` PUSHES without ever clearing.**
   Verified `gateway_pipeline.rs:193` (`handlers.push`) versus
   `shell_hooks.rs:234` (nothing removed). `main.rs:1965` installs once per
   process, so the duplicate-execution path the worker asserted is **not
   reachable today** (asserted claim downgraded). The real defect is that the
   registry has no generation/reset seam, so any future reload doubles the
   commands. Fix if a reload lands: swap the registry atomically.

7. **`dashboard server`: the pid file is position-owned, not record-owned.**
   Verified `cmd_dashboard.rs:103` (overwrite on launch), `:112`
   (unconditional `remove_file` on exit), `:123-141` (no ownership check). A
   second server overwrites the first's record; an exiting process removes
   someone else's. Fix: refuse an overwrite naming a live foreign pid, and
   remove only a record that still names this pid.

8. **`cron run`: the daemon gate is probe-then-release and fail-open.**
   Verified `cmd_cron.rs:151-177` (`flock` closes on return), `:712-726`
   (failure proceeds), `:773` (`run_job_now` mints its own occurrence key, so
   the ledger cannot collide it). Between the probe and the run the daemon can
   start and both dispatch one cadence. Fix: hold `gateway.lock` across the
   await rather than probing it.

9. **`config set`: a failed persist still exits 0 and leaves the mutated
   runtime config installed.** Verified `cmd_config.rs:152` (install) then
   `:157-164` (`Err` branch prints "in-memory only", returns `Ok`). A wrapping
   script cannot detect the divergence. Fix: roll `install_runtime_config`
   back and return the error.

10. **`persist_config_key_at`: unlocked read-modify-rename on a FIXED temp
    path.** Verified `config.rs:3416-3498` (`path.with_extension("toml.tmp")`,
    read at `:3417`, rename at `:3498`, and the module's own note at
    `:3200-3221` that the path lock is test-only). Two concurrent
    `operant config set` calls lose one key while both exit 0. Fix: unique temp
    name + `flock` across read → render → validate → rename.

11. **`check.sh --offline` was a lie: it ran `cargo update -p memory-wire`.**
    Verified `check.sh:38` (`build.sh --sync-only` with no offline channel) and
    `build.sh:105` (guard reads only its own flag). `test-compile-gate.sh:42`
    calls it with `--offline` unconditionally, so every "offline" gate touched
    the network and rewrote the tracked `Cargo.lock` under concurrent turns.
    **FIXED iter-2643** with `scripts/tests/check_sh_offline_test.sh`,
    mutation-proven both ways.

12. **The org-sibling store could resolve to the process CWD.** Verified four
    copies of `database_path.parent().unwrap_or_else(|| PathBuf::from(
    "operant_kanban.db"))` in `cmd_cron.rs:502`, `gateway_runner.rs:5994`,
    `org/seat_budgets.rs:161`, `org/context_injection.rs:292`: a relative
    `database_path` opens a different (usually absent) seat-policy/budget store
    per working directory, so the gate reads no rows and admits. **FIXED
    iter-2643**: one owner, `org::org_sibling_kanban_path`, which falls back to
    `platform::operant_home` and never to the CWD. Writing the fix surfaced a
    SECOND instance of the same bug: `Path::new("database.db").parent()` is
    `Some("")`, an EMPTY path rather than `None`, so the fallback never fired and
    the first version of the helper still returned a relative path. The helper's
    own test caught it (`a_relative_database_path_resolves_against_the_home_never_the_cwd`
    failed with `left: "operant_kanban.db"`); the fix filters the empty parent.
    Recorded because the same `parent()` assumption is easy to repeat.

## Should fix (risk, duplication, dead surface)

13. **`estimate_tokens` is called "CJK-aware" and is not.**
    Verified `context_management.rs:27-45`: `chars().count().div_ceil(4)`
    underestimates CJK ~4x while the doc claims iter-18 made it CJK-aware. A
    CJK-heavy context passes eviction and still 400s at the provider. Either
    fix the estimate or delete the claim.

14. **Dead surface: `crates/operant-cli/web/` (115 files, 2.5 MB, 50,884 TS
    lines).** Verified: no `include_str!`, no `build.rs`, no CI job, no vendored
    doc reads it; its 172 `/api/` calls and `window.__HERMES_PLUGINS__` SDK
    target the bundle RT-44 replaced in iter-1120; a June "WebUI" commit is its
    only recent history. **FIXED iter-2643** (removed, `.gitignore` note kept).

15. **`genome outline` emitted 30 false wiring rows from the dead frontend.**
    Verified by running the derivation: `o_src` read a workspace root's
    nonexistent `name`, fell back to `crates/`, and `o_wiring` walked
    `crates/operant-cli/web/src/plugins/` — so `crates/af` → `crates/types`
    was reported as "the repo's internal wiring". The Gate block was empty
    whenever `[outline] gate` was set, and Dependencies said `none declared`
    for a root with tokio/reqwest/serde. **FIXED iter-2643**: members are the
    source dirs, the gate prints, `[dependencies]` absence is stated as
    "not derivable", and a multi-member workspace reports manifest-declared
    crate edges (verified equal to `cargo metadata`).

16. **`genome lint` is red (9 errors) and includes a doc that describes
    non-existent modules.** `docs/library/voice.md:14-16` cites
    `tts_provider.rs`, `tts_registry.rs`, `tts_command_provider.rs` — none
    exist. E311 names five source files no library page covers; E009 cites a
    missing evidence path. Peer-owned docs; recorded for their next touch.

17. **`cargo fmt --check` is dirty on one file** (`mcp_oauth.rs:1886,1894`,
    peer's tests). Mine were formatted; the peer's drift is untouched.

18. **`is_interactive_tool`'s test asserts the literal against itself.**
    Verified `agent/mod.rs:1098-1103` vs `:1152`: the test calls the function
    with a name from the same literal, so it cannot fail when the registry
    gains or renames a suspending tool. Fix: derive both directions from
    `builtin_tool_names()`.

## Nice to have (lean, clarity)

19. **`crates/operant-cli/web/dist/` ignore entry and the 95 TypeScript files
    it counted** go with finding 14; `genome outline`'s size row drops them on
    the next emit.
20. **Fleet scratch directories were untracked** — `.serena/`, `.tmpwork/`, `.tmp-test/`,
    `.w10*-tmp/` were untracked and one `git clean -fdx` from removal.
    **FIXED iter-2643** (`.gitignore`).

## Verified healthy (do not regress)

- Migration ladder: per-family append-only, transactional, idempotent
  (`migrations.rs:59-105`, three named tests).
- Cron claim-before-dispatch: the ledger refuses a duplicate occurrence even
  across processes (`cronjobs/db.rs:1099-1122`, `scheduler.rs:531-560`).
- Queued prompts hold no cap slot and are cancellable (`gateway_runner/tests.rs`
  `a_queued_prompt_holds_no_cap_slot`, `a_queued_prompt_can_be_cancelled`).
- Generative-UI gate is fail-closed before any depth/scrub work
  (`ui/gate.rs:67-79`, called at `tools/ui_tool.rs:422,525`).
- Overlay catalogue is single-source and compiler-length-checked
  (`tui/app/commands.rs:93-136`, 29 entries).
- `persist_config_key_at` rejects a broken leaf and leaves the file unchanged
  (`config.rs:5502`).
- AFT children do not outlive the bridge (`kill_on_drop(true)`).

## Architecture verdict

The crate split is sound for the stated goal (AI-maintained, recursive
iteration): `operant-api` / `operant-config` / `operant-harness` at the base
with no cycles, `operant-core` the runtime, `operant-cli` the only crate that
knows surfaces, `operant-os` and `operant-plugins` optional. No fragmentation
worth re-architecting was found this pass. The one structural cost that does
hurt recursive work is documentation truth, not code shape: finding 15 shows a
generated section asserting a wiring graph the code does not have, which is
exactly the artefact a future agent reads first. Fixes belong in the generator,
which is why they were made there rather than hand-edited.

## Not checked

Live provider retries; any real socket or dashboard session; release build and
deploy; `operant-os`/`operant-harness`/`operant-plugins` internals beyond the
plugin host's install path; the ACP server's bind race (documented residual at
`session_server.rs:55-65`).
