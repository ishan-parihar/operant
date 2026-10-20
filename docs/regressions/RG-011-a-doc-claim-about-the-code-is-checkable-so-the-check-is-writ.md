---
ID: "RG-011"
Title: "A doc claim about the code is checkable, so the check is written down and run against the broken state"
Status: "Active"
Date: "2026-10-21"
Description: "scripts/check-doc-claims.sh derives AGENTS.md's code-surface claims from the code and reports every difference. It was verified by running it against the pre-fix documents: it catches the stale CLI list (53 vs 52, hardware/peripheral/sub-actions listed, donate/org missing) and the archived BUGS.md tree entry. It shipped three bugs of its own before that, each found by running it."
Severity: "Medium"
---

# RG-011: A doc claim about the code is checkable, so the check is written down and run against the broken state

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

RG-010 established that a doc's claim about the code is checkable, and wrote the
check into its Detection section. That check lived in a record — a place nobody
runs. The claims it protects were still free to drift, and they did.

Two stale claims were found in AGENTS.md on the same pass:

1. **The Architecture Overview tree named a file that does not exist.**
   `├── BUGS.md # Open issues tracker` sits at the repo root in the tree. There
   is no root `BUGS.md`: it was **archived at iter-900** (the 2026-08-03 triage
   snapshot now lives at `docs/archive/BUGS.md`) and the live ledger is
   `docs/audit/2026-06-19-BUGS.md`. This was the only one of AGENTS.md's
   seven `BUGS.md` references still using the old path — the other six were
   correct.
2. **The word "root" qualified a path that is not at the root**, in two places:
   `**File**: docs/audit/2026-06-19-BUGS.md (repo root)` and the routing
   table's `(root — NOT the kb ledger…)`. The parenthetical appears to mean
   "repo-native", but read literally it is a path claim, and it is false. The
   same wording had propagated to `docs/archive/INDEX.md`, whose row for the
   archived copy says "the live audit-fix ledger is root BUGS.md".

Everything else in AGENTS.md's inventories was verified **correct** against the
code: 7 platform adapters, 9 hook events, 24 error classes, 12 workspace
members, 30 overlays. So the drift was specific, not general — which is why it
survived.

## Root cause

**A check that lives only in a record is documentation, not a gate.** RG-010's
Detection section held a working script and nothing ran it, so the same class of
defect it was written to catch kept occurring. The two drifts above are the
exact defect RG-010 names — a doc claim about the code, wrong, with every
existing check passing.

The second root cause is narrower and more embarrassing: **the check itself was
never run against the state it exists to catch.** Written and adjusted in one
pass, it was first executed against a tree that was already green. It then
shipped three bugs of its own, each found only by running it:

| bug | symptom | why it matters |
|---|---|---|
| `grep -c 'PlatformEntry {'` counted the struct DEFINITION | reported 8 platforms where the registry has 7 | a check that reports a false DRIFT will be switched off, and a false GREEN is worse |
| the glob prefix included the directory path (`crates/operant-cli/src/cmd_`) while `os.listdir` returns bare names | reported `cmd_*.rs` as missing | same |
| the tree matcher ignored hierarchy and checked leaf names relative to the root | reported 30 nonexistent paths | same |

Each was a **false DRIFT**, not a false pass — the failure mode that teaches a
team to ignore a gate. The third was fixed by rebuilding the path from the
indentation depth, which is the only way a tree's leaves mean anything.

And the check that was actually needed — a tree whose leaves are paths — was
hand-rolled three separate times in this session before it was written down
once.

## Law

**A checkable doc claim gets a check that runs in the repo. A check is proven by
running it against the broken state, not the green one.**

1. **A check that only exists in prose is not a gate.** If a record's Detection
   section holds a script, the script belongs in `scripts/` and in the local
   gate. `scripts/check-doc-claims.sh` is that script: it derives AGENTS.md's
   code-surface claims from the code and prints the differences.
2. **Prove a check by running it against the state it is meant to catch.** A
   check that has only ever run green is untested. The proof is mechanical:
   restore the pre-fix document, run the check, confirm it reports the drift and
   exits non-zero; restore the fixed document, confirm it is green. Both runs
   were performed for this record, and the outputs are quoted in the Detection
   section below.
3. **A false DRIFT is as bad as a false pass, and easier to ship.** Every bug
   found in this check was a false DRIFT, not a missed one. A gate that cries
   wolf on the correct tree gets disabled, and then it catches nothing.
4. **A tree's leaves are paths, not names.** `mcp.rs` means
   `crates/operant-core/src/mcp.rs` only because of where it sits. Any checker
   for a tree must rebuild the path from the indentation, and a glob leaf must
   be matched against bare filenames in its own directory.
5. **Derive, then verify the derivation.** The check's own source of truth is
   the code; a claim about the check ("it reports 7 platforms") is itself a
   claim about a count, and it is checked by counting the entries in the
   registry function — not by counting the string in the file, which includes
   the struct definition.

## Detection

Run it. It exits 0 when every claim matches and 1 when any differs, printing
the differences as the fix list:

```bash
bash scripts/check-doc-claims.sh
```

Against the current tree:

```
OK    CLI command variants: 52
OK    CLI list: 52 entries, exact match
OK    gateway platforms: 7
OK    error classes: 24
OK    workspace members: 12
OK    Architecture Overview tree: 41 paths, every one exists
OK    AFT tools: 20
OK    TUI modules: 63
OK    issue ledger path: docs/audit/2026-06-19-BUGS.md exists
```

**Against the pre-fix documents** — the proof that it works. With the
pre-iter-1094 AGENTS.md restored:

```
DRIFT CLI command variants: doc says 53, code has 52
DRIFT CLI list: 3 listed but not real ['hardware', 'peripheral', 'sub-actions']; 2 real but not listed ['donate', 'org']
...
DRIFT tree paths that do not exist: ['BUGS.md']
DRIFT AGENTS.md tree still names a root BUGS.md
```

and with the pre-iter-1093 AGENTS.md restored (the BUGS.md tree entry):

```
DRIFT tree paths that do not exist: ['BUGS.md']
DRIFT AGENTS.md tree still names a root BUGS.md
```

Both restorations were reverted and the tree re-verified green.

**The gate was then EXTENDED, and extending it found more drift.** A gate that
checks some claims is an invitation to assume the rest, so the AFT and TUI
surfaces were added. The first run of the extended gate found that AGENTS.md
stated the AFT tool count **three different ways** — "15 IDE-grade coding tools",
"21 AFT IDE tools", "15 IDE-grade tools" — where the code has **20**
(`register_aft_tools` registers 20, and there are 20 distinct `aft_*` names).
And "50+ TUI modules" was a lower bound where the code has 63 `pub mod`
declarations (303 `.rs` files). The AFT check now also asserts that the
REGISTERED count and the distinct-NAME count agree inside the code itself, and
that the doc does not state the AFT count several different ways. The TUI check
treats a lower bound as what it is — an uncheckable claim — and reports it as
drift rather than accepting or ignoring it.

The extended gate was proved against the pre-fix document:

```
DRIFT AFT tools: doc says 15, code has 20
DRIFT AFT: AGENTS.md states the AFT count several different ways: 15 21
DRIFT TUI modules: doc says 50, code has 63
```

and exits 1. Both restorations were reverted and the tree re-verified green.

The generalisable detector, and the one this session has now paid for nine
times: **a doc claim about the code is a test in disguise.** The test is cheap,
the failure is silent, and the claim outlives the code it describes by years.
Write the check once, put it where a machine runs it, and then prove it against
the broken state — because a check that has only ever been seen green is
indistinguishable from a check that does not work.

## Orthogonality

RG-010 says a code-surface list must be derived and gives a check in its Detection section. This record is about the check becoming a STANDALONE GATE that runs in the repo, and about the proof that it works: a check that has only ever been run against a GREEN tree is untested. No record covers the meta-defect — a detection script that was never run against the state it is supposed to catch, and that shipped with three bugs of its own (counting a struct definition as an entry, a glob prefix that included the directory, and a tree matcher that ignored hierarchy). (recon R1021000411758: check script detects drift doc claims derived from code)

## Revisions

- 2026-10-21: created
