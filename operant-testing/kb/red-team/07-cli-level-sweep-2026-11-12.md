# 07 — CLI-level sweep (2026-11-12)

**Status:** EVIDENCE (dated measured verdict; the register owns current truth)
**Scope:** the **binary and the command surface**, not the agent loop. Round 1 tested
*tools through the agent loop*; this sweep tests `operant …` from a shell.
**Working tree it measured:** `758dd4c5` + uncommitted working-tree changes (169 MM / 9 D at
the time of the sweep). **Anchors below are at the working tree, NOT at `758dd4c5`** —
re-resolve every one with `git show 758dd4c5:<path> | sed -n '<line>p'` (or
`git log -1`) before editing the code it points at. This report does not claim register
status for any row; the executor assigns it after reconciliation.

**Full report (outside this KB):** `~/operant-testing/kb/test-reports/cli-core-report.md`
**Probe harness:** `~/operant-testing/tests/{cli_harness,battery_b,battery_c}.py`
**Raw results:** `~/operant-testing/state/battery-{b,c}.jsonl`

## 1. What the sweep covered

| Layer | Command | Result |
|---|---|---|
| Build | `scripts/build.sh --offline` | ✅ RC=0 |
| Full suite | `cargo test --workspace --no-fail-fast` | ⚠️ 89 suites — 5984 passed, **9 failed** |
| Clippy gate | `scripts/check.sh clippy --workspace --all-targets -- -D warnings` | ✅ RC=0, 0 warnings |
| TUI corpus | `crates/operant-cli/tests/tui_scenarios/_run.py verify` | ✅ **PASSED, 0 drift** (90 scenarios / 158 baselines) |
| CLI surface | recursive `--help` walk | ✅ **337/337 nodes, 0 failures** |
| Functional probes | two batteries (86 + 59) | ⚠️ 145 probes, 102 rc=0 |
| Live inference | `operant run --query` | ✅ rc=0, 9.44 s, exact answer |

Isolation: every mutating probe ran with `HERMES_HOME` (the resolver
`crates/operant-core/src/platform.rs:171` actually reads) pointed at
`~/operant-testing/state/sandbox`. The live `~/.operant` was never written by the sweep.
**No source file was modified by this sweep.**

## 2. Candidate rows (IDs unassigned — executor numbers them)

Suggested `G-020`… `G-029`. None of these collides with `G-001`–`G-019`.

| Suggested | Symptom (grep this) | What was measured | Fix shape |
|---|---|---|---|
| **G-020 CRITICAL** | `the memory engine did not accept the retain` | 80 concurrent `operant memory store` → 75 exit 0 → **66 rows**. Nine facts reported stored and never persisted. Log: `trigger memories_ai already exists` then `falling back to in-memory store` (74 fallbacks, 63 `database is locked` warnings) | Make memory-wire's `DROP TRIGGER`/`CREATE TRIGGER` migration idempotent *and* race-safe; stop the silent in-memory fallback for a write path; surface the warning on the CLI's stderr |
| **G-021 HIGH** | `[holon] org mode: recurring work is the organization's` | `operant cron create <name> <bad-schedule> <cmd>` in org mode creates **nothing**, exits **0**, and skips `normalize_schedule` — `not-a-schedule` and `99 99 99 99 99 99 99` are both accepted and proposed verbatim. Six sequential creates → zero rows | Distinct non-zero exit (or `"created": false` in JSON) for the redirect; validate the schedule **before** the holon branch |
| **G-022 HIGH** | `Next Run` column in `operant cron list` | The column prints the stored `next_run_at`, never recomputed — a job created at 14:27 still shows `2026-11-11T15:00:00+00:00` when the clock reads 19:45 | Recompute at list time, or relabel + add a "Due" marker. Extends **G-008** (paused cron jobs) on the same surface |
| **G-023 HIGH** | `libonnxruntime.so.1: cannot open shared object file` | A plain `cargo build`/`cargo test` relinks `target/debug/operant` with `NEEDED libonnxruntime.so.1` and **no RUNPATH** → exit 127. Only `scripts/build.sh` (which sources `scripts/dev-env.sh`) bakes `RUNPATH=/home/ishanp/.local/lib/operant`. Observed twice in one session | Bake the RUNPATH into the dev profile, or refuse to link without it; document it as a hard rule |
| **G-024 HIGH** | `find target -samefile target/debug/operant` | Cargo hardlinks the `--test` harness over `target/debug/operant`, so after `cargo test` the path is not the CLI at all (compounds G-023) | Build with `--out-dir`, or test `target/release/operant` / `~/.local/bin/operant` |
| **G-025 MEDIUM** | `error sending request for url (http://127.0.0.1` in test output | 9 loopback-HTTP tests fail under parallelism (mockito's process-global 50-server pool; a poisoned server is recycled to later tests — the three `model_catalog` failures all cite port 37609). All pass at `--test-threads=1`; CI's 2–4-core runners never see it | Give loopback tests their own assigned port (`bind 127.0.0.1:0` then hand the socket over) or mark them `#[serial]` |
| **G-026 LOW** | `No configuration file found. Using built-in defaults.` | `operant config path` prints that sentence on **stdout** instead of the path when no config exists — a script parsing the path gets a sentence | Always print the path; send the notice to stderr |
| **G-027 LOW** | `loading architecture from architecture.toml` | `operant architecture dump\|validate` resolves it from the process CWD, not the operant home | Resolve under `operant_home()`, then the repo root, then CWD |
| **G-028 LOW** | `"auto_fixable"` in `operant doctor --json` | `"auto_fixable": issues.len()` counts non-fixable errors (a bad API key, unregistered agents). The exit-code policy itself is correct (only `Error` severity exits 1) | Separate `auto_fixable` from `manual_required` by diagnosis kind |
| **G-029 LOW** | `sourcehound does not serve \`cloakctl.cdp\`` | `operant cookies list` needs sourcehound's `web` tool group, while `doctor` prints `✓ browser (available)`. `cookies import chrome` also treats the browser name as a file path unless `--browser` is used | Doctor should report the same dependency it cannot verify |

## 3. What this sweep *confirms* as healthy (evidence, not assumption)

- **CLI surface is complete.** 337 nodes, zero broken `--help` definitions.
- **The build gate is green.** `scripts/build.sh` RC=0; `cargo clippy --workspace
  --all-targets -- -D warnings` RC=0 with zero warnings.
- **The TUI corpus is deterministic.** `verify` → 0 drift across 90 scenarios. Its own
  determinism notes (async delegation ids, live tok/s, absolute paths) are documented
  and correctly excluded from goldens.
- **5984 unit tests pass** across 89 suites. The 9 failures are one root cause (G-025).
- **Live inference works end to end.** `operant run --query "Reply with exactly:
  OPERANT_CLI_LIVE_OK"` → rc=0, exact answer, 9.44 s. Also correct with
  `--max-iterations 1`, which exercises the budget-exhaustion wrap-up path.
- **The `Turn ended early` family (G-001/G-002) is genuinely closed on the observed
  path.** Both notices exist in the source
  (`crates/operant-core/src/agent/turn_finalizer.rs:158`, `:162`;
  `crates/operant-cli/src/tui/app/agent_events.rs:401`, `:408`) and the graceful arm
  returned the model's answer verbatim on the live probe. No reproduction from the CLI.
- **Kanban is concurrency-safe.** 8/8 concurrent `kanban create` persisted, all 8
  visible in `kanban list` — the direct contrast to G-020.
- **Profile isolation is real.** `HERMES_HOME` redirects every store (config, database,
  cron, kanban, skills, memory-wire, logs) and the live install was never written.
- **Tool dispatch is correct.** `operant test datetime` returns a real result;
  `operant test echo` reports `Invalid arguments: missing field 'message'`; an unknown
  tool gives `Tool not found` rc=1; an unknown command gives rc=2.

## 4. Cross-references into the existing register

- **G-008** (13 cron jobs paused, deploy gap) — G-022 and the org-mode branch of G-021
  are on the same `cron` surface and should be worked together.
- **G-005** (MCP response ID mismatch, fixed iter-1822) — `operant mcp list` and the
  streamable-HTTP tests in G-025 are the same transport family; the fix holds.
- **G-009/G-010** (gateway/TUI feature gaps) — `operant tui debug simulate` renders
  every overlay from the CLI, so the TUI surface is now testable headlessly. The
  remaining work is the gap map's, not this sweep's.
- **G-011/G-012** (aft_grep/aft_glob time-limited; `file_state` has no `read`) —
  reproduced unchanged.
- **G-004** (deployed aft bridge predates its source fix) — reproduced unchanged;
  `aft_inspect` still returns `unknown error`.

## 5. Re-test

```bash
cd /home/ishanp/Documents/github/my-projects/agentic-harness/operant
scripts/build.sh --offline        # G-023: NOT a plain cargo build
python3 ~/operant-testing/tests/cli_harness.py
python3 ~/operant-testing/tests/battery_b.py
python3 ~/operant-testing/tests/battery_c.py
cargo test --workspace --no-fail-fast -- --test-threads=4   # defers G-025
bash operant-testing/kb/red-team/docs/verify.sh
python3 scripts/genome lint
```

---

## 6. iter-2104 — what this iteration fixed

The sweep's rows are now closed as follows. Every claim below is measured, and
the regression test that pins it is named.

| Row | Status in iter-2104 | What changed | Pinned by |
|---|---|---|---|
| G-020 (memory silent loss) | **fixed** | The store open retries the transient migration collision (`open_store_with_retry`), a retain retries a lost write lock (`guard_retry`), the fallback log says the retains are NOT persisted, and `operant memory store` REFUSES when the store is not durable instead of printing "Stored memory". | `memory_wire::persistence_tests::{a_locked_database_is_transient_and_a_permanent_failure_is_not, guard_retry_retries_a_transient_failure_and_then_succeeds, guard_retry_does_not_retry_a_permanent_failure, two_providers_open_the_same_database_at_the_same_time}` |
| G-021 (cron org-mode false success) | **fixed** | The schedule is validated BEFORE the org-mode branch, so an unusable schedule never reaches the Chair; the redirect exits non-zero and says that nothing was created; JSON mode prints exactly one JSON document. | `cmd_cron::tests::{org_mode_create_creates_nothing_and_does_not_report_success, org_mode_create_still_validates_the_schedule}` |
| G-022 (cron stale next-run) | **fixed** | A next run that has already gone by is labelled `(overdue)`; a job that was never scheduled shows the next occurrence its schedule implies. | `cmd_cron::tests::{a_next_run_in_the_past_is_labelled_overdue, a_next_run_in_the_future_is_shown_as_it_is, a_job_that_was_never_scheduled_shows_the_next_occurrence}` |
| G-023 (no RUNPATH on a plain build) | **fixed** | `crates/operant-cli/build.rs` emits the ONNX Runtime RUNPATH, so EVERY build of the binary carries it — including the plain `cargo build` and the `cargo build --release -p operant-cli` that `scripts/install.sh` runs. | `crates/operant-cli/build.rs` (`emit_onnx_runtime_rpath`) + `scripts/dev-env.sh` (`OPERANT_RUNTIME_LIB_DIR`) |
| G-024 (cargo test overwrites the binary) | **documented** | Cargo hardlinks the `--test` harness over `target/debug/operant`; that is cargo's behaviour and cannot be changed from here. The build script's doc comment now says so, and the release/deploy path never uses that path. | `crates/operant-cli/build.rs` (module docs) |
| G-025 (loopback tests under parallelism) | **partly fixed, measured** | The mockito-backed tests no longer share mockito's process-global server pool (each gets a dedicated server — the pool was handing four unrelated tests the same dead server), and the slack tests' runtime no longer starts one worker per CPU. MEASURED: the suite is green at `--test-threads` 2, 4, 8, 12 and 16 (3024 passed each) and red at 24 on this 24-core box, where the failures are always the tests that talk to a LOCAL HTTP SERVER — they pass in isolation at 24 threads (24 passed). So the family is sensitive to a saturated machine, not to a defect in the tests. CI (2–4 cores) never sees it. | `model_catalog.rs` (`dedicated_server`), `distillation.rs`, `gateway/slack.rs` (`worker_threads = 2`) |
| G-026 (`config path` prints prose) | **open** | `crates/operant-cli/src/cmd_config.rs` carries a peer's staged change; the fix belongs there and was left alone. | — |
| G-027 (architecture file resolution) | **fixed** | The file is looked for in the working directory, each of its ancestors, and the operant home; when it is nowhere, the error names every place that was checked. | `cmd_architecture::tests::{an_absolute_path_is_looked_for_nowhere_else, the_operant_home_is_one_of_the_places_a_relative_file_is_looked_for, a_missing_architecture_file_says_where_it_looked, an_existing_absolute_file_resolves_to_itself}` |
| G-028 (`doctor --json` auto_fixable) | **open** | `crates/operant-cli/src/cmd_doctor/` carries a peer's in-flight work (a new `checks_integrations.rs`); the fix belongs there and was left alone. | — |
| G-029 (cookies) | **fixed** | A browser NAME in the positional slot is routed to the browser path instead of the file reader, so `operant cookies import chrome` no longer reports "Failed to read cookie file chrome". | `cmd_cookies::tests::{a_browser_name_in_the_positional_slot_routes_to_the_browser_path, a_real_file_path_is_left_alone, an_explicit_browser_flag_wins}` |

### One defect the sweep did not report: the tree did not build from a clean checkout

While preparing this iteration a FRESH worktree — no `Cargo.lock`, because
`.gitignore` has `*.lock` — resolved `ort` to 2.0.0-rc.15, whose
`Session::builder` takes an `&Environment`. The vendored `kokoro-tiny` calls the
older zero-argument form, so **every clean checkout failed to compile**:

```
error[E0061]: this function takes 1 argument but 0 arguments were supplied
   --> vendor/kokoro-tiny/src/lib.rs:99:23
    |
 99 |         let session = Session::builder()
```

The main tree kept building only because its local, ignored lock still pinned
ort 2.0.0-rc.12. `vendor/kokoro-tiny/Cargo.toml` now pins `ort = "=2.0.0-rc.12"`
with the reason in the file; lifting the pin means porting the vendored crate
to the rc.15 API.

**This is the shape of the whole family**: the repository does not track its
lock, so every fresh resolve is a fresh chance to break the build. The
regression that this iteration found was not in the agentic core at all — it
was in the dependency resolution, and it would have taken the fleet's CI with
it.

### 6.1 Two more findings while preparing the fix

**The vendored `kokoro-tiny` and a fresh dependency resolve.** Covered above:
`ort` is now pinned to the version the vendored source supports.

**The fake HTTP servers in the test suite were fragile against hyper's
RST-on-unread-data behaviour.** The repo does not track `Cargo.lock`, so a
fresh resolve picks hyper 1.12 where the main tree still had 1.10. Two tests
failed under the newer resolution and passed under the older one — MEASURED, by
running the same two tests against each lock:

* `client::tests::chat_streaming_retries_transient_connection_drop` — the mock
  server served the SSE response on a socket whose request head it had never
  read. Closing (or shutting down) a socket with unread peer data still in
  flight sends RST, and the reset destroyed the response the client was
  reading. The client then failed every attempt, because the connection was
  gone before the response arrived.
* `tools::http_tool::r48_tests::a_small_body_passes_through_uncapped` — the
  same shape, and its own sibling test in the same file already documents the
  rule ("Drain the request head FIRST: dropping a socket that still holds
  unread peer data sends RST and destroys the in-flight response").

Both now drain the request head before responding. This is the same class of
defect as the `ort` pin: with no tracked lock, the suite's result depends on
WHEN it was resolved, and a test that only passed against one minor version of
a dependency is a test that will break the fleet's CI on the next resolve.
