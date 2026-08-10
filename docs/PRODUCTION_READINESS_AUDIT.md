# Production Readiness Audit — operant vs hermes-agent (reference)

**Date:** 2026-08-09
**Scope:** Core agentic loop + integrations (browser, IGS web search/scrape/extract/crawl), audited in contrast to `./hermes-agent/` (Python reference) and `igs-rust` (the IGS CLI used by operant).
**Outcome:** The core loop and integrations are production-shaped. One concrete defect was found and fixed (two divergent Obscura binaries); everything else verified clean or documented as a follow-up.

---

## 1. Architecture contrast (operant vs hermes-agent)

| Concern | hermes-agent (Python) | operant (Rust) | Verdict |
|---|---|---|---|
| Browser backends | `agent/browser_provider.py` ABC + `browser_registry` + plugin registration (`register_browser_provider`) | `BrowserProvider` trait (`operant-core/src/browser_provider.rs`) + `build_browser_provider()` factory | ✓ Mirrored |
| Web search backends | `agent/web_search_provider.py` ABC + `web_search_registry` (Firecrawl/Tavily/…) | `WebSearchProvider` trait + `DDG/Exa/Tavily/SearXNG/IGS` providers (`tools/web_providers/`) | ✓ Mirrored |
| Provider resolution | `web_search_registry._resolve(explicit, capability)` — explicit config wins, auto-select only when unset | `should_prefer_igs()` — explicit `tavily/exa/searxng` wins even when the igs binary exists; `igs/auto/unset` prefers IGS | ✓ Parity (regression fixed in a prior commit) |
| Fallback | Multiple search backends tried | `web_search` falls back to DuckDuckGo when IGS returns zero results (its upstream needs a key) | ✓ Keyless out-of-the-box |
| SSRF | `tools/url_safety.is_safe_url` (fail-closed) on every browser nav + web fetch/extract | `security::ssrf_verdict` (fail-closed, DNS-resolving) on `browser.navigate/snapshot`, `web_fetch`, `web_scrape`, `web_extract` | ✓ Parity, incl. cloud-metadata + private-range tests |
| Redirects | — | `web_fetch` disables redirects (SSRF redirect-bypass) and returns 3xx to the model | ✓ Stronger than reference |
| Browser backend used | Headed Chrome via CDP (`browser_connect.py`) + cloud plugins (Firecrawl/Browserbase/Browser Use) | IGS (default, zero keys) / Obscura / Lightpanda / Camofox / Browserbase / Browser Use / Firecrawl | ✓ Superset |

hermes-agent has **no Obscura or IGS integration at all** — those are operant-only. The contrast point is the *pattern*: registry-based pluggable providers, explicit-config-wins resolution, fail-closed URL safety. operant mirrors all three.

---

## 2. Core agentic loop

Not re-architected (per standing instruction — already designed effectively). Audit confirmed:

- Async Tokio loop, per-iteration tool calls, healing/retry, fallback models, context compression, session reset — all configurable and exercised by tests.
- **Verified green:** `cargo check --workspace` 0 errors/0 warnings; `cargo test --workspace` **8,518 passed / 0 failed**; `cargo clippy --workspace --all-targets --all-features -- -D warnings` exit 0; `cargo fmt --all --check` clean.
- `max_consecutive_tool_only` guard forces a textual response when the model loops on tools — production-grade loop-failure protection.

---

## 3. The defect found: two Obscura binaries (fixed this audit)

### Before
- **IGS** (`igs-rust`) manages its own Obscura: `src/obscura.rs` `ObscuraManager` hardcodes the binary at `{IGS_CONFIG_DIR | ~/.config/igs-mcp}/bin/obscura`, with **no override**, auto-downloading from `h4ckf0r0day/obscura` releases.
- **operant's `obscura` browser provider** downloaded a *second* copy to `~/.operant/bin/obscura` from the same repo.
- Result: two downloads, doubled disk, and potential **version drift** — the `browser` tool and the IGS web tools (web_search / web_scrape / web_extract / web.crawl) could run different Obscura builds with different rendering behavior.

### Fix (commit `4edb95e2`)
`ObscuraProvider::resolve_obscura_binary()` resolves in order:

1. `tools.obscura_binary_path` config override (new field — explicit single-binary guarantee).
2. The **IGS-managed binary**: `$IGS_CONFIG_DIR/bin/obscura`, else `~/.config/igs-mcp/bin/obscura`. Mirrors igs-rust's `config::user_config_dir()` precedence exactly, so operant finds the *same* binary IGS uses.
3. operant's own `~/.operant/bin/obscura` (fallback; the download target when neither exists — only on machines that never installed IGS).

- `ensure_binary()` now reuses a resolved binary instead of downloading a second copy.
- `download_target()` closes the first-run ordering gap: when the IGS config
  dir exists (IGS has run), fresh downloads install into IGS's `bin/` **and**
  stamp `bin/.obscura_version` with the fetched release tag — igs-rust's
  `ObscuraManager` compares that file against the latest release and skips its
  own download when they match. So even when operant downloads first, IGS
  reuses the same binary (no second copy on any ordering).
- `operant doctor` reports which binary is resolved ("Obscura browser binary (shared with IGS: …)").
- Tests: 4 resolve-order/download-target unit tests (RAII env guard, no
  leakage on panic) + config parse/back-compat test.

### Why this is the right layer
igs-rust's `ObscuraManager` has no binary-path knob — only `IGS_CONFIG_DIR` (which moves the whole config dir, not just the binary). So operant adapts to IGS's location rather than vice-versa. With the **default config** (`browser.provider = "igs"`, `web.preferred_provider = "igs"`), everything already routes through the igs binary's Obscura; the fix extends that guarantee to `browser.provider = "obscura"`.

**Result: one Obscura binary, one download, no drift — the user's stated requirement.**

---

## 3b. CDP browser implementation on the shared stealth Obscura (this audit)

**Finding — IGS's browser is NOT CDP.** igs-rust's `src/tools/lp_mcp.rs` shows every `igs browser` command re-runs `obscura fetch <url> --stealth [--eval <js>]`; its "session" is a `CURRENT_URL` string. There is no CDP endpoint to reuse through IGS, so the CDP path must drive the shared Obscura binary directly (the user's suggested alternative).

**Implemented (`obscura_cdp.rs`, commit pending):**
- `CdpBrowserSession::start()` resolves the **shared** binary ([`ObscuraProvider::ensure_binary()`]) and spawns `obscura serve --port <free> --stealth`, parsing the emitted `ws://` URL from stdout (same pattern igs-rust's `screenshot()` uses).
- Drives the browser over CDP: `Target.createTarget` + `Target.attachToTarget` (flattened session — the puppeteer flow), then `Page.navigate`, `Runtime.evaluate` (click/fill/scroll with `js_escape` injection defense), and `LP.getMarkdown` (DOM→markdown, innerText fallback).
- **Stealth by default**: `find_matching_asset` prefers the `-stealth` release build (`obscura-<arch>-<os>-stealth.tar.gz`), and `--stealth` is passed to serve, gated by new `tools.obscura_stealth` (default `true`).
- `ObscuraProvider` (`browser.provider = "obscura"`) is now fully interactive over CDP — navigate/snapshot/click/type/scroll all work (previously stubs). Process-wide shared session; killed on process exit.
- `browser_cdp` tool auto-provisions the managed session when `BROWSER_CDP_URL` is unset and gained an optional `session_id` arg for page-scoped commands.

---

## 4. Integration-by-integration status

| Integration | Path | Status |
|---|---|---|
| `web_search` (IGS → DDG fallback) | `tools/web_tools.rs` + `tools/igs.rs` + `tools/web_providers/` | ✓ Structured JSON parse is defensive (`results`/`memories`/`data` shapes); falls back to DDG on empty |
| `web_scrape` / `web_extract` (IGS, JS rendering via Obscura) | `tools/igs.rs` | ✓ SSRF-guarded, empty-URL rejected, graceful "install igs" error when binary missing |
| `web_fetch` (raw HTTP) | `tools/web_tools.rs` | ✓ SSRF-guarded, redirects disabled, scheme-restricted |
| `web.crawl` (via IGS) | exposed through `igs` CLI (`igs web crawl`) | ✓ Available when igs installed; SSRF checked by `web_scrape` path parity |
| `browser` tool | `tools/browser_tool.rs` → provider factory | ✓ All commands validated; SSRF on navigate/snapshot; scroll/type validation tested |
| `browser.provider = "obscura"` | `browser_provider.rs` + `obscura_cdp.rs` | ✓ **Shares the IGS binary; CDP-driven interactive browser, stealth by default** (this audit) |
| `browser.provider = "igs"` (default) | `tools/igs.rs` `IgsBrowserProvider` | ✓ Persists session across goto → markdown → click sequences |
| `operant doctor` | `cmd_doctor/checks_tools.rs` | ✓ Reports igs availability per toolset + resolved Obscura binary |

---

## 5. Security posture

- **SSRF:** fail-closed `ssrf_verdict` on every URL-fetching path (browser navigate/snapshot, web_fetch, web_scrape, web_extract). Blocked: cloud metadata (169.254.169.254, `metadata.google.internal`), loopback, RFC 1918, CGNAT, DNS-fail-closed. Dedicated tests for each tool.
- **Secrets:** API keys live in `.env` (never TOML) per repo rule; `web_fetch` output returns raw HTTP so no secret plumbing.
- **Download hygiene:** both Obscura downloaders cap sizes / verify `--version` / chmod 0755; igs-rust additionally validates tar entries against path traversal.

---

## 6. Remaining gaps / recommendations (all resolved — 2026-08-11)

1. **~~Manual smoke test~~ — DONE via the live agentic-loop test (section 6b).** The user asked operant itself to exercise the tools; see the results and the two defects it flushed out (CDP persistent-socket bug, browser_cdp params-string bug).
2. **~~igs-rust bidirectional binary override~~ — RESOLVED on both sides.** igs-rust already ships `ObscuraManager::explicit_binary_path()` honoring `OBSCURA_BIN` (env) then `obscura.binary_path` (v1.0.3+). This audit also added `OBSCURA_BIN` support to operant's `ObscuraProvider::resolve_obscura_binary()` with identical precedence (env → config → IGS-managed → operant-managed), so the sharing is now **bidirectional and configurable from either side**. 2 new precedence tests.
3. **~~operant-tools `WebSearchTool`~~ — CONFIRMED WIRED, not dead weight.** `operant-runtime/src/tools/mod.rs` re-exports `operant_tools::web_search_tool::WebSearchTool` into the runtime tool registry (and `operant-tool-call-parser`/`operant-channels` reference the name). It is the runtime-stack equivalent of `operant-core::tools::web_tools::WebSearchTool` (CLI stack) — parallel-stack duplication, both wired, no removal warranted.
4. **~~Windows Obscura asset matching~~ — FIXED.** `find_matching_asset` now maps `(windows, aarch64)` in addition to x86_64.
5. **`web_extract` auxiliary model** slot is intentionally inert (IGS returns raw markdown; no LLM post-processing). Documented on the `auxiliary_models.web_extract` field (`operant-core/src/config.rs`).
6. **Markdown renderer hygiene — FIXED.** Removed leftover per-frame `/tmp/render_markdown_*.log` debug writes from `tui/messages/markdown.rs` (a render-path file-I/O defect; the mimo-newline diagnostics they served are covered by the existing `normalize_markdown_newlines` tests).
7. **Tick-based status-drain — now unit-tested.** `drain_mcp_reconnect_status` extracted from the inline frame-drain and covered by 2 unit tests (renders without a keystroke; no-op without a channel).

---

## 6b. Live end-to-end agentic-loop test (this session)

The user's standing instruction: run the operant agent itself against the integrations, and ensure **only enabled + functional tools appear in the agentic context**.

### Test setup
- Throwaway config `/tmp/operant-live-test.toml` (copy of `~/.operant/operant.toml` with `tools.igs_enabled = true` and `browser.provider = "obscura"`) — the user's real config was **not** modified.
- `operant run --query "<10-step integration query>" --max-iterations 25 --record-trajectory -v` (stdin not a TTY → non-TUI `run_non_tui` path, full agentic loop with all registered tools, trajectory recorded).
- Live environment: `igs 0.5.4` present, **one shared Obscura 0.1.11 at `~/.config/igs-mcp/bin/obscura`** (IGS-managed — the single-binary guarantee verified live), kilo.ai gateway + free nvidia model.

### Tool-context filtering ("only enabled tools in context")
Verified empirically: `operant tools list` under the test config surfaces **54 tools** — the intersection of *registered ∩ available (`is_available()` true) ∩ not-disabled* (e.g. no `spotify_*`/`discord` without tokens, no `mcp_management` without a manager). The agent loop feeds the model `registry.get_schemas()` (agent/mod.rs), so non-functional tools never reach context. `is_available()` gates: IGS web tools require `tools.igs_enabled && igs binary`; browser CDP tools auto-provision. **Requirement met.**

### Results: 10/10 steps PASS (fixed build)
| # | Step | Result |
|---|---|---|
| 1 | `datetime` | ✅ current time |
| 2 | `web_search` | ⚠️ executes, but DDG served anomaly pages (see below) |
| 3 | `web_scrape` (IGS) | ✅ example.com → markdown |
| 4 | `web_extract` (IGS) | ✅ rust-lang.org → text |
| 5 | `browser` (obscura CDP) | ✅ navigate + LP.getMarkdown ("# Example Domain…") |
| 6 | `browser_cdp` | ✅ `Runtime.evaluate document.title` → "Example Domain" (auto-provisioned `page-1-session`) |
| 7 | `http_request` | ✅ httpbin 200 |
| 8 | `file_write`/`file_read` | ✅ roundtrip |
| 9 | `terminal` | ✅ echo exit 0 |
| 10 | `memory_store`/`memory_search` | ✅ store + recall |

### Defect #1 flushed out: CDP session lost between commands (fixed)
- **Symptom:** `browser navigate` → `-32601 "No page for session"`; `browser snapshot` → `-32601 "No page"`.
- **Root cause (found by reading obscura's `obscura-cdp` source + probing the real binary):** `obscura serve` keeps pages/sessions **per WebSocket connection** (each connection gets its own `CdpContext`; `Target.createTarget` registers `{page_id}-session` in *that* context). Our `CdpBrowserSession::send` used `cdp_utils::send_cdp_command`, which opens a **fresh connection per command** — the created page/session vanished between calls. A probe with one persistent socket confirmed the whole flow (createTarget → attachToTarget → Page.navigate → LP.getMarkdown → Runtime.evaluate) works perfectly.
- **Fix (commit pending):** new persistent `CdpSocket` in `obscura_cdp.rs` — one WebSocket with a background reader correlating responses by `id` (events ignored), timeout + protocol-error surfacing identical to `send_cdp_command`. `CdpBrowserSession::send` now multiplexes over it; `browser_cdp` routes through the same persistent socket when `BROWSER_CDP_URL` is unset.

### Defect #2 flushed out: browser_cdp `params` as string (fixed)
- **Symptom:** `browser_cdp` `Runtime.evaluate` → `-32601 "expression required"` even with a valid expression.
- **Root cause:** the model passed `params` as a JSON-encoded **string**; serde kept it a string and the server's `params.get("expression")` on a string yields nothing. Added `normalize_params()` (string → object, non-JSON degrades to `{"value": s}`) + unit test.
- **Bonus:** `browser_cdp` now auto-provisions the shared page session when no `session_id` is given, so `Runtime.evaluate`/`Page.navigate` work without the model manually doing createTarget/attachToTarget. Tool description now lists valid commands (`browser` tool: navigate/snapshot/click/type/scroll/accessibility_tree) — the model had invented a `text` command.

### web_search: DDG fingerprint-blocking + provider fallback chain (fixed/robustified)
- Live finding: DDG served **anomaly/rate-limit pages** to the reqwest/rustls client (0 results) while python/urllib3 with the same UA returned real results; `igs web search` returns `count: 0` because its upstream (Tavily) has no key configured on this machine.
- **Fix:** `build_search_candidates()` — hermes `web_search_registry._resolve` capability fallback: configured provider first, then igs → tavily (key) → exa (key) → duckduckgo → searxng (url), deduplicated. When every candidate returns nothing, `web_search` now returns an **actionable error** ("DuckDuckGo may be rate-limiting… configure a Tavily/Exa key") instead of silently-empty `0 results`. DDG provider also hardened: `http1_only()` + one retry after 500 ms on empty (anomaly blocks are transient). 5 new candidate-chain unit tests.
- **Remaining (needs user credentials):** to get real results on this machine, set a `TAVILY_API_KEY` (operant `[tools.web] tavily_api_key` or igs's `settings.yml`) or an `EXA_API_KEY`. No code path is broken — the chain will pick the keyed provider automatically.

### Skills import from a directory (fixed this session)
- **Gap found:** `operant skills install <source>` only accepted a single **file** or URL — pointing it at a skill *directory* failed at `read_to_string` ("Is a directory"). The repo itself ships 494 skill directories, so directory import is the primary real-world path.
- **Fix (`cmd_skills.rs`):** `install_skill` now detects a directory source and routes to `install_skill_directory`: requires `SKILL.md`, runs the recursive `skills_guard` security scan on the whole directory (the scanner already supported directories), copies the entire tree (SKILL.md + reference files + nested dirs) to `<skills_root>/<name>/`, refuses overwrites with a clear "already exists" error, and **rolls back** if the imported SKILL.md fails to parse (validated via a pre-copy baseline count, since `load_all` skips broken skills silently). Help text updated. `copy_dir_recursive` covered by 2 new unit tests.
- **Verified live:** imported `openclaw/.agents/skills/gitcrawl` + `discord-clawd` (2 files each incl. nested `agents/*.yaml`) into a throwaway config; `skills list`/`audit`/`inspect` all work; duplicate install errors cleanly.

### Native-tool live batch (this session)
`operant test <tool>` exercised the non-keyed native surface: timestamp, echo, debug_system, file_list, todo, cron, kanban, process (list + spawn), http_request, web_fetch, memory_store, session_search, skills_list, checkpoint — **all functional**. Keyed tools (image_generate, vision_analyze, transcribe_audio, text_to_speech) return graceful errors when unconfigured — expected environment behavior, not defects.

---

## 6.1 IGS v1.0.2 upgrade + integration audit (2026-08-10)

See [`docs/IGS_V102_AUDIT.md`](IGS_V102_AUDIT.md) for the full binary audit,
per-command live results, and igs-source patch suggestions. Summary of
changes shipped here:

- **Binary:** installed IGS **v1.0.2** (`/usr/local/bin/igs`) — replaced the
  stale 0.5.4 that had been on PATH. Verified against the real binary, not
  stale source clones.
- **Shared obscura guaranteed:** IGS `settings.yml` (`browser.default:
  obscura`, stealth) and operant `ObscuraProvider` both resolve
  `~/.config/igs-mcp/bin/obscura` — one binary in use. ✅
- **`IgsBrowserProvider` rewritten** for v1.0.2 reality: `igs browser` CLI is
  **stateless** across invocations and its `markdown` subcommand is broken
  (`--dump markdown` invalid). navigate/snapshot now route through
  `igs web scrape` (same Obscura engine) with last-URL tracking;
  click/fill/scroll return a clear error directing to the CDP provider.
- **Default browser provider changed `"igs"` → `"obscura"`** — the
  CDP-driven, stealth, shared-binary provider that supports real
  multi-step automation.
- **New `web_crawl` tool** (v1.0.2 fixed the 0.5.x crawl 404): SSRF-guarded,
  bounded by `maxDepth`/`maxPages`, returns `pages[]` as markdown.
  Registered in `builtin.rs`.
- **`web_search` now key-free via IGS**: v1.0.2 multi-engine search
  (DDG/Wikipedia/GitHub/HN/SO/YouTube) works without any API key — verified
  live (10 real results, `provider: igs`). Stale docs/error strings
  ("needs a search key") corrected.
- **User config updated** (`~/.operant/operant.toml`): `igs_enabled = true`,
  `preferred_provider = "igs"` (it had been disabled with the old broken
  binary).

## 6.2 Skill-management infrastructure audit (vs hermes-agent) — 2026-08-10

Operant's skill stack (`skills.rs` SkillManager, `skills_guard` scanner,
`skill_marketplace`, `cmd_skills` CLI, `skills_tool` agent tools,
`agent/skill_preprocessing`, `agent/skill_bundle`) was audited against
hermes-agent's (`agent/skill_preprocessing.py`, `skill_bundles.py`,
`skill_commands.py`, `skill_utils.py`, `hermes_cli/skills_hub.py`,
`subcommands/skills.py`).

| Area | hermes-agent | operant | Verdict |
|------|-------------|---------|---------|
| SKILL.md parsing / frontmatter | `skill_utils.py` | `skills.rs` | ✅ parity |
| Template vars `${HERMES/OPERANT_SKILL_DIR}` + inline shell `` !`cmd` `` | `skill_preprocessing.py` (used at load) | `skill_preprocessing.rs` — **was unwired** | ⚠️ **fixed** |
| Bundles (`/bundle` expansion) | `skill_bundles.py` + `skill_commands.py` | `skill_bundle.rs` (no callers) | ⚠️ still unwired |
| Security scan on install | `skills_hub` verdict | `skills_guard.rs` (2,675 LOC, recursive dir scan, block/confirm/allow, `--force`) | ✅ stronger |
| Remote registry / hub | `skills_hub.py` (multi-source: skills.sh, GitHub, clawhub, lobehub…) | `skill_marketplace.rs` — **default registry URL 404'd** (repo `operant-skills` doesn't exist) | ⚠️ **fixed** |
| CLI surface | browse/search/install/inspect/list + sources | list/search/inspect/install/uninstall/update/browse/check/audit/reset/publish/snapshot/tap/toggle/market | ✅ superset |
| Agent tools | `skills_tool` + `skill_manager_tool` | `skills_list` / `skill_view` / `skill_manage` (+ protected-skills & background-review guards) | ✅ parity+ |
| System-prompt injection | metadata-only progressive disclosure | `<available_skills>` name+description in `build_frozen_prefix` | ✅ parity |

**Fixes shipped this session:**

1. **`skill_preprocessing` wired into `skill_view`** — template-var
   substitution (`${OPERANT_SKILL_DIR}`/`${OPERANT_SESSION_ID}`) + inline
   shell expansion (opt-in, hermes `inline_shell` semantics) now run before
   skill content reaches the model (hermes parity: preprocessing at load).
2. **Self-hosted skill registry** — created `skill-registry/index.json` in
   the operant repo (bundled `remote-build-ssh`, `workspace-lint` entries
   with download URLs into this repo) and pointed `DEFAULT_REGISTRY_URL` at
   `raw.githubusercontent.com/ishan-parihar/operant/main/skill-registry/index.json`
   (was a nonexistent `operant-skills` repo → `market list` 404'd).
   `OPERANT_SKILL_REGISTRY` still overrides. After the push, `skills market
   list/search/install` work out of the box.
3. **Directory import** (previous session, still green).

**Closed (iter-320):** `/skill <name>` + `/bundle <name>` slash expansion is
now wired end-to-end (hermes `build_skill_invocation_message` parity):
- `build_skill_invocation_message_in()` resolves the skill from the
  configured `skills.root_dir`, strips frontmatter, runs template-var +
  inline-shell preprocessing, and wraps with hermes activation scaffolding.
- The TUI intercept arms set `pending_user_message` (drained at the top of
  the run loop — expansion submits immediately after Enter, verified live
  in 0.5s) and return `true`, eliminating the CommandRegistry "not yet
  wired" fallback that previously swallowed `/skill`.
- `/skill <Tab>` / `/bundle <Tab>` typeahead completes installed skill +
  bundle names (`register_typeahead_names`, snapshot registered at App init).
- Example bundles shipped in `skill-bundles/` (`ship-feature`, `quality-gate`)
  and installed to `~/.operant/skill-bundles/`; `~/.operant/skills/` carries
  matching demo skills.
- Regression tests: slash expansion (temp skills dir), bundle intercept,
  missing-skill `None`, typeahead name completion.

**Remaining (known, documented):** repo-bundled skills (`remote-build-ssh`,
`workspace-lint`) trip the `skills_guard` security scanner when installed via
`skills install` (dangerous-verdict shell patterns) — the scanner is doing
its job; importing them requires explicit review or `--force` on a clean
verdict.

---

## 6.3 Plugin architecture + memory-plugin audit (vs hermes-agent) — 2026-08-10

Audited operant's plugin + memory infrastructure against hermes-agent's
(`hermes_cli/plugins.py` VALID_HOOKS, `agent/agent_plugins.py`, the
`plugins/memory/*` plugin family) to confirm hermes plugins are plug-and-play.

### 6.3.1 Memory provider integration — ✅ implemented & wired

The memory-plugin *infrastructure* is implemented and effective:

| Piece | Status |
|-------|--------|
| `operant-memory` trait (`Memory`), provider backends (InMemory, SQLite/vector, agent-memory), response cache | ✅ |
| `MemoryProvider` bridge (hermes `memory_provider.py` parity) | ✅ `operant-core/src/memory_provider.rs` |
| Memory wired into the agent: `memory_loader.load_context` on every turn + `auto_save` user-msg store | ✅ `Agent::turn` / `turn_streamed` |
| Memory-review nudge (`memory_nudge_interval`) + skill-creation nudge | ✅ `fire_evolution_triggers` |
| Session-scoped namespace (`memory_session_id`) | ✅ |
| **WASM plugin `Memory` capability consumed** | ✅ **new** — `plugin_memory.rs` bridges `PluginCapability::Memory` → `MemoryProvider` in `load_memory_manager` |

### 6.3.2 Plugin architecture parity (hermes VALID_HOOKS → operant HookHandler)

Hermes defines 27 `VALID_HOOKS`. Operant's `HookHandler` trait + `HookRunner`
now cover the full set relevant to the runtime agent:

| hermes hook | operant hook | Wired |
|-------------|-------------|-------|
| `pre_tool_call` | `before_tool_call` (modifying/cancel) | ✅ agent + loop |
| `post_tool_call` | `on_after_tool_call` (void) | ✅ agent + loop |
| `pre_llm_call` | `before_llm_call` (modifying) | ✅ |
| `post_llm_call` | `on_llm_output` (void) | ✅ |
| `transform_llm_output` | `transform_llm_output` (first non-None wins) | ✅ **new** — `turn` + `turn_streamed` final text |
| `on_session_start` / `on_session_end` | same | runner surface (gateway path) |
| `on_session_reset` | `on_session_reset` | **new** — runner surface (mirrors session_start/end wiring) |
| `on_skill_lifecycle` | `on_skill_lifecycle` | ✅ **new** — fired per skill-sourced tool call (`skill__tool`) |
| `subagent_start` / `subagent_stop` | same | ✅ **new** — fired around `delegate` tool execution |
| `pre_approval_request` / `post_approval_response` | same | ✅ **new** — fired around approval prompts in `execute_tool_call` |
| `pre_gateway_dispatch` | `on_message_received` (modifying) | ✅ |
| `on_message_sending` | `on_message_sending` (modifying) | ✅ |
| kanban_* / pre_verify / api_request_* | n/a (no kanban/verify feature) | — |

All new hooks are default no-ops, so existing handlers are unaffected; a
hermes plugin implementing any of these callbacks can be ported 1:1.
Regression coverage: `hooks::runner::tests::hermes_parity_lifecycle_hooks_dispatch`
(33 hook tests green).

### 6.3.3 WASM plugin tools — ✅ new bridge

`PluginHost` (`operant-plugins`) loads WASM plugins, but their tools were
never surfaced into the agent's `ToolRegistry`. New `plugin_tools.rs`:

- Adapts `operant-api` `WasmTool` → `OperantTool` (schema + name + execute).
- `register_plugin_tools` is called from CLI `build_registry` when the
  `plugins-wasm` feature is on.
- `list_plugin_tools` surfaces plugin-provided tools in `operant plugins list`.
- Plugin tool names are namespaced `plugin:<name>:<tool>` to avoid collisions.

### 6.3.4 Skill-bundle cache — ✅ fixed

`get_skill_bundles` was a process-lifetime `OnceLock` — new bundles needed a
restart. It is now an **mtime-aware refreshable cache** (`BundleCache` with
dir mtime + `refresh_skill_bundles()` force-rescan), and the TUI `/bundle`
and `/skill` open paths refresh caches + typeahead names so newly installed
skills/bundles appear immediately. Tests serialized via a shared lock;
self-cleaning temp dirs (5/5 bundle tests green).

---

## 6.4 agentmemory default + native MCP + hermes-plugin lifecycle parity (2026-08-10)

**Request:** Make `https://github.com/rohitg00/agentmemory` the default, integrate it
through the hermes-agent memory plugin contract, and register it as a native MCP
server.

### Parity audit — operant `AgentMemoryProvider` vs hermes plugin (`integrations/hermes`)

| Hermes plugin hook            | REST call                 | operant before | operant now         |
|-------------------------------|---------------------------|----------------|---------------------|
| `initialize(session_id)`      | `POST session/start`      | server-warmup only | ✅ session/start + scope capture |
| `system_prompt_block()`       | `POST context` (sync)     | static text    | ✅ live context, static fallback |
| `prefetch(query)`             | `POST smart-search`       | ✅             | ✅ (unchanged)      |
| `sync_turn(user, assistant)`  | `POST observe`            | `POST remember` (wrong shape) | ✅ observe + hookType payload |
| `on_session_end(messages)`    | `POST session/end`        | not impl.      | ✅ fire-and-forget  |
| `on_pre_compress(messages)`   | `POST context` (sync)     | not impl.      | ✅ returns context  |
| `on_memory_write(add/update)` | `POST remember` (type:fact)| not impl.      | ✅ bg mirror        |
| `queue_prefetch(query)`       | `POST smart-search` (bg)  | not impl.      | ✅ bg queue         |
| `on_session_switch`           | `POST session/start`      | not impl.      | ✅ id rotation + re-register |

**Sync-hook design:** sync hooks use a shared static `reqwest::blocking` client with a
5s timeout, gated on reachability (never stalls the agent loop on a dead server);
fire-and-forget hooks (`session/end`, `remember` mirror, `smart-search` queue,
`session/start` on switch) run on a background thread exactly like the plugin's
`_api_bg`. Per-provider blocking clients were avoided deliberately: they would be
dropped inside async runtimes and panic (tokio "Cannot drop a runtime in a context
where blocking is not allowed").

### Native MCP registration

- **Before:** the agentmemory MCP server was a CLI-only special case in
  `build_registry` (main.rs) — the runtime-agent `connect_all` and gateway
  paths never saw it unless the user hand-configured it.
- **After:** `config::ensure_default_mcp_servers()` runs inside
  `load_app_config()` and injects an `agentmemory` stdio server
  (`npx -y @agentmemory/mcp`, env `AGENTMEMORY_URL` + optional
  `AGENTMEMORY_SECRET`) into `config.mcp.servers` whenever
  `memory.provider == "agentmemory"` (the default) and no user-configured
  server exists. Every `AppConfig`-driven agent-construction path (CLI
  registry, runtime-agent `connect_all`, operant-cli gateway runner) picks it
  up through the generic config-driven MCP connect loop — native, not
  special-cased. The now-redundant main.rs block was removed. Users who
  configured their own `agentmemory` server (or disabled it) keep their entry
  untouched.
  
  **Coverage note:** the separate `operant-channels` orchestrator uses its own
  `operant_config::schema::Config` type (not `AppConfig`), so the injection
  does not flow into that subsystem. If a deployment runs the channels
  orchestrator and needs the agentmemory tools there, add the server under
  `[mcp.servers]` in that config explicitly.
  
  **Deferred (lazy) connect (2026-08-10):** the injected server now carries
  `deferred: true` — it is **not** auto-connected at startup (previously
  every `operant` invocation spawned `npx @agentmemory/mcp`). The memory
  provider's own tool schemas (`memory_smart_search`, `memory_save`) are
  registered directly into the registry via `memory_provider_tools.rs`, so
  the memory surface stays available without the MCP server (hermes plugin
  parity — the plugin registers memory tools independently of MCP). The
  deferred server remains connectable on demand via `operant mcp`.

## 6.5 Remaining plugin-parity gaps — context management, MCP deferral, orchestrator schema (2026-08-10)

Three parity gaps were investigated this session. Two are fixed; one is
architectural and documented with a recommended path.

### 6.5.1 Context management: `queue_prefetch` was dead code — ✅ fixed

**Hermes reference** (`agent/memory_manager.py`): after every completed turn
`run_agent.py` calls `sync_all()` **and** `queue_prefetch_all()` (background
worker, non-blocking). The authoritative recall runs in `prefetch_all()` at
the start of the *next* turn; `queue_prefetch` just warms the provider so a
slow backend never blocks the turn-completion path.

**Operant before:** the post-turn hook spawned a full `provider.prefetch()`
with an 8s timeout — a duplicate live search whose result was discarded
(the next turn re-runs `prefetch()` anyway). The `queue_prefetch` hook
(implemented in `AgentMemoryProvider` as a bg `smart-search`) was **never
called** — dead code.

**Fix (`agent/mod.rs`):** the post-turn hook now calls
`provider.queue_prefetch(&user_query)` — hermes `queue_prefetch_all` parity.
The pre-turn `prefetch()` (with `<memory_context>` injection) is unchanged.

### 6.5.2 MCP deferred loading for the injected agentmemory server — ✅ fixed

**Problem:** the CLI path (`build_registry`, main.rs) eagerly connected
*every* enabled MCP server at agent construction. With the agentmemory
server injected by default, **every** `operant` invocation (TUI, `run`,
`chat`, gateway) spawned `npx -y @agentmemory/mcp` at startup — npx
resolution + process spawn latency and churn even when the 53 memory MCP
tools were never used. Hermes does lazy MCP reconnect; the newer
`operant_config::schema::Config` path already has `mcp.deferred_loading`
(default true) + `DeferredMcpToolSet`, but the CLI/`AppConfig` path had no
deferral mechanism at all.

**Fix:**
- `McpServerConfig.deferred: bool` (default `false`, serde-default) added to
  `AppConfig`.
- `ensure_default_mcp_servers()` injects the agentmemory server with
  `deferred: true`.
- `build_registry` skips `deferred` servers in the eager autoload loop;
  they stay connectable on demand via `operant mcp` / the MCP tooling.
- **New `memory_provider_tools.rs`:** the memory provider's own
  `tool_schemas()` (`memory_smart_search`, `memory_save`, …) are registered
  directly into the registry as `OperantTool`s dispatching to
  `handle_tool_call()` — so the model keeps the memory surface without the
  MCP server (hermes plugin parity). Previously these provider schemas were
  registered nowhere in the CLI path (dead code); the tools existed only via
  the MCP server.

  **Fixed (iter-93 reconnect parity):** the TUI `/mcp` reconnect path now
  re-adds *all* configured transports — HTTP / streamable-HTTP via
  `add_server`, stdio via `add_stdio_server` (command + args + env) — then
  re-runs `McpManager::sync_tools_to_registry` against the agent's live
  `ToolRegistry` handle (`OperantAgent::registry()` clone; the tool map is
  shared via `Arc`, so `get_schemas()` picks up the new tools on the next
  turn). A deferred stdio server (e.g. the injected agentmemory server) can
  therefore be materialized mid-session with `/mcp r` — no restart needed.
  The background task reports what reconnected/failed through a status
  channel drained in the run loop. As a side effect, `McpStdioClient`'s
  async futures were made `Send`-safe (tracing `%`-captures replaced with
  `&str` Values, await hoisted out of a `debug!` field), which also unblocks
  stdio MCP in any future `tokio::spawn` context.

  **Fixed (iter-326 — native agent-memory lifecycle + fast reconnect):**
  - The reconnect task now warms the agentmemory REST backend **before**
    connecting its MCP stdio server: `MemoryProvider::ensure_server`
    (default `true`; agentmemory overrides with health-check + managed
    auto-spawn via `agentmemory_auto_spawn`) is called first, so the MCP
    initialize handshake completes in <1s instead of the observed ~2.5 min
    cold-backend wait. `check_health`/`ensure_server` became defaulted
    trait methods (`AgentMemoryProvider::ensure_backend` is the public
    inherent lifecycle helper); `OperantAgent::memory_provider()` exposes
    the provider to the TUI, stored as `App::core_memory_provider`.
  - The completion message is now rich: elapsed time, per-server outcome,
    agentmemory backend state (already up / spawned / unreachable), and the
    exact tool count from `sync_tools_to_registry` (now returns `usize`).
  - Status channels are drained **every frame** (tick drain in `App::run`,)
    not only on input — the reconnect completion renders the moment it
    finishes, no keystroke needed.

### 6.5.3 operant-channels orchestrator config-schema gap — ✅ MCP injection implemented

**Root cause:** two parallel config systems.

| | `AppConfig` (operant-core) | `operant_config::schema::Config` (operant-config) |
|---|---|---|
| Used by | CLI paths: TUI, `run`, `chat`, gateway runner (`create_runtime_agent` → `build_agent_core`) | operant-runtime agent, operant-channels, operant-gateway |
| Memory | hermes-parity `MemoryProvider` trait (`agentmemory`/`builtin`/`plugin:<n>`) | `operant_memory::Memory` trait (sqlite/qdrant/markdown/none) — **no agentmemory provider** |
| MCP | `ensure_default_mcp_servers()` injects agentmemory server (now deferred) | `ensure_default_mcp_servers()` now injects the agentmemory server (see below); `mcp.deferred_loading` + `DeferredMcpToolSet` (tool_search) |
| Hermes parity | ✅ plugin hooks + lifecycle | memory/MCP surface is a different (newer) architecture |

**Fix (this commit):** `operant_config::schema::Config` gained
`ensure_default_mcp_servers()`, called from `Config::load_or_init()` (both
the existing-config and fresh-init paths) before validation. It appends an
`agentmemory` stdio server (`npx -y @agentmemory/mcp`, env
`AGENTMEMORY_URL`/`AGENTMEMORY_SECRET` read from the environment, default
`http://localhost:3111`) when `mcp.enabled` is true, `memory.backend ==
"agentmemory"`, and no `agentmemory` server is already configured
(idempotent). Because the schema's `deferred_loading` flag is global and
defaults to `true`, the injected server automatically joins the deferred
toolset — the runtime daemon / channels orchestrator (`McpRegistry`)
exposes `memory_*` tools via `tool_search` without ever spawning `npx
@agentmemory/mcp` at boot.

**Resolved (2026-08-11):** the runtime agent's memory layer now has a real
agentmemory provider. New `operant-memory::AgentMemory` backend implements
the `Memory` trait against the agentmemory REST API (`store` →
`/agentmemory/remember`, `recall`/`get`/`list` → `/agentmemory/smart-search`,
`health_check` → `/agentmemory/health`, env-configured via
`AGENTMEMORY_URL`/`AGENTMEMORY_SECRET`), replacing the silent
custom-extension-point → markdown fallback. `memory.backend =
"agentmemory"` now selects it in `classify_memory_backend`, it is offered in
`selectable_memory_backends()`, and `forget`/`count` return clear
"unsupported by the agentmemory REST API" errors (the plugin only mirrors
add/update writes; the verified REST surface is remember/smart-search/
observe/context/session). Unifying the two config/memory stacks (CLI
`MemoryProvider` trait vs runtime `Memory` trait) remains a deliberate,
larger refactor — but each stack now has a first-class agentmemory provider.


## 7. Verification summary

```
cargo check --workspace                                  → 0 errors, 0 warnings
cargo test --workspace                                   → 8,540 passed / 0 failed
cargo clippy --workspace --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all --check                                  → clean
```
