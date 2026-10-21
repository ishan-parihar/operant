---
ID: "RG-013"
Title: "the doc gate cannot see a cfg(test) item, which is where doc rot hides"
Status: "Active"
Date: "2026-10-21"
Description: "the doc gate cannot see a cfg(test) item, which is where doc rot hides"
Severity: "High"
---

# RG-013: the doc gate cannot see a cfg(test) item, which is where doc rot hides

Immutable once numbered: a correction is a new record that supersedes this one.



## Symptom

The rustdoc gate (`scripts/check.sh doc` = `cargo doc --workspace --no-deps
--all-features` under `-D rustdoc::broken-intra-doc-links` and
`-D rustdoc::invalid-html-tags`) reports zero warnings for a `#[cfg(test)]` item
whose doc comment links to an identifier that does not exist anywhere in the
tree. The gate is green; the doc is broken.

## Root cause

`--document-private-items` documents private ITEMS, but the doc build for a
binary target runs with `cfg(test)` OFF, so a `#[cfg(test)]` item is not in the
AST rustdoc sees at all. Test-only scaffolding therefore occupies a class of
documentation the gate cannot read — and test-only scaffolding is where the
jcode-parity port's residue accumulated (iters 1160 through 1165).

The live instance: `RenderStateTestGuard` in
`crates/operant-cli/src/tui/operant_ui/mod.rs` carried
"Guard for [`render_state_test_lock`]" — an intra-doc link to a function that
was NEVER PORTED and does not exist. The gate stayed green for the whole of
that code's life. iter-1165 found it only because
`cargo clippy --all-targets` (which compiles `cfg(test)`) reported the struct
as never constructed, and a grep then showed zero callers for any of the three
names in the mechanism.

## Law

A doc gate that only sees the production profile cannot certify a
`cfg(test)` doc link. Either the gate gains a second profile that documents
with `cfg(test)` on, or the project's doc-link claims are qualified to the
production profile only. **Do not read a green doc gate as evidence that a
test-only doc link resolves.**

Recorded, not fixed: adding `--cfg test` to the doc gate is a real change to the
gate's meaning — it would surface every existing `#[cfg(test)]` doc link in the
workspace at once, and the iter-1158 sweep found those classes are large. That
is an iteration of its own, not a drive-by.

## Detection

Measured, not assumed (iter-1166). Appended to
`crates/operant-cli/src/tui/operant_ui/mod.rs`:

```rust
#[cfg(test)]
/// Links to [`a_function_that_does_not_exist_anywhere`].
pub(crate) struct ProbeStruct { _x: u8 }
```

Then ran the exact gate command (`cargo doc -p operant-cli --bin operant
--no-deps --document-private-items` under the gate's `-D` flags) and grepped its
output for the probe's names: **0 matches.** The same probe WITHOUT the
`#[cfg(test)]` attribute is reported. So the attribute, not the link, is what
hides it. The probe was removed and the file verified byte-identical to its
pre-probe state.

## Orthogonality

RT-166 fixed the broken intra-doc links that were already visible; this records
that a whole class of them — every `cfg(test)` item — is invisible to the gate,
measured rather than assumed (recon R1021223437834: doc gate cfg test private
items). RT-011 is "a doc claim about the code is checkable, so the check is
written"; this is a check whose COVERAGE was overstated by the very flag that
made it useful (`--document-private-items` reads as "everything", and it is
"everything the production profile contains").

## Revisions

- 2026-10-21: created
