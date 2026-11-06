# CLOSED — tried and rejected

Append-only. An entry names the item, the verdict, the measurement that killed it
(a path), and the re-open condition. Re-proposing needs new evidence.

| item | verdict | measurement | reopen-if |
|---|---|---|---|
| "The MCP server is corrupting ids — fix the transport" | **rejected** | `kb/red-team/05-mcp-correlation.md`: live probe of sourcehound 1.8.0 echoes the request id correctly (1→1, 7→7, 9→9). The defect was the client's single-read-then-hard-fail with no drain. | a probe against a *different* server fails to echo |
| "`aft_inspect` is broken in this repo — fix the tool" | **rejected** | The fix landed upstream: RT-247 CLOSED iter-1799 (`e08a643c`). The live failure is the pre-fix bridge binary v0.63.5. | the new binary is deployed and `aft_inspect` still fails |
| "The cron scheduler is broken" | **rejected** | A disposable no-agent job (`cron_98bfd8e6`, `*/5 * * * *`) was created and its script actually ran: `/tmp/rt-probe-cron.log`. The paused jobs are a model-prefix config gap. | a scheduled job with a valid model does not run |
| "`config_manage set` does not persist — that is a bug" | **rejected** | Runtime-only by design; the TOML still held `max_iterations = 90` while the runtime reported `50`. Restore the baseline and move on. | — |
| "Updating the gap register by hand is sufficient" | **rejected** | Seven of fourteen rows were already closed upstream; the hand-maintained register sent the executor to re-fix closed work. Reconciliation against `origin/main` is now a documented step (`kb/red-team/03-reconciliation-vs-upstream.md`). | — |
| "The `session_search` SQL error is a persistent defect" | **rejected** | The same query returned results on retry. Transient, not reproducing. | it recurs |
