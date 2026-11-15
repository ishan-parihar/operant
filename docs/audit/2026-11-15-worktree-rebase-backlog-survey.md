# The worktree + rebasing backlog — re-measured 3.5 h later

**Date:** 2026-11-15, 11:36 UTC (supersedes the 08:12 survey)
**Base:** `origin/main` at `99abfd3e` (`fix(iter-2439)`).
**Method:** read-only. `git worktree list --porcelain`, per-tree
`rev-list --count HEAD..origin/main` and `origin/main..HEAD`,
`merge-base --is-ancestor` for LANDED, `git diff`/`status --porcelain` for
real uncommitted work, `git merge-tree --write-tree` for the rebase cost
(nothing written). No worktree modified, no branch moved, nothing pruned.

## What changed since 08:12 — the headline is better

| | 08:12 | 11:36 |
|---|---|---|
| registered worktrees | 31 | **29** |
| clean, landed, nothing unique | 22 | **25** |
| worktrees holding unlanded work | 2 | **1** |
| unlanded commit with no other copy | 1 (`~/w2418`) | **0** |
| `target/` across all worktrees | 181.5 G | 176.4 G |
| disk free | 63 G (94% used) | 78 G (92% used) |
| operator's tree behind `origin/main` | 91 | **111** |

**The unique fix was NOT lost.** `~/w2418`'s commit `85808073`
(`fix(iter-2422): one retention rule instead of four`) was re-landed as
`9dc11c2c` / **`fix(iter-2424)`** with an **identical patch**
(`git patch-id --stable` = `e8be4c2b43768d24` on both). The worktree was
deleted and the work survived under the next free number. That is the system
working: the deletion was safe *because* the content had already landed.

## 1. The class breakdown, exactly (all 29)

`sort | uniq -c` of `(dirty-count, is-ancestor)`:

| dirty | landed? | count | meaning |
|---|---|---|---|
| 0 | yes | **25** | work landed, tree clean, nothing unique — pure dead weight |
| 5 | yes | 1 | `~/wt-ctx2` — landed commits + 141 uncommitted lines |
| 16 | yes | 1 | `…/operant/.wt-tui-fix` — landed commits + 205 uncommitted lines |
| 39 | yes | 1 | `…/operant/.wt-skillroots` — landed commits + 1052 uncommitted lines |
| 42 | **no** | 1 | the operator's main tree |

Verified: **all 25 clean landed trees hold zero unique commits**
(`rev-list --count origin/main..HEAD` = 0 for each). They are safe to remove.

## 2. The rebasing backlog — one tree, not two

### 2a. The operator's tree `~/Documents/github/my-projects/agentic-harness/operant`

| | 08:12 | 11:36 |
|---|---|---|
| HEAD | `61b17f4a` | `61b17f4a` (unchanged — it has not pulled) |
| behind | 91 | **111** |
| ahead | 8 | 8 |
| dirty | 42 files | 42 files (+2489/−951) |
| files where dirty ∩ incoming | 24 | **26** |
| `merge-tree` conflicts | 4 | **8** |

The rebase cost has **doubled in 3.5 hours** because the operator's tree is
not moving while `origin/main` is:

```
CONFLICT: tui/app/commands.rs
CONFLICT: tui/app/key_handling.rs
CONFLICT: tui/app/providers.rs
CONFLICT: tui/app/tests.rs
CONFLICT: tui/mod.rs
CONFLICT: tui/session_hub/render.rs
CONFLICT: tui/session_hub/tests.rs
CONFLICT (add/add): docs/plans/plan-2026-11-15-models-and-settings-panels.md
```

26 of the 42 dirty files are also touched by the incoming 111 commits. A
rebase will refuse; `git checkout -- <path>` would destroy the work. Per
AGENTS.md rule 11 the tree must be preserved first (`git stash push -u`, never
a bare pathspec list), and per rule 2 it is the operator's call.

**This is still why the cursor fix appeared not to work** (iter-2421). That
tree is 111 behind and carries `area.width.saturating_sub(4)` in six places.

### 2b. Nothing else

No other worktree holds unlanded work. The 25 clean trees are landed, the 3
dirty-but-landed trees hold only uncommitted leftovers from already-landed
commits, and the one unique commit that existed at 08:12 has landed.

## 3. Storage — still the bigger number

`df -h /` → **824G of 910G, 78G free, 92%** (was 94%).

| worktree | `target/` |
|---|---|
| `~/Documents/…/operant` (main) | **161 G** |
| `~/wskill` | 9.9 G |
| `~/wtc-arch` | 2.9 G |
| `~/wtlive` | 2.3 G |
| `…/operant/.wt2203` | 574 M |
| **total** | **176.4 G** |

25 of 29 worktrees have **no `target/` at all** — the next build in any of them
pays a full cold compile. The main tree's 161 G is the single largest
reclaimable object. AGENTS.md rule 5 forbids a blind `rm -rf target/`.

## 4. 32 local-only branches, 54 unlanded commits, 30 abandoned

**32 local-only branches** are not ancestors of `origin/main`, holding
**54 unlanded commits** between them. 30 are `fleet-hold-*` / `fleet-*`
checkpoints.

Two are **pure duplicates of the operator's 8 commits** — same SHAs, verified
with `cat-file -e`:

| branch | commits | verdict |
|---|---|---|
| `fleet-hold-launch-defects-wip` | 7 | all 7 SHAs already in the operator's tree |
| `fleet/iter-2373-m1` | 2 (of 3) | all 3 SHAs already in the operator's tree |

Deleting those two loses nothing; they are the operator's work held under two
other names. The remaining 28 are the abandoned tail, oldest 2,182 commits
behind (`fleet-hold-597.5-wip`, 2026-10-04).

**14 worktrees sit on local-only branches** (immovable until the worktree goes)
— and all 14 of those branches are **fully landed** (`ahead=0`), so removing the
worktree and the branch together loses nothing:

```
fix/iter-2196-ponytail-posture-parity   feat/iter-2096-process-arch
feat/iter-2366-bar-styles               fix/launch-screen
fix-audit                               audit-swarm
fix/iter-2075-agentic-core              fleet/iter-2240-config-backup
fix/ctx-provider-window                 fleet-paste-2159
fix/iter-2172-redteam-fixes             feat/iter-2405-theming-completion
fix/ctx-architecture-accounting         fix/iter-2137-poke-retry
```

## 5. This worktree (`~/rt4`)

Clean, 0 dirty, HEAD `c7947b10` (`docs(iter-2423)`), **19 behind** `origin/main`,
0 ahead, `target/` 7.3 G → now gone from the list (0). Nothing unique here; it
is the survey's own base and is safe to rebase or drop.

## 6. The tooling gap is unchanged

AGENTS.md rules 4 and 11 still describe the discipline in prose only:
`command -v wt` = none, `scripts/` has no `wt`, and `git worktree prune
--dry-run` reports nothing stale (the admin metadata is consistent). Nothing in
this repo lists stale worktrees, warns that a tree is N commits behind before
you build in it, refuses a build or deploy from a tree that does not contain the
commit, or checks which tree the deployed binary came from.

The 3.5-hour delta makes the cost concrete: **the operator's tree went from 4
conflicts to 8 without anyone touching it.** A `scripts/wt status` that printed
this table would have shown that on every wake.

## 7. What is worth doing, in order

1. **The operator's tree is the whole backlog.** 42 dirty files, 26 overlapping
   the incoming 111, 8 predicted conflicts. It is a hand-merge, not a rebase,
   and the decision on those 26 files is the operator's.
2. **Delete the 25 clean landed worktrees** — nothing unique in any of them
   (verified). Biggest safe win; AGENTS.md rule 11 permits it after the per-tree
   check this survey did.
3. **Delete the two duplicate branches** (`fleet-hold-launch-defects-wip`,
   `fleet/iter-2373-m1`) — every commit is already in the operator's tree.
4. **Delete the 14 landed local-only branches with their worktrees** — all
   `ahead=0`.
5. **Reclaim the main tree's 161 G `target/`** before the disk tightens again
   (92%, 78 G free). Rule 5 forbids a blind sweep.
6. **Add the `wt` tooling** — `scripts/wt status` printing this table, plus a
   deploy guard that refuses a tree not containing the commit. Section 6 is the
   root cause of iter-2421.