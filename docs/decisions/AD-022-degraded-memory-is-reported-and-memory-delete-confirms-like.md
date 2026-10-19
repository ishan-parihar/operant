---
ID: "AD-022"
Title: "Degraded memory is reported, and memory delete confirms like memory clear"
Status: "Active"
Date: "2026-10-19"
Description: "A memory store that fell back to memory is reported in the tool result and in memory stats, and memory delete asks for confirmation as memory clear already did"
---

# AD-022: Degraded memory is reported, and memory delete confirms like memory clear

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

BUGS KB-14 and KB-19. When the sqlite file cannot be opened, `MemoryWireProvider::new` falls back to an in-memory store and logs a warning. Nothing the user sees said so, so memories saved in that state were lost at exit without a visible sign. `memory clear` already asked for confirmation; `memory delete` removed a memory without asking.

## Decision

1. `MemoryProvider::is_persistent` defaults to true. `MemoryWireProvider` reports false when it fell back to the in-memory store. The `memory_store` result carries `"persistent"`, and `memory stats` prints `Storage: in-memory (degraded)`.
2. `memory delete` asks the user to type `yes` unless `--confirm` is given, like `memory clear`. A refusal prints `Aborted — nothing deleted.`

A degraded store still accepts writes. Refusing them would drop what the model tries to save; the report tells the user instead.

## Consequences

- A script that runs `memory delete` must pass `--confirm`. Scripts that run `memory clear` already do.
- A degraded store's writes are still lost at exit. The line is the only signal; fixing the sqlite path is the user's to do.
- Tests: `a_store_that_cannot_open_its_file_reports_itself_in_memory` (it makes `memory_wire.sqlite` a directory so the open fails; red when `is_persistent` returns true) and the `confirm_tests` module (`confirm_yes_from` accepts only a trimmed `yes`).

## Orthogonality

AD-021 covers what is stored. This decision covers whether the user can tell where it is stored.

## Revisions

- 2026-10-19: created

