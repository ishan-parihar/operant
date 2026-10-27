---
ID: "RG-015"
Title: "an uncommitted peer edit can leave the shared tree red, and a red tree is not evidence my change broke it"
Status: "Active"
Date: "2026-10-27"
Description: "an uncommitted peer edit can leave the shared tree red, and a red tree is not evidence my change broke it"
Severity: "High"
---

# RG-015: an uncommitted peer edit can leave the shared tree red, and a red tree is not evidence my change broke it

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

While verifying my own change, `cargo check` failed with a compile error
`cannot find function composite_names in module sourcehound_composites` pointing at
`crates/operant-core/src/tools/builtin.rs:351`. The failing file was NOT one I had
touched, and my own files (`gateway_commands.rs`, `gateway_runner.rs`) compiled fine
on their own minutes earlier. The tree was red from a peer's half-finished edit: they
had replaced `composite_names()` with `composite_specs()` in their UNTRACKED
`sourcehound_composites.rs` while `builtin.rs`, which they also had dirty, still called
the old name.

## Root cause

The build reads the WORKING TREE, not `HEAD`. An in-flight peer edit therefore breaks
everyone's `cargo check` in the shared tree, including edits that are themselves
complete and correct. The failure mode is asymmetric and expensive: the error names a
function, which reads like a defect in whatever most recently changed that file —
and my change was the most recent one in the tree at that moment. Fleet rule 7 says
"if the tip is red from a peer's in-flight subsystem, don't fix it and don't block on
it", but that rule only helps once you have correctly ATTRIBUTED the red. Nothing in
the protocol tells you how to attribute it, and the obvious heuristic (most recent
change) is wrong exactly when a peer has unpublished work.

## Law

**Before treating a compile failure as yours, check whether the failing path is in the
incoming peer range or in a dirty untracked/modified file you do not own.** The check
is two commands: `git status --porcelain` for which files are dirty, and
`git log <my-base>..origin/main --name-only` (or simply: is the failing file one I
staged or committed?). If the failing path belongs to neither my commit nor a pristine
tracked file, it is a peer's in-flight work — record it, do not fix it, and re-verify
after their edit settles rather than reporting a false regression against my own delta.

The verify-my-delta-anyway rule is the practical escape hatch: proving MY change green
against the base (stash my paths, or simply re-run once the peer's edit completes)
produces the evidence without ever touching their files. On this occasion the peer's
edit completed within ~90 seconds and the full battery ran green immediately after.

## Detection

`cargo check -p <crate>` fails naming a symbol in a file you did not edit. Confirm with
`git status --porcelain -- <failing path>`: if it prints `M` or `??`, and the path is
not in your staged set, the red is not yours. Record the observation in the commit body
so the next agent reading history does not inherit a false regression claim — which is
exactly what the peer's `def30e36` "my false compile-defect finding retracted" had to do
in the reverse direction, and what this record now prevents in both.

## Orthogonality

the feed entry is a coordination note inside one ledger-sweep workflow; this is a standing rule about attributing a compile failure found in a shared tree with in-flight peer work, which no AD or RG covers (recon R1027054048765: peer untracked in-flight edit breaks the shared tree build red not my change)

## Revisions

- 2026-10-27: created
