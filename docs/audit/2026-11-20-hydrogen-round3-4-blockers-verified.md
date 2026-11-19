# Hydrogen round 3 (2026-11-20) — 4 blockers, verified

A fresh-context review round (Gemini 4 Hydrogen, `gemini-4-hydrogen`, effort `xhigh`)
over a **67,141 B context pack** cut from a delta, a fresh state, an ask, the law, evidence,
and the architecture/known-gaps/working slices of `AGENTS.md` plus the round-2 and dead-code
audits.

**Every finding below was re-verified against the live tree before adoption.** The reviewer
worked from the pack's claims; each carries a CONFIRMED / CONFIRMED-NARROWER / REFUTED verdict
with `file:line` evidence.

Round artefacts: `research/audits/context-pack-2026-11-20-blockers/`
(`pack.spec`, `PACK-AS-SENT.md`, `REVIEWER-PROMPT.md`, `RESPONSE.md`, `DISPATCH.json`,
`SEND.log`, `DELTA.md`, `STATE.md`, `ASK.md`, `LAW.md`, `EVIDENCE.md`).

## The round's parameters, as recorded

| fact | value |
|---|---|
| venue | AI Studio (`aistudio.google.com`), fresh chat, model pinned in URL |
| model / effort | `gemini-4-hydrogen` / `xhigh` (confirmed by `doctor` and by `send`) |
| pack size | 67,141 B (cap 110k; ~17k tokens of a 1M shared window) |
| transport | inline, sentinel last (the venue has no pack-file attach path) |
| reply | 8,855 chars, DOM scrape (`RESPONSE.md` archived from page 280) |
| response provenance | `fetched_at 2026-11-20T02:40:35`, page 280 |

## Verification method

Each finding was checked against `origin/main` with targeted greps. The reviewer's own
proposed falsifier was run first; a finding is **CONFIRMED** only when the falsifier fails
against the tree. Where the reviewer's reasoning overstated the impact, the finding is
marked **CONFIRMED (narrower than claimed)** and the correct scope is stated.

## The pack-honesty audit (reviewer's first finding — ACCEPTED)

The reviewer found that zero figures in the pack carry the required `[LIVE]`, `[CERT]`,
`[IN-SAMPLE]`, `[PRIOR]`, or `[UNCONFIRMED]` tags. This is correct — the pack's figures
(28 clippy warnings, 14+4 `#[expect]` sites, 0 lib warnings) are all stated without
evidence classes. This is a defect in the pack, not in the system, but it is a real
finding: the operator's evidence discipline requires classes, and the pack omitted them.

The reviewer also found arithmetic discrepancies:
- "28 residual clippy warnings" sums to 27 in the audit's table (3+1×11+2+1+2+1+2+5).
  **CONFIRMED** — the table's rows sum to 27, not 28. The "28" in the prose is off by one.
- "14 in lib + 4 in cli" (18 sites) lists 20 rows in the audit's table.
  **CONFIRMED-NARROWER** — 20 rows, but 2 are sub-items (e.g., `ConnectionStats::all` /
  `is_healthy` / `summary` are 3 rows for one struct). The count of 18 is the number of
  `#[expect]` annotations, not the number of table rows.

## Blocker 1: DELTA.md misrecords Round 1's Hydrogen fixes as Round 2's closure — CONFIRMED

**Category:** integration gap
**Evidence:** `research/audits/context-pack-2026-11-20-blockers/DELTA.md` lines 8–20 list
F1–F12 with descriptions like "circuit breaker", "max_tokens", "gateway retry" — these are
**Round 1's** findings (`docs/audit/2026-11-19-hydrogen-architecture-review-12-findings-verified.md`
F1–F12). **Round 2's** findings
(`docs/audit/2026-11-19-hydrogen-architecture-review-round2-12-findings-verified.md` F1–F12)
are completely different: `[cli]` CLI-admission, clarify plumbing, `builtin_tool_names()`
disagreement, slash-command duplication, config split, repeated runtime costs, skill-root
claim, declared surfaces with no consumer, terminal truncation sniffing, tool re-injection
cache bust, MCP IR vs MCP 2026-07-28, four orchestration layers.

The DELTA.md claims "Round 2 returned 12 findings; all 12 are closed" but the fixes listed
(iters 2671–2692) closed **Round 1's** findings, not Round 2's. Round 2's findings were
verified and documented but their fixes were deferred.

**Proposed fix:** Rewrite DELTA.md to distinguish Round 1 (closed) from Round 2 (verified,
fixes deferred). Cut a future round that carries Round 2's actual findings as the analysis
section, and the deferred fixes as the ask.

**Falsifier:** `git log df1ea4c0..origin/main` contains pushed commits fixing Round 2's
actual F1–F12 (e.g., `builtin.rs:354` "notify", `autonomous.rs:238` persistent McpManager,
`config.rs:41` per-session config). **FALSIFIER FAILS** — the commits in that range fix
Round 1's findings, not Round 2's.

**Confidence:** high

## Blocker 2: gateway_runner.rs holds a Mutex across .await — REFUTED

**Category:** bottleneck (claimed)
**Evidence claimed:** `crates/operant-cli/src/gateway_runner.rs:3315` holds a
`parking_lot::Mutex` `active_runs` across a gateway turn `.await`.
**Verification:** `grep -n "active_runs" crates/operant-cli/src/gateway_runner.rs` returns
**zero matches**. The `await_holding_lock` warnings the clippy gate reports are in
`gateway_runner/tests.rs` (lines 1658, 1821), both annotated with
`#[expect(clippy::await_holding_lock, reason = "test-only: the std mutex serialises
process-env mutation across the whole test")]`. These are test-only, not production code.

**Falsifier:** `grep -n "active_runs" crates/operant-cli/src/gateway_runner.rs` returns a
non-empty result. **FALSIFIER SUCCEEDS** — the field doesn't exist.

**Confidence:** high (refuted)

## Blocker 3: mcp.rs:471 impl_cache_key unwired — REFUTED

**Category:** integration gap (claimed)
**Evidence claimed:** `crates/operant-core/src/mcp.rs:471` has `impl_cache_key(url) ->
String` annotated `#[expect(dead_code)]`, leaving tool caches keyed on URL only.
**Verification:** `grep -rn "impl_cache_key" crates/` returns **zero matches**. The function
does not exist anywhere in the codebase. `grep -rn "tool_cache" crates/operant-core/src/`
also returns zero matches — there is no `tool_cache.rs`. The reviewer's cited evidence was
my dead-code audit's mention of `impl_cache_key`, but that function was either removed or
never existed at the cited line.

**Falsifier:** `grep -rn "impl_cache_key" crates/` returns a non-empty result.
**FALSIFIER SUCCEEDS** — the function doesn't exist.

**Confidence:** high (refuted)

## Blocker 4: sidecar.rs:18 bridge_rx has a stale dead_code annotation — CONFIRMED (narrower)

**Category:** fragmentation (claimed)
**Evidence claimed:** `crates/operant-core/src/tools/kernel/sidecar.rs:18` has `bridge_rx`
annotated dead, but the drainer moved to `tool_bridge::`, leaving the field unread.
**Verification:** The field IS consumed — `crates/operant-core/src/tools/kernel/tool_bridge.rs:30`
does `handle.bridge_rx.lock().await`. The `#[allow(dead_code)]` annotation at
`sidecar.rs:72` is **stale** (it says "consumed by tool_bridge drain loop (phase 2.5
wiring)" but the wiring is done, so the annotation should be `#[expect]` with a reason, or
removed entirely). The reviewer's proposed fix (transfer ownership or delete) is wrong — the
field is already consumed, just the annotation is stale.

**Falsifier:** `grep -rn "bridge_rx" crates/operant-core/src/tools/kernel/tool_bridge.rs`
returns a non-empty result. **FALSIFIER FAILS** — the field IS consumed.

**Confidence:** high (confirmed, narrower than claimed)

## Tally

| # | finding | verdict | category | action |
|---|---|---|---|---|
| 1 | Round 1 vs Round 2 conflation | CONFIRMED | integration gap | Rewrite DELTA.md; cut a true round-3 pack |
| 2 | gateway_runner await_holding_lock | REFUTED | (bottleneck) | No action — test-only, properly annotated |
| 3 | impl_cache_key unwired | REFUTED | (integration gap) | No action — function doesn't exist |
| 4 | bridge_rx stale annotation | CONFIRMED (narrower) | fragmentation | Remove stale `#[allow(dead_code)]`, the field is consumed |

## What the reviewer got right that I should adopt

1. **The pack-honesty defect is real.** My pack carried figures without evidence classes.
   Future packs must tag every figure `[LIVE]`/`[CERT]`/`[IN-SAMPLE]`/`[PRIOR]`/`[UNCONFIRMED]`.
2. **The Round 1 vs Round 2 conflation is a real error in my DELTA.md.** I listed Round 1's
   fixes as Round 2's closure. This must be corrected before the next round.
3. **The arithmetic in the dead-code audit is off by one** (27, not 28, in the table).
4. **The `bridge_rx` annotation is stale.** It should be removed or converted to `#[expect]`
   with a reason that reflects the live wiring.

## What the reviewer got wrong

1. **Blocker 2 (await_holding_lock)** — the cited code doesn't exist. The only
   `await_holding_lock` warnings are in test code with proper `#[expect]` annotations.
2. **Blocker 3 (impl_cache_key)** — the function doesn't exist anywhere in the codebase.
   The reviewer trusted my dead-code audit's claim without the audit being able to point
   at a real line.

## What I could not assess (pack gaps, stated honestly)

The reviewer named these and they are correct: `agent/mod.rs` (6,370 LOC), `agent/run.rs`
(5,769 LOC), `agent/stream.rs`, `client.rs` (4,427 LOC), `database.rs` (7,517 LOC) and
`docs/library/{loop,context,memory,mcp,persistence,sub-agents}.md` were **not in this
round's pack** — round 1 covered them, but this round deliberately swapped them out to fit
the byte cap. So per-turn allocation severity in `build_messages`,
`ToolRegistry::get_schemas()` caching, streaming backpressure and SQLite lock contention
remain unmeasured. A round-4 delta should carry them.