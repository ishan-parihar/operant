# Red-team audit — the subagent swarm and the command-center plan (2026-11-14)

> **STATUS (updated 2026-11-14, after BOTH fix rounds). Read this before acting
> on any row.** This document was written against `main @ 651bc708`. Nine
> iterations landed in the first fix round; a SECOND audit pass then measured the
> swarm again against both reference implementations (jcode `@ f9604362`, codex
> `@ 38cdc4d6`) and produced findings 1-7 of that pass, all of which have landed.
> The rows below are NOT all still open, and a slice that re-fixes a closed row
> is the waste this header exists to prevent.
>
> **Second-pass findings, all landed** (each one iteration, each
> mutation-proven, each recorded in `docs/CHANGELOG.md`):
>
> | # | finding | landed as |
> |---|---|---|
> | 1 | `ToolContext.session_id` was populated only as metadata, so `file_state` and `evidence_recall` ran session-less; the test covering it built its context on the dead channel | iter-2354 |
> | 2 | a delegation record had no parent edge, so the hub could not place a worker under its parent | iter-2355 |
> | 3a | a truncated worker report named a session that NO tool could open — `session_search` required an anchor rowid | iter-2356 |
> | 3b | a swarm pushed every worker's full report into the transcript (25,050 → 4,582 bytes, 5.5x) | iter-2357 |
> | 4 | a restart counted its recovered delegations and threw the count away | iter-2358 |
> | 5 | the breadth refusal said "wait for a worker" for a wait that does not exist | iter-2359 |
> | 6 | a re-dispatched delegation ran the work twice — no idempotency key | iter-2360 |
> | 7 | a swarm was all-or-nothing: one wedged worker held every finished one for the 600 s worker timeout | iter-2361 |
>
> The second pass also **corrected its own finding 3**: it assumed "the pull path
> exists" when `session_search(session_id=…)` silently listed unrelated recent
> sessions. iter-2356 built the path; iter-2357 shrank the push. Both are
> recorded so nobody re-derives the wrong order.
>
> **THIRD PASS (2026-11-15, against `main @ 59bab8e0`).** The second pass fixed
> seven things; this pass asked whether any of those fixes was keyed on a proxy
> that a neighbouring input shape walks past, and audited the swarm against both
> references again (jcode `@ f9604362`, codex `@ 38cdc4d6`). It found eight. All
> eight have landed, each as its own iteration, each mutation-proven.
>
> | # | finding | landed as |
> |---|---|---|
> | 1 | the replay dedup ran AFTER the breadth peek, so a replay was refused exactly when the process was full — the double-spend the key exists to prevent | iter-2366 |
> | 2 | the restart warning counted rows an EARLIER restart had settled (error-text prefix match over a reloaded ledger): 1, then 2, then 3 for one dead delegation | iter-2368 |
> | 3 | `live_only` was honoured by the async registry and ignored by the foreground one, so the "running now" strip drew up to 50 finished workers; `list_live` had zero callers and a doc claiming otherwise | iter-2369 |
> | 4 | `execute_tools` was 1,108 lines owning ten concerns; the pre-flight pass moved out, verbatim (816 lines, 0 mismatches) | iter-2371 |
> | 5 | `session_search` re-parsed the JSON its own struct describes, and the struct carried `#[expect(dead_code)]` on its own fields | iter-2376 |
> | 6 | `SubAgentTool::new` had 9 references, all in two audit docs describing a signature that never existed; zero code callers | iter-2377 |
> | 7 | three compiler-flagged dead hub functions, and `list_pane_width` duplicating the `SPLIT` it was written to derive | iter-2377 |
> | 8 | `SwarmMemberStatus::report_back_to_session_id` — written 3x, read never; operant's parent edge is `SessionEntryV2::parent_id` | iter-2377 |
>
> Plus three dedup cuts (a third `now_secs`, `list_records`/`live_records`
> byte-identical but for one filter, and a bare `30` restating
> `MIN_WORKER_TIMEOUT_SECONDS`) — iter-2378.
>
> **The third pass corrected its own report twice, both worth recording.**
> (a) It first claimed the gallery "ignored the parent edge" by hardcoding
> `report_back_to_session_id: None`. Nothing reads that field at all, so the
> finding was dead surface, not a wiring gap — the fix changed accordingly.
> (b) It first claimed removing `#[serde(alias)]` would make serde REJECT a
> snake_case key. Serde silently DROPS unknown fields: the session id became
> `None` and the call fell through to `list_recent`. That is a worse failure
> than the one claimed, and it is the same "pointer to a store with no door"
> shape iter-2356 closed — so the aliases are load-bearing, not defensive.
>
> **Two invariants the third pass established that are easy to lose.** A test
> that seeds the delegation registry must hold `async_delegation::test_lock()`
> AND drain on drop — there is one registry and one lock for the whole crate, and
> `guard_test_lock` now delegates to it rather than owning a second mutex.
> Second, `LEDGER_PATH` is a `OnceLock`, so only one test per process can ever
> exercise the recovery path; the settle step is split out (`settle_interrupted`)
> precisely so the count is testable without it.
>
> **Where operant is ahead of both references** (do not "harmonise" these):
> tool scoping is computed per child and narrow-only (jcode's production spawns
> get the full default registry; codex has no intersection function at all);
> approval inheritance survives every hop (codex pins delegates to `Never`, jcode
> has no per-member scope); and one atomic `fetch_update` claim with RAII release
> on every exit path beats jcode's per-swarm admission mutex.
>
> | row | state | landed as |
> |---|---|---|
> | 1 (child cwd, no isolation switch) | **MISFRAMED, no code change** — codex never changes a delegated turn's cwd either; its isolation moves capability *roots*, not the directory. operant refuses a differing cwd with the reason, which is a defensible middle. The real gap (no typed isolation value) is not a row here. | — |
> | 2 (zero-tool child says nothing) | **CLOSED before this round** — `flag_no_tools` and the child-prompt NOTE already existed at 651bc708. | pre-existing |
> | 3 (background delegation is RAM-only) | **CLOSED** — opt-in JSON ledger, pending records recovered as failed | iter-2331 |
> | 4 (foreground swarm invisible) | **CLOSED.** The transcript card is reachable now (it was gated on a tool named `swarm` that nothing registers) — iter-2325; the four unreachable render paths went in iter-2337 (504 lines) and the strip/page stubs with their layout band in iter-2346 (net −126). No swarm surface remains that a producer cannot reach. | 2325, 2337, 2346 |
> | 5 (swarm output uncapped) | **MISFRAMED, closed with evidence (2026-11-14, round 4).** The claim was wrong twice. (a) There is a bound: `agent::MAX_DELEGATE_RESULT_LEN` = 32 KiB, applied at the one chokepoint every `delegate_task` result passes through. (b) 32 KiB is not an oversight — it is the measured width of a default swarm: 6 workers x `MAX_WORKER_REPORT_CHARS` (4000) + headings, chosen because the previous 4 KiB default kept worker 1 of 6 and paid for five that were never read. The per-worker cap IS the summariser, applied at the source rather than after the fact. Pinned by `a_delegation_result_gets_the_swarm_budget_and_states_its_unit`. Nothing to change; the residual "no post-hoc summariser" is a preference, not a defect. | closed |
> | 6 (`max_global_workers` has no caller; no `[delegation]` block) | **CLOSED before this round** — `main.rs` calls the setter and `operant.example.toml` documents all five keys. | pre-existing |
> | 7 (`max_iterations` documented 90, enforced 50) | **CLOSED before this round** — both are 50. | pre-existing |
> | 8 (`agent://<name>` does not exist) | **CLOSED** — AGENTS.md rule 3 now names the agent feed | iter-2326 |
> | 9, 16 (`communicate` / `task` / `swarm` map to nothing) | **CLOSED.** `communicate` was already gone from `tool_display.rs`; the three dead `swarm` render arms and their tests went in iter-2337, and the strip/page that fed them in iter-2346. Nothing in this tree registers a tool canonically named `swarm`. | 2337, 2346 |
> | 10 (`Live` is a process fact in a DB projection) | open — by design; it is why the count push-down cannot be pure SQL | open |
> | 11 (tab counts were window counts) | **CLOSED before this round** | pre-existing |
> | 12, 13 (preview read the whole session; no first prompt) | **CLOSED before this round** | pre-existing |
> | 14 (`Show more` does not exist) | **CLOSED before this round** — landed as iter-2235 | pre-existing |
> | 15 (`Archived` has no tab) | open — a decision, not a bug | open |
> | 17 (search haystack includes an always-NULL field) | **CLOSED before this round** | pre-existing |
> | 18 (export paths still load whole sessions) | open — correct for an export; recorded so nobody "fixes" it | open |
> | 19 (the plan's own record was stale in two rows) | **CLOSED before this round** | pre-existing |
> | 20 (the gate can report green without running) | **CLOSED before this round** — `scripts/cargo-bin.sh` resolves a cargo that starts | pre-existing |
>
> Findings 11 and 12 of the fix round are not in this table because they were
> found by that round, not by this audit: the three silent clamps (iter-2330)
> and the constant that contradicted the shipped default (iter-2330). The
> fleet-protocol rows the round closed are in `AGENTS.md`, not here.

Scope: (1) the delegation/subagent machinery as it stands, measured against the
two reference implementations the port names (jcode's `jcode-swarm-core`, codex's
`codex_delegate` + agent roles + message board); (2) the execution of
`docs/plans/plan-2026-11-13-codex-command-center-port.md`.

Method: source reading of both references and of this tree, plus live measurement
against the operator's own database (`~/.operant/database.db`, read-only) and the
running process table. Every number below was measured, not recalled.

## Part 1 — What the two references actually do

**jcode (`jcode-swarm-core`)** scales a swarm by making the *membership record*
the unit of work, not the process: `SwarmMemberRecord` carries `working_dir` and
`report_back_to_session_id`, `MAX_SWARM_MEMBERS = 1000`,
`MAX_SWARM_COMPLETION_REPORT_CHARS = 4000`, `SWARM_OUTPUT_TAIL_TOKENS = 1000`,
`SWARM_TLDR_REQUIRED_OVER_CHARS = 240`, and a `ChannelIndex` routes messages
between members. The lifecycle has 14 states including `Queued` and `Blocked`, so
a member that is waiting is a *state*, not an absence. Output is bounded on the
way back into the parent: a completion report is truncated and summarised before
it is injected.

**codex** scales by making the *isolation decision* explicit and typed:
`SessionIsolation` and `TurnEnvironmentSelection` decide what a delegated turn
can see and touch, `approval_policy` must be `never` for a delegated turn, an
unknown `agent_type` is an error (not a default), and the message board
(`agent-message-board-client`) gives channels, posts and subscriptions with a
512 KiB body cap.

## Part 2 — Findings against this tree (ranked)

Each finding names the measured evidence and what it costs.

1. **A delegated child shares the parent's working directory, with no isolation
   switch.** `delegate_task` builds the child turn from the parent's
   `TurnContext`; `turn_context.rs` stamps the child's session with the *same*
   `cwd` the parent resolved. codex makes this a typed decision
   (`SessionIsolation`); here there is no decision to make, so two agents editing
   one tree is the default, not the exception. Cost: the collision mode the fleet
   protocol spends nine rules mitigating.
2. **A child can be spawned with an empty toolset and say nothing about it.**
   `SubAgentTool` is registered with `parent_toolsets = vec![]` in
   `builtin.rs`, and `register_child_tools` grants 14 tools; a child whose
   requested toolset narrows to nothing still starts, and its "I had no tools"
   is indistinguishable from "I chose not to act".
3. **Background delegation answers live only in RAM.** The async registry is a
   process-local `OnceLock<Mutex<HashMap<..>>>`; a restart forgets every pending
   delegation, and the hub's `Working` count is the only place that loss shows.
4. **Foreground swarm members are invisible to the operator.** The gallery reads
   the same in-memory registry, so a swarm that is running is visible only while
   the process lives and only in the pane that reads it.
5. **Swarm output re-enters the parent's context uncapped.** jcode caps a
   completion report at 4,000 chars and summarises over 240; here the child's
   final text is injected whole, so a 5-member swarm can add tens of thousands of
   tokens to the parent's next turn.
6. **`delegation.max_global_workers` has no production caller.**
   `set_max_global_workers` exists and nothing calls it; the process-wide cap is
   the compile-time `DEFAULT_MAX_GLOBAL_WORKERS = 6`. The config key reads as a
   control and is not one. (`operant.example.toml` also has no `[delegation]`
   section at all, which the repo's own rule forbids.)
7. **`delegation.max_iterations` is documented as 90 and enforced as 50.** A
   config value that disagrees with the code is worse than no value: the
   operator tunes the wrong number.
8. **`agent://<name>` addressing does not exist.** The fleet protocol's rule 3
   names it as the coordination channel for peers sharing files; there are zero
   hits in the tree. The channel the protocol assumes is the `genome feed`, which
   is what this slice used.
9. **Two tool names map to surfaces no tool produces.** `tool_display.rs` maps
   `communicate` → the swarm card and `task`/`task_runner` → the subagent card;
   no such tools are registered, so both render paths are unreachable.
10. **`Live` is a process fact stored in a database projection.** `RowStatus` has
    a `Live` variant, but the row comes from SQL, so the hub patches it after
    loading. Every count that includes "working" is therefore a hybrid of a DB
    query and process state — correct today, and the reason F4's count push-down
    cannot be pure SQL.
11. **The tab counts were window counts.** `compute_filter_counts` counted
    `self.rows` (the newest 500), so a tab reading 0 meant "0 of 500", not "0".
    Fixed in this plan's F4 data layer (the count query exists); the hub-side
    wiring is still open.
12. **The preview read the whole session to show 400 messages.** Measured: the
    largest session on the live store holds 14,103 messages; the load
    materialised every one of them. **Fixed in iter-2212** (this document's
    companion slice): a bounded indexed read plus a store-side prompt count.
13. **The first prompt had no producer.** It was derived from the same full load.
    **Fixed in iter-2212**: a `first_prompt` column written once at turn start and
    backfilled.
14. **The `Show more` row does not exist.** The page cursor landed (iter-2180)
    but no UI asks for the next page, so the 500-row window is still a wall.
    Still open (J2).
15. **`Archived` has no tab.** The `Inactive` filter folds archived rows in, but
    the plan's status ladder has five rungs and the operator cannot see "parked"
    as its own state. A decision, not a bug — recorded here so it is not
    re-discovered.
16. **Two tool names (`communicate`, `task`) and one display name
    (`swarm`) describe a surface with no producer** — see finding 9; kept
    separate because the fix is a deletion, not a mapping.
17. **The hub's search haystack includes a field that is always NULL.** With
    `cwd` unwritten for most rows, searching by directory matched nothing;
    fixed by F1 (iter-2166) and now by the backfill in iter-2212.
18. **`get_session_messages_full` is still the read used by the export paths.**
    The bounded read is the preview's; the export/`insights` paths still load
    whole sessions. That is correct for an export and worth saying out loud so
    nobody "fixes" it into a truncation.
19. **The plan's own execution record was stale in two rows** (I1 and J2 recorded
    as landed when they were not). Amended in this document's companion commit;
    a plan that over-reports its own progress is the cheapest possible way to
    lose a week.
20. **The verification gate can report green without running.** The `cargo`
    wrapper on this host `exec`s `systemd-run` and exits when there is no user
    bus, and a pipeline's exit status is the last command's — so
    `cargo check ... | tail` printed nothing and returned 0. Three "green"
    checks in this session were vacuous before the wrapper was bypassed. Any
    agent verifying here must run `/usr/bin/cargo` (or the `check.sh` wrapper,
    which does) and read the exit marker, not the absence of output.

## Part 3 — The swarm architecture, measured against the references

| axis | jcode | codex | this tree |
|---|---|---|---|
| member record | `SwarmMemberRecord` (dir, report target) | `SessionIsolation` + role | none — the child is a `TurnContext` |
| membership cap | 1000, typed | per-turn | 6 global / 3 per parent (constants) |
| output back into parent | 4,000 chars + TL;DR over 240 | message board, 512 KiB | uncapped |
| lifecycle states | 14, incl. Queued/Blocked | per-turn | 4 (live/pending/done/failed) |
| cross-member channel | `ChannelIndex` | message board | in-memory registry |
| approval posture | n/a | `never` for delegated | inherited from parent |

The one-line version: **jcode scales by making membership a durable record with a
bounded report; codex scales by making isolation a typed decision. This tree has
neither — its swarm is a process-local fan-out with an unbounded report.** Items
1–5 above are that gap, in order of cost.

## Part 4 — What landed from the plan

- **iter-2212** — I1 (the bounded preview read) and I2's producer (the
  `first_prompt` column, written once at turn start, backfilled), plus the
  store-side prompt count. Three core tests and one hub test.
- **iter-2213** — this document and the plan's amended execution record.

Still open: J2 (the `Show more` row) and F4's hub-side count wiring. Both need
the list pane's row model and the hub's count cache, which were in another
agent's hands while these slices ran; they are named here so the next slice does
not rediscover them.