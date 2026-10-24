# Runbook — exact commands, environment traps

Everything below is the shipped path; where a trap exists it is named. The
authority for the iteration loop itself is AGENTS.md ("START HERE").

## Build / verify gates

| task | command | traps |
|---|---|---|
| dev environment | `source scripts/dev-env.sh` | sets LIBCLANG_PATH, ORT_LIB_LOCATION, PKG_CONFIG_PATH; every cargo command needs it (bare-shell `operant doctor` exits 127 — TTS dlopen of libonnxruntime) |
| scoped check (never `--workspace` mid-iteration) | `./scripts/check.sh check -p operant-core --lib` | scope to the crate you edited; workspace is the final gate only |
| build (syncs memory-wire lock first) | `scripts/build.sh --release` | one sync code path; `--offline` skips the sync, `--strict` fails on sync failure |
| clippy gate | `bash scripts/clippy-warning-gate.sh` | red at the pristine tip during peer W-zone work — prove your own delta is zero, don't fix the peer's (fleet rule 7) |
| supply-chain gate | `bash scripts/supply-chain-gate.sh` | `cargo audit` against the committed lockfile; fails on any advisory the committed allowlist does not cover. `cargo-audit` is NOT on PATH — the script looks in `~/.cargo/bin`. `cargo-deny` is not installed, so the licence and duplicate-version policies in `deny.toml` stay unexecuted by design. |
| test compile gate | `bash scripts/test-compile-gate.sh` | every kept crate's TEST targets compile |
| doc claims gate | `bash scripts/check-doc-claims.sh` | derives AGENTS.md's code-surface claims (CLI command count and list, gateway platforms, error classes and their AGREEMENT across the doc, hook events, workspace members, AFT tools, TUI modules, every path in the Architecture Overview tree) FROM THE CODE and prints the differences. Exits 1 on any drift. A doc claim about the code is a test in disguise — RG-011. |
| build script tests | `bash scripts/tests/libclang_resolve_test.sh && bash scripts/tests/supply_chain_gate_test.sh` | the libclang resolver (three scripts resolving libclang three different ways broke the build) and the supply-chain gate's own fail-on-new-advisory contract |
| format | `cargo fmt --all` | |

## Fleet protocol traps

| trap | rule |
|---|---|
| iteration number | read from `origin/main` after `git fetch origin` (top label + 1); local history races concurrent agents |
| ff catch-up | `git diff --name-only <base> origin/main` ∩ `git status --porcelain` must be empty first; git refuses the ff otherwise |
| stashes | repo-wide across worktrees; never pop a peer's stash; `git stash push -u -m` if preserving full WIP |
| build env in worktrees | `local/` is NOT in git: export `LOCAL_DIR=<shared-tree>/local`, `LIBCLANG_PATH=$LOCAL_DIR/libclang`, `AR=/usr/bin/ar` |
| kernel-sidecar tests | `tools::kernel::*` pass ONLY in the main tree (untracked `.venv` + uninitialized `vendor/prime-agent`) |
| deploy | sessions have no sudo: `~/.local/bin/operant` only; `/usr/local/bin` and the system libonnxruntime install need the owner. Deploy: build from a clean worktree at origin/main, `mv`-over (running daemon holds the inode), verify md5 of `/proc/<MainPID>/exe` |
| TUI corpus runner | `env -u OPENAI_BASE_URL -u OPENAI_API_KEY python3 crates/operant-cli/tests/tui_scenarios/_run.py verify <name>` — scenarios without `agent_script` build a real agent and hang on live calls otherwise; per-scenario invocation prints progress; `OPERANT_BIN` must be a FRESH build copy |
| TUI corpus: NEVER run `prove` and `verify` concurrently | `prove` REWRITES the goldens it proves; a `verify` running at the same time reads them mid-rewrite and reports dozens of spurious drifts. Observed: 29 phantom drifts from the overlap, against 6 real ones. One at a time, always. |
| TUI corpus: a golden never encodes a transport error | a scenario that drives a real agent turn renders the failure into the transcript, and the text embeds a reqwest/hyper error string that changes with the library version and the platform — so the golden is machine-dependent. `context-menu` was excluded for exactly this. |
| TUI corpus: a scenario that cannot pass is EXCLUDED, not left red | a red scenario teaches people to ignore red. The retired ones live in `crates/operant-cli/tests/tui_scenarios/excluded.json` with the reason and the unblock condition; `_validate.py` checks that file against the SURFACES ledger and fails on any mismatch. Deleting a scenario file without adding the exclusion is an error. |
| live TUI repro | tmux `-L <fresh-socket>`, 110x45; `OPENAI_BASE_URL=http://localhost:20129/v1` (omp gateway → poolside/laguna); `OPERANT_TUI_DEBUG=1 OPERANT_TUI_EVENT_LOG=<path>` dumps the ring on clean `/quit`; `/tmp` is swept — recreate configs per repro |
| debug a PAST TUI run (post-mortem) | the full transcript is in `~/.operant/database.db`: `sqlite3 ~/.operant/database.db "SELECT rowid, timestamp, substr(tool_calls,1,400) FROM messages WHERE session_id='<sess_…>' AND role='assistant' AND tool_calls LIKE '%<tool>%'"` — tool-call args live in `tool_calls` JSON on assistant rows, tool RESULTS in `content` on `role='tool'` rows; `json_array_length(json_extract(tool_calls,'$'))` counts a turn's parallel calls. Validate args with python `json.loads` (raw newlines inside JSON strings are the recurring failure shape — see plan 2026-10-25 tooling-surface) |
| agent-core trace of a TUI run | tracing goes to `[logging] log_file` (the user's config points it at `~/.operant/logs/gateway.log`); UNSET it routes to a discard sink — a default-config TUI run logs NOTHING to disk (main.rs sink test). Raise verbosity with `RUST_LOG=debug` (EnvFilter). The agent loop's `warn!`s (unrepairable tool args, guardrail ladder) land there — grep the timestamp window of the session under test |

## Autonomous loop traps

One autonomous agent works `TODO.md`. These rules cover that loop and anyone who commits beside it.

| trap | rule |
|---|---|
| dirty tree | `operant autonomous` stages and commits **only the paths its own tick changed** (AD-027): `publish_changes` diffs the tree at tick start against tick end, and passes that path list to both `git add --` and `git commit` (`crates/operant-cli/src/autonomous.rs:664-704`; test `a_tick_does_not_commit_a_file_the_owner_left_dirty` at `:1698`). A file that was already dirty when the tick started is the owner's and is left alone. This row used to say it published with `git add --all .` — that was true before AD-027 and is stale now. |
| manual commit | stage explicit paths and check `git diff --cached --name-only` first: the index can hold a peer's staged deletions. If it does, commit from a temporary index: `GIT_INDEX_FILE=<tmp> git read-tree HEAD`, `git update-index --cacheinfo` for a shared file (blob = HEAD plus your lines), `git add -- <paths>`, `git write-tree`, `git commit-tree <tree> -p HEAD -F <msg>`, then `git update-ref refs/heads/main <new> <old>` |

## Documentation CLI (`./scripts/genome`)

| task | command |
|---|---|
| schema + doctrine check | `./scripts/genome lint` (exit 2 on error; `--advisory` to triage) |
| health: staleness, append-only violations, live claims | `./scripts/genome doctor` |
| wake as a parallel agent | `./scripts/genome feed --to <your-id>` then `./scripts/genome doctor` |
| claim / finish | `./scripts/genome claim <item>`; `./scripts/genome done <item> done --evidence <path>` |
| mint records | `./scripts/genome plan "<title>"`, `./scripts/genome audit "<title>"`, `./scripts/genome rec new ad <title>` / `rg <title>` — the CLI is `genome`; the old `kb` name is gone, so a doc or script citing `kb <verb>` cites a dead binary |
| append entries | `./scripts/genome log "<entry>" --agent <id>` (full text in the argument: `--body` is dropped); `./scripts/genome post "<msg>" --wf <wf> --to @<agent>` |
| supersede a doc | `./scripts/genome archive <path> --reason "<why>"` (tombstone + INDEX row; nothing is deleted) |
| regenerate state | `./scripts/genome handoff --write` |

Role→path map: `genome.toml`. Routing table (which doc answers what): AGENTS.md
"Documentation Genome (genome)".

The command table is the one a fresh agent copies. Before a docs commit, run
`./scripts/genome lint` (must be clean) and `./scripts/genome doctor` (the only
finding allowed is a stale STATE.md you are about to regenerate).
