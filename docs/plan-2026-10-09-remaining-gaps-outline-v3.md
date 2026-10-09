# Remaining implementation gaps — reconciled outline v3 (2026-10-09)

> Supersedes the execution queue of
> `plan-2026-10-09-remaining-gaps-outline-v2.md` (its row 1, delivery-ledger
> durability, landed as iter-722 `065f8d56`: gate 7511/0 at the merged tree).
> The peer's `plan-2026-10-09-remaining-gaps.md` (Telegram/org track) and
> `plan-2026-10-09-tui-live-audit-bugfix-plans.md` (TUI waves) keep
> ownership of their rows; carried here by reference. Every blocker state
> below was re-verified at tip `b30b98e7`.

## §0 State verified at the tip (`b30b98e7`, 2026-10-09)

- **Landed since v2:** iter-722 durable cron delivery ledger (trinity
  P2-A core: `cron_deliveries` v3 migration, queue-before-handoff,
  `mark_delivery_outcome` consumer API, park tombstone at 3 attempts,
  `max(3×timeout, 2h)` stale-claim reclaim replayed before the due-jobs
  gate, `ObserverEvent::CronDelivery*` variants). Peer landed in
  parallel: iter-720 their small-core-debts fix (`3429363f` — context
  items ts index, record_feed seat-map test), iter-719/721 doc
  close-outs, and the iter-722 TUI live audit (six user reports repro'd
  first-hand with code anchors — their track, §3).
- **Blockers unchanged:** `gateway_runner.rs` still carries the peer's
  WIP deltas (two core slices wait on it, §1 row 2);
  `crates/operant-core/src/config.rs` is CLEAN at the tip → the
  config-wiring batch (§1 row 1) is unblocked and is the next slice.
- **v2's corrections stand:** the old §2 dispatch-consolidation row is
  retired (executed by the Wave-1/2 harvests — Loop B deleted iter-663,
  `loop_detector` harvested into `tool_guardrails`); the agent lives in
  `crates/operant-core/src/agent/` (the `operant-agent` crate is gone);
  `max_tool_result_chars` is a schema-only knob in operant-config.

## §1 Core execution queue (ordered)

1. **Config-file wiring batch** — UNBLOCKED, next slice.
   - `guardrail_exempt_tools` config-file schema surface (the
     approval-path exemption knob; the runtime predicate exists, the
     config key does not).
   - `OPERANT_GUARDIAN_LLM` / `OPERANT_MICRO_COMPACTION` config-file
     surfaces — env vars landed (iter-713/714); config parity only.
2. **Gateway ledger consumer mount + registry attach** — BLOCKED on the
   peer's `gateway_runner.rs` WIP. Two small slices on the same file:
   - Consumer mount: the sender loop settles each `CronDelivery` via
     `CronDb::mark_delivery_outcome` (emitting
     `ObserverEvent::CronDeliveryOutcome`) — without it, ledger rows
     linger pending until the 2h reclaim replays them (correct but
     wasteful; the outcome write closes the loop).
   - Registry attach: `.with_employee_registry(Arc::clone(&employee_registry))`
     at the `SeatAuthority::new` site — Bounded delegation fails closed
     without it (latent: no production row sets Bounded yet).
3. **Vision-routing ruling** — `with_vision_route`/`with_multimodal`
   tests+ACP only (`operant-runtime/src/agent/reconciled.rs`,
   `tools/delegate.rs`); `multimodal` deleted at iter-663. Port
   per-iteration vision routing to the facade, or formally accept
   single-provider vision at construction. Ruling, then one slice.
4. **`max_tool_result_chars` wire-or-drop + ingestion-time
   offload/TOC** — PRIORITY RAISED: the TUI live audit's N-4 found the
   live symptom — `http_request` chained raw fetch results charged
   50k tokens (26% of context) because fetch results are untrimmed.
   Either wire the truncation at tool-result ingestion or delete the
   knob; the offload/TOC (`ArtifactIndex`) shape remains the open
   follow-up.
5. **openhuman adversarial suite remainder** — 486-LOC
   `no_progress/mod_tests.rs` fault-injection suite; the 13 pattern
   tests landed at iter-669 live in `tool_guardrails`. Port the
   fault-injection remainder (progress-token stall, duplicate-progress
   spam, oscillation) as pure tests beside them.

## §2 Trinity ports (interleave by size; updated)

| Port | Status |
|---|---|
| P0-A recovery semantics (grace window, CAS terminal write, two-store reconcile) | partial — close-at-detection absorbed at iter-691; grace/CAS/two-store open |
| P0-B error-code taxonomy | **DONE** (iter-691) |
| P1-A per-channel dispatch breaker, half-open probe | **next trinity port** — fixes the Telegram hammering; rides the gateway consumer-mount unblock (same delivery path) |
| P1-B effect-scoped idempotency on outbound sends | open |
| P1-C canary invariant harness (E-01/E-02/E-06) | open |
| P2-A lease/retry redelivery cap + poison-park | **core ledger landed (iter-722)**; consumer mount is the remainder (§1 row 2) |
| P2-B heartbeat liveness, `unsupported`-vs-`stale` hinge | open |
| P2-C execution integrity at terminal-write time | open |
| P2-D CAS/RECONCILED discipline + capacity slots | open |
| P2-E credential encryption at rest + rotation | open |

## §3 Peer tracks (owned by their docs; not re-planned here)

- **TUI live audit waves** (`plan-2026-10-09-tui-live-audit-bugfix-plans.md`):
  P0 user-visible correctness (paste-burst Enter loss, mid-render tool
  splits), P1 clamp+selection, P2 alt-screen architecture, P3 hygiene —
  each its own gated iteration.
- **Telegram/org track** (`plan-2026-10-09-remaining-gaps.md`): items
  1+2 fixed live-verified (iter-704); feed class deployed with Slice C
  live E2E owner-gated (bot must be added as channel admin); Discord/
  Slack adapters credential-blocked; socialization armed with the
  empty-tick defect fixed (iter-715) — 09:30Z outcome watch pending;
  the disclosed non-implementation items (owner flips, tarball
  sign-off, dispatcher artifact header update) sit with the owner.
- **Standing org queue** (unchanged from v2 §3): worklog scope gate →
  feed/DM read adapters → seat-budget provisioning CLI → two-tier
  memory → org loop guard (GAP-2.1) → teams table + Team path → org
  check exit codes → CEO loop → AD-RG → `retention_gc`
  wire-or-delete. **IdentityGate: HOLD** — finished, tested, unmounted.

## §4 Config/ops hygiene

- **Cron hygiene** — ephemeral test regs archive post-smoke-test; DUE
  backlog triage (paused jobs' past `next_run_at` is a pause
  consequence, not a fault).
- **CHANGELOG released-section contamination** — recent iters misfiled
  under `[0.2.1]` etc.; the peer's (pre-existing damage).
- **LTO marker rule** — shipped-binary marker checks must be
  reachable-marker only.
- **Peer's clippy `expect()` deny sites** (`agent/stream.rs:524/:544`)
  — theirs (rule 7); annotate only if still present after their TUI
  waves settle.

## §5 Gate + concurrency hygiene (session-proven rules)

- Gate in the isolated worktree (`wt-670`); solo-green after a
  full-gate failure = load flake; never diagnose channels failures
  from solo runs (feature unification).
- **Push the moment the gate clears.** This session the peer's
  `reset --origin/main` dropped two gated-but-unpushed commits
  (recovered from reflog — both objects intact; re-landed via
  `checkout <commit> -- paths` + pathspec commits). An unpushed commit
  in a shared tree is not safe.
- **Never commit without a pathspec while the peer's WIP is staged**
  in the index — a pathspec-less `git commit` swept 127 of their staged
  files once this session (caught by `--stat`, rebuilt).
- **Diff tree-vs-HEAD before editing shared files** — the tree's
  scheduler/db carried peer WIP reverting their own committed fix;
  editing the tree copy would have committed their mid-flight rework.
- Docs edits: always rebuild from `git show <remote-tip>:docs/<file>`.
- Iteration labels run concurrent (two 714s, three 720s so far);
  renumber before push and leave an append-only note.

## §6 Retired rows (do not re-plan)

- Dispatch consolidation / Loop-B deletions — executed (iters
  608/663/669).
- Telegram credential blocker — retired (live token; delivery hop
  closed; inbound bugs fixed iter-704).
- Cost ledger / seat budgets, desktop/plugin surface, ZeroRelay fleet,
  SOP engine, Landlock sandbox — rejected with evidence (work-outline
  §7, 2026-10-09).
- Delegation governance (712), guardian-LLM arm (713),
  micro-compaction (714), delivery-ledger core (722) — landed.
