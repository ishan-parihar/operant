# Remaining implementation gaps — reconciled outline v2 (2026-10-09)

> Supersedes the execution queue in
> `plan-2026-10-09-remaining-work-outline.md` (all four §8 candidate rows
> landed: cron-stack ruling, iter-712 delegation governance, iter-713
> guardian-LLM arm, iter-714 micro-compaction). The peer's
> `plan-2026-10-09-remaining-gaps.md` keeps ownership of the
> Telegram/org-programme track; its open rows are carried here by
> reference only. Every anchor below was re-verified at tip `d9af0e0e`.

## §0 State verified at the tip (2026-10-09, `d9af0e0e`)

- **Peer concurrency:** the peer's line interleaves the TUI/visual audit
  (iter-718 provider-less auth slots, corpus 1738/0) with the org track —
  their rev-2 gaps doc (`4648dcd7`, iter-719) reports feed class DEPLOYED
  (iter-711), context_items retention DEPLOYED (their iter-713), and
  socialization ARMED with first fire + the empty-tick gate defect fixed
  live (iter-715). None of the core-track rows below were closed by
  their line. Their iteration labels run parallel to mine (known
  collision class; my 713/714 are distinct commits).
- **Stale-row audit (this outline's main correction):** the old §2 row
  "dispatch-consolidation ruling → deletions" is **RETIRED — already
  executed by the Wave-1/2 harvests**: Loop B engine deleted at
  `ff013938` (iter-663), `loop_detector` harvested into
  `crates/operant-core/src/tool_guardrails.rs` (`4ed849ef` iter-608,
  `b87caad6` iter-669). `crates/operant-agent/` no longer exists — the
  agent lives in `crates/operant-core/src/agent/`. `Agent::turn` /
  `loop_::run` anchors are dead paths; nothing left to delete.
- **`max_tool_result_chars` is a schema-only dead knob**: live only in
  `operant-config/src/schema/{core,helpers}.rs`; zero consumers in any
  runtime crate. The row shrinks to wire-or-drop inside operant-config.
- **Unblock changes:** `crates/operant-core/src/config.rs` is CLEAN at
  the tip → the config-wiring batch (§2 row 2) is **unblocked**.
  `crates/operant-cli/src/gateway_runner.rs` still carries the peer's
  WIP deltas → the registry-attach slice (§2 row 3) stays blocked.

## §1 Execution queue (core track, ordered)

1. **~~Delivery-ledger durability~~ EXECUTED iter-722** (trinity
   P2-A concrete): the `cron_deliveries` v3 migration (separate
   append-only table, hermes `deliveries.db` shape, no FK) + queue-before-
   handoff in `deliver_result` + `mark_delivery_outcome` consumer API +
   `MAX_DELIVERY_ATTEMPTS=3` park tombstone + `max(3×timeout, 2h)`
   stale-claim reclaim replayed BEFORE the due-jobs gate (empty-due
   ticks replay too) + `ObserverEvent::CronDelivery*` variants +
   `with_observer` dark-mergeable mount. **Remaining sub-slice:** the
   gateway consumer mount (settle via `mark_delivery_outcome` in the
   sender loop) — blocked on the peer's `gateway_runner.rs` WIP, same
   file as the registry-attach row below.
2. **Config-file wiring batch** — UNBLOCKED (`config.rs` clean):
   - `guardrail_exempt_tools` config-file schema surface (the
     approval-path exemption knob).
   - `OPERANT_GUARDIAN_LLM` / `OPERANT_MICRO_COMPACTION` config-file
     surfaces (env vars landed in 713/714; config parity only).
3. **Gateway registry attach** — BLOCKED on the peer's gateway WIP:
   `SeatAuthority::new` (`gateway_runner.rs:1868`) lacks
   `.with_employee_registry(Arc::clone(&employee_registry))` — the
   registry is built and stored at `:1840/:1846` but never handed to the
   authority, so **Bounded delegation fails closed in the gateway**
   (no production row sets Bounded yet — latent, not live). Small
   one-line slice the moment their file is clean.
4. **Vision-routing ruling** — `with_vision_route`/`with_multimodal`
   tests+ACP only (`crates/operant-runtime/src/agent/reconciled.rs`,
   `tools/delegate.rs`); `multimodal` was deleted at iter-663. Decide:
   port per-iteration vision routing to the facade, or formally accept
   single-provider vision at construction. Ruling, then one slice.
5. **`max_tool_result_chars` wire-or-drop + ingestion-time
   offload/TOC** — knob exists only in the operant-config schema;
   `ArtifactIndex`/`toc`/`offload` absent. Either wire the truncation
   at tool-result ingestion or delete the knob.
6. **openhuman adversarial suite remainder** — 486-LOC
   `no_progress/mod_tests.rs` fault-injection suite; the 13 pattern
   tests landed at iter-669 live in `tool_guardrails`. Port the
   fault-injection remainder (progress-token stall, duplicate-progress
   spam, oscillation) as pure tests beside them.

## §2 Trinity ports (interleave by size; updated)

| Port | Status |
|---|---|
| P0-A recovery semantics (grace window, CAS-guarded terminal write, two-store reconcile) | partial — close-at-detection absorbed at iter-691; grace/CAS/two-store remain |
| P0-B error-code taxonomy | **DONE** (iter-691 `TurnExitReason`/`classify_turn_error`) |
| **P1-A per-channel dispatch breaker, half-open probe** | **next trinity port** — fixes the Telegram hammering; rides §1 row 1 naturally (same delivery path) |
| P1-B effect-scoped idempotency on outbound sends | open |
| P1-C canary invariant harness (E-01/E-02/E-06) | open |
| P2-A lease/retry redelivery cap + poison-park | **core ledger landed (iter-722)** — tombstone cap + stale-claim reclaim live on Stack A; the gateway consumer mount (outcome settle) rides the `gateway_runner.rs` unblock |
| P2-B heartbeat liveness, `unsupported`-vs-`stale` hinge | open |
| P2-C execution integrity at terminal-write time | open |
| P2-D CAS/RECONCILED discipline + capacity slots | open |
| P2-E credential encryption at rest + rotation | open |

## §3 Org layer (independent track — detail owned by the peer's gaps doc)

Unchanged queue: worklog scope gate (`worklog_db.rs:278`) → feed/DM
read adapters → seat-budget provisioning CLI → two-tier memory (org
bank vs seat banks) → org loop guard (GAP-2.1) → teams table +
recipient Team path → org check exit codes → CEO loop → AD-RG →
`retention_gc` wire-or-delete (`NoticeBoard.retention_gc` exists at
`org/notice_db.rs:569` — wire the call or delete the method).
**IdentityGate: HOLD** — finished, tested, deliberately unmounted.
Org-programme state and queue (feed class deployed iter-711; socializer
armed iter-715 with first fire; remaining = owner-gated Slice C E2E,
09:30Z run-outcome watch, three small core debts (ts index, record_feed
CLI test, prune comment), 409 self-race candidate, Discord/Slack adapter
credential block): owned by the peer's `plan-2026-10-09-remaining-gaps.md`
rev 2 — not re-planned here.

## §4 Config/ops hygiene

- **Cron hygiene** — ephemeral test regs archive post-smoke-test;
  DUE-job backlog triage (paused jobs' past `next_run_at` is a pause
  consequence, not a fault).
- **CHANGELOG released-section contamination** — recent iters misfiled
  under `[0.2.1]` etc.; left to the peer (pre-existing damage).
- **LTO marker rule** — shipped-binary marker checks must be
  reachable-marker only.

## §5 Gate hygiene (carried evidence)

- Gate in the isolated worktree (`wt-670`, real target dir); solo-green
  after a full-gate failure = load flake (recipe: reproduce solo at the
  same commit before diagnosing; the `gateway_runner` platform test
  flaked this way twice this session, solo-green both times).
- Feature-unification gotcha: solo `-p operant-channels` builds
  WITHOUT the telegram feature — never diagnose channels failures from
  solo runs.
- Docs edits: always rebuild from `git show <remote-tip>:docs/<file>` —
  two stale-tree clobbers this session proved the cost.

## §6 Retired rows (do not re-plan)

- Dispatch consolidation / Loop-B deletions — **executed** (iters
  608/663/669; see §0).
- Telegram credential blocker — retired 2026-10-09 (live token;
  delivery hop closed; inbound bugs fixed at iter-704).
- Cost ledger / seat budgets, desktop/plugin surface, ZeroRelay fleet,
  SOP engine, Landlock sandbox — rejected with evidence
  (`plan-2026-10-09-remaining-work-outline.md` §7).
- Delegation governance, guardian-LLM arm, micro-compaction,
  cron-stack ruling — landed (iters 712–714).
