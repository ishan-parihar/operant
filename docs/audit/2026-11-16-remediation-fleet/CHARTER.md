# Fleet charter — operant remediation of the 2026-11-16 red-team

**Date:** 2026-11-16
**Base revision:** `cacf7257` (`docs(iter-2486)`)
**Source audit:** `docs/audit/2026-11-16-operant-architecture-red-team/00-INDEX.md`
**Method:** one coordinator (me) owns the gates and the integration; workers own
**disjoint file sets** and never touch a shared file.

---

## The non-negotiables

1. **File ownership is exclusive.** Each worker's `OWNS` list is the only set of
   files it may modify. If a fix needs a file outside the list, the worker
   reports it and stops — it does not edit.
2. **No worker runs `cargo build` on the workspace.** Each worker runs only
   `./scripts/check.sh check -p <crate> --lib` and the specific test filter for
   its area. The coordinator owns `check --workspace`, `test --workspace` and the
   clippy gate. (24 cores, but a shared `target/` and concurrent full builds
   thrash.)
3. **Never `git add -A`, never commit, never push.** Workers edit the tree; the
   coordinator stages explicit paths and owns every commit and push.
4. **Every fix ships a regression test that fails without the fix.** A fix
   without a test is not accepted.
5. **Report ASSERTED vs VERIFIED honestly.** If you could not confirm a
   behaviour, say so. A wrong "verified" costs more than an admitted gap.
6. **Read `AGENTS.md` §"Rust Code Standards" before writing Rust.** No
   `.unwrap()`/`.expect()` in production paths; typed errors in library crates;
   borrow before clone; comments explain *why*.

---

## Worker lanes

### W1 — blast-radius verb families (`Z15`)
**Severity:** HIGH. `mv`/`cp`/`rsync`/`install`/`scp` are absent from
`blast_radius::verb_kind`, so they classify as `BlastRadius::None` and
`screen()` returns before the deny list — those commands are invisible to the
gate entirely.

**OWNS:**
- `crates/operant-core/src/blast_radius.rs` (including its inline `#[cfg(test)] mod tests`)

**Do:**
- Add the missing verbs to `verb_kind`'s match, choosing the right `VerbKind`
  for each: `mv` is `Remove`-shaped for its destination; `cp` needs the
  `RawWrite`-style device check for `cp file /dev/sda`; `rsync --delete` and
  `install` need their own handling.
- Extend `parse_segment` only if the verb needs flags/targets it does not
  already extract (e.g. `install -m`, `rsync --delete`).

**Acceptance:**
- `cargo test -p operant-core --lib blast_radius` passes.
- NEW tests: `mv /etc /tmp/x` and `cp ~/.ssh/id_rsa /tmp/x` and
  `rsync -a /etc/ /tmp/x` are now **denied or require justification** (assert
  the concrete verdict, not just "not None").
- **Non-regression:** `mv ./a ./b`, `cp ./a ./b`, `rsync -a ./src/ ./dst/`
  still rate `Scoped` and are **allowed** without justification. This is the
  test that proves the fix is not "block everything".
- `install -m 644 ./a ./b` in-repo stays allowed.

**Report:** the exact `BlastRadius` each of the five verbs now yields for a
scoped target vs an unbounded one, in a table.

---

### W2 — cron double-fire and the daemon lock (`Z29`, `Z31`)
**Severity:** HIGH. `run_job` executes the job and only then calls
`mark_job_run`, which is the only write advancing `next_run_at`. A crash in
between makes the next `tick()` refire the job, repeating side effects.

**OWNS:**
- `crates/operant-core/src/cronjobs/scheduler.rs`
- `crates/operant-core/src/cronjobs/db.rs`
- `crates/operant-cli/src/cmd_cron.rs`

**Do (Z29):** write a **claim** before dispatch — an `in_progress` marker or a
run-history row keyed by `(job_id, scheduled_for)` — so a crash cannot refire
the same occurrence. Clear/settle the claim after the attempt. Decide and
document what a stale claim means on restart (recommend: a claim older than the
job's own cadence is treated as abandoned and the occurrence is skipped, not
retried).

**Do (Z31):** make `operant cron run <id>` consult the same file-level
`flock` the gateway daemon holds (`acquire_gateway_daemon_lock` in
`crates/operant-cli/src/gateway_runner.rs`, currently called only from
`gateway run`). Refuse the manual run with an actionable message when the
daemon holds the lock. **Do not edit `gateway_runner.rs`** — if the lock helper
is private, make the minimum change in `cmd_cron.rs` and report what you need.

**Acceptance:**
- `cargo test -p operant-core --lib cron` passes.
- NEW test proving the double-fire is closed: simulate the crash window (claim
  written, `mark_job_run` never called) and assert the next `tick()` does NOT
  re-dispatch.
- NEW test proving a legitimate next occurrence still fires (the claim must not
  wedge the job forever).
- If you touched `cmd_cron.rs`: `./scripts/check.sh check -p operant-cli --bin operant` passes.

**Report:** the claim's schema (column or table), its lifetime, and what you
decided a stale claim means — with the reasoning.

---

### W3 — TUI half-built panels (`Z22`, `Z23`)
**Severity:** HIGH. `/model roles` opens a panel that paints nothing but owns
the keyboard and can write config; two mouse hit-test maps are never written so
right-click and thinking-collapse are dead.

**OWNS:**
- `crates/operant-cli/src/tui/models_panel/`
- `crates/operant-cli/src/tui/app/mouse.rs`
- `crates/operant-cli/src/tui/app/commands.rs` — **only** the `overlay_flags()` array and the `/model roles` opener
- `crates/operant-cli/src/tui/operant_ui/ui_viewport.rs` — **only** if you choose to publish the maps

**Do (Z22) — pick ONE and justify it:**
- **(a) Finish it:** add a `paint_models_panel` and an `OVERLAYS` row so the
  panel renders, mirroring how `subagent_navigator`/`agent_picker` are painted
  from `paint_command_palette` (`render/operant_overlays.rs:538`).
- **(b) Delete it:** remove `models_panel/`, repoint `/model roles` at the
  inline picker's Roles scope, and drop the opener.

**Do not** merely add `models_panel.visible` to `overlay_flags()` — that hides
the defect by making the input gate aware of a panel that still paints nothing.

**Do (Z23):** either publish `thinking_row_map`/`message_row_map` during render
(the way `TOOL_ROW_REGIONS` is published at `ui_viewport.rs:1697`) or delete
both maps and the handlers that read them (`mouse.rs:119`, `:533`, `:1237`,
`:1331`) plus the `thinking_expanded` writer at `:1335`.

**Acceptance:**
- `./scripts/check.sh check -p operant-cli --bin operant` passes.
- `cargo test -p operant-cli --bin operant tui::` passes.
- NEW test for Z22: opening the panel either paints something (assert the frame
  contains the panel) or `/model roles` no longer references it.
- NEW test for Z23: a right-click over a transcript row opens a **message**
  context menu (assert the kind), and a click on a thinking header toggles
  collapse.
- If you deleted `models_panel/`, `rg 'models_panel'` must return only
  historical/CHANGELOG references.

**Report:** which option you chose for Z22 and why, in one paragraph. If (a),
name the paint function and where it hooks in.

---

### W4 — disabled-toolset enforcement (`Z17`)
**Severity:** MEDIUM. `ToolRegistry::execute()` never consults
`disabled_names`/`disabled_toolsets`; only `get_schemas*` do. So
`operant tools disable` hides a tool from the model but does not stop it
running.

**OWNS:**
- `crates/operant-core/src/tools.rs` — **only** the `ToolRegistry` impl

**Do:** consult the disabled set at the top of `ToolRegistry::execute()`
(`:1089`) and return a `ToolResult` error naming the tool and saying it is
disabled. Read both sets once, before any `.await`, and never hold the guard
across an await.

**Acceptance:**
- `cargo test -p operant-core --lib tools::` passes.
- NEW tests: a disabled-by-name tool returns an error from `execute()`;
  a disabled-by-toolset tool returns an error; an enabled tool is unaffected;
  the error message names the tool.
- **Non-regression:** an empty disabled set is byte-identical to today
  (assert the happy path still returns the tool's result).

**Report:** whether you put the check in `execute()` itself or factored it into
a helper both `execute()` and `get_schemas*` call. Prefer one shared predicate
if it is clean — say which you chose.

---

### W5 — compression-arm reasoning fix (`Z16`)
**Severity:** HIGH. `ReasoningParamRejected` is handled only in the streaming
arm (`agent/run.rs:1875`); the non-streaming compression arm never calls
`mark_effort_unsupported`, so `fitted_request` re-sends the rejected
`reasoning_effort`.

**OWNS:**
- `crates/operant-core/src/agent/run.rs` — **only** the compression/retry region

**Do:** make the non-streaming path handle `ReasoningParamRejected` the same
way the streaming arm does. Prefer hoisting the handling to where both arms
converge over duplicating the call — but if the arms genuinely cannot converge
without a large refactor, duplicate the two-line call and say so.

**Acceptance:**
- `cargo test -p operant-core --lib agent::` passes.
- NEW test: a non-streaming request that returns `ReasoningParamRejected`
  results in a retry whose `reasoning_effort` is absent.
- **Non-regression:** a provider that accepts reasoning parameters is unaffected.

**Report:** whether you hoisted or duplicated, and the call path for each.

---

### W6 — content-install gate + session socket auth (`Z25`, `Z30`)
**Severity:** MEDIUM / HIGH-on-fleet.

**OWNS:**
- `crates/operant-core/src/binary_updater.rs` — **only** `install_content` / `extract_content_tarball` / `scan_content_against_guard`
- `crates/operant-core/src/session_server.rs`

**Do (Z25):** require each extracted skill directory to contain a `SKILL.md`
with `metadata().len() > 0` **before** the `rename` at `:670`. A 0-byte or
missing `SKILL.md` fails the install and leaves the previous version intact.

**Do (Z30):** add a peer-uid check after `listener.accept()` (`:119`) so only
the same-uid owner may open a session, and make a failure to tighten the socket
to 0600 fatal rather than a warning (the code currently warns and continues).
On non-unix, `#[cfg]` the check out with a comment.

**Acceptance:**
- `cargo test -p operant-core --lib binary_updater` and `... session_server` pass.
- NEW tests: an empty `SKILL.md` refuses the install AND the previous version
  survives; a non-empty `SKILL.md` installs.
- NEW test for the socket: if the platform permits the check, a foreign-uid
  connection is refused (if you cannot construct a foreign-uid peer in a test,
  say so explicitly rather than faking it).

**Report:** how you tested the uid check, and what you could not test.

---

## Coordinator-owned (do NOT touch)

- **P1 — gates (`Z1`,`Z2`,`Z3`):** `scripts/clippy-warning-gate.sh`,
  `.ci/clippy-allowlist.txt`. Determinism (separate target dir per pass),
  refilled allowlist, and the in-file `#[cfg(test)]` exemption.
- **P2 — ponytail cuts:** `adapter_types/output_styles.rs`, the
  `InterruptFlag` `unsafe impl`s, the dead `PlatformAdapter::handle_update`,
  the dead `operant-config` modules. These land after W1–W6 to avoid overlap.
- **Integration:** `cargo fmt`, `check --workspace`, `test --workspace`, the
  clippy gate, the test-compile gate, `genome lint`.

## Definition of done

A lane is done when: its tests pass, `./scripts/check.sh check -p <crate>` is
green, the acceptance tests exist and fail without the change, and the report
answers the question asked. **Not** when the code compiles.