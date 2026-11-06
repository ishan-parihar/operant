# Architectural Blind Spots — Operant Agentic-Core (anchored)

**Reconciled to `origin/main` @ `43a38696` (iter-1818).** `Anchor` = `file:line` at that commit;
verify with `git show 43a38696:<path> | sed -n '<line>p'` before editing.

1. **The MCP stdio client had no resync path.** One foreign stdout line wedged the client for
   the process's lifetime. `operant-core/src/mcp.rs:695` (the check), `:1355` (the drain, iter-1822).
   Fixed by iter-1822 in this workspace; the same drain shape already existed at `:1257`.
2. **The client's `id` type is narrower than the server's.** Client `id: u64` (`mcp.rs:356`);
   server `id: Value` (`mcp_serve.rs:39`), which emits and asserts string ids
   (`mcp_serve.rs:822`). A string-echo server cannot be deserialised. → G-006.
3. **Two timeout budgets that do not agree.** Tool timeout defaults to 30 s
   (`cmd_setup.rs:979`); the MCP HTTP timeout is 60 s (`mcp.rs:418`). Every MCP-backed tool is
   therefore cut off before its own transport would give up. → G-007.
4. **The deployed bridge lags the repo.** `aft_inspect`'s "unknown error" is fixed in source
   (RT-247, iter-1799) but the live bridge is the pre-fix v0.63.5 binary. Source-reconciliation
   alone cannot see this. → G-004, G-008, G-013.
5. **A source fix is not a deployed fix.** Same class: cron jobs stay paused because the model
   they name has no provider prefix — the resolving code exists (`provider.rs:135`,
   `transcription_tool.rs:68`) and the job rows do not use it.
6. **The TUI is not yet the whole interface.** `/cron` is a command with no pane; 72 TUI
   commands have no gateway twin (canonical RT-179). The overlay catalogue is a hand-written
   list that has already drifted twice (RT-251).
7. **Index-backed search silently degrades to a capped walk.** `aft_grep`/`aft_glob` report a
   partial result with no signal that the index was not used (RT-195 family).
8. **No in-harness regression harness for these findings.** Everything above lives in this KB
   but nothing re-runs it. Red-team/docs/06-verify.md is that harness; wire it into a nightly job.
