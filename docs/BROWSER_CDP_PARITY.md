# Browser CDP Parity — Verified Matrix

Status of the managed Obscura session (`obscura serve --stealth`, v0.2.0)
against the Chrome DevTools Protocol surface, verified live **through the
agentic loop** (Aug 2026). The agent drives the browser two ways:

- **`browser_cdp`** — raw CDP passthrough over the shared persistent socket.
- **`browser`** — high-level commands (navigate / snapshot / click / type /
  scroll / accessibility_tree / cookies_*) backed by the same session.

## ✅ Verified working (in-loop and/or raw probe)

| Domain / method | Notes |
|---|---|
| `Target.createTarget` / `attachToTarget` (flatten) / `closeTarget` | single-page flow; page auto-provisioned when no session_id given |
| `Page.navigate` / `reload` / `getFrameTree` / `getNavigationHistory` | |
| `Runtime.evaluate` (sync + `awaitPromise` async) | DOM read/write, `document.cookie`, localStorage ops |
| `DOM.getDocument` / `querySelector` / `getOuterHTML` | |
| `Page.captureScreenshot` | real PNG (`\x89PNG\r\n\x1a\n`, base64 `iVBORw0KGgo`) |
| `Page.printToPDF` | real PDF (`%PDF-1.4`, base64 `JVBERi`) |
| `Page.getLayoutMetrics` / `Emulation.set/clearDeviceMetricsOverride` | viewport control |
| `Network.enable` / `getCookies` / `setCookie` / `deleteCookies` / `setCacheDisabled` | cookie + cache control |
| `Storage.setCookies` / `getCookies` / `clearCookies` | import/export/clear (chunked ×400) |
| `Input.dispatchKeyEvent` (`type: char`) | keyboard typing (verified in-loop: 4×"agent" → readback exact) |
| `Input.dispatchMouseEvent` (moved / pressed / released / wheel) | mouse + scroll |
| `Accessibility.getFullAXTree` | a11y tree (2522 nodes on wikipedia) |
| `Browser.getVersion` | |

**In-loop end-to-end flows verified:**
- Form fill + search: DDG navigate → type `#searchbox_input` → click → form
  submit → 10 organic results (agent worked around DDG's no-op click by
  submitting the form).
- Authenticated session: Zen-imported cookies → x.com shows
  `(4) Home / X` + `SideNav_NewTweet_Button` (logged in).
- Screenshot + cookie round-trip through `browser_cdp`.

## ⚠️ Obscura gaps (v0.2.0) + agent workarounds

| Gap | Workaround |
|---|---|
| `Input.insertText` — `-32601 Unknown Input method` | `Input.dispatchKeyEvent {type:"char", text}` (verified), or JS value-set + input/change events |
| `Page.goBack` / `goForward` — unimplemented; JS `history.back()` is also a no-op | navigate explicitly to the target URL |
| `Target.getTargets` returns `[]` | createTarget works; don't rely on the registry |
| localStorage is not persisted (getItem → null) | use cookies (Storage.setCookies) or file output for persistence |
| `Runtime.evaluate` exception info swallowed (`exceptionDetails` absent) | wrap risky JS in try/catch and return JSON |
| **Second `Target.createTarget` on one connection crashes obscura** (core dump) | one page per session; restart the session for a fresh page |
| `Network.getCookies` ignores the `urls` filter (full-store dump) | filter client-side; use `Network.setCookie` for single cookies |

## Installer note

`./scripts/install.sh` now provisions `igs` + `obscura` as global executables
(`scripts/install-browser-deps.sh`) — idempotent, reuses the IGS-managed
obscura so browser + IGS web tools share one binary, falls back to
downloading the stealth build (`h4ckf0r0day/obscura` releases) on fresh
machines. See the script header for env overrides.
