# Remaining-work outline (reconciled, 2026-10-09)

> **Baseline:** `7a997c0c` — iters 688–691 (record_output, SwitchModel, notice
> inbox, reaper CLOSE + exit columns), the channels strip-alias fix, iters
> 697/698 (preflight ladder, PromptCacheGuard, `HERMES_TURN_TIMEOUT`) landed,
> gated green (6-package, 7468 passed / 0 failed at `f7d06d47`), pushed.
> **Sources:** `plan-2026-10-07-agentic-core-verified-remaining-work.md` (audit
> r2, rows updated by iters 688–691/697/698) + `plan-2026-10-08-trinity-audit-
> and-remaining-outline.md` (port candidates) + `plan-2026-10-08-remaining-
> implementation-gaps.md` (org programme queue). This doc is the single ordered
> ledger of what is left; the parents remain the evidence record.

---

## §1 Done since the last outline (do not re-plan)

| Item | Landed |
|---|---|
| Output-side `record_output` guard | iter-688 `50bc2fbf` |
| `SwitchModel` steer (`/model <name>`) | iter-689 `3dfc420a` |
| Notice-board READ wiring (CLI + seat prompt) | iter-690 `5b0b5f9b` |
| Reaper CLOSE-at-detection + `exit_code`/`exit_reason` + `TurnExitReason` classifier (trinity P0-B) | iter-691 `bb29854b` |
| Telegram `strip_tool_call_tags` underscore-alias fix | `10b139ac` |
| Wave-4 ordered preflight ladder (TOC/trim → decay → summarize → evict + wrap-up rung) | iter-697 `65aba584` |
| PromptCacheGuard (frozen-prefix invariant checked) + `HERMES_TURN_TIMEOUT` override | iter-698 `f7d06d47` |

## §2 Next slices (agreed order, core track)

1. **Vision-routing ruling** — `multimodal` deleted iter-663; `with_multimodal` `/`with_vision_route` tests+ACP only. Decide: port per-iteration vision routing to facade, or formally accept single-provider vision at construction.
2. **`max_tool_result_chars` wire-or-drop + ingestion-time offload/TOC** —
   dead knob (default 50000, zero consumers); `ArtifactIndex`/`toc`/`offload`
   absent.
3. **openhuman adversarial suite port** — 486-LOC `no_progress/mod_tests.rs`
   fault-injection suite; 13 pattern tests already ported (iter-669).
4. **§2 dispatch-consolidation ruling** (option A recommended) → then §7
   deletions: `Agent::turn` (agent.rs:1516), `loop_::run` (loop_/run.rs:102),
   `loop_detector.rs` (post-§3 harvest).

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

**Org-programme queue (from the gaps doc — their doc owns the detail):**

| Item | Status |
|---|---|
| Gap 5 phase 1b: gateway turn-start injection seam (DM turns see feeds; no self-echo) | unblocked, next org build |
| Gap 6 phase 1: socializer cast (10th registry row, 09:30 sessions, `enabled=false` default) | approved design, unblocked — owner flips `[socialization] enabled` to arm |
| Gap 3: breaker threshold tuning | **DECIDED HOLD 6/6** (0 degenerate fires in 87 runs; revisit only on a live fire) |
| D-2 test debt: verify `cron_session_isolation` pins the scheduler session-id derivation | small, opportunistic |
| 4 lib-test warnings from the 679 build | small, opportunistic |
| `packet-e-wt-wip-20261007.tar.gz` cleanup | awaits owner sign-off |
| Gap 5 phase 2: platform read adapters (Telegram/Discord/Slack history → `Dm` class) | **BLOCKED on credentials** — `TELEGRAM_BOT_TOKEN` empty, discord/slack disabled+unset; building now = unverifiable dead code; inbound DMs already land via the iter-679 tap |
| Gaps 1/2/4/7/8 (envelope metering, clarify fail-fast, D-2 posture, budget fold + predicates + read surfaces, synthesis + amendments) | **DONE** (iters 677/678/688/692/695) |

## §5 Config/ops hygiene

- **Config-schema exemption wiring** — `guardrail_exempt_tools` config-file
  surface; BLOCKED on the peer's `config.rs` ownership.
- **Telegram bot-auth credential** — `TELEGRAM_BOT_TOKEN` empty; the only
  delivery-hop blocker AND the org phase-2 read-adapter blocker (discord/slack
  also disabled + unset); operator action, not code.
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
