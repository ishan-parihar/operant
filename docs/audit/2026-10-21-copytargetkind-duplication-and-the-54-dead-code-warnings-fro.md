# Audit — CopyTargetKind duplication and the operant-cli dead-code warnings

**Date:** 2026-10-21
**Agent:** jcode
**Trigger:** an autonomous defect sweep after landing iter-1158/1159.

## What was measured

`cargo clippy -p operant-cli --bin operant --all-targets` reported **69 warnings**
at the start of this sweep. `cargo clippy -p operant-cli --bin operant`
(the production profile) reports **0**. Every one of the 69 is therefore in the
test profile, and every one is `dead_code` on a module the jcode parity port
vendored or partially ported.

After iter-1160 (five dead test wrappers, one false-promise accessor, one macro
`allow`), the count is **54**.

## The finding that is not just a lint: two `CopyTargetKind` enums

There are two definitions of the same concept, both carrying near-identical
`label()` / `copied_notice()` implementations:

1. `crates/operant-cli/src/tui/operant_markdown/markdown_types.rs:30` —
   `CopyTargetKind` with `Math { display: bool }`.
2. `crates/operant-cli/src/tui/copy_targets.rs:82` — `CopyTargetKind` with
   `Math` (no payload), plus doc comments and `#[must_use]` on both methods.

`operant_model/prepared.rs:8-12` carries the decision comment and says the
operant-markdown one is "the canonical home upstream (markdown_types.rs); the
W1 interim re-root to operant's copy_targets.rs predates the W2 markdown port."
That comment is correct, and the tree follows it: `prepared.rs:12`,
`operant_ui/mod.rs:152` and `render/selection.rs:266` all name the
operant-markdown type.

But the copy_targets.rs copies are still there, and after this sweep removed
`VisibleCopyTarget`'s unread `kind_label`/`copied_notice` fields (which were
the last production caller of those methods), **their `label()` and
`copied_notice()` are now referenced only by their own tests** in
`copy_targets.rs:688-710`.

So the duplication is now: one set of methods with a real test suite and no
production caller, and one set with production callers and no tests.

## Why this was NOT fixed in this iteration

Deleting `copy_targets.rs`'s enum is not a mechanical cleanup, and this audit
records why rather than doing it:

- `copy_targets.rs` also defines `CopyTarget` and the region-extraction
  helpers that the port's own tests drive. Whether those are superseded by
  `operant_markdown::RawCopyTarget` + `extract_copy_targets` needs a
  line-by-line comparison, not an assumption.
- Two modules in the tree cite `copy_targets.rs` in
  `[port-decision]`/`[port-excision]` comments (`vendor_types.rs:18`,
  `prepared.rs:8`). Those comments are the next agent's map; changing the
  code without updating them recreates the drift this audit exists to stop.
- The `Math { display: bool }` vs `Math` difference is a real semantic
  difference (display-vs-inline math), not a cosmetic one.

## Recommendation

One focused iteration that (a) diffs `copy_targets.rs` against
`operant_markdown::{CopyTargetKind, RawCopyTarget, CopyTarget}` variant by
variant, (b) collapses the pair to the canonical operant-markdown spelling or
moves the `display` payload into it, (c) repoints `prepared.rs` /
`operant_ui/mod.rs` / `render/selection.rs` to one import, (d) ports the
`label()`/`copied_notice()` tests onto the surviving method or deletes the
methods if the badge no longer shows a label, and (e) updates the three
`[port-decision]` comments in the same commit.

Until then the duplication is recorded, not fixed.

## The 54 remaining warnings, classified

Most are one of three legitimate shapes, which is why they are warnings and
not deletions:

- **Exhaustive-match residue.** `BackgroundTaskRowStatus::{Running, Completed}`
  are matched by `active_background_task_line` (ui_viewport.rs) but never
  constructed — the producer `background_task_rows()` returns `&[]` by a
  documented `[port-decision]`. `AgentModelTarget::Review/Judge/Memory/Ambient`
  and `PickerAction`'s unconstructed variants are the same shape. Deleting a
  variant the renderer still matches would break the match.
- **Partially-vendored domain schemas.** `memory.rs`'s `StepStatus`/
  `MemoryEvent`/`MemoryState`, `usage.rs`'s `OpenAIUsageData` fields,
  `info_widget/mod.rs`'s `UsageProvider::Copilot`. Each is a schema for a
  subsystem only some of whose states this build produces. Deleting a variant
  makes the schema lie about what exists.
- **Struct fields written at construction and read by nothing.**
  `swarm_gallery.rs`'s three row types, `prompt.rs`, `usage_overlay.rs`.
  These are candidates for removal once a consumer is confirmed absent, but
  several sit in modules whose consumers are behind `#[cfg(not(test))]`, so the
  lint cannot see the read even where one exists.

The five genuinely-dead items removed in iter-1160 are cited in that commit.
