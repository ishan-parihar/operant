---
ID: "RG-023"
Title: "a library contract no gate runs is a contract nobody has"
Status: "Active"
Date: "2026-11-17"
Description: "a library contract no gate runs is a contract nobody has"
Severity: "High"
---

# RG-023: a library contract no gate runs is a contract nobody has

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

`scripts/verify-library.sh` was written, worked, and then sat unrun. It was wired
into no gate, so a library page could lose its `Source:` header, drop its
`Owns:` / `Not for:` boundary, break a sibling link, or put a tool name in a
heading that no module registers, and nothing would say so. The same trap is
recorded in `lint_doc_claims`' own docstring: "right and UNRUN for weeks".

The version of the script that first shipped was itself wrong twice over. Its
tool-name check used a hand-written allowlist that contained four names no module
registers (`sub_agent`, `learning_mutation`, `home_assistant`,
`transcription`), so it validated the wrong answer (RG-022's failure again, one
layer down). When that allowlist was replaced by the derived registry, the check
scanned every backticked token and reported config keys and struct fields,
which is a different question with a useless answer.

## Root cause

A check that is not in a gate is documentation of a check. Nothing about writing
the script improved the library; only running it did, and nothing ran it.

The second failure is the sharper one: the allowlist encoded the ANSWER rather
than deriving it. A hand-maintained list of valid tool names ages exactly like
the prose it was meant to police.

## Law

A contract the gate does not run is not a contract. Wire the script that already
implements the checks into the gate rather than reimplementing its derivations,
so the tool a human runs and the tool CI runs cannot drift apart. Derive the
registry from the code; never transcribe it.

And the tool-name check must answer a QUESTION, not match a PATTERN. A heading
that names a tool asserts the tool exists and this page owns its contract; a
backticked config value does not. The check is scoped to headings, and a
config-value table (also derived) keeps a heading like "when its inject mode is
`always_on`" from being read as a tool claim.

## Detection

E020 in `genome lint` runs `scripts/verify-library.sh` (named by
`ops.library_pages` in `genome.toml`) and fails on `NO-SOURCE`, `NO-OWNS`,
`NO-NOTFOR`, `NO-INTENTION`, `UNKNOWN-TOOL` or `DEAD-LINK`.

Two properties make it non-vacuous:

- The registry is derived (`name()` impls, `TOOL_*_NAME` consts,
  `NO_EFFECT_TOOL_NAMES`, the alias table, and the config-value table), so a new
  tool gates on arrival and a retired one stops passing immediately.
- A signal the script prints but the gate does not recognise is itself reported
  (`E020 unknown failure signal from ...`), so adding a check to the script
  cannot silently fail to gate.

Mutation-proven: stripping `Source:` / `Owns:` / `Not for:` from a page fails
E020, and renaming a real heading to a name no module registers fails it too.

## Orthogonality

RG-022 is the reverse direction (every code unit named by a page) and stops at source units. This one is a page's own contract: its Source: header, its Owns:/Not for: boundary, its sibling links, and every tool name it puts in a heading. No owner covers that, and verify-library.sh was the only check that did while nothing ran it. (recon R1117050336abb: library)

## Revisions

- 2026-11-17: created
