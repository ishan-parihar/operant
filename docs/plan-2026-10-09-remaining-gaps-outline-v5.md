# Remaining implementation gaps — reconciled outline v5 (2026-10-09)

> Supersedes the execution queue of
> `plan-2026-10-09-remaining-gaps-outline-v4.md` (its row 2 trim half
> landed as iter-729: code `da42909b`, docs `c5bfb754`; gate at the
> merged tree 7522 passed / 1 failed — browser_provider, solo-green =
> environment flake). The peer's
> `plan-2026-10-09-remaining-gaps.md` (Telegram/org track) and
> `plan-2026-10-09-tui-live-audit-bugfix-plans.md` (TUI waves) keep
> ownership of their rows; carried here by reference. Every blocker
> state below was re-verified at tip `c5bfb754`.

## §0 State verified at the tip (`c5bfb754`, 2026-10-09)

- **Landed since v4:** iter-729 `[agent] max_tool_result_chars`
  wiring (outline v4 §1 row 2, trim half) — ingestion-time
  head(2/3)+tail(1/3) trim at both runtime-Agent turn paths,
  observer/TurnEvent keep full output, core path already capped at
  4096. Peer landed in parallel: their TUI live-audit P0/P1 waves —
  iter-725 (P0-2/N-3 tool rows), iter-726 (P0-1 paste-burst Enter),
  iter-727 (N-2 transcript commit), iter-728 (P1-6 clean transcript
  copy).
- **The `5193f155` escalation-regression flag softens:** the
  seat-policy/nested-agent deadline tests did NOT fail in the
  iter-729 gate (7522/1) — under lighter load they pass. Consistent
  with the load-sensitivity diagnosis (per-construction skills-tree
  discovery cost tripling suite duration, making the 10s/60s
  deadlines marginal). Still the peer's track: their fix is either
  the discovery cost or the deadlines; flagged, not re-planned here.
- **Blockers unchanged:** `gateway_runner.rs` still carries the
  peer's WIP (`MM` re-verified at this tip) and still has no
  `with_employee_registry` — the two gateway slices (§1 row 1) wait.
  Next unblocked slice: §1 row 4 (openhuman adversarial remainder).

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
2. **~~`max_tool_result_chars` wire-or-drop~~ TRIM WIRED iter-729**
   (code `da42909b`, labeled 728 at push; renumbered twice
   append-only — the peer's waves took 725/726/727, then 728).
   **The offload/TOC (`ArtifactIndex`) shape remains the open
   follow-up** — ingestion-time offload of oversized results to a
   durable artifact with a TOC stub in the message; nothing exists
   yet (`offload`/`toc` had zero matches at the v4 audit).
3. **Vision-routing hardening test** — RESOLVED BY AUDIT at v4: the
   port already exists (`[multimodal]` config →
   `ReconciledAgent::from_config_with` → `ProviderModelClient`
   per-request preflight + lazy route, incl. the delegate sub-agent
   path; `with_vision_route` stays an injection seam). Remainder: one
   run-path test proving a config-file `vision_provider` routes a
   marker request through a full facade turn (unit tests cover the
   client in isolation only). Hardening, not a gap.
4. **openhuman adversarial suite remainder — NEXT UNBLOCKED SLICE.**
   486-LOC `no_progress/mod_tests.rs` fault-injection suite; the 13
   pattern tests landed at iter-669 live in `tool_guardrails`. Port
   the fault-injection remainder (progress-token stall,
   duplicate-progress spam, oscillation) as pure tests beside them.
5. **Peer's skills discovery cost (their track, tracked here for
   visibility)** — the §0 load-sensitivity finding; fixed by them in
   their skills/TUI line or by relaxing the deadline-sensitive
   run-path tests. Not this line's slice.

## §2 Trinity ports (interleave by size; unchanged from v4)

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

- **TUI live audit waves** (`plan-2026-10-09-tui-live-audit-bugfix-plans.md`):
  P0 wave landed (iters 725/726), N-2/P1-6 landed (727/728); P1
  clamp+selection, P2 alt-screen architecture, P3 hygiene remain —
  each its own gated iteration.
- **Skills/meta-skill-creator**
  (`plan-2026-10-09-meta-skill-creator-nested-references.md`): iter-723
  landed; carries the §0 load-sensitivity finding.
- **Telegram/org track** (`plan-2026-10-09-remaining-gaps.md`): feed
  class deployed; socialization armed (iter-715); owner-gated items
  (channel-admin flip, tarball sign-off) sit with the owner.
- **Standing org queue** (unchanged from v4 §3): worklog scope gate →
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
  slice's (proven: iter-724's five, iter-729's one).
- **Push the moment the gate clears.** An unpushed commit in a shared
  tree is not safe.
- **Never commit without a pathspec while the peer's WIP is staged**
  in the index — a pathspec-less `git commit` swept 127 of their
  staged files once.
- **Diff tree-vs-HEAD before editing shared files**; rebuild from HEAD
  when the tree carries their overlay; restore their bytes after.
- **The peer commits AND pushes mid-slice — twice per slice this
  round.** Re-`git fetch github` before every re-land and every docs
  commit; check the tip's CHANGELOG labels BEFORE naming an
  iteration (iter-729 was renumbered twice in-flight: 725→728→729 as
  the peer's waves took each label first). After a `reset --soft`
  re-land, SHARED files in the worktree are one tip behind — rebuild
  docs from `git show <remote-tip>:docs/<file>`, never diff or commit
  the stale worktree copy (caught by stat before commit).
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
  wiring (724), vision-routing "port to the facade" (resolved by
  audit, v4), `max_tool_result_chars` trim wiring (729) — landed.
