---
ID: "RG-024"
Title: "source comment cites a doc deleted without a tombstone"
Status: "Active"
Date: "2026-11-17"
Description: "a Rust comment points at a design doc that was deleted, so a reader following the pointer hits nothing"
Severity: "Medium"
---

# RG-024: source comment cites a doc deleted without a tombstone

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

A `//!` or `///` comment in source says "see `docs/design/whatever.md`" and that
file does not exist. A reader who follows the pointer to learn *why* the code is
shaped this way finds nothing, and the comment reads as if the design were
available.

## Root cause

Docs are archived by moving them to `docs/archive/` with a tombstone, or they are
deleted outright. Deletion is faster and nothing catches the dangling pointer,
because every documentation gate this repo runs checks paths that appear *inside
docs* (E303 over `docs/INDEX.md` and `docs/library/`, E020 over library pages).
None of them looks at paths cited *from source*, so the code-to-doc direction is
unchecked.

Measured 2026-11-17: five live pointers into deleted docs across
`peripherals_traits.rs`, `operant-os/src/lib.rs`, `cronjobs/scheduler.rs`,
`org/context_injection.rs` and `schema/media.rs`. In three of the five the doc
still existed under a different root (`docs/plans/plan-*.md`), so the pointer was
stale rather than wrong, and the fix was a path correction.

## Law

A document is deleted by archiving it with a tombstone, never by removing the
file. When a doc is archived, every source comment that names it is corrected in
the same iteration. A source comment that cites a path must cite one that exists,
and the check for that is mechanical, not a matter of reviewer memory.

## Detection

```bash
for f in $(grep -rhoE 'docs/[a-z0-9_/-]+\.md' crates/*/src --include=*.rs | sort -u); do
  [ -f "$f" ] || echo "DEAD: $f"
done
```

Treat a hit as a defect unless the path is test fixture data or names another
project's tree.

## Orthogonality

RG-002 covers editing numbered records, not dead doc pointers. The genome E303
and E020 gates check paths that appear *inside* docs; none checks doc paths cited
*from* source comments. No existing record covers the code-to-doc direction.
(recon R1117092349de5)

## Revisions

- 2026-11-17: created