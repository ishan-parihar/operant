---
ID: "AD-025"
Title: "The settings.json hooks menu is removed; /hooks points to operant hooks"
Status: "Active"
Date: "2026-10-19"
Description: "The read-only settings.json hooks menu is removed because no runtime path reads or fires those hooks; /hooks names the command that manages shell hooks"
---

# AD-025: The settings.json hooks menu is removed; /hooks points to operant hooks

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

RT-14. The menu read `settings.json` hooks from `Settings::config_dir()`. No runtime path read them. The live hook runtime is `HookRegistry`, which has no listing API. The `[hooks]` table has no production reader. A user who edited the file saw hooks listed that never ran.

## Decision

1. Remove the menu: the `hooks_config_menu` module, its overlay entry, key and dialog routing, `DialogPriority::HooksConfig`, and the App field.
2. `/hooks` prints that Operant runs no hooks from `settings.json` and names `operant hooks` for shell hooks.
3. The overlay list drops from 32 entries to 31; the debug-snapshot consistency test still passes.

Options not taken: a warning banner on the menu (keeps 642 lines of UI that show hooks which do not run); a runner that reads `settings.json` (a new surface, not asked for). Removing the `[hooks]` table is a separate RT-1 item and is not part of this record.

## Consequences

- `settings.json` hooks are shown nowhere in Operant. Making them run needs a runner, which is not built.
- `Settings::config_dir()` may now have fewer readers; no other use was changed.

## Orthogonality

AD-024 covers the other config table that is accepted and not acted on.

## Revisions

- 2026-10-19: created

