# MCP correlation: root cause, live probe, and the iter-1822 fix

**Question asked:** why did every sourcehound-backed tool occasionally fail with
`MCP response ID mismatch: expected N, got M (possible out-of-order response)`?

## Live probe of the actual server (sourcehound 1.8.0, stdio)

```
id 1  initialize   -> response id = 1
id 7  tools/list   -> response id = 7
id 9  tools/call   -> response id = 9
```

**The server echoes the request id correctly.** So the transport is not the culprit.

## The actual defect

`operant-core/src/mcp.rs`, stdio client, `send_request_impl`:

1. it read **one** line from the child's stdout and treated it as the response;
2. it then compared `rpc_response.id != request_id` at `mcp.rs:695` (HTTP) / the stdio arm;
3. on a single foreign line — a duplicate, an orphan, a log line the banner-skip
   did not catch — it returned a `ReqError::Protocol`.
4. There is **no drain, no resync and no reconnect** on that error path. The client
   was then permanently wedged: every later request consumed a *stale* response and
   failed the same check. Live evidence: 9 consecutive calls all reported `expected >N, got <M`
   for the rest of the process's life.

`step_result.rs:1092` proves this class bugs the harness, not just the agent:
a failing `aft_inspect` answered the model with a dead end while the real diagnostics sat
in a 64-line stderr ring nobody read (that one is fixed upstream, RT-247 / iter-1799).

## The fix (iter-1822, in this workspace)

- `read_response_line(io, timeout_secs)` — extracted, banner-tolerant, parameterised.
- Drain loop: keep reading until the response carrying `request_id` arrives.
- A response for a **later** id is parked in `buffered` (cap `MCP_BUFFERED_RESPONSE_CAP = 64`)
  so the call that owns it still claims it — **no response is dropped and none is handed
  to the wrong caller**.
- A response for an **earlier** id cannot be claimed (ids only increase) and is dropped.
- `MCP_MAX_RESPONSE_DRAIN = 64` lines; `MCP_DRAIN_READ_TIMEOUT_SECS = 5` — a server that
  never matches fails **boundedly** and names the drain, instead of hanging 60 s per line.
- 10c6475c (`crates/operant-core/src/mcp.rs:1257`) shows the same shape exists in one other
  path; the drain fixes it.

## Proof

`a_server_that_never_matches_the_request_id_fails_boundedly` — RED for 60 s, GREEN in 5 s.
`an_orphan_response_does_not_wedge_the_stdio_client` — the second call used to fail forever.
`a_parked_orphan_is_never_handed_to_a_caller` — the iter-187 guard survives.
`cargo test -p operant-core --lib mcp::` → **27 passed, 0 failed**.
