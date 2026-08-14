# Operant Browser & Web

## Browser (Obscura CDP)

One `browser` tool with commands:

- `navigate` (url), `snapshot`, `click` (selector), `type` (selector, text), `scroll` (text up/down), `accessibility_tree`, `cookies_import` / `cookies_list` / `cookies_clear`

The stealth Obscura binary is shared with IGS web tools (single-binary guarantee). `operant cookies` imports cookies from Chrome/Brave/Edge/Firefox so accounts can be used without manual login. Provisioned by `install-browser-deps.sh`.

## IGS web tools

- `web_search` — keyless search (igs → ddg failover, per-provider timeout)
- `web_crawl` — multi-page crawl
- `web_fetch` — single URL fetch (`url=` param), returns markdown

## Vision

- `vision_analyze` (image_url, question) — inspect rendered pages, PDFs, screenshots

## Terminal fallback

`terminal` tool runs CLI commands — curl, gh, igs, obscura, etc.
