# Audit: advisor delivery + sourcehound composite live verification

Status: closed
Date frozen: 2026-10-28
Owner: agent-jcode (operator session, "Sisyphus")
Method: raw-protocol probes (curl against the configured advisory route;
stdio JSON-RPC against the live sourcehound binary) + live E2E `operant
run` invocations + unit/mutation test batteries in the shared tree.

## What was audited

1. The operator's report that the advisor/recap integration "does not
   work like oh-my-pi" (Stream 2), and
2. The operant-agent self-report's Stream 3 claims: MCP servers not
   connected; five sourcehound composite schema mismatches (social_feed
   `target`, web_crawl float `limit`, cloakctl_call `profile`,
   workflow_run `chain_name`, gov_search bill designator); env gaps.

## Findings

### F1 — Advisor delivery was the defect (Stream 2), now ported

- The configured route + advisory model WORK: direct curl with the
  operator's JWT against `https://aroute.ishanparihar.com/v1` and model
  `small-stack` returns a completion (resolves to reflection/beam-free).
  Credentials are not the failure.
- The model is a REASONING model whose `content` can arrive empty when
  `max_tokens` is small (the reasoning consumes the budget,
  `finish_reason: length`). Both advisor and recap send requests with
  `max_tokens: None`, so this is a probe hazard, not a live defect.
- The REAL defect: operant's advisor delivered notes ONLY to the TUI
  (BackgroundReview card). The module doc claimed omp "never lets
  advisor output reach the model" — a misreading of omp
  (`session-advisors.ts` #routeAdvice / #deliverAdvisorBatch /
  resolveAdvisorDeliveryChannel: aside/steer/preserve ALL deliver into
  the primary's context; the quarantine + guidance framing exist
  BECAUSE notes are model-visible). Fixed per the frozen plan
  `docs/plans/2026-10-28-omp-faithful-advisor-delivery-route-notes-into-the-acting-lo.md`
  (see its "Deliberately NOT ported" list for the YAGNI cuts).
- Timing fact that shaped the port: TurnEnd fires once per `run()`, at
  the success-path return; the side-call completes after. Notes
  therefore structurally arrive when the agent is IDLE — the live
  channel policy is omp's idle/terminal arm (continue vs preserve), and
  the streaming aside/steer channels are unreachable in operant today.

### F2 — The five sourcehound schema mismatches were already fixed upstream

Live probe (`sourcehound mcp` → `tools/list`, v1.8.0, the installed
binary): `social.feed` advertises `target` ("Subreddit name or
`u/<name>` post history"), `crawl` advertises `limit` as
`integer`/uint32, `cloakctl.call` requires `tool`, `workflow.run`
requires `chain_name`, `gov.search` carries the full kind enum. The
self-report was written against an older sourcehound. Operant passes
the server's own schemas through verbatim
(`sourcehound_composites.rs`: "the sourcehound server's OWN inputSchema
… so a sourcehound release … does not silently desynchronise"), so the
fix arrived by upstream release + the peer's iter-1379 bridge. No
operant change needed.

### F3 — "MCP servers not connected" is the deferred-bridge design, not a defect

`operant mcp list` reports none because the operator's config has
`servers = []`. sourcehound (the one real server on this box) is served
through the BUILT-IN bridge: the four web tools + 23 composites
register as first-class operant tools over a lazily-spawned stdio
child, deliberately NOT through `[[mcp.servers]]` (that would
double-register the same tools). E2E proves the path works: live
`operant run` invocations through `papers_search` return cited answers
(RustCompCert). A web-search MCP example remains in
`operant.example.toml` for genuinely external servers.

### F4 — Composite-call failures under load: real, transient, mechanism identified, NOT fixed

Two consecutive live `operant run` invocations at 03:29-03:33Z and
04:16-04:18Z failed with "MCP response ID mismatches and timeouts"
while `papers_search` worked over raw stdio (5s) — both failures
coincided with heavy box load (release build + concurrent probes).
Three consecutive clean reruns afterward (incl. the original failing
prompt) return correct cited answers. Mechanism (read, measured, not
conjectured): all sourcehound tools share ONE stdio child behind one
io mutex (`send_request_impl` holds the guard across write AND read);
the 8-worker tool pool fires composite calls concurrently; default-depth
`news.fetch` alone measured 22.6s over MCP (quick: ~2s; the CLI with
its own defaults: 25s). Heavy calls queued behind heavy calls plus box
load can push individual reads past the 60s deadline, which trips the
hung-server force-reconnect — killing the shared child under the other
callers' feet (the "ID mismatch" family). No deterministic repro was
achieved; fixing without one (raising the deadline, per-call children,
or dropping the io lock across reads) is speculative tuning against a
noise floor. RECOMMENDED follow-up if it recurs: per-call timeout that
starts when the io guard is ACQUIRED (it already does — verify under a
deterministic load harness before changing anything), or a bounded
per-server call queue with admission control.

### F5 — Env gaps remain open (not blocking)

`SOURCEHOUND_CONTACT_EMAIL` unset, `FAL_KEY` unset (image_gen),
vision model config unset. All three are operator-owned credentials;
the audit did not modify `~/.operant/.env`. The inert-slot warnings
(iter-1349) already surface the unread-slot class honestly.

## Verdicts

- Stream 2 root cause found and fixed (delivery channel port, mutation-
  proven); shipped pending the peer's session_hub landing (feed F0134).
- Stream 3 schema items: CLOSED as already-fixed upstream (F2).
- Stream 3 MCP item: CLOSED as design-not-defect (F3).
- Stream 3 composite flakiness: RECORDED with mechanism (F4), no code
  change this session; reopen on recurrence.
- Env gaps: OPEN, operator-owned.
