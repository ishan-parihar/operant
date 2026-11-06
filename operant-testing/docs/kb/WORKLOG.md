# Worklog

Append-only: `genome log "<entry>"`, one per unit of work.

## 2026-11-06 — MCP correlation root cause, and the drain-until-match fix (iter-1822)

Ran the exhaustive red-team protocol against the agentic-core. Every reported failure
was reproduced three times, then classified into five layers — the crucial one being
`DEPLOY-LAYER`, which a source-only reading cannot see.

**Root cause (G-005).** The MCP stdio client read ONE line from the child's stdout and
treated it as the response, then hard-failed on `id != request_id`. On a single foreign
line — a duplicate, an orphan, a mis-skipped banner line — it returned a protocol error
with **no drain, no resync and no reconnect**. The client was then permanently wedged:
every later request consumed a stale response and failed the same check. Live evidence
was 9 consecutive failures, each reporting `expected >N, got <M`, for the rest of the
process's life.

**Live probe of the server cleared it.** sourcehound 1.8.0 over stdio echoes the request
id correctly for initialize (1→1), tools/list (7→7) and tools/call (9→9). The transport
was never the culprit.

**Fix.** `read_response_line(io, timeout_secs)` extracted, banner-tolerant and
parameterised; a drain loop keeps reading until the response carrying `request_id`
arrives; a response for a LATER id is parked in `buffered` (cap 64) so the call that
owns it claims it; a response for an EARLIER id is dropped; the drain is bounded at 64
lines / 5 s per line and names the drain on failure. The iter-187 guard — a response is
only ever returned to the caller whose id it carries — survives.

**Proof.** `a_server_that_never_matches_the_request_id_fails_boundedly` went from
RED for 60 s to GREEN in 5 s. `an_orphan_response_does_not_wedge_the_stdio_client` —
the second call used to fail forever. `cargo test -p operant-core --lib mcp::` → 27
passed, 0 failed.

**Reconciliation against `origin/main` @ `43a38696` (iter-1818)** found that seven of the
fourteen rows in the sweep's first report had already been closed upstream
(RT-234/iter-1783, RT-225/iter-1769, RT-249/iter-1800, RT-247/iter-1799,
RT-251/iter-1802). The gap register now carries that reconciliation and a layer column
so the next executor does not re-fix closed work.

**Open the sweep contributed that upstream had not:** the client/server `id` type
asymmetry (G-006), the 30 s tool timeout vs 60 s MCP timeout (G-007), three DEPLOY gaps
(G-004, G-008, G-013), the TUI lifecycle asymmetry (G-009, G-010), and the silent
index-degradation in `aft_grep`/`aft_glob` (G-011).
