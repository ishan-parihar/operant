# 008 — Parity: `working_diff` tool (hermes `tools/working_diff.py`)

Stamped: `d394c136`. Priority: **P1** (core — user-designated).

## Why

Operant has no way for the agent to see the current uncommitted working diff; hermes
exposes `working_diff.py` (`collect_working_diff(cwd, mode="working")`) — a git-based
diff collector with sane caps. This is core agent capability (lets the model see and
reason about in-progress edits).

## Hermes reference (hermes-agent/tools/working_diff.py, 130 LOC)

- `collect_working_diff(cwd, mode="working")` — returns the diff of the working tree
  (`git diff`) or staged (`git diff --cached`) per mode.
- `_MAX_UNTRACKED_FILES = 50` sanity cap so a `node_modules` explosion can't hang the
  tool; `_untracked_diff` lists up to 50 untracked files + `"... (N more untracked files
  not shown)"`.
- `_run(args, cwd, timeout=_GIT_TIMEOUT)` — bounded git subprocess calls.

## Files in scope

- New `crates/operant-core/src/tools/working_diff_tool.rs`
- `crates/operant-core/src/tools.rs` (mod decl) + `crates/operant-core/src/tools/builtin.rs`
  (register `WorkingDiffTool`)

## Files out of scope

- `patch`/`file_edit` tools (different surface). No changes to existing file tools.

## Steps

1. Implement `WorkingDiffTool` following the existing tool pattern (see
   `todo_tool.rs` or `checkpoint_tool.rs` for the `OperantTool` trait shape + how
   `builtin.rs` registers tools).
2. Action `collect` with args: `mode` (`"working"` | `"staged"`, default `"working"`),
   optional `path` (repo root; default cwd). Run `git diff` / `git diff --cached` via a
   bounded child process. **Reuse the existing git-invocation pattern in
   `checkpoint_tool.rs`** (it already runs bounded `git checkout`/`add`/`rev-parse`
   subprocesses with timeouts) rather than inventing a new one — extract or mirror it.
   Cap output at ~100KB with a truncation marker.
3. If `git rev-parse --is-inside-work-tree` fails → honest tool error ("not a git
   repository").
4. Untracked list: `git status --porcelain` + cap at 50 files with the "…N more"
   suffix. Never follow into `node_modules`/`.git` (only list, don't walk).
5. Register in `builtin.rs`; add to the tool registry test if there is a
   registered-tools assertion.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-core --all-targets -- -D warnings
cargo test -p operant-core --lib working_diff
cargo test --workspace --all-features --lib          # final gate
```
Live smoke (deployed binary): `operant run -q 'Call working_diff and summarize what changed'`
in a dirty temp repo returns the actual diff.

## Test plan

- `working_diff_shows_modified_lines`: temp repo, edit a file, assert diff contains the
  change.
- `working_diff_staged_vs_working`: staged vs unstaged modes return the correct halves.
- `untracked_cap_50`: create 60 untracked files → output lists 50 + "... (10 more...)".
- `not_a_git_repo_errors`: non-repo dir → tool error mentioning "not a git repository".
- `diff_output_truncated`: huge diff (e.g. 200KB generated file) → bounded output with
  truncation marker.

## Maintenance note

- `_MAX_UNTRACKED_FILES=50` and any new caps must be consts at the top of the file with
  the hermes reference comment (matches repo convention).
- The tool must not run in `~/.operant` (not a repo) without a clear error — the agent
  should get a path hint.

## Escape hatches

- If a `git diff --no-index` fallback is needed for non-repo dirs, keep it out of scope
  (hermes does not do it) — error instead.
