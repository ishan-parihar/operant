# The operator's tree, rebased and deployed — 2026-11-15

**Date:** 2026-11-15, 13:27 IST
**Asked:** "Proceed with rebasing effectively, ensuring that all conflicts are
resolved effectively, and that the binary compiles effectively. When everything
is finalized we would be cleaning up the redundant worktrees, and targets."
**Base:** `origin/main` at `f24f485e`.

Everything below was measured. Nothing was reverted, and the pre-state is
recoverable (see the last section).

---

## 1. What the "rebase" actually turned out to be

The backlog survey said the operator's tree was 111 behind and 8 ahead, with 42
dirty files. Before touching anything I classified all 8 commits.

**All 8 commits are already on `origin/main`.**

| commit | subject | how it is upstream |
|---|---|---|
| `32fad6a1` | docs(iter-2372) | identical patch-id → `5ffe1a38` |
| `2b869c4a` | fix(iter-2373) | identical patch-id → `ef85100d` |
| `b205591c` | fix(iter-2381) | re-landed by a peer as `d41a91a6` (same subject) |
| `b8f9883b` | fix(iter-2383) | re-landed as `b2e12f6c` (same subject) |
| `ebacaa66` | feat(iter-2388) | identical patch-id → `d403f52b` |
| `ace25ec3` | fix(iter-2389) | re-landed as `a95cc013` |
| `10357edc` | fix(iter-2390) | re-landed as `fdc0fa0c` |
| `61b17f4a` | docs(iter-2419) | identical patch-id → `2dc005fe` |

`crates/operant-cli/src/tui/model_effort.rs` is **byte-identical** between the
operator's HEAD and `origin/main`. So there was nothing to rebase: the correct
operation was a fast-forward, not a rebase. A `git rebase --onto` probe (run on
a throwaway branch, then aborted) confirmed the rebase would have produced
spurious conflicts **on already-landed commits**.

**The 42 dirty files were the only genuinely unlanded work** — and they were
real: `attaching_as`, `is_mcp_tool`, `pid_is_gateway` and
`create_runtime_agent_with_client` are absent from `origin/main`.

## 2. What was done

```
git branch fleet-hold-operator-tree-snapshot-20261115-1729 HEAD   # backup
git stash push -u -m "operator-tree-before-rebase-..."            # 42 files
git reset --hard origin/main                                      # 0 behind, 0 ahead
git stash pop                                                     # restored
```

`stash pop` applied with **no conflicts at all** — the 8 commits were the
conflict source, and they were already upstream. The pre-state is on branch
`fleet-hold-operator-tree-snapshot-20261115-1729` and in `stash@{0}`.

## 3. Conflicts that did have to be resolved

Six real conflicts appeared after the pop, because the stash was written against
an older tree:

| file | resolution |
|---|---|
| `operant-core/src/agent/run.rs` | upstream's `recall_fit_budget(budget)` helper supersedes the stash's inline `saturating_sub` |
| `operant-core/src/tools/builtin.rs` (×2) | upstream's `skills_autoload` gate wins; second hunk unioned to keep `mcp_manager.clone()` + `skills_autoload` |
| `operant-core/src/tools/sub_agent_tool.rs` | kept the stash's two `with_mcp_manager*` builders (needed by `builtin.rs`); dropped the stash's `new()`, which upstream deliberately deleted (third-pass audit finding 6) |
| `operant-cli/src/gateway_runner.rs` | upstream's full `/reload-skills` arm with the autoload gate; the stash's duplicate half dropped |
| `operant-cli/src/gateway_runner/tests.rs` | **both** sides kept — upstream added skill-root tests, the stash added drain tests. Also repaired a truncated `assert!` the split left behind |

## 4. Two source bugs the restore exposed

Neither is a conflict; both are the operator's own in-progress work meeting a
newer upstream.

1. **`tui/theme_screen.rs:740`** — `buf.cell((x+k, y))` mixed `u16` and `usize`
   in the `probe_how_many_themes_fit` test. Fixed to `x + k as u16`.
2. **`operant-core/src/schema.rs`** — `every_registry_publish_variant_attaches_the_intent`
   text-scans `get_schemas_for_request`'s body for `get_schemas().await`. The
   iter-2366 refactor made that function **delegate** to
   `get_schemas_for_request_with_catalog`, so the scan read "the intent
   augmentation is gone" while it is one hop down. The test now follows the
   delegation. The wiring is intact: `_with_catalog` calls `get_schemas().await`.

## 5. Verification

| check | result |
|---|---|
| `cargo check -p operant-core --lib` | clean |
| `cargo check -p operant-cli --bin operant` | clean |
| `cargo test -p operant-cli --bin operant` | 2746 passed, 8 failed |
| **vs pristine `origin/main`** | **identical failure set — zero new** |
| `cargo test -p operant-core --lib` | 3477 passed, 4 failed |
| **vs pristine `origin/main`** | **zero new** (the 3 that differed pass in isolation: network/port contention) |
| conflict markers remaining | 0 |

The 8 CLI failures and the core failures are all pre-existing peer-file
failures (`cmd_channel`, `dashboard_server`, `gateway_runner`, `advisor_card`,
`commands`, `session_hub`, Slack/HTTP/MCP network tests). They reproduce
identically on pristine `origin/main` in my own clean worktree.

## 6. Deployed, and the cursor fix is live

```
scripts/build.sh --release   → 13m 29s, finished, artifact mtime 18:38
cp target/release/operant ~/.local/bin/operant.new && mv ... ~/.local/bin/operant
systemctl --user restart operant-gateway
```

The deployed binary is **byte-identical** to the built artifact (`e0a0fcb3…`),
and the gateway restarted 9 s after the install.

**Behavioral proof on the deployed binary, in tmux, 80×24:**

| case | caret before → after Up | verdict |
|---|---|---|
| multi-line draft (`line one` / `line two`) | 17 → 16 | **walks** |
| wrapped 90-char line (2 painted rows) | 17 → 16 | **walks** |
| Down after Up | 16 → 17 | **walks** |
| Up at the top row | recalls history | **correct** |

The composer-width fix (iter-2401) is present (`composer_text_width` = 1) and
the old `saturating_sub(4)` is gone (0 occurrences).

## 7. Cleanup

**Worktrees: 29 → 5.** Removed 22 that were verified, one at a time, to be
clean, an ancestor of `origin/main`, and carrying **zero unique commits**. Each
was checked again immediately before removal (AGENTS.md rule 11).

Kept: the operator's tree (40 restored files), `.wt-skillroots` (39 dirty),
`.wt-tui-fix` (16 dirty), `wt-ctx2` (5 dirty), and my own `~/rt4`.

`git worktree remove` refuses any tree whose repo has a `vendor/prime-agent`
submodule in `.gitmodules`, even after `submodule deinit`. `--force` removes
them; that is what was used, after the per-tree safety check.

**Disk: 63 G → 85 G free (94% → 91%).** My own `~/rt4/target` (3.6 G) is gone.

**Not reclaimed, deliberately:** the operator tree's `target/debug` is **128 G**,
and **two other agents have live TUI sessions running that debug binary**
(tmux sockets `fix21968448`, `fix33714900`, both `operant chat`). Removing it
would break their sessions. AGENTS.md rule 5 also forbids a blind
`rm -rf target/`. That 128 G is the operator's call, once those sessions end.
The `target/release` (36 G) is redundant — the deployed binary is a separate
copy — and can go too.

## 8. Recovery

```
git -C ~/Documents/github/my-projects/agentic-harness/operant reset --hard \
    fleet-hold-operator-tree-snapshot-20261115-1729
git -C ~/Documents/github/my-projects/agentic-harness/operant stash apply stash@{0}
```

`stash@{0}` was kept by the pop (it stays until explicitly dropped), and the
snapshot branch is a plain branch, so nothing here is one-way.