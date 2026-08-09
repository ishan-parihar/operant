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
- `operant doctor` reports which binary is resolved ("Obscura browser binary (shared with IGS: …)").
- Tests: 3 resolve-order unit tests + config parse/back-compat test.

### Why this is the right layer
igs-rust's `ObscuraManager` has no binary-path knob — only `IGS_CONFIG_DIR` (which moves the whole config dir, not just the binary). So operant adapts to IGS's location rather than vice-versa. With the **default config** (`browser.provider = "igs"`, `web.preferred_provider = "igs"`), everything already routes through the igs binary's Obscura; the fix extends that guarantee to `browser.provider = "obscura"`.

**Result: one Obscura binary, one download, no drift — the user's stated requirement.**

---

## 4. Integration-by-integration status

| Integration | Path | Status |
|---|---|---|
| `web_search` (IGS → DDG fallback) | `tools/web_tools.rs` + `tools/igs.rs` + `tools/web_providers/` | ✓ Structured JSON parse is defensive (`results`/`memories`/`data` shapes); falls back to DDG on empty |
| `web_scrape` / `web_extract` (IGS, JS rendering via Obscura) | `tools/igs.rs` | ✓ SSRF-guarded, empty-URL rejected, graceful "install igs" error when binary missing |
| `web_fetch` (raw HTTP) | `tools/web_tools.rs` | ✓ SSRF-guarded, redirects disabled, scheme-restricted |
| `web.crawl` (via IGS) | exposed through `igs` CLI (`igs web crawl`) | ✓ Available when igs installed; SSRF checked by `web_scrape` path parity |
| `browser` tool | `tools/browser_tool.rs` → provider factory | ✓ All commands validated; SSRF on navigate/snapshot; scroll/type validation tested |
| `browser.provider = "obscura"` | `browser_provider.rs` | ✓ **Now shares the IGS binary** (this audit) |
| `browser.provider = "igs"` (default) | `tools/igs.rs` `IgsBrowserProvider` | ✓ Persists session across goto → markdown → click sequences |
| `operant doctor` | `cmd_doctor/checks_tools.rs` | ✓ Reports igs availability per toolset + resolved Obscura binary |

---

## 5. Security posture

- **SSRF:** fail-closed `ssrf_verdict` on every URL-fetching path (browser navigate/snapshot, web_fetch, web_scrape, web_extract). Blocked: cloud metadata (169.254.169.254, `metadata.google.internal`), loopback, RFC 1918, CGNAT, DNS-fail-closed. Dedicated tests for each tool.
- **Secrets:** API keys live in `.env` (never TOML) per repo rule; `web_fetch` output returns raw HTTP so no secret plumbing.
- **Download hygiene:** both Obscura downloaders cap sizes / verify `--version` / chmod 0755; igs-rust additionally validates tar entries against path traversal.

---

## 6. Remaining gaps / recommendations (no code change yet)

1. **Manual smoke test (user's own):** the binary-sharing fix is covered by unit tests, but a live end-to-end run is recommended:
   - `operant doctor` → confirm "Obscura browser binary (shared with IGS: ~/.config/igs-mcp/bin/obscura)".
   - `operant run --query "search the web for X"`, then a `web_scrape`, then `browser` navigate → confirm all three work and only one `obscura` binary exists on disk (`ls ~/.config/igs-mcp/bin ~/.operant/bin`).
2. **igs-rust feature (upstream):** add a `binaryPath`/`OBSCURA_BIN` override to `ObscuraManager` so the sharing is bidirectional and configurable on the IGS side too. Out of operant's control; noted for the igs-rust repo.
3. **`operant-tools` crate:** a second `WebSearchTool` exists at `crates/operant-tools/src/web_search_tool.rs` (plus an `operant-browser`-style naming in docs). Confirm whether `operant-tools` is wired into the runtime tool registry or is dead weight for the next dead-code pass.
4. **Windows Obscura asset matching** only handles x86_64 (no aarch64-windows entry) — fine for now, note for cross-compile targets.
5. **`web_extract` auxiliary model** slot exists in config but the IGS path returns raw markdown (no LLM post-processing) — intended; document that `auxiliary_models.web_extract` only applies when an LLM-backed extractor is wired.

---

## 7. Verification summary

```
cargo check --workspace                                  → 0 errors, 0 warnings
cargo test --workspace                                   → 8,518 passed / 0 failed
cargo clippy --workspace --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all --check                                  → clean
```
