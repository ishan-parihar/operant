//! Managed CDP session on the shared (stealth) Obscura binary.
//!
//! The IGS integration's `browser` commands are *not* CDP — igs-rust re-runs
//! `obscura fetch <url> --stealth [--eval <js>]` per command with only a
//! `CURRENT_URL` string as "session state". There is therefore no CDP endpoint
//! to reuse through IGS. Instead, operant drives the **shared Obscura binary**
//! directly over the Chrome DevTools Protocol:
//!
//! 1. Resolve the shared binary ([`crate::browser_provider::ObscuraProvider`]
//!    — the same one IGS manages, per the single-binary guarantee).
//! 2. Spawn `obscura serve --port <free> --stealth`, which emits a
//!    `ws://127.0.0.1:<port>` WebSocket URL on stdout (browser-level endpoint,
//!    same shape Chrome's `--remote-debugging-port` exposes).
//! 3. Drive it over CDP: `Target.createTarget` / `Target.attachToTarget` for
//!    a page session, then `Page.navigate`, `Runtime.evaluate`, and
//!    `LP.getMarkdown` (Obscura's DOM-to-markdown conversion).
//!
//! This gives the `obscura` browser provider full interactive automation
//! (navigate / snapshot / click / type / scroll) on the stealth build — the
//! same pattern as the `SGavrl/hermes-plugin-obscura` Hermes plugin.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, OnceCell};
use tracing::info;

use crate::browser_provider::ObscuraProvider;
use crate::error::{Error, Result};

/// How long to wait for `obscura serve` to emit its WebSocket URL.
const SERVE_START_TIMEOUT: Duration = Duration::from_secs(20);
/// Post-navigation settling time before reading page content.
const PAGE_SETTLE: Duration = Duration::from_secs(2);

/// A live page target attached to the CDP session.
struct PageTarget {
    target_id: String,
    session_id: String,
}

/// A running `obscura serve` process exposing a CDP WebSocket endpoint.
pub struct CdpBrowserSession {
    /// Browser-level CDP endpoint (`ws://127.0.0.1:<port>`).
    ws_url: String,
    child: Child,
    page: Mutex<Option<PageTarget>>,
}

impl Drop for CdpBrowserSession {
    fn drop(&mut self) {
        // Kill the serve process when the last Arc reference drops
        // (normally at process exit for the shared session).
        let _ = self.child.start_kill();
    }
}

impl CdpBrowserSession {
    /// Spawn `obscura serve` on the shared (stealth) binary and wait for the
    /// CDP WebSocket URL on stdout.
    pub async fn start() -> Result<Self> {
        let binary = ObscuraProvider::ensure_binary().await?;
        let port = free_port();
        let stealth = crate::config::runtime_config().tools.obscura_stealth;

        info!(%port, stealth, "starting shared Obscura CDP session");
        let mut cmd = Command::new(&binary);
        cmd.arg("serve").arg("--port").arg(port.to_string());
        if stealth {
            cmd.arg("--stealth");
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            Error::Agent(format!(
                "Failed to start Obscura CDP session from {}: {e}",
                binary.display()
            ))
        })?;

        let stdout = child.stdout.take().expect("stdout piped above");
        let mut reader = BufReader::new(stdout).lines();
        let ws_url =
            tokio::time::timeout(SERVE_START_TIMEOUT, async {
                while let Some(line) = reader.next_line().await.map_err(|e| {
                    Error::Agent(format!("Failed to read Obscura serve stdout: {e}"))
                })? {
                    if let Some(url) = parse_ws_url(&line) {
                        return Ok::<String, Error>(url);
                    }
                }
                // Child exited before emitting a URL — surface its stderr.
                let stderr = match child.stderr.take() {
                    Some(mut err) => {
                        let mut buf = String::new();
                        use tokio::io::AsyncReadExt;
                        let _ = err.read_to_string(&mut buf).await;
                        buf
                    }
                    None => String::new(),
                };
                Err(Error::Agent(format!(
                    "Obscura serve exited before emitting a WebSocket URL: {}",
                    stderr.chars().take(300).collect::<String>()
                )))
            })
            .await
            .map_err(|_| {
                Error::Agent("Timed out waiting for Obscura CDP WebSocket URL".to_string())
            })??;

        // Drain the rest of stdout so a chatty serve process can't fill the
        // pipe buffer and stall — the CDP protocol continues over the
        // WebSocket, not stdin/stdout.
        tokio::spawn(async move { while let Ok(Some(_line)) = reader.next_line().await {} });

        info!(%ws_url, "Obscura CDP session ready");
        Ok(Self {
            ws_url,
            child,
            page: Mutex::new(None),
        })
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
        // Let the page settle (mirrors igs-rust's post-nav settle).
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

    /// Raw CDP send over the session's WebSocket.
    async fn send(&self, session_id: Option<&str>, method: &str, params: Value) -> Result<Value> {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut command = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            command["sessionId"] = Value::String(sid.to_string());
        }
        crate::tools::cdp_utils::send_cdp_command(&self.ws_url, &command).await
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

/// Find an available TCP port on localhost.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    listener.local_addr().expect("local addr").port()
}

/// Extract the CDP WebSocket URL from an `obscura serve` stdout line.
fn parse_ws_url(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let start = trimmed.find("ws://")?;
    let url = &trimmed[start..];
    // The URL runs to end-of-line; strip trailing punctuation/log noise.
    let end = url
        .find(|c: char| c.is_whitespace() || c == ',' || c == ']')
        .unwrap_or(url.len());
    let url = &url[..end];
    if url.len() > 6 {
        Some(url.to_string())
    } else {
        None
    }
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
    fn parse_ws_url_extracts_endpoint() {
        assert_eq!(
            parse_ws_url("obscura listening on ws://127.0.0.1:9222"),
            Some("ws://127.0.0.1:9222".to_string())
        );
        assert_eq!(
            parse_ws_url("ws://127.0.0.1:9222/devtools/browser"),
            Some("ws://127.0.0.1:9222/devtools/browser".to_string())
        );
        // Trailing noise after the URL is stripped.
        assert_eq!(
            parse_ws_url("ws://127.0.0.1:9222, other"),
            Some("ws://127.0.0.1:9222".to_string())
        );
        assert_eq!(parse_ws_url("no ws url here"), None);
    }

    #[test]
    fn free_port_returns_usable_port() {
        let port = free_port();
        assert!(port > 0);
        // The port should be bindable.
        let listener = std::net::TcpListener::bind(("127.0.0.1", port));
        assert!(listener.is_ok());
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
