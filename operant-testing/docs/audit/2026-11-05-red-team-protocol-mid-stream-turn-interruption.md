# red-team protocol: mid-stream turn interruption

Dated-frozen: audited `2026-11-05`; findings mark verified vs asserted.

## Scope

Every sourcehound-backed tool in the agentic-core, plus the turn-finalization path that
produced the operator-visible notice "⚠️ Turn ended early — the wrap-up ran out of room
mid-summary". Reproduction was 3× per failing tool, with the source traced to a
`file:line` in the parent repo.

## Findings

- **[verified]** The MCP stdio client read ONE line from the child's stdout and treated it
  as the response, then hard-failed when `rpc_response.id != request_id`. There was no
  drain, no resync and no reconnect on that path — one foreign line wedged the client for
  the rest of the process's life. `crates/operant-core/src/mcp.rs:1396` (stdio) and
  `:659` (HTTP) at `758dd4c5`.
- **[verified]** The *server* is not at fault. A live probe of sourcehound 1.8.0 over stdio
  echoed the request id correctly: initialize 1→1, tools/list 7→7, tools/call 9→9.
- **[verified]** Nine consecutive failures were observed, each reporting
  `expected >N, got <M` — the client reading one response behind, permanently.
- **[verified]** The fix is a drain-until-match loop with a parked-response buffer
  (`MCP_BUFFERED_RESPONSE_CAP = 64`), a bounded drain (`MCP_MAX_RESPONSE_DRAIN = 64`
  lines, `MCP_DRAIN_READ_TIMEOUT_SECS = 5` per line) and a bounded-failure message that
  names the drain instead of hanging. The iter-187 guarantee — a response is only ever
  returned to the caller whose id it carries — is preserved and pinned by a test.
- **[verified]** `cargo test -p operant-core --lib mcp::` → **27 passed, 0 failed**. The
  never-matching-server test went from RED for 60 s to GREEN in 5 s.
- **[verified]** The turn-early notice the operator saw was ALREADY FIXED upstream before
  this audit: RT-234 (iter-1783, `8264cdea`) split the notice into the two real causes
  and added a bounded auto-poke; RT-225 (iter-1769, `6bc1b7af`) kept the wrap-up in the
  conversation and transcript and continued a provider cut-off. The sweep's first report
  claimed no auto-poke existed — that claim was false and is corrected in
  `kb/red-team/03-reconciliation-vs-upstream.md`.
- **[asserted]** The 30 s tool timeout (`crates/operant-cli/src/cmd_setup.rs:979`) cutting
  calls whose transport allows 60 s (`crates/operant-core/src/mcp.rs:399`) is the cause of
  the five deferred tools' uniform timeout. Consistent with observation; not yet proven by
  a controlled run at a raised timeout.

## What this changed

`kb/gap-register.md` rows G-005 (the defect, fixed here), G-006 (client/server `id` type
asymmetry), G-007 (the timeout budget), and the `closed-upstream` verdicts for G-001/G-002.
