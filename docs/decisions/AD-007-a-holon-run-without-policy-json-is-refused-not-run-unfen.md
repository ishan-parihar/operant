---
ID: "AD-007"
Title: "A Holon run without policy.json is refused, not run unfenced"
Status: "Active"
Date: "2026-10-19"
Description: "A Holon run whose policy.json is missing fails to start, instead of running with no seat authority"
---

# AD-007: A Holon run without policy.json is refused, not run unfenced

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

`enforcement::run_policy()` returned `None` when `policy.json` was absent from `HOLON_RUN_DIR`. The agent then started with no seat authority, and the native `holon_*` tools stayed available. The code comment called this "the same no-op" as a personal run. The Holon red-team recorded it as OS-3: a design gap, with no log line.

The kernel writes `policy.json` for every run it claims. `holon/execution/briefs.py` writes it whenever a policy is compiled, and `compile_policy` always returns a dict. A missing file therefore means the run was not claimed by a kernel that writes the document, or the file was lost.

## Decision

`run_policy()` returns `Some(Err(PolicyMissing))` when `HOLON_RUN_DIR` is set and `policy.json` is absent. `holon_authority` already propagates that error, so the agent refuses to start with `holon enforcement: policy document missing`. A personal run (no `HOLON_RUN_DIR`) is unchanged.

## Consequences

- A Holon kernel that predates `policy.json` cannot run an operant agent in org mode. That is intended: such a run has no reach to enforce.
- A run the kernel wrote with a policy is unaffected.
- The refusal happens at agent start, not per tool call, so a run never gets as far as one unfenced call.

## Orthogonality

AD-006 covers the supervisor's own fence, and the same run directory is the source for both.

## Revisions

- 2026-10-19: created
