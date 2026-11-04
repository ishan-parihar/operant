//! Extism-based WASM execution bridge.
//!
//! Creates Extism plugin instances with permission-gated host functions
//! (`zc_http_request`, `zc_env_read`) and calls plugin-exported functions
//! (`tool_metadata`, `execute`).

use crate::PluginPermission;
use anyhow::{Context, Result};
use extism::*;
use operant_api::tool::ToolResult;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

// ── Host function context ─────────────────────────────────────────

/// Permissions available to a single plugin invocation.
#[derive(Debug, Clone)]
struct HostContext {
    permissions: HashSet<PluginPermission>,
}

// ── Data types exchanged with plugins ─────────────────────────────

/// HTTP request sent from plugin to host via `zc_http_request`.
#[derive(Debug, Serialize, Deserialize)]
struct HttpRequest {
    method: String,
    url: String,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    body: Option<String>,
}

/// HTTP response returned from host to plugin.
#[derive(Debug, Serialize, Deserialize)]
struct HttpResponse {
    status: u16,
    body: String,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
}

/// Tool metadata returned by the `tool_metadata` export.
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
}

/// Result returned by the `execute` export.
#[derive(Debug, Serialize, Deserialize)]
struct PluginToolResult {
    success: bool,
    output: String,
    #[serde(default)]
    error: Option<String>,
}

/// Verdict returned by the `observe` export (iter-1722): a plugin declaring
/// the `observer` capability receives interception events and may veto or
/// replace. `{"action":"allow"}`, `{"action":"block","reason":"..."}`, or —
/// gap G2 (dsh's post-execute replace seat) — `{"action":"replace",
/// "content":"..."}`: the content the tool result should carry instead.
/// The host applies a Replace only at post-execute (at pre-execute there is
/// nothing to replace yet; it passes through).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ObserveVerdict {
    Allow,
    Block {
        #[serde(default)]
        reason: String,
    },
    Replace {
        #[serde(default)]
        content: String,
    },
}

// ── Host function implementations ─────────────────────────────────

/// Whether an address is one a plugin may not reach (KB-17): loopback, private,
/// link-local (cloud metadata sits there), unique-local and unspecified. An
/// IPv4-mapped IPv6 address is judged as the IPv4 address it wraps.
fn is_internal_address(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            v4.is_loopback()
                || v4.is_unspecified()
                || a == 10
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 169 && b == 254)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_internal_address(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

/// Where a plugin's HTTP request may go (KB-17). Resolves the host and refuses
/// the request if any address is internal. For a host name, returns the checked
/// address to pin, so the request cannot resolve again to another address
/// (DNS rebinding).
fn checked_destination(url: &reqwest::Url) -> Result<Option<std::net::SocketAddr>, Error> {
    let addrs = url
        .socket_addrs(|| None)
        .map_err(|e| Error::msg(format!("HTTP host does not resolve: {e}")))?;
    if addrs.is_empty() {
        return Err(Error::msg("HTTP host does not resolve to any address"));
    }
    if addrs.iter().any(|addr| is_internal_address(addr.ip())) {
        return Err(Error::msg(
            "permission denied: HTTP request to an internal address",
        ));
    }
    Ok(if url.domain().is_some() {
        addrs.first().copied()
    } else {
        None
    })
}

#[expect(
    clippy::unwrap_used,
    reason = "poisoned lock: panic is the intended recovery"
)]
fn handle_http_request(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostContext>,
) -> Result<(), Error> {
    let ctx = user_data.get()?;
    let ctx = ctx.lock().unwrap();

    if !ctx.permissions.contains(&PluginPermission::HttpClient) {
        return Err(Error::msg(
            "permission denied: plugin does not have 'http_client' permission",
        ));
    }

    // Read input string from WASM memory
    let request_json: String = plugin.memory_get_val(&inputs[0])?;

    let req: HttpRequest = serde_json::from_str(&request_json)
        .map_err(|e| Error::msg(format!("invalid HTTP request JSON: {e}")))?;

    let url =
        reqwest::Url::parse(&req.url).map_err(|e| Error::msg(format!("invalid HTTP URL: {e}")))?;
    let pinned = checked_destination(&url)?;

    // 120s ceiling covers legitimate slow cases: large file downloads and slow
    // model-inference endpoints (fal.ai image generation routinely takes 20-60s
    // on cold models). A per-plugin override or tighter default is a candidate
    // follow-up — see ADR-003 §"Known gaps". Note: this runs inside
    // spawn_blocking, so a stalled request holds a blocking-pool thread for
    // the full duration.
    let mut client_builder = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        // A redirect is handed back to the plugin, not followed: a followed one
        // would reach an address the check above never saw (KB-17).
        .redirect(reqwest::redirect::Policy::none());
    if let (Some(domain), Some(addr)) = (url.domain(), pinned) {
        client_builder = client_builder.resolve(domain, addr);
    }
    let client = client_builder
        .build()
        .map_err(|e| Error::msg(format!("failed to create HTTP client: {e}")))?;

    let mut builder = match req.method.to_uppercase().as_str() {
        "GET" => client.get(&req.url),
        "POST" => client.post(&req.url),
        "PUT" => client.put(&req.url),
        "DELETE" => client.delete(&req.url),
        "PATCH" => client.patch(&req.url),
        "HEAD" => client.head(&req.url),
        other => {
            return Err(Error::msg(format!("unsupported HTTP method: {other}")));
        }
    };

    for (k, v) in &req.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    if let Some(body) = req.body {
        builder = builder.body(body);
    }

    let resp = builder
        .send()
        .map_err(|e| Error::msg(format!("HTTP request failed: {e}")))?;

    let status = resp.status().as_u16();
    let headers: std::collections::HashMap<String, String> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = resp
        .text()
        .map_err(|e| Error::msg(format!("failed to read response body: {e}")))?;

    let response = HttpResponse {
        status,
        body,
        headers,
    };

    let response_json = serde_json::to_string(&response)
        .map_err(|e| Error::msg(format!("failed to serialize response: {e}")))?;

    plugin.memory_set_val(&mut outputs[0], response_json)?;

    Ok(())
}

#[expect(
    clippy::unwrap_used,
    reason = "poisoned lock: panic is the intended recovery"
)]
fn handle_env_read(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostContext>,
) -> Result<(), Error> {
    let ctx = user_data.get()?;
    let ctx = ctx.lock().unwrap();

    if !ctx.permissions.contains(&PluginPermission::EnvRead) {
        return Err(Error::msg(
            "permission denied: plugin does not have 'env_read' permission",
        ));
    }

    let var_name: String = plugin.memory_get_val(&inputs[0])?;

    let value = std::env::var(&var_name)
        .map_err(|_| Error::msg(format!("environment variable '{var_name}' not set")))?;

    plugin.memory_set_val(&mut outputs[0], value)?;

    Ok(())
}

// ── Plugin creation and invocation ────────────────────────────────

/// How long a caller waits for a free instance before a resident checkout
/// gives up (`Ok(None)`). Not a tuned threshold — a ceiling against every
/// instance of a module being stuck in a hung call; extism calls themselves
/// are unbounded (pre-existing behavior, unchanged by pooling).
pub(crate) const PLUGIN_POOL_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A resident pool of plugin instances for ONE WASM module (red-team audit
/// F1, iter-1725). Both the tool bridge (`wasm_tool`) and the observer
/// bridge used to instantiate a fresh `extism::Plugin` per call — one
/// compile + instantiate per tool invocation. `extism::Plugin` is Send +
/// Sync (upstream unsafe impls, extism-1.41.0/src/plugin.rs:188-189 — the
/// "Plugin is !Send" comment that justified the old per-call loads was
/// stale) and ships `Pool` for exactly this pattern: instances are created
/// lazily up to `available_parallelism` and reused.
///
/// All methods are SYNC (extism calls block); async callers own the hop to
/// `spawn_blocking` (see `wasm_tool::WasmTool::execute`).
#[derive(Clone)]
pub struct PluginPool {
    pool: extism::Pool,
}

impl PluginPool {
    /// A pool whose instances are created from `wasm_path` with the given
    /// permissions. Nothing is compiled until the first call; a module that
    /// fails to instantiate fails that call, and the callers' policies
    /// decide what surfaces (tools → error result, observers → fail-open).
    pub fn new(wasm_path: &Path, permissions: &[PluginPermission]) -> Self {
        let wasm_path = wasm_path.to_path_buf();
        let permissions = permissions.to_vec();
        Self {
            pool: extism::Pool::new(move || {
                create_plugin(&wasm_path, &permissions)
                    .map_err(|e| extism::Error::msg(format!("{e:#}")))
            }),
        }
    }

    /// Call the `observe` export on a checked-out instance. `Ok(None)`:
    /// every instance was busy past `PLUGIN_POOL_WAIT`. `Err`: the module
    /// failed to instantiate or the call failed — the caller decides the
    /// fail-open policy.
    pub fn observe(&self, event_json: &str) -> Result<Option<ObserveVerdict>> {
        let event_json = event_json.to_string();
        match self.pool.with_plugin(PLUGIN_POOL_WAIT, |plugin| {
            call_observe(plugin, &event_json).map_err(|e| extism::Error::msg(format!("{e:#}")))
        }) {
            Err(e) => Err(e.context("plugin module failed to load")),
            Ok(None) => Ok(None),
            Ok(Some(verdict)) => Ok(Some(verdict)),
        }
    }

    /// Call the `execute` export on a checked-out instance (same contract
    /// as `observe`; the tool bridge surfaces `Ok(None)` as a plain error
    /// result to the model).
    pub fn execute(&self, args_json: &[u8]) -> Result<Option<ToolResult>> {
        let args_json = args_json.to_vec();
        match self.pool.with_plugin(PLUGIN_POOL_WAIT, |plugin| {
            call_execute(plugin, &args_json).map_err(|e| extism::Error::msg(format!("{e:#}")))
        }) {
            Err(e) => Err(e.context("plugin module failed to load")),
            Ok(None) => Ok(None),
            Ok(Some(result)) => Ok(Some(result)),
        }
    }
}

/// Create an Extism plugin from a WASM file with the given permissions.
pub fn create_plugin(wasm_path: &Path, permissions: &[PluginPermission]) -> Result<extism::Plugin> {
    let perm_set: HashSet<PluginPermission> = permissions.iter().cloned().collect();
    let ctx = UserData::new(HostContext {
        permissions: perm_set,
    });

    let http_fn = Function::new(
        "zc_http_request",
        [PTR],
        [PTR],
        ctx.clone(),
        handle_http_request,
    );

    let env_fn = Function::new("zc_env_read", [PTR], [PTR], ctx, handle_env_read);

    let manifest = Manifest::new([Wasm::file(wasm_path)]);

    Plugin::new(manifest, [http_fn, env_fn], true)
        .with_context(|| format!("failed to load WASM plugin from {}", wasm_path.display()))
}

/// Call the `tool_metadata` export and parse the result.
pub fn call_tool_metadata(plugin: &mut extism::Plugin) -> Result<ToolMetadata> {
    let output = plugin
        .call::<&str, String>("tool_metadata", "")
        .context("failed to call tool_metadata export")?;

    serde_json::from_str(&output).context("failed to parse tool_metadata JSON")
}

/// Call the `execute` export with the given args JSON and return a `ToolResult`.
pub fn call_execute(plugin: &mut extism::Plugin, args_json: &[u8]) -> Result<ToolResult> {
    let input = std::str::from_utf8(args_json).context("plugin args are not valid UTF-8")?;

    let output = plugin
        .call::<&str, String>("execute", input)
        .context("failed to call plugin execute export")?;

    let result: PluginToolResult =
        serde_json::from_str(&output).context("failed to parse plugin execute result")?;

    Ok(ToolResult {
        success: result.success,
        output: result.output,
        error: result.error,
    })
}

/// Call the `observe` export with an interception-event JSON and parse the
/// verdict. A plugin without the export errors here; the caller decides the
/// fail-open policy (the tool seam allows on error).
pub fn call_observe(plugin: &mut extism::Plugin, event_json: &str) -> Result<ObserveVerdict> {
    let output = plugin
        .call::<&str, String>("observe", event_json)
        .context("failed to call observe export")?;
    serde_json::from_str(&output).context("failed to parse observe verdict JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_context_permission_check() {
        let ctx = HostContext {
            permissions: HashSet::from([PluginPermission::HttpClient]),
        };
        assert!(ctx.permissions.contains(&PluginPermission::HttpClient));
        assert!(!ctx.permissions.contains(&PluginPermission::EnvRead));
    }

    #[test]
    fn http_request_serde_roundtrip() {
        let req = HttpRequest {
            method: "POST".into(),
            url: "https://example.com/api".into(),
            headers: [("Authorization".into(), "Bearer tok".into())]
                .into_iter()
                .collect(),
            body: Some(r#"{"key":"value"}"#.into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: HttpRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.url, "https://example.com/api");
        assert_eq!(parsed.body.as_deref(), Some(r#"{"key":"value"}"#));
    }

    #[test]
    fn tool_metadata_serde() {
        let meta = ToolMetadata {
            name: "test_tool".into(),
            description: "A test tool".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: ToolMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test_tool");
    }

    #[test]
    fn plugin_tool_result_serde() {
        let result = PluginToolResult {
            success: true,
            output: "hello".into(),
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: PluginToolResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.output, "hello");
    }

    #[test]
    fn missing_wasm_file_returns_error() {
        let result = create_plugin(Path::new("/nonexistent/plugin.wasm"), &[]);
        assert!(result.is_err());
    }

    #[test]
    fn pool_on_a_module_that_fails_to_instantiate_errors_cleanly() {
        // iter-1725: the resident pool's source closure failing (broken or
        // missing module) surfaces as a typed Err, not a panic — both
        // callers' policies (tool error result, observer fail-open) key
        // off this.
        let pool = PluginPool::new(Path::new("/nonexistent/pool-test.wasm"), &[]);
        let err = pool.observe(r#"{"point":"pre_execute"}"#).unwrap_err();
        assert!(err.to_string().contains("failed to load"), "{err:#}");
        let err = pool.execute(b"{}").unwrap_err();
        assert!(err.to_string().contains("failed to load"), "{err:#}");
    }

    #[test]
    fn observe_verdict_parses_allow_and_block() {
        // iter-1722: the `observe` export's contract, pinned without WASM.
        let allow: ObserveVerdict =
            serde_json::from_str(r#"{"action":"allow"}"#).expect("allow parses");
        assert_eq!(allow, ObserveVerdict::Allow);

        let block: ObserveVerdict =
            serde_json::from_str(r#"{"action":"block","reason":"no writes on sundays"}"#)
                .expect("block parses");
        assert_eq!(
            block,
            ObserveVerdict::Block {
                reason: "no writes on sundays".into()
            }
        );

        // gap G2: the replace verdict — the post-execute replace seat.
        let replace: ObserveVerdict =
            serde_json::from_str(r#"{"action":"replace","content":"[redacted by policy]"}"#)
                .expect("replace parses");
        assert_eq!(
            replace,
            ObserveVerdict::Replace {
                content: "[redacted by policy]".into()
            }
        );

        // A block without a reason is still a block (empty reason).
        let block_no_reason: ObserveVerdict =
            serde_json::from_str(r#"{"action":"block"}"#).expect("bare block parses");
        assert_eq!(
            block_no_reason,
            ObserveVerdict::Block {
                reason: String::new()
            }
        );

        // Anything else is an error — the caller fails open.
        assert!(serde_json::from_str::<ObserveVerdict>(r#"{"action":"pause"}"#).is_err());
        assert!(serde_json::from_str::<ObserveVerdict>("not json").is_err());
    }

    /// Integration tests that load the actual image-gen WASM plugin.
    /// These require the plugin to be built first:
    ///   cd plugins/image-gen-fal && cargo build --target wasm32-wasip1 --release
    mod integration {
        use super::*;

        fn wasm_path() -> Option<std::path::PathBuf> {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins/image-gen-fal/image_gen_fal.wasm");
            if path.exists() { Some(path) } else { None }
        }

        #[test]
        fn load_and_read_metadata() {
            let Some(path) = wasm_path() else {
                eprintln!("SKIP: image_gen_fal.wasm not found (build the plugin first)");
                return;
            };
            let perms = vec![PluginPermission::HttpClient, PluginPermission::EnvRead];
            let mut plugin = create_plugin(&path, &perms).unwrap();
            let meta = call_tool_metadata(&mut plugin).unwrap();
            assert_eq!(meta.name, "image_gen_fal");
            assert!(meta.description.contains("image"));
            assert!(
                meta.parameters_schema["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == "prompt")
            );
        }

        #[test]
        fn execute_missing_prompt() {
            let Some(path) = wasm_path() else { return };
            let perms = vec![PluginPermission::HttpClient, PluginPermission::EnvRead];
            let mut plugin = create_plugin(&path, &perms).unwrap();
            let args = serde_json::to_vec(&serde_json::json!({})).unwrap();
            let result = call_execute(&mut plugin, &args).unwrap();
            assert!(!result.success);
            assert!(result.error.as_deref().unwrap().contains("prompt"));
        }

        #[test]
        fn execute_invalid_size() {
            let Some(path) = wasm_path() else { return };
            let perms = vec![PluginPermission::HttpClient, PluginPermission::EnvRead];
            let mut plugin = create_plugin(&path, &perms).unwrap();
            let args =
                serde_json::to_vec(&serde_json::json!({"prompt": "test", "size": "bad"})).unwrap();
            let result = call_execute(&mut plugin, &args).unwrap();
            assert!(!result.success);
            assert!(result.error.as_deref().unwrap().contains("Invalid size"));
        }

        #[test]
        fn execute_invalid_model_traversal() {
            let Some(path) = wasm_path() else { return };
            let perms = vec![PluginPermission::HttpClient, PluginPermission::EnvRead];
            let mut plugin = create_plugin(&path, &perms).unwrap();
            let args =
                serde_json::to_vec(&serde_json::json!({"prompt": "test", "model": "../../evil"}))
                    .unwrap();
            let result = call_execute(&mut plugin, &args).unwrap();
            assert!(!result.success);
            assert!(result.error.as_deref().unwrap().contains("Invalid model"));
        }

        /// End-to-end: missing `FAL_API_KEY` exercises the `zc_env_read` host
        /// function — the host returns Err (var unset), which Extism propagates
        /// as a plugin-call trap. Proves the env_read path is wired.
        #[test]
        fn execute_missing_api_key_exercises_env_read_host_fn() {
            let Some(path) = wasm_path() else { return };
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("FAL_API_KEY") };
            let perms = vec![PluginPermission::HttpClient, PluginPermission::EnvRead];
            let mut plugin = create_plugin(&path, &perms).unwrap();
            let args = serde_json::to_vec(&serde_json::json!({"prompt": "a sunset"})).unwrap();
            let err = call_execute(&mut plugin, &args).unwrap_err();
            let msg = format!("{err:#}");
            assert!(
                msg.contains("FAL_API_KEY") || msg.contains("not set"),
                "expected env-var error, got: {msg}"
            );
        }

        /// End-to-end permission enforcement: without `EnvRead`, the host
        /// function returns permission-denied and Extism propagates it as a trap.
        #[test]
        fn execute_without_env_read_permission_fails() {
            let Some(path) = wasm_path() else { return };
            // Only HttpClient granted — EnvRead missing
            let perms = vec![PluginPermission::HttpClient];
            let mut plugin = create_plugin(&path, &perms).unwrap();
            let args = serde_json::to_vec(&serde_json::json!({"prompt": "a sunset"})).unwrap();
            let err = call_execute(&mut plugin, &args).unwrap_err();
            let msg = format!("{err:#}");
            assert!(
                msg.contains("permission") || msg.contains("env_read"),
                "expected permission-denied error, got: {msg}"
            );
        }
    }
}

#[cfg(test)]
mod internal_address_tests {
    use super::{checked_destination, is_internal_address};
    use std::net::IpAddr;

    #[test]
    fn internal_addresses_are_refused_and_public_ones_are_not() {
        // KB-17: loopback, private, link-local (metadata) and unique-local are
        // refused, including their IPv4-mapped forms; public addresses pass.
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.5",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_internal_address(ip.parse::<IpAddr>().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "8.8.8.8", "2606:4700::1111"] {
            assert!(!is_internal_address(ip.parse::<IpAddr>().unwrap()), "{ip}");
        }
    }

    #[test]
    fn a_request_to_an_internal_literal_is_refused_before_it_is_sent() {
        for raw in [
            "http://127.0.0.1:8080/x",
            "http://169.254.169.254/latest/meta-data/",
        ] {
            let url = reqwest::Url::parse(raw).unwrap();
            assert!(checked_destination(&url).is_err(), "{raw}");
        }
        let public = reqwest::Url::parse("https://93.184.216.34/").unwrap();
        assert_eq!(checked_destination(&public).unwrap(), None);
    }
}
