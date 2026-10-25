# Memory-wire + sourcehound integration gaps — 2026-10-25

Audit of the two native integrations in the agentic core: **memory-wire**
(`agentic-utility/memory/memory-wire`, embedded in-process as the ONE memory
provider) and **sourcehound** (`agentic-utility/internet/sourcehound`, the
ONE web/browser engine, driven over its stdio MCP server). Everything below
was verified against the LIVE deployed binary (`operant 0.2.1`, build of
2026-10-25 09:02), the LIVE sourcehound 1.8.0 install, and the LIVE memory
stores on this machine — not by reading code alone.

Findings are filed in `docs/audit/2026-06-19-BUGS.md` as **RT-188 ..
RT-194**. Two live defects (RT-188, RT-189) were FIXED in the same iteration;
the rest are decisions or features, filed with their options.

---

## 1. What was verified working (live, through the binary)

| Surface | Result |
|---|---|
| `web_search` → sourcehound `web.search` | ✅ 3 real results (tokio query); JSON contract from iter-1210 holds |
| `web_scrape` → `web.scrape` | ✅ example.com markdown returned; structured payload |
| `web_extract` → `web.extract` | ✅ main-content extraction, boilerplate stripped |
| `web_crawl` → v2 `crawl` | ❌ **RT-188 — FIXED this iteration** |
| `browser` (cloakctl navigate/read/act) | ⚠️ provider path worked; **cdp path dead — RT-189 FIXED** |
| `browser_cdp` / `accessibility_tree` / `cookies_*` | ❌ **RT-189 — FIXED this iteration** (all go through `cloakctl.cdp`) |
| `memory_store` tool | ✅ stored + recallable; round-trips to the same bank |
| `prefetch` auto-recall per turn | ✅ recalled 3 relevant facts on a live query, injected under `<memory_context>` |
| `sync_turn` auto-retention | ✅ fires after each completed turn (verified in the live bank) |
| Seat/sender bank isolation (KB-6/KB-8) | ✅ code-verified: `turn_retains()` gate + `MemoryScope` partitioning |

## 2. The two live defects (both fixed same-day)

### RT-188 — `web_crawl` answered a different question with `success: true`

The deployed binary, asked to crawl, replied *"the sourcehound backend
reports it does not serve `web.crawl`"* and offered to fall back to
web_scrape. Direct MCP probes against the same server showed the truth:
the v2 `crawl` composite exists and answers, but operant's call was doubly
wrong:

1. **No `mode: "crawl"`.** v2's `crawl` is dual-mode — `mode` defaults to
   `"map"` (a sitemap/link listing). Verified live:
   `crawl {url}` → `{links: [], count: 0, provider: "sitemap-parser"}`;
   `crawl {url, mode:"crawl"}` → `pages[1]{url,title,content,depth}`. A
   success answer to the wrong question is worse than an error: the agent
   reads "0 pages" as "site has nothing".
2. **No `format: "json"`** (iter-1210's contract, applied to search only).
   Even a correct crawl answered in TOON text that `unwrap_tool_result`
   cannot parse into a page list.

Fix: `crawl_wire_name` → `crawl_wire_surface` returning
`(wire_name, needs_mode)`, which forces the handshake before reading the
advertised set (the live re-verification caught a THIRD stacked defect —
ordering: the first crawl in a process read the advertised set before the
spawn, so it still sent the v1 name; the probe-call preambles in the unit
tests had masked it); the call sends `mode: "crawl"` only on v2 (a v1
server never receives the key) plus `format: "json"` always. The same
format contract was extended to scrape/extract (same latent defect, closed
here rather than left for the next audit). Mutation-proved: removing the
mode insert fails `a_v2_crawl_names_crawl_mode_and_asks_for_json`; removing
the handshake-force fails `first_crawl_call_resolves_the_v2_surface_before_dispatch`.

### RT-189 — the whole CDP-backed browser surface was dead

Live gateway/session logs showed every `browser_cdp` and
`accessibility_tree` call failing with
`sourcehound 'cloakctl.cdp' published no ws_url for profile 'operant'` —
while the same call WITH `format: "json"` in its arguments returns a full
structured payload including `ws_url`. Root cause: sourcehound's
`dispatch_value` (their `src/server.rs:1074-1078`) strips
`structuredContent` unless the call's **top-level** `arguments.format ==
"json"`; operant called with `{profile}` only, so `unwrap_tool_result`
wrapped TOON text as `{"text": ...}` and the `ws_url` field lookup missed.
The engine, profile, and endpoint were all live — operant just could not
read the address.

Fix: `CdpBrowserSession::start` carries `format: "json"`; the same contract
was applied to `cloakctl.navigate` and `cloakctl.act` (whose TOON replies
degraded every click/type/scroll result to a rescued text blob).
**`cloakctl.read` keeps `format: "markdown"` deliberately**: its `format`
field is BOTH the read format and (via a serde-flatten collision with
`OutputOptions`) the only output-switch key the server can see — `json`
there would yield the TEXT walker's body instead of markdown. That
collision is a sourcehound-side schema wart, documented at our call site.

Also recorded: my first browser test failed because example.com was
redesigned (no `<h1>` anymore) — the engine rendered the page faithfully.
Verify against a page with known structure.

## 3. Memory-wire integration gaps (RT-190..193, decisions/features)

The embedded provider (`memory_wire.rs`) implements the engine correctly —
panic containment, bank scoping, seat isolation, the one-time MEMORY.md
import, secret redaction, CLI/agent unification. What it does NOT do is use
the hooks the infrastructure already emits:

- **RT-190 — two disconnected bank families.** operant reads
  `~/.operant/memory_wire.sqlite` (bank `operant`, 2157 rows); the
  `memory-wire serve` daemon reads `~/.local/share/memory-wire/memory.db`
  (banks `omp` 2539 rows, `memory-wire` 1218) and serves every OTHER
  harness via `memory-wire connect`. Knowledge captured in operant is
  invisible to claude-code/codex/opencode/copilot and vice versa. Three
  options filed (join the serve / bridge the stores / accept the boundary);
  the choice changes what every other agent sees, so it is the user's call.
- **RT-191 — only 2 of 7 lifecycle hooks implemented.** The agent loop
  emits `on_session_end`, `on_delegation`, `queue_prefetch`,
  `on_session_switch`, `on_turn_start` (plus `on_pre_compress`, which the
  peer's RT-181 row already files); `MemoryWireProvider` drops them all on
  the floor (trait-default no-ops). Measured cost: the live bank holds
  only 4 `User: `-prefixed turn rows EVER, against 2137 imported legacy
  rows — `sync_turn` (500/2000 chars, head-truncated, drops the question
  on long answers) is the only write path that fires. memory-wire's own
  GAP-6 diagnosed this exact shape in hermes; the port inherited it.
- **RT-192 — five memory tools, three semantics, ~310 tokens/turn.**
  `memory_store` + `memory_save` (twin stores),
  `memory_search` + `memory_smart_search` (the same call twice),
  `memory_recall`. All five route through identical provider methods; the
  background-review whitelist names the old trio while the registry
  advertises all five. Consolidation is a registry change, not an engine
  change — but which names survive is the user's call.
- **RT-193 — the fused score is discarded.** `format_hits` drops
  `ScoredMemory.score`; `memory_search` fabricates `relevance: 1.0` for
  every hit. memory-wire's whole retrieval-efficacy program was built to
  carry that number; operant is where it dies. Fix shape filed; gating on
  scores stays deferred per memory-wire's own doctrine until a labelled
  dev set exists.

## 4. Sourcehound integration gaps (RT-194)

- **RT-194 — the 23-tool v2 composite surface has no first-class operant
  tool.** `papers.search`, `news.fetch`, `data.query`, `recon.target`,
  `social.search`, `video.transcript`, `archive`, `security.vuln`,
  `gov.search`, `patents.search`, `company.lookup`, `workflow.run` etc.
  are all reachable through operant's MCP client but invisible to the
  model unless the user hand-configures `sourcehound` as an MCP server.
  Registering all 23 eagerly would add ~2-3k tokens to every request. The
  efficient shape: extend `is_deferrable` to the sourcehound composites
  and expose them through the existing `tool_search` bridge.

## 5. Architecture — the token/tool-call-efficient tooling set

The objective stated for this audit. The constraints, MEASURED (not
estimated — every number below was taken from the live binary's own
`tools/list` over `operant mcp serve`, serialized in the exact wire format
`client.rs::build_chat_request` sends after `clean_schema`, and
counted at the agent loop's own chars/4 heuristic):

1. **74 tools ride every request at ~14.5k tokens of wire schema**
   (72 from the mcp-serve registry at ~14.3k clean + the two memory
   provider twins at ~200). `tools.tool_search` (auto) defers only
   `mcp_*`-prefixed tools — the entire native surface is eager. The
   heaviest single schemas: `todo` ~777, `delegate_task` ~773,
   `verify_task` ~545, `skill_manage` ~449, `kanban` ~435,
   `browser` ~430. Every schema also carries the shared `intent` +
   `accept_large_output` fields (`with_intent()`, added centrally at
   publication).
2. **Memory costs ~590 tokens per request for five tools that are two
   tools.** The live trio (`memory_store` ~160, `memory_search` ~142,
   `memory_recall` ~126, intent-augmented) plus the two provider twins
   (`memory_save` ~106, `memory_smart_search` ~97). Consolidating to
   `memory_store` + `memory_search(+key)` saves ~300 tokens per
   request — every request, not just turns that touch memory.
3. **Every sourcehound call must send `format: "json"`** — the server
   strips `structuredContent` otherwise (the RT-188/189 root cause; now
   pinned by tests on all six call sites).
4. **The v2 composites are dual-mode and folded**: `crawl` = map+crawl,
   `read` = extract+page. A bridge that forwards one name must forward
   the mode/args each side expects (the RT-188 lesson, generalized).
5. **The 23 sourcehound composites measured**: eager registration would
   add ~4,738 tokens to EVERY request (live tools/list serialized in
   wire format); the same 23 as `tool_search` catalog lines (name +
   first-sentence description) cost ~760 tokens, paid only on the turn
   the model actually searches. The bridge saves ~3,978 tokens per
   request that does not search — and the heaviest composite
   (`news.fetch`, ~810 tokens eager) is the clearest case.

The recommended shape, in priority order:

1. **Fix-first (done): every sourcehound call carries the JSON contract.**
   Six call sites pinned by tests; the mock servers reproduce the live
   server's dispatch behavior, so a future regression fails locally.
2. **Consolidate the memory tools to two** (RT-192): `memory_store`
   (+key param) and `memory_search` (+optional key param). Saves ~300
   tokens per REQUEST (measured: five tools ≈ 590 tokens, two ≈ 290) and
   removes a wrong-tool-choice failure mode. The provider twins
   (`memory_save`, `memory_smart_search`) retire; the review whitelist
   already names the survivors.
3. **Extend the `tool_search` deferral to heavy native tools** (RT-194's
   mechanism): a toolset/partition change so the 23 sourcehound
   composites (and optionally the AFT 20) ride the deferred bridge —
   measured: eager costs ~4.7k tokens on every request, bridge catalog
   lines cost ~760 tokens on the searching turn only. The same partition
   could defer the heaviest native schemas (`todo` ~777, `delegate_task`
   ~773, `verify_task` ~545) for a further ~2k. The bridge already
   exists and is proven (`tool_search` auto, names listing, materialise
   on demand); this is a partition edit, not a new mechanism.
4. **Implement the remaining memory hooks** (RT-191) so session-end and
   delegation prose lands in the bank — the recall side then has real
   corpus to rank, which is also the precondition for any future score
   gating (RT-193).
5. **Decide the bank topology** (RT-190) BEFORE (4): new rows should land
   in the store the user wants them read from.

Items 1 is shipped; 2-5 are filed with mechanisms and wait on user
decisions where the decision is the user's (names, topology), and on the
bridge-partition architecture where it changes the model-visible surface
mid-stream.
