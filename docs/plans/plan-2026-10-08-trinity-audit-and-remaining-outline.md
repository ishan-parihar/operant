# Trinity Audit → Operant Port Candidates + Remaining-Work Outline

Date: 2026-10-08 · Scope: `/parent-projects/trinity` (abilityai/trinity v0.9.5, Python/FastAPI + Redis, Apache 2.0) audited against operant's live surface. Companion to `plan-2026-10-07-agentic-core-verified-remaining-work.md` and `plan-2026-10-07-organism-operational-memory.md`.

## §0 Paradigm mismatch — what does NOT port

Trinity orchestrates **external** Claude Code agent containers over HTTP (backend holds no agent loop; agents run in Docker, backend owns scheduling/capacity/audit). Operant embeds the agent runtime **in-process** (Rust, sqlite, no Redis). Trinity's Redis plumbing, Docker lifecycle, HTTP agent-client layer, and Postgres parity are infrastructure, not ideas. The ports below are **concepts and invariants**, re-expressed against sqlite + in-process state.

## §1 Port candidates, ranked

### P0-A. Startup recovery: two-pass reconcile + grace window + CAS-guarded terminal writes
Blueprint: `src/backend/services/cleanup_service.py::recover_orphaned_executions` (#128/#748/#749/#1804).
- **Two asymmetric passes**: state-store→executor (rows whose executor is gone → terminal) AND executor→state-store (executor entries whose row is terminal/missing → reclaim). One-directional sweeps leak the other half.
- **Grace window** (`STARTUP_RECOVERY_GRACE_SECONDS`): rows younger than grace are skipped, not failed — recovery must not race an in-flight handler into a leaked slot.
- **CAS-guarded terminal write**: `mark_execution_failed_by_watchdog` returns won/lost; lost means a real completion won the row during restart — benign, and reported as `cas_lost`, never conflated with `errors`.
- **Cross-worker in-flight markers** (TTL per execution): "cannot verify owner" → leave running, let the periodic sweep catch it later.
- **Operant landing**: the Wave-3 "reaper CLOSES non-terminal runs" row. Gateway startup: close non-terminal journal runs via guarded write (status precondition), skip runs newer than a grace window, and reconcile BOTH stores (journal `.turn_state` rows × `operant_cron.db` `last_run_at`/`state`). A lost CAS = the run actually finished while we restarted — log, don't error.

### P0-B. Machine-readable error-code taxonomy + negative-marker classifier
Blueprints: `services/execution_envelope.py::TaskExecutionErrorCode` (TIMEOUT/CAPACITY/AUTH/BILLING/AGENT_ERROR/NETWORK/CIRCUIT_OPEN/RECONCILED/LEASE_EXPIRED/EPHEMERAL_EXHAUSTED) and `scheduler/failure_classifier.py` (#904): auth-indicator substring match is **short-circuited by SIGKILL/OOM/exit-code-137 markers** — a signal death is never misclassified as auth, which would burn skip-list slots on a credential that's actually fine.
- **Operant landing**: `last_status`/`last_error` are free strings; retries can't branch on failure class. Add an `error_code` enum column/field on run terminals; classifier with anti-markers before any auth/billing classification (subprocess signal deaths from our bash tool must never count as provider auth failures). Unlocks correct retry-vs-escalate policy and feeds the dispatch breaker (P1-A).

### P1-A. Per-channel dispatch breaker with half-open probe (producer-side fast-fail)
Blueprint: `services/dispatch_breaker.py` (#526): consecutive-failure `closed→open→half-open(probe)→closed`, atomic Lua transitions, probe-lock so exactly one probe flies, capped exponential cooldown (30s→300s), **separate namespaces per concern** (transport-unreachable vs auth-dead vs wedged never contaminate each other's counters). Fail-open on infra down.
- **Operant landing**: the Telegram saga (401→400→503→404 hammering on every cycle) is precisely an open dispatch breaker being ignored. Gateway outbound per-channel breaker keyed on channel + failure class; open → skip sends fast (log once, not every cycle), half-open probe = the existing getMe probe. SQLite table, no Redis needed.

### P1-B. Effect-scoped idempotency for outbound side effects
Blueprint: `services/idempotency_service.py` (#525/#1084): (scope, key) claims with NEW/IN_FLIGHT/COMPLETED + result-snapshot replay; **effect-guard at the sink** keyed on *resolved immutable identity* (recipient+channel+type), never the LLM-generated body; fail-open on claim-infrastructure failure; in-flight duplicate → retryable "already in progress", never silent None-as-success. Key-derivation rule worth stealing verbatim: key on what the *client retry unit* is (raw fire spec, token+body hash), not the resolved value.
- **Operant landing**: a retried/re-fired cron turn can double-send a channel message today. Wrap every outbound channel send in `effect_guard(run_id, channel, recipient)`; a failed send releases the claim so retry proceeds; a completed one replays/skips. Also guards the planned `register_delivery_fn` path.

### P1-C. Canary invariant harness (self-checking orchestration invariants)
Blueprint: `src/backend/canary/` — ~15 named invariants as pure functions over a typed, roughly-simultaneous `Snapshot` (Redis × SQL × agent registry), run on a 5-minute loop + on-demand endpoint. Highlights: S-01 slot↔row bijection, S-02 no overbooking, E-01 terminal-state closure, E-02 no phantom reversal (terminal→non-terminal), E-06 no overdue next_run, R-01 no zombie agents, G-04 no creds in queue metadata, L-03 delete cascades leave no orphans.
- **Operant landing**: huge synergy with the org layer's `org check`. Build `operant-core` snapshot collector over (cron_jobs × journal × gateway run-state × channel queues) + an invariant registry; the governor's governance digest consumes violations. Each invariant is a small pure fn — cheap to port one at a time, starting with E-01 (terminal closure — the reaper's acceptance test), E-02, E-06 (overdue next_run_at — the dispatcher already trips on this by hand).

### P2-A. Lease/retry redelivery cap + poison-park, with alert-before-terminal ordering
Blueprint: `services/lease_reaper_service.py` (#1402): expired lease → requeue the SAME row (`execution_id` preserved because idempotency is execution-scoped), `redelivery_count++`; at cap → **create the operator alert FIRST, then** write the terminal park (a crash between park and alert would make the poison invisible and unrecoverable — alert-first inverts to a benign failure). Idempotent alert insert via derived id.
- **Operant landing**: cron job retries currently retry forever-ish. Cap → poison-park state (`state=poisoned`) + operator/queue notification before the terminal write. The alert-first ordering generalizes to ALL operant terminal transitions that need a human eye.

### P2-B. Heartbeat liveness with `unsupported`-vs-`stale` hinge + transition-only alerts
Blueprint: `services/heartbeat_service.py` (#307): 15s TTL beats; persistent `seen` marker distinguishes "never supported heartbeats" (never mark dead) from "went stale" (miss-threshold → one soft degraded alert per episode, cooldown-debounced; recovered alert only after a prior downgrade). Per-tick state in TTL counters, never the SQL db.
- **Operant landing**: seat liveness for the org layer. Seats already run on cycles; a `seat_heartbeat` watermark (unix ts per seat in the org db) + a check in the governance digest gives crew-chief/hrmaster a real "seat X hasn't reported in N hours" signal with the same never-false-dead hinge for seats that haven't been taught to beat yet.

### P2-C. Execution integrity: detect "succeeded but work lost" at terminal-write time
Blueprint: `services/execution_integrity.py` (#2467): a `claude --print` turn that exits while a background task is in flight kills it ~5s later and logs it only in the raw stream; the module derives a structured record at terminal time from the transcript — structural-only fields persisted (never description/command/output), malformed shapes degrade to no-entry (never false-positive).
- **Operant landing**: operant's bash tool can spawn background/detached processes; a turn that "succeeds" while its child died is currently indistinguishable from a clean success. At turn close, scan the turn's journal for spawned-but-unreaped children and stamp an integrity flag on the run row.

### P2-D. CAS/RECONCILED discipline + capacity slots with stale reclaim
- **Guarded terminal writes** (status-as-precondition UPDATE; loser records `RECONCILED`, never double-acts) — apply to journal close paths and cron state flips generally.
- **CapacityManager** (#428): per-agent N-ary slot gate with overflow policies and `reclaim_stale(agent_timeouts)` — maps to per-seat concurrent-run caps in the scheduler if seat overlap ever becomes a problem. Defer until a real overlap incident.

### P2-E. Credential encryption at rest with rotation fallback
Blueprint: `services/credential_encryption.py`: AES-256-GCM archive of credential files; primary key env + **decrypt-only secondary key** for rotation (old ciphertext still opens; new writes use primary).
- **Operant landing**: TELEGRAM_BOT_TOKEN etc. currently env-only (and empty, hence the blocker). An encrypted credential store in the app db keyed by a master key would decouple channel credentials from the environment and make rotation non-breaking. Only worth it when >1 channel credential exists.

## §2 What trinity does that operant already covers (verified, no action)
- Scheduling with execution history, manual trigger, enable/disable → operant cron scheduler.
- Inter-agent messaging/delegation (MCP tools) → operant delegate tool + org notice board.
- Audit trail → journal + org worklog.
- Token/cost observability → budget accounting (deterministic ledger).
- Agent-to-agent auth scoping → seat authority/governance escalation (richer in operant).
- Template/skill libraries → operant skills (out of audit scope).

## §3 Remaining-work outline (operant, ordered)

**Wave-3 — steering/cancellation/journal** (§4 of the core plan):
1. Startup reaper CLOSE — now with the trinity blueprint (P0-A): grace window, CAS-guarded write, two-store reconcile.
2. `CancellationToken` threaded into core `run()`; bridge `interrupt_flag`; select! at `execute_tools` + `request_approval`.
3. ~~`SwitchModel` steer — needs the interior-cell `self.model` refactor~~ **RESOLVED 2026-10-08: the interior-cell swap already landed at iter-162** — `OperantAgent::set_model` is `&self` via `RwLock<Option<String>>`, gateway applies session-metadata overrides through it, and the fallback chain switches models mid-run (run.rs:1227 + budget refund). The plan doc's "~24 direct reads" premise is obsolete. Remaining work is a small slice: `SwitchModel(String)` variant on `SteeringCommand`, `/model <name>` parse (strict-prefix so `/modeling tips` stays guidance), drain arm calling `set_model` + refunding the iteration budget. ~30 lines + tests, rides with any Wave-3 slice.
4. Per-turn journal rows: exit_reason/halt_verdict columns + the error-code enum (P0-B) in the same slice.
5. Mid-turn steering on live gateway path (route same-conversation new messages through `steer()`).

**Wave-4 — context ladder** (§5): ordered preflight ladder (summarizer→decay→wrap-up→TOC) in `build_messages`; PromptCacheGuard; `TURN_WALL_CLOCK_LIMIT_SECS` env override (run.rs:391); vision-routing ruling; ingestion-time offload+TOC + wire-or-drop the dead `max_tool_result_chars` knob.

**openhuman residue**: `record_output` output-side identical-output guard (feed from stream.rs assistant-text accumulation, exempt-aware, warn/skip-not-halt); full adversarial suite port.

**New ports from this audit** (interleave by size): P0-B error codes (small, do with Wave-3 journal columns) → P1-A channel dispatch breaker (fixes the Telegram hammering) → P1-B effect idempotency on sends → P1-C canary invariants (start E-01/E-02/E-06) → P2 items as capacity allows.

**Org layer** (§6 of organism plan): notice-board READ wiring (the only genuinely unwired surface — resolver doc confirms zero production inbox callers; wire `org notice inbox` CLI + seat-prompt/tool consumption), worklog scope gate, feed/DM read adapters, seat-budget provisioning CLI, two-tier memory, org loop guard, teams table, recipient Team path, CEO loop, AD-RG. **IdentityGate: HOLD** — verified 2026-10-08 it is finished, tested, and deliberately unmounted (dark-mergeable seatbelt, one-line mount); do NOT delete; decide the mount as an explicit org-layer-default policy when that layer stabilizes.

**Config/ops hygiene**: config-schema exemption wiring (blocked on peer's config.rs ownership); ~~`register_delivery_fn`~~ **RESOLVED 2026-10-08: delivery is already wired** — `CronScheduler::with_delivery(cron_tx)` is mounted in gateway_runner.rs:3799/3813 with the CronDelivery channel; the R39 warn is about jobs created *without origin fields*, and the Telegram credential remains the only delivery blocker; ephemeral cron registrations archive post-smoke-test.

**Gate hygiene** (from this session's evidence): future gate worktrees should use an **isolated target dir** — the shared symlinked target is the proven source of governance-suite load flakes (3× green solo whenever the peer's builds go quiet).
