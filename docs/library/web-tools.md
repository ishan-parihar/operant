# Web tools — sourcehound intelligence

Owns: the web intelligence tools — search, scrape, extract, crawl — and
the sourcehound integration's self-update mechanism.
Not for: the browser engine ([browser.md](browser.md)), MCP transport
([mcp.md](mcp.md)).

Source: `crates/operant-core/src/tools/sourcehound.rs`,
`crates/operant-core/src/tools/sourcehound_update.rs`,
`crates/operant-core/src/binary_updater.rs`,
`crates/operant-core/src/tools/http_tool.rs` (`http_request`),
`[tools.web]` config.

## The four tools

Served by the sourcehound bridge over its MCP server (stdio), no API
keys required:

- **`web_search`** — search the web.
- **`web_scrape`** — pull page content.
- **`web_extract`** — extract structured data from a page.
- **`web_crawl`** — breadth-first site crawl.

All degrade to a helpful error naming the fix when the `sourcehound`
binary is missing (PATH or `SOURCEHOUND_BINARY`).

## `http_request`

The general-purpose HTTP tool: full control over method, headers, body.
It complements the sourcehound family for API calls that are not
web-page-shaped.

## The integration mechanism

`tools/sourcehound.rs` builds the `IntegrationSpec`; the
download/verify/replace/prune/throttle machinery lives in the shared
`binary_updater.rs` (also used by AFT). `tools/sourcehound_update.rs`
kicks a once-per-session background update check from the first
sourcehound tool call — the update is self-maintaining without a
startup cost.

## Intention

Web intelligence rides a subprocess speaking MCP rather than a cargo
dependency: the `sourcehound_mcp` git dependency is deliberately HELD
OUT of `operant-core/Cargo.toml` (the vendored obscura-render /
taffy / cosmic-text breakage is documented in that manifest). The
bridge keeps the binary small and the engine independently updatable —
the same seam AFT uses. This is an operator design choice — sourcehound
is the default web/browser engine; do not switch it.
