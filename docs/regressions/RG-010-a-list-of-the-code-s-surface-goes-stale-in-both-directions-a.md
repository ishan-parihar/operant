---
ID: "RG-010"
Title: "A list of the code's surface goes stale in BOTH directions, and the count in the heading is a third, independent error"
Status: "Active"
Date: "2026-10-20"
Description: "AGENTS.md's 'What IS Working' listed 53 CLI command variants. The Commands enum has 52. The list named hardware and peripheral, both deleted in iter-892, and omitted donate and org, both real commands with handlers; 'gateway (16 sub-actions)' contributed prose to the count. The heading count, the list contents, and the code disagreed three ways."
Severity: "Medium"
---

# RG-010: A list of the code's surface goes stale in BOTH directions, and the count in the heading is a third, independent error

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

AGENTS.md's "What IS Working" section opens with a **count and a list** of the
CLI's command surface:

> ### CLI (53 command variants — real handlers, except the two accepted stubs
> below)
>
> run, chat, autonomous, tools, test, config, sessions, mcp, skills, model,
> completion, cron, kanban, gateway (16 sub-actions), checkpoints, memory,
> profile, auth/login/logout, version, doctor, status, dump, logs, backup,
> import, uninstall, update, insights, webhook, debug, plugins, curator,
> setup, acp, dashboard, trajectory, architecture, channel, context, cookies,
> **hardware, hooks, migrate, pause, peripheral, resume, service, sop,
> suggestions, tui**

Three things were wrong, and they were wrong in three different directions:

1. **The count.** The `Commands` enum in `crates/operant-cli/src/main.rs` has
   **52** variants. The heading said 53.
2. **Dead entries kept.** `hardware` and `peripheral` were **deleted from the
   enum** in iter-892 (`chore(iter-892): plan 10 phase 1 — delete the 6 crates
   the binary never calls`). They have been listed here ever since.
3. **Live entries missing.** `donate` and `org` are real commands with real
   handlers (`main.rs:3345` and `:3213`) and were **not on the list at all**.

A fourth error inflated the count: `gateway (16 sub-actions)` put the prose
`sub-actions` and the number `16` into a list of single-word command names, so
the "53" was never a count of anything.

The list is not decoration. It is the **first** summary of what the binary can
do, in the file AGENTS.md makes every agent read first, and it is what an agent
consults to learn whether a capability exists before looking for it.

## Root cause

**A prose inventory of a code surface is maintained by hand, and it drifts in
both directions at once.**

Removal drift is the one people expect: a command is deleted, the list keeps
it, and the list grows a ghost. It is also the more damaging half, because a
listed command sends the agent looking for an implementation that is not there
— and the search finds a *deleted* one in git history, which is worse than
finding nothing.

Addition drift is the half nobody expects: a new command ships and the list is
never extended, so the agent concludes the capability does not exist and either
does not use it or builds it again. `org` and `donate` are the two most
consequential omissions here — `org` is the whole Holon surface, and the repo
has a large Holon subsystem.

The headline count is a **third, independent** error, and the worst of the three
in kind: it is maintained by hand, apart from the list, and it was never
checked against either. 53 agreed with nothing — not the enum (52), not the
list it sat on (53 names, two of them prose), and not the code.

And nothing in the toolchain could see any of it. `genome doctor` counts
records; `genome lint` checks the docs' schema; the pre-commit docs gate
checks formatting. A doc that says "53 commands" is perfectly well-formed
whether or not it is true.

## Law

**A list that describes a code surface is derived, not written. Where it cannot
be derived, it is a claim that ages in both directions and must be checked in
both directions.**

1. **Derive the list from the source, and say where it came from.** The `Commands`
   enum is the source; the AGENTS.md list now says so and names the file and
   the count. A list without a stated source is a list that will be trusted
   past its shelf life.
2. **Check for ADDITIONS as well as removals.** A stale list is not only
   ghosts. `donate` and `org` were missing and `org` is a whole subsystem.
   Removing a command is a visible event; adding one is invisible in a prose
   list, so additions need their own check.
3. **The count in the heading is a separate claim from the list and needs its
   own check.** Here 53 was wrong, the list was wrong, and the code was right —
   three-way disagreement. A count that is written by hand next to a list is
   the first thing to go stale and the last to be noticed, because it reads as a
   summary of the list below it.
4. **Keep the list machine-checkable.** The check in this record's Detection
   section compares the document's list to the enum and reports the difference
   in both directions. It takes seconds, and it is the difference between a
   list that is correct and a list that is believed.
5. **A prose list and a count in the same sentence need different sources.** The
   list comes from the code; the count comes from the list. When they are both
   typed by hand they are two chances to be wrong instead of one.

## Detection

The check, and it is the one that should have existed all along:

```python
import re
src = open('crates/operant-cli/src/main.rs').read()
i = src.index('enum Commands {'); depth = 0
for k in range(i + len('enum Commands {') - 1, len(src)):
    if src[k] == '{': depth += 1
    elif src[k] == '}':
        depth -= 1
        if depth == 0: end = k; break
actual = {v.lower() for v in re.findall(r'^    ([A-Z][A-Za-z0-9]*)', src[i:end], re.M)}

text = open('AGENTS.md').read()
# The list block: the first run of consecutive lines that start with a lowercase
# letter after the heading. Tolerates a blank line between heading and list.
m = re.search(r'### CLI \(\d+ command variants[^\n]*\n+(?:\n*)([a-z][^\n]*\n(?:[a-z][^\n]*\n)*)', text)
listed = set()
for tok in re.split(r'[,\s]+', m.group(1)):
    tok = tok.strip(' .()')
    if tok and re.fullmatch(r'[a-z][a-z/-]*', tok):
        for p in tok.split('/'): listed.add(p)

print("listed:", len(listed), "actual:", len(actual))
print("LISTED BUT NOT REAL:", sorted(listed - actual))
print("REAL BUT NOT LISTED:", sorted(actual - listed))
```

The list block is matched by "the first run of consecutive lines that start
with a lowercase letter", not by "up to the first blank line". The old layout
had no blank line after the heading and the new one does, so a matcher that
assumed either arrangement silently matched the **next** section instead — and
reported 27 garbage tokens as the list. A detection script that can match the
wrong block is worse than none, so the matcher is part of the check, and it was
written by running it against BOTH layouts.

Run against the state before this record: `listed: 53 actual: 52`, with
`hardware` and `peripheral` listed-but-not-real and `donate` and `org`
real-but-not-listed. Run against the state after: `listed: 52 actual: 52`, both
difference sets empty, **MATCH**. Both runs were executed, not reasoned about.

The generalisable detector, and the one this session's records keep arriving
at: **a summary of a code surface is checked by diffing it against the surface,
in both directions, and never by reading it.** Reading a 52-item list and
judging it "looks right" is exactly how two deleted commands and two missing
ones survived in the same document.

## Orthogonality

RG-008 is about a count being wrong, this is about a LIST being wrong in both directions at once — dead entries kept and live entries missing — with a headline count that agrees with neither. RG-009 is about a pointer to a location. Neither covers the specific shape: a prose inventory of a code surface, which drifts by ADDITION and by REMOVAL, and whose count is maintained by hand independently of its contents. (recon R10202355568fa: command list stale both directions removed commands still listed omits new commands)

## Revisions

- 2026-10-20: created
