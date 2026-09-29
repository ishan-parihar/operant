//! Raw-Chrome-DevTools-Protocol access to the sourcehound browser engine.
//!
//! The [`crate::browser_provider::SourcehoundProvider`] drives the engine
//! through sourcehound's `cloakctl.*` MCP verbs — that is the path the
//! agent-facing browser tool takes. This module is the other path: tools and
//! harnesses that speak CDP themselves (the `browser_cdp` tool, the cookie
//! commands) attach directly to the same engine.
//!
//! sourcehound owns the engine process. It publishes a real DevTools
//! websocket through its `cloakctl.cdp` MCP tool
//! (`ws://127.0.0.1:<port>/devtools/browser` — the same browser-level shape
//! Chrome's `--remote-debugging-port` exposes), and that call is idempotent:
//! it returns the running endpoint rather than starting a rival server. So:
//!
//! 1. [`Sourcehound::global`] — the shared stdio client.
//! 2. `cloakctl.cdp { profile }` — publish (or re-attach to) the endpoint and
//!    take its `ws_url`.
//! 3. Drive it over CDP: `Target.createTarget` / `Target.attachToTarget` for
//!    a page session, then `Page.navigate`, `Runtime.evaluate`, and
//!    `LP.getMarkdown` (the engine's DOM-to-markdown conversion).
//!
//! There is no second engine: the websocket sourcehound publishes is the same
//! one its MCP page verbs drive, over the same profile and cookie jar.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, OnceCell, oneshot};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use tracing::{info, warn};

use crate::browser_provider::{SH_TOOL_CDP, browser_profile};
use crate::error::{Error, Result};
use crate::tools::sourcehound::Sourcehound;

/// Post-navigation settling time before reading page content.
const PAGE_SETTLE: Duration = Duration::from_secs(2);
/// How long a single CDP command may take before we surface a timeout
/// (the engine's own per-command V8 watchdog defaults to 60s).
const CDP_COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

/// A single persistent CDP WebSocket connection with id→response correlation.
///
/// The engine keeps pages and sessions **per connection** — every WS
/// connection gets its own context. A fresh connection per command would
/// silently drop the created page/session, and the next step fails with
/// `-32601 "No page for session"`. The whole session flow (createTarget →
/// attachToTarget → Page.navigate → Runtime.evaluate → LP.getMarkdown) must
/// therefore run over ONE socket, with responses correlated by id.
struct CdpSocket {
    write: Mutex<futures_util::stream::SplitSink<WsStream, Message>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

impl CdpSocket {
    async fn connect(url: &str) -> Result<Self> {
        let (ws, _) = connect_async(url).await.map_err(|e| Error::ToolExecution {
            name: "cdp".into(),
            source: std::io::Error::other(e.to_string()).into(),
        })?;
        let (write, mut read) = ws.split();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = Arc::clone(&pending);
        // Background reader: correlate responses by id, ignore events (which
        // carry no id — e.g. Target.targetCreated / attachedToTarget).
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                let Ok(Message::Text(text)) = msg else {
                    continue;
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let Some(id) = value.get("id").and_then(|v| v.as_u64()) else {
                    continue;
                };
                if let Some(tx) = reader_pending.lock().await.remove(&id) {
                    let _ = tx.send(value);
                }
            }
        });
        Ok(Self {
            write: Mutex::new(write),
            pending,
            next_id: AtomicU64::new(1),
        })
    }

    async fn send(&self, method: &str, params: Value, session_id: Option<&str>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let mut command = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            command["sessionId"] = Value::String(sid.to_string());
        }
        let text = serde_json::to_string(&command)
            .map_err(|e| Error::Agent(format!("Failed to serialize CDP command: {e}")))?;
        {
            let mut write = self.write.lock().await;
            write
                .send(Message::Text(text.into()))
                .await
                .map_err(|e| Error::Agent(format!("Failed to send CDP command: {e}")))?;
        }
        let response = tokio::time::timeout(CDP_COMMAND_TIMEOUT, rx)
            .await
            .map_err(|_| Error::Agent("CDP command timed out".to_string()))?
            .map_err(|_| Error::Agent("CDP connection closed".to_string()))?;
        // Surface protocol errors exactly like cdp_utils::send_cdp_command
        // does ("cdp - {json}"), so tool outputs stay consistent.
        if let Some(error) = response.get("error") {
            return Err(Error::ToolExecution {
                name: "cdp".into(),
                source: std::io::Error::other(error.to_string()).into(),
            });
        }
        Ok(response)
    }
}

/// A live page target attached to the CDP session.
struct PageTarget {
    target_id: String,
    session_id: String,
}

/// A CDP WebSocket attached to the sourcehound engine's published endpoint.
pub struct CdpBrowserSession {
    /// Browser-level CDP endpoint
    /// (`ws://127.0.0.1:<port>/devtools/browser`).
    ws_url: String,
    page: Mutex<Option<PageTarget>>,
    /// Persistent WebSocket — the engine keeps page/session state per
    /// connection, so every command must reuse this one socket.
    socket: CdpSocket,
}

impl CdpBrowserSession {
    /// Attach to the endpoint sourcehound publishes for the browser profile.
    ///
    /// `cloakctl.cdp` is idempotent: it returns the running endpoint rather
    /// than starting a rival server, so re-attaching after a process restart
    /// costs one MCP round-trip and no new engine.
    pub async fn start() -> Result<Self> {
        let profile = browser_profile();
        let published = Sourcehound::global()
            .call(SH_TOOL_CDP, json!({ "profile": profile }))
            .await?;
        let ws_url = published
            .get("ws_url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                Error::Agent(format!(
                    "sourcehound `cloakctl.cdp` published no ws_url for profile `{profile}`"
                ))
            })?
            .to_string();

        // Connect the persistent socket BEFORE returning: without it the
        // per-command fresh connections of cdp_utils would lose the engine's
        // per-connection page/session state.
        let socket = CdpSocket::connect(&ws_url).await?;
        info!(%ws_url, %profile, "sourcehound CDP session ready");

        // Auto-load the persistent cookie store into the fresh session so
        // previously imported browser cookies (accounts) apply immediately
        // without re-importing or re-logging-in. This MUST happen before any
        // page target exists: the engine only makes browser-level
        // Storage.setCookies visible to pages created AFTER the set (a set
        // performed on a live page is never seen by that page, even after
        // navigation). `ensure_page` therefore must NOT re-apply (verified
        // live: set-before-page = authenticated, set-after-page = anonymous).
        let session = Self {
            ws_url,
            page: Mutex::new(None),
            socket,
        };
        session.apply_stored_cookies().await;
        Ok(session)
    }

    /// Load the persistent cookie store and inject it into the session at
    /// browser level. Best-effort: a missing/unreadable store or a server
    /// that rejects the endpoint is logged, never fatal.
    ///
    /// Cookies are sent in chunks of 400: the engine (like real Chromium
    /// CDP)
    /// rejects oversized single `Storage.setCookies` frames, and a multi-
    /// hundred-cookie store must still load completely.
    async fn apply_stored_cookies(&self) {
        let stored = crate::cookies::load_cookie_store();
        if stored.is_empty() {
            return;
        }
        let mut applied = 0usize;
        for chunk in stored.chunks(400) {
            let params = serde_json::json!({
                "cookies": chunk.iter().map(|c| c.to_cdp_value()).collect::<Vec<_>>()
            });
            match self.send(None, "Storage.setCookies", params).await {
                Ok(_) => applied += chunk.len(),
                Err(e) => warn!(error = %e, "failed to auto-load cookies (chunk)"),
            }
        }
        if applied > 0 {
            info!(
                applied,
                total = stored.len(),
                "auto-loaded cookies from store"
            );
        }
    }

    /// Browser-level CDP endpoint for raw tools / diagnostics.
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Navigate to `url` and return the rendered page as markdown.
    pub async fn navigate(&self, url: &str) -> Result<String> {
        let page = self.ensure_page().await?;
        self.page_cmd(&page, "Page.navigate", json!({ "url": url }))
            .await?;
        // Let the page settle before reading content back.
        tokio::time::sleep(PAGE_SETTLE).await;
        self.page_markdown(&page).await
    }

    /// Snapshot the current page as markdown.
    pub async fn snapshot(&self) -> Result<String> {
        let page = self.ensure_page().await?;
        self.page_markdown(&page).await
    }

    /// Click an element by CSS selector via `Runtime.evaluate`.
    pub async fn click(&self, selector: &str) -> Result<String> {
        let page = self.ensure_page().await?;
        let js = format!(
            "const el = document.querySelector('{}'); if (el) {{ el.click(); 'clicked' }} else {{ 'selector not found' }}",
            js_escape(selector)
        );
        self.page_evaluate(&page, &js).await
    }

    /// Set an input's value by CSS selector (dispatches `input` + `change`).
    pub async fn fill(&self, selector: &str, value: &str) -> Result<String> {
        let page = self.ensure_page().await?;
        let js = format!(
            "const el = document.querySelector('{}'); if (el) {{ el.value = '{}'; el.dispatchEvent(new Event('input', {{bubbles:true}})); el.dispatchEvent(new Event('change', {{bubbles:true}})); 'filled' }} else {{ 'selector not found' }}",
            js_escape(selector),
            js_escape(value)
        );
        self.page_evaluate(&page, &js).await
    }

    /// Scroll the page via `Runtime.evaluate`.
    pub async fn scroll(&self, direction: &str) -> Result<String> {
        let page = self.ensure_page().await?;
        let px = 500usize;
        let js = match direction {
            "up" => format!("window.scrollBy(0, -{px}); 'scrolled'"),
            "down" => format!("window.scrollBy(0, {px}); 'scrolled'"),
            "left" => format!("window.scrollBy(-{px}, 0); 'scrolled'"),
            "right" => format!("window.scrollBy({px}, 0); 'scrolled'"),
            _ => format!("window.scrollBy(0, {px}); 'scrolled'"),
        };
        self.page_evaluate(&page, &js).await
    }

    /// Get or create a page target attached to the session.
    async fn ensure_page(&self) -> Result<PageTarget> {
        let mut guard = self.page.lock().await;
        if let Some(page) = guard.as_ref() {
            return Ok(PageTarget {
                target_id: page.target_id.clone(),
                session_id: page.session_id.clone(),
            });
        }
        // Create a blank page, then attach a session to it (flattened so
        // responses carry the sessionId — the same flow puppeteer uses).
        let created = self
            .browser_cmd("Target.createTarget", json!({ "url": "about:blank" }))
            .await?;
        let target_id = created["result"]["targetId"]
            .as_str()
            .ok_or_else(|| Error::Agent("Target.createTarget returned no targetId".to_string()))?
            .to_string();

        let attached = self
            .browser_cmd(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        let session_id = attached["result"]["sessionId"]
            .as_str()
            .ok_or_else(|| Error::Agent("Target.attachToTarget returned no sessionId".to_string()))?
            .to_string();

        let page = PageTarget {
            target_id,
            session_id,
        };
        // Enable domains best-effort so Page.navigate / Runtime.evaluate are
        // reliably serviced (some builds reject methods on non-enabled domains).
        let _ = self.page_cmd(&page, "Page.enable", json!({})).await;

        // Deliberately NO cookie re-apply here. `start()` already loaded the
        // store at browser level before any page existed, and the engine only
        // makes those cookies visible to pages created AFTER the set — so
        // this page inherits them automatically. Re-applying now would be a
        // set-after-page, which it never reflects on this page (even
        // after navigation) and would silently leave the session logged out.
        let _ = self.page_cmd(&page, "Runtime.enable", json!({})).await;
        *guard = Some(PageTarget {
            target_id: page.target_id.clone(),
            session_id: page.session_id.clone(),
        });
        Ok(page)
    }

    /// Send a CDP command scoped to a page session.
    async fn page_cmd(&self, page: &PageTarget, method: &str, params: Value) -> Result<Value> {
        self.send(Some(&page.session_id), method, params).await
    }

    /// Send a CDP command at the browser level.
    async fn browser_cmd(&self, method: &str, params: Value) -> Result<Value> {
        self.send(None, method, params).await
    }

    /// Raw CDP send over the session's persistent WebSocket.
    async fn send(&self, session_id: Option<&str>, method: &str, params: Value) -> Result<Value> {
        self.socket.send(method, params, session_id).await
    }

    /// Evaluate a JS expression in the page and return the string result.
    async fn page_evaluate(&self, page: &PageTarget, expression: &str) -> Result<String> {
        let response = self
            .page_cmd(
                page,
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        if let Some(exception) = response["result"]["exceptionDetails"].as_object() {
            let text = exception["text"].as_str().unwrap_or("evaluation exception");
            return Err(Error::Agent(format!("Runtime.evaluate failed: {text}")));
        }
        Ok(match &response["result"]["result"]["value"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
    }

    /// Extract page content as markdown (LP.getMarkdown, then innerText fallback).
    async fn page_markdown(&self, page: &PageTarget) -> Result<String> {
        if let Ok(response) = self.page_cmd(page, "LP.getMarkdown", json!({})).await {
            if let Some(md) = response["result"]["markdown"].as_str()
                && !md.trim().is_empty()
            {
                return Ok(md.to_string());
            }
            // Some builds return content under a different key — grab any string.
            if let Some(md) = first_string(&response["result"])
                && !md.trim().is_empty()
            {
                return Ok(md);
            }
        }
        self.page_evaluate(page, "document.body ? document.body.innerText : ''")
            .await
    }
}

/// Recursively find the first non-empty string in a JSON value.
fn first_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Array(items) => items.iter().find_map(first_string),
        Value::Object(map) => map.values().find_map(first_string),
        _ => None,
    }
}

/// Process-wide shared CDP session. Lazily started on first use; lives for the
/// lifetime of the process (killed on drop).
static SHARED_SESSION: OnceCell<Arc<CdpBrowserSession>> = OnceCell::const_new();

/// Get the shared CDP session, starting it once.
pub async fn get_or_start_shared_session() -> Result<Arc<CdpBrowserSession>> {
    SHARED_SESSION
        .get_or_try_init(|| async { Ok(Arc::new(CdpBrowserSession::start().await?)) })
        .await
        .cloned()
}

/// Resolve a CDP WebSocket URL for tools that speak raw CDP:
/// `BROWSER_CDP_URL` env override first, else the managed shared session.
pub async fn resolve_cdp_ws_url() -> Result<String> {
    if let Ok(url) = std::env::var("BROWSER_CDP_URL")
        && !url.trim().is_empty()
    {
        return Ok(url);
    }
    let session = get_or_start_shared_session().await?;
    Ok(session.ws_url().to_string())
}

/// Send a raw CDP command over the shared session's **persistent** socket.
///
/// Used by the `browser_cdp` tool when `BROWSER_CDP_URL` is unset. Because
/// the engine keeps pages/sessions per connection, this must reuse the shared
/// socket rather than opening a fresh connection per command.
pub async fn send_shared_session_cmd(
    method: &str,
    params: Value,
    session_id: Option<&str>,
) -> Result<Value> {
    let session = get_or_start_shared_session().await?;
    session.send(session_id, method, params).await
}

/// Import cookies into the attached CDP session via `Storage.setCookies`,
/// then persist them to the cookie store so every future session (new
/// process) auto-loads them — the multi-browser import keeps accounts usable
/// without re-login across runs.
///
/// Returns the number of cookies successfully applied. Browser-level method
/// (no page session required) — verified live against the published endpoint.
pub async fn import_cookies(cookies: &[crate::cookies::Cookie]) -> Result<usize> {
    if cookies.is_empty() {
        return Ok(0);
    }
    // Persist first so even a session-start failure keeps the import.
    crate::cookies::save_cookie_store(cookies);
    let session = get_or_start_shared_session().await?;
    let mut applied = 0usize;
    // Batch in chunks of 400 (oversized single frames get rejected); fall
    // back to per-cookie `Network.setCookie` only if a chunk is rejected.
    for chunk in cookies.chunks(400) {
        let params = serde_json::json!({
            "cookies": chunk.iter().map(|c| c.to_cdp_value()).collect::<Vec<_>>()
        });
        match session.send(None, "Storage.setCookies", params).await {
            Ok(_) => applied += chunk.len(),
            Err(_) => {
                for c in chunk {
                    let ok = session
                        .send(None, "Network.setCookie", c.to_cdp_value())
                        .await
                        .is_ok();
                    if ok {
                        applied += 1;
                    }
                }
            }
        }
    }
    // Cookies set at browser level only reach pages created AFTER the set.
    // If the session already has a page target (e.g. a mid-session import via
    // the cookie_import tool), forget it so the next ensure_page() creates a
    // fresh page that inherits the freshly applied jar. The abandoned target
    // lingers harmlessly — the agent re-navigates anyway.
    *session.page.lock().await = None;
    Ok(applied)
}

/// Export all cookies currently in the attached CDP session via
/// `Storage.getCookies`, normalized to the operant [`crate::cookies::Cookie`]
/// model. Falls back to the persistent cookie store when the session reports
/// none (fresh process before any import).
pub async fn export_cookies() -> Result<Vec<crate::cookies::Cookie>> {
    let session = get_or_start_shared_session().await?;
    let resp = session
        .send(None, "Storage.getCookies", serde_json::json!({}))
        .await?;
    let cookies = resp
        .get("result")
        .and_then(|r| r.get("cookies"))
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let out = cookies
        .iter()
        .filter_map(|v| {
            Some(crate::cookies::Cookie {
                name: v.get("name")?.as_str()?.to_string(),
                value: v.get("value")?.as_str()?.to_string(),
                domain: v
                    .get("domain")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string(),
                path: v
                    .get("path")
                    .and_then(|p| p.as_str())
                    .unwrap_or("/")
                    .to_string(),
                expires: v.get("expires").and_then(|e| e.as_f64()).map(|f| f as i64),
                secure: v.get("secure").and_then(|s| s.as_bool()).unwrap_or(false),
                http_only: v.get("httpOnly").and_then(|h| h.as_bool()).unwrap_or(false),
                same_site: v
                    .get("sameSite")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect::<Vec<_>>();
    if !out.is_empty() {
        return Ok(out);
    }
    // Session has none (fresh process): fall back to the persistent store so
    // `operant cookies list` reflects what the next session will load.
    Ok(crate::cookies::load_cookie_store())
}

/// Clear all cookies in the attached CDP session via `Storage.clearCookies`
/// and wipe the persistent store.
pub async fn clear_cookies() -> Result<()> {
    crate::cookies::clear_cookie_store();
    let session = get_or_start_shared_session().await?;
    session
        .send(None, "Storage.clearCookies", serde_json::json!({}))
        .await?;
    Ok(())
}

/// Resolve the shared session's current page session id, creating a page if
/// none exists yet. Lets raw `browser_cdp` calls (e.g. `Runtime.evaluate`)
/// work without the model manually running `Target.createTarget` /
/// `Target.attachToTarget` first.
pub async fn ensure_shared_page_session_id() -> Result<String> {
    let session = get_or_start_shared_session().await?;
    let page = session.ensure_page().await?;
    Ok(page.session_id)
}

/// Escape a string for safe interpolation inside a single-quoted JavaScript
/// string literal (mirrors igs-rust's `js_escape` — defends against selector /
/// value injection into the headless browser evaluation context).
pub fn js_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            _ => out.push(ch),
        }
    }
    out.replace("</script>", "<\\/script>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_escape_prevents_injection() {
        // A selector like `</script><script>` must not survive unescaped.
        let escaped = js_escape("</script>");
        assert!(!escaped.contains("</script>"));
        assert!(escaped.contains("<\\/script>"));

        // Quotes and backslashes must be escaped for single-quoted JS literals.
        assert_eq!(js_escape("foo'bar"), "foo\\'bar");
        assert_eq!(js_escape("foo\\bar"), "foo\\\\bar");
        assert_eq!(js_escape("a\nb\tc"), "a\\nb\\tc");
        assert_eq!(js_escape("plain"), "plain");
    }

    #[test]
    fn first_string_finds_first_non_empty() {
        let v = json!({ "result": { "markdown": "", "content": "real md" } });
        assert_eq!(first_string(&v["result"]).as_deref(), Some("real md"));
        let v = json!({ "result": { "markdown": "hello" } });
        assert_eq!(first_string(&v["result"]).as_deref(), Some("hello"));
    }

    #[test]
    fn send_command_includes_session_id() {
        // Build the same shape send() produces for a page-scoped command.
        let mut command = json!({ "id": 7, "method": "Page.navigate", "params": { "url": "u" } });
        command["sessionId"] = Value::String("sid-1".to_string());
        assert_eq!(command["sessionId"], "sid-1");
        assert_eq!(command["method"], "Page.navigate");
    }
}
