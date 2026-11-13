# Intent Table — TUI exit leftover + `operant continue` (session-management gaps)

Workspace: `/home/ishanp/Documents/github/my-projects/agentic-harness/operant`
Created: this session (post-compaction continuation).

## 1. Deliverable
A working `operant continue <session-id>` path: (a) the TUI prints, after it closes, the
exact command that re-enters the session it just left; (b) that command exists and really
attaches the TUI to the session; (c) a gap report on operant session management.

## 2. Verbs (each is a constraint row)
- C1 **Print/emit** — on TUI exit, leftover must name `operant continue <session-id>`.
- C2 **Implement** — `operant continue <session-id>` attaches the TUI to that session
  (rehydrated transcript), not just a recap.
- C3 **Investigate** — report the implementation gaps in session management, with evidence.

## 3. Explicit constraints (verbatim)
- "After closing the TUI, the leftover must throw a operant continue/session subcommand
  with the session-id or something, so that the previous session can be continued effectively."
- "Investigate the implementation gaps in the operant session management."

## 4. Implicit constraints (each with source)
- (a) Prior turns: the repo is `agentic-harness/operant`; `operant-testing/kb/` is the
  established record for findings; session resume UX is already inspired by
  `external-repos/codex` (`TuiCli::resume`, `codex resume <id>`) and `jcode`.
- (b) Codebase conventions: `cmd_*.rs` = one CLI subcommand each, `mod cmd_x;` in
  main.rs; comment style = "why" prose with plan/iter references; tests live next to the
  code in `mod tests`.
- (c) Platform: Linux terminal; the leftover prints AFTER `LeaveAlternateScreen` so it must
  be plain stdout, single pass, no progress redraw.

## 5. Scope boundaries
- IN: TUI exit leftover; `operant continue` subcommand; TUI launch mode that attaches to an
  existing session; the presence-claim defect that makes `session_is_live` always false;
  the gap report.
- OUT: rewriting the session hub; gateway resume behaviour; DB schema changes; the
  unrelated `turn ended early` bug (separate investigation, recorded not fixed here).

## 6. Divergence checks
- Symptom vs solution — the symptom is "I cannot get back into the session I just closed".
  The requested change (print a command) only fixes it if the command WORKS. So C2 is
  the goal and the printed command is a hypothesis for it.
- Guessed mechanism — "leftover must throw a operant continue/session subcommand": assumed
  no such command exists. Verified: no top-level `Continue`; `operant sessions resume`
  exists but only prints a recap + manual instructions (cmd_sessions.rs:606-672).
- Question vs request — "Investigate the implementation gaps" is a genuine sub-deliverable
  (the report), not a preamble to a change.

## 7. Ranked constraints (fixed ladder)
1. Safety / correctness — never fork two writers on one session (presence registry).
2. Explicit requester instructions.
3. Repo conventions.
4. Style.

## Conflicts row
- **Interrupted-session marker consumption**: `operant continue` (no id) could `consume_marker()`
  the way `sessions resume --last` does, but that steals the marker from the gateway boot
  notice and the TUI boot notice (both `consume_marker()`). **Resolution**: `continue` uses
  `read_marker()` (non-consuming) as a hint; the next TUI boot consumes it as its greeting.
  Lower rank: the gateway's second-chance notice loses a rename-only benefit, so it loses.
- **`sessions resume` semantics**: it currently recap-PRINTS. Making it launch the TUI would
  change an existing documented command. **Resolution**: leave `sessions resume` alone; add
  `continue` as the action, `resume` stays the re-orient.


---

# Second request (same session, later turn)

> "When I restarted the session, it did not throw the command print so that I can copy and
> continue this session. Investigate the gaps, so that we can utilize it in runtime."

## Deliverable
The copy-pasteable `operant continue <session-id>` must be available **at runtime**, not only in
the exit leftover: (a) a restart (the boot notice) must carry it, and (b) the running TUI must be
able to produce it on demand.

## Verbs
- C4 **Print at restart** — the boot notice carries the whole id + the command.
- C5 **Print at runtime** — a slash command produces it while the TUI is open.
- C6 **Investigate** — the gaps behind "the restart printed nothing".

## Explicit constraints (verbatim)
- "When I restarted the session, it did not throwed the command print so that I can copy and
  continue this session."
- "Investigate the gaps, os that we we can utilize it in runtime".

## Divergence checks
- Symptom vs solution — the symptom is "after a restart I have nothing to copy". The requested
  change is the print. Verified the gap is REAL, not a misreading: the boot notice was a
  STATUS-BAR line naming a TRUNCATED id (`marker.session_id.chars().take(8)`) and no command at
  all; a truncated id is not a valid id, so nothing could be typed or copied from it.
- Second gap — even with a boot print, an operator who wants to leave NOW has no way to ask for
  the command while the TUI is open. Hence C5 as a separate surface, not a bonus.

## Scope
- IN: the boot announcement, the runtime slash command, the shared formatter, the tests.
- OUT: the gateway's command surface (a different surface, a different ask), the session hub's
  row rendering (a peer's audit owns it).

## Conflicts row
- **One formatter, three surfaces**: the exit leftover, the boot notice, and `/continue` must not
  drift into three different command strings. Resolution: ONE `format_continue_hint(state, id,
  title)`; the exit leftover is a wrapper over it. The `state` sentence differs per surface
  because each annotation must make sense on its own — a status line flashes away, an annotation
  does not.
