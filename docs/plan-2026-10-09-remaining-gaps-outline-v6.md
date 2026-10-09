# Remaining implementation gaps — reconciled outline v6 (2026-10-09)

> Supersedes the execution queue of
> `plan-2026-10-09-remaining-gaps-outline-v5.md` (its row 1 landed as
> iter-734 `4c699106`; its row 4 was corrected and partially closed at
> iters 732/733). The peer's
> `plan-2026-10-09-remaining-gaps.md` (Telegram/org track) and
> `plan-2026-10-09-tui-live-audit-bugfix-plans.md` (TUI waves) keep
> ownership of their rows; carried here by reference. Every blocker
> state below was re-verified at tip `4c699106`.

## §0 State verified at the tip (`4c699106`, 2026-10-09)

- **Landed since v5** (five commits, `f78dde93` → `4c699106`):
  - iter-730 (peer): TUI P1-7 — rotating tips off by default.
  - iter-731 (peer): TUI P3-10 — SSRF DNS failures name the condition;
    live config migrated off `igs`.
  - iter-732 (this line): row 4's one pure-test remainder —
    `duplicate_progress_spam_survives_the_output_batch_reset`.
  - iter-733 (this line): phantom "progress-oscillation" rung deleted
    (config.rs + example.toml); v5's row 4 corrected in place with an
    appended audit note; row 1's stale BLOCKED note retired.
  - iter-734 (this line): **row 1 landed — both slices** (consumer
    mount + registry attach).
- **The `5193f155` escalation-regression flag: unchanged.** The 10s
  deadlines are still in place (`seat_policy_run_path.rs:375`, `:431`,
  `:530`); the load-sensitivity diagnosis stands (gate 7522/1 at
  iter-729). Still the peer's track.
- **No blockers outstanding in this line's queue.** Next unblocked
  slice: §1 row 2 (offload/TOC).

## §1 Core execution queue (ordered)

1. **~~Gateway ledger consumer mount + registry attach~~ LANDED
   iter-734 `4c699106`.** The v5 "BLOCKED on the peer's WIP" note was
   false (the overlay is a rolled-back copy — see §5). Both slices
   shipped in `start_gateway`: the sender loop settles each
   `CronDelivery` via `CronDb::mark_delivery_outcome` (success terminal,
   failure leaves the row pending for the 2h reclaim; needed an
   `Arc::clone` of `cron_db` taken before the WriteBarrier match, whose
   both arms moved it), and `SeatAuthority::new` carries
   `.with_employee_registry(Arc::clone(&employee_registry))`.
2. **offload/TOC (`ArtifactIndex`) — NEXT UNBLOCKED SLICE.** iter-729
   wired the trim (`head(2/3)+tail(1/3)`, default 50000) but oversized
   results still enter history as truncated text with no retrieval
   path. The follow-up: ingestion-time offload of oversized results to
   a durable artifact with a TOC stub in the message. Verified still
   open at this tip: zero matches for
   `artifact_index`/`ArtifactIndex`/`artifactindex` across `crates/`,
   and zero `offload` matches under `operant-core/src` +
   `operant-runtime/src`.
3. **Vision-routing hardening test** — RESOLVED BY AUDIT at v4: the
   port already exists (`[multimodal]` config → `from_config_with` →
   per-request preflight + lazy route, incl. the delegate sub-agent
   path). Remainder: one run-path test proving a config-file
   `vision_provider` routes a marker request through a full facade
   turn. Hardening, not a gap.
4. **~~openhuman adversarial suite remainder~~ CORRECTED + CLOSED
   (iters 732/733).** The row's premise was false: no
   `no_progress/mod_tests.rs` ever existed in-tree (upstream-only; the
   port plan declined to create the path), and `pattern_tests` holds
   12 tests, not 13. The one uncovered pure case landed at iter-732
   (mutation-proven). The other two named faults are production work,
   re-queued below: oscillation = BUGS.md S7; progress-token stall =
   Wave-2 detector design.
5. **`ObserverEvent::CronDeliveryOutcome` emit seam — NEW, found while
   landing iter-734.** The variant exists (`observer.rs:112`, Display at
   `:334`) and iter-734's commit body documented the deviation, but
   nothing emits it: the gateway holds no observer handle (grep: zero
   matches in `gateway_runner.rs`). The ledger is the contract that
   matters, so this is observability polish, not a correctness gap —
   decide `observe`-seam wiring or `wire-or-delete` the variant.
6. **Production work re-queued out of row 4** (not tests):
   - **S7 (BUGS.md)**: `ping_pong_cycles` matches tool NAMES only, never
     result hashes — an A/B alternation returning identical output every
     cycle escalates as ping-pong Break instead of no-progress. Fix:
     carry result hashes in `name_window` (`tool_guardrails.rs`).
   - **Progress-token stall** (Wave-2): no "progress token" concept
     exists; needs a new detector (a `RepeatPattern` variant +
     claim-vs-action comparison in `observe_output`) — a design decision
     before any test.
7. **Peer's skills discovery cost (their track, tracked here for
   visibility)** — the §0 load-sensitivity finding; fixed by them in
   their skills/TUI line or by relaxing the deadline-sensitive
   run-path tests. Not this line's slice.

## §2 Trinity ports (interleave by size)

| Port | Status |
|---|---|
| P0-A recovery semantics (grace window, CAS terminal write, two-store reconcile) | partial — close-at-detection absorbed at iter-691; grace/CAS/two-store open |
| P0-B error-code taxonomy | **DONE** (iter-691) |
| P1-A per-channel dispatch breaker, half-open probe | **UNBLOCKED by iter-734 — next trinity port.** Hook is `gateway/mod.rs:526 send_to_platform` (it resolves the **platform-adapter** key — `self.adapters.get(platform)` — and returns `Result`; the channel id rides unindexed on `OutgoingMessage`, `gateway/types.rs:192-205`, and must be plumbed into the breaker's channel+failure-class key); NOT the cron loop — the cron sender is a separate spawn (`gateway_runner.rs:3944-3982`) and `send_to_platform` is the shared seam for ~19 call sites. No breaker exists anywhere (verified: TurnLease is inbound-session serialization, DeliveryLedger records without gating, PollRecoveryState is poll-loop debounce). Design: `plan-2026-10-08-trinity-audit-and-remaining-outline.md:23-25` |
| P1-B effect-scoped idempotency on outbound sends | open |
| P1-C canary invariant harness (E-01/E-02/E-06) | open |
| P2-A lease/retry redelivery cap + poison-park | **DONE** — core ledger iter-722 + the consumer-mount remainder iter-734 |
| P2-B heartbeat liveness, `unsupported`-vs-`stale` hinge | open |
| P2-C execution integrity at terminal-write time | open |
| P2-D CAS/RECONCILED discipline + capacity slots | open |
| P2-E credential encryption at rest + rotation | open |

## §3 Peer tracks (owned by their docs; not re-planned here)

- **TUI live audit waves** (`plan-2026-10-09-tui-live-audit-bugfix-plans.md`,
  doc last touched iter-722 — state below read from the commit log):
  P0 landed (P0-2/N-3 iter-725, P0-1 iter-726; P0-4/N-4 landed
  cross-track as iter-729's `max_tool_result_chars` wiring), P1-6
  iter-728, P1-7 iter-730, P3-10 iter-731. **Remaining: P1-5 scroll
  clamp, then P2-8 terminal scrollback** (multi-iteration, design doc
  first, only after P1-5), plus the carried ledger (`last_msg_area`
  deletion, right-click corpus scenario, multibyte selection unit,
  `tui/latex.rs` wire-or-delete, Up/Down history-exhaustion scroll
  fallthrough, `/cls` registered-no-arm).
- **Skills/meta-skill-creator**
  (`plan-2026-10-09-meta-skill-creator-nested-references.md`): iter-723
  landed; carries the §0 load-sensitivity finding.
- **Telegram/org track** (`plan-2026-10-09-remaining-gaps.md`, doc last
  touched iter-711/721): inbound bugs fixed iter-704; feed class
  slices A/B/D executed (709/715) with Slice C owner-gated; Discord/
  Slack read adapters blocked on owner credentials.
- **Standing org queue** (unchanged from v5 §3): worklog scope gate →
  feed/DM read adapters → seat-budget provisioning CLI → two-tier
  memory → org loop guard (GAP-2.1) → teams table + Team path → org
  check exit codes → CEO loop → AD-RG → `retention_gc` wire-or-delete.
  **IdentityGate: HOLD** — finished, tested, unmounted.
- **Unowned red tests — CORRECTED by the iter-736 audit (one row was
  stale, one was mis-measured, one was missing):**
  - **K-1 RETIRED as a defect.** `tools::kernel` roundtrip is **green in
    the main tree at this tip — 7/7 measured** (ping_roundtrip,
    harness_apply_and_rollback_roundtrip, exec_state, probe, tool_bridge,
    transparent_restart, import_allowlist). The clean-worktree red is
    the documented fleet-rule-6 precondition (uninitialized
    `vendor/prime-agent` + untracked `kernel-sidecar/.venv`),
    root-caused at `kernel-sidecar/kernel_sidecar/vendor.py:24-45`
    (`_HAS_PRIME_RUNTIME`) — both prerequisites are present in this
    tree. BUGS.md's row cites `kernel/mod.rs:285` (stale; the asserts
    are at `:274`/`:275`) and calls the cause undiagnosed — both wrong.
    Action: fix BUGS.md, do not spend an iteration.
  - **K-2 keeps a row, re-characterized.** Measured at this tip: the
    narrow filter (`--lib -- loop_request_timeout`) is green 3/3; the
    broad `--lib -- config` battery was red in **2 of 4 runs** — a real
    intermittent flake, but not BUGS.md's hypothesized mechanism:
    `config_tool.rs:456-458` explicitly restores the process-global to
    defaults (the contamination is already guarded), and the asserted
    path reads only per-agent fields (`agent/mod.rs:2810-2862`,
    `events.rs:23-29`). Measured before re-planning; do not trust the
    "order-dependent" label until re-measured.
  - **NEW — peer-owned red, not a K-class row.**
    `config::tests::agent_block_parses_guardrail_exempts_and_env_knob_defaults`
    is deterministically red **in the shared tree only** (fails in
    isolation, every run) from the peer's in-flight `agent/` WIP (5
    files dirty); green at origin/main per their iter-731 commit body.
    §5 rule 7 territory — don't fix, don't block, don't mistake it for
    an unowned red when running the battery.

## §4 Config/ops hygiene

- **CHANGELOG released-section contamination** — v5 recorded
  "recent iters misfiled"; the audit found it worse: the
  iter-716/714/710/709 block is duplicated **5×** across
  `[0.2.1]`/`[0.2.0]`/`[0.1.4]`/`[0.1.3]`/`[0.1.2]` (`iter-716` alone
  appears 9 times). Recent iters (730-734) are correctly filed under
  `[Unreleased]`. A cleanup slice should be scoped against the full
  duplication, not the tail.
- **BUGS.md stale headers** — D-4's header still reads OPEN but the fix
  is in code (`gateway_runner.rs` passes `true` for the gateway agent,
  `false` for cron, with the LCM-maintenance comment pinned). Same
  failure mode that sent one agent re-planning the fixed D-1.
- **Cron hygiene** — ephemeral test regs archive post-smoke-test; DUE
  backlog triage (paused jobs' past `next_run_at` is a pause
  consequence, not a fault).
- **LTO marker rule** — shipped-binary marker checks must be
  reachable-marker only.
- **Peer's clippy `expect()` deny sites** (`agent/stream.rs` — the
  `#[expect(clippy::expect_used)]` attributes at **:447** and **:611**;
  the earlier `:524/:544` refs were wrong and propagated four doc
  generations unchecked) — theirs (rule 7); annotate only if still
  present after their TUI waves settle.

## §5 Gate + concurrency hygiene (session-proven rules)

- Gate in the isolated worktree (`wt-670`); solo-green after a
  full-gate failure = load flake; a failure that reproduces at the
  prior commit alone is real, pre-existing, and NOT the current
  slice's.
- **Push the moment the gate clears.** An unpushed commit in a shared
  tree is not safe.
- **Never commit without a pathspec while the peer's WIP is staged in
  the index** — and verify the staged set first: this session opened
  with **139 of the peer's files staged** (their whole TUI overlay).
  The pathspec form (`git commit -m … -- <paths>`) commits the named
  paths' worktree contents and disregards the rest of the index; a
  bare `git commit` would have swept them all.
- **A peer file's `MM` can be a stale rolled-back copy, not active
  work.** Proven this session on `gateway_runner.rs`: the index blob
  equalled `3429363f^` (one commit stale, having dropped iter-720's
  116-line test) and the worktree blob matched no commit while
  reverting iter-704's `entry_for_source` fix. Check blobs against
  commit history before believing a blocker note — and preserve the
  bytes (`cp` aside) before rebuilding from HEAD.
- **Preserve → rebuild from HEAD → edit → commit pathspec → re-apply
  the peer's regenerable delta.** This session's `config.rs` pattern:
  the peer held a 6-line rustfmt delta on the file; the delta was
  re-applied to the worktree after the push (`rustfmt --edition 2024`),
  leaving the commit minimal and the tree's intent intact.
- **Docs edits: always rebuild from `git show <remote-tip>:docs/<file>`.**
  The staged `docs/CHANGELOG.md` was a pre-iter-730 base whose
  committing would have deleted the 730/731 entries — pure stale
  residue, but only the blob comparison proved it carried no unique
  content.
- **The peer takes your label mid-session.** iter-731 was pushed while
  this line's audit was running; the plan's own iters shifted 731→732,
  732→733, 733→734 in-flight. Re-fetch and hand-compute the next label
  before every push; after a push, `git log origin/main --oneline |
  grep -c "iter-N"` must print 1.
- Iteration labels run concurrent between the lines; renumber before
  push and leave an append-only note.

## §6 Retired rows (do not re-plan)

- Dispatch consolidation / Loop-B deletions — executed (iters
  608/663/669).
- Telegram credential blocker — retired (live token; delivery hop
  closed; inbound bugs fixed iter-704).
- Cost ledger / seat budgets, desktop/plugin surface, ZeroRelay fleet,
  SOP engine, Landlock sandbox — rejected with evidence (work-outline
  §7, 2026-10-09).
- Delegation governance (712), guardian-LLM arm (713),
  micro-compaction (714), delivery-ledger core (722), config-file
  wiring (724), vision-routing "port to the facade" (resolved by
  audit, v4), `max_tool_result_chars` trim wiring (729), openhuman
  adversarial pure-test remainder (732), phantom-rung + outline
  corrections (733), gateway consumer mount + registry attach (734),
  trinity P2-A (core 722 + consumer 734) — landed.

---

## Audit pass appended 2026-10-09 (iter-736)

Every concrete claim in this outline was re-verified against the tree
at tip `0b39222d` — an independent read-only subagent for the trinity/
gate rows plus direct measurement for the test rows. Verdicts:

- **HOLDS**: §1 row 2 (offload/TOC — zero `artifact_index`/`offload`
  matches; the iter-729 trim's production call site is
  `operant-runtime/src/agent/agent.rs:1470`), row 3 (vision port at
  `reconciled.rs:2026`/`:2066` + `delegate.rs:1285`), row 5
  (CronDeliveryOutcome zero emitters — the only two observer mentions
  in gateway_runner are the iter-734 comment), row 6a
  (`ping_pong_cycles` is a `VecDeque<String>` — name-only, S7
  confirmed), row 6b (zero progress-token matches); §2 P2-A (both
  halves; exactly one production `mark_delivery_outcome` caller at
  `gateway_runner.rs:3958`, file clean at HEAD), P0-A (grace/CAS/
  two-store all genuinely open — `check_interrupted_turns_in` never
  compares the timestamp it reads, `save_turn_state_in_for` is a blind
  write), P1-B, P2-B, P2-C, P2-D, P2-E (all open; scope notes: the
  guarded UPDATEs at `cronjobs/db.rs:806/836/843` are a CAS precedent
  scoped to P2-A only, and P1-B's settle-after-send is not a pre-send
  claim — a crash between send and settle re-sends), P1-C (zero
  E-01/E-02/E-06 matches); §3 IdentityGate HOLD (13 tests, zero
  production `.with_org_gate` callers); §4 CHANGELOG 5× duplication.
- **CORRECTED in place above**: P1-A's key nuance (platform adapter,
  not channel key); K-1 retired (green 7/7 measured; environment
  precondition, root cause named); K-2 re-characterized (real
  intermittent flake, 2-of-4 battery runs; the env-mutation mechanism
  is refuted by `config_tool.rs:456-458`); §4 clippy refs (real sites
  `:447`/`:611`).
- **Non-redundancy**: every row that also lives in another doc is a
  pointer (S7 → BUGS.md; trinity design → the trinity doc; peer
  tracks → their own docs); no duplicated work is queued across the
  corpus. Relevance confirmed: row 2 is the next unblocked core
  slice, P1-A the next trinity port, and the re-queued rows (S7,
  progress-token stall) are production work, not tests.
