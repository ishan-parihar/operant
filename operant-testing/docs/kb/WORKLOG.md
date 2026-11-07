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

**Reconciliation against `origin/main` @ `758dd4c5` (iter-1818)** found that seven of the
fourteen rows in the sweep's first report had already been closed upstream
(RT-234/iter-1783, RT-225/iter-1769, RT-249/iter-1800, RT-247/iter-1799,
RT-251/iter-1802). The gap register now carries that reconciliation and a layer column
so the next executor does not re-fix closed work.

**Open the sweep contributed that upstream had not:** the client/server `id` type
asymmetry (G-006), the 30 s tool timeout vs 60 s MCP timeout (G-007), three DEPLOY gaps
(G-004, G-008, G-013), the TUI lifecycle asymmetry (G-009, G-010), and the silent
index-degradation in `aft_grep`/`aft_glob` (G-011).

## 2026-11-06 — the fix landed, and the knowledge base landed with it

**iter-1822** (`092f53a0`) — the MCP drain-until-match fix, pushed to `origin/main`.
`cargo test -p operant-core --lib mcp::` → 27 passed, 0 failed. `cargo check -p operant-core --lib`
green. Only `crates/operant-core/src/mcp.rs` was staged; the peers' in-flight WIP in nine other
files was left untouched, because the shared tree carries their work.

**iter-1823** (`7e3253a6`) — the knowledge base itself: 41 files under `operant-testing/`, a
nested project-genome whose `lint` is clean and whose `doctor` reports healthy.

**iter-1824** (`5633d84a`) — the verify harness distinguished OPEN from FAIL. It had been
reporting a known-open gap (the junk dirs) as a failure, so the gate could never be green.
Only a regressed CLOSED finding now exits non-zero.

**The lesson the KB now carries.** The first report said operant had no auto-poke; upstream
had ported one five days earlier. Seven of fourteen rows were closed before the
reconciliation ran. A gap register is not a document — it is a live claim about a moving
tree, and it is only worth what its reconciliation discipline is worth.

## 2026-11-06 — the operator's five TUI reports, reconciled

The operator reported five TUI defects: setting goal did not work; the advisor preview
lumped everything instead of showing blocks; ctrl+enter did not queue; queued and goal
messages never reached the prompt-history; and the input area carried tool-tip text.

**All five were already fixed upstream** and are recorded as G-015…G-019, `closed-upstream`:

| report | closed by | what actually changed |
|---|---|---|
| goal does not stand | iter-1839 `15cec34e` | a STANDING goal injected into the volatile turn-context tail of every turn |
| advisor preview lumps everything | iter-1842 `23015b38` | one BLOCK per note; the payload is split from the card at the continuation marker |
| ctrl+enter does not queue | iter-1797 `2acd8efa` + iter-1839 `15cec34e` | the hint tells one truth; the accessor was `plan_mode`, a jcode vestige; the band mirrors the live queue |
| queue/goal missing from prompt-history | iter-1839 `15cec34e` | `remember_prompt_history` is the ONE writer |
| non-input text in the input area | iter-1845 `64094d60` | the hints are STATUS, painted on the row above the composer |

**The register was re-pinned** from `43a38696` (iter-1818) to `758dd4c5` (iter-1867) and
every anchor re-resolved — seven had drifted, which is exactly the decay the pin exists
to expose. The alias table also had to be corrected: the first report's `G-015`/`G-016`
(delegate, dup) were folded into `G-003`/`G-004` so the numbers could carry the new
reports without two rows claiming one id.

**`verify.sh` grew check 8** — the eight fix symbols for G-015…G-019 must still be present
in the source. A regression in any of them now fails the harness instead of waiting to be
found by the operator.
