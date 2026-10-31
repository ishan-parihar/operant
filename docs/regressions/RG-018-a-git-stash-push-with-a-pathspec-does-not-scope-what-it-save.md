---
ID: "RG-018"
Title: "A git stash push with a pathspec does not scope what it saves"
Status: "Active"
Date: "2026-10-31"
Description: "A git stash push with a pathspec does not scope what it saves"
Severity: "High"
---

# RG-018: A git stash push with a pathspec does not scope what it saves

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

A `git stash push -- <paths>` run while trying to A/B a test suite did not save only the named paths. It swept **eight peer files I never listed** — `Cargo.lock`, three TUI files, `sourcehound_composites.rs`, `tool_search.rs`, `BUGS.md`, `feed.md` — because they were dirty in the same working tree, and removed all of them from the tree at once. Nothing announced this: the command succeeded and reported nothing about the files it took beyond my list.

The consequences were silent and compounding. Later `git stash pop` resolution with `--ours`/`--theirs` **silently took the HEAD side** — `tests.rs` showed as `-468` lines, i.e. a peer's tests gone from the working tree while the command reported a successful pop with conflicts. A shell loop of `git show stash@{0}:$f > $f` was correctly refused by the destructive gate, for exactly the reason that made it dangerous.

Observed 2026-10-31 (iter-1552 window, recorded as RT-204 in `docs/kb/WORKLOG.md`; **the id collides with a peer's unrelated TUI RT-204 in `BUGS.md` — theirs is about a frozen rewind highlight, mine is this. Both stand, append-only.**)

## Root cause

The pathspec is a filter over what git *considers*, not a whitelist of what it *takes*. Any dirty file in the tree enters the stash. The command's success message therefore does not enumerate its effect, so the operator believes they saved N paths when they captured M.

The damage is not the stash itself — it is that the stash is a single opaque entry. `git stash pop` then conflicts on files whose HEAD and stashed sides differ for reasons unrelated to the operator's intent, and the two-way resolution picks a side without telling the operator it discarded work. An agent's "restore" instinct (`--theirs` to keep changes) takes whichever side git labels that, which need not be the peer's work.

## Law

**Never `git stash push` when the tree holds a peer's WIP you cannot re-derive.** Before any stash: `git stash list` first (fleet rule 9 exists for exactly this moment), then `git status --porcelain`, and prefer reading the files you need over stashing. The fleet protocol already forbids destroying peer WIP; this is the concrete way it happens by accident.

If a stash has already captured more than intended, the recovery path that works, in order:
1. `git stash list` — identify the entry, do not proceed without it.
2. `git checkout stash@{0} -- <each file>` — per file, restoring that file's exact stashed blob.
3. md5sum every restored file against `git show stash@{0}:<file>` — **all of them must match**.
4. Drop the stash only after step 3 passes for every file.

**Do NOT resolve a conflicted stash pop with `--ours`/`--theirs`.** It silently takes one side and reports success; measured, it took the HEAD side and showed a peer's `tests.rs` at `-468` lines. `git stash push -u` with an explicit path list is the form that applies cleanly — validated a second time when the same agent re-ran it correctly hours later and it applied with no conflict.

## Detection

The signature is a `git stash pop` that reports `CONFLICT (content)` on files the operator never named, or a post-pop `git diff --numstat` showing a large negative count on a peer's file. Both were visible immediately and both were nearly missed.

Cheap pre-flight: `git stash show <ref> --stat` names every file in the entry. If it lists more than the paths you passed, the stash already captured peer work.

## Orthogonality

RG-015 governs attributing a red tree to a peer's uncommitted edit; this record governs the act that CREATES the damage - my own stash pathspec sweeping in peer WIP I never named. RG-017 governs a cached green summary masking red. None of them describes unscoped stash capture, its recovery path, or why the recovery nearly failed. (recon R1031114821222: stash wip peer sweep pathspec)

## Revisions

- 2026-10-31: created
