//! Browser provider for Operant-RS.
//!
//! (plan 11C2) sourcehound is the only engine: `config.browser.provider`
//! is accepted for config compatibility but ignored — every value lands on
//! the sourcehound browser (over MCP, CDP-capable). The camofox /
//! browserbase / browser-use / firecrawl backends are deleted.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::tools::sourcehound::Sourcehound;
use reqwest;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait BrowserProvider: Send + Sync {
    fn name(&self) -> &str;
    /// True when required credentials / binary are present.
    fn is_configured(&self) -> bool;
    /// Navigate to URL; return page text/snapshot.
    async fn navigate(&self, url: &str) -> Result<String>;
    /// Take a text snapshot of the current page.
    async fn snapshot(&self) -> Result<String>;
    /// Click element identified by selector.
    async fn click(&self, selector: &str) -> Result<String>;
    /// Type text into selector.
    async fn type_text(&self, selector: &str, text: &str) -> Result<String>;
    /// Scroll the page.
    async fn scroll(&self, direction: &str) -> Result<String>;
    /// Generic command dispatch (for providers that speak their own protocol).
    async fn execute(&self, command: &str, args: Value) -> Result<String> {
        match command {
            "navigate" => self.navigate(args["url"].as_str().unwrap_or("")).await,
            "snapshot" => self.snapshot().await,
            "click" => self.click(args["selector"].as_str().unwrap_or("")).await,
            "type" => {
                self.type_text(
                    args["selector"].as_str().unwrap_or(""),
                    args["text"].as_str().unwrap_or(""),
                )
                .await
            }
            "scroll" => {
                self.scroll(args["direction"].as_str().unwrap_or("down"))
                    .await
            }
            _ => Err(Error::Agent(format!(
                "Unknown browser command: {}",
                command
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Sourcehound — browser engine driven over MCP (`sourcehound mcp`, stdio)
// ---------------------------------------------------------------------------

/// MCP tool names on the wire. The sourcehound server advertises **dotted**
/// names in `tools/list` (`cloakctl.cdp`, `cloakctl.navigate`, …). The `lp_*`
/// browser verbs (`lp_goto`, `lp_markdown`) exist only as the
/// `sourcehound browser …` CLI subcommands — they are not MCP tools — so
/// navigation goes through `cloakctl.navigate` + `cloakctl.read`.
pub const SH_TOOL_PROFILE_CREATE: &str = "cloakctl.profile_create";
pub const SH_TOOL_OPEN: &str = "cloakctl.open";
pub const SH_TOOL_CDP: &str = "cloakctl.cdp";
pub const SH_TOOL_NAVIGATE: &str = "cloakctl.navigate";
pub const SH_TOOL_READ: &str = "cloakctl.read";
pub const SH_TOOL_ACT: &str = "cloakctl.act";

/// The sourcehound profile the browser provider drives. Stable across runs so
/// the profile's cookie jar — and any logged-in session in it — survives a
/// restart. Overridable so a second operant install (or a test) can use its own
/// profile instead of taking the single-writer lock on the default one.
pub fn browser_profile() -> String {
    std::env::var("OPERANT_BROWSER_PROFILE")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "operant".to_string())
}

/// Readable page text out of a sourcehound page payload. Falls back to the
/// whole payload so a shape change degrades to raw JSON rather than an empty
/// string the agent cannot use.
fn page_text(payload: &Value, key: &str) -> String {
    match payload.get(key).and_then(Value::as_str) {
        Some(text) if !text.is_empty() => text.to_string(),
        _ => payload.to_string(),
    }
}

/// Browser provider backed by the sourcehound MCP server.
///
/// sourcehound owns the engine process, its persistent profile (and therefore
/// its cookie jar) and its DevTools endpoint; operant only speaks MCP to it
/// over stdio, through the shared [`Sourcehound`] client. `cloakctl.cdp`
/// publishes a real DevTools websocket
/// (`ws://127.0.0.1:<port>/devtools/browser`) for the tools that attach to CDP
/// directly — see [`crate::sourcehound_cdp`].
pub struct SourcehoundProvider {
    sh: Sourcehound,
    profile: String,
    /// Set once the profile has been created and opened. `None` until the
    /// first call.
    prepared: tokio::sync::OnceCell<()>,
}

impl Default for SourcehoundProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SourcehoundProvider {
    pub fn new() -> Self {
        Self {
            sh: Sourcehound::global(),
            profile: browser_profile(),
            prepared: tokio::sync::OnceCell::const_new(),
        }
    }

    /// Build a provider on an explicit MCP server command. Test seam: the
    /// dispatch path below is identical, only the transport is swapped.
    #[cfg(test)]
    fn with_command(command: impl Into<String>, args: Vec<String>, profile: &str) -> Self {
        Self {
            sh: Sourcehound::new(command, args),
            profile: profile.to_string(),
            prepared: tokio::sync::OnceCell::const_new(),
        }
    }

    /// Claim the profile. Both sourcehound verbs are idempotent by contract —
    /// creating an existing profile is a no-op ack, and opening a live profile
    /// re-attaches rather than taking a second writer — so this runs once.
    async fn prepare(&self) -> Result<()> {
        self.prepared
            .get_or_try_init(|| async {
                for tool in [SH_TOOL_PROFILE_CREATE, SH_TOOL_OPEN] {
                    self.sh
                        .call(tool, serde_json::json!({ "profile": self.profile }))
                        .await?;
                }
                Ok(())
            })
            .await
            .copied()
    }

    /// One `cloakctl.act` call with the profile injected.
    async fn act(&self, args: Value) -> Result<String> {
        self.prepare().await?;
        let mut args = args;
        if let Some(map) = args.as_object_mut() {
            map.insert("profile".to_string(), Value::String(self.profile.clone()));
        }
        let payload = self.sh.call(SH_TOOL_ACT, args).await?;
        Ok(payload.to_string())
    }
}

#[async_trait]
impl BrowserProvider for SourcehoundProvider {
    fn name(&self) -> &str {
        "sourcehound"
    }

    fn is_configured(&self) -> bool {
        self.sh.is_available()
    }

    async fn navigate(&self, url: &str) -> Result<String> {
        self.prepare().await?;
        self.sh
            .call(
                SH_TOOL_NAVIGATE,
                serde_json::json!({ "profile": self.profile, "action": "url", "url": url }),
            )
            .await?;
        // `cloakctl.navigate` answers with the navigation envelope only
        // (profile / tab / action / url / title); the page body is a separate
        // read, exactly as the tool's own snapshot → act → verify loop expects.
        self.snapshot().await
    }

    async fn snapshot(&self) -> Result<String> {
        self.prepare().await?;
        let payload = self
            .sh
            .call(
                SH_TOOL_READ,
                serde_json::json!({ "profile": self.profile, "format": "markdown" }),
            )
            .await?;
        Ok(page_text(&payload, "content"))
    }

    async fn click(&self, selector: &str) -> Result<String> {
        self.act(serde_json::json!({ "kind": "click", "selector": selector }))
            .await
    }

    async fn type_text(&self, selector: &str, text: &str) -> Result<String> {
        self.act(serde_json::json!({ "kind": "fill", "selector": selector, "text": text }))
            .await
    }

    async fn scroll(&self, direction: &str) -> Result<String> {
        // One 500px step per direction — the same step the managed CDP session
        // used, so scroll distance does not change with the transport.
        const STEP: i64 = 500;
        let (dx, dy) = match direction {
            "up" => (0, -STEP),
            "left" => (-STEP, 0),
            "right" => (STEP, 0),
            _ => (0, STEP),
        };
        self.act(serde_json::json!({ "kind": "scroll", "dx": dx, "dy": dy }))
            .await
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// (plan 11C2) The one browser provider: sourcehound. The `name` argument
/// is accepted so existing configs keep loading, but every value — the
/// purged camofox/browserbase/browser-use/firecrawl and the retired
/// `obscura`/`lightpanda` — now lands on sourcehound; the load-time
/// warning (`BrowserSettings` migration in config.rs) tells the user.
pub fn build_browser_provider(_name: &str) -> std::sync::Arc<dyn BrowserProvider> {
    std::sync::Arc::new(SourcehoundProvider::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Mock sourcehound MCP server. Records every `tools/call` (name +
    /// arguments) as one JSON line per call into the log file, then answers
    /// with the same `structuredContent` shape the real server returns:
    /// `cloakctl.read` carries a page body, everything else an envelope.
    const MOCK_SOURCEHOUND: &str = r#"import sys, json
log = sys.argv[1]
def emit(rid, result):
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    rid = req.get('id')
    if rid is None:
        continue
    mid = req.get('method')
    if mid == 'initialize':
        emit(rid, {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'mock-sourcehound', 'version': '1.0'}})
    elif mid == 'tools/list':
        emit(rid, {'tools': []})
    elif mid == 'tools/call':
        params = req.get('params') or {}
        with open(log, 'a') as fh:
            fh.write(json.dumps({'name': params.get('name'), 'arguments': params.get('arguments')}) + chr(10))
        if params.get('name') == 'cloakctl.read':
            payload = {'profile': 'p', 'tab': 'page-1', 'url': 'https://example.test/', 'format': 'markdown', 'content': 'MOCK PAGE BODY'}
        elif params.get('name') == 'cloakctl.navigate':
            payload = {'profile': 'p', 'tab': 'page-1', 'action': 'url', 'url': params.get('arguments', {}).get('url'), 'title': 'Mock'}
        else:
            payload = {'profile': 'p'}
        emit(rid, {'content': [{'type': 'text', 'text': 'ok'}], 'structuredContent': payload})
    else:
        emit(rid, {})
"#;

    /// A provider wired to a mock sourcehound MCP server, plus the call-log
    /// path the mock appends every `tools/call` to.
    fn mock_sourcehound() -> (SourcehoundProvider, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "operant_sh_mock_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("mock_sourcehound.py");
        std::fs::write(&script, MOCK_SOURCEHOUND).unwrap();
        let log = dir.join("calls.log");
        std::fs::write(&log, b"").unwrap();
        let provider = SourcehoundProvider::with_command(
            "python3",
            vec![
                script.to_string_lossy().to_string(),
                log.to_string_lossy().to_string(),
            ],
            "operant-test",
        );
        (provider, log)
    }

    /// Every `tools/call` the mock recorded, in order.
    fn recorded_calls(log: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn provider_navigates_through_sourcehound_navigate_and_read() {
        let (provider, log) = mock_sourcehound();

        let body = provider.navigate("https://example.test/").await.unwrap();

        // The provider returns the page body it read back after navigating —
        // not the navigation envelope.
        assert_eq!(body, "MOCK PAGE BODY");

        let calls = recorded_calls(&log);
        let names: Vec<&str> = calls.iter().filter_map(|c| c["name"].as_str()).collect();
        assert_eq!(
            names,
            vec![
                "cloakctl.profile_create",
                "cloakctl.open",
                "cloakctl.navigate",
                "cloakctl.read",
            ],
            "navigation must dispatch the sourcehound tool names, in order"
        );

        // Every call carries the profile, and the navigate call carries the URL.
        for call in &calls {
            assert_eq!(call["arguments"]["profile"], "operant-test");
        }
        assert_eq!(
            calls[2]["arguments"]["action"], "url",
            "cloakctl.navigate needs action=url for a target address"
        );
        assert_eq!(calls[2]["arguments"]["url"], "https://example.test/");
        assert_eq!(calls[3]["arguments"]["format"], "markdown");
    }

    #[tokio::test]
    async fn provider_interactions_dispatch_cloakctl_act() {
        let (provider, log) = mock_sourcehound();

        provider.click("#submit").await.unwrap();
        provider.type_text("#name", "ada").await.unwrap();
        provider.scroll("up").await.unwrap();
        provider.scroll("down").await.unwrap();
        provider.scroll("left").await.unwrap();

        let calls = recorded_calls(&log);
        let acts: Vec<&Value> = calls
            .iter()
            .filter(|c| c["name"] == "cloakctl.act")
            .collect();
        assert_eq!(acts.len(), 5, "one cloakctl.act per interaction");

        assert_eq!(acts[0]["arguments"]["kind"], "click");
        assert_eq!(acts[0]["arguments"]["selector"], "#submit");

        assert_eq!(acts[1]["arguments"]["kind"], "fill");
        assert_eq!(acts[1]["arguments"]["selector"], "#name");
        assert_eq!(acts[1]["arguments"]["text"], "ada");

        // Directions map to pixel steps, not to a string the server never sees.
        assert_eq!(
            (
                acts[2]["arguments"]["dy"].as_i64(),
                acts[2]["arguments"]["dx"].as_i64()
            ),
            (Some(-500), Some(0))
        );
        assert_eq!(
            (
                acts[3]["arguments"]["dy"].as_i64(),
                acts[3]["arguments"]["dx"].as_i64()
            ),
            (Some(500), Some(0))
        );
        assert_eq!(
            (
                acts[4]["arguments"]["dy"].as_i64(),
                acts[4]["arguments"]["dx"].as_i64()
            ),
            (Some(0), Some(-500))
        );

        for act in &acts {
            assert_eq!(act["arguments"]["profile"], "operant-test");
        }
    }

    #[tokio::test]
    async fn provider_snapshot_reads_the_page_body() {
        let (provider, log) = mock_sourcehound();

        assert_eq!(provider.snapshot().await.unwrap(), "MOCK PAGE BODY");
        let calls = recorded_calls(&log);
        assert_eq!(calls.last().unwrap()["name"], "cloakctl.read");
    }

    #[test]
    fn page_text_falls_back_to_the_whole_payload() {
        let payload = serde_json::json!({ "profile": "p", "tab": "page-1" });
        // No `content` key: degrade to raw JSON rather than an empty string.
        assert_eq!(page_text(&payload, "content"), payload.to_string());
        assert_eq!(
            page_text(&serde_json::json!({ "content": "" }), "content"),
            serde_json::json!({ "content": "" }).to_string()
        );
    }

    #[test]
    fn browser_profile_falls_back_to_operant() {
        let _guard = env_lock().lock().unwrap();
        let previous = std::env::var_os("OPERANT_BROWSER_PROFILE");
        // SAFETY: test-only env mutation under the module's exclusive lock.
        unsafe { std::env::remove_var("OPERANT_BROWSER_PROFILE") };
        assert_eq!(browser_profile(), "operant");
        unsafe { std::env::set_var("OPERANT_BROWSER_PROFILE", "custom") };
        assert_eq!(browser_profile(), "custom");
        match previous {
            Some(value) => unsafe { std::env::set_var("OPERANT_BROWSER_PROFILE", value) },
            None => unsafe { std::env::remove_var("OPERANT_BROWSER_PROFILE") },
        }
    }

    #[test]
    fn sourcehound_provider_maps_from_the_factory() {
        let p = build_browser_provider("sourcehound");
        assert_eq!(p.name(), "sourcehound");
    }

    #[test]
    fn test_unknown_falls_back_to_sourcehound() {
        let p = build_browser_provider("unknown");
        assert_eq!(p.name(), "sourcehound");
    }

    #[test]
    fn retired_provider_names_fall_back_to_sourcehound() {
        // The retired `igs` / `obscura` / `lightpanda` names used to resolve to
        // their own providers. A config still naming one now hits the catch-all,
        // so the fallback has to stay the documented default rather than fail.
        for retired in ["igs", "obscura", "lightpanda"] {
            assert_eq!(build_browser_provider(retired).name(), "sourcehound");
        }
    }
}
