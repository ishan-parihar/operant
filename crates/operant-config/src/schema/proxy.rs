//! `proxy` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::{Context, Result};
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use super::*;

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
    "tool.composio",
    "tool.http_request",
    "tool.pushover",
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

pub(crate) static RUNTIME_PROXY_CONFIG: OnceLock<RwLock<ProxyConfig>> = OnceLock::new();
pub(crate) static RUNTIME_PROXY_CLIENT_CACHE: OnceLock<RwLock<HashMap<String, reqwest::Client>>> =
    OnceLock::new();

// ── Top-level config ──────────────────────────────────────────────

/// Top-level Operant configuration, loaded from `config.toml`.
///
/// Resolution order: `OPERANT_WORKSPACE` env → `active_workspace.toml` marker → `~/.operant/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct Config {
    /// Workspace directory - computed from home, not serialized
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    /// Path to config.toml - computed from home, not serialized
    #[serde(skip)]
    pub config_path: PathBuf,
    /// Config file schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    /// Provider configuration (`[providers]`).
    #[serde(default)]
    #[nested]
    pub providers: crate::providers::ProvidersConfig,

    /// Observability backend configuration (`[observability]`).
    #[serde(default)]
    #[nested]
    pub observability: ObservabilityConfig,

    /// Autonomy and security policy configuration (`[autonomy]`).
    #[serde(default)]
    #[nested]
    pub autonomy: AutonomyConfig,

    /// Trust scoring and regression detection configuration (`[trust]`).
    #[serde(default)]
    #[nested]
    pub trust: crate::scattered_types::TrustConfig,

    /// Security subsystem configuration (`[security]`).
    #[serde(default)]
    #[nested]
    pub security: SecurityConfig,

    /// Backup tool configuration (`[backup]`).
    #[serde(default)]
    #[nested]
    pub backup: BackupConfig,

    /// Data retention and purge configuration (`[data_retention]`).
    #[serde(default)]
    #[nested]
    pub data_retention: DataRetentionConfig,

    /// Cloud transformation accelerator configuration (`[cloud_ops]`).
    #[serde(default)]
    #[nested]
    pub cloud_ops: CloudOpsConfig,

    /// Conversational AI agent builder configuration (`[conversational_ai]`).
    ///
    /// Experimental / future feature — not yet wired into the agent runtime.
    /// Omitted from generated config files when disabled (the default).
    /// Existing configs that already contain this section will continue to
    /// deserialize correctly thanks to `#[serde(default)]`.
    #[serde(default, skip_serializing_if = "ConversationalAiConfig::is_disabled")]
    #[nested]
    pub conversational_ai: ConversationalAiConfig,

    /// Managed cybersecurity service configuration (`[security_ops]`).
    #[serde(default)]
    #[nested]
    pub security_ops: SecurityOpsConfig,

    /// Runtime adapter configuration (`[runtime]`). Controls native vs Docker execution.
    #[serde(default)]
    #[nested]
    pub runtime: RuntimeConfig,

    /// Reliability settings: retries, fallback providers, backoff (`[reliability]`).
    #[serde(default)]
    #[nested]
    pub reliability: ReliabilityConfig,

    /// Scheduler configuration for periodic task execution (`[scheduler]`).
    #[serde(default)]
    #[nested]
    pub scheduler: SchedulerConfig,

    /// Agent orchestration settings (`[agent]`).
    #[serde(default)]
    #[nested]
    pub agent: AgentConfig,

    /// Pacing controls for slow/local LLM workloads (`[pacing]`).
    #[serde(default)]
    #[nested]
    pub pacing: PacingConfig,

    /// Skills loading and community repository behavior (`[skills]`).
    #[serde(default)]
    #[nested]
    pub skills: SkillsConfig,

    /// Pipeline tool configuration (`[pipeline]`).
    #[serde(default)]
    #[nested]
    pub pipeline: PipelineConfig,

    /// Automatic query classification — maps user messages to model hints.
    #[serde(default)]
    #[nested]
    pub query_classification: QueryClassificationConfig,

    /// Heartbeat configuration for periodic health pings (`[heartbeat]`).
    #[serde(default)]
    #[nested]
    pub heartbeat: HeartbeatConfig,

    /// Cron job configuration (`[cron]`).
    #[serde(default)]
    #[nested]
    pub cron: CronConfig,

    /// Channel configurations: Telegram, Discord, Slack, etc. (`[channels]`).
    #[serde(default, alias = "channels_config")]
    #[nested]
    pub channels: ChannelsConfig,

    /// Memory backend configuration: sqlite, markdown, embeddings (`[memory]`).
    #[serde(default)]
    #[nested]
    pub memory: MemoryConfig,

    /// Persistent storage provider configuration (`[storage]`).
    #[serde(default)]
    #[nested]
    pub storage: StorageConfig,

    /// Tunnel configuration for exposing the gateway publicly (`[tunnel]`).
    #[serde(default)]
    #[nested]
    pub tunnel: TunnelConfig,

    /// Gateway server configuration: host, port, pairing, rate limits (`[gateway]`).
    #[serde(default)]
    #[nested]
    pub gateway: GatewayConfig,

    /// Composio managed OAuth tools integration (`[composio]`).
    #[serde(default)]
    #[nested]
    pub composio: ComposioConfig,

    /// Microsoft 365 Graph API integration (`[microsoft365]`).
    #[serde(default)]
    #[nested]
    pub microsoft365: Microsoft365Config,

    /// Secrets encryption configuration (`[secrets]`).
    #[serde(default)]
    #[nested]
    pub secrets: SecretsConfig,

    /// Browser automation configuration (`[browser]`).
    #[serde(default)]
    #[nested]
    pub browser: BrowserConfig,

    /// Browser delegation configuration (`[browser_delegate]`).
    ///
    /// Delegates browser-based tasks to a browser-capable CLI subprocess (e.g.
    /// Claude Code with `claude-in-chrome` MCP tools). Useful for interacting
    /// with corporate web apps (Teams, Outlook, Jira, Confluence) that lack
    /// direct API access. A persistent Chrome profile can be configured so SSO
    /// sessions survive across invocations.
    ///
    /// Fields:
    /// - `enabled` (`bool`, default `false`) — enable the browser delegation tool.
    /// - `cli_binary` (`String`, default `"claude"`) — CLI binary to spawn for browser tasks.
    /// - `chrome_profile_dir` (`String`, default `""`) — Chrome user-data directory for
    ///   persistent SSO sessions. When empty, a fresh profile is used each invocation.
    /// - `allowed_domains` (`Vec<String>`, default `[]`) — allowlist of domains the browser
    ///   may navigate to. Empty means all non-blocked domains are permitted.
    /// - `blocked_domains` (`Vec<String>`, default `[]`) — denylist of domains. Blocked
    ///   domains take precedence over allowed domains.
    /// - `task_timeout_secs` (`u64`, default `120`) — per-task timeout in seconds.
    ///
    /// Compatibility: additive and disabled by default; existing configs remain valid when omitted.
    /// Rollback/migration: remove `[browser_delegate]` or keep `enabled = false` to disable.
    #[serde(default)]
    #[nested]
    pub browser_delegate: crate::scattered_types::BrowserDelegateConfig,

    /// HTTP request tool configuration (`[http_request]`).
    #[serde(default)]
    #[nested]
    pub http_request: HttpRequestConfig,

    /// Multimodal (image) handling configuration (`[multimodal]`).
    #[serde(default)]
    #[nested]
    pub multimodal: MultimodalConfig,

    /// Automatic media understanding pipeline (`[media_pipeline]`).
    #[serde(default)]
    #[nested]
    pub media_pipeline: MediaPipelineConfig,

    /// Web fetch tool configuration (`[web_fetch]`).
    #[serde(default)]
    #[nested]
    pub web_fetch: WebFetchConfig,

    /// Link enricher configuration (`[link_enricher]`).
    #[serde(default)]
    #[nested]
    pub link_enricher: LinkEnricherConfig,

    /// Text browser tool configuration (`[text_browser]`).
    #[serde(default)]
    #[nested]
    pub text_browser: TextBrowserConfig,

    /// Web search tool configuration (`[web_search]`).
    #[serde(default)]
    #[nested]
    pub web_search: WebSearchConfig,

    /// Project delivery intelligence configuration (`[project_intel]`).
    #[serde(default)]
    #[nested]
    pub project_intel: ProjectIntelConfig,

    /// Google Workspace CLI (`gws`) tool configuration (`[google_workspace]`).
    #[serde(default)]
    #[nested]
    pub google_workspace: GoogleWorkspaceConfig,

    /// Proxy configuration for outbound HTTP/HTTPS/SOCKS5 traffic (`[proxy]`).
    #[serde(default)]
    #[nested]
    pub proxy: ProxyConfig,

    /// Identity format configuration: OpenClaw or AIEOS (`[identity]`).
    #[serde(default)]
    #[nested]
    pub identity: IdentityConfig,

    /// Cost tracking and budget enforcement configuration (`[cost]`).
    #[serde(default)]
    #[nested]
    pub cost: CostConfig,

    /// Peripheral board configuration for hardware integration (`[peripherals]`).
    #[serde(default)]
    #[nested]
    pub peripherals: PeripheralsConfig,

    /// Delegate tool global default configuration (`[delegate]`).
    #[serde(default)]
    #[nested]
    pub delegate: DelegateToolConfig,

    /// Delegate agent configurations for multi-agent workflows.
    #[serde(default)]
    #[nested]
    pub agents: HashMap<String, DelegateAgentConfig>,

    /// Swarm configurations for multi-agent orchestration.
    #[serde(default)]
    pub swarms: HashMap<String, SwarmConfig>,

    /// Hooks configuration (lifecycle hooks and built-in hook toggles).
    #[serde(default)]
    #[nested]
    pub hooks: HooksConfig,

    /// Hardware configuration (wizard-driven physical world setup).
    #[serde(default)]
    #[nested]
    pub hardware: HardwareConfig,

    /// Voice transcription configuration (Whisper API via Groq).
    #[serde(default)]
    #[nested]
    pub transcription: TranscriptionConfig,

    /// Text-to-Speech configuration (`[tts]`).
    #[serde(default)]
    #[nested]
    pub tts: TtsConfig,

    /// External MCP server connections (`[mcp]`).
    #[serde(default, alias = "mcpServers")]
    #[nested]
    pub mcp: McpConfig,

    /// Dynamic node discovery configuration (`[nodes]`).
    #[serde(default)]
    #[nested]
    pub nodes: NodesConfig,

    /// Multi-client workspace isolation configuration (`[workspace]`).
    #[serde(default)]
    #[nested]
    pub workspace: WorkspaceConfig,

    /// Meta-state for `operant onboard` (which sections the user has
    /// already walked through). Not user-facing config (`[onboard_state]`).
    #[serde(default)]
    #[nested]
    pub onboard_state: OnboardStateConfig,

    /// Notion integration configuration (`[notion]`).
    #[serde(default)]
    #[nested]
    pub notion: NotionConfig,

    /// Jira integration configuration (`[jira]`).
    #[serde(default)]
    #[nested]
    pub jira: JiraConfig,

    /// Secure inter-node transport configuration (`[node_transport]`).
    #[serde(default)]
    #[nested]
    pub node_transport: NodeTransportConfig,

    /// Knowledge graph configuration (`[knowledge]`).
    #[serde(default)]
    #[nested]
    pub knowledge: KnowledgeConfig,

    /// LinkedIn integration configuration (`[linkedin]`).
    #[serde(default)]
    #[nested]
    pub linkedin: LinkedInConfig,

    /// Standalone image generation tool configuration (`[image_gen]`).
    #[serde(default)]
    #[nested]
    pub image_gen: ImageGenConfig,

    /// Plugin system configuration (`[plugins]`).
    #[serde(default)]
    #[nested]
    pub plugins: PluginsConfig,

    /// Locale for tool descriptions (e.g. `"en"`, `"zh-CN"`).
    ///
    /// When set, tool descriptions shown in system prompts are loaded from
    /// Fluent `.ftl` locale files. Falls back to embedded English, then to
    /// hardcoded descriptions.
    ///
    /// If omitted or empty, the locale is auto-detected from `OPERANT_LOCALE`,
    /// `LANG`, or `LC_ALL` environment variables (defaulting to `"en"`).
    #[serde(default)]
    pub locale: Option<String>,

    /// Verifiable Intent (VI) credential verification and issuance (`[verifiable_intent]`).
    #[serde(default)]
    #[nested]
    pub verifiable_intent: VerifiableIntentConfig,

    /// Claude Code tool configuration (`[claude_code]`).
    #[serde(default)]
    #[nested]
    pub claude_code: ClaudeCodeConfig,

    /// Claude Code task runner with Slack progress and SSH session handoff (`[claude_code_runner]`).
    #[serde(default)]
    #[nested]
    pub claude_code_runner: ClaudeCodeRunnerConfig,

    /// Codex CLI tool configuration (`[codex_cli]`).
    #[serde(default)]
    #[nested]
    pub codex_cli: CodexCliConfig,

    /// Gemini CLI tool configuration (`[gemini_cli]`).
    #[serde(default)]
    #[nested]
    pub gemini_cli: GeminiCliConfig,

    /// OpenCode CLI tool configuration (`[opencode_cli]`).
    #[serde(default)]
    #[nested]
    pub opencode_cli: OpenCodeCliConfig,

    /// Standard Operating Procedures engine configuration (`[sop]`).
    #[serde(default)]
    #[nested]
    pub sop: SopConfig,

    /// Shell tool configuration (`[shell_tool]`).
    #[serde(default)]
    #[nested]
    pub shell_tool: ShellToolConfig,

    /// Escalation routing configuration (`[escalation]`).
    #[serde(default)]
    #[nested]
    pub escalation: EscalationConfig,
}

// ── Proxy ───────────────────────────────────────────────────────

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
                    "Unsupported proxy service selector '{selector}'. Use tool `proxy_config` action `list_services` for valid values"
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
        // SAFETY: called during single-threaded config init before async runtime starts.
        unsafe {
            std::env::set_var(key, &value);
            std::env::set_var(lowercase_key, value);
        }
    } else {
        // SAFETY: called during single-threaded config init before async runtime starts.
        unsafe {
            std::env::remove_var(key);
            std::env::remove_var(lowercase_key);
        }
    }
}

pub(crate) fn clear_proxy_env_pair(key: &str) {
    // SAFETY: called during single-threaded config init before async runtime starts.
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

/// Connect a WebSocket through the configured proxy (if any).
///
/// When no proxy applies, this is a thin wrapper around
/// `tokio_tungstenite::connect_async`.  When a proxy is active the
/// function tunnels the TCP connection through the proxy before
/// performing the WebSocket upgrade.
///
/// `service_key` is the proxy-service selector (e.g. `"channel.discord"`).
/// `channel_proxy_url` is the optional per-channel proxy override.
pub async fn ws_connect_with_proxy(
    ws_url: &str,
    service_key: &str,
    channel_proxy_url: Option<&str>,
) -> anyhow::Result<(
    ProxiedWsStream,
    tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
)> {
    let proxy_url = resolve_ws_proxy_url(service_key, ws_url, channel_proxy_url);

    match proxy_url {
        None => {
            // No proxy — establish TCP+TLS manually, wrap in BoxedIo, then
            // perform the WebSocket handshake over the wrapped stream.
            //
            // Previous implementation used `connect_async` followed by
            // `into_inner()` + `from_raw_socket` to normalize the return
            // type.  That pattern discards data already buffered by the
            // tungstenite frame codec, causing channels (Slack Socket Mode,
            // Discord, etc.) to silently miss the first frames sent by the
            // server and all subsequent events.
            use tokio::net::TcpStream;

            let target = reqwest::Url::parse(ws_url)
                .with_context(|| format!("Invalid WebSocket URL: {ws_url}"))?;
            let target_host = target
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("WebSocket URL has no host: {ws_url}"))?
                .to_string();
            let target_port = target
                .port_or_known_default()
                .unwrap_or(if target.scheme() == "wss" { 443 } else { 80 });

            let tcp = TcpStream::connect(format!("{target_host}:{target_port}"))
                .await
                .with_context(|| format!("TCP connect to {target_host}:{target_port}"))?;

            let is_secure = target.scheme() == "wss";
            let stream: BoxedIo = if is_secure {
                let mut root_store = rustls::RootCertStore::empty();
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let tls_config = std::sync::Arc::new(
                    rustls::ClientConfig::builder()
                        .with_root_certificates(root_store)
                        .with_no_client_auth(),
                );
                let connector = tokio_rustls::TlsConnector::from(tls_config);
                let server_name = rustls_pki_types::ServerName::try_from(target_host.clone())
                    .with_context(|| format!("Invalid TLS server name: {target_host}"))?;
                let tls_stream = connector
                    .connect(server_name, tcp)
                    .await
                    .with_context(|| format!("TLS handshake with {target_host}"))?;
                BoxedIo(Box::new(tls_stream))
            } else {
                BoxedIo(Box::new(tcp))
            };

            let default_port = if is_secure { 443 } else { 80 };
            let host_header = if target_port == default_port {
                target_host.clone()
            } else {
                format!("{target_host}:{target_port}")
            };

            let ws_request = tokio_tungstenite::tungstenite::http::Request::builder()
                .uri(ws_url)
                .header("Host", host_header)
                .header("Connection", "Upgrade")
                .header("Upgrade", "websocket")
                .header(
                    "Sec-WebSocket-Key",
                    tokio_tungstenite::tungstenite::handshake::client::generate_key(),
                )
                .header("Sec-WebSocket-Version", "13")
                .body(())
                .with_context(|| "Failed to build WebSocket upgrade request")?;

            let (ws_stream, response) =
                tokio_tungstenite::client_async(ws_request, stream)
                    .await
                    .with_context(|| format!("WebSocket handshake failed for {ws_url}"))?;

            Ok((ws_stream, response))
        }
        Some(proxy) => ws_connect_via_proxy(ws_url, &proxy).await,
    }
}

/// Establish a WebSocket connection tunnelled through the given proxy URL.
pub(crate) async fn ws_connect_via_proxy(
    ws_url: &str,
    proxy_url: &str,
) -> anyhow::Result<(
    ProxiedWsStream,
    tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt as _};
    use tokio::net::TcpStream;

    let target =
        reqwest::Url::parse(ws_url).with_context(|| format!("Invalid WebSocket URL: {ws_url}"))?;
    let target_host = target
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("WebSocket URL has no host: {ws_url}"))?
        .to_string();
    let target_port = target
        .port_or_known_default()
        .unwrap_or(if target.scheme() == "wss" { 443 } else { 80 });

    let proxy = reqwest::Url::parse(proxy_url)
        .with_context(|| format!("Invalid proxy URL: {proxy_url}"))?;

    let stream: BoxedIo = match proxy.scheme() {
        "socks5" | "socks5h" | "socks" => {
            let proxy_addr = format!(
                "{}:{}",
                proxy.host_str().unwrap_or("127.0.0.1"),
                proxy.port_or_known_default().unwrap_or(1080)
            );
            let target_addr = format!("{target_host}:{target_port}");
            let socks_stream = if proxy.username().is_empty() {
                tokio_socks::tcp::Socks5Stream::connect(proxy_addr.as_str(), target_addr.as_str())
                    .await
                    .with_context(|| format!("SOCKS5 connect to {target_addr} via {proxy_addr}"))?
            } else {
                let password = proxy.password().unwrap_or("");
                tokio_socks::tcp::Socks5Stream::connect_with_password(
                    proxy_addr.as_str(),
                    target_addr.as_str(),
                    proxy.username(),
                    password,
                )
                .await
                .with_context(|| format!("SOCKS5 auth connect to {target_addr} via {proxy_addr}"))?
            };
            let tcp: TcpStream = socks_stream.into_inner();
            BoxedIo(Box::new(tcp))
        }
        "http" | "https" => {
            let proxy_host = proxy.host_str().unwrap_or("127.0.0.1");
            let proxy_port = proxy.port_or_known_default().unwrap_or(8080);
            let proxy_addr = format!("{proxy_host}:{proxy_port}");

            let mut tcp = TcpStream::connect(&proxy_addr)
                .await
                .with_context(|| format!("TCP connect to HTTP proxy {proxy_addr}"))?;

            // Send HTTP CONNECT request.
            let connect_req = format!(
                "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n\r\n"
            );
            tcp.write_all(connect_req.as_bytes()).await?;

            // Read the response (we only need the status line).
            let mut buf = vec![0u8; 4096];
            let mut total = 0usize;
            loop {
                let n = tcp.read(&mut buf[total..]).await?;
                if n == 0 {
                    anyhow::bail!("HTTP CONNECT proxy closed connection before response");
                }
                total += n;
                // Look for end of HTTP headers.
                if let Some(pos) = find_header_end(&buf[..total]) {
                    let status_line = std::str::from_utf8(&buf[..pos])
                        .unwrap_or("")
                        .lines()
                        .next()
                        .unwrap_or("");
                    if !status_line.contains("200") {
                        anyhow::bail!(
                            "HTTP CONNECT proxy returned non-200 response: {status_line}"
                        );
                    }
                    break;
                }
                if total >= buf.len() {
                    anyhow::bail!("HTTP CONNECT proxy response too large");
                }
            }

            BoxedIo(Box::new(tcp))
        }
        scheme => {
            anyhow::bail!("Unsupported proxy scheme '{scheme}' for WebSocket connections");
        }
    };

    // If the target is wss://, wrap in TLS.
    let is_secure = target.scheme() == "wss";
    let stream: BoxedIo = if is_secure {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls_config = std::sync::Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );
        let connector = tokio_rustls::TlsConnector::from(tls_config);
        let server_name = rustls_pki_types::ServerName::try_from(target_host.clone())
            .with_context(|| format!("Invalid TLS server name: {target_host}"))?;

        // `stream` is `BoxedIo` — we need a concrete `AsyncRead + AsyncWrite`
        // for `TlsConnector::connect`.  Since `BoxedIo` already satisfies
        // those bounds we can pass it directly.
        let tls_stream = connector
            .connect(server_name, stream)
            .await
            .with_context(|| format!("TLS handshake with {target_host}"))?;
        BoxedIo(Box::new(tls_stream))
    } else {
        stream
    };

    // Perform the WebSocket client handshake over the tunnelled stream.
    let ws_request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(ws_url)
        .header("Host", format!("{target_host}:{target_port}"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .header("Sec-WebSocket-Version", "13")
        .body(())
        .with_context(|| "Failed to build WebSocket upgrade request")?;

    let (ws_stream, response) = tokio_tungstenite::client_async(ws_request, stream)
        .await
        .with_context(|| format!("WebSocket handshake failed for {ws_url}"))?;

    Ok((ws_stream, response))
}

pub(crate) fn parse_proxy_scope(raw: &str) -> Option<ProxyScope> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "environment" | "env" => Some(ProxyScope::Environment),
        "operant" | "internal" | "core" => Some(ProxyScope::Zeroclaw),
        "services" | "service" => Some(ProxyScope::Services),
        _ => None,
    }
}

pub(crate) fn parse_proxy_enabled(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

// ── Proxy-aware WebSocket connect ────────────────────────────────
//
// `tokio_tungstenite::connect_async` does not honour proxy settings.
// The helpers below resolve the effective proxy URL for a given service
// key and, when a proxy is active, establish a tunnelled TCP connection
// (HTTP CONNECT for http/https proxies, SOCKS5 for socks5/socks5h)
// before handing the stream to `tokio_tungstenite` for the WebSocket
// handshake.

/// Combined async IO trait for boxed WebSocket transport streams.
pub(crate) trait AsyncReadWrite:
    tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send
{
}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

/// A boxed async IO stream used when a WebSocket connection is tunnelled
/// through a proxy.  The concrete type varies depending on the proxy
/// kind (HTTP CONNECT vs SOCKS5) and the target scheme (ws vs wss).
///
/// We wrap in a newtype so we can implement `AsyncRead` and `AsyncWrite`
/// via delegation, since Rust trait objects cannot combine multiple
/// non-auto traits.
pub struct BoxedIo(pub(crate) Box<dyn AsyncReadWrite>);

impl tokio::io::AsyncRead for BoxedIo {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.0).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for BoxedIo {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut *self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.0).poll_shutdown(cx)
    }
}

impl Unpin for BoxedIo {}

/// Convenience alias for the WebSocket stream returned by the proxy-aware
/// connect helpers.
pub type ProxiedWsStream = tokio_tungstenite::WebSocketStream<BoxedIo>;

/// Resolve the effective proxy URL for a WebSocket connection to the
/// given `ws_url`, taking into account the per-channel `proxy_url`
/// override, the runtime proxy config, scope and no_proxy list.
pub(crate) fn resolve_ws_proxy_url(
    service_key: &str,
    ws_url: &str,
    channel_proxy_url: Option<&str>,
) -> Option<String> {
    // 1. Explicit per-channel proxy always wins.
    if let Some(url) = normalize_proxy_url_option(channel_proxy_url) {
        return Some(url);
    }

    // 2. Consult the runtime proxy config.
    let cfg = runtime_proxy_config();
    if !cfg.should_apply_to_service(service_key) {
        return None;
    }

    // Check the no_proxy list against the WebSocket target host.
    if let Ok(parsed) = reqwest::Url::parse(ws_url)
        && let Some(host) = parsed.host_str()
    {
        let no_proxy_entries = cfg.normalized_no_proxy();
        if !no_proxy_entries.is_empty() {
            let host_lower = host.to_ascii_lowercase();
            let matches_no_proxy = no_proxy_entries.iter().any(|entry| {
                let entry = entry.trim().to_ascii_lowercase();
                if entry == "*" {
                    return true;
                }
                if host_lower == entry {
                    return true;
                }
                // Support ".example.com" matching "foo.example.com"
                if let Some(suffix) = entry.strip_prefix('.') {
                    return host_lower.ends_with(suffix) || host_lower == suffix;
                }
                // Support "example.com" also matching "foo.example.com"
                host_lower.ends_with(&format!(".{entry}"))
            });
            if matches_no_proxy {
                return None;
            }
        }
    }

    // For wss:// prefer https_proxy, for ws:// prefer http_proxy, fall
    // back to all_proxy in both cases.
    let is_secure = ws_url.starts_with("wss://") || ws_url.starts_with("wss:");
    let preferred = if is_secure {
        normalize_proxy_url_option(cfg.https_proxy.as_deref())
    } else {
        normalize_proxy_url_option(cfg.http_proxy.as_deref())
    };
    preferred.or_else(|| normalize_proxy_url_option(cfg.all_proxy.as_deref()))
}

/// Find the `\r\n\r\n` boundary marking the end of HTTP headers.
pub(crate) fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}
