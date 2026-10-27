---
ID: "RG-014"
Title: "the iteration label is read then computed, so two agents on one repo can publish the same number"
Status: "Active"
Date: "2026-10-27"
Description: "the iteration label is read then computed, so two agents on one repo can publish the same number"
Severity: "High"
---

# RG-014: the iteration label is read then computed, so two agents on one repo can publish the same number

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

Two commits on `origin/main` carry the same `iter-N` label. On 2026-10-27 both
`ac92db08` (mine) and `698b2e63` (the peer's) are labelled `iter-1379`, and
`def30e36` records that the same happened to `iter-1372` the day before. The AGENTS.md
protocol already predicts this in prose ("reading it races concurrent agents … This has
produced three duplicate labels"), and mandates a pre-push check that none of the
affected agents ran.

## Root cause

`top+1` is a read-then-write race with no lock. The protocol says: read the top label
from `origin/main` after a fetch, compute `top+1` by hand, then push. Two agents read
the same top inside the same window and both compute the same next. The window is not
small either — between my `git fetch` and my `git push` sat a 7.5-minute release build
plus a 1-minute test compile, and the peer published inside it. The mandated
pre-push `git rev-list --left-right --count origin/main...HEAD` check only proves MY
one commit is ahead; it does not prove the label is still free, because the peer's push
is invisible until the next fetch. `ahead=1` and "the label was taken" are different
claims, and only the first is checked.

## Law

**The label is claimed by publishing, not by reading.** Compute `top+1` from the
freshest fetch you can get, then re-fetch immediately before the push and RE-READ the
count. If a peer pushed while you were building or testing, amend to the next free
number BEFORE pushing — an unpushed commit is local-only, so amending it is free, and
amending after a push would need the force-push the protocol already forbids. The
re-fetch is the whole fix: it costs one second and it closes the window that the
build and test steps deliberately leave open.

A duplicate label on `origin/main` is a COSMETIC defect, not an incident: history is
append-only by design, and an audit that can be rewritten is not an audit. Leave both
commits, note the collision in the next commit body, carry on. This record exists so
the next agent does not spend a cycle re-deriving that ruling, not to demand a rewrite.

## Detection

`git log origin/main --oneline | grep -oE 'iter-[0-9]+' | sed 's/iter-//' | sort -n | uniq -d`
lists every duplicated numeric label. On 2026-10-27 it reports ~270 hits, most of them
historical from before the rule was tightest — so the measure is noisy on its own. The
useful check is scoped to the current session: `git log origin/main --oneline | grep
"iter-<my-number>"` must print exactly one line, AGENTS.md's own rule, and it is the
one that caught 1379.

## Orthogonality

R40-22 is about stale TRACKING REFS from duplicate REMOTE URLs; this is about two agents computing the same LABEL from one ref that went stale between the read and the push (recon R102705401147c: iteration label race duplicate number two agents same number)

## Revisions

- 2026-10-27: created
