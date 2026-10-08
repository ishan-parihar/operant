# Remaining-work outline (reconciled, 2026-10-09)

> **Baseline:** `477d9841` — iters 688–691 (record_output, SwitchModel, notice
> inbox, reaper CLOSE + exit columns) and the channels strip-alias fix landed,
> gated green (6-package, 7446 passed / 0 failed at `10b139ac`), pushed.
> **Sources:** `plan-2026-10-07-agentic-core-verified-remaining-work.md` (audit
> r2, rows updated by iters 688–691) + `plan-2026-10-08-trinity-audit-and-
> remaining-outline.md` (port candidates). This doc is the single ordered
> ledger of what is left; the two parents remain the evidence record.

---

## §1 Done since the last outline (do not re-plan)

| Item | Landed |
|---|---|
| Output-side `record_output` guard | iter-688 `50bc2fbf` |
| `SwitchModel` steer (`/model <name>`) | iter-689 `3dfc420a` |
| Notice-board READ wiring (CLI + seat prompt) | iter-690 `5b0b5f9b` |
| Reaper CLOSE-at-detection + `exit_code`/`exit_reason` + `TurnExitReason` classifier (trinity P0-B) | iter-691 `bb29854b` |
| Telegram `strip_tool_call_tags` underscore-alias fix | `10b139ac` |

## §2 Next slices (agreed order, core track)

1. ~~**Wave-4 ordered preflight ladder**~~ **EXECUTED iter-697** — ordered rungs TOC/trim → decay → summarize → evict in `build_messages`, each gated on still-over-threshold; wrap-up rung appends final-call copy; `fast_trim_tool_results` gains its first production caller.
2. ~~**PromptCacheGuard**~~ **EXECUTED iter-698** — `prompt_cache_guard` verifies the frozen prefix byte-identical after the ladder (warn in release, debug_assert in tests).
3. ~~**`TURN_WALL_CLOCK_LIMIT_SECS` env override**~~ **EXECUTED iter-698** — `HERMES_TURN_TIMEOUT` (seconds), default 20 min kept on malformed input.
4. **Vision-routing ruling** — `multimodal` deleted iter-663; `with_multimodal` `/`with_vision_route` tests+ACP only. Decide: port per-iteration vision routing to facade, or formally accept single-provider vision at construction.
5. **`max_tool_result_chars` wire-or-drop + ingestion-time offload/TOC** —
   dead knob (default 50000, zero consumers); `ArtifactIndex`/`toc`/`offload`
   absent.
6. **openhuman adversarial suite port** — 486-LOC `no_progress/mod_tests.rs`
   fault-injection suite; 13 pattern tests already ported (iter-669).
7. **§2 dispatch-consolidation ruling** (gates Wave-4 entry formally; option A
   recommended) → then §7 deletions: `Agent::turn` (agent.rs:1516),
   `loop_::run` (loop_/run.rs:102), `loop_detector.rs` (post-§3 harvest).

**Deferred by dependency:** `CancellationToken` into core `run()` (§4 row 102),
mid-turn steering on the live gateway path (§4 row 106), cron trigger
cancellation surface (§4 row 107, optional). S6/S7 ladder-order decision
(guardrail skip → warn → R35 abort) rides with slice 6.

## §3 Trinity port candidates (interleave by size)

| Port | Status |
|---|---|
| P0-A recovery semantics (grace window, CAS-guarded terminal write, two-store reconcile) | partially absorbed into iter-691 (close-at-detection); grace/CAS/two-store remain unbuilt |
| P0-B error-code taxonomy | **DONE** (iter-691 `TurnExitReason`/`classify_turn_error`, kill-marker precedence) |
| P1-A per-channel dispatch breaker, half-open probe | next of the trinity ports — fixes the Telegram hammering |
| P1-B effect-scoped idempotency on outbound sends | open |
| P1-C canary invariant harness (start E-01/E-02/E-06) | open |
| P2-A lease/retry redelivery cap + poison-park | open |
| P2-B heartbeat liveness, `unsupported`-vs-`stale` hinge | open |
| P2-C execution integrity at terminal-write time | open |
| P2-D CAS/RECONCILED discipline + capacity slots | open |
| P2-E credential encryption at rest + rotation | open |

## §4 Org layer (§6 rows, independent track)

Worklog scope gate (`worklog_db.rs:278`) → feed/DM read adapters →
seat-budget provisioning CLI → two-tier memory (org bank vs seat banks) →
org loop guard (GAP-2.1 acceptance) → teams table + recipient Team path →
org check exit codes → CEO loop → AD-RG → `retention_gc` wire-or-delete.
**IdentityGate: HOLD** — finished, tested, deliberately unmounted; decide the
mount as an org-layer-default policy when the layer stabilizes (do NOT delete).

## §5 Config/ops hygiene

- **Config-schema exemption wiring** — `guardrail_exempt_tools` config-file
  surface; BLOCKED on the peer's `config.rs` ownership.
- **Telegram bot-auth credential** — `TELEGRAM_BOT_TOKEN` empty; the only
  delivery-hop blocker; operator action, not code.
- **Cron hygiene** — ephemeral test regs to archive post-smoke-test; DUE-job
  backlog.
- **LTO marker rule** — shipped-binary marker checks must be
  reachable-marker only.

## §6 Gate hygiene (session evidence)

- Gate worktrees use an isolated `target` dir (wt-670 now real-dir; the old
  shared symlink was the proven source of governance-suite load flakes —
  solo-green confirms).
- **Feature-unification gotcha:** solo `-p operant-channels` builds WITHOUT
  the telegram feature (874 tests; telegram tests absent). Only multi-package
  runs including `-p operant-cli` compile `telegram::tests` (1108+). Never
  diagnose channels failures from solo runs; a "0 tests matched" filter result
  means the module is not in the binary.
- Solo-green after a full-gate failure = load flake (recipe), unless the test
  reproduces at the prior commit under unified features (real, pre-existing).
