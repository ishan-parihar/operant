---
ID: "AD-024"
Title: "A [keybindings] table loads and is warned about; dispatch does not read it yet"
Status: "Active"
Date: "2026-10-19"
Description: "A [keybindings] table in operant.toml is accepted and produces one warning, instead of failing the whole config load"
---

# AD-024: A [keybindings] table loads and is warned about; dispatch does not read it yet

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

RT-3. `AppConfig` is `deny_unknown_fields`, so a `[keybindings]` table failed the whole parse: `Invalid TOML ... TOML parse error at line 1, column 2`. The CLI and the gateway would not start. The keybinding registry and `/keys` exist, but the TUI dispatch is an if-chain in `key_handling.rs` and does not read overrides (`keybindings.rs:7-11`). No shipped document gives a `[keybindings]` example.

## Decision

1. `AppConfig.keybindings: Option<toml::Table>` accepts the table. It is `None` unless the file carries one.
2. `load` prints one warning when the table is present: the table is ignored, the TUI does not read key overrides yet, and `/keys` lists the bindings in effect.
3. The parsed table is not used. The override loader waits for the dispatch migration, which the row lists as the prerequisite.

Options not taken: keeping the parse error (one table blocks every command, and the message names no table); silently accepting it (a false feature).

## Consequences

- A user with the table gets the warning and the default bindings, and the process starts.
- The field is parsed and never read, which is the RT-1 pattern. The warning is its only reader; RT-3 stays open for the loader.
- Test `keybindings_tests::a_keybindings_table_still_loads_and_is_kept_for_the_warning`; mutant (serde key renamed so the table is unknown again) turned it red.

## Orthogonality

Follows the retired-key pattern used for `[browser]` and `[memory]` (plan 11C2, 11C3): accepted, warned once, not acted on.

## Revisions

- 2026-10-19: created

