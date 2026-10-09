# Remaining implementation gaps — reconciled outline v4 (2026-10-09)

> Supersedes the execution queue of
> `plan-2026-10-09-remaining-gaps-outline-v3.md` (its row 1, config-file
> wiring, landed as iter-724 `a3e89493` + docs `987b80f5`; gate at the
> merged tree 7510 passed / 5 failed — every failure bisected away from
> the slice, see §0). The peer's `plan-2026-10-09-remaining-gaps.md`
> (Telegram/org track) and `plan-2026-10-09-tui-live-audit-bugfix-plans.md`
> (TUI waves) keep ownership of their rows; carried here by reference.
> Every blocker state below was re-verified at tip `987b80f5`.

## §0 State verified at the tip (`987b80f5`, 2026-10-09)

- **Landed since v3:** iter-724 config-file wiring (outline v3 §1 row 1 —
  `[agent] guardrail_exempt_tools` flows via the From impl to the
  guardrail tracker; `[agent] guardian_llm`/`micro_compaction` are
  config-file DEFAULTS below the `OPERANT_*` env vars, applied by
  `apply_agent_config_defaults` at all three agent factories + ACP).
  Peer landed in parallel: iter-723 skills nested-references validator
  (`5193f155`, pushed).
- **OPEN REGRESSION ON ORIGIN (peer's `5193f155`, not fixed):** the
  seat-policy suite duration tripled (~13s → 36–45s) and the deadline-
  sensitive run-path tests now fail reproducibly at their commit alone
  (3× solo): `seat_policy_run_path::{lockdown_seat_escalates…,
  no_source_agent_behaves_as_today…}` both blow the 10s permission
  deadline (`Elapsed`); `nested_agent_run` times out at its 60s cap.
  Their 38 skills unit tests pass instantly → smells like per-
  construction discovery cost (BFS over the real skills tree), not a
  logic break. Their slice, their fix; flagged before their next gated
  push. Everything else in the 7510/5 gate was load flakes
  (`gateway_commands::grant_seats…`, `facade_tests::cancellation_token…`
  — both solo-green).
- **Blockers unchanged:** `gateway_runner.rs` still carries the peer's
  WIP (`MM` at the tip check) and still has no
  `with_employee_registry` — the two gateway slices (§1 row 1) wait.
  `crates/operant-core/src/config.rs` is clean but row 1 is done; the
  next unblocked slice is §1 row 2 (offload/TOC).

## §1 Core execution queue (ordered)

1. **Gateway ledger consumer mount + registry attach** — BLOCKED on the
   peer's `gateway_runner.rs` WIP. Two small slices on the same file:
   - Consumer mount: the sender loop settles each `CronDelivery` via
     `CronDb::mark_delivery_outcome` (emitting
     `ObserverEvent::CronDeliveryOutcome`) — without it, ledger rows
     linger pending until the 2h reclaim replays them (correct but
     wasteful; the outcome write closes the loop).
   - Registry attach: `.with_employee_registry(Arc::clone(&employee_registry))`
     at the `SeatAuthority::new` site — Bounded delegation fails closed
     without it (latent: no production row sets Bounded yet).
   - The trinity P1-A per-channel dispatch breaker rides this same
     unblock (same delivery path).
2. **~~`max_tool_result_chars` wire-or-drop~~ TRIM WIRED iter-729
   (code commit `da42909b`, labeled 728 at push; renumbered twice
   append-only — the peer's TUI waves took 725/726/727, then 728);
   offload/TOC remains the open follow-up.** The runtime Agent's two
   turn paths ingested raw tool output into history (zero knob
   consumers since the old engine's removal sweep) — the TUI live
   audit's N-4 symptom (untrimmed `http_request` fetch, 50k tokens).
   `truncate_tool_result` now applies at both ingestion sites with
   the knob's contract (head 2/3 + tail 1/3, marker, 0 disables,
   default 50000); observer/TurnEvent keep full output; core path
   already capped at 4096. Gate 7522/1 (browser_provider flake,
   solo-green). The `ArtifactIndex` offload/TOC shape stays open.
3. **Vision-routing ruling — RESOLVED BY AUDIT at the tip; v3's premise
   was stale.** Config-file vision routing IS ported to the facade:
   `[multimodal] vision_provider`/`vision_model` (schema
   `operant-config/src/schema/media.rs`) flows through
   `ReconciledAgent::from_config_with` (reconciled.rs:2384 — the
   production factory, also used per-sub-agent by
   `operant-runtime/src/tools/delegate.rs:1285`) into
   `ProviderModelClient::with_multimodal`, which does per-request
   marker preflight (`select_provider_and_model` — markers>0 and
   !supports_vision → route) plus lazy provider construction
   (`resolve_vision_route`). `with_vision_route` is a deliberate
   injection seam for owned provider instances (unit-tested only) —
   keep. v3's "`multimodal` deleted at iter-663" claim was wrong: the
   `[multimodal]` config section exists (whatever was deleted then was
   a different surface). **Remainder (hardening, one small slice):**
   a run-path test proving a config-file `vision_provider` actually
   routes a marker request through a full facade turn — the existing
   unit tests cover `ProviderModelClient` in isolation only.
4. **openhuman adversarial suite remainder** — 486-LOC
   `no_progress/mod_tests.rs` fault-injection suite; the 13 pattern
   tests landed at iter-669 live in `tool_guardrails`. Port the
   fault-injection remainder (progress-token stall, duplicate-progress
   spam, oscillation) as pure tests beside them.

## §2 Trinity ports (interleave by size; unchanged from v3)

| Port | Status |
|---|---|
| P0-A recovery semantics (grace window, CAS terminal write, two-store reconcile) | partial — close-at-detection absorbed at iter-691; grace/CAS/two-store open |
| P0-B error-code taxonomy | **DONE** (iter-691) |
| P1-A per-channel dispatch breaker, half-open probe | **next trinity port** — fixes the Telegram hammering; rides the gateway consumer-mount unblock (same delivery path) |
| P1-B effect-scoped idempotency on outbound sends | open |
| P1-C canary invariant harness (E-01/E-02/E-06) | open |
| P2-A lease/retry redelivery cap + poison-park | **core ledger landed (iter-722)**; consumer mount is the remainder (§1 row 1) |
| P2-B heartbeat liveness, `unsupported`-vs-`stale` hinge | open |
| P2-C execution integrity at terminal-write time | open |
| P2-D CAS/RECONCILED discipline + capacity slots | open |
| P2-E credential encryption at rest + rotation | open |

## §3 Peer tracks (owned by their docs; not re-planned here)

- **Skills/meta-skill-creator** (`plan-2026-10-09-meta-skill-creator-nested-references.md`):
  iter-723 landed and pushed — **carries the §0 open regression**; the
  fix belongs to this track.
- **TUI live audit waves** (`plan-2026-10-09-tui-live-audit-bugfix-plans.md`):
  P0 user-visible correctness (paste-burst Enter loss, mid-render tool
  splits), P1 clamp+selection, P2 alt-screen architecture, P3 hygiene —
  each its own gated iteration.
- **Telegram/org track** (`plan-2026-10-09-remaining-gaps.md`): feed
  class deployed; socialization armed (iter-715); owner-gated items
  (channel-admin flip, tarball sign-off) sit with the owner.
- **Standing org queue** (unchanged from v3 §3): worklog scope gate →
  feed/DM read adapters → seat-budget provisioning CLI → two-tier
  memory → org loop guard (GAP-2.1) → teams table + Team path → org
  check exit codes → CEO loop → AD-RG → `retention_gc` wire-or-delete.
  **IdentityGate: HOLD** — finished, tested, unmounted.

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
  full-gate failure = load flake; a failure that reproduces at the
  prior commit alone is real, pre-existing, and NOT the current
  slice's (proven twice this session: iter-724's five gate failures
  all bisected to the peer's `5193f155` or to load flakes).
- **Push the moment the gate clears.** An unpushed commit in a shared
  tree is not safe (the peer's `reset --origin/main` dropped two
  gated-but-unpushed commits earlier; reflog recovery worked).
- **Never commit without a pathspec while the peer's WIP is staged**
  in the index — a pathspec-less `git commit` swept 127 of their
  staged files once.
- **Diff tree-vs-HEAD before editing shared files**; rebuild from HEAD
  when the tree carries their overlay; restore their bytes after.
- **The peer commits AND pushes mid-slice.** Their iter-723 entered
  this line's ancestry mid-slice (fetched only because a docs rebuild
  re-read the remote tip). Re-`git fetch github` before any rebase,
  re-land, or docs rebuild; a "their entry is uncommitted" premise can
  be stale within the hour. (Their iter-723 CHANGELOG entry went from
  in-flight to pushed mid-slice — the label collision never happened.)
- Docs edits: always rebuild from `git show <remote-tip>:docs/<file>`.
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
  wiring (724) — landed.
- Vision-routing "port to the facade" — retired as a gap by v4 §1 row 3
  audit: the port already exists; only the run-path hardening test
  remains.
