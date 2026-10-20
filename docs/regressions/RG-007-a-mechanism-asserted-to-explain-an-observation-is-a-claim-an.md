---
ID: "RG-007"
Title: "A mechanism asserted to explain an observation is a claim, and it went into a commit message unverified"
Status: "Active"
Date: "2026-10-20"
Description: "A peer's commit swept an entire staged docs change set. The cause was asserted as a git add -A sweep in a commit message, a worklog entry and a feed post. Three candidate mechanisms were then tested and none of them is it: the pre-commit hook performs no git writes, and operant autonomous stages only tick-changed paths since AD-027. The mechanism is unknown and now recorded as unknown."
Severity: "Low"
---

# RG-007: A mechanism asserted to explain an observation is a claim, and it went into a commit message unverified

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

A peer agent's commit `988ebdb9` ("fix(iter-1076): the MCP SSE stream is no
longer cut every 300 seconds") carried an entire staged docs change set that
was mine: 11 files, four RG records, a retired record in
`docs/archive/records/`, an archived section in `docs/archive/`, the archive
INDEX, the worklog and STATE.md. Their commit body describes only the MCP fix.

I attributed it immediately, and in three places at once: a commit body, a
worklog entry, and a cross-agent feed post (F0038) to the peer. The attribution
was:

> "swept my entire staged docs change set into their commit via what looks like
> an `git add -A`" — "The mechanism is AGENTS.md rule 3, stated and then not
> followed: 'Never `git add -A`. Stage explicit paths.'"

**That mechanism was never checked.** It was the most plausible story, it fit
the observation, and it was written down as fact in three places — one of them a
commit message, which is the hardest kind of place to quietly correct.

A later pass tested it, and every candidate failed:

| candidate | result |
|---|---|
| the `pre-commit` hook stages everything | **No.** `.git/hooks/pre-commit` execs `docs-check.sh --staged --mode=warn`, and that script contains no `git add`, `update-index` or `commit-tree` — it reads staged files and warns |
| `operant autonomous` publishes with `add --all` | **No.** AD-027 changed it: `publish_changes` computes `tick_paths` and passes them to BOTH `git add --` and `git commit` (`autonomous.rs:675`, `:704`), with `a_tick_does_not_commit_a_file_the_owner_left_dirty` at `:1698` pinning exactly that |
| a git alias for `add` | **No.** `git config --get-all alias.add` is empty |

None of them is the mechanism. The observation stands (my staged work went into
their commit; their body does not mention it), and the cause is **unknown**.

## Root cause

The failure is not "I guessed wrong". It is that a guess was **promoted to
explanation by repetition** — asserted in a worklog, then in a commit body,
then in a feed post to the peer, where it became a report about their conduct
rather than a note about my own uncertainty. By the third mention it read as
established fact, and a reader of F0038 would reasonably conclude the peer had
run `git add -A` in deliberate defiance of a written rule.

The underlying process error is that the explanation was chosen for how well it
fit, before any candidate was tested. There was a plausible story, a rule in
AGENTS.md that the story implicated, and a recent instruction in the repo's own
RUNBOOK that said `operant autonomous` publishes with `git add --all .` — which
made the story feel corroborated. That RUNBOOK line is itself **stale** (AD-027
changed the behaviour it describes), so the corroboration was an artefact of a
second unverified claim.

Two claims compounded into one confident report, and neither was checked.

## Law

**An explanation is a claim until a candidate mechanism has been tested, and
"the most plausible story" is not a test.** Three rules for the next one:

1. **Enumerate the candidates before naming one.** For a git-level observation
   the set is small and checkable: the hooks, the alias, the script that did the
   commit, the loop that drove it. Read each. A mechanism that has not been
   eliminated is not the mechanism.
2. **Separate the OBSERVATION from the EXPLANATION in the sentence.** "Their
   commit contains my staged files and does not mention them" is an observation
   and is checkable. "They ran `git add -A`" is a hypothesis about intent and
   conduct. Writing the second in the first's voice is what turned a bug report
   into an accusation.
3. **When a claim is retracted, retract it in every place it was made** — the
   commit body cannot be edited, so the retraction needs a new commit that names
   the old claim; the feed needs a new post, because an append-only channel
   cannot be rewritten either; and the worklog is append-only for the same
   reason. Three surfaces, three retractions, none of them silent.

The specific trap, and the reason this is a law rather than a note: **a
mechanism that implicates another agent's conduct gets asserted faster than one
that implicates only your own tooling.** The same standard of proof applies, and
it must be applied BEFORE the sentence is written, because afterwards it is
already in their inbox.

## Detection

For a git-level claim, the candidates are enumerable and each is one command:

```bash
cat .git/hooks/pre-commit                       # does the hook write to the index?
git config --get-all alias.add                  # is `add` aliased?
rg -n 'git add|update-index|commit-tree' <the script that made the commit>
```

For the specific one here, the decisive read was `autonomous.rs:664-704`: the
`add` is `vec!["add", "--"]` extended with `tick_paths`, and `commit` is
extended with the same `tick_paths`. Reading the twenty lines that perform the
action settles it; reading the RUNBOOK's description of those lines does not —
and the RUNBOOK was the stale one.

The generalisable detector: **before asserting a cause, ask what evidence would
distinguish it from its nearest rival, and then go get that evidence.** Here the
rival was "the autonomous loop published with everything staged", and the
distinguishing evidence was three lines of `add_args` construction.

## Orthogonality

RG-001 is stale board statuses, RG-002 the immutability bypass, RG-003 an overfit detector, RG-006 a tool limit asserted from memory. None covers this: an EXPLANATION invented for a specific observed event, stated as fact in a commit body, a worklog entry and a cross-agent feed post, when it was never checked against any candidate mechanism. RG-006's defect was about a tool capability; this is about attributing a cause, and the fix differs — enumerate and test the candidates, and if none fits, say so. (recon R10202201134e9: asserted mechanism unproven never verified sweep staged work)

## Revisions

- 2026-10-20: created
