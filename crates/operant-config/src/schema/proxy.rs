//! `proxy` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use super::*;

/// The process-global proxy configuration, set at startup.
pub(crate) static RUNTIME_PROXY_CONFIG: OnceLock<RwLock<ProxyConfig>> = OnceLock::new();
/// The process-global proxy client cache (one client per proxy URL).
pub(crate) static RUNTIME_PROXY_CLIENT_CACHE: OnceLock<RwLock<HashMap<String, reqwest::Client>>> =
    OnceLock::new();
use anyhow::{Context, Result};
use operant_macros::Configurable;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// The crypto provider this build uses, named EXPLICITLY.
///
/// `rustls::ClientConfig::builder()` resolves the provider from the crate
/// FEATURES, and PANICS when they are ambiguous. They ARE ambiguous in
/// this workspace: `hyper-rustls` (reached through `reqwest`) turns on
/// `aws-lc-rs` by default, and `tokio-rustls` turns on `ring` — so both
/// are on whenever the workspace is built with `--all-features`, and the
/// workspace-wide feature unification brings `aws-lc-rs` in even when a
/// crate's own dependency does not.
///
/// `builder_with_provider` names the provider instead of inferring it, so
/// the code works under EVERY feature combination — and the choice is
/// visible at the call site rather than implied by a dependency's
/// defaults.
pub fn crypto_provider() -> std::sync::Arc<rustls::crypto::CryptoProvider> {
    std::sync::Arc::new(rustls::crypto::ring::default_provider())
}

/// The rustls client configuration, with the provider named.
///
/// Use this instead of `rustls::ClientConfig::builder()`: the builder
/// resolves the provider from the crate features and panics when they are
/// ambiguous, which they are here.
///
/// The protocol versions are set explicitly because `builder_with_provider`
/// does not default them — unlike `builder()`, which takes the defaults
/// along with the (inferable) provider.
pub fn client_config(roots: rustls::RootCertStore) -> anyhow::Result<rustls::ClientConfig> {
    Ok(
        rustls::ClientConfig::builder_with_provider(crypto_provider())
            .with_safe_default_protocol_versions()
            .context("rustls protocol versions")?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

pub(crate) const SUPPORTED_PROXY_SERVICE_KEYS: &[&str] = &[
    "provider.anthropic",
    "provider.compatible",
    "provider.copilot",
    "provider.gemini",
    "provider.glm",
    "provider.ollama",
    "provider.openai",
    "provider.openrouter",
    "channel.dingtalk",
    "channel.discord",
    "channel.feishu",
    "channel.lark",
    "channel.matrix",
    "channel.mattermost",
    "channel.nextcloud_talk",
    "channel.qq",
    "channel.signal",
    "channel.slack",
    "channel.telegram",
    "channel.wati",
    "channel.wechat",
    "channel.whatsapp",
    "tool.browser",
    "tool.http_request",
    "tool.web_search",
    "memory.embeddings",
    "tunnel.custom",
    "transcription.groq",
];

pub(crate) const SUPPORTED_PROXY_SERVICE_SELECTORS: &[&str] = &[
    "provider.*",
    "channel.*",
    "tool.*",
    "memory.*",
    "tunnel.*",
    "transcription.*",
];

/// Proxy application scope — determines which outbound traffic uses the proxy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProxyScope {
    /// Use system environment proxy variables only.
    Environment,
    /// Apply proxy to all Operant-managed HTTP traffic (default).
    #[default]
    Zeroclaw,
    /// Apply proxy only to explicitly listed service selectors.
    Services,
}

/// Proxy configuration for outbound HTTP/HTTPS/SOCKS5 traffic (`[proxy]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "proxy"]
pub struct ProxyConfig {
    /// Enable proxy support for selected scope.
    #[serde(default)]
    pub enabled: bool,
    /// Proxy URL for HTTP requests (supports http, https, socks5, socks5h).
    #[serde(default)]
    pub http_proxy: Option<String>,
    /// Proxy URL for HTTPS requests (supports http, https, socks5, socks5h).
    #[serde(default)]
    pub https_proxy: Option<String>,
    /// Fallback proxy URL for all schemes.
    #[serde(default)]
    pub all_proxy: Option<String>,
    /// No-proxy bypass list. Same format as NO_PROXY.
    #[serde(default)]
    pub no_proxy: Vec<String>,
    /// Proxy application scope.
    #[serde(default)]
    pub scope: ProxyScope,
    /// Service selectors used when scope = "services".
    #[serde(default)]
    pub services: Vec<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            http_proxy: None,
            https_proxy: None,
            all_proxy: None,
            no_proxy: Vec::new(),
            scope: ProxyScope::Zeroclaw,
            services: Vec::new(),
        }
    }
}

impl ProxyConfig {
    /// All service keys the proxy can be scoped to.
    pub fn supported_service_keys() -> &'static [&'static str] {
        SUPPORTED_PROXY_SERVICE_KEYS
    }

    /// All service selectors the proxy can be scoped to.
    pub fn supported_service_selectors() -> &'static [&'static str] {
        SUPPORTED_PROXY_SERVICE_SELECTORS
    }

    /// `true` when any of `http_proxy` / `https_proxy` / `all_proxy` is set.
    pub fn has_any_proxy_url(&self) -> bool {
        normalize_proxy_url_option(self.http_proxy.as_deref()).is_some()
            || normalize_proxy_url_option(self.https_proxy.as_deref()).is_some()
            || normalize_proxy_url_option(self.all_proxy.as_deref()).is_some()
    }

    /// The `services` list after normalization (trim, dedupe, lowercase).
    pub fn normalized_services(&self) -> Vec<String> {
        normalize_service_list(self.services.clone())
    }

    /// The `no_proxy` list after normalization (trim, dedupe, lowercase).
    pub fn normalized_no_proxy(&self) -> Vec<String> {
        normalize_no_proxy_list(self.no_proxy.clone())
    }

    /// Validate proxy URLs and service selectors, bailing on invalid values.
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("http_proxy", self.http_proxy.as_deref()),
            ("https_proxy", self.https_proxy.as_deref()),
            ("all_proxy", self.all_proxy.as_deref()),
        ] {
            if let Some(url) = normalize_proxy_url_option(value) {
                validate_proxy_url(field, &url)?;
            }
        }

        for selector in self.normalized_services() {
            if !is_supported_proxy_service_selector(&selector) {
                anyhow::bail!(
                    "Unsupported proxy service selector '{selector}'. Valid service keys: {}",
                    Self::supported_service_keys().join(", ")
                );
            }
        }

        if self.enabled && !self.has_any_proxy_url() {
            anyhow::bail!(
                "Proxy is enabled but no proxy URL is configured. Set at least one of http_proxy, https_proxy, or all_proxy"
            );
        }

        if self.enabled
            && self.scope == ProxyScope::Services
            && self.normalized_services().is_empty()
        {
            anyhow::bail!(
                "proxy.scope='services' requires a non-empty proxy.services list when proxy is enabled"
            );
        }

        Ok(())
    }

    /// `true` when this proxy config applies to the given service key.
    pub fn should_apply_to_service(&self, service_key: &str) -> bool {
        if !self.enabled {
            return false;
        }

        match self.scope {
            ProxyScope::Environment => false,
            ProxyScope::Zeroclaw => true,
            ProxyScope::Services => {
                let service_key = service_key.trim().to_ascii_lowercase();
                if service_key.is_empty() {
                    return false;
                }

                self.normalized_services()
                    .iter()
                    .any(|selector| service_selector_matches(selector, &service_key))
            }
        }
    }

    /// Apply proxy settings to a `reqwest` builder for the given service.
    pub fn apply_to_reqwest_builder(
        &self,
        mut builder: reqwest::ClientBuilder,
        service_key: &str,
    ) -> reqwest::ClientBuilder {
        if !self.should_apply_to_service(service_key) {
            return builder;
        }

        let no_proxy = self.no_proxy_value();

        if let Some(url) = normalize_proxy_url_option(self.all_proxy.as_deref()) {
            match reqwest::Proxy::all(&url) {
                Ok(proxy) => {
                    builder = builder.proxy(apply_no_proxy(proxy, no_proxy.clone()));
                }
                Err(error) => {
                    tracing::warn!(
                        proxy_url = %url,
                        service_key,
                        "Ignoring invalid all_proxy URL: {error}"
                    );
                }
            }
        }

        if let Some(url) = normalize_proxy_url_option(self.http_proxy.as_deref()) {
            match reqwest::Proxy::http(&url) {
                Ok(proxy) => {
                    builder = builder.proxy(apply_no_proxy(proxy, no_proxy.clone()));
                }
                Err(error) => {
                    tracing::warn!(
                        proxy_url = %url,
                        service_key,
                        "Ignoring invalid http_proxy URL: {error}"
                    );
                }
            }
        }

        if let Some(url) = normalize_proxy_url_option(self.https_proxy.as_deref()) {
            match reqwest::Proxy::https(&url) {
                Ok(proxy) => {
                    builder = builder.proxy(apply_no_proxy(proxy, no_proxy));
                }
                Err(error) => {
                    tracing::warn!(
                        proxy_url = %url,
                        service_key,
                        "Ignoring invalid https_proxy URL: {error}"
                    );
                }
            }
        }

        builder
    }

    /// Export proxy settings into the process environment (`HTTP_PROXY`, ...).
    pub fn apply_to_process_env(&self) {
        set_proxy_env_pair("HTTP_PROXY", self.http_proxy.as_deref());
        set_proxy_env_pair("HTTPS_PROXY", self.https_proxy.as_deref());
        set_proxy_env_pair("ALL_PROXY", self.all_proxy.as_deref());

        let no_proxy_joined = {
            let list = self.normalized_no_proxy();
            (!list.is_empty()).then(|| list.join(","))
        };
        set_proxy_env_pair("NO_PROXY", no_proxy_joined.as_deref());
    }

    /// Clear proxy environment variables previously set via [`ProxyConfig::apply_to_process_env`].
    pub fn clear_process_env() {
        clear_proxy_env_pair("HTTP_PROXY");
        clear_proxy_env_pair("HTTPS_PROXY");
        clear_proxy_env_pair("ALL_PROXY");
        clear_proxy_env_pair("NO_PROXY");
    }

    fn no_proxy_value(&self) -> Option<reqwest::NoProxy> {
        let joined = {
            let list = self.normalized_no_proxy();
            (!list.is_empty()).then(|| list.join(","))
        };
        joined.as_deref().and_then(reqwest::NoProxy::from_string)
    }
}

pub(crate) fn apply_no_proxy(
    proxy: reqwest::Proxy,
    no_proxy: Option<reqwest::NoProxy>,
) -> reqwest::Proxy {
    proxy.no_proxy(no_proxy)
}

pub(crate) fn normalize_proxy_url_option(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn normalize_no_proxy_list(values: Vec<String>) -> Vec<String> {
    normalize_comma_values(values)
}

pub(crate) fn is_supported_proxy_service_selector(selector: &str) -> bool {
    if SUPPORTED_PROXY_SERVICE_KEYS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(selector))
    {
        return true;
    }

    SUPPORTED_PROXY_SERVICE_SELECTORS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(selector))
}

pub(crate) fn service_selector_matches(selector: &str, service_key: &str) -> bool {
    if selector == service_key {
        return true;
    }

    if let Some(prefix) = selector.strip_suffix(".*") {
        return service_key.starts_with(prefix)
            && service_key
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('.'));
    }

    false
}

pub(crate) fn validate_proxy_url(field: &str, url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)
        .with_context(|| format!("Invalid {field} URL: '{url}' is not a valid URL"))?;

    match parsed.scheme() {
        "http" | "https" | "socks5" | "socks5h" | "socks" => {}
        scheme => {
            anyhow::bail!(
                "Invalid {field} URL scheme '{scheme}'. Allowed: http, https, socks5, socks5h, socks"
            );
        }
    }

    if parsed.host_str().is_none() {
        anyhow::bail!("Invalid {field} URL: host is required");
    }

    Ok(())
}

pub(crate) fn set_proxy_env_pair(key: &str, value: Option<&str>) {
    let lowercase_key = key.to_ascii_lowercase();
    if let Some(value) = value.and_then(|candidate| normalize_proxy_url_option(Some(candidate))) {
        // SAFETY: no production path calls this on a live runtime; startup calls it from
        // `apply_startup_proxy` before the tokio runtime is built (BUGS.md RT-6b).
        unsafe {
            std::env::set_var(key, &value);
            std::env::set_var(lowercase_key, value);
        }
    } else {
        // SAFETY: no production path calls this on a live runtime; startup calls it from
        // `apply_startup_proxy` before the tokio runtime is built (BUGS.md RT-6b).
        unsafe {
            std::env::remove_var(key);
            std::env::remove_var(lowercase_key);
        }
    }
}

pub(crate) fn clear_proxy_env_pair(key: &str) {
    // SAFETY: no production path calls this on a live runtime; startup calls it from
    // `apply_startup_proxy` before the tokio runtime is built (BUGS.md RT-6b).
    unsafe {
        std::env::remove_var(key);
        std::env::remove_var(key.to_ascii_lowercase());
    }
}

pub(crate) fn runtime_proxy_state() -> &'static RwLock<ProxyConfig> {
    RUNTIME_PROXY_CONFIG.get_or_init(|| RwLock::new(ProxyConfig::default()))
}

pub(crate) fn runtime_proxy_client_cache() -> &'static RwLock<HashMap<String, reqwest::Client>> {
    RUNTIME_PROXY_CLIENT_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub(crate) fn clear_runtime_proxy_client_cache() {
    match runtime_proxy_client_cache().write() {
        Ok(mut guard) => {
            guard.clear();
        }
        Err(poisoned) => {
            poisoned.into_inner().clear();
        }
    }
}

pub(crate) fn runtime_proxy_cache_key(
    service_key: &str,
    timeout_secs: Option<u64>,
    connect_timeout_secs: Option<u64>,
) -> String {
    format!(
        "{}|timeout={}|connect_timeout={}",
        service_key.trim().to_ascii_lowercase(),
        timeout_secs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        connect_timeout_secs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

pub(crate) fn runtime_proxy_cached_client(cache_key: &str) -> Option<reqwest::Client> {
    match runtime_proxy_client_cache().read() {
        Ok(guard) => guard.get(cache_key).cloned(),
        Err(poisoned) => poisoned.into_inner().get(cache_key).cloned(),
    }
}

pub(crate) fn set_runtime_proxy_cached_client(cache_key: String, client: reqwest::Client) {
    match runtime_proxy_client_cache().write() {
        Ok(mut guard) => {
            guard.insert(cache_key, client);
        }
        Err(poisoned) => {
            poisoned.into_inner().insert(cache_key, client);
        }
    }
}

/// Store the runtime proxy configuration, invalidating the client cache.
pub fn set_runtime_proxy_config(config: ProxyConfig) {
    match runtime_proxy_state().write() {
        Ok(mut guard) => {
            *guard = config;
        }
        Err(poisoned) => {
            *poisoned.into_inner() = config;
        }
    }

    clear_runtime_proxy_client_cache();
}

/// Return the current runtime proxy configuration (defaults when unset).
pub fn runtime_proxy_config() -> ProxyConfig {
    match runtime_proxy_state().read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Apply the runtime proxy configuration to a `reqwest` builder.
pub fn apply_runtime_proxy_to_builder(
    builder: reqwest::ClientBuilder,
    service_key: &str,
) -> reqwest::ClientBuilder {
    runtime_proxy_config().apply_to_reqwest_builder(builder, service_key)
}

/// Build (and cache) a proxied `reqwest` client for the given service.
pub fn build_runtime_proxy_client(service_key: &str) -> reqwest::Client {
    let cache_key = runtime_proxy_cache_key(service_key, None, None);
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let builder = apply_runtime_proxy_to_builder(reqwest::Client::builder(), service_key);
    let client = builder.build().unwrap_or_else(|error| {
        tracing::warn!(service_key, "Failed to build proxied client: {error}");
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

/// Build (and cache) a proxied `reqwest` client with explicit timeouts.
pub fn build_runtime_proxy_client_with_timeouts(
    service_key: &str,
    timeout_secs: u64,
    connect_timeout_secs: u64,
) -> reqwest::Client {
    let cache_key =
        runtime_proxy_cache_key(service_key, Some(timeout_secs), Some(connect_timeout_secs));
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .connect_timeout(std::time::Duration::from_secs(connect_timeout_secs));
    let builder = apply_runtime_proxy_to_builder(builder, service_key);
    let client = builder.build().unwrap_or_else(|error| {
        tracing::warn!(
            service_key,
            "Failed to build proxied timeout client: {error}"
        );
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

/// Build an HTTP client for a channel, pinning the API host to a fallback IP
/// (hermes `fallback_ips` parity: keeps the channel working when DNS for the
/// API host is broken or poisoned). Uses an explicit per-channel proxy URL
/// when configured, else the global runtime proxy.
pub fn build_channel_proxy_client_resolved(
    service_key: &str,
    proxy_url: Option<&str>,
    resolve_host: &str,
    resolve_ip: &str,
) -> reqwest::Client {
    let ip: std::net::IpAddr = match resolve_ip.trim().parse() {
        Ok(ip) => ip,
        Err(_) => {
            tracing::warn!(service_key, "Ignoring invalid fallback IP: {resolve_ip}");
            return build_channel_proxy_client(service_key, proxy_url);
        }
    };
    match normalize_proxy_url_option(proxy_url) {
        Some(url) => build_explicit_proxy_client_resolved(service_key, &url, resolve_host, ip),
        None => build_runtime_proxy_client_resolved(service_key, resolve_host, ip),
    }
}

/// Runtime-proxy client variant that additionally pins `resolve_host` to `ip`.
pub(crate) fn build_runtime_proxy_client_resolved(
    service_key: &str,
    resolve_host: &str,
    ip: std::net::IpAddr,
) -> reqwest::Client {
    let cache_key = format!(
        "runtime|{}|resolve={}:{}",
        service_key.trim().to_ascii_lowercase(),
        resolve_host,
        ip
    );
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let addr = std::net::SocketAddr::new(ip, 443);
    let builder = apply_runtime_proxy_to_builder(reqwest::Client::builder(), service_key)
        .resolve(resolve_host, addr);
    let client = builder.build().unwrap_or_else(|error| {
        tracing::warn!(
            service_key,
            "Failed to build resolved proxied client: {error}"
        );
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

/// Explicit-proxy client variant that additionally pins `resolve_host` to `ip`.
pub(crate) fn build_explicit_proxy_client_resolved(
    service_key: &str,
    proxy_url: &str,
    resolve_host: &str,
    ip: std::net::IpAddr,
) -> reqwest::Client {
    let cache_key = format!(
        "explicit|{}|{}|resolve={}:{}",
        service_key.trim().to_ascii_lowercase(),
        proxy_url,
        resolve_host,
        ip
    );
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let addr = std::net::SocketAddr::new(ip, 443);
    let mut builder = reqwest::Client::builder().resolve(resolve_host, addr);
    builder = apply_explicit_proxy_to_builder(builder, service_key, proxy_url);
    let client = builder.build().unwrap_or_else(|error| {
        tracing::warn!(
            service_key,
            proxy_url,
            "Failed to build resolved channel proxy client: {error}"
        );
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

/// Build an HTTP client for a channel, using an explicit per-channel proxy URL
/// when configured.  Falls back to the global runtime proxy when `proxy_url` is
/// `None` or empty.
pub fn build_channel_proxy_client(service_key: &str, proxy_url: Option<&str>) -> reqwest::Client {
    match normalize_proxy_url_option(proxy_url) {
        Some(url) => build_explicit_proxy_client(service_key, &url, None, None),
        None => build_runtime_proxy_client(service_key),
    }
}

/// Build an HTTP client for a channel with custom timeouts, using an explicit
/// per-channel proxy URL when configured.  Falls back to the global runtime
/// proxy when `proxy_url` is `None` or empty.
pub fn build_channel_proxy_client_with_timeouts(
    service_key: &str,
    proxy_url: Option<&str>,
    timeout_secs: u64,
    connect_timeout_secs: u64,
) -> reqwest::Client {
    match normalize_proxy_url_option(proxy_url) {
        Some(url) => build_explicit_proxy_client(
            service_key,
            &url,
            Some(timeout_secs),
            Some(connect_timeout_secs),
        ),
        None => build_runtime_proxy_client_with_timeouts(
            service_key,
            timeout_secs,
            connect_timeout_secs,
        ),
    }
}

/// Apply an explicit proxy URL to a `reqwest::ClientBuilder`, returning the
/// modified builder.  Used by channels that specify a per-channel `proxy_url`.
pub fn apply_channel_proxy_to_builder(
    builder: reqwest::ClientBuilder,
    service_key: &str,
    proxy_url: Option<&str>,
) -> reqwest::ClientBuilder {
    match normalize_proxy_url_option(proxy_url) {
        Some(url) => apply_explicit_proxy_to_builder(builder, service_key, &url),
        None => apply_runtime_proxy_to_builder(builder, service_key),
    }
}

/// Build a client with a single explicit proxy URL (http+https via `Proxy::all`).
pub(crate) fn build_explicit_proxy_client(
    service_key: &str,
    proxy_url: &str,
    timeout_secs: Option<u64>,
    connect_timeout_secs: Option<u64>,
) -> reqwest::Client {
    let cache_key = format!(
        "explicit|{}|{}|timeout={}|connect_timeout={}",
        service_key.trim().to_ascii_lowercase(),
        proxy_url,
        timeout_secs
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_string()),
        connect_timeout_secs
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_string()),
    );
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let mut builder = reqwest::Client::builder();
    if let Some(t) = timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(t));
    }
    if let Some(ct) = connect_timeout_secs {
        builder = builder.connect_timeout(std::time::Duration::from_secs(ct));
    }
    builder = apply_explicit_proxy_to_builder(builder, service_key, proxy_url);
    let client = builder.build().unwrap_or_else(|error| {
        tracing::warn!(
            service_key,
            proxy_url,
            "Failed to build channel proxy client: {error}"
        );
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

/// Apply a single explicit proxy URL to a builder via `Proxy::all`.
pub(crate) fn apply_explicit_proxy_to_builder(
    mut builder: reqwest::ClientBuilder,
    service_key: &str,
    proxy_url: &str,
) -> reqwest::ClientBuilder {
    match reqwest::Proxy::all(proxy_url) {
        Ok(proxy) => {
            builder = builder.proxy(proxy);
        }
        Err(error) => {
            tracing::warn!(
                proxy_url,
                service_key,
                "Ignoring invalid channel proxy_url: {error}"
            );
        }
    }
    builder
}
