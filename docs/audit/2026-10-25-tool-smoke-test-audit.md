# Audit — the 10-batch Agentic-Core Tool Smoke Test (2026-10-25)

**Auditor:** jcode (operator-agent session) · **Date:** 2026-10-26
**Subject:** the pasted "Agentic-Core Tool Smoke Test — Complete Report" (~50 calls, 10 batches), re-verified
against the live deployed binary (`operant 0.2.1`, md5 `a3d10964…` at audit start), the real sourcehound/aft
binaries, and the source tree at `iter-1291`.
**Question:** which of the report's ✅/⚠️ verdicts hold, where are the real bugs vs infrastructure gaps,
and what did the smoke test itself miss?

---

## Verdict summary

| # | Report claim | Re-verified | Class |
|---|---|---|---|
| 1 | Batch 4: `web_crawl` ⚠️ "needs web group enabled in sourcehound settings + restart" | **WRONG diagnosis, REAL bug underneath** — the web group is *not* disabled; the failure was a first-call ordering defect in operant itself. **Fixed by peer iter-1291** (verified live end-to-end: crawl now returns 1 page). | BUG (now fixed) |
| 2 | Batch 2: `aft_search` ⚠️ "index not ready (runtime_not_observed) — uses grep fallback" | **Half-wrong.** There is no automatic fallback: the tool *errors* and tells the model to use grep. Both index lanes report `runtime_not_observed` even though the aft schema defaults them to `true`; even `aft warmup --only search` does not clear it. The tool's own description promises a "lexical fallback" that does not exist. | BUG (description) + INTEGRATION GAP (lanes) |
| 3 | Batch 2: `aft_apply_patch` ⚠️ "requires specific header format" | **Not a tool bug.** Live-verified working with the documented `*** Begin Patch` dialect. The smoke run's model formatted the patch wrong; the tool description embeds the exact dialect + example. | MODEL-SIDE MISS |
| 4 | Batch 5: `process list` → "Empty process list" | **Correct behavior, ambiguous contract.** The tool lists the *agent's own spawned background processes* (session registry), not system processes. The description says "list all processes", which invites the misread. | DOC GAP |
| 5 | Batch 6: checkpoint "0 changed files — no tracked files in checkpoint" | **Correct but subtle.** Live-probed: checkpoints capture only *git-tracked* files. A checkpoint of an untracked file records `file_count: 0` and the restore is a silent no-op. The tool description does not say this. | DOC GAP (real trap) |
| 6 | Batch 6: `aft_undo` ⚠️ "no undo history" | Consistent with #5: the probe edits were in untracked files, so there was nothing in the undo history either. Not independently a bug. | MODEL-SIDE MISS |
| 7 | Batch 7: `mcp_management list_servers` → empty | **Correct**: `[mcp] servers = []` in the live config; no MCP servers are configured. | ✅ holds |
| 8 | Batch 8: `skill_view (operant)` → "not found" | **Correct**: no `operant` skill exists in `~/.operant/skills` (the `ms` skill does; the smoke caller likely named the wrong one). Clear error + available list is the designed behavior. | ✅ holds |
| 9 | Batch 8: `sop list` → "No SOPs found" | **Correct**: `~/.operant/sops/` has no `.toml` files. | ✅ holds |
| 10 | Batch 1/3/4/5/9/10 ✅s (echo, datetime, debug_*, skills_list, session_search, config_manage, cron, kanban, env_probe, timestamp, memory×5, web_search/fetch/scrape/extract, code_execution py+js, process spawn/wait, notify, aft_read/glob/outline/grep/write/edit, checkpoint/paths/list, ast_search, zoom, tool_search/describe/backend, learning_manage, browser navigate/snapshot, delegate_task, verify_task) | Spot-re-probed live: env_probe masks values (names+lengths only), memory save→recall roundtrip works, web_scrape/extract return real payloads, code_execution py 3.14.8 + js, browser navigate/snapshot works, secret masking holds. | ✅ holds |
| 11 | Summary: "All tools have been exercised… The agentic core is fully operational" | **Overstated.** 22 of the 80-93 registered tools were never exercised (list below), and two of the three "notable limitations" were misdiagnosed. | COVERAGE GAP |

---

## Finding 1 (the real bug): web_crawl's first-call v2 ordering — FIXED by peer iter-1291

The smoke report's diagnosis ("needs web group enabled in sourcehound settings + restart") was **wrong**:
`~/.config/sourcehound/settings.yml` has `tools.safe_profile: true` with the web group reachable —
live `tools/list` advertises 23 composite tools including `crawl`. The group-disabled hint the report
echoed is operant's *fallback* verdict when the server answers "unknown tool".

**Root cause (independently derived, then confirmed identical to the peer's):**
`WebCrawlTool::execute` consulted `crawl_wire_surface()` (which reads the advertised tool set) **before**
`call()` spawned the sourcehound child. On a cold process where `web_crawl` is the *first* sourcehound
call, the advertised set was still `None`, so the v1 name `web.crawl` was chosen — and sourcehound 1.8.0's
v2 surface only serves `crawl`, answering `tool not found`, which operant wrapped in the misleading
group-hint. Every *subsequent* call in the same process would succeed, which is why the report's
"restart" folklore seemed plausible.

Evidence: gateway log for a fresh `operant run` showed `Listed MCP stdio tools, count: 23` (including
`crawl`) *immediately after* the failing `web.crawl` dispatch; direct wire probes confirmed the server
serves `crawl` and rejects `web.crawl`; the existing unit tests masked the bug because they warmed the
handle with a probe `call` first.

**Fix (peer, `b0b9fa03` iter-1291):** `crawl_wire_surface` now forces the handshake (`self.client()`)
before resolving the name, with a v1-safe degradation when the spawn fails. **Live-verified end-to-end
post-fix**: `web_crawl example.com` → "1" (pages crawled), no refusal. The mutation-proven tests
(`first_crawl_call_resolves_the_v2_surface_before_dispatch`,
`first_crawl_call_on_a_v1_server_keeps_the_granular_name`,
`a_dead_sourcehound_binary_degrades_to_the_v1_surface_not_a_panic`) pin the ordering.

Note for the operator: the smoke report's remediation ("enable the web group in
`~/.config/sourcehound-mcp/settings.yml`") also named a path that does not exist on this box — the live
settings live at `~/.config/sourcehound/settings.yml` (the `-mcp` dir is only created when
`SOURCEHOUND_CONFIG_DIR` is pointed elsewhere). The hint string in `sourcehound.rs` still names the
`-mcp` path; worth a follow-up wording pass (it is only shown on genuine group refusals).

## Finding 2 (description bug + gap): aft_search's "lexical fallback" does not exist

Live behavior (`aft 0.62.0`): `aft_search` returns

```
[search_lanes_unavailable] aft_search has no ready index lane.
trigram: unavailable: runtime_not_observed
semantic: unavailable: runtime_not_observed
Use grep, or inspect the unavailable lane's configuration.
```

- The tool's model-facing description says "(lexical fallback when the semantic index is not enabled)"
  — **there is no automatic fallback**. The call fails; the error text *suggests* grep. A model that
  believes the description will call aft_search expecting degraded-but-working results.
- Both lanes report `runtime_not_observed` although the aft schema defaults `indexes.trigram` and
  `indexes.semantic` to `true` — i.e. something in the runtime never observed/started either lane.
  `aft warmup --only search` exits 0 (symbol cache loads, watcher starts) but does **not** clear the
  verdict. The bridge already runs a detached `aft warmup --only callgraph` per project root
  (aft_bridge.rs `warm_callgraph_detached`); there is no equivalent for the search lanes, and no
  operant-side surface to enable or diagnose them (`aft status` hangs/prints nothing usable in this
  environment).
- Meanwhile `aft_grep` works fine and its JSON honestly reports
  `"index":{"trigram":{"reason":"runtime_not_observed","status":"unavailable"}}` while still returning
  28 files of matches — so the *indexed* grep falls back to a direct scan transparently. The asymmetry
  is that grep has a real fallback and search does not, while search's description claims one.

Filed as **RT-195** (description honesty, one-line fix + test) and the lane enablement as part of the
same row's follow-up (operant-side lane warmup/diagnostic surface, decision needed).

## Finding 3 (coverage): 22 registered tools never exercised

The report's "All tools have been exercised across 10 batches" is false. Diffing the live registry
(80 enabled in the default config) against the ~51 exercised names leaves these unexercised:

`aft_ast_replace`, `aft_bash`, `aft_inspect`, `approval_request`, `browser_cdp`, `browser_dialog`,
`checkpoint`, `clarify`, `http_request`, `image_generate`, `osv_check`, `react_to_message`,
`send_message`, `session_insights`, `skill_manage`, `text_to_speech`, `todo`, `tool_call`,
`transcribe_audio`, `video_analyze`, `vision_analyze`, `working_diff`

(`file_read/write/search/list/state`, `terminal`, and `patch` are also unexercised but are
intentionally auto-disabled under `aft_enabled=true` — the AFT dedup from iter-51 — so they are
correctly absent, not missed. `scroll`/"authenticated" entries in the raw list are description-text
artifacts of list parsing, not tools.)

Notable high-risk unexercised tools: `browser_cdp`/`browser_dialog` (recently changed in iter-1246),
`vision_analyze` (honesty rework iters 1254/1261), `todo` (persistence rework iter-1243),
`aft_bash` (the AFT-backed terminal), `checkpoint`/`clarify` (agent-loop interactive tools that need a
turn context), and `send_message`/`react_to_message` (outbound side-effects — deliberately skip in a
smoke test, but then the report cannot claim "all tools"). Filed as **RT-196** (recommendation:
rerun the smoke harness with a second batch covering these, with side-effect tools behind an
explicitly-flagded consent pass).

## Finding 4 (doc gaps that read as bugs in the report)

1. **process list** — description says "list all processes"; it lists *this agent's spawned background
   sessions*. The smoke tester read "Empty process list" as a wart. One-line description fix.
2. **aft_checkpoint** — captures only *git-tracked* files. A smoke test that writes an untracked probe
   file gets `file_count: 0` and a silent no-op restore — exactly what the report hit, twice. The tool
   description should say so (this is a real operator trap, not just test hygiene).
3. **aft_apply_patch** — works as documented; the smoke run's failure was model-side formatting. No
   code change; noted so the next smoke run reads the dialect from the description before ⚠️-ing it.

## What the smoke report got right

- Every ✅ spot-checked re-verified live (10 direct probes across batches 1, 3, 4, 5, 9).
- Secret masking held under direct probing (values never rendered; names + lengths only).
- The three "notable limitations" were correctly identified as infrastructure-level rather than silent
  failures — the *categories* were right even where the *diagnoses* were wrong.

## Recommendations

1. ~~web_crawl first-call ordering~~ — **closed by iter-1291**; no further action.
2. Fix the `aft_search` description (remove the fallback promise or implement a real lexical fallback
   via `aft_grep` when lanes are unavailable) — RT-195.
3. Add an operant-side aft search-lane warmup/diagnostic (extend `warm_callgraph_detached` shape) or
   document the `aft setup` requirement — RT-195 follow-up.
4. Second smoke pass covering the 22 unexercised tools — RT-196.
5. Description pass: `process` list semantics, `aft_checkpoint` tracked-files-only — RT-196 bundle.
6. Wording pass on `SOURCEHOUND_GROUP_HINT` (names a config path that doesn't exist in the default
   layout) — folded into RT-195.

## Files/rows

- `docs/audit/2026-06-19-BUGS.md`: RT-195, RT-196 (open rows).
- Live-probe evidence: gateway.log lines at 22:06:15 (the pre-fix failure),
  post-fix `operant run` outputs in this session's worklog entry.
