---
ID: "AD-021"
Title: "Stored knowledge is bounded, redacted, and written whole"
Status: "Active"
Date: "2026-10-19"
Description: "memory_store has size bounds, memory and skill text is redacted or refused for secrets, memory and skill files are replaced atomically, and skill paths refuse symlinks"
---

# AD-021: Stored knowledge is bounded, redacted, and written whole

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

Four BUGS rows on what the agent stores: KB-12 (no bound on `memory_store`), KB-13 (secrets in memory and skills), KB-15 (MEMORY.md and USER.md written in place, so a crash or a second writer could truncate or lose an edit) and KB-16 (a symlink inside a skill directory was followed by writes and by the file walker).

## Decision

1. `memory_store` refuses a key over 128 characters, content over 8000 bytes, more than 16 tags, or a tag over 64 characters. The refusal names the bound.
2. Turns and facts pass through `redact_sensitive_text` before retain. A skill whose text holds a secret value is refused (`secret_in_skill`); it is not rewritten, because rewriting would change code the user wrote.
3. MEMORY.md, USER.md and the learning writes to SKILL.md are replaced through a temp file in the same directory, synced and renamed over the target. The existing mode carries over. The memory-chunk edits also take an in-process lock.
4. `resolve_skill_target` refuses a symlink on any component of the path. The file walker uses `file_type()`, which does not follow links, and skips symlinks.

## Consequences

- A memory save over 8000 bytes fails with the bound in the message, instead of being cut. The model sees the refusal.
- A skill that embeds a key value fails to save. The message says to reference an environment variable instead.
- The memory engine (memory-wire, tracked from `main`) already masks `sk-` and `ghp_` on retain; the operant layer covers more shapes (for example `sk_live_`) and does not depend on the engine's pattern set. The test uses `sk_live_` because only the operant layer masks it.
- The atomic replace does not serialize across processes. Two processes editing one MEMORY.md still last-write-wins. `write_screened`, which `skill_manage` uses for its own writes, still writes in place; that is a follow-up.
- Files inside a skill that are symlinks no longer appear in its file list. Skills that are themselves linked into the skills root are discovered as before; only the walk inside a skill changed.
- Tests: `an_oversized_write_is_refused_and_a_bounded_one_is_not`, `a_secret_in_a_turn_or_a_fact_is_not_stored_in_clear`, `a_skill_carrying_a_secret_value_is_refused_and_plain_text_is_not`, `a_replacement_leaves_no_temp_file_and_keeps_the_mode`, `a_symlink_on_the_path_is_refused_and_a_plain_path_is_not`. Mutants that turned red and were restored: the user-side, assistant-side and fact-side redaction, the skill refusal, the permission copy, and the symlink check.

## Orthogonality

The skill write screen (KB-4) is recorded in BUGS and the CHANGELOG, not in an AD. The refusal here runs first in `write_screened`, before the file is written and before the screen.

## Revisions

- 2026-10-19: created

