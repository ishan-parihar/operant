# The rebasing backlog — every worktree, measured

**Date:** 2026-11-15
**Asked:** "investigate all the various worktrees, including this one, and
investigate what kind of rebasing backlog do we have."
**Base:** `origin/main` at `b4528fd0` (iter-2421).
**Method:** `git worktree list` (31 worktrees), per-worktree `rev-list
--count HEAD..origin/main` and `origin/main..HEAD`, `merge-base
--is-ancestor` to decide LANDED vs NOT-LANDED, and `git diff` per tree to
separate real uncommitted work from build noise. Read-only throughout: no
worktree was modified, no branch moved, nothing pruned.

> **AGENTS.md has no `wt` command.** The protocol references a worktree
> discipline in prose (rules 4, 10, 11) but there is no `scripts/wt` script
> and no `wt` binary in the tree — `ls scripts/ | grep wt` finds only
> `workspace_lint.py`. The checks below were run by hand. That absence is
> itself finding 4.

---

## 1. The headline

**31 worktrees. 24 are stale-identical to `origin/main` and carry no unique
work at all. 7 carry something. 2 carry something that is not on the remote
and is not recoverable from anywhere else.**

| dirty (tracked) | HEAD landed? | count | what it means |
|---|---|---|---|
| 0 | yes | **22** | work landed, tree clean — dead weight, safe to remove |
| 1 | yes | 1 | landed commits + 1 uncommitted file |
| 3 | yes | 2 | landed commits + uncommitted leftovers |
| 4, 5, 15, 16, 39 | yes | 5 | landed commits + uncommitted leftovers |
| 42 | **no** | 1 | the operator's main tree — real unlanded work |
| 0 | **no** | 1 | `~/w2418` — 1 unique unlanded commit |

(That is the raw `sort | uniq -c` of `dirty-count, is-ancestor` over all 31
worktrees; it is the table the rest of this document reasons from.)

The 26 landed-but-dirty-and-behind trees are **not** a rebasing backlog. They
are a *storage* backlog, and at 181 GB of `target/` on a disk at 94% they are
the more urgent of the two.

## 2. The rebasing backlog, exactly

Only two worktrees hold work that a rebase would have to reconcile:

### 2a. `~/Documents/github/my-projects/agentic-harness/operant` — the operator's tree

| | |
|---|---|
| HEAD | `61b17f4a` |
| behind `origin/main` | **91** |
| ahead | **8** |
| uncommitted (tracked) | **42 files, +2489 / −951** |

The 8 unpushed commits are the model/effort grammar series (`iter-2372` …
`iter-2419`). The 42 dirty files include `iter-2366`'s `--attach-as` work,
which `origin/main` does **not** contain (`git show origin/main:…/main.rs |
grep -c attach_as` = 0). So this tree is not merely behind — it holds real
unlanded feature work in both its commits and its working tree.

**Rebase cost, measured with `git merge-tree --write-tree` (read-only, writes
nothing):** the rebase conflicts in **four** files — `commands.rs`,
`key_handling.rs`, `providers.rs`, `tests.rs`. All four come from the 8
commits; none of them is the composer width.

**But the working tree is the real hazard.** 24 of the 42 dirty files are
also changed by the incoming 91 commits:

```
$ comm -12 <(git diff --name-only | sort) <(git diff --name-only HEAD origin/main | sort) | wc -l
24
```

That is 24 files where uncommitted edits and incoming history touch the same
code. A rebase will refuse, and `git checkout -- <path>` would destroy the
work. Per AGENTS.md rule 11 the tree must be preserved first
(`git stash push -u`, never a bare pathspec list), and per rule 2 the operator's
own call.

**This is why the cursor fix appeared not to work** (iter-2421): that tree is
91 behind, still carries `area.width.saturating_sub(4)` in six places, and the
binary the operator runs was built from it.

### 2b. `~/w2418` — the only other unlanded commit

| | |
|---|---|
| HEAD | `85808073` (detached) |
| behind | 0 |
| ahead | **1** |
| commit | `fix(iter-2422): one retention rule instead of four…` |

`iter-2422` is not on `origin/main` (`git log origin/main | grep -c iter-2422`
= 0). One file, `async_delegation.rs`, +124/−42. **This is a genuine unlanded
fix that exists in exactly one place on the box.** It is not in any other
worktree, not on any branch, not in a stash (`git stash list` = 0).

## 3. The storage backlog (bigger than the rebase one)

`df -h /` → **839G used of 910G, 63G free, 94%**.

| worktree | `target/` |
|---|---|
| `~/Documents/…/operant` (main) | **160 G** (`target/debug` 125G, `target/release` 34G) |
| `~/wskill` | 8.9 G |
| `~/rt4` | 7.3 G |
| `~/wtc-arch` | 2.9 G |
| `~/wtlive` | 2.3 G |
| `…/operant/.wt2203` | 574 M |
| **total across all worktrees** | **181.5 G** |

22 worktrees have no `target/` at all, which means the next build in any of
them pays a full cold compile. The 160 G in the main tree is the single
largest reclaimable object on the box — and AGENTS.md rule 5 forbids a blind
`rm -rf target/`, so it needs the operator's call too.

## 4. Dead `fleet-hold-*` branches — 30 of them, 54 unlanded commits total

There are **32 local-only branches** that are not ancestors of `origin/main`,
holding **54 unlanded commits** between them. **30 of the 32 are `fleet-hold-*`
or `fleet-*` checkpoints**; most are abandoned:

| branch | last commit | behind main |
|---|---|---|
| `fleet-hold-597.6-wip` | 2026-10-04 | 2151 |
| `fleet-hold-597.5-wip` | 2026-10-04 | 2162 |
| `fleet-hold-amend-518ef4c9` | 2026-10-06 | 2109 |
| `fleet-hold-peer-iter649-wip` | 2026-10-06 | 2111 |
| `fleet-hold-notif-purge-wip` | 2026-10-10 | 1951 |
| `fleet-hold-rt177-*` (2) | 2026-10-25 | 1333 |
| `fleet-hold-*` (11 more) | 2026-10-30 … 11-04 | 829–1083 |

A branch 2,151 commits behind main is a museum piece. Of the 32, exactly one
(`fleet-hold-launch-defects-wip`, 7 commits, 91 behind, 2026-11-15) is recent
enough to be live work — and it is the same series as the operator's 8
unpushed commits, so it is likely a duplicate of 2a.

**16 branches are checked out in a worktree and cannot be deleted or rebased
without removing the worktree first** (measured: 16 `refs/heads/` in `git
worktree list --porcelain`). The other 16 are reviewable or prunable
independently.

## 5. AGENTS.md's `wt` protocol is prose-only, and that is the gap

AGENTS.md rule 4 says "use a TEMPORARY worktree"; rule 11 says "before removing
any `wt-*` directory, `git worktree list`, then per worktree `git -C <wt>
status --porcelain`". Both are instructions to a human or an agent, with no
tooling behind them. Nothing in this repo:

- lists which worktrees are stale,
- warns that a worktree is N commits behind before you build in it,
- refuses a build or a deploy from a tree that does not contain the commit,
- checks that the deployed binary was built from a tree containing it.

That is precisely how iter-2421 happened: every gate passed, and the binary was
built from a tree 91 commits behind.

## 6. What I did not do

Nothing was modified. No worktree removed, no branch deleted, no stash, no
`target/` cleaned, no rebase run on another agent's tree. The operator's tree
holds 42 uncommitted files and 8 unpushed commits; resolving those conflicts is
theirs to make, and doing it unasked is how a peer's work gets eaten (AGENTS.md
rule 2, rule 11).

## 7. What is worth doing, in order

1. **Nothing destructive to the operator's tree.** 42 uncommitted files + 91
   behind is a hand-merge, not a rebase. It needs the operator's decision on
   what the 24 overlapping files should say.
2. **`~/w2418`'s `iter-2422` is the only unlanded fix with no other copy.**
   Cherry-pick it onto `origin/main` from a clean tree, or it dies with that
   directory.
3. **Reclaim disk before the box fills.** 63G free, 94% used, 181G of it
   worktree `target/`. The main tree's 160G is the target; rule 5 forbids a
   blind sweep, so it is a judgement call.
4. **Delete the 22 landed worktrees** (work is in `origin/main`, trees are
   clean, nothing unique). That is the safest large win and rule 11 permits it
   after the per-tree check this audit already did.
5. **Review the 32 `fleet-hold-*` branches.** Most are older than 800 commits
   and are abandoned; `fleet-hold-launch-defects-wip` needs a look because it
   is a day old.
6. **Add the `wt` tooling AGENTS.md assumes.** A `scripts/wt status` that
   prints the table in section 2, and a build/deploy guard that refuses a tree
   which does not contain the commit being deployed. Section 5 is the root
   cause of this whole investigation.