# RAW PAGE — the scrape as it stood at archive time

## SECTION 1: the DOM extraction (complete — not capped)

Verdict: operant is not in steady state and is not ready for an unconditional shift to feature work. Auditing the pack against itself reveals (1) a false-closure error where Round 1’s Hydrogen fixes were logged as Round 2’s closure, (2) a live async lock-across-.await correctness bottleneck masked by an inflated clippy allowlist, (3) two unwired/orphaned channel and cache seams, and (4) unmeasured core hot paths explicitly deferred to Round 3.
Pack Honesty, Evidence Classes, and STATE.md Audit
Missing evidence classes (pack-honesty defect): Zero figures in # SECTION A-DELTA through # SECTION F-ANALYSIS carry the required [LIVE], [CERT], [IN-SAMPLE], [PRIOR], or [UNCONFIRMED] tags; all figures below are therefore class-unstated claims.
Figure metadata & arithmetic discrepancies:
28 residual clippy warnings (# FILE: research/audits/context-pack-2026-11-20-blockers/EVIDENCE.md (lines 1,40) lines 17, 22; population: workspace warnings; basis: clippy-warning-gate.sh at HEAD db218f15; selection constraint: excludes 124-entry allowlist containing 96 stale pre-iter-2444 entries) sums to 27, not 28, in # FILE: docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md (lines 1,178) lines 132–151 (3+1×11+2+1+2+1+2+5).
14 in lib + 4 in cli (18 sites) #[expect(dead_code)] (same audit, lines 15, 34, 63; population: #[expect(dead_code)] items in operant-core lib and operant-cli; basis: audit grep/compile at db218f15; selection constraint: excludes #[allow(dead_code)] on 11 vendored TUI files) lists 20 rows in lines 41–61.
0 warnings (# FILE: research/audits/context-pack-2026-11-20-blockers/STATE.md (lines 1,55) line 31; population: operant-core --lib only; basis: cargo check -p operant-core --lib; selection constraint: excludes operant-cli binary/tests) is cited alongside # SECTION C-ASK (ASK.md line 10) "clippy gate is green" despite 27–28 active operant-cli warnings.
Contradictions and stale/unfalsifiable claims in STATE.md:
Crate count wrong (STATE.md lines 10–11): Claims 7 crates while listing operant-providers (deleted) (removed iter-1215 per # FILE: AGENTS.md (lines 1360,1594)), leaving 6 live crates listed or omitting the 7th member (e.g., operant-macros).
Gateway inbound contradiction (STATE.md lines 46–47 vs # FILE: AGENTS.md (lines 1360,1594)): STATE.md claims WhatsApp/Email/SMS are OUTBOUND-ONLY, while AGENTS.md claims all three implement webhook inbound (Cloud API/SMTP/Twilio webhook inbound).
Unclosed/unfalsifiable items (STATE.md lines 37–42): RT-190 remains OPEN; RT-197 admits an unwritten fix; RT-179 ("most twins are live") is unfalsifiable without naming inactive twins; Lib suite RED at tip and TUI visual corpus RED at 56 drifts (population: headless tui_scenarios/; basis: post-iter-1461 verify; selection constraint: excludes live scrollback) contradict LAW.md line 31’s mandatory pre-push cargo test gate while LAW.md line 40 instructs peers not to fix or block on peer reds.
Absent Round-3 measurements (stated assumption): # FILE: docs/audit/2026-11-19-hydrogen-architecture-review-round2-12-findings-verified.md (lines 1,300) (lines 273–281) omitted agent/mod.rs (6,370 LOC), agent/run.rs (5,769 LOC), agent/stream.rs, client.rs (4,427 LOC), and database.rs (7,517 LOC) (population: named operant-core modules; basis: Round-2 measure*.py; selection constraint: omitted from pack) and requested them for Round 3. None are in this pack; per-turn allocation, get_schemas() caching, backpressure, and SQLite contention remain unverified (assumed non-zero risk until measured).
Blocker 1: DELTA.md and EVIDENCE.md misrecord Round 1’s Hydrogen fixes as Round 2’s closure, leaving Round 2’s verified architectural blockers unclosed
Category: integration gap
Evidence: # FILE: research/audits/context-pack-2026-11-20-blockers/DELTA.md (lines 1,55) (lines 8–20, 51) and # FILE: research/audits/context-pack-2026-11-20-blockers/EVIDENCE.md (lines 1,40) (lines 5–8) declare Round 2 closed while listing Round 1’s hydrogen-arch findings (circuit breaker, max_tokens, gateway retry, evidence recall, HistorianKey, parked-key warning, cache invalidation). Yet # FILE: docs/audit/2026-11-19-hydrogen-architecture-review-round2-12-findings-verified.md (lines 1,300) (lines 8–10, 17, 38–224) covers 12 distinct findings ([cli]/cli_exec, clarify plumbing, tools/builtin.rs:354 "notification" vs "notify", gateway_commands.rs duplication, config.rs:41 cross-chat leak, autonomous.rs:238 300s MCP respawn, RT-190, deferred.rs:128,150 cache bust). Proof they remain open: STATE.md line 37 keeps RT-190 OPEN, and the HEAD db218f15 dead-code audit still inspects gateway_commands.rs (line 147) and notes tool_cache.rs (F11 disposition) (line 115, confirming F11 was Round 1’s cache invalidation).
Proposed fix: Reconcile df1ea4c0..origin/main against Round 2’s actual F1–F12 ledger and close the verified non-decision-gated code fixes (tools/builtin.rs:354, autonomous.rs:238, config.rs:41 tenant scoping, tools/tool_search/deferred.rs:128,150), then cut a true Round-3 pack containing agent/{mod,run,stream}.rs, client.rs, and database.rs.
Falsifier: git log df1ea4c0..origin/main contains pushed commits fixing Round 2’s actual F1–F12 (builtin.rs:354 "notify", autonomous.rs:238 persistent McpManager, config.rs:41 per-session config, gateway_commands.rs removal).
Confidence: high
Blocker 2: gateway_runner.rs:3315 holds a parking_lot::Mutex (active_runs) across .await, creating an executor deadlock bottleneck masked by clippy’s 124-entry allowlist
Category: bottleneck
Evidence: # FILE: docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md (lines 1,178) (lines 149, 153–160) flags crates/operant-cli/src/gateway_runner.rs:3315 (plus gateway_runner/tests.rs) for clippy::await_holding_lock on parking_lot::Mutex active_runs across a gateway turn .await ("a deadlock is possible... correctness risk"), directly contradicting EVIDENCE.md line 31 ("28 mechanical warnings remain, all one-line fixes") and line 22 of the audit ("None is a correctness bug").
Proposed fix: Refactor crates/operant-cli/src/gateway_runner.rs:3315 (and gateway_runner/tests.rs) to drop the parking_lot::MutexGuard before .await (or switch active_runs to tokio::sync::Mutex), and deny clippy::await_holding_lock while pruning the 96 stale entries from scripts/clippy-warning-gate.sh.
Falsifier: crates/operant-cli/src/gateway_runner.rs:3315 drops every synchronous lock guard prior to any .await and cargo clippy -p operant-cli reports zero await_holding_lock warnings without allowlist suppression.
Confidence: high
Blocker 3: mcp.rs:471 leaves impl_cache_key unwired (#[expect(dead_code)]), causing MCP tool caches to serve stale tool schemas across server updates
Category: integration gap
Evidence: # FILE: docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md (lines 1,178) (lines 108–115) shows crates/operant-core/src/mcp.rs:471 (impl_cache_key(url) -> String) is still dead under #[expect(dead_code)], leaving tool_cache.rs (which still carries a sibling F11 TODO) keyed on URL only so restarted/upgraded MCP servers—including LAW-mandated sourcehound over MCP stdio—serve stale tool definitions; also flagged in DELTA.md line 38 and EVIDENCE.md line 16.
Proposed fix: Wire impl_cache_key at crates/operant-core/src/mcp.rs:471 into tool_cache.rs cache lookups/writes (keying on server implementation/version identity alongside URL/command) and remove #[expect(dead_code)].
Falsifier: tool_cache.rs already incorporates MCP server serverInfo.version/binary identity in its cache key and removing #[expect(dead_code)] at crates/operant-core/src/mcp.rs:471 triggers no unused warning (or keeping it fails -D clippy::unfulfilled_lint_expectations).
Confidence: high
Blocker 4: sidecar.rs:18 retains a dead-marked bridge_rx field after the drainer moved to tool_bridge::, leaving sidecar channel consumption split or silently dropped
Category: fragmentation
Evidence: # FILE: docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md (lines 1,178) (lines 117–122) documents sidecar.rs:18: bridge_rx as annotated dead with stale comment "currently only drained by the sidecar's supervise() loop" while admitting supervise() no longer reads it and tool_bridge:: owns the drainer; retaining an unread dead_code receiver either queues/drops unconsumed sidecar messages or duplicates channel endpoints.
Proposed fix: Transfer ownership of bridge_rx from sidecar.rs:18 directly into tool_bridge::’s drainer loop (removing any dead_code attribute) or delete bridge_rx if tool_bridge:: already reads a distinct receiver.
Falsifier: sidecar.rs:18 moves bridge_rx into tool_bridge:: at initialization without a dead_code suppression, or intentionally holds an unused sentinel with zero messages sent to its tx.
Confidence: medium

## SECTION 2: the markdown read (may be truncated by the read tool)

ExplorePlaygroundHistoryBuildNew appMy appsGalleryManageDashboardDocumentation

# Architecture Review: Blocker Analysis

61,668  tokens

 User 2:39 AM

# Reviewer prompt — round 3 (2026-11-20)

You are a fresh-context senior architect reviewing the operant project. You have no
session history with this project; the pack is all you know. Treat the pack's claims
as CLAIMS TO AUDIT, not as ground truth.

## The task

operant has just closed a 12-finding architecture review (round 2) and a dead-code audit.
The operator believes the project has reached a local plateau: no dead code, no stub
adapters, no duplicate registries, clippy green, all hermes-agent gaps closed. The
operator is asking whether the NEXT phase's blockers are bottlenecks, redundancies,
fragmentations, or integration gaps — or whether the system is in a steady state and the
operator should shift from architecture work to feature work.

## What I need from you

Return a short list of BLOCKER DECISIONS. For each:

1. **The finding** — what is the blocker, in one sentence.
1. **The evidence** — which file:line in the pack (or in the repo, if you can infer it
from the pack's citations) supports it.
1. **The proposed fix** — one concrete change that does NOT violate any law in the LAW
section.
1. **The falsifier** — what would prove this finding wrong.
1. **The category** — bottleneck, redundancy, fragmentation, or integration gap.

If the system is in a steady state with no new blockers, say so — that is a valid answer.
Do not invent findings to fill a list.

## What I do NOT need

- A restatement of the pack's claims.
- A re-litigation of rounds 1 or 2's findings (all closed).
- A re-enumeration of the dead-code audit (already done).
- A judgment call without a file:line or a falsifier.
- A proposal that violates any law in the LAW section.

## What would make this round worth the operator's session

A single blocker that the operator can fix in the next session, with a file:line and a
proposed fix, is worth more than a list of ten judgment calls. Quality over quantity.

## The pack's figures carry three attributes

When you cite a figure from the pack, state its population, basis, and selection
constraint. A figure without these is a claim, not evidence.

## Evidence classes

The pack uses [LIVE], [CERT], [IN-SAMPLE], [PRIOR], [UNCONFIRMED] to mark
evidence classes. If a figure's class is not stated, say so — that is a finding about
the pack's honesty, not the system's.

## The pack's law is binding

A proposal that violates a law in the LAW section is not a finding. The operator will
reject it. If you think a law is wrong, say so explicitly and explain why — but do not
propose a fix that requires changing a law.

## Output format

Code

```
## Blocker 1: <one sentence>
- Category: <bottleneck|redundancy|fragmentation|integration gap>
- Evidence: <file:line from the pack or repo>
- Proposed fix: <one concrete change>
- Falsifier: <what would prove this wrong>
- Confidence: <high|medium|low>

## Blocker 2: ...
(empty if steady state)
```

Keep the whole response under 1,200 words.

--- PACK (inline) ---

# CONTEXT PACK — context-pack-2026-11-20-blockers

## READING CONTRACT

One message, one file: every input this round needs is below. Do not open repository files to re-read
what is already quoted here — that spends your tool calls and adds nothing.

- Cite by anchor: # SECTION <NAME> or # FILE: <path> (lines a,b). A finding that rests on a figure
must name its anchor and keep the figure's evidence class ([LIVE], [CERT], [IN-SAMPLE],
[PRIOR], [UNCONFIRMED]). A figure with no anchor is an assumption: label it one.
- Lens families, where the pack carries them, are instruments, not vocabulary. Naming a lens earns
nothing; the move the lens forces is the finding.
- If a number you need is absent, say which one and continue under a stated assumption. The report is
judged on structure, arithmetic and falsifiers, not on filling gaps with plausible values.

# SECTION A-DELTA

# FILE: research/audits/context-pack-2026-11-20-blockers/DELTA.md (lines 1,55)

# Delta since round 2 (2026-11-19 → 2026-11-20)

This section carries what landed on origin/main since round 2's pack was cut
(df1ea4c0..origin/main). Each row is a pushed commit with a verified status.

## The Hydrogen campaign closed (iters 2671–2692)

Round 2 returned 12 findings. Every one was verified against the live tree and dispositioned:

## Residual cleanup closed (iters 2693–2702)

- **iter-2694**: OUTLINE.md crate-graph claim corrected (operant-core does NOT reach operant-plugins);
MCP server attribution fixed (server is in operant-cli); AGENTS.md slack.rs handle_update stale
claim corrected; TUI count 73→74. Flipped RT-136/RT-285 checkboxes (stale, closed by code).
- **iter-2695**: Deleted gateway/admin.rs (179 lines, zero production callers) + re-exports.
- **iter-2698**: Fixed 5 E303 dead file citations in library docs (write_approval.rs, schema/media.rs,
t

**NOTE: the markdown read above is TRUNCATED** (it returned 4966 chars, at the read tool's ~5 kB cap). Section 1 is the complete extraction and is the one to re-read.