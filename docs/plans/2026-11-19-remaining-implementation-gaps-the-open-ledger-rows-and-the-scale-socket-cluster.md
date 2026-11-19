# What stands after iter-2651 — the open ledger rows, the scale/socket cluster, and the dormant surfaces

Dated-frozen: published `2026-11-19`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a `Resume here` section). This plan is
published as dated-frozen and is **not** added to `living_plans`: it is a worklist
with a measured base, and each row it schedules re-derives its own state when it is
claimed (a stale row invites re-implementing shipped work — the RT-181 trap).

**Status:** dated-frozen WORKLIST. It amends nothing; it is filed alongside
`docs/plans/2026-11-13-thin-client-session-server-plan.md` (dated-frozen, execution
record accurate) and it names the rows that still stand. Where this file disagrees
with a row's checkbox, the checkbox is treated as untrusted until re-derived — per
the ledger's own RG-016 note beside the count.

**Authority:** `docs/audit/2026-06-19-BUGS.md`, "What is OPEN right now".
**Base:** `origin/main` @ `89df7b41` (iter-2651, published 2026-11-19). Every file,
symbol and count below was read from the working tree at that commit. Line numbers
drift; the symbol names are the contract, and every tranche re-verifies its own
anchors in the iteration that changes them.

**What this plan is NOT:** it is not a re-audit (the 2026-11-12 process-architecture
audit, the seven-subsystem fleet audit, and the iter-2490 red-team are the sources it
schedules); it is not an implementation (nothing here is built at publication time);
and it does not change the deployment mechanism (docs-only tranches do not build or
deploy). The `research/` directory in the shared tree is a peer's scratch output and
is not read or referenced by any tranche.

---

## The measured landscape at `89df7b41`

Re-derived with the commands the ledger header names (not read from it):

```text
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md    # → 16
rg -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md    # → 643
```

The 16 open checkboxes, and the section each sits in:

- **Scale/socket cluster** (all in "## OPEN — Process architecture: gateway, server,
  session management (2026-11-12 audit)", opened 2026-11-12): RT-542 (the thin client
  builds no registry), RT-543 (two daemons), RT-544 (plan/comment promise a shipped
  scale story), RT-545 (ACP protocol cannot carry an interactive session — the wire
  protocol that shipped instead), RT-547 (the thin client had no caller — now it
  does, for `chat`), RT-552 (the read-only HTTP session API), RT-527 (a process-
  global makes the same chord behave differently in a later fresh app).
- **The classification defect**: RT-554 (a crashed session is neither flagged
  interrupted nor reachable by `operant continue` with no id; the row is an index
  row with NO checkbox and NO detail section; one half closed in iter-2196).
- **The parked rows**: RT-275 (the unwired public surfaces), RT-280 (the gateway's
  "resume after restart / reset after /stop" flags, implemented, never set),
  RT-285 (three dormant surfaces that look live), RT-293 (two surfaces that claim a
  delivery / a memory engine that do not exist).
- **The carried records**: RT-136 (confirmed-not-a-defect), RT-179 (feature,
  advancing), RT-190 (owner ruling), RT-209 (the turn-attribution consumer).

And **two rows are stale and must be closed before any work is scheduled against
them** (this plan's T1):

- **RT-555** ("A PEER'S TEST IN gateway_commands.rs DOES NOT COMPILE ON
  origin/main") is **STALE**. The "destroyed identifier"
  (`telegr[REDACTED](&vendor)` at `gateway_commands.rs:3767`) was already fixed by
  **f9770403 (iter-2181, 2026-11-13, "the CLI test target compiled again")** — the
  line now reads `telegram_bot_commands(&[])`. The row's checkbox is still open.
- **RT-605** and **RT-610** (fleet-audit findings 6 and 13) shipped in **437474e0
  (iter-2649, 2026-11-19)** — `HookRegistry` generation/reset seam
  (`gateway_pipeline.rs`, `shell_hooks.rs` `SHELL_HOOK_OWNER`) and the script-weighted
  `estimate_tokens` in `context_management.rs` — but both are still index rows
  marked "OPEN", and the ledger header prose at "What is OPEN right now" still says
  they "stay open (a reset seam with no caller is speculative, and the CJK estimate
  is a behaviour change)". Both claims are no longer true.

**What is NOT a defect and must not be closed**: RT-136, RT-190 (owner rulings —
closing them would rewrite a ruled decision), and the RT-555/RT-555 id collision
(RG-021: the later publisher renumbers forward; this plan does not renumber any id).

**Out of scope, named so it is not absorbed:** RT-593 (an operator decision), and
RT-598 (a documented v1 limit owned by the generative-UI plan's successor, tracked in
`docs/plans/2026-11-19-close-the-remaining-generative-ui-gaps-the-gateway-rasterise.md`)
— both stay where they are.

---

## What already landed (do not repeat)

- **iter-2181 / f9770403** — the RT-555 test-target defect is fixed; `cargo check -p
  operant-cli --tests` is clean at the base.
- **iter-2649 / 437474e0** — RT-605 and RT-610 fixed, each with tests
  (`a_concurrently_registered_non_shell_hook_survives_the_reset`,
  `installing_twice_runs_a_hook_command_exactly_once_per_event`; the CJK estimator
  tests in `context_management.rs`).
- **iter-2197 … iter-2239** (thin-client campaign, execution record in
  `docs/plans/2026-11-13-thin-client-session-server-plan.md`) — RT-546, RT-548,
  RT-549, RT-551 closed; the session socket exists, the gateway binds it by default
  (`run_session_socket` in `start_gateway`), the TUI and `chat --attach` render the
  server's stream, the cap and the slot count are one number.
- **iter-2223 / d34177c1** — the stale `gateway_runner.rs` "scale story" comment the
  RT-544 row names was REMOVED there; what remains is the dated-frozen 2026-11-12
  plan's row 7 (see T5).
- **iter-2646 / 452b9d26** — T5's `flock` in `persist_config_key_at` already exists
  as `ConfigWriteLock` in `operant-core/src/config.rs`; the onboarding plan carries
  Amendment 1 saying so.
- **The fleet-audit doc** (`2026-11-18-operant-fleet-red-team-and-ponytail-audit-seven-subsystems.md`)
  needs "**FIXED iter-2649**" markers at findings 6 and 13; finding 1's CI claim was
  already corrected there (iter-2648/2649).
- **The changelog convention**, observed at the base: docs-only iterations
  (iter-2642, 2645, 2648, 2650, 2651) do NOT append to `docs/CHANGELOG.md`; code
  iterations do. Doc-only tranches here follow that convention; code tranches carry
  their entries.

## Mechanism

Each item is ONE iteration: change → scoped test → crate battery → the clippy warning
gate → commit with a fresh `origin/main` number → push → deploy only for code-adjacent
tranches. Docs-only items skip the build. Ordering rule: **stale rows are corrected
first (T1), before any code is scheduled against them.**

**Iteration numbers**: computed from `origin/main` AFTER a fetch (the tree numbering
moved to iter-2651 on 2026-11-19; the whole run below is numbered from there:
T1 = iter-2652 …, but re-derive at claim time — a concurrent agent may hold the next
number).

## Gates / success criteria

(fixed before the run — each tranche clears all of these before it is pushed;
doc-only tranches clear 1, 4, 5 and the ledger gates)

1. `source scripts/dev-env.sh`; `cargo fmt --all` — clean.
2. `cargo check -p operant-core --lib` and `cargo check -p operant-cli --bin operant`
   — **0 errors**; and `cargo check -p operant-cli --tests` — **0 errors** (the
   RT-555 lesson: a red TEST target is not a green build).
3. `bash scripts/clippy-warning-gate.sh` — **no NEW warnings** vs the base (the
   pre-existing set is peer-owned; `--update` is never used to absorb new ones).
4. `bash scripts/check-doc-claims.sh` — exit 0; and
   `./scripts/genome lint` — no NOVEL error vs the base run (the pre-existing set is
   peer-owned; compare by category, not by count).
5. The ledger gate: after any tranche that touches `BUGS.md`,
   `python3 scripts/audit-ledger-index.py` is run and its output is recorded in the
   commit body — it is EXPECTED to fail at the base (RT-554 carries no checkbox;
   RT-555 carries two rows and two sections, a known peer collision) and every
   tranche that touches it must say whether the failure set is unchanged or grew/
   shrank and why.
6. Every code change carries a regression test that fails without it, cited in the
   commit body; mutation-proven where the change is a single call site.
7. Live claims are verified on the deployed binary, never inherited from a source
   read (deploy steps in `AGENTS.md` §7; R&D Protocol).

## Cost

Small to medium. T1, T5, T10 are documentation. T2, T3, T4, T7 are one test each plus
a row flip. T6 and T9 are decision gates (the work behind them is bounded once the
owner rules). T8 is the only one with a mechanism change, and it is bounded by the
three `claim_session_here` production call sites.

| tranche | files | ~LOC | new deps | risk |
|---|---|---|---|---|
| T1 ledger sync | `BUGS.md`, fleet-audit doc | ~40 | none | none |
| T2 RT-542 pin | `gateway_runner/tests.rs` (+ `cmd_chat_attach`/smoke) | ~80 | none | low |
| T3 RT-545 pin | `gateway_runner/tests.rs` | ~90 | none | low |
| T4 RT-547 pin | `tui/adapter_types/tui_app.rs` (`attach_tests`) or `main.rs` contract | ~60 | none | low |
| T5 RT-544 errata + RT-552 guard | 2026-11-12 plan (amendment), gateway_api/dashboard guard | ~120 | none | low |
| T6 RT-543 disposition | `cmd_acp.rs`, `session-server.md`, `cli-surface.md`, `AGENTS.md`, `CHANGELOG.md` | ~60 + docs | none | low (ruling-gated) |
| T7 RT-527 | `tui/app/tests.rs` (make_app), `theme_colors.rs` guard | ~30 | none | low |
| T8 RT-554 | `active_sessions.rs`, `database.rs` (guarded mark), `cmd_sessions.rs`, 3 claim call sites, `sessions.md` | ~150 | none | medium (DB-handle threading) |
| T9 product decisions | `turn_end.rs`/`advisor.rs`/`recap.rs`, `database.rs`, `gateway_session.rs`, `notification_tool.rs`, `memory.rs` | per ruling | none | medium |
| T10 dispositions | `BUGS.md` | ~60 | none | none |

Ten iterations, each independently revertable. T1 lands first (it corrects the base
every other tranche is measured against); T2–T4 are independent and can land in any
order; T5 depends on T2–T4 having run (its errata names them); T6 is ruling-gated;
T7–T10 are independent of T6.

---

# T1 — correct the ledger first (iter-2652)

Zero risk, prevents wasted work. **Files:** `docs/audit/2026-06-19-BUGS.md`,
`docs/audit/2026-11-18-operant-fleet-red-team-and-ponytail-audit-seven-subsystems.md`.

1. **Close the stale RT-555 (test target)**: tick its checkbox with
   "**CLOSED iter-2181 (f9770403)** — the CLI test target compiled again; the
   `telegr[REDACTED]` identifier is gone (`gateway_commands.rs` reads
   `telegram_bot_commands(&[])`). Verified at the base: `cargo check -p operant-cli
   --tests` exits 0." Do NOT touch the sibling RT-555 (TAB ON A SLASH COMMAND, CLOSED
   iter-2209) — the duplicate id stays as recorded in the header's RG-021 note;
   renumbering is the collision owner's call, not this tranche's.
2. **Close RT-605 and RT-610** with receipts `437474e0` (iter-2649): the index rows
   become CLOSED, naming the two mutation-proven tests; the "OPEN — Process
   architecture" prose block in the header ("RT-605 and RT-610 stay open…" and the
   "sixteen" count) is corrected — `rg -c` re-derives **15** open checkboxes, and the
   header must name the derivation, the history of the stale claim, and the two
   closures by id.
3. **Add `**FIXED iter-2649**` markers** to fleet-audit findings 6 and 13 (both are
   currently unmarked).
4. **Re-derive the header prose**: "SIXTEEN" → "FIFTEEN"; the two commands stay the
   authority; the sentence that lists RT-542..RT-552/RT-527/RT-554 as open stays
   accurate after the T2–T10 tranches land (update it per tranche, not only here).
5. **Do not touch**: the peer stashes, `research/`, `.serena/`, the duplicate-RT-555
   rows, RT-553/RT-554's index-row-without-detail state, or the known-failing
   `audit-ledger-index.py` expectation set — record it, don't repair it.
6. CHANGELOG: docs-only, no entry (convention recorded in "What already landed").

**Acceptance:** `rg -c '^- \[ \]'` → **15**; `check-doc-claims.sh` exits 0; the
fleet-audit doc names iter-2649 at findings 6 and 13; the commit body lists the exact
old/new prose for the header block so the diff is auditable as a claim change, not a
wording change.

---

# T2 — RT-542: the pin that an attached client builds no registry (iter-2653)

`operant chat --attach <path>` builds no registry, no agent, no MCP children, no AFT
bridge (`main.rs` Chat arm: "with --attach this process builds NO runtime"; the client
path drains `tui/adapter_types/tui_app.rs::attaching()`; the socket bind is
`start_gateway`'s, `run_session_socket`). The row's remaining claim is that NO test
proves the no-build property.

**Files:** `crates/operant-cli/src/gateway_runner/tests.rs` (new test beside
`the_gateway_session_socket_serves_two_clients_on_one_process`, ~:3260 and the two
smoke tests that pass `None`).

**Test (mutation-proven):** `an_attached_client_builds_no_registry` — start a real
`start_gateway(Some(socket))` instance in-process (the smoke-test fixture with
`Some`), attach through the same code path `chat --attach` uses, and assert:
(a) the attach resolves and Hello round-trips (Hello/HelloOk over the wire, the
`exchange()` helper shape used by the two-client test), and (b) the no-build
invariant: the client process builds no `ToolRegistry` / no `McpManager` / no AFT
bridge — pinned at the seam the row names (a build counter or a source-contract
guard on `cmd_chat_attach`'s absence of `build_registry`, matching the style of the
existing source-guard tests). The two existing smoke tests pass `None` and pin the
`None` path; this pins the `Some` path.

**On green:** RT-542 closes (tick, receipt, one sentence naming the test). CHANGELOG
entry (code-adjacent). The "Open" prose in the ledger header and the index row flip
in the same iteration.

---

# T3 — RT-545: wire round-trips for Cancel, Steer and Queue (iter-2654)

The row asked for "a protocol test that a prompt, its events and a cancel round-trip
over the socket" (written against ACP). The surface that shipped is the session
socket; the unit tests call `GatewaySessionHandler::handle` directly and the two
socket tests cover Hello + SessionBus routing. What is missing is wire-level
round-trips.

**Files:** `crates/operant-cli/src/gateway_runner/tests.rs`.

**Tests (mutation-proven), each over a real bound socket using the `exchange()`
pattern (Hello round-trip helper at ~:4108):**
- `a_cancel_crosses_the_wire_and_stops_the_turn` — send `WireMessage::Cancel`,
  assert the byte reply decodes to the "the turn was cancelled" arm; mutation: break
  the wire encode and the decode must fail.
- `a_steer_crosses_the_wire_into_the_slot_queue` — send `Steer`, assert the reply
  decodes to the accepted arm (and, where a turn is seeded as running, that the
  steer landed in the slot's steer queue — reuse the seeded-running pattern of the
  RT-550 cancel test).
- `a_queue_crosses_the_wire_into_the_next_turn_queue` — send `Queue`, assert the
  reply decodes to the bounded-accept arm (32 cap).

The row's prose is corrected to name the shipping surface (the session socket, not
ACP — the ACP protocol disposition is T6's). **On green:** RT-545 closes. CHANGELOG
entry.

---

# T4 — RT-547: `chat --attach` is reachable and builds no runtime (iter-2655)

The row: "the thin client has no caller" — now `operant chat --attach` is the caller
(I5/iter-2241/2225) and the TUI attach path exists. What remains is a pin at the
ENTRY POINT, so a refactor cannot silently re-grow a runtime under the attach flag.

**Files:** `crates/operant-cli/src/tui/adapter_types/tui_app.rs` (extend the
`attach_tests` module, :~3188) or a `main.rs` contract test beside the existing
`include_str!` contracts.

**Test:** `the_chat_attach_entry_point_builds_no_runtime` — source-contract: the
`Some(Commands::Chat)` arm's attach branch names `--attach` and the no-runtime
wiring, and the non-attach path still builds (both halves, so the pin cannot pass by
deleting the local build). Plus, in `attach_tests`, assert `attaching()` is `true`
iff `attach_socket` is set, and that the reader-spawn is the only agent-event
producer on that path (the module's existing pattern).

**On green:** RT-547 closes. CHANGELOG entry.

---

# T5 — RT-544's frozen errata, and RT-552's source guard (iter-2656)

## T5a — the frozen-plan errata (RT-544, the remaining half)

The `2026-11-12` plan is **dated-frozen** ("a plan that changes is a new plan (or
declare it in genome.toml `core.living_plans`)") and is NOT in `living_plans`. So
the correction is an **appended Amendment 1**, exactly the shape iter-2648/2649 used
on the other plans — never a rewrite of the frozen body:

- Add `## Amendment 1 (iter-2656)` to
  `docs/plans/2026-11-12-process-architecture-integrated-gateway-server-safe-drain-sh.md`
  stating, with the mechanism by which the original row 7 claim died: the wire
  protocol that shipped is the session socket (`run_session_socket`, bound by
  `start_gateway`), NOT ACP-over-socket; the ACP daemon remains a second program
  (RT-543); "the attach point for every CLI session" in the mechanism diagram
  (lines 26/29-30) is corrected by the amendment, and row 7's
  `one_daemon_two_clients_one_shared_child_set` result stands while its
  `operant chat`/`run` attach claim is superseded by the socket; RT-542, RT-545,
  RT-547 and RT-552 are the remaining rows and are scheduled in this plan.
- The AGENT_SLOT_COUNT comment the row names is already gone (iter-2223 /
  d34177c1) — the errata records that too, so a reader of the amendment learns both
  halves are settled.

## T5b — RT-552: prove the socket is the write path and the HTTP API never was

The row: "the gateway's session API is read-only — three GET routes" (all `get()`
today, `gateway_api.rs`; the dashboard is likewise `get()` throughout,
`dashboard_server.rs`). The row's pinned-by wants: "a test that the socket path can
prompt a session the HTTP API cannot."

**Files:** `crates/operant-cli/src/gateway_api.rs` (or a source-contract test in
`cmd_gateway.rs`'s test module).

**Test (two halves, both needed, neither sufficient alone):**
1. `the_gateway_api_exposes_no_write_route` — source-contract over
   `gateway_api.rs` (and the dashboard's routes): every `route(`/`.route(` call
   names `get(` — the mutation target is any future `post`/`put`/`delete` on the
   session API.
2. `the_socket_can_prompt_and_the_http_api_cannot` — integration: reuse the
   T2/T3 socket harness to prompt a session over the socket (the agent's events
   flow), and assert the HTTP API has no corresponding route (the guard in (1) —
   plus, if the HTTP test server is cheap, an explicit 404/405 probe).

**On green:** RT-544 closes (row tick + index row flip). RT-552 closes only when
BOTH halves are green; if the HTTP probe half proves environmental, the row closes
on the source-contract + socket half with the residual named in the commit body —
not silently. CHANGELOG entry for T5a (docs) is skipped per convention; T5b carries
one if code changed.

---

# T6 — RT-543: one ruling, then one iteration (iter-2657) — DECISION GATE

**This tranche does not start without an owner ruling.** The two options, and the
recommendation this plan records so the ruling is informed:

- **(a) Deprecate the ACP daemon socket, keep stdio** (RECOMMENDED). Editors and
  coding agents already speak ACP over stdio; the socket half of
  `operant acp server` duplicates exactly what the gateway's session socket now
  serves better (the same 0600 gate via `socket_auth`, the same slots, the same
  child set, and the channel surfaces). Deprecation = a loud warning naming
  `operant gateway run --socket` as the replacement, a CHANGELOG entry, and the
  doc edits listed below. The `one_server_one_child_fleet`-shaped pin the thin-client
  plan wanted is replaced by: "the ACP socket server refuses to be a second default"
  (a warn-on-start test) plus the existing `one_daemon_two_clients_one_shared_child_set`
  staying green.
- **(b) Full retire of `operant acp server`** (the stdio server stays — it is the
  editor integration; retiring it is a separate, breaking change and is NOT what
  RT-543 asks). Removes `AcpSubcommand::Server`'s socket arm, the dispatch, the
  include_str contract references, and the CLI surface rows.

Whichever ruling lands, the same file set is edited in the same iteration:
`crates/operant-cli/src/cmd_acp.rs` (the warning or the removal),
`crates/operant-cli/src/main.rs` (`AcpSubcommand`/docs if (b)),
`docs/library/session-server.md` (lines 15, 33, the "The first is the ACP daemon"
framing, the 60-82 flock/0600 passage naming the second server, the "future unified
server" closing passage), `docs/library/cli-surface.md` (`acp` row), `AGENTS.md`
(line ~1363 command list + the gateway surface notes), `docs/CHANGELOG.md`.

**Do not** do (b) unilaterally; **do not** leave the row half-flipped. If the owner
rules neither, the row stays OPEN by ruling and this plan is amended (not silently
extended).

---

# T7 — RT-527: the process-global palette stops leaking across apps (iter-2658)

`tui/app/tests.rs`'s `make_app()` already serializes construction on a theme test
lock but does not restore the palette; `theme_colors.rs::set_active_theme` also syncs
role overrides, so a chord rebound under one palette behaves differently in a later
fresh app in the same process. Precedent for the restore-guard shape:
`config_shim::reasoning_override_test_guard()`.

**Files:** `crates/operant-cli/src/tui/app/tests.rs` (make_app resets the default
palette + role overrides under the same lock, then drops a restore guard), and — if
a production-side leak is actually reachable — `theme_colors.rs` gains the same
drop-restore. **Production first-frame write stays untouched.**

**Test (mutation-proven):** `two_fresh_apps_bind_chords_identically` — build app A,
rebind a chord under theme X (Alt+B, `key_handling.rs`, `move_word_backward`),
tear it down; build app B, press Alt+B, assert the same action fires
(mutation: remove the reset and B observes X's rebinding).

**On green:** RT-527 closes. CHANGELOG entry.

---

# T8 — RT-554: the interrupted flag, made honest and visible (iter-2659)

The row's one half closed (iter-2196: `most_recent_session_by_activity`). What
remains is the CLASSIFICATION half; both routes are recorded in the row as design
decisions, and this plan picks **route (a)**, with route (b) kept as the fallback:

**Route (a) — the claim-time mark.** `prune_dead_presence` drops dead-pid presence
records on the next claim, so the evidence ("this session's owner died mid-flight")
is transient and currently vanishes unrecorded. Change `active_sessions.rs` to also
surface the pruned records' ids (a sibling `prune_dead_presence_ids()` or a widened
return — an API change, so it carries a regression test), then at the three
`claim_session_here` production call sites (`tui_app.rs` :~642/:~1727,
`herdr_lifecycle.rs` :~94) — where a `Database` handle already exists — call a new
`Database::mark_session_interrupted` guarded to only write when `end_reason` is
NULL (never overwrite "completed"). The marker's meaning changes from "the session
that was cut off" to "a session whose owner died while it was claimed", which is the
meaning `end_reason = 'interrupted'` already carries for the TUI teardown path —
both now agree.

**The `sessions` surface (the operator-visible half):**
`cmd_sessions.rs` (`cmd_list_filtered`, ~:161) gains a `Status` column derived from
`RowStatus` (interrupted/idle/archived — the row shape already carries it; the
column is one `println!` change) and `cmd_browse` (~:455) prints the same marker on
the FuzzySelect rows; the `--interrupted` filter keeps working unchanged.
`gateway_commands`'s `/sessions` already prints `(interrupted)` badges (its
`join_bindings_to_transcripts` maps the same `RowStatus`).

**Route (b) — if the DB handle cannot reach** (a claim site without a database, or
`active_sessions.rs` must not grow an I/O dependency to core): derive "interrupted"
at prune time inside the presence registry itself is rejected (the registry has no
database); instead, `sessions list` gains the `Status` column regardless (the
banner/badge half is free), and the row is closed as "classification half resolved
by making the flag visible; claim-time marking rejected as unthreadable at the
three call sites" — recorded as a documented limit in `docs/library/sessions.md`.

**Also in T8:** `docs/library/sessions.md` gains the claim-time behavior beside the
exit-teardown marker (they must not contradict each other).

**Tests:** a `mark_session_interrupted` NULL-guard test; a prune-returns-ids test;
a `sessions list` output test (or the existing list tests updated for the column);
the `--interrupted` filter regression stays green.

**On green:** RT-554's remaining half closes — with its "index row with no checkbox"
state RESOLVED by ticking a real checkbox (the row gains a checkbox in this
iteration, closing both the classification defect and the RG-016-adjacent
irregularity, or route (b) documents the limit and the row stays open-by-decision
with the banner shipped). If the owner instead wants route (b) explicitly, amend
this plan — not the row — in the same iteration.

---

# T9 — the product-decision rows that need a ruling (iters 2660-2663) — DECISION GATES

Each row is **parked until the owner rules**; the plan records the recommendation and
the work once ruled. No tranche below starts without the ruling.

- **RT-209** (the turn-attribution subscriber): `log_attribution` at the seam is
  already the consumer (iter-1661); no SUBSCRIBER reads `TurnEnd::attribution`.
  Recommendation: subscribe in `advisor.rs` and `recap.rs` (both read
  `end.session_id` today, both are optional-but-present hosts) with a `debug!` or
  the advisor card's existing shape; the cost is one read each. Alternative:
  re-scope the row to "consumer delivered at the seam (iter-1661); event-carried
  value intentionally unread" and close it as ruled-deferred.
- **RT-275** (the unwired surfaces): recommend DELETE the dead surfaces — the
  checkpoint store pair `store_checkpoint`/`get_checkpoint`, the tool-state pair
  `set_tool_state`/`get_tool_state`, the `write_approval::{stage,list,get,discard}_pending`
  path (zero production callers anywhere), and the three schema-only config types
  `ReliabilityConfig`, `MemoryPolicyConfig`, `MultimodalConfig` — with a CHANGELOG
  note that they were never live, per the iter-2610 deletion precedent (prove by
  removal). Alternative: park with a `#[doc(hidden)]` note; the recommendation is
  deletion.
- **RT-280** (`suspended` / `resume_pending`): recommend REMOVE — both fields have
  zero production callers, `prune_old_entries`' suspended branch is dead, and the
  `/stop` auto-reset behaviour was never wired; the honest record is a removal entry
  in the CHANGELOG, not a retained illusion. Alternative: wire them (auto-reset
  after `/stop`, resume-pending on restart) — an operator-visible behaviour change
  the plan cannot make silently.
- **RT-285** (three dormant surfaces): (1) `TurnRetryState`'s restart signals
  (`restart_with_compressed`, `restart_with_rotated_credential`,
  `rebuilt_messages`) — recommend DELETE (unread public API, `ContentPolicyBlocked`
  never handled; no Autocompactor file exists to confuse it with). (2)
  `SessionStore::discard` — recommend either the durable implementation (drop the
  in-memory copy AND the DB rows, matching its own doc's claim; it is a destructive
  op so the caller sites — `agent/events.rs` — get a `#[must_use]`/confirm) or a
  rename to make its in-memory scope explicit; owner's call. (3) The
  `Autocompactor` table entry in the row is already stale (no such file exists in
  the tree) — recorded, not scheduled.
- **RT-293** (two surfaces that lie): (1) the `notify` tool — recommend RE-SCOPE
  honestly: description loses "Send a notification", result loses `"delivered":
  true`, the tool name/registration may stay (agents call it) but its claim becomes
  "log a notification intent"; the alternative (a real sink) is a feature nobody
  asked for. (2) `MemoryManager` — recommend DELETE (memory-wire is the only
  engine; the field is write-only; every method has zero production callers) with a
  CHANGELOG note.

Each ruling either lands as one iteration or re-scopes the row as a documented
acceptance (parked, in the CHANGELOG). Rulings that change observable behaviour are
listed in the CHANGELOG of their iteration.

---

# T10 — the disposition lines (iter-2664)

Doc-only. **Files:** `docs/audit/2026-06-19-BUGS.md`.

- **RT-136** — confirm and keep OPEN as a record (the two unused tables are labelled
  NOT READ on purpose; nothing to build) — one sentence, no checkbox change.
- **RT-179** — stays OPEN (feature advancing); the row already says what remains;
  no work scheduled; one sentence noting this plan does not advance it.
- **RT-190** — stays OPEN by owner ruling (memory bank topology); nothing scheduled.
- **RT-199** — closed (iter-1505); RT-209 handled in T9.
- **RT-593** — operator decision; explicitly out of scope; not scheduled.
- **RT-598** — peer-owned (generative-UI successor); explicitly out of scope; not
  scheduled.
- **RT-175 / RT-176 / RT-553** — CLOSED (iter-1298, iter-1284, iter-2165/2182);
  one sentence each so a reader does not re-audit them.
- **Ledger bookkeeping:** re-derive the open count after T1–T9 (`rg -c`), update the
  header, and re-run `python3 scripts/audit-ledger-index.py`; record the failure set
  in the commit body. If every checkbox the plan touched landed, the set shrinks by
  exactly the earned rows and nothing else.
- **Index flip:** RT-542, RT-544, RT-545, RT-547, RT-552, RT-527, RT-555,
  RT-605, RT-610 and RT-554 (if route (a) lands) all get their State cells updated
  in this or their own tranche — each with its receipt.

---

## Order and dependency

**T1 → (T2, T3, T4 independent) → T5 → T6 (ruling) → {T7, T8} → T9 (rulings, in any
order) → T10.**

T1 first because it corrects the base every gate is measured against. T2–T4 next:
they are the thin-client campaign's remaining receipts and are independent of each
other. T5 depends on T2–T4 having run (its amendment names them). T6 is a ruling,
not code — it can be requested at any time and blocks only its own files.
T7 and T8 are independent of T6. T9 waits on rulings; T10 last so the disposition
and the counts are written once, after the earned closes.

## Explicitly NOT in scope

- RT-593 (operator decision), RT-598 (peer-owned generative-UI v1 limit), RT-179
  (advancing feature), RT-190/RT-136 (rulings/records).
- Any edit to `docs/kb/feed.md`, `research/`, `.serena/`, or peer stashes.
- Re-auditing closed rows (RT-175, RT-176, RT-540, RT-546, RT-548, RT-549, RT-553,
  RT-584, RT-585…).
- Renumbering the duplicate RT-555 (RG-021 — the collision owner's call).
- Repairing the known-failing `audit-ledger-index.py` expectation set by any means
  other than the earned row closes (no bulk re-mark, no `--update`).

## Re-measuring after the run

```text
git fetch origin
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md   # 16 at base; 15 after T1; 12–13 after T2–T5
rg -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md
python3 scripts/audit-ledger-index.py             # expected fail set, recorded per tranche
bash scripts/check-doc-claims.sh
./scripts/genome lint                            # compare by category, not count
```

**Cost of the whole outline:** ten iterations, no new dependencies, no crate
changes except `operant-cli` test targets and the three small core edits named in
T7/T8/T9. Each tranche is independently revertable; T1 is pure documentation and can
never regress behaviour.

**Done means:** the ledger's open set is honest (every open row names its receipt
or its ruling), the scale/socket cluster's claims are pinned to tests at the wire,
and no row is open because it was never re-read. The pain this plan exists to
remove: stale "open" rows and scale claims nobody measured.
