---
ID: "AD-020"
Title: "Knowledge writes need the user's approval; an unattended review cannot write memory"
Status: "Active"
Date: "2026-10-19"
Description: "memory_store, skill_manage and learning_manage prompt for approval like file writes, and the background review refuses memory writes and runs its skill guard"
---

# AD-020: Knowledge writes need the user's approval; an unattended review cannot write memory

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

BUGS KB-3. The three knowledge-write tools had no approval gate, and the background review fork ran them with no user present. While checking the fix, a second defect came up: `is_background_review()` read only the process-global origin. Production sets the origin through `scope_background_review`, which is task-local, so the review's skill guard (`review_write_guard`, which requires a skill to be read before it is modified and protects pinned skills) returned `Allowed` for every background write.

## Decision

1. `memory_store`, `skill_manage` and `learning_manage` are in `is_permission_gated_tool`, so each call goes through the approval prompt. Their description is "This changes what the agent remembers or how it behaves in later turns".
2. `is_background_review()` reads the origin through `get_write_origin()`, which prefers the task-local tier.
3. A background review refuses `memory_store`, with a refusal that names the review. Skill and learning writes stay permitted, under `review_write_guard`.

Rejected: a staged queue (`write_approval`). Nothing applies a staged write, and the queue is process-local, so learning writes would be parked and never applied, without a visible signal. The module has no caller now and is a deletion candidate for the dead-code row.

## Consequences

- Every explicit memory save or skill edit in the foreground asks first. Expect more prompts.
- In org mode, `stream.rs` passes these tools to the seat authority as dangerous, so a seat policy can now deny them.
- A background review that edits a skill it has not read in the same review, or a pinned skill, is now refused. That is the guard's documented design; before this change the guard did not run.
- Tests: `the_review_guard_sees_the_task_scoped_origin`, `a_background_review_cannot_write_memory`, and the extended `mutation_and_gate_classifiers_cover_their_tools`. Mutants that turned red and were restored: the global-only origin read, the memory guard removed, and `memory_store` dropped from the gate list.

## Orthogonality

AD-011 fixes where the approval prompt sits relative to the other TUI gates. This decision changes what the gate covers, not where it sits.

## Revisions

- 2026-10-19: created

