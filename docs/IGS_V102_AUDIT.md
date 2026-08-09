# IGS Integration Audit — Binary v1.0.2 (2026-08-10)

Audit of the **operant ↔ IGS** integration against the **actual installed binary
(`/usr/local/bin/igs`, v1.0.2)** — not the stale 0.5.x or older source clones.

## Binary provenance

| Item | Value |
|------|-------|
| Binary | `/usr/local/bin/igs` (installed from official release) |
| Version | `igs 1.0.2` (was stale `0.5.4` before this audit) |
| Release | `ishan-parihar/igs-rust` `v1.0.2` |
| Asset | `igs-1.0.2-x86_64-unknown-linux-musl.tar.gz` |
| Config dir | `~/.config/igs-mcp/` (`settings.yml`, `pools.yml`, `sources.yml`) |
| Browser | `browser.default: obscura`, stealth: true — the **same** binary operant's `ObscuraProvider` resolves (`~/.config/igs-mcp/bin/obscura`) |

**Shared-binary guarantee (verified):** IGS's `browser.default: obscura` and
operant's `ObscuraProvider::resolve_obscura_binary` both resolve
`~/.config/igs-mcp/bin/obscura` (v0.1.11, stealth). There is exactly one
obscura binary in use. ✅

## Live-tested capability surface (v1.0.2)

| Command | Result | Notes |
|---------|--------|-------|
| `igs web search --query Q` | ✅ **works, key-free** | Multi-engine: DDG, Wikipedia, GitHub, HN, StackOverflow, YouTube. JSON: `results[]` with `title/url/content/source/domain/score`. |
| `igs web scrape --url U` | ✅ works | JSON: `markdown` (clean), `metadata`, `meta`. |
| `igs web crawl --url U` | ✅ **fixed in v1.0.2** | 0.5.x 404'd ("Obscura fetch failed: Download returned status 404"); v1.0.2 crawls via obscura. JSON: `pages[]` with `url/title/content/depth/status`. |
| `igs web extract --url U` | ✅ **new in v1.0.2** | JSON: `content` + `markdown` + rich `metadata` (word_count, reading_time, language). |
| `igs web image-search` | ✅ new | Wikimedia Commons, key-free. |
| `igs web screenshot` | ✅ new | Obscura CDP headless. |
| `igs browser goto --url U` | ✅ works | JSON: `content` (raw HTML). |
| `igs browser markdown` | ❌ **broken in v1.0.2 too** | Internally passes `--dump markdown`, but `--dump` only accepts `html\|text\|links`. Returns `invalid value 'markdown'`. |
| `igs browser evaluate/links/click/fill` | ❌ **stateless CLI** | Every `igs browser <cmd>` spawns a fresh `about:blank` session — session state does NOT persist across invocations ("URI scheme is not allowed (about:blank)"). |

## Defects found + patch suggestions for igs source (`ishan-parihar/igs-rust`)

### D1 — `browser markdown` passes an invalid `--dump` value (critical for CLI users)

**Observed:** `igs browser markdown --format json` → `error: invalid value
'markdown' for '--dump <DUMP>' [possible values: html, text, links]`.

**Root cause:** the `markdown` subcommand forwards `markdown` to a `--dump`
flag whose enum only allows `html|text|links`.

**Suggested patch (igs source, `crates/igs-cli/src/browser.rs` or equivalent):**

```rust
// Before (conceptually):
BrowserDumpFormat::from_str("markdown")  // → error

// After: the `markdown` subcommand should dump with an HTML→markdown
// conversion pass instead of passing "markdown" as a DumpFormat.
// Either (a) extend DumpFormat with a `Markdown` variant that internally
// converts the HTML dump to markdown, or (b) have the markdown subcommand
// call the html dump path and run `html_to_markdown` on the result.
```

### D2 — `igs browser` CLI is stateless across invocations (blocks multi-step automation)

**Observed:** `igs browser goto --url X` succeeds, then a separate
`igs browser evaluate --expression 'document.title'` starts at `about:blank`
and fails. The CLI help claims "persistent session" but each invocation
spawns a fresh Obscura CDP context.

**Suggested patch:** either
- (a) run a server-side obscura session (daemon) that CLI commands attach to
  (e.g. `igs browser start` → returns a session id; `igs browser --session <id> …`
  reuses it), or
- (b) remove the "persistent session" claim from `browser --help` and document
  that `browser` subcommands are single-shot, or
- (c) auto-chain: `igs browser goto X; evaluate Y` in one invocation.

**Operant-side resolution (already applied):** `IgsBrowserProvider` no longer
depends on `browser goto`+`markdown`. `navigate`/`snapshot` route through
`igs web scrape` (same Obscura engine, reliable, returns markdown), tracking
the last URL for snapshots. click/fill/scroll return a clear error directing
users to `browser.provider = "obscura"` (CDP-driven, multi-step capable).

### D3 — `igs web crawl` auto-update 404 (fixed in v1.0.2, keep regression test)

**Observed (0.5.x):** `igs web crawl` → `Obscura fetch failed: Download
returned status 404 Not Found` — the auto-update tried to download a stale
obscura URL.

**Status:** verified fixed in v1.0.2. Suggested source hardening: when the
auto-update download fails, **fall back to the existing local binary**
(`~/.config/igs-mcp/bin/obscura`) instead of erroring out, and log a warning
("auto-update failed, using local binary vX").

### D4 — `web search` is now key-free (stale docs only)

v1.0.2 `web search` runs multi-engine without any API key. Older operant docs
and error strings claimed IGS search needs a Tavily/Firecrawl key — those
have been corrected (see `web_providers/igs.rs`, `web_tools.rs`, `tools/igs.rs`).

## Operant changes shipped in this audit

1. **IGS v1.0.2 installed** at `/usr/local/bin/igs` (replaced stale 0.5.4).
2. **`IgsBrowserProvider`** rewritten for v1.0.2 reality: navigate/snapshot via
   `igs web scrape` with last-URL tracking; click/fill/scroll → clear error
   directing to the `obscura` CDP provider.
3. **Default browser provider** changed `"igs"` → `"obscura"` (CDP-driven,
   shared stealth binary, live-tested multi-step capable).
4. **New `web_crawl` tool** added (v1.0.2 crawl is fixed; SSRF-guarded, bounded
   by maxDepth/maxPages), registered in `builtin.rs`.
5. **User config** `~/.operant/operant.toml`: `igs_enabled = true`,
   `preferred_provider = "igs"` (was disabled + duckduckgo — the old binary
   was why it had been turned off).
6. **Stale docs/error strings** corrected (IGS search is key-free ≥1.0).
7. **Self-hosted skill registry** (`skill-registry/index.json`) — see the
   skill-management audit below; `skills market list` now points at the
   operant repo itself instead of a nonexistent `operant-skills` repo.

## Live verification summary

```
operant test web_search  → success: true (10 results, provider: igs)
operant test web_scrape  → success: true (clean markdown)
operant test web_extract → success: true (clean content)
operant test web_crawl   → success: true (pages[] as markdown)
operant test browser navigate → success: true (markdown via web scrape)
```

## Remaining gaps (need igs source patch, not operant)

- `igs browser` multi-step automation (evaluate/click/fill across invocations)
  — blocked by D2 until igs ships a stateful browser session or CDP attach.
  Use `browser.provider = "obscura"` for interactive automation today.
- `igs web search` upstream engines occasionally return 0 for very recent
  queries — operant's fallback chain (igs → tavily → exa → ddg → searxng)
  covers this.
