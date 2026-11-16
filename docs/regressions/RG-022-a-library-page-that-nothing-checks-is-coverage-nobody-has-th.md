---
ID: "RG-022"
Title: "A library page that nothing checks is coverage nobody has: the genome gains a reverse coverage gate (E311)"
Status: "Active"
Date: "2026-11-17"
Description: "A library page that nothing checks is coverage nobody has: the genome gains a reverse coverage gate (E311)"
Severity: "High"
---

# RG-022: A library page that nothing checks is coverage nobody has: the genome gains a reverse coverage gate (E311)

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

The mechanism library (`docs/library/`, 31 pages over a 516,350-line Rust
workspace) had no check in the OTHER direction. `genome lint` E301 asks whether
every doc in an owner location is listed in `docs/INDEX.md`; nothing asked
whether every code unit is NAMED by a doc. Measured on the first run of the new
gate: **278 of 840 source files are named by no library page at all** — 36 in
`tui/`, 31 in `tools/`, 31 in `tui/operant_ui/`, 23 in
`tui/operant_app/info_widget/`, and 15 in `operant-core/src/` including
`turn_attribution.rs` (992 lines), `model_roles.rs` (794), `herdr.rs` (680),
`process_registry.rs` (593) and `session_server.rs` (512).

Every one of those is a subsystem an agent or an operator can be asked about and
will not find. The absence was invisible because the pages that DO exist are
good: reading them tells you the library is thorough, and there is no list
anywhere of what is missing from it.

## Root cause

The coverage contract was written down as a prose law ("a new mechanism doc
lands here when a subsystem's mechanism needs explaining") and enforced in one
direction only. A prose law about completeness has no falsifier, so the failure
mode is silent accumulation rather than a red gate: each individual omission is
a judgement call nobody is obliged to make, and the set of omissions is never
computed.

This is the doc-side twin of the defect the repo already knows in code — the
`turn_attribution.rs` header that claimed to be a live fix while nothing called
it (RT-209). An orphaned module and an undocumented module are the same reader
failure: the reader believes the surface is smaller than it is.

## Law

**A coverage claim that no gate computes is not a coverage claim.** If the
documentation contract says "every architectural element is documented", the
element set and the doc set must both be enumerable and their difference must be
a lint finding. Direction matters and both are needed: E301 walks doc → map,
E311 walks code → doc. One without the other leaves exactly the half that
readers feel.

The check is deliberately LEXICAL — the unit's file stem must appear in some
page's text — because a gate has to be mechanical to be a gate. "Is this
documented WELL ENOUGH" stays a reviewer's judgement; "is this documented AT
ALL" does not.

## Detection

`genome lint` E311, configured in `genome.toml`:

```toml
[ops]
coverage_surfaces = ["docs/library/*.md"]   # the text that must name the unit
coverage_units = ["crates/*/src/**/*.rs"]   # the unit set
coverage_exempt = []                        # a unit with no page needs a REASON here
```

An exemption with no reason is not an exemption; it is a deferral with the
paperwork missing, and the config comment says so.

## Orthogonality

RG-011 says a doc CLAIM about code must be written down and checked; it governs the content of a claim. This is the set question - whether a code unit has any page at all - which E301 does not ask (E301 asks if a listed doc is in the map, not whether a module is in a doc) (recon R1117033418c68: documentation coverage gate library)

## Revisions

- 2026-11-17: created
