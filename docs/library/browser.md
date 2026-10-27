# Browser — the sourcehound engine

Owns: operant's browser automation mechanism — the sourcehound engine,
the cloakctl verb surface, raw CDP, cookie import, and the
accessibility tree.
Not for: the web search/scrape tools ([web-tools.md](web-tools.md)),
MCP generally ([mcp.md](mcp.md)).

Source: `crates/operant-core/src/browser_provider.rs`,
`crates/operant-core/src/sourcehound_cdp.rs`,
`crates/operant-core/src/cookies.rs`,
`crates/operant-core/src/accessibility.rs`,
`crates/operant-core/src/tools/browser_tool.rs`,
`browser_cdp_tool.rs`, `browser_dialog_tool.rs`,
`crates/operant-cli/src/cmd_cookies.rs`.

## One engine, by design

`browser_provider.rs` (plan 11C2): **sourcehound is the only engine**.
`config.browser.provider` is accepted for config compatibility but
ignored — every value lands on the sourcehound browser (over MCP,
CDP-capable). The camofox / browserbase / browser-use / firecrawl
backends are deleted; old provider names are accepted-with-warning and
ignored, exactly like the memory provider names. This is an operator
design choice — do not switch the default away from sourcehound.

## The two paths into the engine

1. **`browser` tool** (`tools/browser_tool.rs`) — the agent-facing
   surface: navigate, snapshot, and interact with page elements through
   sourcehound's `cloakctl.*` MCP verbs (`cloakctl.navigate`,
   `cloakctl.read`, `cloakctl.act`, raw CDP via the `cloakctl.cdp`
   endpoint). Driven over the sourcehound MCP server (stdio), spawned
   from the `sourcehound` binary on PATH (`SOURCEHOUND_BINARY`
   override). A missing binary degrades to a helpful error naming the
   fix — never a dead tool surface.
2. **`sourcehound_cdp.rs`** — the raw-Chrome-DevTools-Protocol path for
   tools and harnesses that speak CDP themselves: the `browser_cdp`
   tool and the cookie commands attach directly to the same engine over
   the managed shared session (auto-start/attach via `cloakctl.cdp` on
   the persistent socket, with page session ids — no `BROWSER_CDP_URL`
   required, iter-1246).

`browser_dialog` handles page dialogs (alerts/confirms) on the same
session.

## Cookie import (`cookies.rs` + `cmd_cookies.rs`)

Multi-browser cookie support so accounts from Chrome / Brave / Edge /
Chromium / Firefox are usable in the engine session without manual
login: Netscape `cookies.txt` (the universal export format),
EditThisCookie / Cookie-Editor JSON, and live-browser imports.
`operant cookies import` is the operator surface.

## The accessibility tree (`accessibility.rs`)

Structured a11y snapshots from CDP's `Accessibility.getFullAXTree`, with
text rendering and ref-based element selectors (`@e1`, `@e2`, …)
matching the Python operant-agent's `ariaSnapshot` output — so the
model can address elements semantically instead of by coordinate.

## Intention

One engine instead of a provider zoo: the sourcehound binary already
speaks MCP, drives a CDP-capable browser, and self-updates
(`binary_updater.rs`, shared with AFT), so a second browser backend
would duplicate the entire seam. The split between the cloakctl verbs
(agent-facing, high-level) and raw CDP (harness-facing, low-level) keeps
the agent's surface stable while the debug/introspection tools talk the
native protocol.
