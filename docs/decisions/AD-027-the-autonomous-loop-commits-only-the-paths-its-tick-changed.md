---
ID: "AD-027"
Title: "The autonomous loop commits only the paths its tick changed"
Status: "Active"
Date: "2026-10-19"
Description: "The autonomous publish stages and commits only the paths this tick made dirty, so a file the owner left uncommitted is not published"
---

# AD-027: The autonomous loop commits only the paths its tick changed

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The TODO Review list recorded that `publish_changes` ran `git add --all .`. Any uncommitted file in the checkout (a BUGS row being edited, `Cargo.lock`, the user's `xd:/` data) was committed to `[autonomous] git_branch`.

## Decision

1. `publish_changes` takes the status at the start of the tick (`snapshot.worktree_status`) and the status at its end, and stages the paths that are dirty at the end and were not dirty at the start.
2. The add and the commit both name those paths: `git add -- <paths>` and `git commit -- <paths>`. A commit that names paths takes only them, so anything the owner staged by hand is not committed.
3. A rename names both sides, so the removal of the old path is committed with the new file.
4. When the tick changed no path, the publish is skipped with the same outcome as "nothing to commit".

Options not taken: keep `add --all` and require a clean tree before the loop starts (the owner's own working tree would then never run the loop); record the paths the agent's file tools wrote (more plumbing, and a command can change files too).

## Consequences

- A file that was already dirty when the tick started is not published, even if the agent edited it in this tick. The publish reports nothing for that file. The owner decides how to commit it.
- A path that git quotes in `--short` output (non-ASCII or control characters) makes `git add` fail, so the publish fails. It does not commit the wrong set.
- Tests: `a_tick_does_not_commit_a_file_the_owner_left_dirty` (the owner's untracked draft stays out of the commit); `status_paths_names_both_sides_of_a_rename`. The two fake-executor tests now expect the status read first.

## Orthogonality

AD-027 changes what is published. The status-file exclusion in `filter_status_output` is unchanged.

## Revisions

- 2026-10-19: created
- 2026-10-19: the integration test `success_tick_auto_commits_after_a_worktree_diff` (`crates/operant-cli/tests/autonomous_ticks.rs`) pinned the old `add --all` arguments and the old call order. It now expects the status read before the add and `add -- src/lib.rs`. The contract change is this record's decision; the test was not weakened.

