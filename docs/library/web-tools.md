# Web tools — sourcehound intelligence

Owns: the web intelligence tools (`web_search`, `web_fetch`,
`web_scrape`, `web_extract`, `web_crawl`), the sourcehound v2 composite
surface, the stdio/MCP seam they ride, and the engine's background
self-update.
Not for: the browser engine's own verbs and CDP access
([browser.md](browser.md)), the MCP protocol itself and its transports
([mcp.md](mcp.md)), SSRF/path-safety internals
([security.md](security.md)), the memory-backed research recall
([memory.md](memory.md)).

Source: `crates/operant-core/src/tools/sourcehound.rs` (the bridge + three
tools), `crates/operant-core/src/tools/sourcehound_composites.rs` (the 23
v2 composites), `crates/operant-core/src/tools/sourcehound_update.rs` (the
update kick), `crates/operant-core/src/binary_updater.rs` (shared
download/verify/install machinery), `crates/operant-core/src/tools/web_tools.rs`
(`web_search`, `web_fetch`), `crates/operant-core/src/tools/web_providers/`
(the one search provider), `crates/operant-core/src/tools/http_tool.rs`
(`http_request`).

## The seam

operant does not link sourcehound. `tools/sourcehound.rs` speaks to a
**subprocess over MCP stdio**: `find_sourcehound_binary()` resolves
`SOURCEHOUND_BINARY` (a configured path that does not exist is skipped
with a warning, then PATH is tried) and falls back to `sourcehound` on
PATH; `Sourcehound::global()` runs `<binary> mcp`. The handle is a
`OnceLock` clone whose state is shared, so construction is free and the
child spawns on the **first** call, not at registry build. `McpStdioClient`
does the `initialize` handshake and caches `tools/list`; `Sourcehound::call`
then dispatches by tool name and unwraps the reply
(`structuredContent` first, else the first text block parsed as JSON,
else `{"text": …}`).

Why a subprocess and not a cargo dependency: the comment in
`operant-core/Cargo.toml` records that the `sourcehound_mcp` git
dependency is held out (the vendored obscura-render breaks against
registry taffy, plus a cosmic-text feature-union divergence), and the
library pulls an unconditional render stack. The same seam AFT uses,
kept independently updatable.

## Data flow

1. **Register.** `register_builtin_tools` registers `WebSearchTool`,
   `WebFetchTool`, and `WebScrapeTool::global()` / `WebExtractTool::global()`
   / `WebCrawlTool::global()`, then `sourcehound_composites::register_composites`.
   Each sourcehound-backed tool's `is_available()` returns
   `Sourcehound::is_available()`, and the registry's `get_schemas`
   filters on that — so a host without the binary never sees the tools
   advertised at all.
2. **Validate before dispatch.** Every URL argument goes through
   `ssrf_verdict(&url)` first (fail-closed on DNS errors), so a private,
   loopback, link-local, CGNAT or metadata address is refused with the
   block message and the engine is never asked.
3. **Dispatch, then judge.** `Sourcehound::call` deliberately does NOT
   consult the advertised tool set to decide availability: sourcehound
   1.8.0's default v2 surface lists 23 composite names and none of the
   granular v1 names operant calls, yet answers them. A local refusal on
   the advertised set failed every web tool on a stock install. So the call
   goes out, and only the server's own `unknown tool` verdict adds the
   `SOURCEHOUND_GROUP_HINT`.
4. **Ask for JSON.** Every call sets `format: "json"` (RT-188/189): the
   server strips `structuredContent` unless the call names it, and its
   default TOON text cannot be parsed into a page list or a result array.
   `parse_search_results` additionally has a TOON fallback
   (`toon_results` + `split_toon_field`) so a build that ignores `format`
   still yields rows.
5. **Crawl surface negotiation.** `crawl_wire_surface()` reads the
   advertised set on the call's own connect and picks `web.crawl` (v1) or
   `crawl` (v2). v2's `crawl` composite defaults to `mode: "map"`, which
   returns a sitemap link list and reports success — so the tool adds
   `mode: "crawl"` when the v2 name is in use. A first-in-process crawl
   resolves the set BEFORE reading it, because a crawl is frequently the
   first sourcehound call and the un-handshaked read used to walk the v1
   name into a v2 server's "tool not found".

## The tools, as registered

| tool | wire call | arguments |
|---|---|---|
| `web_search` | `web.search` | `{query, max_results, format:"json"}`; falls back to the provider chain |
| `web_scrape` | `web.scrape` | `{url, format:"json"}` |
| `web_extract` | `web.extract` | `{url, format:"json"}` |
| `web_crawl` | `web.crawl` / `crawl` | `{url, max_depth (2), max_pages (20), format:"json", mode?:"crawl"}` |
| `web_fetch` | plain reqwest, not sourcehound | `{url, method?, headers?, body?, timeout?}` |

`web_search` goes through `web_providers::SourcehoundSearchProvider` and a
one-candidate chain (`build_search_candidates` returns exactly one box:
`SourcehoundSearchProvider` — the tavily/exa/duckduckgo/searxng
alternatives were deleted in plan 11C1). Each candidate is bounded by
`tools.web.search_timeout_secs`, and an empty result set is an ERROR whose
message names the fix (`operant update --integrations`) when the binary is
missing, rather than a silent empty list.

`web_fetch` is the general-purpose HTTP tool and is NOT sourcehound-shaped:
it validates the scheme is http/https, applies the same `ssrf_verdict`,
disables redirects (the guard validates the initial URL only; a 3xx is
returned to the model, which can re-issue against `Location`), uppercases
the method so `post` works (RT-306), and reads the body through the
**shared bounded reader** `read_bounded_body` (2 MiB cap,
`MAX_HTTP_BODY_BYTES`, truncation marker appended) that `http_request`
also uses — closing R48-7's unbounded `response.text()`.

## The 23 composites

`sourcehound_composites.rs` registers sourcehound's v2 composite surface
as first-class tools, one proxy struct per spec from a static table
(`composite_specs()`, 23 `spec(...)` entries). The name mapping is
mechanical: `composite_tool_name` replaces `.` with `_`, so
`papers.search` → `papers_search`; `composite_names()` is derived from
the same table, so the name surface and the registration cannot drift.

Each spec's `parameters` is the **server's own `inputSchema`**, with
`format` trimmed out (operant sets it), and the description is the
server's own text. Every composite declares
`ToolExposure::Deferred` **on its type** — eagerly described the 23 cost
~4,738 tokens on every request (measured 2026-10-25); as `tool_search`
catalog lines they cost ~760 and only on the turn that searches. A
request that does not research carries the four web tools alone.

The contract drift caveat is stated in the module: the guards are
`composite_names_map_dots_to_underscores`,
`composite_table_has_no_duplicates`, `every_composite_is_deferred_by_type`
and `every_composite_carries_a_real_description` — naming, uniqueness,
deferred-ness, description presence. **No test compares the table against
a live server.**

## The self-update

`sourcehound_update::kick_update_check()` runs from the first
`Sourcehound::call` in a process (so the first call is never blocked on
it). It builds an `IntegrationSpec { name: "sourcehound", repo:
"ishan-parihar/sourcehound", asset_marker: "linux-x86_64",
content_tarball: false, install: InstallKind::ReplaceOnPath(bin) }` and
hands it to `binary_updater::kick_session_update`, which is a no-op when
`OPERANT_NO_AUTO_UPDATE` is set non-empty (and always in a test build),
fires at most once per process per integration, and polls at most once
per 6 h on disk (`THROTTLE`).

`binary_updater::update` reads the installed version from
`<bin> --version` (a binary that will not report one is left alone),
resolves the latest release through the `gh` CLI (no `gh` or no auth
means no poll), and installs only when the remote tag is newer. It
verifies SHA256SUMS when the release carries them; when it does not, it
installs anyway and **logs a warning that the download was not verified**
rather than pretending protection exists. The replace is atomic
(`atomic_replace`). `operant doctor` / `/version` can report the
installed-vs-latest pair without installing (`sourcehound_update::check`).

## Configuration

| key | default | what it changes |
|---|---|---|
| `tools.web.search_timeout_secs` | 15 | per-provider budget for `web_search` |
| `tools.web.fetch_timeout_secs` | 30 | `web_fetch` timeout when the call omits `timeout` |
| `tools.web.default_results` | 10 | `web_search` result count when `num_results` is omitted |
| `tools.web.max_results` | 20 | hard cap on a `web_search` request |
| `tools.web.user_agent` | `"Mozilla/5.0 (compatible; OperantAgent/0.1)"` | accepted, not read: `web_search` / `web_fetch` build their own clients |
| `browser.provider` | `"sourcehound"` | accepted for migration; a non-sourcehound value warns and is ignored |

`tools.web.search_url`, `preferred_provider`, `tavily_api_key`,
`exa_api_key` and `searxng_base_url` are **retired keys kept only so an
old config still parses** (`deny_unknown_fields` would reject them). They
are skipped at serialization, read by no code, and `warn_legacy_keys`
prints one line naming the replacement when a value is set.

Environment: `SOURCEHOUND_BINARY` overrides the binary path;
`OPERANT_NO_AUTO_UPDATE` disables the background poll.

## Surfaces

- Tools: `web_search`, `web_fetch`, `web_scrape`, `web_extract`,
  `web_crawl`, plus the 23 composites (all in `builtin_tool_names()`, so
  `operant tools list` shows them, `tools disable` matches them, and the
  deferred ones still resolve through the `tool_search` bridge:
  `tool_search` → `tool_describe` → `tool_call`).
- CLI: `operant update integrations` / `operant update --integrations`
  (the same hint the missing-binary error prints), `operant doctor`
  (integration version row).
- Runtime: the agent calls them as ordinary tools; the gateway maps
  `web_*` results through the same tool-result path as every other tool.

## Failure modes

- **Binary missing** → the tool is never advertised
  (`is_available()` false), and a direct call returns
  `SOURCEHOUND_INSTALL_HINT` naming the `cargo install --git` line and
  the `SOURCEHOUND_BINARY` override.
- **Configured path does not exist** → a `tracing::warn!` naming
  `SOURCEHOUND_BINARY`, then PATH resolution; the tool still works if
  PATH has it.
- **Group disabled on the server** → the server answers `unknown tool`;
  `SOURCEHOUND_GROUP_HINT` names the real settings path
  (`~/.config/sourcehound/settings.yml`, or under
  `SOURCEHOUND_CONFIG_DIR` / `XDG_CONFIG_HOME`) and points at
  `sourcehound status` as the ground truth.
- **TOON reply despite `format: "json"`** → the search path degrades to
  the TOON parser; the scrape/extract/crawl paths degrade to
  `first_text_field`, and a crawl with no parseable page list reads as
  empty.
- **SSRF refusal** → `ssrf_verdict`'s block message, before any engine
  call.
- **Empty search results** → an error that distinguishes "not installed"
  (run `operant update --integrations`) from "the engine returned
  nothing" (retry).

## Intention

Web intelligence rides a subprocess speaking MCP because the engine is
independently versioned, independently updatable, and does not force a
render stack into operant's dependency tree — the same reasoning the AFT
bridge follows. The second honest choice is dispatch-before-judgement:
operant asks the server what it serves rather than deciding from a
cached advertisement, because the advertisement is only a hint and the
server's own "unknown tool" is the only authoritative absence signal.
That is what keeps a v1 install and a v2 install on the same four tool
names.
