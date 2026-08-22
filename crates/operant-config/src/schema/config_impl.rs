//! `config_impl` — extracted verbatim from the former schema/core.rs monolith.
//! Re-exported from `schema` so every path is unchanged.

use super::*;
use crate::autonomy::AutonomyLevel;
use crate::domain_matcher::DomainMatcher;
use crate::provider_aliases::{is_glm_alias, is_zai_alias};
use crate::traits::{HasPropKind, PropKind};
use crate::validation_bail;
use anyhow::{Context, Result};
use directories::UserDirs;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tokio::fs::File;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

// ── Config impl ──────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        let home =
            UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
        let operant_dir = home.join(".operant");

        Self {
            workspace_dir: operant_dir.join("workspace"),
            config_path: operant_dir.join("config.toml"),
            schema_version: crate::migration::CURRENT_SCHEMA_VERSION,
            providers: crate::providers::ProvidersConfig::default(),
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            trust: crate::scattered_types::TrustConfig::default(),
            backup: BackupConfig::default(),
            data_retention: DataRetentionConfig::default(),
            cloud_ops: CloudOpsConfig::default(),
            conversational_ai: ConversationalAiConfig::default(),
            security: SecurityConfig::default(),
            security_ops: SecurityOpsConfig::default(),
            runtime: RuntimeConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            agent: AgentConfig::default(),
            pacing: PacingConfig::default(),
            skills: SkillsConfig::default(),
            pipeline: PipelineConfig::default(),
            heartbeat: HeartbeatConfig::default(),
            cron: CronConfig::default(),
            channels: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            microsoft365: Microsoft365Config::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            browser_delegate: crate::scattered_types::BrowserDelegateConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            media_pipeline: MediaPipelineConfig::default(),
            web_fetch: WebFetchConfig::default(),
            link_enricher: LinkEnricherConfig::default(),
            text_browser: TextBrowserConfig::default(),
            web_search: WebSearchConfig::default(),
            project_intel: ProjectIntelConfig::default(),
            google_workspace: GoogleWorkspaceConfig::default(),
            proxy: ProxyConfig::default(),
            identity: IdentityConfig::default(),
            cost: CostConfig::default(),
            peripherals: PeripheralsConfig::default(),
            delegate: DelegateToolConfig::default(),
            agents: HashMap::new(),
            swarms: HashMap::new(),
            hooks: HooksConfig::default(),
            hardware: HardwareConfig::default(),
            query_classification: QueryClassificationConfig::default(),
            transcription: TranscriptionConfig::default(),
            tts: TtsConfig::default(),
            mcp: McpConfig::default(),
            nodes: NodesConfig::default(),
            workspace: WorkspaceConfig::default(),
            onboard_state: OnboardStateConfig::default(),
            notion: NotionConfig::default(),
            jira: JiraConfig::default(),
            node_transport: NodeTransportConfig::default(),
            knowledge: KnowledgeConfig::default(),
            linkedin: LinkedInConfig::default(),
            image_gen: ImageGenConfig::default(),
            plugins: PluginsConfig::default(),
            locale: None,
            verifiable_intent: VerifiableIntentConfig::default(),
            claude_code: ClaudeCodeConfig::default(),
            claude_code_runner: ClaudeCodeRunnerConfig::default(),
            codex_cli: CodexCliConfig::default(),
            gemini_cli: GeminiCliConfig::default(),
            opencode_cli: OpenCodeCliConfig::default(),
            sop: SopConfig::default(),
            shell_tool: ShellToolConfig::default(),
            escalation: EscalationConfig::default(),
        }
    }
}

/// Resolve the current runtime config/workspace directories for onboarding flows.
///
/// This mirrors the same precedence used by `Config::load_or_init()`:
/// `OPERANT_CONFIG_DIR` > `OPERANT_WORKSPACE` > active workspace marker > defaults.
pub async fn resolve_runtime_dirs_for_onboarding() -> Result<(PathBuf, PathBuf)> {
    let (default_operant_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
    let (config_dir, workspace_dir, _) =
        resolve_runtime_config_dirs(&default_operant_dir, &default_workspace_dir).await?;
    Ok((config_dir, workspace_dir))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigResolutionSource {
    EnvConfigDir,
    EnvWorkspace,
    ActiveWorkspaceMarker,
    DefaultConfigDir,
}

impl ConfigResolutionSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::EnvConfigDir => "OPERANT_CONFIG_DIR",
            Self::EnvWorkspace => "OPERANT_WORKSPACE",
            Self::ActiveWorkspaceMarker => "active_workspace.toml",
            Self::DefaultConfigDir => "default",
        }
    }
}

/// Parse the `OPERANT_EXTRA_HEADERS` environment variable value.
///
/// Format: `Key:Value,Key2:Value2`
///
/// Entries without a colon or with an empty key are silently skipped.
/// Leading/trailing whitespace on both key and value is trimmed.
pub fn parse_extra_headers_env(raw: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((key, value)) = entry.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                tracing::warn!("Ignoring extra header with empty name in OPERANT_EXTRA_HEADERS");
                continue;
            }
            result.push((key.to_string(), value.to_string()));
        } else {
            tracing::warn!("Ignoring malformed extra header entry (missing ':'): {entry}");
        }
    }
    result
}

pub(crate) fn read_codex_openai_api_key() -> Option<String> {
    let home = UserDirs::new()?.home_dir().to_path_buf();
    let auth_path = home.join(".codex").join("auth.json");
    let raw = std::fs::read_to_string(auth_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;

    parsed
        .get("OPENAI_API_KEY")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Ensure that essential bootstrap files exist in the workspace directory.
///
/// When the workspace is created outside of `operant onboard` (e.g., non-tty
/// daemon/cron sessions), these files would otherwise be missing. This function
/// creates sensible defaults that allow the agent to operate with a basic identity.
pub(crate) async fn ensure_bootstrap_files(workspace_dir: &Path) -> Result<()> {
    let defaults: &[(&str, &str)] = &[
        (
            "IDENTITY.md",
            "# IDENTITY.md — Who Am I?\n\n\
             I am Operant, an autonomous AI agent.\n\n\
             ## Traits\n\
             - Helpful, precise, and safety-conscious\n\
             - I prioritize clarity and correctness\n",
        ),
        (
            "SOUL.md",
            "# SOUL.md — Who You Are\n\n\
             You are Operant, an autonomous AI agent.\n\n\
             ## Core Principles\n\
             - Be helpful and accurate\n\
             - Respect user intent and boundaries\n\
             - Ask before taking destructive actions\n\
             - Prefer safe, reversible operations\n",
        ),
    ];

    for (filename, content) in defaults {
        let path = workspace_dir.join(filename);
        if !path.exists() {
            fs::write(&path, content)
                .await
                .with_context(|| format!("Failed to create default {filename} in workspace"))?;
        }
    }

    Ok(())
}

impl Config {
    /// Collect the `IntegrationDescriptor` from every nested config that
    /// declares one via `#[integration(...)]`. Adding a new toggleable
    /// integration is one struct-level attribute on the new config + one
    /// row in this method. The integrations registry consumes the result
    /// without per-vendor branches.
    pub fn integration_descriptors(&self) -> Vec<crate::config::IntegrationDescriptor> {
        vec![
            self.browser.integration_descriptor(),
            self.cron.integration_descriptor(),
            self.google_workspace.integration_descriptor(),
        ]
    }

    /// Combine top-level `[cost.prices.<key>]` entries with any per-provider
    /// `pricing` entries declared on `[providers.models.<id>]`. Per-provider
    /// pricing is keyed as `<provider_id>/<model>` to align with the lookup
    /// pattern in `record_tool_loop_cost_usage` (qualified `<provider>/<model>`
    /// → bare `<model>` → suffix-after-last-slash). The qualified-first lookup
    /// order is what makes per-provider disambiguation actually take effect:
    /// an operator who sets `[providers.models.openai.pricing]` for `gpt-4o`
    /// gets that rate even if a generic `[cost.prices.gpt-4o]` is also set.
    /// Top-level entries still win on exact-key conflict so existing operator
    /// overrides keyed as `<provider>/<model>` are never silently shadowed.
    pub fn combined_pricing(&self) -> std::collections::HashMap<String, ModelPricing> {
        let mut combined = self.cost.prices.clone();
        for (provider_id, provider) in &self.providers.models {
            let (Some(pricing), Some(model)) = (&provider.pricing, &provider.model) else {
                continue;
            };
            if model.is_empty() {
                continue;
            }
            combined
                .entry(format!("{provider_id}/{model}"))
                .or_insert_with(|| pricing.clone());
        }
        combined
    }

    /// Return top-level TOML keys in `raw_toml` that Config does not recognise.
    ///
    /// Keys present in `Config::default()` serialization pass immediately.
    /// Remaining keys are probed: the key is deserialized in isolation and
    /// the result compared to the default — a changed output means serde
    /// consumed it (covers `Option<T>` fields and `#[serde(alias)]` names).
    /// V1 legacy keys (consumed by migration) are also accepted.
    pub fn unknown_keys(raw_toml: &str) -> Vec<String> {
        let raw: toml::Table = match raw_toml.parse() {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };
        static DEFAULTS: OnceLock<toml::Table> = OnceLock::new();
        let defaults = DEFAULTS.get_or_init(|| {
            toml::to_string(&Config::default())
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default()
        });
        raw.keys()
            .filter(|key| {
                if defaults.contains_key(key.as_str()) {
                    return false;
                }
                if crate::migration::V1_LEGACY_KEYS.contains(&key.as_str()) {
                    return false;
                }
                let mut t = toml::Table::new();
                t.insert((*key).clone(), raw[key.as_str()].clone());
                let consumed = toml::to_string(&t)
                    .ok()
                    .and_then(|s| toml::from_str::<Config>(&s).ok())
                    .and_then(|c| toml::to_string(&c).ok())
                    .and_then(|s| s.parse::<toml::Table>().ok())
                    .is_some_and(|t| t != *defaults);
                !consumed
            })
            .cloned()
            .collect()
    }

    /// Load the on-disk config, or initialize a fresh default config when none exists.
    pub async fn load_or_init() -> Result<Self> {
        let (default_operant_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;

        let (operant_dir, workspace_dir, resolution_source) =
            resolve_runtime_config_dirs(&default_operant_dir, &default_workspace_dir).await?;

        let config_path = operant_dir.join("config.toml");

        fs::create_dir_all(&operant_dir)
            .await
            .with_context(|| config_dir_creation_error(&operant_dir))?;
        fs::create_dir_all(&workspace_dir)
            .await
            .context("Failed to create workspace directory")?;

        ensure_bootstrap_files(&workspace_dir).await?;

        if config_path.exists() {
            // Warn if config file is world-readable (may contain API keys)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = fs::metadata(&config_path).await
                    && meta.permissions().mode() & 0o004 != 0
                {
                    tracing::warn!(
                        "Config file {:?} is world-readable (mode {:o}). \
                             Consider restricting with: chmod 600 {:?}",
                        config_path,
                        meta.permissions().mode() & 0o777,
                        config_path,
                    );
                }
            }

            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read config file")?;

            // Deserialize the config with the standard TOML parser.
            //
            // Previously this used `serde_ignored::deserialize` for both
            // deserialization and unknown-key detection.  However,
            // `serde_ignored` silently drops field values inside nested
            // structs that carry `#[serde(default)]` (e.g. the entire
            // `[autonomy]` table), causing user-supplied values to be
            // replaced by defaults.  See #4171.
            //
            // We now deserialize with `toml::from_str` (which is correct)
            // and run `serde_ignored` separately just for diagnostics.
            //
            // Before deserialization, run `prepare_table` to handle nested
            // field migrations (e.g. room_id → allowed_rooms in matrix)
            // that `#[serde(flatten)]` cannot capture.
            let mut table: toml::Table =
                toml::from_str(&contents).context("Failed to parse config as TOML table")?;
            crate::migration::prepare_table(&mut table);
            let table_str =
                toml::to_string(&table).context("Failed to re-serialize prepared table")?;
            let compat: crate::migration::V1Compat =
                toml::from_str(&table_str).context("Failed to deserialize config file")?;
            let mut config: Config = compat.into_config();

            // Ensure the built-in default auto_approve entries are always
            // present.  When a user specifies `auto_approve` in their TOML
            // (e.g. to add a custom tool), serde replaces the default list
            // instead of merging.  This caused default-safe tools like
            // `weather` or `calculator` to lose their auto-approve status
            // and get silently denied in non-interactive channel runs.
            // See #4247.
            //
            // Users who want to require approval for a default tool can
            // add it to `always_ask`, which takes precedence over
            // `auto_approve` in the approval decision (see approval/mod.rs).
            config.autonomy.ensure_default_auto_approve();

            // Backward-compatible `enabled` backfill: if a channel section
            // exists in the TOML but has no explicit `enabled` key, the user
            // configured it before `enabled` was introduced — treat it as
            // enabled so existing setups don't silently break.
            config.channels.backfill_enabled(&contents);

            // Detect unknown top-level config keys by comparing the raw
            // TOML table keys against what Config actually deserializes.
            // This replaces the previous serde_ignored-based approach which
            // had false-positive issues with #[serde(default)] nested structs.
            for key in Self::unknown_keys(&contents) {
                tracing::warn!(
                    "Unknown config key ignored: \"{key}\". Check config.toml for typos or deprecated options.",
                );
            }
            // Set computed paths that are skipped during serialization
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            let store = crate::secrets::SecretStore::new(&operant_dir, config.secrets.encrypt);
            // Decrypt all #[secret]-annotated fields via Configurable derive
            config.decrypt_secrets(&store)?;

            config.apply_env_overrides();
            config.ensure_default_mcp_servers();
            config.validate()?;
            tracing::info!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = true,
                "Config loaded"
            );
            Ok(config)
        } else {
            let mut config = Config {
                config_path: config_path.clone(),
                workspace_dir,
                ..Config::default()
            };
            config.save().await?;

            // Restrict permissions on newly created config file (may contain API keys)
            #[cfg(unix)]
            {
                use std::{fs::Permissions, os::unix::fs::PermissionsExt};
                let _ = fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await;
            }

            config.apply_env_overrides();
            config.ensure_default_mcp_servers();
            config.validate()?;
            tracing::info!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = true,
                "Config loaded"
            );
            Ok(config)
        }
    }

    /// Inject the agentmemory MCP server into `config.mcp.servers` when MCP
    /// is enabled (`mcp.enabled`), the memory backend selects agentmemory
    /// (`memory.backend == "agentmemory"`), and no `agentmemory` server is
    /// already configured.
    ///
    /// Mirrors `operant_core::config::ensure_default_mcp_servers` (the CLI/
    /// `AppConfig` path) so the runtime daemon / gateway / channels-orchestrator
    /// path — which loads `operant_config::schema::Config` — exposes the same
    /// native-MCP agentmemory surface. The schema's `mcp.deferred_loading`
    /// flag is global and defaults to `true`, so an appended server
    /// automatically joins the deferred toolset (`DeferredMcpToolSet` +
    /// `tool_search`) and never spawns `npx @agentmemory/mcp` at boot.
    ///
    /// The schema `[memory]` section has no agentmemory-specific URL/secret
    /// fields; the server reads `AGENTMEMORY_URL` from the environment and
    /// falls back to the agentmemory default (`http://localhost:3111`),
    /// mirroring the AppConfig default.
    pub fn ensure_default_mcp_servers(&mut self) {
        if !self.mcp.enabled {
            return;
        }
        // Schema-world equivalent of the AppConfig `provider == "agentmemory"`
        // trigger: the `[memory] backend` key selects agentmemory.
        if self.memory.backend != "agentmemory" {
            return;
        }
        if self.mcp.servers.iter().any(|s| s.name == "agentmemory") {
            return;
        }

        let url = std::env::var("AGENTMEMORY_URL")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| "http://localhost:3111".to_string());
        let mut env = std::collections::HashMap::new();
        env.insert("AGENTMEMORY_URL".to_string(), url);
        if let Some(secret) = std::env::var("AGENTMEMORY_SECRET")
            .ok()
            .filter(|s| !s.trim().is_empty())
        {
            env.insert("AGENTMEMORY_SECRET".to_string(), secret);
        }

        self.mcp.servers.push(McpServerConfig {
            name: "agentmemory".to_string(),
            transport: McpTransport::Stdio,
            url: None,
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@agentmemory/mcp".to_string()],
            env,
            headers: std::collections::HashMap::new(),
            tool_timeout_secs: None,
        });
    }

    pub(crate) fn lookup_model_provider_profile(
        &self,
        provider_name: &str,
    ) -> Option<(String, ModelProviderConfig)> {
        let needle = provider_name.trim();
        if needle.is_empty() {
            return None;
        }

        self.providers
            .models
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(needle))
            .map(|(name, profile)| (name.clone(), profile.clone()))
    }

    /// Apply Codex-app-server compatibility shims to the resolved fallback provider entry.
    ///
    /// Historically this method mutated `self.providers.fallback` to a "canonical" key
    /// derived from the profile's `name` field, `wire_api`, or `base_url`. That mutation
    /// caused two problems:
    ///
    /// 1. **CLI get/set divergence.** `Config::load_or_init` calls `apply_env_overrides`,
    ///    which calls this function. After load, `providers.fallback` no longer matched
    ///    what was on disk, so `operant config get providers.fallback` returned the
    ///    rewritten value while the file still had the user's literal value. The next
    ///    `save()` would then persist the rewrite, silently changing the user's config.
    /// 2. **Orphaned references.** When the rewrite pointed at a key that did not exist
    ///    in `providers.models` (e.g. profile had `name = "gemini"` but no
    ///    `[providers.models.gemini]` entry), runtime `fallback_provider()` lookups
    ///    returned `None` and downstream code fell through to a hardcoded default model.
    ///
    /// The fix: keep `self.providers.fallback` as the literal user-supplied key.
    /// Propagate the profile's `base_url` / `api_path` / `max_tokens` / `api_key` onto the
    /// resolved entry as before, and mirror the entry under any canonical alias keys so
    /// runtime lookups by either name still resolve. The user's `[providers] fallback`
    /// value is preserved end-to-end through load → save → load.
    pub(crate) fn apply_named_model_provider_profile(&mut self) {
        let Some(current_provider) = self.providers.fallback.clone() else {
            return;
        };

        let Some((profile_key, profile)) = self.lookup_model_provider_profile(&current_provider)
        else {
            return;
        };

        let base_url = profile
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);

        {
            let fallback_provider = self.providers.fallback_provider();
            let current_url = fallback_provider
                .and_then(|e| e.base_url.as_deref())
                .map(str::trim);
            if current_url.is_none_or(|value| value.is_empty())
                && let Some(base_url) = base_url.as_ref()
                && let Some(entry) = self.providers.fallback_provider_mut()
            {
                entry.base_url = Some(base_url.clone());
            }
        }

        // Propagate api_path from the profile when not already set on fallback entry.
        {
            let has_api_path = self
                .providers
                .fallback_provider()
                .and_then(|e| e.api_path.as_ref())
                .is_some();
            if !has_api_path && let Some(ref path) = profile.api_path {
                let trimmed = path.trim();
                if !trimmed.is_empty()
                    && let Some(entry) = self.providers.fallback_provider_mut()
                {
                    entry.api_path = Some(trimmed.to_string());
                }
            }
        }

        // Propagate max_tokens from the profile when not already set on fallback entry.
        {
            let has_max_tokens = self
                .providers
                .fallback_provider()
                .and_then(|e| e.max_tokens)
                .is_some();
            if !has_max_tokens
                && let Some(max_tokens) = profile.max_tokens
                && let Some(entry) = self.providers.fallback_provider_mut()
            {
                entry.max_tokens = Some(max_tokens);
            }
        }

        if profile.requires_openai_auth {
            let needs_key = self
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.as_deref())
                .map(str::trim)
                .is_none_or(|value| value.is_empty());
            if needs_key {
                let codex_key = std::env::var("OPENAI_API_KEY")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .or_else(read_codex_openai_api_key);
                if let Some(codex_key) = codex_key
                    && let Some(entry) = self.providers.fallback_provider_mut()
                {
                    entry.api_key = Some(codex_key);
                }
            }
        }

        let normalized_wire_api = profile.wire_api.as_deref().and_then(normalize_wire_api);
        let profile_name = profile
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        // Mirror the resolved entry under any canonical alias keys so that runtime
        // lookups by the profile-implied name (e.g. wire_api → "openai-codex",
        // explicit `name = ...`, or `custom:<base_url>`) also resolve. We do NOT
        // rewrite `providers.fallback` itself: that is the user's literal config
        // value and must round-trip cleanly through CLI get/set/save.
        let mut alias_keys: Vec<String> = Vec::new();
        if normalized_wire_api == Some("responses") {
            alias_keys.push("openai-codex".to_string());
        }
        if let Some(profile_name) = profile_name
            && !profile_name.eq_ignore_ascii_case(&profile_key)
        {
            alias_keys.push(profile_name.to_string());
        }
        if let Some(ref base_url) = base_url {
            alias_keys.push(format!("custom:{base_url}"));
        }

        for alias in alias_keys {
            if !self.providers.models.contains_key(&alias)
                && let Some(entry) = self.providers.models.get(&profile_key).cloned()
            {
                self.providers.models.insert(alias, entry);
            }
        }
    }

    /// Collect non-fatal validation warnings — config that loads and
    /// validates successfully (`validate()` returns `Ok(())`) but will fail
    /// at runtime because of a logical inconsistency the schema cannot
    /// enforce structurally.
    ///
    /// Called by `validate()` (which emits each warning via `tracing::warn!`
    /// for log visibility) and by the gateway HTTP API (which returns the
    /// structured list in `PropResponse` / `PatchResponse` so dashboard
    /// callers see the same signal the CLI sees on stderr).
    ///
    /// Adding a new warning: append a check here, pick a stable `code`,
    /// and document the code in `validation_warnings.rs`.
    pub fn collect_warnings(&self) -> Vec<crate::validation_warnings::ValidationWarning> {
        use crate::validation_warnings::ValidationWarning;
        let mut warnings = Vec::new();

        // providers.fallback references a key not present in providers.models
        if let Some(ref fallback_key) = self.providers.fallback
            && !self.providers.models.contains_key(fallback_key)
        {
            warnings.push(ValidationWarning::new(
                "dangling_provider_fallback",
                format!(
                    "providers.fallback references '{fallback_key}' which does not exist in providers.models; provider resolution will fail at runtime"
                ),
                "providers.fallback",
            ));
        }

        warnings
    }

    /// Validate configuration values that would cause runtime failures.
    ///
    /// Called after TOML deserialization and env-override application to catch
    /// obviously invalid values early instead of failing at arbitrary runtime points.
    pub fn validate(&self) -> Result<()> {
        // Tunnel — OpenVPN
        if self.tunnel.provider.trim() == "openvpn" {
            let openvpn = self.tunnel.openvpn.as_ref().ok_or_else(|| {
                anyhow::anyhow!("tunnel.provider='openvpn' requires [tunnel.openvpn]")
            })?;

            if openvpn.config_file.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "tunnel.openvpn.config_file",
                    "tunnel.openvpn.config_file must not be empty"
                );
            }
            if openvpn.connect_timeout_secs == 0 {
                validation_bail!(
                    InvalidNumericRange,
                    "tunnel.openvpn.connect_timeout_secs",
                    "tunnel.openvpn.connect_timeout_secs must be greater than 0"
                );
            }
        }

        // Gateway
        if self.gateway.host.trim().is_empty() {
            validation_bail!(
                RequiredFieldEmpty,
                "gateway.host",
                "gateway.host must not be empty"
            );
        }
        if let Some(ref prefix) = self.gateway.path_prefix {
            // Validate the raw value — no silent trimming so the stored
            // value is exactly what was validated.
            if !prefix.is_empty() {
                if !prefix.starts_with('/') {
                    validation_bail!(
                        InvalidFormat,
                        "gateway.path_prefix",
                        "gateway.path_prefix must start with '/'"
                    );
                }
                if prefix.ends_with('/') {
                    validation_bail!(
                        InvalidFormat,
                        "gateway.path_prefix",
                        "gateway.path_prefix must not end with '/' (including bare '/')"
                    );
                }
                // Reject characters unsafe for URL paths or HTML/JS injection.
                // Whitespace is intentionally excluded from the allowed set.
                if let Some(bad) = prefix.chars().find(|c| {
                    !matches!(c, '/' | '-' | '_' | '.' | '~'
                        | 'a'..='z' | 'A'..='Z' | '0'..='9'
                        | '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '='
                        | ':' | '@')
                }) {
                    anyhow::bail!(
                        "gateway.path_prefix contains invalid character '{bad}'; \
                         only unreserved and sub-delim URI characters are allowed"
                    );
                }
            }
        }

        // Autonomy
        if self.autonomy.max_actions_per_hour == 0 {
            validation_bail!(
                InvalidNumericRange,
                "autonomy.max_actions_per_hour",
                "autonomy.max_actions_per_hour must be greater than 0"
            );
        }
        for (i, env_name) in self.autonomy.shell_env_passthrough.iter().enumerate() {
            if !is_valid_env_var_name(env_name) {
                anyhow::bail!(
                    "autonomy.shell_env_passthrough[{i}] is invalid ({env_name}); expected [A-Za-z_][A-Za-z0-9_]*"
                );
            }
        }

        // Security OTP / estop
        if self.security.otp.challenge_max_attempts == 0 {
            validation_bail!(
                InvalidNumericRange,
                "security.otp.challenge_max_attempts",
                "security.otp.challenge_max_attempts must be greater than 0"
            );
        }
        if self.security.otp.token_ttl_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "security.otp.token_ttl_secs",
                "security.otp.token_ttl_secs must be greater than 0"
            );
        }
        if self.security.otp.cache_valid_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "security.otp.cache_valid_secs",
                "security.otp.cache_valid_secs must be greater than 0"
            );
        }
        if self.security.otp.cache_valid_secs < self.security.otp.token_ttl_secs {
            anyhow::bail!(
                "security.otp.cache_valid_secs must be greater than or equal to security.otp.token_ttl_secs"
            );
        }
        if self.security.otp.challenge_max_attempts == 0 {
            validation_bail!(
                InvalidNumericRange,
                "security.otp.challenge_max_attempts",
                "security.otp.challenge_max_attempts must be greater than 0"
            );
        }
        for (i, action) in self.security.otp.gated_actions.iter().enumerate() {
            let normalized = action.trim();
            if normalized.is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("security.otp.gated_actions[{i}]"),
                    "security.otp.gated_actions[{i}] must not be empty"
                );
            }
            if !normalized
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "security.otp.gated_actions[{i}] contains invalid characters: {normalized}"
                );
            }
        }
        DomainMatcher::new(
            &self.security.otp.gated_domains,
            &self.security.otp.gated_domain_categories,
        )
        .with_context(
            || "Invalid security.otp.gated_domains or security.otp.gated_domain_categories",
        )?;
        if self.security.estop.state_file.trim().is_empty() {
            validation_bail!(
                RequiredFieldEmpty,
                "security.estop.state_file",
                "security.estop.state_file must not be empty"
            );
        }

        // Scheduler
        if self.scheduler.max_concurrent == 0 {
            validation_bail!(
                InvalidNumericRange,
                "scheduler.max_concurrent",
                "scheduler.max_concurrent must be greater than 0"
            );
        }
        if self.scheduler.max_tasks == 0 {
            validation_bail!(
                InvalidNumericRange,
                "scheduler.max_tasks",
                "scheduler.max_tasks must be greater than 0"
            );
        }

        // Model routes
        for (i, route) in self.providers.model_routes.iter().enumerate() {
            if route.hint.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("model_routes[{i}].hint"),
                    "model_routes[{i}].hint must not be empty"
                );
            }
            if route.provider.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("model_routes[{i}].provider"),
                    "model_routes[{i}].provider must not be empty"
                );
            }
            if route.model.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("model_routes[{i}].model"),
                    "model_routes[{i}].model must not be empty"
                );
            }
        }

        // Embedding routes
        for (i, route) in self.providers.embedding_routes.iter().enumerate() {
            if route.hint.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("embedding_routes[{i}].hint"),
                    "embedding_routes[{i}].hint must not be empty"
                );
            }
            if route.provider.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("embedding_routes[{i}].provider"),
                    "embedding_routes[{i}].provider must not be empty"
                );
            }
            if route.model.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("embedding_routes[{i}].model"),
                    "embedding_routes[{i}].model must not be empty"
                );
            }
        }

        for (profile_key, profile) in &self.providers.models {
            let profile_name = profile_key.trim();
            if profile_name.is_empty() {
                anyhow::bail!("model_providers contains an empty profile name");
            }

            let has_name = profile
                .name
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            let has_base_url = profile
                .base_url
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());

            // Entries created by migration from top-level fields use the provider
            // name as the map key and may not have explicit `name` or `base_url`
            // (the provider factory resolves known names). An entry with no
            // identifying information at all is almost always an in-progress
            // onboarding state — the user picked the provider but hasn't filled
            // anything in yet. Warn but don't bail; the runtime falls back to
            // provider-trait defaults at use time, and a chat against the
            // unconfigured provider fails with a clear error then.
            let has_api_key = profile
                .api_key
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty());
            let has_model = profile
                .model
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty());
            if !has_name && !has_base_url && !has_api_key && !has_model {
                tracing::warn!(
                    provider = %profile_name,
                    "providers.models.{profile_name} is empty (no name / base_url / api_key / model). \
                     Skipping at runtime; finish onboarding via the dashboard or `operant onboard` \
                     to make this provider usable.",
                );
                continue;
            }

            if let Some(base_url) = profile.base_url.as_deref().map(str::trim)
                && !base_url.is_empty()
            {
                let parsed = reqwest::Url::parse(base_url).with_context(|| {
                    format!("model_providers.{profile_name}.base_url is not a valid URL")
                })?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    anyhow::bail!("model_providers.{profile_name}.base_url must use http/https");
                }
            }

            if let Some(wire_api) = profile.wire_api.as_deref().map(str::trim)
                && !wire_api.is_empty()
                && normalize_wire_api(wire_api).is_none()
            {
                anyhow::bail!(
                    "model_providers.{profile_name}.wire_api must be one of: responses, chat_completions"
                );
            }

            if let Some(temp) = profile.temperature {
                validate_temperature(temp).map_err(|e| {
                    anyhow::anyhow!("providers.models.{profile_name}.temperature: {e}")
                })?;
            }
        }

        // Non-fatal validation warnings: surfaced both via tracing (CLI sees
        // on stderr) and via Config::collect_warnings (gateway HTTP returns
        // structured to dashboard callers). Single source of truth lives in
        // collect_warnings; emit each one to tracing here so the existing
        // log behavior is preserved.
        for w in self.collect_warnings() {
            tracing::warn!(path = %w.path, code = %w.code, "{}", w.message);
        }

        // Ollama cloud-routing safety checks
        if self
            .providers
            .fallback
            .as_deref()
            .is_some_and(|provider| provider.trim().eq_ignore_ascii_case("ollama"))
            && self
                .providers
                .fallback_provider()
                .and_then(|e| e.model.as_deref())
                .is_some_and(|model| model.trim().ends_with(":cloud"))
        {
            if is_local_ollama_endpoint(
                self.providers
                    .fallback_provider()
                    .and_then(|e| e.base_url.as_deref()),
            ) {
                anyhow::bail!(
                    "default_model uses ':cloud' with provider 'ollama', but api_url is local or unset. Set api_url to a remote Ollama endpoint (for example https://ollama.com)."
                );
            }

            if !has_ollama_cloud_credential(
                self.providers
                    .fallback_provider()
                    .and_then(|e| e.api_key.as_deref()),
            ) {
                anyhow::bail!(
                    "default_model uses ':cloud' with provider 'ollama', but no API key is configured. Set api_key or OLLAMA_API_KEY."
                );
            }
        }

        // Microsoft 365
        if self.microsoft365.enabled {
            let tenant = self
                .microsoft365
                .tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if tenant.is_none() {
                anyhow::bail!(
                    "microsoft365.tenant_id must not be empty when microsoft365 is enabled"
                );
            }
            let client = self
                .microsoft365
                .client_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if client.is_none() {
                anyhow::bail!(
                    "microsoft365.client_id must not be empty when microsoft365 is enabled"
                );
            }
            let flow = self.microsoft365.auth_flow.trim();
            if flow != "client_credentials" && flow != "device_code" {
                anyhow::bail!(
                    "microsoft365.auth_flow must be 'client_credentials' or 'device_code'"
                );
            }
            if flow == "client_credentials"
                && self
                    .microsoft365
                    .client_secret
                    .as_deref()
                    .is_none_or(|s| s.trim().is_empty())
            {
                anyhow::bail!(
                    "microsoft365.client_secret must not be empty when auth_flow is 'client_credentials'"
                );
            }
        }

        // Microsoft 365
        if self.microsoft365.enabled {
            let tenant = self
                .microsoft365
                .tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if tenant.is_none() {
                anyhow::bail!(
                    "microsoft365.tenant_id must not be empty when microsoft365 is enabled"
                );
            }
            let client = self
                .microsoft365
                .client_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if client.is_none() {
                anyhow::bail!(
                    "microsoft365.client_id must not be empty when microsoft365 is enabled"
                );
            }
            let flow = self.microsoft365.auth_flow.trim();
            if flow != "client_credentials" && flow != "device_code" {
                anyhow::bail!("microsoft365.auth_flow must be client_credentials or device_code");
            }
            if flow == "client_credentials"
                && self
                    .microsoft365
                    .client_secret
                    .as_deref()
                    .is_none_or(|s| s.trim().is_empty())
            {
                anyhow::bail!(
                    "microsoft365.client_secret must not be empty when auth_flow is client_credentials"
                );
            }
        }

        // MCP
        if self.mcp.enabled {
            validate_mcp_config(&self.mcp)?;
        }

        // Knowledge graph
        if self.knowledge.enabled {
            if self.knowledge.max_nodes == 0 {
                validation_bail!(
                    InvalidNumericRange,
                    "knowledge.max_nodes",
                    "knowledge.max_nodes must be greater than 0"
                );
            }
            if self.knowledge.db_path.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "knowledge.db_path",
                    "knowledge.db_path must not be empty"
                );
            }
        }

        // Google Workspace allowed_services validation
        let mut seen_gws_services = std::collections::HashSet::new();
        for (i, service) in self.google_workspace.allowed_services.iter().enumerate() {
            let normalized = service.trim();
            if normalized.is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("google_workspace.allowed_services[{i}]"),
                    "google_workspace.allowed_services[{i}] must not be empty"
                );
            }
            if !normalized
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "google_workspace.allowed_services[{i}] contains invalid characters: {normalized}"
                );
            }
            if !seen_gws_services.insert(normalized.to_string()) {
                anyhow::bail!(
                    "google_workspace.allowed_services contains duplicate entry: {normalized}"
                );
            }
        }

        // Build the effective allowed-services set for cross-validation.
        // When the operator leaves allowed_services empty the tool falls back to
        // DEFAULT_GWS_SERVICES; use the same constant here so validation is
        // consistent in both cases.
        let effective_services: std::collections::HashSet<&str> =
            if self.google_workspace.allowed_services.is_empty() {
                DEFAULT_GWS_SERVICES.iter().copied().collect()
            } else {
                self.google_workspace
                    .allowed_services
                    .iter()
                    .map(|s| s.trim())
                    .collect()
            };

        let mut seen_gws_operations = std::collections::HashSet::new();
        for (i, operation) in self.google_workspace.allowed_operations.iter().enumerate() {
            let service = operation.service.trim();
            let resource = operation.resource.trim();

            if service.is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("google_workspace.allowed_operations[{i}].service"),
                    "google_workspace.allowed_operations[{i}].service must not be empty"
                );
            }
            if resource.is_empty() {
                anyhow::bail!(
                    "google_workspace.allowed_operations[{i}].resource must not be empty"
                );
            }

            if !effective_services.contains(service) {
                anyhow::bail!(
                    "google_workspace.allowed_operations[{i}].service '{service}' is not in the \
                     effective allowed_services; this entry can never match at runtime"
                );
            }
            if !service
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "google_workspace.allowed_operations[{i}].service contains invalid characters: {service}"
                );
            }
            if !resource
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "google_workspace.allowed_operations[{i}].resource contains invalid characters: {resource}"
                );
            }

            if let Some(ref sub_resource) = operation.sub_resource {
                let sub = sub_resource.trim();
                if sub.is_empty() {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].sub_resource must not be empty when present"
                    );
                }
                if !sub
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
                {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].sub_resource contains invalid characters: {sub}"
                    );
                }
            }

            if operation.methods.is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("google_workspace.allowed_operations[{i}].methods"),
                    "google_workspace.allowed_operations[{i}].methods must not be empty"
                );
            }

            let mut seen_methods = std::collections::HashSet::new();
            for (j, method) in operation.methods.iter().enumerate() {
                let normalized = method.trim();
                if normalized.is_empty() {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].methods[{j}] must not be empty"
                    );
                }
                if !normalized
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
                {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].methods[{j}] contains invalid characters: {normalized}"
                    );
                }
                if !seen_methods.insert(normalized.to_string()) {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].methods contains duplicate entry: {normalized}"
                    );
                }
            }

            let sub_key = operation
                .sub_resource
                .as_deref()
                .map(str::trim)
                .unwrap_or("");
            let operation_key = format!("{service}:{resource}:{sub_key}");
            if !seen_gws_operations.insert(operation_key.clone()) {
                anyhow::bail!(
                    "google_workspace.allowed_operations contains duplicate service/resource/sub_resource entry: {operation_key}"
                );
            }
        }

        // Project intelligence
        if self.project_intel.enabled {
            let lang = &self.project_intel.default_language;
            if !["en", "de", "fr", "it"].contains(&lang.as_str()) {
                anyhow::bail!(
                    "project_intel.default_language must be one of: en, de, fr, it (got '{lang}')"
                );
            }
            let sens = &self.project_intel.risk_sensitivity;
            if !["low", "medium", "high"].contains(&sens.as_str()) {
                anyhow::bail!(
                    "project_intel.risk_sensitivity must be one of: low, medium, high (got '{sens}')"
                );
            }
            if let Some(ref tpl_dir) = self.project_intel.templates_dir
                && !std::path::Path::new(tpl_dir).exists()
            {
                anyhow::bail!("project_intel.templates_dir path does not exist: {tpl_dir}");
            }
        }

        // Proxy (delegate to existing validation)
        self.proxy.validate()?;
        self.cloud_ops.validate()?;

        // Notion
        if self.notion.enabled {
            if self.notion.database_id.trim().is_empty() {
                anyhow::bail!("notion.database_id must not be empty when notion.enabled = true");
            }
            if self.notion.poll_interval_secs == 0 {
                validation_bail!(
                    InvalidNumericRange,
                    "notion.poll_interval_secs",
                    "notion.poll_interval_secs must be greater than 0"
                );
            }
            if self.notion.max_concurrent == 0 {
                validation_bail!(
                    InvalidNumericRange,
                    "notion.max_concurrent",
                    "notion.max_concurrent must be greater than 0"
                );
            }
            if self.notion.status_property.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "notion.status_property",
                    "notion.status_property must not be empty"
                );
            }
            if self.notion.input_property.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "notion.input_property",
                    "notion.input_property must not be empty"
                );
            }
            if self.notion.result_property.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "notion.result_property",
                    "notion.result_property must not be empty"
                );
            }
        }

        // Pinggy tunnel region — validate allowed values (case-insensitive, auto-lowercased at runtime).
        if let Some(ref pinggy) = self.tunnel.pinggy
            && let Some(ref region) = pinggy.region
        {
            let r = region.trim().to_ascii_lowercase();
            if !r.is_empty() && !matches!(r.as_str(), "us" | "eu" | "ap" | "br" | "au") {
                anyhow::bail!(
                    "tunnel.pinggy.region must be one of: us, eu, ap, br, au (or omitted for auto)"
                );
            }
        }

        // Jira
        if self.jira.enabled {
            if self.jira.base_url.trim().is_empty() {
                anyhow::bail!("jira.base_url must not be empty when jira.enabled = true");
            }
            if self.jira.api_token.trim().is_empty()
                && std::env::var("JIRA_API_TOKEN")
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
            {
                anyhow::bail!(
                    "jira.api_token must be set (or JIRA_API_TOKEN env var) when jira.enabled = true"
                );
            }
            let valid_actions = ["get_ticket", "search_tickets", "comment_ticket"];
            for action in &self.jira.allowed_actions {
                if !valid_actions.contains(&action.as_str()) {
                    anyhow::bail!(
                        "jira.allowed_actions contains unknown action: '{}'. \
                         Valid: get_ticket, search_tickets, comment_ticket",
                        action
                    );
                }
            }
        }

        // Nevis IAM — delegate to NevisConfig::validate() for field-level checks
        if let Err(msg) = self.security.nevis.validate() {
            anyhow::bail!("security.nevis: {msg}");
        }

        // Delegate agent timeouts
        const MAX_DELEGATE_TIMEOUT_SECS: u64 = 3600;
        for (name, agent) in &self.agents {
            if let Some(timeout) = agent.timeout_secs {
                if timeout == 0 {
                    validation_bail!(
                        InvalidNumericRange,
                        format!("agents.{name}.timeout_secs"),
                        "agents.{name}.timeout_secs must be greater than 0"
                    );
                }
                if timeout > MAX_DELEGATE_TIMEOUT_SECS {
                    anyhow::bail!(
                        "agents.{name}.timeout_secs exceeds max {MAX_DELEGATE_TIMEOUT_SECS}"
                    );
                }
            }
            if let Some(timeout) = agent.agentic_timeout_secs {
                if timeout == 0 {
                    validation_bail!(
                        InvalidNumericRange,
                        format!("agents.{name}.agentic_timeout_secs"),
                        "agents.{name}.agentic_timeout_secs must be greater than 0"
                    );
                }
                if timeout > MAX_DELEGATE_TIMEOUT_SECS {
                    anyhow::bail!(
                        "agents.{name}.agentic_timeout_secs exceeds max {MAX_DELEGATE_TIMEOUT_SECS}"
                    );
                }
            }
        }

        // Transcription
        {
            let dp = self.transcription.default_provider.trim();
            match dp {
                "groq" | "openai" | "deepgram" | "assemblyai" | "google" | "local_whisper" => {}
                other => {
                    anyhow::bail!(
                        "transcription.default_provider must be one of: groq, openai, deepgram, assemblyai, google, local_whisper (got '{other}')"
                    );
                }
            }
        }

        // Delegate tool global defaults
        if self.delegate.timeout_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "delegate.timeout_secs",
                "delegate.timeout_secs must be greater than 0"
            );
        }
        if self.delegate.agentic_timeout_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "delegate.agentic_timeout_secs",
                "delegate.agentic_timeout_secs must be greater than 0"
            );
        }

        // Per-agent delegate timeout overrides
        for (name, agent) in &self.agents {
            if let Some(t) = agent.timeout_secs
                && t == 0
            {
                validation_bail!(
                    InvalidNumericRange,
                    format!("agents.{name}.timeout_secs"),
                    "agents.{name}.timeout_secs must be greater than 0"
                );
            }
            if let Some(t) = agent.agentic_timeout_secs
                && t == 0
            {
                validation_bail!(
                    InvalidNumericRange,
                    format!("agents.{name}.agentic_timeout_secs"),
                    "agents.{name}.agentic_timeout_secs must be greater than 0"
                );
            }
        }

        // Channel reply-intent precheck. Zero timeout or empty/whitespace model would
        // silently fail open to REPLY and quietly disable the group-chat noise filter
        // — reject the typo cases explicitly. Use `enabled = false` to disable instead.
        if self.agent.precheck.timeout_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "agent.precheck.timeout_secs",
                "agent.precheck.timeout_secs must be greater than 0 (use agent.precheck.enabled = false to disable the precheck)"
            );
        }
        if let Some(ref model) = self.agent.precheck.model
            && model.trim().is_empty()
        {
            validation_bail!(
                RequiredFieldEmpty,
                "agent.precheck.model",
                "agent.precheck.model must not be empty or whitespace; omit the key to fall back to the route model"
            );
        }

        Ok(())
    }

    /// Ensure the fallback provider entry exists, creating it if necessary.
    pub fn ensure_fallback_provider(&mut self) -> &mut ModelProviderConfig {
        let fallback = self
            .providers
            .fallback
            .clone()
            .unwrap_or_else(|| "default".into());
        if self.providers.fallback.is_none() {
            self.providers.fallback = Some(fallback.clone());
        }
        self.providers.models.entry(fallback).or_default()
    }

    /// Apply environment variable overrides to config
    pub fn apply_env_overrides(&mut self) {
        // API Key: OPERANT_API_KEY or API_KEY (generic)
        if let Ok(key) = std::env::var("OPERANT_API_KEY").or_else(|_| std::env::var("API_KEY"))
            && !key.is_empty()
        {
            self.ensure_fallback_provider().api_key = Some(key);
        }
        // API Key: GLM_API_KEY overrides when provider is a GLM/Zhipu variant.
        if self.providers.fallback.as_deref().is_some_and(is_glm_alias)
            && let Ok(key) = std::env::var("GLM_API_KEY")
            && !key.is_empty()
        {
            self.ensure_fallback_provider().api_key = Some(key);
        }

        // API Key: ZAI_API_KEY overrides when provider is a Z.AI variant.
        if self.providers.fallback.as_deref().is_some_and(is_zai_alias)
            && let Ok(key) = std::env::var("ZAI_API_KEY")
            && !key.is_empty()
        {
            self.ensure_fallback_provider().api_key = Some(key);
        }

        // Provider override precedence:
        // 1) OPERANT_PROVIDER always wins when set.
        // 2) OPERANT_MODEL_PROVIDER/MODEL_PROVIDER (Codex app-server style).
        // 3) Legacy PROVIDER is honored only when config still uses default provider.
        if let Ok(provider) = std::env::var("OPERANT_PROVIDER")
            && !provider.is_empty()
        {
            self.providers.fallback = Some(provider);
        } else if let Ok(provider) =
            std::env::var("OPERANT_MODEL_PROVIDER").or_else(|_| std::env::var("MODEL_PROVIDER"))
            && !provider.is_empty()
        {
            self.providers.fallback = Some(provider);
        } else if let Ok(provider) = std::env::var("PROVIDER") {
            let should_apply_legacy_provider = self
                .providers
                .fallback
                .as_deref()
                .is_none_or(|configured| configured.trim().eq_ignore_ascii_case("openrouter"));
            if should_apply_legacy_provider && !provider.is_empty() {
                self.providers.fallback = Some(provider);
            }
        }

        // Model: OPERANT_MODEL or MODEL
        if let Ok(model) = std::env::var("OPERANT_MODEL").or_else(|_| std::env::var("MODEL"))
            && !model.is_empty()
        {
            self.ensure_fallback_provider().model = Some(model);
        }

        // Provider HTTP timeout: OPERANT_PROVIDER_TIMEOUT_SECS
        if let Ok(timeout_secs) = std::env::var("OPERANT_PROVIDER_TIMEOUT_SECS")
            && let Ok(timeout_secs) = timeout_secs.parse::<u64>()
            && timeout_secs > 0
        {
            self.ensure_fallback_provider().timeout_secs = Some(timeout_secs);
        }

        // Extra provider headers: OPERANT_EXTRA_HEADERS
        // Format: "Key:Value,Key2:Value2"
        // Env var headers override config file headers with the same name.
        if let Ok(raw) = std::env::var("OPERANT_EXTRA_HEADERS") {
            let entry = self.ensure_fallback_provider();
            for header in parse_extra_headers_env(&raw) {
                entry.extra_headers.insert(header.0, header.1);
            }
        }

        // Apply named provider profile remapping (Codex app-server compatibility).
        self.apply_named_model_provider_profile();

        // Workspace directory: OPERANT_WORKSPACE
        if let Ok(workspace) = std::env::var("OPERANT_WORKSPACE")
            && !workspace.is_empty()
        {
            let expanded = expand_tilde_path(&workspace);
            let (_, workspace_dir) = resolve_config_dir_for_workspace(&expanded);
            self.workspace_dir = workspace_dir;
        }

        // Open-skills opt-in flag: OPERANT_OPEN_SKILLS_ENABLED
        if let Ok(flag) = std::env::var("OPERANT_OPEN_SKILLS_ENABLED")
            && !flag.trim().is_empty()
        {
            match flag.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => self.skills.open_skills_enabled = true,
                "0" | "false" | "no" | "off" => self.skills.open_skills_enabled = false,
                _ => tracing::warn!(
                    "Ignoring invalid OPERANT_OPEN_SKILLS_ENABLED (valid: 1|0|true|false|yes|no|on|off)"
                ),
            }
        }

        // Open-skills directory override: OPERANT_OPEN_SKILLS_DIR
        if let Ok(path) = std::env::var("OPERANT_OPEN_SKILLS_DIR") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                self.skills.open_skills_dir = Some(trimmed.to_string());
            }
        }

        // Skills script-file audit override: OPERANT_SKILLS_ALLOW_SCRIPTS
        if let Ok(flag) = std::env::var("OPERANT_SKILLS_ALLOW_SCRIPTS")
            && !flag.trim().is_empty()
        {
            match flag.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => self.skills.allow_scripts = true,
                "0" | "false" | "no" | "off" => self.skills.allow_scripts = false,
                _ => tracing::warn!(
                    "Ignoring invalid OPERANT_SKILLS_ALLOW_SCRIPTS (valid: 1|0|true|false|yes|no|on|off)"
                ),
            }
        }

        // Skills prompt mode override: OPERANT_SKILLS_PROMPT_MODE
        if let Ok(mode) = std::env::var("OPERANT_SKILLS_PROMPT_MODE")
            && !mode.trim().is_empty()
        {
            if let Some(parsed) = parse_skills_prompt_injection_mode(&mode) {
                self.skills.prompt_injection_mode = parsed;
            } else {
                tracing::warn!("Ignoring invalid OPERANT_SKILLS_PROMPT_MODE (valid: full|compact)");
            }
        }

        // Gateway port: OPERANT_GATEWAY_PORT or PORT
        if let Ok(port_str) =
            std::env::var("OPERANT_GATEWAY_PORT").or_else(|_| std::env::var("PORT"))
            && let Ok(port) = port_str.parse::<u16>()
        {
            self.gateway.port = port;
        }

        // Gateway host: OPERANT_GATEWAY_HOST or HOST
        if let Ok(host) = std::env::var("OPERANT_GATEWAY_HOST").or_else(|_| std::env::var("HOST"))
            && !host.is_empty()
        {
            self.gateway.host = host;
        }

        // Allow public bind: OPERANT_ALLOW_PUBLIC_BIND
        if let Ok(val) = std::env::var("OPERANT_ALLOW_PUBLIC_BIND") {
            self.gateway.allow_public_bind = val == "1" || val.eq_ignore_ascii_case("true");
        }

        // Require pairing: OPERANT_REQUIRE_PAIRING
        if let Ok(val) = std::env::var("OPERANT_REQUIRE_PAIRING") {
            self.gateway.require_pairing = val == "1" || val.eq_ignore_ascii_case("true");
        }

        // Web dist dir: OPERANT_WEB_DIST_DIR
        if let Ok(path) = std::env::var("OPERANT_WEB_DIST_DIR") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                self.gateway.web_dist_dir = Some(trimmed.to_string());
            }
        }

        // Temperature: OPERANT_TEMPERATURE
        if let Ok(temp_str) = std::env::var("OPERANT_TEMPERATURE") {
            match temp_str.parse::<f64>() {
                Ok(temp) if TEMPERATURE_RANGE.contains(&temp) => {
                    self.ensure_fallback_provider().temperature = Some(temp);
                }
                Ok(temp) => {
                    tracing::warn!(
                        "Ignoring OPERANT_TEMPERATURE={temp}: \
                         value out of range (expected {}..={})",
                        TEMPERATURE_RANGE.start(),
                        TEMPERATURE_RANGE.end()
                    );
                }
                Err(_) => {
                    tracing::warn!("Ignoring OPERANT_TEMPERATURE={temp_str:?}: not a valid number");
                }
            }
        }

        // Reasoning override: OPERANT_REASONING_ENABLED or REASONING_ENABLED
        if let Ok(flag) = std::env::var("OPERANT_REASONING_ENABLED")
            .or_else(|_| std::env::var("REASONING_ENABLED"))
        {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.runtime.reasoning_enabled = Some(true),
                "0" | "false" | "no" | "off" => self.runtime.reasoning_enabled = Some(false),
                _ => {}
            }
        }

        if let Ok(raw) = std::env::var("OPERANT_REASONING_EFFORT")
            .or_else(|_| std::env::var("REASONING_EFFORT"))
            .or_else(|_| std::env::var("OPERANT_CODEX_REASONING_EFFORT"))
        {
            match normalize_reasoning_effort(&raw) {
                Ok(effort) => self.runtime.reasoning_effort = Some(effort),
                Err(message) => tracing::warn!("Ignoring reasoning effort env override: {message}"),
            }
        }

        // Web search enabled: OPERANT_WEB_SEARCH_ENABLED or WEB_SEARCH_ENABLED
        if let Ok(enabled) = std::env::var("OPERANT_WEB_SEARCH_ENABLED")
            .or_else(|_| std::env::var("WEB_SEARCH_ENABLED"))
        {
            self.web_search.enabled = enabled == "1" || enabled.eq_ignore_ascii_case("true");
        }

        // Web search provider: OPERANT_WEB_SEARCH_PROVIDER or WEB_SEARCH_PROVIDER
        if let Ok(provider) = std::env::var("OPERANT_WEB_SEARCH_PROVIDER")
            .or_else(|_| std::env::var("WEB_SEARCH_PROVIDER"))
        {
            let provider = provider.trim();
            if !provider.is_empty() {
                self.web_search.provider = provider.to_string();
            }
        }

        // Brave API key: OPERANT_BRAVE_API_KEY or BRAVE_API_KEY
        if let Ok(api_key) =
            std::env::var("OPERANT_BRAVE_API_KEY").or_else(|_| std::env::var("BRAVE_API_KEY"))
        {
            let api_key = api_key.trim();
            if !api_key.is_empty() {
                self.web_search.brave_api_key = Some(api_key.to_string());
            }
        }

        // Tavily API key: OPERANT_TAVILY_API_KEY or TAVILY_API_KEY
        if let Ok(api_key) =
            std::env::var("OPERANT_TAVILY_API_KEY").or_else(|_| std::env::var("TAVILY_API_KEY"))
        {
            let api_key = api_key.trim();
            if !api_key.is_empty() {
                self.web_search.tavily_api_key = Some(api_key.to_string());
            }
        }

        // SearXNG instance URL: OPERANT_SEARXNG_INSTANCE_URL or SEARXNG_INSTANCE_URL
        if let Ok(instance_url) = std::env::var("OPERANT_SEARXNG_INSTANCE_URL")
            .or_else(|_| std::env::var("SEARXNG_INSTANCE_URL"))
        {
            let instance_url = instance_url.trim();
            if !instance_url.is_empty() {
                self.web_search.searxng_instance_url = Some(instance_url.to_string());
            }
        }

        // Web search max results: OPERANT_WEB_SEARCH_MAX_RESULTS or WEB_SEARCH_MAX_RESULTS
        if let Ok(max_results) = std::env::var("OPERANT_WEB_SEARCH_MAX_RESULTS")
            .or_else(|_| std::env::var("WEB_SEARCH_MAX_RESULTS"))
            && let Ok(max_results) = max_results.parse::<usize>()
            && (1..=10).contains(&max_results)
        {
            self.web_search.max_results = max_results;
        }

        // Web search timeout: OPERANT_WEB_SEARCH_TIMEOUT_SECS or WEB_SEARCH_TIMEOUT_SECS
        if let Ok(timeout_secs) = std::env::var("OPERANT_WEB_SEARCH_TIMEOUT_SECS")
            .or_else(|_| std::env::var("WEB_SEARCH_TIMEOUT_SECS"))
            && let Ok(timeout_secs) = timeout_secs.parse::<u64>()
            && timeout_secs > 0
        {
            self.web_search.timeout_secs = timeout_secs;
        }

        // Storage provider key (optional backend override): OPERANT_STORAGE_PROVIDER
        if let Ok(provider) = std::env::var("OPERANT_STORAGE_PROVIDER") {
            let provider = provider.trim();
            if !provider.is_empty() {
                self.storage.provider.config.provider = provider.to_string();
            }
        }

        // Storage connection URL (for remote backends): OPERANT_STORAGE_DB_URL
        if let Ok(db_url) = std::env::var("OPERANT_STORAGE_DB_URL") {
            let db_url = db_url.trim();
            if !db_url.is_empty() {
                self.storage.provider.config.db_url = Some(db_url.to_string());
            }
        }

        // Storage connect timeout: OPERANT_STORAGE_CONNECT_TIMEOUT_SECS
        if let Ok(timeout_secs) = std::env::var("OPERANT_STORAGE_CONNECT_TIMEOUT_SECS")
            && let Ok(timeout_secs) = timeout_secs.parse::<u64>()
            && timeout_secs > 0
        {
            self.storage.provider.config.connect_timeout_secs = Some(timeout_secs);
        }
        // Proxy enabled flag: OPERANT_PROXY_ENABLED
        let explicit_proxy_enabled = std::env::var("OPERANT_PROXY_ENABLED")
            .ok()
            .as_deref()
            .and_then(parse_proxy_enabled);
        if let Some(enabled) = explicit_proxy_enabled {
            self.proxy.enabled = enabled;
        }

        // Proxy URLs: OPERANT_* wins, then generic *PROXY vars.
        let mut proxy_url_overridden = false;
        if let Ok(proxy_url) =
            std::env::var("OPERANT_HTTP_PROXY").or_else(|_| std::env::var("HTTP_PROXY"))
        {
            self.proxy.http_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(proxy_url) =
            std::env::var("OPERANT_HTTPS_PROXY").or_else(|_| std::env::var("HTTPS_PROXY"))
        {
            self.proxy.https_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(proxy_url) =
            std::env::var("OPERANT_ALL_PROXY").or_else(|_| std::env::var("ALL_PROXY"))
        {
            self.proxy.all_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(no_proxy) =
            std::env::var("OPERANT_NO_PROXY").or_else(|_| std::env::var("NO_PROXY"))
        {
            self.proxy.no_proxy = normalize_no_proxy_list(vec![no_proxy]);
        }

        if explicit_proxy_enabled.is_none()
            && proxy_url_overridden
            && self.proxy.has_any_proxy_url()
        {
            self.proxy.enabled = true;
        }

        // Proxy scope and service selectors.
        if let Ok(scope_raw) = std::env::var("OPERANT_PROXY_SCOPE") {
            if let Some(scope) = parse_proxy_scope(&scope_raw) {
                self.proxy.scope = scope;
            } else {
                tracing::warn!(
                    scope = %scope_raw,
                    "Ignoring invalid OPERANT_PROXY_SCOPE (valid: environment|operant|services)"
                );
            }
        }

        if let Ok(services_raw) = std::env::var("OPERANT_PROXY_SERVICES") {
            self.proxy.services = normalize_service_list(vec![services_raw]);
        }

        if let Err(error) = self.proxy.validate() {
            tracing::warn!("Invalid proxy configuration ignored: {error}");
            self.proxy.enabled = false;
        }

        if self.proxy.enabled && self.proxy.scope == ProxyScope::Environment {
            self.proxy.apply_to_process_env();
        }

        set_runtime_proxy_config(self.proxy.clone());

        if self.conversational_ai.enabled {
            tracing::warn!(
                "conversational_ai.enabled = true but conversational AI features are not yet \
                 implemented; this section is reserved for future use and will be ignored"
            );
        }

        // Slack channel-token env-var fallbacks. Resolved here (after the
        // file is parsed and all other overrides applied) so a config that
        // omits `bot_token` entirely still deserializes — channel
        // construction picks up the env value at startup. See #6237.
        // OPERANT_-prefixed variants take precedence over the bare names.
        if let Some(ref mut sl) = self.channels.slack {
            if sl.bot_token.as_deref().is_none_or(str::is_empty)
                && let Ok(v) = std::env::var("OPERANT_SLACK_BOT_TOKEN")
                    .or_else(|_| std::env::var("SLACK_BOT_TOKEN"))
                && !v.is_empty()
            {
                sl.bot_token = Some(v);
            }
            if sl.app_token.as_deref().is_none_or(str::is_empty)
                && let Ok(v) = std::env::var("OPERANT_SLACK_APP_TOKEN")
                    .or_else(|_| std::env::var("SLACK_APP_TOKEN"))
                && !v.is_empty()
            {
                sl.app_token = Some(v);
            }
        }
    }

    pub(crate) async fn resolve_config_path_for_save(&self) -> Result<PathBuf> {
        if self
            .config_path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty())
        {
            return Ok(self.config_path.clone());
        }

        let (default_operant_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
        let (operant_dir, _workspace_dir, source) =
            resolve_runtime_config_dirs(&default_operant_dir, &default_workspace_dir).await?;
        let file_name = self
            .config_path
            .file_name()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| std::ffi::OsStr::new("config.toml"));
        let resolved = operant_dir.join(file_name);
        tracing::warn!(
            path = %self.config_path.display(),
            resolved = %resolved.display(),
            source = source.as_str(),
            "Config path missing parent directory; resolving from runtime environment"
        );
        Ok(resolved)
    }

    /// Persist this config to disk, encrypting secrets and syncing to disk.
    pub async fn save(&self) -> Result<()> {
        // Encrypt secrets before serialization
        let mut config_to_save = self.clone();
        let config_path = self.resolve_config_path_for_save().await?;
        let operant_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?;
        let store = crate::secrets::SecretStore::new(operant_dir, self.secrets.encrypt);

        // Encrypt all #[secret]-annotated fields via Configurable derive
        config_to_save.encrypt_secrets(&store)?;

        let new_toml =
            toml::to_string_pretty(&config_to_save).context("Failed to serialize config")?;

        // If an existing config file is present, sync the new values onto it
        // to preserve comments and formatting. Otherwise, use the fresh serialization.
        let toml_str = if config_path.exists() {
            let existing = fs::read_to_string(&config_path).await.unwrap_or_default();
            if existing.is_empty() {
                new_toml
            } else {
                let new_table: toml::Table =
                    toml::from_str(&new_toml).context("Failed to round-trip serialized config")?;
                let mut doc: toml_edit::DocumentMut = existing
                    .parse()
                    .context("Failed to parse existing config for comment preservation")?;
                crate::migration::sync_table(doc.as_table_mut(), &new_table);
                doc.to_string()
            }
        } else {
            new_toml
        };

        let parent_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?;

        fs::create_dir_all(parent_dir).await.with_context(|| {
            format!(
                "Failed to create config directory: {}",
                parent_dir.display()
            )
        })?;

        let file_name = config_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("config.toml");
        let temp_path = parent_dir.join(format!(".{file_name}.tmp-{}", uuid::Uuid::new_v4()));
        let backup_path = parent_dir.join(format!("{file_name}.bak"));

        let mut temp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to create temporary config file: {}",
                    temp_path.display()
                )
            })?;
        temp_file
            .write_all(toml_str.as_bytes())
            .await
            .context("Failed to write temporary config contents")?;
        temp_file
            .sync_all()
            .await
            .context("Failed to fsync temporary config file")?;
        drop(temp_file);

        let had_existing_config = config_path.exists();
        if had_existing_config {
            fs::copy(&config_path, &backup_path)
                .await
                .with_context(|| {
                    format!(
                        "Failed to create config backup before atomic replace: {}",
                        backup_path.display()
                    )
                })?;
        }

        if let Err(e) = fs::rename(&temp_path, &config_path).await {
            let _ = fs::remove_file(&temp_path).await;
            if had_existing_config && backup_path.exists() {
                fs::copy(&backup_path, &config_path)
                    .await
                    .context("Failed to restore config backup")?;
            }
            anyhow::bail!("Failed to atomically replace config file: {e}");
        }

        #[cfg(unix)]
        {
            use std::{fs::Permissions, os::unix::fs::PermissionsExt};
            if let Err(err) = fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await
            {
                tracing::warn!(
                    "Failed to harden config permissions to 0600 at {}: {}",
                    config_path.display(),
                    err
                );
            }
        }

        sync_directory(parent_dir).await?;

        if had_existing_config {
            let _ = fs::remove_file(&backup_path).await;
        }

        Ok(())
    }
}

#[allow(clippy::unused_async)] // async needed on unix for tokio File I/O; no-op on other platforms
pub(crate) async fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let dir = File::open(path)
            .await
            .with_context(|| format!("Failed to open directory for fsync: {}", path.display()))?;
        dir.sync_all()
            .await
            .with_context(|| format!("Failed to fsync directory metadata: {}", path.display()))?;
        Ok(())
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x02000000;
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .with_context(|| format!("Failed to open directory for fsync: {}", path.display()))?;
        // FlushFileBuffers on directory handles returns ERROR_ACCESS_DENIED on
        // Windows (OS Error 5). This is expected — NTFS does not support
        // flushing directory metadata the same way Unix does. The individual
        // files have already been synced, so it is safe to ignore this error.
        if let Err(e) = dir.sync_all() {
            if e.raw_os_error() == Some(5) {
                tracing::trace!(
                    "Ignoring expected ACCESS_DENIED when fsyncing directory on Windows: {}",
                    path.display()
                );
            } else {
                return Err(e).with_context(|| {
                    format!("Failed to fsync directory metadata: {}", path.display())
                });
            }
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

// ── HasPropKind impls for config enums ──
// Scalars (bool, String, integers, floats) are covered by impl_prop_kind! in traits.rs.
// Config enums serialize as TOML strings and are classified as PropKind::Enum.
macro_rules! impl_enum_prop_kind {
    ($($ty:ty),+ $(,)?) => {
        $(impl HasPropKind for $ty { const PROP_KIND: PropKind = PropKind::Enum; })+
    };
}

impl_enum_prop_kind!(
    SwarmStrategy,
    HardwareTransport,
    McpTransport,
    ToolFilterGroupMode,
    SkillsPromptInjectionMode,
    FirecrawlMode,
    ProxyScope,
    SearchMode,
    CronScheduleDecl,
    StreamMode,
    WhatsAppWebMode,
    WhatsAppChatPolicy,
    LineDmPolicy,
    LineGroupPolicy,
    LarkReceiveMode,
    OtpMethod,
    SandboxBackend,
    AutonomyLevel,
);

impl HasPropKind for ModelPricing {
    // ModelPricing is a 2-field struct (`input`, `output`). Wire form is a
    // JSON object (e.g. `{"input": 1.0, "output": 2.5}`); the dashboard
    // renders a sub-form for the inner fields. `PropKind::Object` (vs
    // `String`) is what makes the round-trip through `Config::set_prop`
    // succeed — `parse_prop_value` parses the JSON into a TOML table so
    // serde deserializes it back into the typed `ModelPricing` instead
    // of failing on a TOML string (#6357 review).
    const PROP_KIND: PropKind = PropKind::Object;
}

impl HasPropKind for serde_json::Value {
    // `serde_json::Value` is an arbitrary JSON document, not an enum.
    // Classifying it as `Enum` previously made `enum_variants_for::<Value>()`
    // hand back the literal placeholder `"(unknown variants)"`, and the
    // dashboard form rendered fields like `providers.models.<key>.provider_extra`
    // as a single-option dropdown. `String` is the closest scalar kind —
    // the form renders a text input where the user pastes raw JSON.
    // Round-trip via `set_prop` stays correct: serde deserializes the TOML
    // string back into `Value::String(...)`. Power users editing complex
    // objects still use `operant config set --json` or hand-edit the
    // `config.toml`.
    const PROP_KIND: PropKind = PropKind::String;
}
