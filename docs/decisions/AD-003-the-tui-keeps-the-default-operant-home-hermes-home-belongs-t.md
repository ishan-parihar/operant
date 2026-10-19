---
ID: "AD-003"
Title: "The TUI keeps the default operant home; HERMES_HOME belongs to the Holon driver"
Status: "Active"
Date: "2026-10-19"
Description: "The TUI keeps the default operant home; HERMES_HOME belongs to the Holon driver"
---

# AD-003: The TUI keeps the default operant home; HERMES_HOME belongs to the Holon driver

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The TUI runs as the operator's personal seat. `operant_home()` checks `HERMES_HOME` first and falls back to `~/.operant` (`crates/operant-core/src/platform.rs`). The Holon driver (`~/holon/config/drivers/operant.toml`) sets `HERMES_HOME = /home/ishanp/holon/adapters/operant/home`, so each body's stores land in that body home. The 2026-10-19 audit found body history there and no `database.db`.

## Decision

The TUI keeps the default operant home. `HERMES_HOME` is set only in the Holon driver environment, and operant does not export it to the TUI. The TUI does not read body stores directly.

Correction to earlier records: the live driver is `/usr/local/bin/operant run -c "/home/ishanp/holon/adapters/operant/home/{posture}.toml" --max-iterations 40 --no-stream -q {briefing}`, with `cwd = "{workdir}"`, `timeout_s = 3600` and `[env] HERMES_HOME`. The driver form in `docs/agentic-os/IMPLEMENTATION-PLAN.md:102` (`operant -c {posture} run --no-stream -s {identity} -q {briefing}`) and §D.4 of the 2026-10-19 audit do not match it. The live driver passes no `-s`, and `-s` is `--system` in `operant run`, not a session id. Those pages are frozen and are not edited; this record supersedes them.

## Consequences

- The session browser lists the operator's own sessions. Body history is reached through the org views (AD-002) and the trajectory metadata (AD-001).
- A shell that exports `HERMES_HOME` still changes the TUI's home, as it always did. This decision adds no guard for it.
- No TUI home switch is needed for the org work.

## Orthogonality

No record decides which HERMES_HOME the TUI launches under. The HERMES_HOME override is set only in the Holon driver config (~/holon/config/drivers/operant.toml). (recon R1019053854: session tags body run holon roster)

## Revisions

- 2026-10-19: created
