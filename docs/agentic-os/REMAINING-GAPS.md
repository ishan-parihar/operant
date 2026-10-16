# Remaining implementation gaps (2026-10-16)

Status: **outline.** The audit's W-packages are closed (`AUDIT-2026-10-15.md` §7, plus §8's
red-team close-out, iters 854–861). This file is the map of everything still open after that
close-out: what each gap is, where it lives, who owns the prerequisite, and the acceptance that
would close it. It is the working list for the next batch of iterations; each G-item is one or
more iterations under `AGENTS.md`.

Nothing here blocks the owner's **turn org mode on** checklist (§7): archive taken ✓ (twice now,
the second under the WAL-safe code with the `org/` charter tree in it), configure absolute
`[holon] home` + `bin`, run one supervised routine under owner control. G1 below is that last
step seen through.

## The one-line state

The integration is **built and proven in tests**: engagement, fences, organs, channel router,
capture, archive — all have regression evidence against the real kernel. It has **never run a
real org day**. Every remaining gap except G8/G9 exists to make that first day survivable or to
pay down deliberate deferrals.

## G1 — Turn org mode on (owner-held, the last checklist step)

- **What**: the §7 checklist's remaining steps, on the live workstation.
- **Where**: `~/.operant/operant.toml` (`[holon]` section), `~/holon` (the live instance).
- **Steps**:
  1. `operant org holon archive` — done 2026-10-16 (the `org/` tree is in a snapshot now).
  2. Configure `[holon]` with an absolute `home` (`~/holon` resolved) and `bin` (an absolute
     launcher — `HOLON` is a shell string; `bin` is the one operant resolves itself).
  3. Install the adapter into the live instance
     (`python3 adapters/operant/install.py --holon-home ~/holon …`) — the new installer probes
     the deployed binary and drops the Python MCP mount on its own.
  4. `supervise_employees` set, `[holon] supervise = true`, `clock = true`.
  5. Run **one** routine supervised end to end with the owner watching (`operant org holon
     supervise --loop` in a terminal, or the gateway organ at boot).
  6. After the day: `operant doctor` — the §8 lines (notion script, archive age) must be green.
- **Accept**: one routine's plan→execute→verify cycle completes with bodies on the operant
  binary, `usage.json` populated from the provider, and no Python `holon mcp` process alive
  during any body run.
- **Owner**: the owner runs steps 2–5 (the standing rule: the Chair key never reaches an agent).

## G2 — Chair notify outbound (audit W5's last clause)

- **What**: Holon notifications delivered to the Chair's thread. The inbound half is complete
  (is_chair gate → decision router → run_as_chair → reply), but nothing in operant POSTS to the
  Chair's channel on its own initiative; `run_as_chair`/`dispatch_message` only *respond*.
- **Where**: `crates/operant-os/src/chair.rs` (dispatch half done) +
  `crates/operant-cli/src/gateway_runner.rs` (the outbound send path exists per-reply at
  `~:3495`, nothing calls it from kernel events).
- **Design already fixed by the audit**: "outbound: a `[notify]` command that posts through the
  gateway". The kernel gains the ability to ask operant to deliver a message (its own routines
  finish with "tell the Chair X"); operant's gateway posts it to the Chair's configured channel.
- **Prerequisite**: a kernel-side `[notify]` surface (Holon repo — which command emits, where the
  Chair's channel ref comes from). Operant cannot invent the protocol.
- **Accept**: a kernel `notify` (or post-execute hook) surfaces in the Chair's Telegram thread.

## G3 — Clock evidence (last tick)

- **What**: `clock` organ runs `holon tick` every 2 minutes, but there is no observable
  "last tick" anywhere — not kernel-side (no `last_tick` field exists), not operant-side.
  Doctor reports the organ's *configuration*, not its *health*. A silently-dead clock looks
  identical to a live one.
- **Where**: `crates/operant-os/src/organs.rs:76` (tick_once — fire-and-log only),
  `crates/operant-cli/src/cmd_doctor/mod.rs:229` (the clock line).
- **Lazy fix**: operant stamps `<operant home>/org/last-tick` (mtime = evidence) after every
  successful tick; doctor reads the stamp and reports age. No kernel change — the tick's caller
  is the evidence. (A kernel-side `last_tick` in `status --json` would be better long-term; the
  stamp needs no cross-repo coordination.)
- **Accept**: `operant doctor` with `clock = true` reports "last tick Ns ago" and flags a clock
  that has not fired in > 5 minutes.

## G4 — Supervisor claim loop is still pass-oriented (partial O-12 residue)

- **What**: the organs loop and the CLI loop both do *pass → wait for ALL bodies → next pass*.
  True slot utilization needs the claim loop: when a body finishes, claim the next stage
  immediately instead of waiting for its siblings. `max_concurrent_runs` now genuinely bounds
  concurrency (iter-855), but an idle slot waits for the slowest sibling to finish the pass.
- **Where**: `crates/operant-os/src/organs.rs:111-156` (handles collected, awaited, then loop).
- **Ladder check**: this is a *throughput* refinement, not a correctness gap. With 1-employee
  pilot orgs it changes nothing. **Defer until a real org has >1 concurrent body and a measured
  slot-idle time worth claiming.**
- **Accept (when pulled)**: with `max_concurrent_runs = 2` and 3 ready stages, a freed slot
  claims the third stage before the first pair both finish.

## G5 — `aft_ast_replace` outside the roots fence (documented ceiling)

- **What**: the O-18 roots fence covers `file_write`, `patch`, `aft_write`, `aft_edit`,
  `aft_apply_patch`. `aft_ast_replace` names no path in its arguments (AFT resolves the pattern
  to files internally), so a Holon body could use it to edit outside its roots — bounded only by
  the adapter's `disabled_tools` (which disables it in both postures TODAY) and the kernel's
  workdir fingerprint (after the fact).
- **Where**: `crates/operant-core/src/tools/aft_tools.rs` (`AftAstReplaceTool::execute` — no
  guard call), Holon `adapters/operant/install.py` (the disabled_tools lists).
- **Ladder check**: it is disabled in every real posture config; the exposure is a misconfigured
  posture. **The honest fix is one line**: route `aft_ast_replace` through the fence by
  *extracting file paths from the pattern* — but an AST pattern is not a path, so the real fix is
  AFT-side (the bridge reports the files it touched; operant fences the report). That is a
  bridge-protocol change. **Defer with the ceiling documented** (it already is, in the audit §8);
  revisit when/if AFT grows a touched-files response field.
- **Accept (when pulled)**: an `aft_ast_replace` that rewrites a file outside the roots is
  refused or reverted, in a posture where it is enabled.

## G6 — The live instance has never run an operant-bodied stage

- **What**: `~/holon` has no operant adapter installed (verified 2026-10-16). Every proof this
  session produced ran against scratch instances in tests. The FIRST real run always finds
  what tests cannot (env, permissions, the real provider, the real org data shape).
- **Where**: live workstation state, not code.
- **What it needs**: G1 steps 2–5. Nothing else — this is G1's shadow, listed so it is not
  forgotten that "done in tests" ≠ "done".
- **Accept**: same as G1's.

## G7 — The org's `MEMORY.md` charter files in `~/.operant/org/` are single-copy

- **What**: the 10-16 archive now protects them (`org-2026-10-16/org/`), but the LIVE tree
  under `~/.operant/org/` has no scheduled backup — the archive is a manual, dated snapshot.
  A G+1-style retirement of those readers without a recurring archive would leave the live
  tree the only copy.
- **Where**: `~/.operant/org/` + the manual `operant org holon archive` command.
- **Ladder check**: the disposition says the org stores' writers go dark in org mode — the files
  stop *changing*, so a one-time snapshot plus the existing archive is arguably complete.
  **The residual risk is deletion, not drift.** If the owner wants belt-and-braces: a cron row
  (`operant cron add` already proposes the argv — iter-851's O-23 fix) or simply re-running the
  archive command on a cadence the owner chooses.
- **Accept (if scheduled)**: two consecutive dated archives exist without manual intervention.

## G8 — Runner/broker port (O-30, deliberate deferral)

- **What**: Holon's Python runner and broker are unported to Rust. Operant's supervisor refuses
  isolated instances outright (O-11's guard, iter-845) rather than run bodies as the wrong user.
- **Where**: not started, by design — tied to the isolation-root setup, an **owner decision**
  (which agent user, whether the isolation model is used at all in the pilot).
- **Accept (when pulled)**: the plan's E acceptance — Holon's isolation conformance suite passes
  against a Rust runner; the broker refuses every Chair command.

## G9 — TUI org panels (O-31, behind the TUI wave)

- **What**: phase B's TUI panels (roster/board views). The CLI views + doctor lines carry the
  information today.
- **Where**: `crates/operant-cli/src/tui/` — the jcode-parity render discipline (AGENTS.md
  "TUI visual layer") applies; this work ships with the TUI wave, not the OS wave.
- **Accept (when pulled)**: the plan's B acceptance as written for the TUI.

## G10 — Small test-hygiene residuals (O-28 tail)

- **What**: three items the hardening pass left:
  1. `tests/api.rs` skips when `HOLON_REPO` is unset — the skip is *loud* now (prints SKIPPED),
     but the invariant-1 tests still do not run in the default suite. An optional
     `--features holon-it` CI-shaped job (or a documented rule that the battery runs with
     `HOLON_REPO` set, which this session did on every run) would pin it.
  2. The e2e's canned provider answers with a *fixed* completion — a multi-turn body (tool call
     → result → completion) is not exercised; `max-iterations 1` keeps it honest. A second
     canned response (a tool_calls frame then a completion) would cover the loop.
  3. `native_tools.rs` drives four argv shapes; the other 19 tools are pinned only by the
     vendor-drift comparison (the vendor JSON regenerates from the kernel's own registry). That
     is the intended design (one definition); the residual is that a *behavioral* battery of
     all 23 against a scratch instance does not exist. The stop-after-submit + e2e tests cover
     the two that matter end to end (submit, whoami via the run).
- **Ladder check**: none of these have caught or would have caught a real defect this session;
  all are depth-of-coverage refinements. **Fold into G1's post-first-day tuning**, when real
  failures tell us which coverage actually pays.
- **Accept (when pulled)**: each named test exists and runs in the default battery with
  `HOLON_REPO` set.

## Order of attack

1. **G1** (owner) — the checklist's last steps, including installing the adapter into `~/holon`
   and one supervised routine. Everything else waits on or is informed by this.
2. **G3** (one small iteration) — the last-tick stamp; doctor should report clock health the
   day org mode goes live, not after.
3. **G2** (needs a Holon-side prerequisite first; one iteration once it lands) — notify outbound
   completes the Chair loop.
4. **G10** — coverage tuning informed by G1's first day; pull individual items as they pay.
5. **G4, G5, G7** — deferred by their own ladder checks (throughput refinement, bridge-protocol
   ceiling, no-drift disposition); revisit when their trigger conditions hold (multi-body org,
   AFT touched-files field, scheduled-archive decision).
6. **G8, G9** — owner-decision-gated (isolation model; TUI wave); unchanged from the audit.

## Keeping this current

Each G-item that closes gets a line in `AUDIT-2026-10-15.md` §8's sibling pattern (or this file
grows a closure column) in the same iteration that closes it, per the audit's §6 discipline.
