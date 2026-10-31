---
ID: "RG-018"
Title: "A stash entry became a second cause of loss while recovering from the first, and my recorded root cause for it was wrong"
Status: "Active"
Date: "2026-10-31"
Description: "A stash entry became a second cause of loss while recovering from the first, and my recorded root cause for it was wrong"
Severity: "High"
---

# RG-018: A stash entry became a second cause of loss while recovering from the first

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

A `git stash push -- <paths>` run while trying to A/B a test suite did not save only the named paths. On my contemporaneous record the entry held **eight files I never passed** — `Cargo.lock`, three TUI files, `sourcehound_composites.rs`, `tool_search.rs`, `BUGS.md`, `feed.md` — and they were removed from the tree at once. **Two limits on that sentence, stated rather than glossed.** (1) The original entry was resolved and dropped, and it is no longer in `git stash list`, so the eight-file count rests on my own record of the event and NOT on a stash I can re-inspect — I cannot now re-derive it. (2) **I first wrote that the extra files entered "because they were dirty in the same tree". That mechanism is DISPROVEN by a controlled reproduction (see Root cause) — a pathspec does scope the capture.** So the symptom is that the entry held more than I passed, while the explanation for how is NOT established by this record.

What is certain, because it is mechanical rather than remembered: **nothing announced the surplus.** The command succeeded and reported nothing about the files it took beyond my list. A pathspec failure mode is detection-blind at the moment of the capture, which is why the law below is a pre-flight `git stash show --stat` and not a better command.

(That same silent-batch shape recurred later the same day when a peer commit absorbed my staged index — WORKLOG 2026-10-31T12:31. It is a different operation and is recorded there, not as evidence for this record.)

The consequences were silent and compounding. Later `git stash pop` resolution with `--ours`/`--theirs` **silently took the HEAD side** — `tests.rs` showed as `-468` lines, i.e. a peer's tests gone from the working tree while the command reported a successful pop with conflicts. A shell loop of `git show stash@{0}:$f > $f` was correctly refused by the destructive gate, for exactly the reason that made it dangerous.

Observed 2026-10-31 at 09:28, while preparing iter-1553 (recorded as RT-204 in `docs/kb/WORKLOG.md`; **the id collides with a peer's unrelated TUI RT-204 in `BUGS.md` — theirs is about a frozen rewind highlight, mine is this. Both stand, append-only.**)

## Root cause

**CORRECTED after a controlled reproduction DISPROVED the cause I first wrote here.** I originally recorded "a pathspec filters what git *considers*, not a whitelist of what it *takes* — any dirty file in the tree enters the stash." That is false. Reproduced 2026-10-31 in a throwaway repo, three ways, each time with a second dirty file present and untouched: `git stash push -- a.txt`, `git stash push -u -- a.txt`, and `git stash push -- a.txt` with `a.txt` both staged and further modified. In every case the entry contained ONLY `a.txt` and the other file survived. `git stash show --stat` reported exactly the entry's contents. So a pathspec DOES scope the capture, and the detection law below is sound precisely because of that.

What the incident actually was, by the evidence available: I created a stash with an explicit 4-path pathspec, and the entry ended up holding 8 files including `Cargo.lock`, three TUI files, `sourcehound_composites.rs`, `tool_search.rs`, `BUGS.md` and `feed.md` — none of which I passed. The proximate cause on the record is the **`genome log` failure that ran in the same sequence and exited with `error: could not write index`**, which left the index mutated; the pathspec command is what I blamed, and blaming a command that a controlled test clears is how a wrong cause gets frozen into an immutable record. **I have not isolated this further and the remaining mechanism is unverified — recorded as such rather than closed, per RG-007 (a mechanism asserted to explain an observation is a claim).**

What IS proven and still the substance of this record: once a stash entry holds more than you intended, **resolving its conflicts with `--ours`/`--theirs` silently takes one side** — measured, it took the HEAD side and reported success while a peer's `tests.rs` sat 468 lines down. The second loss was the recovery, not the capture.

## Law

**Never run a second mutating git operation while recovering from a failed one.** The loss that cost real work here was not the stash capture — it was running `git stash pop` and then resolving its conflicts with `--ours`/`--theirs`, which silently took the HEAD side and reported success while discarding a peer's tests. Fleet rule 9 (`git stash list` before any ref move) exists for exactly this moment.

The recovery that works, in order, verified on the real incident:
1. `git stash list` — identify the entry. Do not proceed without it.
2. `git stash show <ref> --stat` — **it names every file in the entry.** If it lists more than
   you passed, the entry already captured peer work. (Proven: it does scope correctly; this is
   the check that tells you the entry is wider than your intent.)
3. `git checkout stash@{0} -- <each file>` — per file, restoring that file's exact stashed blob.
4. `git md5sum`-compare every restored file against `git show stash@{0}:<file>` — **all must match**.
5. Drop the stash only after step 4 passes for every file.

**Do NOT resolve a conflicted stash pop with `--ours`/`--theirs`.** It picks a side and reports success without saying what it discarded. `git stash push -u` with an explicit path list is the form that applies cleanly — validated a second time hours later when the same agent re-ran it correctly and it applied with no conflict.

**A note on how a wrong cause gets into an immutable record.** My first version of this file
asserted a git behaviour I had not tested. Testing it took three commands and disproved it. The
correction lives here rather than in a superseding record because this record is minutes old and
nothing cites the withdrawn claim beyond its own two entries — but the lesson is AGENTS.md's
already: a mechanism asserted to explain an observation is a claim, and claims get tested before
they get numbered.

## Detection

The signature is a `git stash pop` that reports `CONFLICT (content)` on files you never named, or a post-pop `git diff --numstat` showing a large negative count on a peer's file. Both were visible immediately and both were nearly missed.

Cheap pre-flight, before touching a stash at all: `git stash show <ref> --stat` names every file in the entry. If it lists more than the paths you passed, the entry already captured peer work — stop and read those files instead of moving them.
