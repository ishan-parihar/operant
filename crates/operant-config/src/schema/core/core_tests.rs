//! Core config tests (verbatim body of the former inline `mod tests`).
use super::*;
use crate::autonomy::AutonomyLevel;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};
use tokio::test;

// ── Tilde expansion ───────────────────────────────────────

#[test]
async fn expand_tilde_path_handles_absolute_path() {
    let path = expand_tilde_path("/absolute/path");
    assert_eq!(path, PathBuf::from("/absolute/path"));
}

#[test]
async fn expand_tilde_path_handles_relative_path() {
    let path = expand_tilde_path("relative/path");
    assert_eq!(path, PathBuf::from("relative/path"));
}

#[test]
async fn expand_tilde_path_expands_tilde_when_home_set() {
    // This test verifies that tilde expansion works when HOME is set.
    // In normal environments, HOME is set, so ~ should expand.
    let path = expand_tilde_path("~/.operant");
    // The path should not literally start with '~' if HOME is set
    // (it should be expanded to the actual home directory)
    if std::env::var("HOME").is_ok() {
        assert!(
            !path.to_string_lossy().starts_with('~'),
            "Tilde should be expanded when HOME is set"
        );
    }
}

// ── Defaults ─────────────────────────────────────────────

fn has_test_table(raw: &str, table: &str) -> bool {
    let exact = format!("[{table}]");
    let nested = format!("[{table}.");
    raw.lines()
        .map(str::trim)
        .any(|line| line == exact || line.starts_with(&nested))
}

fn parse_test_config(raw: &str) -> Config {
    let mut merged = raw.trim().to_string();
    for table in [
        "data_retention",
        "cloud_ops",
        "conversational_ai",
        "security",
        "security_ops",
    ] {
        if has_test_table(&merged, table) {
            continue;
        }
        if !merged.is_empty() {
            merged.push_str("\n\n");
        }
        merged.push('[');
        merged.push_str(table);
        merged.push(']');
    }
    merged.push('\n');
    // Deserialize through V1Compat to handle legacy top-level fields.
    let compat: crate::migration::V1Compat = toml::from_str(&merged).unwrap();
    let mut config = compat.into_config();
    config.autonomy.ensure_default_auto_approve();
    config
}

#[test]
async fn http_request_config_default_has_correct_values() {
    let cfg = HttpRequestConfig::default();
    assert_eq!(cfg.timeout_secs, 30);
    assert_eq!(cfg.max_response_size, 1_000_000);
    assert!(cfg.enabled);
    assert_eq!(cfg.allowed_domains, vec!["*".to_string()]);
}

#[test]
async fn config_default_has_sane_values() {
    let c = Config::default();
    // V2: no fallback provider by default — set during onboarding.
    assert!(c.providers.fallback.is_none());
    assert!(c.providers.fallback_provider().is_none());
    assert!(!c.skills.open_skills_enabled);
    assert!(!c.skills.allow_scripts);
    assert!(!c.skills.install_suggestions.enabled);
    assert_eq!(
        c.skills.prompt_injection_mode,
        SkillsPromptInjectionMode::Full
    );
    assert!(c.workspace_dir.to_string_lossy().contains("workspace"));
    assert!(c.config_path.to_string_lossy().contains("config.toml"));
}

#[test]
async fn skills_install_suggestions_config_deserializes_enabled() {
    let c = parse_test_config(
        r#"
[skills.install_suggestions]
enabled = true
"#,
    );

    assert!(c.skills.install_suggestions.enabled);
}

#[test]
async fn skills_install_suggestions_config_accepts_hyphen_alias() {
    let c = parse_test_config(
        r#"
[skills.install-suggestions]
enabled = true
"#,
    );

    assert!(c.skills.install_suggestions.enabled);
}

#[derive(Clone, Default)]
struct SharedLogBuffer(Arc<StdMutex<Vec<u8>>>);

struct SharedLogWriter(Arc<StdMutex<Vec<u8>>>);

impl SharedLogBuffer {
    fn captured(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogBuffer {
    type Writer = SharedLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        SharedLogWriter(self.0.clone())
    }
}

impl io::Write for SharedLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
async fn config_dir_creation_error_mentions_openrc_and_path() {
    let msg = config_dir_creation_error(Path::new("/etc/operant"));
    assert!(msg.contains("/etc/operant"));
    assert!(msg.contains("OpenRC"));
    assert!(msg.contains("operant"));
}

#[test]
async fn config_schema_export_contains_expected_contract_shape() {
    #[cfg(feature = "schema-export")]
    let schema = schemars::schema_for!(Config);
    let schema_json = serde_json::to_value(&schema).expect("schema should serialize to json");

    // schemars 0.8 uses draft-07; the exact URL format may vary across versions
    let schema_version = schema_json
        .get("$schema")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(
        schema_version.contains("draft-07"),
        "schema should use JSON Schema draft-07, got: {schema_version}"
    );

    let properties = schema_json
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("schema should expose top-level properties");

    assert!(properties.contains_key("providers"));
    assert!(properties.contains_key("skills"));
    assert!(properties.contains_key("gateway"));
    assert!(properties.contains_key("channels"));
    assert!(!properties.contains_key("workspace_dir"));
    assert!(!properties.contains_key("config_path"));
    // These fields are now #[serde(skip)] cache fields, not in schema.
    assert!(!properties.contains_key("default_provider"));
    assert!(!properties.contains_key("api_key"));
    assert!(!properties.contains_key("default_model"));

    // schemars 0.8 (draft-07) uses `definitions`; draft-2020-12 uses `$defs`
    assert!(
        schema_json
            .get("definitions")
            .or_else(|| schema_json.get("$defs"))
            .and_then(serde_json::Value::as_object)
            .is_some(),
        "schema should include reusable type definitions"
    );
}

#[cfg(unix)]
#[test]
async fn save_sets_config_permissions_on_new_file() {
    let temp = TempDir::new().expect("temp dir");
    let config_path = temp.path().join("config.toml");
    let workspace_dir = temp.path().join("workspace");

    let config = Config {
        config_path: config_path.clone(),
        workspace_dir,
        ..Default::default()
    };

    config.save().await.expect("save config");

    let mode = std::fs::metadata(&config_path)
        .expect("config metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
async fn observability_config_default() {
    let o = ObservabilityConfig::default();
    assert_eq!(o.backend, "none");
    assert_eq!(o.runtime_trace_mode, "none");
    assert_eq!(o.runtime_trace_path, "state/runtime-trace.jsonl");
    assert_eq!(o.runtime_trace_max_entries, 200);
}

#[test]
async fn autonomy_config_default() {
    let a = AutonomyConfig::default();
    assert_eq!(a.level, AutonomyLevel::Supervised);
    assert!(a.workspace_only);
    assert!(a.allowed_commands.contains(&"git".to_string()));
    assert!(a.allowed_commands.contains(&"cargo".to_string()));
    assert!(a.forbidden_paths.contains(&"/etc".to_string()));
    assert_eq!(a.max_actions_per_hour, 20);
    assert_eq!(a.max_cost_per_day_cents, 500);
    assert!(a.require_approval_for_medium_risk);
    assert!(a.block_high_risk_commands);
    assert!(a.shell_env_passthrough.is_empty());
}

#[test]
async fn runtime_config_default() {
    let r = RuntimeConfig::default();
    assert_eq!(r.kind, "native");
    assert_eq!(r.docker.image, "alpine:3.20");
    assert_eq!(r.docker.network, "none");
    assert_eq!(r.docker.memory_limit_mb, Some(512));
    assert_eq!(r.docker.cpu_limit, Some(1.0));
    assert!(r.docker.read_only_rootfs);
    assert!(r.docker.mount_workspace);
}

#[test]
async fn heartbeat_config_default() {
    let h = HeartbeatConfig::default();
    assert!(h.enabled);
    assert_eq!(h.interval_minutes, 30);
    assert!(h.message.is_none());
    assert!(h.target.is_none());
    assert!(h.to.is_none());
}

#[test]
async fn heartbeat_config_parses_delivery_aliases() {
    let raw = r#"
enabled = true
interval_minutes = 10
message = "Ping"
channel = "telegram"
recipient = "42"
"#;
    let parsed: HeartbeatConfig = toml::from_str(raw).unwrap();
    assert!(parsed.enabled);
    assert_eq!(parsed.interval_minutes, 10);
    assert_eq!(parsed.message.as_deref(), Some("Ping"));
    assert_eq!(parsed.target.as_deref(), Some("telegram"));
    assert_eq!(parsed.to.as_deref(), Some("42"));
}

#[test]
async fn cron_config_default() {
    let c = CronConfig::default();
    assert!(c.enabled);
    assert_eq!(c.max_run_history, 50);
}

#[test]
async fn cron_config_serde_roundtrip() {
    let c = CronConfig {
        enabled: false,
        catch_up_on_startup: false,
        max_run_history: 100,
        jobs: Vec::new(),
    };
    let json = serde_json::to_string(&c).unwrap();
    let parsed: CronConfig = serde_json::from_str(&json).unwrap();
    assert!(!parsed.enabled);
    assert!(!parsed.catch_up_on_startup);
    assert_eq!(parsed.max_run_history, 100);
}

#[test]
async fn config_defaults_cron_when_section_missing() {
    let toml_str = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;

    let parsed = parse_test_config(toml_str);
    assert!(parsed.cron.enabled);
    assert!(parsed.cron.catch_up_on_startup);
    assert_eq!(parsed.cron.max_run_history, 50);
}

#[test]
async fn memory_config_default_hygiene_settings() {
    let m = MemoryConfig::default();
    assert_eq!(m.backend, "agentmemory");
    assert!(m.auto_save);
    assert!(m.hygiene_enabled);
    assert_eq!(m.archive_after_days, 7);
    assert_eq!(m.purge_after_days, 30);
    assert_eq!(m.conversation_retention_days, 30);
    assert!(m.sqlite_open_timeout_secs.is_none());
    assert_eq!(m.search_mode, SearchMode::Hybrid);
}

#[test]
async fn search_mode_config_deserialization() {
    let toml_str = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
search_mode = "bm25"
"#;
    let parsed = parse_test_config(toml_str);
    assert_eq!(parsed.memory.search_mode, SearchMode::Bm25);

    let toml_str_embedding = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
search_mode = "embedding"
"#;
    let parsed = parse_test_config(toml_str_embedding);
    assert_eq!(parsed.memory.search_mode, SearchMode::Embedding);

    let toml_str_hybrid = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
search_mode = "hybrid"
"#;
    let parsed = parse_test_config(toml_str_hybrid);
    assert_eq!(parsed.memory.search_mode, SearchMode::Hybrid);
}

#[test]
async fn search_mode_defaults_to_hybrid_when_omitted() {
    let toml_str = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
"#;
    let parsed = parse_test_config(toml_str);
    assert_eq!(parsed.memory.search_mode, SearchMode::Hybrid);
}

#[test]
async fn search_mode_serde_roundtrip() {
    let json_bm25 = serde_json::to_string(&SearchMode::Bm25).unwrap();
    assert_eq!(json_bm25, "\"bm25\"");
    let parsed: SearchMode = serde_json::from_str(&json_bm25).unwrap();
    assert_eq!(parsed, SearchMode::Bm25);

    let json_embedding = serde_json::to_string(&SearchMode::Embedding).unwrap();
    assert_eq!(json_embedding, "\"embedding\"");
    let parsed: SearchMode = serde_json::from_str(&json_embedding).unwrap();
    assert_eq!(parsed, SearchMode::Embedding);

    let json_hybrid = serde_json::to_string(&SearchMode::Hybrid).unwrap();
    assert_eq!(json_hybrid, "\"hybrid\"");
    let parsed: SearchMode = serde_json::from_str(&json_hybrid).unwrap();
    assert_eq!(parsed, SearchMode::Hybrid);
}

#[test]
async fn storage_provider_config_defaults() {
    let storage = StorageConfig::default();
    assert!(storage.provider.config.provider.is_empty());
    assert!(storage.provider.config.db_url.is_none());
    assert_eq!(storage.provider.config.schema, "public");
    assert_eq!(storage.provider.config.table, "memories");
    assert!(storage.provider.config.connect_timeout_secs.is_none());
}

#[test]
async fn memory_config_pgvector_defaults() {
    let memory = MemoryConfig::default();
    assert!(!memory.postgres.vector_enabled);
    assert_eq!(memory.postgres.vector_dimensions, 1536);
}

#[test]
async fn memory_config_pgvector_roundtrip() {
    // `auto_save` is required on MemoryConfig and unrelated to the pgvector
    // fields these tests exercise. Including it keeps the fixture parseable
    // without coupling the test to schema-default behavior on auto_save.
    let toml = r#"
            backend = "postgres"
            auto_save = true
            [postgres]
            vector_enabled = true
            vector_dimensions = 768
        "#;
    let parsed: MemoryConfig = toml::from_str(toml).unwrap();
    assert!(parsed.postgres.vector_enabled);
    assert_eq!(parsed.postgres.vector_dimensions, 768);

    let serialized = toml::to_string(&parsed).unwrap();
    let reparsed: MemoryConfig = toml::from_str(&serialized).unwrap();
    assert!(reparsed.postgres.vector_enabled);
    assert_eq!(reparsed.postgres.vector_dimensions, 768);
}

#[test]
async fn memory_config_pgvector_defaults_when_omitted() {
    let toml = r#"
            backend = "postgres"
            auto_save = true
        "#;
    let parsed: MemoryConfig = toml::from_str(toml).unwrap();
    assert!(!parsed.postgres.vector_enabled);
    assert_eq!(parsed.postgres.vector_dimensions, 1536);
}

#[test]
async fn model_provider_config_ollama_tuning_fields_roundtrip() {
    let toml = r#"
            ollama_num_ctx = 16384
            ollama_num_predict = 4096
            ollama_temperature_override = 0.5
        "#;
    let parsed: ModelProviderConfig = toml::from_str(toml).unwrap();
    assert_eq!(parsed.ollama_num_ctx, Some(16384));
    assert_eq!(parsed.ollama_num_predict, Some(4096));
    assert_eq!(parsed.ollama_temperature_override, Some(0.5));

    let serialized = toml::to_string(&parsed).unwrap();
    let reparsed: ModelProviderConfig = toml::from_str(&serialized).unwrap();
    assert_eq!(reparsed.ollama_num_ctx, Some(16384));
    assert_eq!(reparsed.ollama_num_predict, Some(4096));
    assert_eq!(reparsed.ollama_temperature_override, Some(0.5));
}

#[test]
async fn model_provider_config_ollama_tuning_fields_default_to_none() {
    let toml = r#"
            api_key = "sk-test"
        "#;
    let parsed: ModelProviderConfig = toml::from_str(toml).unwrap();
    assert!(parsed.ollama_num_ctx.is_none());
    assert!(parsed.ollama_num_predict.is_none());
    assert!(parsed.ollama_temperature_override.is_none());
}

#[test]
async fn channels_default() {
    let c = ChannelsConfig::default();
    assert!(c.cli);
    assert!(c.telegram.is_none());
    assert!(c.discord.is_none());
    assert!(!c.show_tool_calls);
}

// ── Serde round-trip ─────────────────────────────────────

#[test]
async fn config_toml_roundtrip() {
    let config = Config {
        schema_version: crate::migration::CURRENT_SCHEMA_VERSION,
        providers: crate::providers::ProvidersConfig {
            fallback: Some("openrouter".into()),
            fallback_chain: Vec::new(),
            models: {
                let mut m = HashMap::new();
                m.insert(
                    "openrouter".into(),
                    ModelProviderConfig {
                        api_key: Some("sk-test-key".into()),
                        model: Some("gpt-4o".into()),
                        temperature: Some(0.5),
                        timeout_secs: Some(120),
                        ..Default::default()
                    },
                );
                m
            },
            model_routes: Vec::new(),
            embedding_routes: Vec::new(),
        },
        workspace_dir: PathBuf::from("/tmp/test/workspace"),
        config_path: PathBuf::from("/tmp/test/config.toml"),
        observability: ObservabilityConfig {
            backend: "log".into(),
            ..ObservabilityConfig::default()
        },
        autonomy: AutonomyConfig {
            level: AutonomyLevel::Full,
            workspace_only: false,
            allowed_commands: vec!["docker".into()],
            forbidden_paths: vec!["/secret".into()],
            max_actions_per_hour: 50,
            max_cost_per_day_cents: 1000,
            require_approval_for_medium_risk: false,
            block_high_risk_commands: true,
            shell_env_passthrough: vec!["DATABASE_URL".into()],
            auto_approve: vec!["file_read".into()],
            always_ask: vec![],
            allowed_roots: vec![],
            non_cli_excluded_tools: vec![],
            shell_timeout_secs: default_shell_timeout_secs(),
        },
        trust: crate::scattered_types::TrustConfig::default(),
        backup: BackupConfig::default(),
        data_retention: DataRetentionConfig::default(),
        cloud_ops: CloudOpsConfig::default(),
        conversational_ai: ConversationalAiConfig::default(),
        security: SecurityConfig::default(),
        security_ops: SecurityOpsConfig::default(),
        runtime: RuntimeConfig {
            kind: "docker".into(),
            ..RuntimeConfig::default()
        },
        reliability: ReliabilityConfig::default(),
        scheduler: SchedulerConfig::default(),
        skills: SkillsConfig::default(),
        pipeline: PipelineConfig::default(),
        query_classification: QueryClassificationConfig::default(),
        heartbeat: HeartbeatConfig {
            enabled: true,
            interval_minutes: 15,
            two_phase: true,
            message: Some("Check London time".into()),
            target: Some("telegram".into()),
            to: Some("123456".into()),
            ..HeartbeatConfig::default()
        },
        cron: CronConfig::default(),
        channels: ChannelsConfig {
            cli: true,
            telegram: Some(TelegramConfig {
                enabled: true,
                dm_topics_enabled: false,
                dm_topic_name: default_telegram_dm_topic_name(),
                bot_token: "123:ABC".into(),
                allowed_users: vec!["user1".into()],
                stream_mode: StreamMode::default(),
                draft_update_interval_ms: default_draft_update_interval_ms(),
                interrupt_on_new_message: false,
                mention_only: false,
                ack_reactions: None,
                proxy_url: None,
                approval_timeout_secs: default_telegram_approval_timeout_secs(),
                disable_link_previews: false,
                typing_cooldown_seconds: default_telegram_typing_cooldown_secs(),
                fallback_ips: vec![],
            }),
            discord: None,
            discord_history: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: None,
            matrix: None,
            signal: None,
            whatsapp: None,
            linq: None,
            wati: None,
            nextcloud_talk: None,
            email: None,
            gmail_push: None,
            irc: None,
            lark: None,
            line: None,
            feishu: None,
            dingtalk: None,
            wecom: None,
            wechat: None,
            qq: None,
            twitter: None,
            mochat: None,
            #[cfg(feature = "channel-nostr")]
            nostr: None,
            clawdtalk: None,
            reddit: None,
            bluesky: None,
            voice_call: None,
            voice_duplex: None,
            #[cfg(feature = "voice-wake")]
            voice_wake: None,
            mqtt: None,
            message_timeout_secs: 300,
            ack_reactions: true,
            show_tool_calls: true,
            session_persistence: true,
            session_backend: default_session_backend(),
            session_ttl_hours: 0,
            debounce_ms: 0,
        },
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
        agent: AgentConfig::default(),
        pacing: PacingConfig::default(),
        identity: IdentityConfig::default(),
        cost: CostConfig::default(),
        peripherals: PeripheralsConfig::default(),
        delegate: DelegateToolConfig::default(),
        agents: HashMap::new(),
        swarms: HashMap::new(),
        hooks: HooksConfig::default(),
        hardware: HardwareConfig::default(),
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
    };
    // Provider fields are now resolved directly — no cache needed.

    let toml_str = toml::to_string_pretty(&config).unwrap();
    let parsed = parse_test_config(&toml_str);

    assert_eq!(parsed.providers.fallback, config.providers.fallback);
    assert_eq!(parsed.observability.backend, "log");
    assert_eq!(parsed.observability.runtime_trace_mode, "none");
    assert_eq!(parsed.autonomy.level, AutonomyLevel::Full);
    assert!(!parsed.autonomy.workspace_only);
    assert_eq!(parsed.runtime.kind, "docker");
    assert!(parsed.heartbeat.enabled);
    assert_eq!(parsed.heartbeat.interval_minutes, 15);
    assert_eq!(
        parsed.heartbeat.message.as_deref(),
        Some("Check London time")
    );
    assert_eq!(parsed.heartbeat.target.as_deref(), Some("telegram"));
    assert_eq!(parsed.heartbeat.to.as_deref(), Some("123456"));
    assert!(parsed.channels.telegram.is_some());
    assert_eq!(parsed.channels.telegram.unwrap().bot_token, "123:ABC");
}

#[test]
async fn config_minimal_toml_uses_defaults() {
    let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
    let parsed = parse_test_config(minimal);
    assert!(
        parsed
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.as_deref())
            .is_none()
    );
    assert_eq!(parsed.observability.backend, "none");
    assert_eq!(parsed.observability.runtime_trace_mode, "none");
    assert_eq!(parsed.autonomy.level, AutonomyLevel::Supervised);
    assert_eq!(parsed.runtime.kind, "native");
    assert!(parsed.heartbeat.enabled);
    assert!(parsed.channels.cli);
    assert!(parsed.memory.hygiene_enabled);
    assert_eq!(parsed.memory.archive_after_days, 7);
    assert_eq!(parsed.memory.purge_after_days, 30);
    assert_eq!(parsed.memory.conversation_retention_days, 30);
    // Temperature migrated to the fallback provider entry
    assert!(
        (parsed
            .providers
            .fallback_provider()
            .and_then(|e| e.temperature)
            .unwrap_or(0.7)
            - 0.7)
            .abs()
            < f64::EPSILON
    );
    assert_eq!(
        parsed
            .providers
            .fallback_provider()
            .and_then(|e| e.timeout_secs)
            .unwrap_or(120),
        DEFAULT_DELEGATE_TIMEOUT_SECS
    );
}

/// Regression test for #4171: the `[autonomy]` section must not be
/// silently dropped when parsing config TOML.
#[test]
async fn autonomy_section_is_not_silently_ignored() {
    let raw = r#"
default_temperature = 0.7

[autonomy]
level = "full"
max_actions_per_hour = 99
auto_approve = ["file_read", "memory_recall", "http_request"]
"#;
    let parsed = parse_test_config(raw);
    assert_eq!(
        parsed.autonomy.level,
        AutonomyLevel::Full,
        "autonomy.level must be parsed from config (was silently defaulting to Supervised)"
    );
    assert_eq!(
        parsed.autonomy.max_actions_per_hour, 99,
        "autonomy.max_actions_per_hour must be parsed from config"
    );
    assert!(
        parsed
            .autonomy
            .auto_approve
            .contains(&"http_request".to_string()),
        "autonomy.auto_approve must include http_request from config"
    );
}

/// Regression test for #4247: when a user provides a custom auto_approve
/// list, the built-in defaults must still be present.
#[test]
async fn auto_approve_merges_user_entries_with_defaults() {
    let raw = r#"
default_temperature = 0.7

[autonomy]
auto_approve = ["my_custom_tool", "another_tool"]
"#;
    let parsed = parse_test_config(raw);
    // User entries are preserved
    assert!(
        parsed
            .autonomy
            .auto_approve
            .contains(&"my_custom_tool".to_string()),
        "user-supplied tool must remain in auto_approve"
    );
    assert!(
        parsed
            .autonomy
            .auto_approve
            .contains(&"another_tool".to_string()),
        "user-supplied tool must remain in auto_approve"
    );
    // Defaults are merged in
    for default_tool in &[
        "file_read",
        "memory_recall",
        "weather",
        "calculator",
        "web_fetch",
    ] {
        assert!(
            parsed
                .autonomy
                .auto_approve
                .contains(&String::from(*default_tool)),
            "default tool '{default_tool}' must be present in auto_approve even when user provides custom list"
        );
    }
}

/// Regression test: empty auto_approve still gets defaults merged.
#[test]
async fn auto_approve_empty_list_gets_defaults() {
    let raw = r#"
default_temperature = 0.7

[autonomy]
auto_approve = []
"#;
    let parsed = parse_test_config(raw);
    let defaults = default_auto_approve();
    for tool in &defaults {
        assert!(
            parsed.autonomy.auto_approve.contains(tool),
            "default tool '{tool}' must be present even when user sets auto_approve = []"
        );
    }
}

/// When no autonomy section is provided, defaults are applied normally.
#[test]
async fn auto_approve_defaults_when_no_autonomy_section() {
    let raw = r#"
default_temperature = 0.7
"#;
    let parsed = parse_test_config(raw);
    let defaults = default_auto_approve();
    for tool in &defaults {
        assert!(
            parsed.autonomy.auto_approve.contains(tool),
            "default tool '{tool}' must be present when no [autonomy] section"
        );
    }
}

/// Duplicates are not introduced when ensure_default_auto_approve runs
/// on a list that already contains the defaults.
#[test]
async fn auto_approve_no_duplicates() {
    let raw = r#"
default_temperature = 0.7

[autonomy]
auto_approve = ["weather", "file_read"]
"#;
    let parsed = parse_test_config(raw);
    let weather_count = parsed
        .autonomy
        .auto_approve
        .iter()
        .filter(|t| *t == "weather")
        .count();
    assert_eq!(weather_count, 1, "weather must not be duplicated");
    let file_read_count = parsed
        .autonomy
        .auto_approve
        .iter()
        .filter(|t| *t == "file_read")
        .count();
    assert_eq!(file_read_count, 1, "file_read must not be duplicated");
}

#[test]
async fn provider_timeout_secs_parses_from_toml() {
    let raw = r#"
default_temperature = 0.7
provider_timeout_secs = 300
"#;
    let parsed = parse_test_config(raw);
    assert_eq!(
        parsed
            .providers
            .fallback_provider()
            .and_then(|e| e.timeout_secs)
            .unwrap_or(120),
        300
    );
}

#[test]
async fn parse_extra_headers_env_basic() {
    let headers = parse_extra_headers_env("User-Agent:MyApp/1.0,X-Title:operant");
    assert_eq!(headers.len(), 2);
    assert_eq!(
        headers[0],
        ("User-Agent".to_string(), "MyApp/1.0".to_string())
    );
    assert_eq!(headers[1], ("X-Title".to_string(), "operant".to_string()));
}

#[test]
async fn parse_extra_headers_env_with_url_value() {
    let headers = parse_extra_headers_env("HTTP-Referer:https://github.com/zeroclaw-labs/operant");
    assert_eq!(headers.len(), 1);
    // Only splits on first colon, preserving URL colons in value
    assert_eq!(headers[0].0, "HTTP-Referer");
    assert_eq!(headers[0].1, "https://github.com/zeroclaw-labs/operant");
}

#[test]
async fn parse_extra_headers_env_empty_string() {
    let headers = parse_extra_headers_env("");
    assert!(headers.is_empty());
}

#[test]
async fn parse_extra_headers_env_whitespace_trimming() {
    let headers = parse_extra_headers_env("  X-Title : operant , User-Agent : cli/1.0 ");
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[0], ("X-Title".to_string(), "operant".to_string()));
    assert_eq!(
        headers[1],
        ("User-Agent".to_string(), "cli/1.0".to_string())
    );
}

#[test]
async fn parse_extra_headers_env_skips_malformed() {
    let headers = parse_extra_headers_env("X-Valid:value,no-colon-here,Another:ok");
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[0], ("X-Valid".to_string(), "value".to_string()));
    assert_eq!(headers[1], ("Another".to_string(), "ok".to_string()));
}

#[test]
async fn parse_extra_headers_env_skips_empty_key() {
    let headers = parse_extra_headers_env(":value,X-Valid:ok");
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0], ("X-Valid".to_string(), "ok".to_string()));
}

#[test]
async fn parse_extra_headers_env_allows_empty_value() {
    let headers = parse_extra_headers_env("X-Empty:");
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0], ("X-Empty".to_string(), String::new()));
}

#[test]
async fn parse_extra_headers_env_trailing_comma() {
    let headers = parse_extra_headers_env("X-Title:operant,");
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0], ("X-Title".to_string(), "operant".to_string()));
}

#[test]
async fn extra_headers_parses_from_toml() {
    let raw = r#"
default_temperature = 0.7

[extra_headers]
User-Agent = "MyApp/1.0"
X-Title = "operant"
"#;
    let parsed = parse_test_config(raw);
    let headers = &parsed
        .providers
        .fallback_provider()
        .expect("fallback provider")
        .extra_headers;
    assert_eq!(headers.len(), 2);
    assert_eq!(headers.get("User-Agent").unwrap(), "MyApp/1.0");
    assert_eq!(headers.get("X-Title").unwrap(), "operant");
}

#[test]
async fn extra_headers_defaults_to_empty() {
    let raw = r#"
default_temperature = 0.7
"#;
    let parsed = parse_test_config(raw);
    assert!(
        parsed
            .providers
            .fallback_provider()
            .map(|e| e.extra_headers.is_empty())
            .unwrap_or(true)
    );
}

#[test]
async fn storage_provider_dburl_alias_deserializes() {
    let raw = r#"
default_temperature = 0.7

[storage.provider.config]
provider = "qdrant"
dbURL = "http://localhost:6333"
schema = "public"
table = "memories"
connect_timeout_secs = 12
"#;

    let parsed = parse_test_config(raw);
    assert_eq!(parsed.storage.provider.config.provider, "qdrant");
    assert_eq!(
        parsed.storage.provider.config.db_url.as_deref(),
        Some("http://localhost:6333")
    );
    assert_eq!(parsed.storage.provider.config.schema, "public");
    assert_eq!(parsed.storage.provider.config.table, "memories");
    assert_eq!(
        parsed.storage.provider.config.connect_timeout_secs,
        Some(12)
    );
}

#[test]
async fn runtime_reasoning_enabled_deserializes() {
    let raw = r#"
default_temperature = 0.7

[runtime]
reasoning_enabled = false
"#;

    let parsed = parse_test_config(raw);
    assert_eq!(parsed.runtime.reasoning_enabled, Some(false));
}

#[test]
async fn runtime_reasoning_effort_deserializes() {
    let raw = r#"
default_temperature = 0.7

[runtime]
reasoning_effort = "HIGH"
"#;

    let parsed: Config = toml::from_str(raw).unwrap();
    assert_eq!(parsed.runtime.reasoning_effort.as_deref(), Some("high"));
}

#[test]
async fn runtime_reasoning_effort_rejects_invalid_values() {
    let raw = r#"
default_temperature = 0.7

[runtime]
reasoning_effort = "turbo"
"#;

    let error = toml::from_str::<Config>(raw).expect_err("invalid value should fail");
    assert!(error.to_string().contains("reasoning_effort"));
}

#[test]
async fn agent_config_defaults() {
    let cfg = AgentConfig::default();
    assert!(cfg.compact_context);
    assert_eq!(cfg.max_tool_iterations, 10);
    assert_eq!(cfg.max_history_messages, 50);
    assert!(!cfg.parallel_tools);
    assert_eq!(cfg.tool_dispatcher, "auto");
}

#[test]
async fn agent_config_deserializes() {
    let raw = r#"
default_temperature = 0.7
[agent]
compact_context = true
max_tool_iterations = 20
max_history_messages = 80
parallel_tools = true
tool_dispatcher = "xml"
"#;
    let parsed = parse_test_config(raw);
    assert!(parsed.agent.compact_context);
    assert_eq!(parsed.agent.max_tool_iterations, 20);
    assert_eq!(parsed.agent.max_history_messages, 80);
    assert!(parsed.agent.parallel_tools);
    assert_eq!(parsed.agent.tool_dispatcher, "xml");
}

#[test]
async fn agent_config_evolution_nudge_intervals_deserialize() {
    // R24: the gateway path (runtime Agent) reads these from `[agent]`;
    // they drive the per-turn memory-review and skill-nudge triggers.
    let raw = r#"
default_temperature = 0.7
[agent]
memory_nudge_interval = 3
creation_nudge_interval = 7
"#;
    let parsed: Config = toml::from_str(raw).unwrap();
    assert_eq!(parsed.agent.memory_nudge_interval, 3);
    assert_eq!(parsed.agent.creation_nudge_interval, 7);
}

#[test]
async fn agent_config_evolution_nudge_intervals_default_to_ten() {
    let parsed: Config = toml::from_str(
        r#"default_temperature = 0.7
[agent]
"#,
    )
    .unwrap();
    assert_eq!(parsed.agent.memory_nudge_interval, 10);
    assert_eq!(parsed.agent.creation_nudge_interval, 10);
}

#[test]
async fn pacing_config_defaults_are_all_none_or_empty() {
    let cfg = PacingConfig::default();
    assert!(cfg.step_timeout_secs.is_none());
    assert!(cfg.loop_detection_min_elapsed_secs.is_none());
    assert!(cfg.loop_ignore_tools.is_empty());
    assert!(cfg.message_timeout_scale_max.is_none());
}

#[test]
async fn pacing_config_deserializes_from_toml() {
    let raw = r#"
default_temperature = 0.7
[pacing]
step_timeout_secs = 120
loop_detection_min_elapsed_secs = 60
loop_ignore_tools = ["browser_screenshot", "browser_navigate"]
message_timeout_scale_max = 8
"#;
    let parsed: Config = toml::from_str(raw).unwrap();
    assert_eq!(parsed.pacing.step_timeout_secs, Some(120));
    assert_eq!(parsed.pacing.loop_detection_min_elapsed_secs, Some(60));
    assert_eq!(
        parsed.pacing.loop_ignore_tools,
        vec!["browser_screenshot", "browser_navigate"]
    );
    assert_eq!(parsed.pacing.message_timeout_scale_max, Some(8));
}

#[test]
async fn pacing_config_absent_preserves_defaults() {
    let raw = r#"
default_temperature = 0.7
"#;
    let parsed: Config = toml::from_str(raw).unwrap();
    assert!(parsed.pacing.step_timeout_secs.is_none());
    assert!(parsed.pacing.loop_detection_min_elapsed_secs.is_none());
    assert!(parsed.pacing.loop_ignore_tools.is_empty());
    assert!(parsed.pacing.message_timeout_scale_max.is_none());
}

#[tokio::test]
async fn sync_directory_handles_existing_directory() {
    let dir = std::env::temp_dir().join(format!(
        "operant_test_sync_directory_{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).await.unwrap();

    sync_directory(&dir).await.unwrap();

    let _ = fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn config_save_and_load_tmpdir() {
    let dir = std::env::temp_dir().join("operant_test_config");
    let _ = fs::remove_dir_all(&dir).await;
    fs::create_dir_all(&dir).await.unwrap();

    let config_path = dir.join("config.toml");
    let mut providers = crate::providers::ProvidersConfig {
        fallback: Some("openrouter".into()),
        ..Default::default()
    };
    providers.models.insert(
        "openrouter".into(),
        ModelProviderConfig {
            api_key: Some("sk-roundtrip".into()),
            model: Some("test-model".into()),
            temperature: Some(0.9),
            timeout_secs: Some(120),
            ..Default::default()
        },
    );
    let config = Config {
        schema_version: crate::migration::CURRENT_SCHEMA_VERSION,
        providers,
        workspace_dir: dir.join("workspace"),
        config_path: config_path.clone(),
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
        skills: SkillsConfig::default(),
        pipeline: PipelineConfig::default(),
        query_classification: QueryClassificationConfig::default(),
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
        agent: AgentConfig::default(),
        pacing: PacingConfig::default(),
        identity: IdentityConfig::default(),
        cost: CostConfig::default(),
        peripherals: PeripheralsConfig::default(),
        delegate: DelegateToolConfig::default(),
        agents: HashMap::new(),
        swarms: HashMap::new(),
        hooks: HooksConfig::default(),
        hardware: HardwareConfig::default(),
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
    };

    // Provider fields are now resolved directly — no cache needed.
    config.save().await.unwrap();
    assert!(config_path.exists());

    let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
    let compat: crate::migration::V1Compat = toml::from_str(&contents).unwrap();
    let loaded = compat.into_config();
    let entry = &loaded.providers.models["openrouter"];
    assert!(
        entry
            .api_key
            .as_deref()
            .is_some_and(crate::secrets::SecretStore::is_encrypted)
    );
    let store = crate::secrets::SecretStore::new(&dir, true);
    let decrypted = store.decrypt(entry.api_key.as_deref().unwrap()).unwrap();
    assert_eq!(decrypted, "sk-roundtrip");
    assert_eq!(entry.model.as_deref(), Some("test-model"));
    assert!(
        entry
            .temperature
            .is_some_and(|t| (t - 0.9).abs() < f64::EPSILON)
    );

    let _ = fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn config_save_encrypts_nested_credentials() {
    let dir = std::env::temp_dir().join(format!(
        "operant_test_nested_credentials_{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).await.unwrap();

    let mut config = Config {
        workspace_dir: dir.join("workspace"),
        config_path: dir.join("config.toml"),
        ..Default::default()
    };
    config.providers.fallback = Some("default".into());
    config.providers.models.insert(
        "default".into(),
        ModelProviderConfig {
            api_key: Some("root-credential".into()),
            ..Default::default()
        },
    );
    // Provider fields are now resolved directly — no cache needed.
    config.composio.api_key = Some("composio-credential".into());
    config.browser.computer_use.api_key = Some("browser-credential".into());
    config.web_search.brave_api_key = Some("brave-credential".into());
    config.web_search.tavily_api_key = Some("tavily-credential".into());
    config.storage.provider.config.db_url = Some("postgres://user:pw@host/db".into());
    config.channels.feishu = Some(FeishuConfig {
        enabled: true,
        app_id: "cli_feishu_123".into(),
        app_secret: "feishu-secret".into(),
        encrypt_key: Some("feishu-encrypt".into()),
        verification_token: Some("feishu-verify".into()),
        allowed_users: vec!["*".into()],
        mention_only: false,
        receive_mode: LarkReceiveMode::Websocket,
        port: None,
        proxy_url: None,
    });

    config.agents.insert(
        "worker".into(),
        DelegateAgentConfig {
            provider: "openrouter".into(),
            model: "model-test".into(),
            system_prompt: None,
            api_key: Some("agent-credential".into()),
            temperature: None,
            max_depth: 3,
            agentic: false,
            allowed_tools: Vec::new(),
            max_iterations: 10,
            timeout_secs: None,
            agentic_timeout_secs: None,
            skills_directory: None,
            memory_namespace: None,
        },
    );

    config.save().await.unwrap();

    let contents = tokio::fs::read_to_string(config.config_path.clone())
        .await
        .unwrap();
    let stored: Config = toml::from_str::<crate::migration::V1Compat>(&contents)
        .unwrap()
        .into_config();
    let store = crate::secrets::SecretStore::new(&dir, true);

    let root_encrypted = stored
        .providers
        .models
        .get("default")
        .and_then(|m| m.api_key.as_deref())
        .unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(root_encrypted));
    assert_eq!(store.decrypt(root_encrypted).unwrap(), "root-credential");

    let composio_encrypted = stored.composio.api_key.as_deref().unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(
        composio_encrypted
    ));
    assert_eq!(
        store.decrypt(composio_encrypted).unwrap(),
        "composio-credential"
    );

    let browser_encrypted = stored.browser.computer_use.api_key.as_deref().unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(browser_encrypted));
    assert_eq!(
        store.decrypt(browser_encrypted).unwrap(),
        "browser-credential"
    );

    let web_search_encrypted = stored.web_search.brave_api_key.as_deref().unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(
        web_search_encrypted
    ));
    assert_eq!(
        store.decrypt(web_search_encrypted).unwrap(),
        "brave-credential"
    );

    let tavily_encrypted = stored.web_search.tavily_api_key.as_deref().unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(tavily_encrypted));
    assert_eq!(
        store.decrypt(tavily_encrypted).unwrap(),
        "tavily-credential"
    );

    let worker = stored.agents.get("worker").unwrap();
    let worker_encrypted = worker.api_key.as_deref().unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(worker_encrypted));
    assert_eq!(store.decrypt(worker_encrypted).unwrap(), "agent-credential");

    let storage_db_url = stored.storage.provider.config.db_url.as_deref().unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(storage_db_url));
    assert_eq!(
        store.decrypt(storage_db_url).unwrap(),
        "postgres://user:pw@host/db"
    );

    let feishu = stored.channels.feishu.as_ref().unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(
        &feishu.app_secret
    ));
    assert_eq!(store.decrypt(&feishu.app_secret).unwrap(), "feishu-secret");
    assert!(
        feishu
            .encrypt_key
            .as_deref()
            .is_some_and(crate::secrets::SecretStore::is_encrypted)
    );
    assert_eq!(
        store
            .decrypt(feishu.encrypt_key.as_deref().unwrap())
            .unwrap(),
        "feishu-encrypt"
    );
    assert!(
        feishu
            .verification_token
            .as_deref()
            .is_some_and(crate::secrets::SecretStore::is_encrypted)
    );
    assert_eq!(
        store
            .decrypt(feishu.verification_token.as_deref().unwrap())
            .unwrap(),
        "feishu-verify"
    );

    let _ = fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn config_save_atomic_cleanup() {
    let dir = std::env::temp_dir().join(format!("operant_test_config_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).await.unwrap();

    let config_path = dir.join("config.toml");
    let mut config = Config {
        workspace_dir: dir.join("workspace"),
        config_path: config_path.clone(),
        ..Default::default()
    };
    config.providers.fallback = Some("test".into());
    config.providers.models.insert(
        "test".into(),
        ModelProviderConfig {
            model: Some("model-a".into()),
            ..Default::default()
        },
    );
    config.save().await.unwrap();
    assert!(config_path.exists());

    config.providers.models.get_mut("test").unwrap().model = Some("model-b".into());
    config.save().await.unwrap();

    let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
    assert!(contents.contains("model-b"));

    let mut names: Vec<String> = Vec::new();
    let mut read_dir = fs::read_dir(&dir).await.unwrap();
    while let Some(entry) = read_dir.next_entry().await.unwrap() {
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    assert!(!names.iter().any(|name| name.contains(".tmp-")));
    assert!(!names.iter().any(|name| name.ends_with(".bak")));

    let _ = fs::remove_dir_all(&dir).await;
}

// ── Telegram / Discord config ────────────────────────────

#[test]
async fn telegram_config_serde() {
    let tc = TelegramConfig {
        enabled: true,
        bot_token: "123:XYZ".into(),
        allowed_users: vec!["alice".into(), "bob".into()],
        stream_mode: StreamMode::Partial,
        draft_update_interval_ms: 500,
        interrupt_on_new_message: true,
        mention_only: false,
        ack_reactions: None,
        proxy_url: None,
        approval_timeout_secs: 120,
        dm_topics_enabled: false,
        dm_topic_name: default_telegram_dm_topic_name(),
        disable_link_previews: false,
        typing_cooldown_seconds: default_telegram_typing_cooldown_secs(),
        fallback_ips: vec![],
    };
    let json = serde_json::to_string(&tc).unwrap();
    let parsed: TelegramConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.bot_token, "123:XYZ");
    assert_eq!(parsed.allowed_users.len(), 2);
    assert_eq!(parsed.stream_mode, StreamMode::Partial);
    assert_eq!(parsed.draft_update_interval_ms, 500);
    assert!(parsed.interrupt_on_new_message);
}

#[test]
async fn telegram_config_defaults_stream_off() {
    let json = r#"{"bot_token":"tok","allowed_users":[]}"#;
    let parsed: TelegramConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.stream_mode, StreamMode::Off);
    assert_eq!(parsed.draft_update_interval_ms, 1000);
    assert!(!parsed.interrupt_on_new_message);
}

#[test]
async fn discord_config_serde() {
    let dc = DiscordConfig {
        enabled: true,
        bot_token: "discord-token".into(),
        guild_id: Some("12345".into()),
        allowed_users: vec![],
        listen_to_bots: false,
        interrupt_on_new_message: false,
        mention_only: false,
        proxy_url: None,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1000,
        multi_message_delay_ms: 800,
        stall_timeout_secs: 0,
        approval_timeout_secs: 300,
    };
    let json = serde_json::to_string(&dc).unwrap();
    let parsed: DiscordConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.bot_token, "discord-token");
    assert_eq!(parsed.guild_id.as_deref(), Some("12345"));
}

#[test]
async fn discord_config_optional_guild() {
    let dc = DiscordConfig {
        enabled: true,
        bot_token: "tok".into(),
        guild_id: None,
        allowed_users: vec![],
        listen_to_bots: false,
        interrupt_on_new_message: false,
        mention_only: false,
        proxy_url: None,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1000,
        multi_message_delay_ms: 800,
        stall_timeout_secs: 0,
        approval_timeout_secs: 300,
    };
    let json = serde_json::to_string(&dc).unwrap();
    let parsed: DiscordConfig = serde_json::from_str(&json).unwrap();
    assert!(parsed.guild_id.is_none());
}

// ── iMessage / Matrix config ────────────────────────────

#[test]
async fn imessage_config_serde() {
    let ic = IMessageConfig {
        enabled: true,
        allowed_contacts: vec!["+1234567890".into(), "user@icloud.com".into()],
    };
    let json = serde_json::to_string(&ic).unwrap();
    let parsed: IMessageConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.allowed_contacts.len(), 2);
    assert_eq!(parsed.allowed_contacts[0], "+1234567890");
}

#[test]
async fn imessage_config_empty_contacts() {
    let ic = IMessageConfig {
        enabled: true,
        allowed_contacts: vec![],
    };
    let json = serde_json::to_string(&ic).unwrap();
    let parsed: IMessageConfig = serde_json::from_str(&json).unwrap();
    assert!(parsed.allowed_contacts.is_empty());
}

#[test]
async fn imessage_config_wildcard() {
    let ic = IMessageConfig {
        enabled: true,
        allowed_contacts: vec!["*".into()],
    };
    let toml_str = toml::to_string(&ic).unwrap();
    let parsed: IMessageConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.allowed_contacts, vec!["*"]);
}

#[test]
async fn matrix_config_serde() {
    let mc = MatrixConfig {
        enabled: true,
        homeserver: "https://matrix.org".into(),
        access_token: "syt_token_abc".into(),
        user_id: Some("@bot:matrix.org".into()),
        device_id: Some("DEVICE123".into()),
        allowed_users: vec!["@user:matrix.org".into()],
        allowed_rooms: vec!["!room123:matrix.org".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };
    let json = serde_json::to_string(&mc).unwrap();
    let parsed: MatrixConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.homeserver, "https://matrix.org");
    assert_eq!(parsed.access_token, "syt_token_abc");
    assert_eq!(parsed.user_id.as_deref(), Some("@bot:matrix.org"));
    assert_eq!(parsed.device_id.as_deref(), Some("DEVICE123"));
    assert_eq!(
        parsed.allowed_rooms.first().map(|s| s.as_str()),
        Some("!room123:matrix.org")
    );
    assert_eq!(parsed.allowed_users.len(), 1);
}

#[test]
async fn matrix_config_toml_roundtrip() {
    let mc = MatrixConfig {
        enabled: true,
        homeserver: "https://synapse.local:8448".into(),
        access_token: "tok".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec!["@admin:synapse.local".into(), "*".into()],
        allowed_rooms: vec!["!abc:synapse.local".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };
    let toml_str = toml::to_string(&mc).unwrap();
    let parsed: MatrixConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.homeserver, "https://synapse.local:8448");
    assert_eq!(parsed.allowed_users.len(), 2);
}

#[test]
async fn matrix_config_backward_compatible_without_session_hints() {
    // room_id in TOML is now migrated by prepare_table at the top level;
    // a bare MatrixConfig parse just ignores unknown keys.
    let toml = r#"
homeserver = "https://matrix.org"
access_token = "tok"
allowed_users = ["@ops:matrix.org"]
allowed_rooms = ["!ops:matrix.org"]
"#;

    let parsed: MatrixConfig = toml::from_str(toml).unwrap();
    assert_eq!(parsed.homeserver, "https://matrix.org");
    assert!(parsed.user_id.is_none());
    assert!(parsed.device_id.is_none());
    assert_eq!(parsed.allowed_rooms, vec!["!ops:matrix.org"]);
}

#[test]
async fn matrix_config_reply_in_thread_defaults_to_true() {
    let toml = r#"
homeserver = "https://matrix.org"
access_token = "tok"
allowed_users = ["@u:matrix.org"]
"#;
    let parsed: MatrixConfig = toml::from_str(toml).unwrap();
    assert!(parsed.reply_in_thread);
}

#[test]
async fn signal_config_serde() {
    let sc = SignalConfig {
        enabled: true,
        http_url: "http://127.0.0.1:8686".into(),
        account: "+1234567890".into(),
        group_id: Some("group123".into()),
        allowed_from: vec!["+1111111111".into()],
        ignore_attachments: true,
        ignore_stories: false,
        proxy_url: None,
        approval_timeout_secs: 300,
    };
    let json = serde_json::to_string(&sc).unwrap();
    let parsed: SignalConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.http_url, "http://127.0.0.1:8686");
    assert_eq!(parsed.account, "+1234567890");
    assert_eq!(parsed.group_id.as_deref(), Some("group123"));
    assert_eq!(parsed.allowed_from.len(), 1);
    assert!(parsed.ignore_attachments);
    assert!(!parsed.ignore_stories);
}

#[test]
async fn signal_config_toml_roundtrip() {
    let sc = SignalConfig {
        enabled: true,
        http_url: "http://localhost:8080".into(),
        account: "+9876543210".into(),
        group_id: None,
        allowed_from: vec!["*".into()],
        ignore_attachments: false,
        ignore_stories: true,
        proxy_url: None,
        approval_timeout_secs: 300,
    };
    let toml_str = toml::to_string(&sc).unwrap();
    let parsed: SignalConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.http_url, "http://localhost:8080");
    assert_eq!(parsed.account, "+9876543210");
    assert!(parsed.group_id.is_none());
    assert!(parsed.ignore_stories);
}

#[test]
async fn signal_config_defaults() {
    let json = r#"{"http_url":"http://127.0.0.1:8686","account":"+1234567890"}"#;
    let parsed: SignalConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.group_id.is_none());
    assert!(parsed.allowed_from.is_empty());
    assert!(!parsed.ignore_attachments);
    assert!(!parsed.ignore_stories);
}

#[test]
async fn channels_with_imessage_and_matrix() {
    let c = ChannelsConfig {
        cli: true,
        telegram: None,
        discord: None,
        discord_history: None,
        slack: None,
        mattermost: None,
        webhook: None,
        imessage: Some(IMessageConfig {
            enabled: true,
            allowed_contacts: vec!["+1".into()],
        }),
        matrix: Some(MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "tok".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec!["@u:m".into()],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        }),
        signal: None,
        whatsapp: None,
        linq: None,
        wati: None,
        nextcloud_talk: None,
        email: None,
        gmail_push: None,
        irc: None,
        lark: None,
        line: None,
        feishu: None,
        dingtalk: None,
        wecom: None,
        wechat: None,
        qq: None,
        twitter: None,
        mochat: None,
        #[cfg(feature = "channel-nostr")]
        nostr: None,
        clawdtalk: None,
        reddit: None,
        bluesky: None,
        voice_call: None,
        voice_duplex: None,
        #[cfg(feature = "voice-wake")]
        voice_wake: None,
        mqtt: None,
        message_timeout_secs: 300,
        ack_reactions: true,
        show_tool_calls: true,
        session_persistence: true,
        session_backend: default_session_backend(),
        session_ttl_hours: 0,
        debounce_ms: 0,
    };
    let toml_str = toml::to_string_pretty(&c).unwrap();
    let parsed: ChannelsConfig = toml::from_str(&toml_str).unwrap();
    assert!(parsed.imessage.is_some());
    assert!(parsed.matrix.is_some());
    assert_eq!(parsed.imessage.unwrap().allowed_contacts, vec!["+1"]);
    assert_eq!(parsed.matrix.unwrap().homeserver, "https://m.org");
}

#[test]
async fn channels_default_has_no_imessage_matrix() {
    let c = ChannelsConfig::default();
    assert!(c.imessage.is_none());
    assert!(c.matrix.is_none());
}

// ── Edge cases: serde(default) for allowed_users ─────────

#[test]
async fn discord_config_deserializes_without_allowed_users() {
    // Old configs won't have allowed_users — serde(default) should fill vec![]
    let json = r#"{"bot_token":"tok","guild_id":"123"}"#;
    let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.allowed_users.is_empty());
}

#[test]
async fn discord_config_deserializes_with_allowed_users() {
    let json = r#"{"bot_token":"tok","guild_id":"123","allowed_users":["111","222"]}"#;
    let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.allowed_users, vec!["111", "222"]);
}

#[test]
async fn slack_config_deserializes_without_allowed_users() {
    let json = r#"{"bot_token":"xoxb-tok"}"#;
    let parsed: SlackConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.channel_ids.is_empty());
    assert!(parsed.allowed_users.is_empty());
    assert!(!parsed.interrupt_on_new_message);
    assert_eq!(parsed.thread_replies, None);
    assert!(!parsed.mention_only);
}

#[test]
async fn slack_config_deserializes_with_allowed_users() {
    let json = r#"{"bot_token":"xoxb-tok","allowed_users":["U111"]}"#;
    let parsed: SlackConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.channel_ids.is_empty());
    assert_eq!(parsed.allowed_users, vec!["U111"]);
    assert!(!parsed.interrupt_on_new_message);
    assert_eq!(parsed.thread_replies, None);
    assert!(!parsed.mention_only);
}

#[test]
async fn slack_config_deserializes_with_channel_ids() {
    let json = r#"{"bot_token":"xoxb-tok","channel_ids":["C111","D222"]}"#;
    let parsed: SlackConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.channel_ids, vec!["C111", "D222"]);
    assert!(parsed.allowed_users.is_empty());
    assert!(!parsed.interrupt_on_new_message);
    assert_eq!(parsed.thread_replies, None);
    assert!(!parsed.mention_only);
}

#[test]
async fn slack_config_deserializes_with_mention_only() {
    let json = r#"{"bot_token":"xoxb-tok","mention_only":true}"#;
    let parsed: SlackConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.mention_only);
    assert!(!parsed.interrupt_on_new_message);
    assert_eq!(parsed.thread_replies, None);
}

#[test]
async fn slack_config_deserializes_interrupt_on_new_message() {
    let json = r#"{"bot_token":"xoxb-tok","interrupt_on_new_message":true}"#;
    let parsed: SlackConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.interrupt_on_new_message);
    assert_eq!(parsed.thread_replies, None);
    assert!(!parsed.mention_only);
}

#[test]
async fn slack_config_deserializes_thread_replies() {
    let json = r#"{"bot_token":"xoxb-tok","thread_replies":false}"#;
    let parsed: SlackConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.thread_replies, Some(false));
    assert!(!parsed.interrupt_on_new_message);
    assert!(!parsed.mention_only);
}

/// Regression test for #6237 — before the fix, omitting `bot_token`
/// from `[channels.slack]` made the entire config file fail to
/// deserialize with `missing field 'bot_token'`, blocking startup
/// even when `SLACK_BOT_TOKEN` was provided via the environment
/// (the env-fallback never ran because deserialization aborted first).
#[test]
async fn slack_config_deserializes_without_bot_token() {
    let json = r#"{}"#;
    let parsed: SlackConfig = serde_json::from_str(json).expect(
        "SlackConfig must deserialize without bot_token so the env-var \
             fallback in apply_env_overrides has a chance to populate it",
    );
    assert!(parsed.bot_token.is_none());
    assert!(parsed.app_token.is_none());
}

#[test]
async fn slack_config_deserializes_explicit_bot_token() {
    let json = r#"{"bot_token":"xoxb-from-toml"}"#;
    let parsed: SlackConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.bot_token.as_deref(), Some("xoxb-from-toml"));
}

/// `apply_env_overrides` populates `bot_token` from `SLACK_BOT_TOKEN`
/// when the config field is `None`. This is the path that #6237
/// reporters were trying to use — they had `SLACK_BOT_TOKEN` set in
/// their environment but the schema rejected the config before the
/// override could fire.
#[test]
async fn slack_apply_env_overrides_populates_bot_token_from_env() {
    let _env_guard = env_override_lock().await;
    // SAFETY: test-only, single-threaded test runner.
    unsafe {
        std::env::remove_var("OPERANT_SLACK_BOT_TOKEN");
        std::env::remove_var("OPERANT_SLACK_APP_TOKEN");
        std::env::remove_var("SLACK_APP_TOKEN");
        std::env::set_var("SLACK_BOT_TOKEN", "xoxb-from-env");
    }

    let mut config = Config::default();
    config.channels.slack = Some(SlackConfig {
        bot_token: None,
        ..Default::default()
    });
    config.apply_env_overrides();

    assert_eq!(
        config
            .channels
            .slack
            .as_ref()
            .and_then(|s| s.bot_token.as_deref()),
        Some("xoxb-from-env")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("SLACK_BOT_TOKEN") };
}

/// The `OPERANT_SLACK_BOT_TOKEN` variant takes precedence over
/// `SLACK_BOT_TOKEN` so workspace-scoped envs can override a
/// generic one set on the host.
#[test]
async fn slack_apply_env_overrides_prefers_operant_prefix() {
    let _env_guard = env_override_lock().await;
    // SAFETY: test-only, single-threaded test runner.
    unsafe {
        std::env::set_var("SLACK_BOT_TOKEN", "xoxb-generic");
        std::env::set_var("OPERANT_SLACK_BOT_TOKEN", "xoxb-operant-prefix");
    }

    let mut config = Config::default();
    config.channels.slack = Some(SlackConfig {
        bot_token: None,
        ..Default::default()
    });
    config.apply_env_overrides();

    assert_eq!(
        config
            .channels
            .slack
            .as_ref()
            .and_then(|s| s.bot_token.as_deref()),
        Some("xoxb-operant-prefix")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe {
        std::env::remove_var("SLACK_BOT_TOKEN");
        std::env::remove_var("OPERANT_SLACK_BOT_TOKEN");
    }
}

/// `apply_env_overrides` must NOT clobber a config-supplied bot_token,
/// otherwise users who set the value in `config.toml` would silently
/// have it replaced by an env var they didn't intend to be authoritative.
#[test]
async fn slack_apply_env_overrides_preserves_config_supplied_bot_token() {
    let _env_guard = env_override_lock().await;
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("SLACK_BOT_TOKEN", "xoxb-from-env") };

    let mut config = Config::default();
    config.channels.slack = Some(SlackConfig {
        bot_token: Some("xoxb-from-toml".to_string()),
        ..Default::default()
    });
    config.apply_env_overrides();

    assert_eq!(
        config
            .channels
            .slack
            .as_ref()
            .and_then(|s| s.bot_token.as_deref()),
        Some("xoxb-from-toml"),
        "config-supplied bot_token must win over env var"
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("SLACK_BOT_TOKEN") };
}

#[test]
async fn discord_config_default_interrupt_on_new_message_is_false() {
    let json = r#"{"bot_token":"tok"}"#;
    let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
    assert!(!parsed.interrupt_on_new_message);
}

#[test]
async fn discord_config_deserializes_interrupt_on_new_message_true() {
    let json = r#"{"bot_token":"tok","interrupt_on_new_message":true}"#;
    let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.interrupt_on_new_message);
}

#[test]
async fn discord_config_toml_backward_compat() {
    let toml_str = r#"
bot_token = "tok"
guild_id = "123"
"#;
    let parsed: DiscordConfig = toml::from_str(toml_str).unwrap();
    assert!(parsed.allowed_users.is_empty());
    assert_eq!(parsed.bot_token, "tok");
}

#[test]
async fn slack_config_toml_with_channel_ids() {
    let toml_str = r#"
bot_token = "xoxb-tok"
channel_ids = ["C123", "D456"]
"#;
    let parsed: SlackConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(parsed.channel_ids, vec!["C123", "D456"]);
    assert!(parsed.allowed_users.is_empty());
    assert!(!parsed.interrupt_on_new_message);
    assert_eq!(parsed.thread_replies, None);
    assert!(!parsed.mention_only);
}

#[test]
async fn slack_config_toml_without_channel_ids_defaults_empty() {
    let toml_str = r#"
bot_token = "xoxb-tok"
"#;
    let parsed: SlackConfig = toml::from_str(toml_str).unwrap();
    assert!(parsed.channel_ids.is_empty());
}

#[test]
async fn mattermost_config_default_interrupt_on_new_message_is_false() {
    let json = r#"{"url":"https://mm.example.com","bot_token":"tok"}"#;
    let parsed: MattermostConfig = serde_json::from_str(json).unwrap();
    assert!(!parsed.interrupt_on_new_message);
}

#[test]
async fn mattermost_config_deserializes_interrupt_on_new_message_true() {
    let json =
        r#"{"url":"https://mm.example.com","bot_token":"tok","interrupt_on_new_message":true}"#;
    let parsed: MattermostConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.interrupt_on_new_message);
}

#[test]
async fn webhook_config_with_secret() {
    let json = r#"{"port":8080,"secret":"my-secret-key"}"#;
    let parsed: WebhookConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.secret.as_deref(), Some("my-secret-key"));
}

#[test]
async fn webhook_config_without_secret() {
    let json = r#"{"port":8080}"#;
    let parsed: WebhookConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.secret.is_none());
    assert_eq!(parsed.port, 8080);
}

// ── WhatsApp config ──────────────────────────────────────

#[test]
async fn whatsapp_config_serde() {
    let wc = WhatsAppConfig {
        enabled: true,
        access_token: Some("EAABx...".into()),
        phone_number_id: Some("123456789".into()),
        verify_token: Some("my-verify-token".into()),
        app_secret: None,
        session_path: None,
        pair_phone: None,
        pair_code: None,
        allowed_numbers: vec!["+1234567890".into(), "+9876543210".into()],
        mention_only: false,
        mode: WhatsAppWebMode::default(),
        dm_policy: WhatsAppChatPolicy::default(),
        group_policy: WhatsAppChatPolicy::default(),
        self_chat_mode: false,
        dm_mention_patterns: vec![],
        group_mention_patterns: vec![],
        proxy_url: None,
        approval_timeout_secs: 300,
    };
    let json = serde_json::to_string(&wc).unwrap();
    let parsed: WhatsAppConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.access_token, Some("EAABx...".into()));
    assert_eq!(parsed.phone_number_id, Some("123456789".into()));
    assert_eq!(parsed.verify_token, Some("my-verify-token".into()));
    assert_eq!(parsed.allowed_numbers.len(), 2);
}

#[test]
async fn whatsapp_config_toml_roundtrip() {
    let wc = WhatsAppConfig {
        enabled: true,
        access_token: Some("tok".into()),
        phone_number_id: Some("12345".into()),
        verify_token: Some("verify".into()),
        app_secret: Some("secret123".into()),
        session_path: None,
        pair_phone: None,
        pair_code: None,
        allowed_numbers: vec!["+1".into()],
        mention_only: false,
        mode: WhatsAppWebMode::default(),
        dm_policy: WhatsAppChatPolicy::default(),
        group_policy: WhatsAppChatPolicy::default(),
        self_chat_mode: false,
        dm_mention_patterns: vec![],
        group_mention_patterns: vec![],
        proxy_url: None,
        approval_timeout_secs: 300,
    };
    let toml_str = toml::to_string(&wc).unwrap();
    let parsed: WhatsAppConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.phone_number_id, Some("12345".into()));
    assert_eq!(parsed.allowed_numbers, vec!["+1"]);
}

#[test]
async fn whatsapp_config_deserializes_without_allowed_numbers() {
    let json = r#"{"access_token":"tok","phone_number_id":"123","verify_token":"ver"}"#;
    let parsed: WhatsAppConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.allowed_numbers.is_empty());
}

#[test]
async fn whatsapp_config_wildcard_allowed() {
    let wc = WhatsAppConfig {
        enabled: true,
        access_token: Some("tok".into()),
        phone_number_id: Some("123".into()),
        verify_token: Some("ver".into()),
        app_secret: None,
        session_path: None,
        pair_phone: None,
        pair_code: None,
        allowed_numbers: vec!["*".into()],
        mention_only: false,
        mode: WhatsAppWebMode::default(),
        dm_policy: WhatsAppChatPolicy::default(),
        group_policy: WhatsAppChatPolicy::default(),
        self_chat_mode: false,
        dm_mention_patterns: vec![],
        group_mention_patterns: vec![],
        proxy_url: None,
        approval_timeout_secs: 300,
    };
    let toml_str = toml::to_string(&wc).unwrap();
    let parsed: WhatsAppConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.allowed_numbers, vec!["*"]);
}

#[test]
async fn whatsapp_config_backend_type_cloud_precedence_when_ambiguous() {
    let wc = WhatsAppConfig {
        enabled: true,
        access_token: Some("tok".into()),
        phone_number_id: Some("123".into()),
        verify_token: Some("ver".into()),
        app_secret: None,
        session_path: Some("~/.operant/state/whatsapp-web/session.db".into()),
        pair_phone: None,
        pair_code: None,
        allowed_numbers: vec!["+1".into()],
        mention_only: false,
        mode: WhatsAppWebMode::default(),
        dm_policy: WhatsAppChatPolicy::default(),
        group_policy: WhatsAppChatPolicy::default(),
        self_chat_mode: false,
        dm_mention_patterns: vec![],
        group_mention_patterns: vec![],
        proxy_url: None,
        approval_timeout_secs: 300,
    };
    assert!(wc.is_ambiguous_config());
    assert_eq!(wc.backend_type(), "cloud");
}

#[test]
async fn whatsapp_config_backend_type_web() {
    let wc = WhatsAppConfig {
        enabled: true,
        access_token: None,
        phone_number_id: None,
        verify_token: None,
        app_secret: None,
        session_path: Some("~/.operant/state/whatsapp-web/session.db".into()),
        pair_phone: None,
        pair_code: None,
        allowed_numbers: vec![],
        mention_only: false,
        mode: WhatsAppWebMode::default(),
        dm_policy: WhatsAppChatPolicy::default(),
        group_policy: WhatsAppChatPolicy::default(),
        self_chat_mode: false,
        dm_mention_patterns: vec![],
        group_mention_patterns: vec![],
        proxy_url: None,
        approval_timeout_secs: 300,
    };
    assert!(!wc.is_ambiguous_config());
    assert_eq!(wc.backend_type(), "web");
}

#[test]
async fn channels_with_whatsapp() {
    let c = ChannelsConfig {
        cli: true,
        telegram: None,
        discord: None,
        discord_history: None,
        slack: None,
        mattermost: None,
        webhook: None,
        imessage: None,
        matrix: None,
        signal: None,
        whatsapp: Some(WhatsAppConfig {
            enabled: true,
            access_token: Some("tok".into()),
            phone_number_id: Some("123".into()),
            verify_token: Some("ver".into()),
            app_secret: None,
            session_path: None,
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["+1".into()],
            mention_only: false,
            mode: WhatsAppWebMode::default(),
            dm_policy: WhatsAppChatPolicy::default(),
            group_policy: WhatsAppChatPolicy::default(),
            self_chat_mode: false,
            dm_mention_patterns: vec![],
            group_mention_patterns: vec![],
            proxy_url: None,
            approval_timeout_secs: 300,
        }),
        linq: None,
        wati: None,
        nextcloud_talk: None,
        email: None,
        gmail_push: None,
        irc: None,
        lark: None,
        line: None,
        feishu: None,
        dingtalk: None,
        wecom: None,
        wechat: None,
        qq: None,
        twitter: None,
        mochat: None,
        #[cfg(feature = "channel-nostr")]
        nostr: None,
        clawdtalk: None,
        reddit: None,
        bluesky: None,
        voice_call: None,
        voice_duplex: None,
        #[cfg(feature = "voice-wake")]
        voice_wake: None,
        mqtt: None,
        message_timeout_secs: 300,
        ack_reactions: true,
        show_tool_calls: true,
        session_persistence: true,
        session_backend: default_session_backend(),
        session_ttl_hours: 0,
        debounce_ms: 0,
    };
    let toml_str = toml::to_string_pretty(&c).unwrap();
    let parsed: ChannelsConfig = toml::from_str(&toml_str).unwrap();
    assert!(parsed.whatsapp.is_some());
    let wa = parsed.whatsapp.unwrap();
    assert_eq!(wa.phone_number_id, Some("123".into()));
    assert_eq!(wa.allowed_numbers, vec!["+1"]);
}

#[test]
async fn channels_default_has_no_whatsapp() {
    let c = ChannelsConfig::default();
    assert!(c.whatsapp.is_none());
}

#[test]
async fn channels_default_has_no_nextcloud_talk() {
    let c = ChannelsConfig::default();
    assert!(c.nextcloud_talk.is_none());
}

// ══════════════════════════════════════════════════════════
// SECURITY CHECKLIST TESTS — Gateway config
// ══════════════════════════════════════════════════════════

#[test]
async fn checklist_gateway_default_requires_pairing() {
    let g = GatewayConfig::default();
    assert!(g.require_pairing, "Pairing must be required by default");
}

#[test]
async fn checklist_gateway_default_blocks_public_bind() {
    let g = GatewayConfig::default();
    assert!(
        !g.allow_public_bind,
        "Public bind must be blocked by default"
    );
}

#[test]
async fn checklist_gateway_default_no_tokens() {
    let g = GatewayConfig::default();
    assert!(
        g.paired_tokens.is_empty(),
        "No pre-paired tokens by default"
    );
    assert_eq!(g.pair_rate_limit_per_minute, 10);
    assert_eq!(g.webhook_rate_limit_per_minute, 60);
    assert!(!g.trust_forwarded_headers);
    assert_eq!(g.rate_limit_max_keys, 10_000);
    assert_eq!(g.idempotency_ttl_secs, 300);
    assert_eq!(g.idempotency_max_keys, 10_000);
}

#[test]
async fn checklist_gateway_cli_default_host_is_localhost() {
    // The CLI default for --host is 127.0.0.1 (checked in main.rs)
    // Here we verify the config default matches
    let c = Config::default();
    assert!(
        c.gateway.require_pairing,
        "Config default must require pairing"
    );
    assert!(
        !c.gateway.allow_public_bind,
        "Config default must block public bind"
    );
}

#[test]
async fn checklist_gateway_serde_roundtrip() {
    let g = GatewayConfig {
        port: 42617,
        host: "127.0.0.1".into(),
        require_pairing: true,
        allow_public_bind: false,
        paired_tokens: vec!["zc_test_token".into()],
        pair_rate_limit_per_minute: 12,
        webhook_rate_limit_per_minute: 80,
        trust_forwarded_headers: true,
        path_prefix: Some("/operant".into()),
        rate_limit_max_keys: 2048,
        idempotency_ttl_secs: 600,
        idempotency_max_keys: 4096,
        session_persistence: true,
        session_ttl_hours: 0,
        pairing_dashboard: PairingDashboardConfig::default(),
        web_dist_dir: None,
        tls: None,
        platform_toolsets: HashMap::new(),
    };
    let toml_str = toml::to_string(&g).unwrap();
    let parsed: GatewayConfig = toml::from_str(&toml_str).unwrap();
    assert!(parsed.require_pairing);
    assert!(parsed.session_persistence);
    assert_eq!(parsed.session_ttl_hours, 0);
    assert!(!parsed.allow_public_bind);
    assert_eq!(parsed.paired_tokens, vec!["zc_test_token"]);
    assert_eq!(parsed.pair_rate_limit_per_minute, 12);
    assert_eq!(parsed.webhook_rate_limit_per_minute, 80);
    assert!(parsed.trust_forwarded_headers);
    assert_eq!(parsed.path_prefix.as_deref(), Some("/operant"));
    assert_eq!(parsed.rate_limit_max_keys, 2048);
    assert_eq!(parsed.idempotency_ttl_secs, 600);
    assert_eq!(parsed.idempotency_max_keys, 4096);
}

#[test]
async fn checklist_gateway_backward_compat_no_gateway_section() {
    // Old configs without [gateway] should get secure defaults
    let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
    let parsed = parse_test_config(minimal);
    assert!(
        parsed.gateway.require_pairing,
        "Missing [gateway] must default to require_pairing=true"
    );
    assert!(
        !parsed.gateway.allow_public_bind,
        "Missing [gateway] must default to allow_public_bind=false"
    );
}

#[test]
async fn checklist_autonomy_default_is_workspace_scoped() {
    let a = AutonomyConfig::default();
    assert!(a.workspace_only, "Default autonomy must be workspace_only");
    assert!(
        a.forbidden_paths.contains(&"/etc".to_string()),
        "Must block /etc"
    );
    assert!(
        a.forbidden_paths.contains(&"/proc".to_string()),
        "Must block /proc"
    );
    assert!(
        a.forbidden_paths.contains(&"~/.ssh".to_string()),
        "Must block ~/.ssh"
    );
}

// ══════════════════════════════════════════════════════════
// COMPOSIO CONFIG TESTS
// ══════════════════════════════════════════════════════════

#[test]
async fn composio_config_default_disabled() {
    let c = ComposioConfig::default();
    assert!(!c.enabled, "Composio must be disabled by default");
    assert!(c.api_key.is_none(), "No API key by default");
    assert_eq!(c.entity_id, "default");
}

#[test]
async fn composio_config_serde_roundtrip() {
    let c = ComposioConfig {
        enabled: true,
        api_key: Some("comp-key-123".into()),
        entity_id: "user42".into(),
    };
    let toml_str = toml::to_string(&c).unwrap();
    let parsed: ComposioConfig = toml::from_str(&toml_str).unwrap();
    assert!(parsed.enabled);
    assert_eq!(parsed.api_key.as_deref(), Some("comp-key-123"));
    assert_eq!(parsed.entity_id, "user42");
}

#[test]
async fn composio_config_backward_compat_missing_section() {
    let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
    let parsed = parse_test_config(minimal);
    assert!(
        !parsed.composio.enabled,
        "Missing [composio] must default to disabled"
    );
    assert!(parsed.composio.api_key.is_none());
}

#[test]
async fn composio_config_partial_toml() {
    let toml_str = r"
enabled = true
";
    let parsed: ComposioConfig = toml::from_str(toml_str).unwrap();
    assert!(parsed.enabled);
    assert!(parsed.api_key.is_none());
    assert_eq!(parsed.entity_id, "default");
}

#[test]
async fn composio_config_enable_alias_supported() {
    let toml_str = r"
enable = true
";
    let parsed: ComposioConfig = toml::from_str(toml_str).unwrap();
    assert!(parsed.enabled);
    assert!(parsed.api_key.is_none());
    assert_eq!(parsed.entity_id, "default");
}

// ══════════════════════════════════════════════════════════
// SECRETS CONFIG TESTS
// ══════════════════════════════════════════════════════════

#[test]
async fn secrets_config_default_encrypts() {
    let s = SecretsConfig::default();
    assert!(s.encrypt, "Encryption must be enabled by default");
}

#[test]
async fn secrets_config_serde_roundtrip() {
    let s = SecretsConfig { encrypt: false };
    let toml_str = toml::to_string(&s).unwrap();
    let parsed: SecretsConfig = toml::from_str(&toml_str).unwrap();
    assert!(!parsed.encrypt);
}

#[test]
async fn secrets_config_backward_compat_missing_section() {
    let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
    let parsed = parse_test_config(minimal);
    assert!(
        parsed.secrets.encrypt,
        "Missing [secrets] must default to encrypt=true"
    );
}

#[test]
async fn config_default_has_composio_and_secrets() {
    let c = Config::default();
    assert!(!c.composio.enabled);
    assert!(c.composio.api_key.is_none());
    assert!(c.secrets.encrypt);
    assert!(c.browser.enabled);
    assert_eq!(c.browser.allowed_domains, vec!["*".to_string()]);
}

#[test]
async fn browser_config_default_enabled() {
    let b = BrowserConfig::default();
    assert!(b.enabled);
    assert_eq!(b.allowed_domains, vec!["*".to_string()]);
    assert_eq!(b.backend, "agent_browser");
    assert!(b.native_headless);
    assert_eq!(b.native_webdriver_url, "http://127.0.0.1:9515");
    assert!(b.native_chrome_path.is_none());
    assert_eq!(b.computer_use.endpoint, "http://127.0.0.1:8787/v1/actions");
    assert_eq!(b.computer_use.timeout_ms, 15_000);
    assert!(!b.computer_use.allow_remote_endpoint);
    assert!(b.computer_use.window_allowlist.is_empty());
    assert!(b.computer_use.max_coordinate_x.is_none());
    assert!(b.computer_use.max_coordinate_y.is_none());
}

#[test]
async fn browser_config_serde_roundtrip() {
    let b = BrowserConfig {
        enabled: true,
        allowed_domains: vec!["example.com".into(), "docs.example.com".into()],
        session_name: None,
        backend: "auto".into(),
        native_headless: false,
        native_webdriver_url: "http://localhost:4444".into(),
        native_chrome_path: Some("/usr/bin/chromium".into()),
        computer_use: BrowserComputerUseConfig {
            endpoint: "https://computer-use.example.com/v1/actions".into(),
            api_key: Some("test-token".into()),
            timeout_ms: 8_000,
            allow_remote_endpoint: true,
            window_allowlist: vec!["Chrome".into(), "Visual Studio Code".into()],
            max_coordinate_x: Some(3840),
            max_coordinate_y: Some(2160),
        },
    };
    let toml_str = toml::to_string(&b).unwrap();
    let parsed: BrowserConfig = toml::from_str(&toml_str).unwrap();
    assert!(parsed.enabled);
    assert_eq!(parsed.allowed_domains.len(), 2);
    assert_eq!(parsed.allowed_domains[0], "example.com");
    assert_eq!(parsed.backend, "auto");
    assert!(!parsed.native_headless);
    assert_eq!(parsed.native_webdriver_url, "http://localhost:4444");
    assert_eq!(
        parsed.native_chrome_path.as_deref(),
        Some("/usr/bin/chromium")
    );
    assert_eq!(
        parsed.computer_use.endpoint,
        "https://computer-use.example.com/v1/actions"
    );
    assert_eq!(parsed.computer_use.api_key.as_deref(), Some("test-token"));
    assert_eq!(parsed.computer_use.timeout_ms, 8_000);
    assert!(parsed.computer_use.allow_remote_endpoint);
    assert_eq!(parsed.computer_use.window_allowlist.len(), 2);
    assert_eq!(parsed.computer_use.max_coordinate_x, Some(3840));
    assert_eq!(parsed.computer_use.max_coordinate_y, Some(2160));
}

#[test]
async fn browser_config_backward_compat_missing_section() {
    let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
    let parsed = parse_test_config(minimal);
    assert!(parsed.browser.enabled);
    assert_eq!(parsed.browser.allowed_domains, vec!["*".to_string()]);
}

// ── Environment variable overrides (Docker support) ─────────

async fn env_override_lock() -> MutexGuard<'static, ()> {
    static ENV_OVERRIDE_TEST_LOCK: Mutex<()> = Mutex::const_new(());
    ENV_OVERRIDE_TEST_LOCK.lock().await
}

fn clear_proxy_env_test_vars() {
    for key in [
        "OPERANT_PROXY_ENABLED",
        "OPERANT_HTTP_PROXY",
        "OPERANT_HTTPS_PROXY",
        "OPERANT_ALL_PROXY",
        "OPERANT_NO_PROXY",
        "OPERANT_PROXY_SCOPE",
        "OPERANT_PROXY_SERVICES",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var(key) };
    }
}

#[test]
async fn env_override_api_key() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    assert!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.as_ref())
            .is_none()
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_API_KEY", "sk-test-env-key") };
    config.apply_env_overrides();
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.as_deref()),
        Some("sk-test-env-key")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_API_KEY") };
}

#[test]
async fn env_override_api_key_fallback() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_API_KEY") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("API_KEY", "sk-fallback-key") };
    config.apply_env_overrides();
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.as_deref()),
        Some("sk-fallback-key")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("API_KEY") };
}

#[test]
async fn env_override_provider() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_PROVIDER", "anthropic") };
    config.apply_env_overrides();
    assert_eq!(config.providers.fallback.as_deref(), Some("anthropic"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_PROVIDER") };
}

#[test]
async fn env_override_model_provider_alias() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_PROVIDER") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_MODEL_PROVIDER", "openai-codex") };
    config.apply_env_overrides();
    assert_eq!(config.providers.fallback.as_deref(), Some("openai-codex"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_MODEL_PROVIDER") };
}

#[test]
async fn toml_supports_model_provider_and_model_alias_fields() {
    let raw = r#"
default_temperature = 0.7
model_provider = "sub2api"
model = "gpt-5.3-codex"

[model_providers.sub2api]
name = "sub2api"
base_url = "https://api.tonsof.blue/v1"
wire_api = "responses"
requires_openai_auth = true
"#;

    let parsed = parse_test_config(raw);
    assert_eq!(parsed.providers.fallback.as_deref(), Some("sub2api"));
    assert_eq!(
        parsed
            .providers
            .fallback_provider()
            .and_then(|e| e.model.as_deref()),
        Some("gpt-5.3-codex")
    );
    let profile = parsed
        .providers
        .models
        .get("sub2api")
        .expect("profile should exist");
    assert_eq!(profile.wire_api.as_deref(), Some("responses"));
    assert!(profile.requires_openai_auth);
}

#[test]
async fn env_override_open_skills_enabled_and_dir() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    assert!(!config.skills.open_skills_enabled);
    assert!(config.skills.open_skills_dir.is_none());
    assert_eq!(
        config.skills.prompt_injection_mode,
        SkillsPromptInjectionMode::Full
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_OPEN_SKILLS_ENABLED", "true") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_OPEN_SKILLS_DIR", "/tmp/open-skills") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_SKILLS_ALLOW_SCRIPTS", "yes") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_SKILLS_PROMPT_MODE", "compact") };
    config.apply_env_overrides();

    assert!(config.skills.open_skills_enabled);
    assert!(config.skills.allow_scripts);
    assert_eq!(
        config.skills.open_skills_dir.as_deref(),
        Some("/tmp/open-skills")
    );
    assert_eq!(
        config.skills.prompt_injection_mode,
        SkillsPromptInjectionMode::Compact
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_OPEN_SKILLS_ENABLED") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_OPEN_SKILLS_DIR") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_SKILLS_ALLOW_SCRIPTS") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_SKILLS_PROMPT_MODE") };
}

#[test]
async fn env_override_open_skills_enabled_invalid_value_keeps_existing_value() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.skills.open_skills_enabled = true;
    config.skills.allow_scripts = true;
    config.skills.prompt_injection_mode = SkillsPromptInjectionMode::Compact;

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_OPEN_SKILLS_ENABLED", "maybe") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_SKILLS_ALLOW_SCRIPTS", "maybe") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_SKILLS_PROMPT_MODE", "invalid") };
    config.apply_env_overrides();

    assert!(config.skills.open_skills_enabled);
    assert!(config.skills.allow_scripts);
    assert_eq!(
        config.skills.prompt_injection_mode,
        SkillsPromptInjectionMode::Compact
    );
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_OPEN_SKILLS_ENABLED") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_SKILLS_ALLOW_SCRIPTS") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_SKILLS_PROMPT_MODE") };
}

#[test]
async fn env_override_provider_fallback() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_PROVIDER") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("PROVIDER", "openai") };
    config.apply_env_overrides();
    assert_eq!(config.providers.fallback.as_deref(), Some("openai"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("PROVIDER") };
}

#[test]
async fn env_override_provider_fallback_does_not_replace_non_default_provider() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("custom:https://proxy.example.com/v1".to_string());

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_PROVIDER") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("PROVIDER", "openrouter") };
    config.apply_env_overrides();
    assert_eq!(
        config.providers.fallback.as_deref(),
        Some("custom:https://proxy.example.com/v1")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("PROVIDER") };
}

#[test]
async fn env_override_zero_claw_provider_overrides_non_default_provider() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("custom:https://proxy.example.com/v1".to_string());

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_PROVIDER", "openrouter") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("PROVIDER", "anthropic") };
    config.apply_env_overrides();
    assert_eq!(config.providers.fallback.as_deref(), Some("openrouter"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_PROVIDER") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("PROVIDER") };
}

#[test]
async fn env_override_glm_api_key_for_regional_aliases() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("glm-cn".to_string());

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("GLM_API_KEY", "glm-regional-key") };
    config.apply_env_overrides();
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.as_deref()),
        Some("glm-regional-key")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("GLM_API_KEY") };
}

#[test]
async fn env_override_zai_api_key_for_regional_aliases() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("zai-cn".to_string());

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("ZAI_API_KEY", "zai-regional-key") };
    config.apply_env_overrides();
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.as_deref()),
        Some("zai-regional-key")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("ZAI_API_KEY") };
}

#[test]
async fn env_override_model() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_MODEL", "gpt-4o") };
    config.apply_env_overrides();
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.model.as_deref()),
        Some("gpt-4o")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_MODEL") };
}

#[test]
async fn model_provider_profile_maps_to_custom_endpoint() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("sub2api".to_string());
    config.providers.models.insert(
        "sub2api".to_string(),
        ModelProviderConfig {
            name: Some("sub2api".to_string()),
            base_url: Some("https://api.tonsof.blue/v1".to_string()),
            wire_api: None,
            requires_openai_auth: false,
            azure_openai_resource: None,
            azure_openai_deployment: None,
            azure_openai_api_version: None,
            api_path: None,
            max_tokens: None,
            ..Default::default()
        },
    );

    config.apply_env_overrides();
    // The user's literal fallback key is preserved; we no longer rewrite it
    // to a canonical alias. This is the round-trip-safe contract that the
    // `operant config get/set` CLI relies on.
    assert_eq!(config.providers.fallback.as_deref(), Some("sub2api"));
    // The original entry is still stored under its config key.
    assert_eq!(
        config
            .providers
            .models
            .get("sub2api")
            .and_then(|e| e.base_url.as_deref()),
        Some("https://api.tonsof.blue/v1")
    );
    // The entry is also mirrored under the canonical alias key so runtime
    // lookups by `custom:<base_url>` still resolve even though the user's
    // fallback string is the original profile key.
    assert_eq!(
        config
            .providers
            .models
            .get("custom:https://api.tonsof.blue/v1")
            .and_then(|e| e.base_url.as_deref()),
        Some("https://api.tonsof.blue/v1")
    );
    assert!(config.providers.fallback_provider().is_some());
}

#[test]
async fn model_provider_profile_responses_uses_openai_codex_and_openai_key() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("sub2api".to_string());
    config.providers.models.insert(
        "sub2api".to_string(),
        ModelProviderConfig {
            name: Some("sub2api".to_string()),
            base_url: Some("https://api.tonsof.blue".to_string()),
            wire_api: Some("responses".to_string()),
            requires_openai_auth: true,
            azure_openai_resource: None,
            azure_openai_deployment: None,
            azure_openai_api_version: None,
            api_path: None,
            max_tokens: None,
            ..Default::default()
        },
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPENAI_API_KEY", "sk-test-codex-key") };
    config.apply_env_overrides();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPENAI_API_KEY") };

    // The user's literal fallback key is preserved; we no longer rewrite it
    // to "openai-codex". The Codex-app-server compatibility shim instead
    // mirrors the resolved entry under that alias key for runtime lookups.
    assert_eq!(config.providers.fallback.as_deref(), Some("sub2api"));
    // The original entry is still stored under its config key.
    let entry = config
        .providers
        .models
        .get("sub2api")
        .expect("sub2api entry");
    assert_eq!(entry.base_url.as_deref(), Some("https://api.tonsof.blue"));
    assert_eq!(entry.api_key.as_deref(), Some("sk-test-codex-key"));
    // The entry is mirrored under the "openai-codex" alias so any code
    // path that looks providers up by that canonical key still finds it.
    let aliased = config
        .providers
        .models
        .get("openai-codex")
        .expect("openai-codex alias entry");
    assert_eq!(aliased.base_url.as_deref(), Some("https://api.tonsof.blue"));
    assert_eq!(aliased.api_key.as_deref(), Some("sk-test-codex-key"));
}

/// Regression test for the config CLI get/set divergence bug.
///
/// Before the fix, `apply_named_model_provider_profile` rewrote
/// `self.providers.fallback` to the profile's `name` field whenever they
/// differed. That meant:
///
/// - `operant config get providers.fallback` returned the rewritten value
///   even though the on-disk TOML still held the user's literal key.
/// - `operant config set providers.fallback <new>` would persist `<new>`
///   to disk, but the next load mutated it back in memory, so a subsequent
///   `get` reported a stale value and the daemon's resolver looked up a
///   provider key that did not exist in `[providers.models.*]`.
///
/// The fix preserves the literal fallback key end-to-end. The named-profile
/// shim now only mirrors the resolved entry under canonical alias keys for
/// runtime lookup convenience.
#[test]
async fn apply_env_overrides_preserves_user_supplied_fallback_key() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    // User configures fallback = "primary" with a profile whose `name` field
    // differs from the key. This is the exact shape that triggered the bug.
    config.providers.fallback = Some("primary".to_string());
    config.providers.models.insert(
        "primary".to_string(),
        ModelProviderConfig {
            name: Some("alias-name".to_string()),
            base_url: Some("https://example.invalid/v1".to_string()),
            model: Some("primary-model".to_string()),
            ..Default::default()
        },
    );

    config.apply_env_overrides();

    // The literal user key must survive. This is what `config get` returns
    // and what `config set` persists.
    assert_eq!(
        config.providers.fallback.as_deref(),
        Some("primary"),
        "providers.fallback must preserve the user's literal key after \
             apply_env_overrides; got {:?}",
        config.providers.fallback,
    );
    // Runtime resolution must still find the entry under the original key.
    assert!(
        config.providers.fallback_provider().is_some(),
        "fallback_provider() must still resolve via the user's literal key",
    );
}

/// Round-trip test for the config CLI: a TOML file with the user's value
/// must deserialize, apply env overrides, and serialize back to the same
/// `providers.fallback`. This is the full path that backed the user-visible
/// `config set` -> `config get` divergence.
#[test]
async fn fallback_round_trips_through_load_apply_serialize() {
    let _env_guard = env_override_lock().await;
    let toml_in = r#"
schema_version = 1

[providers]
fallback = "primary"

[providers.models.primary]
name = "alias-name"
base_url = "https://example.invalid/v1"
model = "primary-model"
"#;

    let mut config: Config = toml::from_str(toml_in).expect("parse toml");
    config.apply_env_overrides();

    // What `config get providers.fallback` returns post-load.
    assert_eq!(
        config.get_prop("providers.fallback").unwrap(),
        "primary",
        "config get providers.fallback must return the user's literal value",
    );

    // What `config save` would write back to disk.
    let toml_out = toml::to_string(&config).expect("serialize toml");
    assert!(
        toml_out.contains(r#"fallback = "primary""#),
        "serialized config must keep fallback = \"primary\"; got:\n{toml_out}",
    );
}

/// `set_prop` followed by `get_prop` must return the value that was set,
/// even when the surrounding profile shape would historically have caused
/// the in-memory value to be rewritten.
#[test]
async fn set_prop_then_get_prop_round_trips_for_fallback() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.models.insert(
        "primary".to_string(),
        ModelProviderConfig {
            name: Some("alias-name".to_string()),
            model: Some("primary-model".to_string()),
            ..Default::default()
        },
    );
    // Simulate the daemon's load path before the user runs `config set`.
    config.apply_env_overrides();

    config.set_prop("providers.fallback", "primary").unwrap();
    // Mimic any post-set normalization a future codepath might add.
    config.apply_env_overrides();

    assert_eq!(config.get_prop("providers.fallback").unwrap(), "primary");
}

/// `resolve_default_model` returns the fallback provider's model when set,
/// and falls through to the first available `models.*` entry otherwise.
/// Returning `None` is reserved for "no provider has any model configured",
/// which callers must surface as a configuration error rather than silently
/// substituting a vendor default.
#[test]
async fn resolve_default_model_prefers_fallback_then_first_available() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    // Empty config: no model anywhere -> None (caller errors loudly).
    assert_eq!(config.providers.resolve_default_model(), None);

    // Add an entry without a model -> still None.
    config
        .providers
        .models
        .insert("secondary".to_string(), ModelProviderConfig::default());
    assert_eq!(config.providers.resolve_default_model(), None);

    // Add an entry with a model -> first-available wins when no fallback.
    config.providers.models.insert(
        "tertiary".to_string(),
        ModelProviderConfig {
            model: Some("tertiary-model".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(
        config.providers.resolve_default_model().as_deref(),
        Some("tertiary-model"),
    );

    // Set fallback to a provider with its own model -> fallback wins.
    config.providers.fallback = Some("primary".to_string());
    config.providers.models.insert(
        "primary".to_string(),
        ModelProviderConfig {
            model: Some("primary-model".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(
        config.providers.resolve_default_model().as_deref(),
        Some("primary-model"),
    );
}

#[test]
async fn save_repairs_bare_config_filename_using_runtime_resolution() {
    let _env_guard = env_override_lock().await;
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let workspace_dir = temp_home.join("workspace");
    let resolved_config_path = temp_home.join(".operant").join("config.toml");

    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOME", &temp_home) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

    let mut config = Config {
        workspace_dir,
        config_path: PathBuf::from("config.toml"),
        ..Default::default()
    };
    config.providers.fallback = Some("default".into());
    config.providers.models.insert(
        "default".into(),
        ModelProviderConfig {
            temperature: Some(0.5),
            ..Default::default()
        },
    );
    // Provider fields are now resolved directly — no cache needed.
    config.save().await.unwrap();

    assert!(resolved_config_path.exists());
    let saved = tokio::fs::read_to_string(&resolved_config_path)
        .await
        .unwrap();
    let parsed = parse_test_config(&saved);
    assert!(
        (parsed
            .providers
            .fallback_provider()
            .and_then(|e| e.temperature)
            .unwrap_or(0.7)
            - 0.5)
            .abs()
            < f64::EPSILON
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    if let Some(home) = original_home {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", home) };
    } else {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOME") };
    }
    let _ = tokio::fs::remove_dir_all(temp_home).await;
}

#[test]
async fn validate_ollama_cloud_model_requires_remote_api_url() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("ollama".to_string());
    config.providers.models.insert(
        "ollama".to_string(),
        ModelProviderConfig {
            model: Some("glm-5:cloud".to_string()),
            base_url: None,
            api_key: Some("ollama-key".to_string()),
            ..Default::default()
        },
    );

    let error = config.validate().expect_err("expected validation to fail");
    assert!(error.to_string().contains(
        "default_model uses ':cloud' with provider 'ollama', but api_url is local or unset"
    ));
}

#[test]
async fn validate_ollama_cloud_model_accepts_remote_endpoint_and_env_key() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("ollama".to_string());
    config.providers.models.insert(
        "ollama".to_string(),
        ModelProviderConfig {
            model: Some("glm-5:cloud".to_string()),
            base_url: Some("https://ollama.com/api".to_string()),
            api_key: None,
            ..Default::default()
        },
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OLLAMA_API_KEY", "ollama-env-key") };
    let result = config.validate();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OLLAMA_API_KEY") };

    assert!(result.is_ok(), "expected validation to pass: {result:?}");
}

#[test]
async fn validate_rejects_unknown_model_provider_wire_api() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.providers.fallback = Some("sub2api".to_string());
    config.providers.models.insert(
        "sub2api".to_string(),
        ModelProviderConfig {
            name: Some("sub2api".to_string()),
            base_url: Some("https://api.tonsof.blue/v1".to_string()),
            wire_api: Some("ws".to_string()),
            requires_openai_auth: false,
            azure_openai_resource: None,
            azure_openai_deployment: None,
            azure_openai_api_version: None,
            api_path: None,
            max_tokens: None,
            ..Default::default()
        },
    );

    let error = config.validate().expect_err("expected validation failure");
    assert!(
        error
            .to_string()
            .contains("wire_api must be one of: responses, chat_completions")
    );
}

#[test]
async fn env_override_model_fallback() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_MODEL") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("MODEL", "anthropic/claude-3.5-sonnet") };
    config.apply_env_overrides();
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.model.as_deref()),
        Some("anthropic/claude-3.5-sonnet")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("MODEL") };
}

#[test]
async fn env_override_workspace() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_WORKSPACE", "/custom/workspace") };
    config.apply_env_overrides();
    assert_eq!(config.workspace_dir, PathBuf::from("/custom/workspace"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
}

#[test]
async fn resolve_runtime_config_dirs_uses_env_workspace_first() {
    let _env_guard = env_override_lock().await;
    let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let default_workspace_dir = default_config_dir.join("workspace");
    let workspace_dir = default_config_dir.join("profile-a");

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };
    let (config_dir, resolved_workspace_dir, source) =
        resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
            .await
            .unwrap();

    assert_eq!(source, ConfigResolutionSource::EnvWorkspace);
    assert_eq!(config_dir, workspace_dir);
    assert_eq!(resolved_workspace_dir, workspace_dir.join("workspace"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    let _ = fs::remove_dir_all(default_config_dir).await;
}

#[test]
async fn resolve_runtime_config_dirs_uses_env_config_dir_first() {
    let _env_guard = env_override_lock().await;
    let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let default_workspace_dir = default_config_dir.join("workspace");
    let explicit_config_dir = default_config_dir.join("explicit-config");
    let marker_config_dir = default_config_dir.join("profiles").join("alpha");
    let state_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

    fs::create_dir_all(&default_config_dir).await.unwrap();
    let state = ActiveWorkspaceState {
        config_dir: marker_config_dir.to_string_lossy().into_owned(),
    };
    fs::write(&state_path, toml::to_string(&state).unwrap())
        .await
        .unwrap();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_CONFIG_DIR", &explicit_config_dir) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };

    let (config_dir, resolved_workspace_dir, source) =
        resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
            .await
            .unwrap();

    assert_eq!(source, ConfigResolutionSource::EnvConfigDir);
    assert_eq!(config_dir, explicit_config_dir);
    assert_eq!(
        resolved_workspace_dir,
        explicit_config_dir.join("workspace")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") };
    let _ = fs::remove_dir_all(default_config_dir).await;
}

#[test]
async fn resolve_runtime_config_dirs_uses_active_workspace_marker() {
    let _env_guard = env_override_lock().await;
    let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let default_workspace_dir = default_config_dir.join("workspace");
    let marker_config_dir = default_config_dir.join("profiles").join("alpha");
    let state_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    fs::create_dir_all(&default_config_dir).await.unwrap();
    let state = ActiveWorkspaceState {
        config_dir: marker_config_dir.to_string_lossy().into_owned(),
    };
    fs::write(&state_path, toml::to_string(&state).unwrap())
        .await
        .unwrap();

    let (config_dir, resolved_workspace_dir, source) =
        resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
            .await
            .unwrap();

    assert_eq!(source, ConfigResolutionSource::ActiveWorkspaceMarker);
    assert_eq!(config_dir, marker_config_dir);
    assert_eq!(resolved_workspace_dir, marker_config_dir.join("workspace"));

    let _ = fs::remove_dir_all(default_config_dir).await;
}

#[test]
async fn resolve_runtime_config_dirs_falls_back_to_default_layout() {
    let _env_guard = env_override_lock().await;
    let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let default_workspace_dir = default_config_dir.join("workspace");

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    let (config_dir, resolved_workspace_dir, source) =
        resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
            .await
            .unwrap();

    assert_eq!(source, ConfigResolutionSource::DefaultConfigDir);
    assert_eq!(config_dir, default_config_dir);
    assert_eq!(resolved_workspace_dir, default_workspace_dir);

    let _ = fs::remove_dir_all(default_config_dir).await;
}

#[test]
async fn default_path_under_config_dir_respects_operant_config_dir() {
    let _env_guard = env_override_lock().await;
    let custom_dir = std::env::temp_dir().join("operant-test-profile");
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_CONFIG_DIR", &custom_dir) };

    let result = default_path_under_config_dir("knowledge.db");

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") };

    assert_eq!(
        result,
        custom_dir.join("knowledge.db").to_string_lossy().as_ref(),
        "expected path under OPERANT_CONFIG_DIR, got: {result}"
    );
}

#[test]
async fn load_or_init_workspace_override_uses_workspace_root_for_config() {
    let _env_guard = env_override_lock().await;
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let workspace_dir = temp_home.join("profile-a");

    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOME", &temp_home) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

    let config = Box::pin(Config::load_or_init()).await.unwrap();

    assert_eq!(config.workspace_dir, workspace_dir.join("workspace"));
    assert_eq!(config.config_path, workspace_dir.join("config.toml"));
    assert!(workspace_dir.join("config.toml").exists());

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    if let Some(home) = original_home {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", home) };
    } else {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOME") };
    }
    let _ = fs::remove_dir_all(temp_home).await;
}

#[test]
async fn load_or_init_workspace_suffix_uses_legacy_config_layout() {
    let _env_guard = env_override_lock().await;
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let workspace_dir = temp_home.join("workspace");
    let legacy_config_path = temp_home.join(".operant").join("config.toml");

    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOME", &temp_home) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

    let config = Box::pin(Config::load_or_init()).await.unwrap();

    assert_eq!(config.workspace_dir, workspace_dir);
    assert_eq!(config.config_path, legacy_config_path);
    assert!(config.config_path.exists());

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    if let Some(home) = original_home {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", home) };
    } else {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOME") };
    }
    let _ = fs::remove_dir_all(temp_home).await;
}

#[test]
async fn load_or_init_workspace_override_keeps_existing_legacy_config() {
    let _env_guard = env_override_lock().await;
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let workspace_dir = temp_home.join("custom-workspace");
    let legacy_config_dir = temp_home.join(".operant");
    let legacy_config_path = legacy_config_dir.join("config.toml");

    fs::create_dir_all(&legacy_config_dir).await.unwrap();
    fs::write(
        &legacy_config_path,
        r#"default_temperature = 0.7
default_model = "legacy-model"
"#,
    )
    .await
    .unwrap();

    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOME", &temp_home) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

    let config = Box::pin(Config::load_or_init()).await.unwrap();

    assert_eq!(config.workspace_dir, workspace_dir);
    assert_eq!(config.config_path, legacy_config_path);
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.model.as_deref()),
        Some("legacy-model")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    if let Some(home) = original_home {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", home) };
    } else {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOME") };
    }
    let _ = fs::remove_dir_all(temp_home).await;
}

#[test]
async fn load_or_init_decrypts_feishu_channel_secrets() {
    let _env_guard = env_override_lock().await;
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let config_dir = temp_home.join(".operant");
    let config_path = config_dir.join("config.toml");

    fs::create_dir_all(&config_dir).await.unwrap();

    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOME", &temp_home) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };

    let mut config = Config {
        config_path: config_path.clone(),
        workspace_dir: config_dir.join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = true;
    config.channels.feishu = Some(FeishuConfig {
        enabled: true,
        app_id: "cli_feishu_123".into(),
        app_secret: "feishu-secret".into(),
        encrypt_key: Some("feishu-encrypt".into()),
        verification_token: Some("feishu-verify".into()),
        allowed_users: vec!["*".into()],
        mention_only: false,
        receive_mode: LarkReceiveMode::Websocket,
        port: None,
        proxy_url: None,
    });
    config.save().await.unwrap();

    let loaded = Box::pin(Config::load_or_init()).await.unwrap();
    let feishu = loaded.channels.feishu.as_ref().unwrap();
    assert_eq!(feishu.app_secret, "feishu-secret");
    assert_eq!(feishu.encrypt_key.as_deref(), Some("feishu-encrypt"));
    assert_eq!(feishu.verification_token.as_deref(), Some("feishu-verify"));

    if let Some(home) = original_home {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", home) };
    } else {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOME") };
    }
    let _ = fs::remove_dir_all(temp_home).await;
}

#[test]
async fn load_or_init_uses_persisted_active_workspace_marker() {
    let _env_guard = env_override_lock().await;
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let temp_default_dir = temp_home.join(".operant");
    let custom_config_dir = temp_home.join("profiles").join("agent-alpha");

    fs::create_dir_all(&custom_config_dir).await.unwrap();
    // Pre-create the default dir so is_temp_directory() can canonicalize
    // the path on macOS (where /var → /private/var symlink requires
    // the directory to exist for canonicalize to resolve correctly).
    fs::create_dir_all(&temp_default_dir).await.unwrap();
    fs::write(
        custom_config_dir.join("config.toml"),
        "default_temperature = 0.7\ndefault_model = \"persisted-profile\"\n",
    )
    .await
    .unwrap();

    // Write the marker using the explicit default dir (no HOME manipulation
    // needed for the persist call itself).
    persist_active_workspace_config_dir_in(&custom_config_dir, &temp_default_dir)
        .await
        .unwrap();

    // Config::load_or_init still reads HOME to find the marker, so we
    // must override HOME here. The persist above already wrote to the
    // correct temp location, so no stale marker can leak.
    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOME", &temp_home) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };

    let config = Box::pin(Config::load_or_init()).await.unwrap();

    assert_eq!(config.config_path, custom_config_dir.join("config.toml"));
    assert_eq!(config.workspace_dir, custom_config_dir.join("workspace"));
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.model.as_deref()),
        Some("persisted-profile")
    );

    if let Some(home) = original_home {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", home) };
    } else {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOME") };
    }
    let _ = fs::remove_dir_all(temp_home).await;
}

#[test]
async fn load_or_init_env_workspace_override_takes_priority_over_marker() {
    let _env_guard = env_override_lock().await;
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let temp_default_dir = temp_home.join(".operant");
    let marker_config_dir = temp_home.join("profiles").join("persisted-profile");
    let env_workspace_dir = temp_home.join("env-workspace");

    fs::create_dir_all(&marker_config_dir).await.unwrap();
    fs::write(
        marker_config_dir.join("config.toml"),
        "default_temperature = 0.7\ndefault_model = \"marker-model\"\n",
    )
    .await
    .unwrap();

    // Write marker via explicit default dir, then set HOME for load_or_init.
    persist_active_workspace_config_dir_in(&marker_config_dir, &temp_default_dir)
        .await
        .unwrap();

    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOME", &temp_home) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_WORKSPACE", &env_workspace_dir) };

    let config = Box::pin(Config::load_or_init()).await.unwrap();

    assert_eq!(config.workspace_dir, env_workspace_dir.join("workspace"));
    assert_eq!(config.config_path, env_workspace_dir.join("config.toml"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    if let Some(home) = original_home {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", home) };
    } else {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOME") };
    }
    let _ = fs::remove_dir_all(temp_home).await;
}

#[test]
async fn persist_active_workspace_marker_is_cleared_for_default_config_dir() {
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let default_config_dir = temp_home.join(".operant");
    let custom_config_dir = temp_home.join("profiles").join("custom-profile");
    let marker_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

    // Use the _in variant directly -- no HOME manipulation needed since
    // this test only exercises persist/clear logic, not Config::load_or_init.
    persist_active_workspace_config_dir_in(&custom_config_dir, &default_config_dir)
        .await
        .unwrap();
    assert!(marker_path.exists());

    persist_active_workspace_config_dir_in(&default_config_dir, &default_config_dir)
        .await
        .unwrap();
    assert!(!marker_path.exists());

    let _ = fs::remove_dir_all(temp_home).await;
}

#[test]
#[allow(clippy::large_futures)]
async fn load_or_init_logs_existing_config_as_initialized() {
    let _env_guard = env_override_lock().await;
    let temp_home =
        std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
    let workspace_dir = temp_home.join("profile-a");
    let config_path = workspace_dir.join("config.toml");

    fs::create_dir_all(&workspace_dir).await.unwrap();
    fs::write(
        &config_path,
        r#"default_temperature = 0.7
default_model = "persisted-profile"
"#,
    )
    .await
    .unwrap();

    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOME", &temp_home) };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

    let capture = SharedLogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(capture.clone())
        .finish();
    let dispatch = tracing::Dispatch::new(subscriber);
    let guard = tracing::dispatcher::set_default(&dispatch);

    let config = Box::pin(Config::load_or_init()).await.unwrap();

    drop(guard);
    let logs = capture.captured();

    assert_eq!(config.workspace_dir, workspace_dir.join("workspace"));
    assert_eq!(config.config_path, config_path);
    assert_eq!(
        config
            .providers
            .fallback_provider()
            .and_then(|e| e.model.as_deref()),
        Some("persisted-profile")
    );
    assert!(logs.contains("Config loaded"), "{logs}");
    assert!(logs.contains("initialized=true"), "{logs}");
    assert!(!logs.contains("initialized=false"), "{logs}");

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    if let Some(home) = original_home {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", home) };
    } else {
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOME") };
    }
    let _ = fs::remove_dir_all(temp_home).await;
}

#[test]
async fn env_override_empty_values_ignored() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    let original_provider = config.providers.fallback.clone();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_PROVIDER", "") };
    config.apply_env_overrides();
    assert_eq!(config.providers.fallback, original_provider);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_PROVIDER") };
}

#[test]
async fn env_override_gateway_port() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    assert_eq!(config.gateway.port, 42617);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_GATEWAY_PORT", "8080") };
    config.apply_env_overrides();
    assert_eq!(config.gateway.port, 8080);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_GATEWAY_PORT") };
}

#[test]
async fn env_override_port_fallback() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_GATEWAY_PORT") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("PORT", "9000") };
    config.apply_env_overrides();
    assert_eq!(config.gateway.port, 9000);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("PORT") };
}

#[test]
async fn env_override_gateway_host() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    assert_eq!(config.gateway.host, "127.0.0.1");

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_GATEWAY_HOST", "0.0.0.0") };
    config.apply_env_overrides();
    assert_eq!(config.gateway.host, "0.0.0.0");

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_GATEWAY_HOST") };
}

#[test]
async fn env_override_host_fallback() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_GATEWAY_HOST") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("HOST", "0.0.0.0") };
    config.apply_env_overrides();
    assert_eq!(config.gateway.host, "0.0.0.0");

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("HOST") };
}

#[test]
async fn env_override_require_pairing() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    assert!(config.gateway.require_pairing);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_REQUIRE_PAIRING", "false") };
    config.apply_env_overrides();
    assert!(!config.gateway.require_pairing);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_REQUIRE_PAIRING", "true") };
    config.apply_env_overrides();
    assert!(config.gateway.require_pairing);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_REQUIRE_PAIRING") };
}

#[test]
async fn env_override_temperature() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_TEMPERATURE", "0.5") };
    config.apply_env_overrides();
    assert!(
        (config
            .providers
            .fallback_provider()
            .and_then(|e| e.temperature)
            .unwrap_or(0.7)
            - 0.5)
            .abs()
            < f64::EPSILON
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_TEMPERATURE") };
}

#[test]
async fn env_override_temperature_out_of_range_ignored() {
    let _env_guard = env_override_lock().await;
    // Clean up any leftover env vars from other tests
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_TEMPERATURE") };

    let mut config = Config::default();
    let original_temp = config
        .providers
        .fallback_provider()
        .and_then(|e| e.temperature)
        .unwrap_or(0.7);

    // Temperature > 2.0 should be ignored
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_TEMPERATURE", "3.0") };
    config.apply_env_overrides();
    assert!(
        (config
            .providers
            .fallback_provider()
            .and_then(|e| e.temperature)
            .unwrap_or(0.7)
            - original_temp)
            .abs()
            < f64::EPSILON,
        "Temperature 3.0 should be ignored (out of range)"
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_TEMPERATURE") };
}

#[test]
async fn validate_rejects_out_of_range_temperature() {
    let mut config = Config::default();
    config.providers.fallback = Some("test".into());
    config.providers.models.insert(
        "test".into(),
        ModelProviderConfig {
            name: Some("test-provider".into()),
            temperature: Some(99.0),
            ..Default::default()
        },
    );
    let err = config.validate().unwrap_err();
    assert!(
        err.to_string().contains("temperature"),
        "expected temperature validation error, got: {err}"
    );
}

#[test]
async fn validate_rejects_negative_temperature() {
    let mut config = Config::default();
    config.providers.fallback = Some("test".into());
    config.providers.models.insert(
        "test".into(),
        ModelProviderConfig {
            name: Some("test-provider".into()),
            temperature: Some(-0.5),
            ..Default::default()
        },
    );
    let err = config.validate().unwrap_err();
    assert!(
        err.to_string().contains("temperature"),
        "expected temperature validation error, got: {err}"
    );
}

#[test]
async fn validate_accepts_valid_temperature() {
    let mut config = Config::default();
    config.providers.fallback = Some("test".into());
    config.providers.models.insert(
        "test".into(),
        ModelProviderConfig {
            name: Some("test-provider".into()),
            temperature: Some(0.7),
            ..Default::default()
        },
    );
    assert!(config.validate().is_ok());
}

#[test]
async fn validate_rejects_precheck_timeout_zero() {
    let mut config = Config::default();
    config.agent.precheck.timeout_secs = 0;
    let err = config.validate().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("agent.precheck.timeout_secs") && msg.contains("greater than 0"),
        "expected precheck timeout validation error, got: {msg}"
    );
}

#[test]
async fn validate_rejects_precheck_empty_model() {
    let mut config = Config::default();
    config.agent.precheck.model = Some("   ".into());
    let err = config.validate().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("agent.precheck.model"),
        "expected precheck model validation error, got: {msg}"
    );
}

#[test]
async fn validate_accepts_default_precheck() {
    let config = Config::default();
    assert!(
        config.validate().is_ok(),
        "default ChannelPrecheckConfig must pass validation"
    );
}

#[test]
async fn validate_accepts_precheck_model_override() {
    let mut config = Config::default();
    config.agent.precheck.model = Some("fast-classifier".into());
    config.agent.precheck.timeout_secs = 3;
    assert!(config.validate().is_ok());
}

#[test]
async fn validate_rejects_unpublished_jira_actions() {
    for action in ["list_projects", "myself"] {
        let mut config = Config::default();
        config.jira.enabled = true;
        config.jira.base_url = "https://jira.example.test".into();
        config.jira.api_token = "token".into();
        config.jira.allowed_actions = vec![action.into()];

        let err = config
            .validate()
            .expect_err("unpublished Jira action should be rejected")
            .to_string();
        assert!(
            err.contains("jira.allowed_actions contains unknown action"),
            "expected Jira allowed action error for {action}, got: {err}"
        );
    }
}

#[test]
async fn env_override_reasoning_enabled() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    assert_eq!(config.runtime.reasoning_enabled, None);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_REASONING_ENABLED", "false") };
    config.apply_env_overrides();
    assert_eq!(config.runtime.reasoning_enabled, Some(false));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_REASONING_ENABLED", "true") };
    config.apply_env_overrides();
    assert_eq!(config.runtime.reasoning_enabled, Some(true));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_REASONING_ENABLED") };
}

#[test]
async fn env_override_reasoning_invalid_value_ignored() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    config.runtime.reasoning_enabled = Some(false);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_REASONING_ENABLED", "maybe") };
    config.apply_env_overrides();
    assert_eq!(config.runtime.reasoning_enabled, Some(false));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_REASONING_ENABLED") };
}

#[test]
async fn env_override_reasoning_effort() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    assert_eq!(config.runtime.reasoning_effort, None);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_REASONING_EFFORT", "HIGH") };
    config.apply_env_overrides();
    assert_eq!(config.runtime.reasoning_effort.as_deref(), Some("high"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_REASONING_EFFORT") };
}

#[test]
async fn env_override_reasoning_effort_legacy_codex_env() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_CODEX_REASONING_EFFORT", "minimal") };
    config.apply_env_overrides();
    assert_eq!(config.runtime.reasoning_effort.as_deref(), Some("minimal"));

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_CODEX_REASONING_EFFORT") };
}

#[test]
async fn env_override_invalid_port_ignored() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    let original_port = config.gateway.port;

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("PORT", "not_a_number") };
    config.apply_env_overrides();
    assert_eq!(config.gateway.port, original_port);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("PORT") };
}

#[test]
async fn env_override_web_search_config() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("WEB_SEARCH_ENABLED", "false") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("WEB_SEARCH_PROVIDER", "brave") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("WEB_SEARCH_MAX_RESULTS", "7") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("WEB_SEARCH_TIMEOUT_SECS", "20") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("BRAVE_API_KEY", "brave-test-key") };

    config.apply_env_overrides();

    assert!(!config.web_search.enabled);
    assert_eq!(config.web_search.provider, "brave");
    assert_eq!(config.web_search.max_results, 7);
    assert_eq!(config.web_search.timeout_secs, 20);
    assert_eq!(
        config.web_search.brave_api_key.as_deref(),
        Some("brave-test-key")
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("WEB_SEARCH_ENABLED") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("WEB_SEARCH_PROVIDER") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("WEB_SEARCH_MAX_RESULTS") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("WEB_SEARCH_TIMEOUT_SECS") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("BRAVE_API_KEY") };
}

#[test]
async fn env_override_web_search_invalid_values_ignored() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();
    let original_max_results = config.web_search.max_results;
    let original_timeout = config.web_search.timeout_secs;

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("WEB_SEARCH_MAX_RESULTS", "99") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("WEB_SEARCH_TIMEOUT_SECS", "0") };

    config.apply_env_overrides();

    assert_eq!(config.web_search.max_results, original_max_results);
    assert_eq!(config.web_search.timeout_secs, original_timeout);

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("WEB_SEARCH_MAX_RESULTS") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("WEB_SEARCH_TIMEOUT_SECS") };
}

#[test]
async fn env_override_storage_provider_config() {
    let _env_guard = env_override_lock().await;
    let mut config = Config::default();

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_STORAGE_PROVIDER", "qdrant") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_STORAGE_DB_URL", "http://localhost:6333") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_STORAGE_CONNECT_TIMEOUT_SECS", "15") };

    config.apply_env_overrides();

    assert_eq!(config.storage.provider.config.provider, "qdrant");
    assert_eq!(
        config.storage.provider.config.db_url.as_deref(),
        Some("http://localhost:6333")
    );
    assert_eq!(
        config.storage.provider.config.connect_timeout_secs,
        Some(15)
    );

    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_STORAGE_PROVIDER") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_STORAGE_DB_URL") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::remove_var("OPERANT_STORAGE_CONNECT_TIMEOUT_SECS") };
}

#[test]
async fn proxy_config_scope_services_requires_entries_when_enabled() {
    let proxy = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://127.0.0.1:7890".into()),
        https_proxy: None,
        all_proxy: None,
        no_proxy: Vec::new(),
        scope: ProxyScope::Services,
        services: Vec::new(),
    };

    let error = proxy.validate().unwrap_err().to_string();
    assert!(error.contains("proxy.scope='services'"));
}

#[test]
async fn env_override_proxy_scope_services() {
    let _env_guard = env_override_lock().await;
    clear_proxy_env_test_vars();

    let mut config = Config::default();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_PROXY_ENABLED", "true") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_HTTP_PROXY", "http://127.0.0.1:7890") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe {
        std::env::set_var(
            "OPERANT_PROXY_SERVICES",
            "provider.openai, tool.http_request",
        );
    }
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_PROXY_SCOPE", "services") };

    config.apply_env_overrides();

    assert!(config.proxy.enabled);
    assert_eq!(config.proxy.scope, ProxyScope::Services);
    assert_eq!(
        config.proxy.http_proxy.as_deref(),
        Some("http://127.0.0.1:7890")
    );
    assert!(config.proxy.should_apply_to_service("provider.openai"));
    assert!(config.proxy.should_apply_to_service("tool.http_request"));
    assert!(!config.proxy.should_apply_to_service("provider.anthropic"));

    clear_proxy_env_test_vars();
}

#[test]
async fn env_override_proxy_scope_environment_applies_process_env() {
    let _env_guard = env_override_lock().await;
    clear_proxy_env_test_vars();

    let mut config = Config::default();
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_PROXY_ENABLED", "true") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_PROXY_SCOPE", "environment") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_HTTP_PROXY", "http://127.0.0.1:7890") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_HTTPS_PROXY", "http://127.0.0.1:7891") };
    // SAFETY: test-only, single-threaded test runner.
    unsafe { std::env::set_var("OPERANT_NO_PROXY", "localhost,127.0.0.1") };

    config.apply_env_overrides();

    assert_eq!(config.proxy.scope, ProxyScope::Environment);
    assert_eq!(
        std::env::var("HTTP_PROXY").ok().as_deref(),
        Some("http://127.0.0.1:7890")
    );
    assert_eq!(
        std::env::var("HTTPS_PROXY").ok().as_deref(),
        Some("http://127.0.0.1:7891")
    );
    assert!(
        std::env::var("NO_PROXY")
            .ok()
            .is_some_and(|value| value.contains("localhost"))
    );

    clear_proxy_env_test_vars();
}

#[test]
async fn google_workspace_allowed_operations_require_methods() {
    let mut config = Config::default();
    config.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
        service: "gmail".into(),
        resource: "users".into(),
        sub_resource: Some("drafts".into()),
        methods: Vec::new(),
    }];

    let err = config.validate().unwrap_err().to_string();
    assert!(err.contains("google_workspace.allowed_operations[0].methods"));
}

#[test]
async fn google_workspace_allowed_operations_reject_duplicate_service_resource_sub_resource_entries()
 {
    let mut config = Config::default();
    config.google_workspace.allowed_operations = vec![
        GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("drafts".into()),
            methods: vec!["create".into()],
        },
        GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("drafts".into()),
            methods: vec!["update".into()],
        },
    ];

    let err = config.validate().unwrap_err().to_string();
    assert!(err.contains("duplicate service/resource/sub_resource entry"));
}

#[test]
async fn google_workspace_allowed_operations_allow_same_resource_different_sub_resource() {
    let mut config = Config::default();
    config.google_workspace.allowed_operations = vec![
        GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("messages".into()),
            methods: vec!["list".into(), "get".into()],
        },
        GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("drafts".into()),
            methods: vec!["create".into(), "update".into()],
        },
    ];

    assert!(config.validate().is_ok());
}

#[test]
async fn google_workspace_allowed_operations_reject_duplicate_methods_within_entry() {
    let mut config = Config::default();
    config.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
        service: "gmail".into(),
        resource: "users".into(),
        sub_resource: Some("drafts".into()),
        methods: vec!["create".into(), "create".into()],
    }];

    let err = config.validate().unwrap_err().to_string();
    assert!(
        err.contains("duplicate entry"),
        "expected duplicate entry error, got: {err}"
    );
}

#[test]
async fn google_workspace_allowed_operations_accept_valid_entries() {
    let mut config = Config::default();
    config.google_workspace.allowed_operations = vec![
        GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("messages".into()),
            methods: vec!["list".into(), "get".into()],
        },
        GoogleWorkspaceAllowedOperation {
            service: "drive".into(),
            resource: "files".into(),
            sub_resource: None,
            methods: vec!["list".into(), "get".into()],
        },
    ];

    assert!(config.validate().is_ok());
}

#[test]
async fn google_workspace_allowed_operations_reject_invalid_sub_resource_characters() {
    let mut config = Config::default();
    config.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
        service: "gmail".into(),
        resource: "users".into(),
        sub_resource: Some("bad resource!".into()),
        methods: vec!["list".into()],
    }];

    let err = config.validate().unwrap_err().to_string();
    assert!(err.contains("sub_resource contains invalid characters"));
}

fn runtime_proxy_cache_contains(cache_key: &str) -> bool {
    match runtime_proxy_client_cache().read() {
        Ok(guard) => guard.contains_key(cache_key),
        Err(poisoned) => poisoned.into_inner().contains_key(cache_key),
    }
}

#[test]
async fn runtime_proxy_client_cache_reuses_default_profile_key() {
    let service_key = format!(
        "provider.cache_test.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    );
    let cache_key = runtime_proxy_cache_key(&service_key, None, None);

    clear_runtime_proxy_client_cache();
    assert!(!runtime_proxy_cache_contains(&cache_key));

    let _ = build_runtime_proxy_client(&service_key);
    assert!(runtime_proxy_cache_contains(&cache_key));

    let _ = build_runtime_proxy_client(&service_key);
    assert!(runtime_proxy_cache_contains(&cache_key));
}

#[test]
async fn set_runtime_proxy_config_clears_runtime_proxy_client_cache() {
    let service_key = format!(
        "provider.cache_timeout_test.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    );
    let cache_key = runtime_proxy_cache_key(&service_key, Some(30), Some(5));

    clear_runtime_proxy_client_cache();
    let _ = build_runtime_proxy_client_with_timeouts(&service_key, 30, 5);
    assert!(runtime_proxy_cache_contains(&cache_key));

    set_runtime_proxy_config(ProxyConfig::default());
    assert!(!runtime_proxy_cache_contains(&cache_key));
}

#[test]
async fn gateway_config_default_values() {
    let g = GatewayConfig::default();
    assert_eq!(g.port, 42617);
    assert_eq!(g.host, "127.0.0.1");
    assert!(g.require_pairing);
    assert!(!g.allow_public_bind);
    assert!(g.paired_tokens.is_empty());
    assert!(!g.trust_forwarded_headers);
    assert_eq!(g.rate_limit_max_keys, 10_000);
    assert_eq!(g.idempotency_max_keys, 10_000);
}

// ── Peripherals config ───────────────────────────────────────

#[test]
async fn peripherals_config_default_disabled() {
    let p = PeripheralsConfig::default();
    assert!(!p.enabled);
    assert!(p.boards.is_empty());
}

#[test]
async fn peripheral_board_config_defaults() {
    let b = PeripheralBoardConfig::default();
    assert!(b.board.is_empty());
    assert_eq!(b.transport, "serial");
    assert!(b.path.is_none());
    assert_eq!(b.baud, 115_200);
}

#[test]
async fn peripherals_config_toml_roundtrip() {
    let p = PeripheralsConfig {
        enabled: true,
        boards: vec![PeripheralBoardConfig {
            board: "nucleo-f401re".into(),
            transport: "serial".into(),
            path: Some("/dev/ttyACM0".into()),
            baud: 115_200,
        }],
        datasheet_dir: None,
    };
    let toml_str = toml::to_string(&p).unwrap();
    let parsed: PeripheralsConfig = toml::from_str(&toml_str).unwrap();
    assert!(parsed.enabled);
    assert_eq!(parsed.boards.len(), 1);
    assert_eq!(parsed.boards[0].board, "nucleo-f401re");
    assert_eq!(parsed.boards[0].path.as_deref(), Some("/dev/ttyACM0"));
}

#[test]
async fn lark_config_serde() {
    let lc = LarkConfig {
        enabled: true,
        app_id: "cli_123456".into(),
        app_secret: "secret_abc".into(),
        encrypt_key: Some("encrypt_key".into()),
        verification_token: Some("verify_token".into()),
        allowed_users: vec!["user_123".into(), "user_456".into()],
        mention_only: false,
        use_feishu: true,
        receive_mode: LarkReceiveMode::Websocket,
        port: None,
        proxy_url: None,
    };
    let json = serde_json::to_string(&lc).unwrap();
    let parsed: LarkConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.app_id, "cli_123456");
    assert_eq!(parsed.app_secret, "secret_abc");
    assert_eq!(parsed.encrypt_key.as_deref(), Some("encrypt_key"));
    assert_eq!(parsed.verification_token.as_deref(), Some("verify_token"));
    assert_eq!(parsed.allowed_users.len(), 2);
    assert!(parsed.use_feishu);
}

#[test]
async fn lark_config_toml_roundtrip() {
    let lc = LarkConfig {
        enabled: true,
        app_id: "cli_123456".into(),
        app_secret: "secret_abc".into(),
        encrypt_key: Some("encrypt_key".into()),
        verification_token: Some("verify_token".into()),
        allowed_users: vec!["*".into()],
        mention_only: false,
        use_feishu: false,
        receive_mode: LarkReceiveMode::Webhook,
        port: Some(9898),
        proxy_url: None,
    };
    let toml_str = toml::to_string(&lc).unwrap();
    let parsed: LarkConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.app_id, "cli_123456");
    assert_eq!(parsed.app_secret, "secret_abc");
    assert!(!parsed.use_feishu);
}

#[test]
async fn lark_config_deserializes_without_optional_fields() {
    let json = r#"{"app_id":"cli_123","app_secret":"secret"}"#;
    let parsed: LarkConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.encrypt_key.is_none());
    assert!(parsed.verification_token.is_none());
    assert!(parsed.allowed_users.is_empty());
    assert!(!parsed.mention_only);
    assert!(!parsed.use_feishu);
}

#[test]
async fn lark_config_defaults_to_lark_endpoint() {
    let json = r#"{"app_id":"cli_123","app_secret":"secret"}"#;
    let parsed: LarkConfig = serde_json::from_str(json).unwrap();
    assert!(
        !parsed.use_feishu,
        "use_feishu should default to false (Lark)"
    );
}

#[test]
async fn lark_config_with_wildcard_allowed_users() {
    let json = r#"{"app_id":"cli_123","app_secret":"secret","allowed_users":["*"]}"#;
    let parsed: LarkConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.allowed_users, vec!["*"]);
}

#[test]
async fn feishu_config_serde() {
    let fc = FeishuConfig {
        enabled: true,
        app_id: "cli_feishu_123".into(),
        app_secret: "secret_abc".into(),
        encrypt_key: Some("encrypt_key".into()),
        verification_token: Some("verify_token".into()),
        allowed_users: vec!["user_123".into(), "user_456".into()],
        mention_only: false,
        receive_mode: LarkReceiveMode::Websocket,
        port: None,
        proxy_url: None,
    };
    let json = serde_json::to_string(&fc).unwrap();
    let parsed: FeishuConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.app_id, "cli_feishu_123");
    assert_eq!(parsed.app_secret, "secret_abc");
    assert_eq!(parsed.encrypt_key.as_deref(), Some("encrypt_key"));
    assert_eq!(parsed.verification_token.as_deref(), Some("verify_token"));
    assert_eq!(parsed.allowed_users.len(), 2);
}

#[test]
async fn feishu_config_toml_roundtrip() {
    let fc = FeishuConfig {
        enabled: true,
        app_id: "cli_feishu_123".into(),
        app_secret: "secret_abc".into(),
        encrypt_key: Some("encrypt_key".into()),
        verification_token: Some("verify_token".into()),
        allowed_users: vec!["*".into()],
        mention_only: false,
        receive_mode: LarkReceiveMode::Webhook,
        port: Some(9898),
        proxy_url: None,
    };
    let toml_str = toml::to_string(&fc).unwrap();
    let parsed: FeishuConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.app_id, "cli_feishu_123");
    assert_eq!(parsed.app_secret, "secret_abc");
    assert_eq!(parsed.receive_mode, LarkReceiveMode::Webhook);
    assert_eq!(parsed.port, Some(9898));
}

#[test]
async fn feishu_config_deserializes_without_optional_fields() {
    let json = r#"{"app_id":"cli_123","app_secret":"secret"}"#;
    let parsed: FeishuConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.encrypt_key.is_none());
    assert!(parsed.verification_token.is_none());
    assert!(parsed.allowed_users.is_empty());
    assert_eq!(parsed.receive_mode, LarkReceiveMode::Websocket);
    assert!(parsed.port.is_none());
}

// ── LINE ──────────────────────────────────────────────────

#[test]
async fn line_config_toml_roundtrip() {
    // Full [channels.line] TOML block — covers every user-facing field.
    //
    // channel_access_token and channel_secret can be omitted here and
    // supplied via LINE_CHANNEL_ACCESS_TOKEN / LINE_CHANNEL_SECRET env vars
    // instead; both fields default to "" when absent.
    let toml = r#"
[channels_config.line]
enabled = true
channel_access_token = "ChannelAccessToken=="
channel_secret = "abc123secret"
dm_policy = "pairing"
group_policy = "mention"
allowed_users = []
webhook_port = 8443
"#;
    let config: Config = toml::from_str(toml).unwrap();
    let ln = config.channels.line.as_ref().unwrap();
    assert!(ln.enabled);
    assert_eq!(ln.channel_access_token, "ChannelAccessToken==");
    assert_eq!(ln.channel_secret, "abc123secret");
    assert_eq!(ln.dm_policy, LineDmPolicy::Pairing);
    assert_eq!(ln.group_policy, LineGroupPolicy::Mention);
    assert_eq!(ln.webhook_port, 8443);
    assert!(ln.proxy_url.is_none());
}

#[test]
async fn line_config_defaults() {
    // Minimal config — only the required secret fields are provided.
    // All optional fields should resolve to documented defaults.
    let toml = r#"
[channels_config.line]
channel_access_token = "tok"
channel_secret = "sec"
"#;
    let config: Config = toml::from_str(toml).unwrap();
    let ln = config.channels.line.as_ref().unwrap();
    assert!(!ln.enabled, "enabled should default to false");
    assert_eq!(
        ln.dm_policy,
        LineDmPolicy::Pairing,
        "dm_policy default is pairing"
    );
    assert_eq!(
        ln.group_policy,
        LineGroupPolicy::Mention,
        "group_policy default is mention"
    );
    assert_eq!(ln.webhook_port, 8443, "webhook_port default is 8443");
    assert!(ln.allowed_users.is_empty());
    assert!(ln.proxy_url.is_none());
}

#[test]
async fn line_config_allowlist_policy() {
    // dm_policy = allowlist with an explicit user ID list.
    let toml = r#"
[channels_config.line]
channel_access_token = "tok"
channel_secret = "sec"
dm_policy = "allowlist"
allowed_users = ["Uabc123", "Udef456"]
"#;
    let config: Config = toml::from_str(toml).unwrap();
    let ln = config.channels.line.as_ref().unwrap();
    assert_eq!(ln.dm_policy, LineDmPolicy::Allowlist);
    assert_eq!(ln.allowed_users, vec!["Uabc123", "Udef456"]);
}

#[test]
async fn line_config_open_policies() {
    // dm_policy = open + group_policy = open — most permissive combination.
    let toml = r#"
[channels_config.line]
channel_access_token = "tok"
channel_secret = "sec"
dm_policy = "open"
group_policy = "open"
"#;
    let config: Config = toml::from_str(toml).unwrap();
    let ln = config.channels.line.as_ref().unwrap();
    assert_eq!(ln.dm_policy, LineDmPolicy::Open);
    assert_eq!(ln.group_policy, LineGroupPolicy::Open);
}

#[test]
async fn line_config_group_disabled() {
    // group_policy = disabled — bot ignores all group/room messages.
    let toml = r#"
[channels_config.line]
channel_access_token = "tok"
channel_secret = "sec"
group_policy = "disabled"
"#;
    let config: Config = toml::from_str(toml).unwrap();
    let ln = config.channels.line.as_ref().unwrap();
    assert_eq!(ln.group_policy, LineGroupPolicy::Disabled);
}

#[test]
async fn nextcloud_talk_config_serde() {
    let nc = NextcloudTalkConfig {
        enabled: true,
        base_url: "https://cloud.example.com".into(),
        app_token: "app-token".into(),
        webhook_secret: Some("webhook-secret".into()),
        allowed_users: vec!["user_a".into(), "*".into()],
        proxy_url: None,
        bot_name: None,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1000,
    };

    let json = serde_json::to_string(&nc).unwrap();
    let parsed: NextcloudTalkConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.base_url, "https://cloud.example.com");
    assert_eq!(parsed.app_token, "app-token");
    assert_eq!(parsed.webhook_secret.as_deref(), Some("webhook-secret"));
    assert_eq!(parsed.allowed_users, vec!["user_a", "*"]);
}

#[test]
async fn nextcloud_talk_config_defaults_optional_fields() {
    let json = r#"{"base_url":"https://cloud.example.com","app_token":"app-token"}"#;
    let parsed: NextcloudTalkConfig = serde_json::from_str(json).unwrap();
    assert!(parsed.webhook_secret.is_none());
    assert!(parsed.allowed_users.is_empty());
}

// ── Config file permission hardening (Unix only) ───────────────

#[cfg(unix)]
#[test]
async fn new_config_file_has_restricted_permissions() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    // Create a config and save it
    let config = Config {
        config_path: config_path.clone(),
        ..Default::default()
    };
    config.save().await.unwrap();

    let meta = fs::metadata(&config_path).await.unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "New config file should be owner-only (0600), got {mode:o}"
    );
}

#[cfg(unix)]
#[test]
async fn save_restricts_existing_world_readable_config_to_owner_only() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let mut config = Config {
        config_path: config_path.clone(),
        ..Default::default()
    };
    config.save().await.unwrap();

    // Simulate the regression state observed in issue #1345.
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let loose_mode = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        loose_mode, 0o644,
        "test setup requires world-readable config"
    );

    if let Some(entry) = config.providers.fallback_provider_mut() {
        entry.temperature = Some(0.6);
    }
    config.save().await.unwrap();

    let hardened_mode = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        hardened_mode, 0o600,
        "Saving config should restore owner-only permissions (0600)"
    );
}

#[cfg(unix)]
#[test]
async fn world_readable_config_is_detectable() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    // Create a config file with intentionally loose permissions
    std::fs::write(&config_path, "# test config").unwrap();
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let meta = std::fs::metadata(&config_path).unwrap();
    let mode = meta.permissions().mode();
    assert!(
        mode & 0o004 != 0,
        "Test setup: file should be world-readable (mode {mode:o})"
    );
}

#[test]
async fn transcription_config_defaults() {
    let tc = TranscriptionConfig::default();
    assert!(!tc.enabled);
    assert!(tc.api_url.contains("groq.com"));
    assert_eq!(tc.model, "whisper-large-v3-turbo");
    assert!(tc.language.is_none());
    assert_eq!(tc.max_duration_secs, 120);
    assert!(!tc.transcribe_non_ptt_audio);
}

#[test]
async fn config_roundtrip_with_transcription() {
    let mut config = Config::default();
    config.transcription.enabled = true;
    config.transcription.language = Some("en".into());

    let toml_str = toml::to_string_pretty(&config).unwrap();
    let parsed = parse_test_config(&toml_str);

    assert!(parsed.transcription.enabled);
    assert_eq!(parsed.transcription.language.as_deref(), Some("en"));
    assert_eq!(parsed.transcription.model, "whisper-large-v3-turbo");
}

#[test]
async fn config_without_transcription_uses_defaults() {
    let toml_str = r#"
            default_provider = "openrouter"
            default_model = "test-model"
            default_temperature = 0.7
        "#;
    let parsed = parse_test_config(toml_str);
    assert!(!parsed.transcription.enabled);
    assert_eq!(parsed.transcription.max_duration_secs, 120);
}

#[test]
async fn security_defaults_are_backward_compatible() {
    let parsed = parse_test_config(
        r#"
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4.6"
default_temperature = 0.7
"#,
    );

    assert!(!parsed.security.otp.enabled);
    assert_eq!(parsed.security.otp.method, OtpMethod::Totp);
    assert!(!parsed.security.estop.enabled);
    assert!(parsed.security.estop.require_otp_to_resume);
}

#[test]
async fn security_toml_parses_otp_and_estop_sections() {
    let parsed = parse_test_config(
        r#"
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4.6"
default_temperature = 0.7

[security.otp]
enabled = true
method = "totp"
token_ttl_secs = 30
cache_valid_secs = 120
gated_actions = ["shell", "browser_open"]
gated_domains = ["*.chase.com", "accounts.google.com"]
gated_domain_categories = ["banking"]

[security.estop]
enabled = true
state_file = "~/.operant/estop-state.json"
require_otp_to_resume = true
"#,
    );

    assert!(parsed.security.otp.enabled);
    assert!(parsed.security.estop.enabled);
    assert_eq!(parsed.security.otp.gated_actions.len(), 2);
    assert_eq!(parsed.security.otp.gated_domains.len(), 2);
    parsed.validate().unwrap();
}

#[test]
async fn security_validation_rejects_invalid_domain_glob() {
    let mut config = Config::default();
    config.security.otp.gated_domains = vec!["bad domain.com".into()];

    let err = config.validate().expect_err("expected invalid domain glob");
    assert!(err.to_string().contains("gated_domains"));
}

#[test]
async fn validate_accepts_local_whisper_as_transcription_default_provider() {
    let mut config = Config::default();
    config.transcription.default_provider = "local_whisper".to_string();

    config
        .validate()
        .expect("local_whisper must be accepted by the transcription.default_provider allowlist");
}

#[test]
async fn validate_rejects_unknown_transcription_default_provider() {
    let mut config = Config::default();
    config.transcription.default_provider = "unknown_stt".to_string();

    let err = config
        .validate()
        .expect_err("expected validation to reject unknown transcription provider");
    assert!(
        err.to_string().contains("transcription.default_provider"),
        "got: {err}"
    );
}

#[tokio::test]
async fn channel_secret_telegram_bot_token_roundtrip() {
    let dir = std::env::temp_dir().join(format!(
        "operant_test_tg_bot_token_{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).await.unwrap();

    let plaintext_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";

    let mut config = Config {
        workspace_dir: dir.join("workspace"),
        config_path: dir.join("config.toml"),
        ..Default::default()
    };
    config.channels.telegram = Some(TelegramConfig {
        enabled: true,
        bot_token: plaintext_token.into(),
        allowed_users: vec!["user1".into()],
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: default_draft_update_interval_ms(),
        interrupt_on_new_message: false,
        mention_only: false,
        ack_reactions: None,
        proxy_url: None,
        approval_timeout_secs: default_telegram_approval_timeout_secs(),
        dm_topics_enabled: false,
        dm_topic_name: default_telegram_dm_topic_name(),
        disable_link_previews: false,
        typing_cooldown_seconds: default_telegram_typing_cooldown_secs(),
        fallback_ips: vec![],
    });

    // Save (triggers encryption)
    config.save().await.unwrap();

    // Read raw TOML and verify plaintext token is NOT present
    let raw_toml = tokio::fs::read_to_string(&config.config_path)
        .await
        .unwrap();
    assert!(
        !raw_toml.contains(plaintext_token),
        "Saved TOML must not contain the plaintext bot_token"
    );

    // Parse stored TOML and verify the value is encrypted
    let stored: Config = toml::from_str(&raw_toml).unwrap();
    let stored_token = &stored.channels.telegram.as_ref().unwrap().bot_token;
    assert!(
        crate::secrets::SecretStore::is_encrypted(stored_token),
        "Stored bot_token must be marked as encrypted"
    );

    // Decrypt and verify it matches the original plaintext
    let store = crate::secrets::SecretStore::new(&dir, true);
    assert_eq!(store.decrypt(stored_token).unwrap(), plaintext_token);

    // Simulate a full load: deserialize then decrypt (mirrors load_or_init logic)
    let mut loaded: Config = toml::from_str(&raw_toml).unwrap();
    loaded.config_path = dir.join("config.toml");
    let load_store = crate::secrets::SecretStore::new(&dir, loaded.secrets.encrypt);
    loaded.decrypt_secrets(&load_store).unwrap();
    assert_eq!(
        loaded.channels.telegram.as_ref().unwrap().bot_token,
        plaintext_token,
        "Loaded bot_token must match the original plaintext after decryption"
    );

    let _ = fs::remove_dir_all(&dir).await;
}

#[test]
async fn security_validation_rejects_unknown_domain_category() {
    let mut config = Config::default();
    config.security.otp.gated_domain_categories = vec!["not_real".into()];

    let err = config
        .validate()
        .expect_err("expected unknown domain category");
    assert!(err.to_string().contains("gated_domain_categories"));
}

#[test]
async fn security_validation_rejects_zero_token_ttl() {
    let mut config = Config::default();
    config.security.otp.token_ttl_secs = 0;

    let err = config
        .validate()
        .expect_err("expected ttl validation failure");
    assert!(err.to_string().contains("token_ttl_secs"));
}

// ── MCP config validation ─────────────────────────────────────────────

fn stdio_server(name: &str, command: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Stdio,
        command: command.to_string(),
        ..Default::default()
    }
}

fn http_server(name: &str, url: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Http,
        url: Some(url.to_string()),
        ..Default::default()
    }
}

fn sse_server(name: &str, url: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Sse,
        url: Some(url.to_string()),
        ..Default::default()
    }
}

#[test]
async fn validate_mcp_config_empty_servers_ok() {
    let cfg = McpConfig::default();
    assert!(validate_mcp_config(&cfg).is_ok());
}

#[test]
async fn validate_mcp_config_valid_stdio_ok() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![stdio_server("fs", "/usr/bin/mcp-fs")],
        ..Default::default()
    };
    assert!(validate_mcp_config(&cfg).is_ok());
}

#[test]
async fn validate_mcp_config_valid_http_ok() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![http_server("svc", "http://localhost:8080/mcp")],
        ..Default::default()
    };
    assert!(validate_mcp_config(&cfg).is_ok());
}

#[test]
async fn validate_mcp_config_valid_sse_ok() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![sse_server("svc", "https://example.com/events")],
        ..Default::default()
    };
    assert!(validate_mcp_config(&cfg).is_ok());
}

#[test]
async fn validate_mcp_config_rejects_empty_name() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![stdio_server("", "/usr/bin/tool")],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("empty name should fail");
    assert!(
        err.to_string().contains("name must not be empty"),
        "got: {err}"
    );
}

#[test]
async fn validate_mcp_config_rejects_whitespace_name() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![stdio_server("   ", "/usr/bin/tool")],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("whitespace name should fail");
    assert!(
        err.to_string().contains("name must not be empty"),
        "got: {err}"
    );
}

#[test]
async fn validate_mcp_config_rejects_duplicate_names() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![
            stdio_server("fs", "/usr/bin/mcp-a"),
            stdio_server("fs", "/usr/bin/mcp-b"),
        ],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("duplicate name should fail");
    assert!(err.to_string().contains("duplicate name"), "got: {err}");
}

#[test]
async fn validate_mcp_config_rejects_zero_timeout() {
    let mut server = stdio_server("fs", "/usr/bin/mcp-fs");
    server.tool_timeout_secs = Some(0);
    let cfg = McpConfig {
        enabled: true,
        servers: vec![server],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("zero timeout should fail");
    assert!(err.to_string().contains("greater than 0"), "got: {err}");
}

#[test]
async fn validate_mcp_config_rejects_timeout_exceeding_max() {
    let mut server = stdio_server("fs", "/usr/bin/mcp-fs");
    server.tool_timeout_secs = Some(MCP_MAX_TOOL_TIMEOUT_SECS + 1);
    let cfg = McpConfig {
        enabled: true,
        servers: vec![server],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("oversized timeout should fail");
    assert!(err.to_string().contains("exceeds max"), "got: {err}");
}

#[test]
async fn validate_mcp_config_allows_max_timeout_exactly() {
    let mut server = stdio_server("fs", "/usr/bin/mcp-fs");
    server.tool_timeout_secs = Some(MCP_MAX_TOOL_TIMEOUT_SECS);
    let cfg = McpConfig {
        enabled: true,
        servers: vec![server],
        ..Default::default()
    };
    assert!(validate_mcp_config(&cfg).is_ok());
}

#[test]
async fn validate_mcp_config_rejects_stdio_with_empty_command() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![stdio_server("fs", "")],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("empty command should fail");
    assert!(
        err.to_string().contains("requires non-empty command"),
        "got: {err}"
    );
}

#[test]
async fn validate_mcp_config_rejects_http_without_url() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![McpServerConfig {
            name: "svc".to_string(),
            transport: McpTransport::Http,
            url: None,
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("http without url should fail");
    assert!(err.to_string().contains("requires url"), "got: {err}");
}

#[test]
async fn validate_mcp_config_rejects_sse_without_url() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![McpServerConfig {
            name: "svc".to_string(),
            transport: McpTransport::Sse,
            url: None,
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("sse without url should fail");
    assert!(err.to_string().contains("requires url"), "got: {err}");
}

#[test]
async fn validate_mcp_config_rejects_non_http_scheme() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![http_server("svc", "ftp://example.com/mcp")],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("non-http scheme should fail");
    assert!(err.to_string().contains("http/https"), "got: {err}");
}

#[test]
async fn validate_mcp_config_rejects_invalid_url() {
    let cfg = McpConfig {
        enabled: true,
        servers: vec![http_server("svc", "not a url at all !!!")],
        ..Default::default()
    };
    let err = validate_mcp_config(&cfg).expect_err("invalid url should fail");
    assert!(err.to_string().contains("valid URL"), "got: {err}");
}

#[test]
async fn mcp_config_default_disabled_with_empty_servers() {
    let cfg = McpConfig::default();
    assert!(!cfg.enabled);
    assert!(cfg.servers.is_empty());
}

#[test]
async fn mcp_transport_serde_roundtrip_lowercase() {
    let cases = [
        (McpTransport::Stdio, "\"stdio\""),
        (McpTransport::Http, "\"http\""),
        (McpTransport::Sse, "\"sse\""),
    ];
    for (variant, expected_json) in &cases {
        let serialized = serde_json::to_string(variant).expect("serialize");
        assert_eq!(&serialized, expected_json, "variant: {variant:?}");
        let deserialized: McpTransport = serde_json::from_str(expected_json).expect("deserialize");
        assert_eq!(&deserialized, variant);
    }
}

#[test]
async fn swarm_strategy_roundtrip() {
    let cases = vec![
        (SwarmStrategy::Sequential, "\"sequential\""),
        (SwarmStrategy::Parallel, "\"parallel\""),
        (SwarmStrategy::Router, "\"router\""),
    ];
    for (variant, expected_json) in &cases {
        let serialized = serde_json::to_string(variant).expect("serialize");
        assert_eq!(&serialized, expected_json, "variant: {variant:?}");
        let deserialized: SwarmStrategy = serde_json::from_str(expected_json).expect("deserialize");
        assert_eq!(&deserialized, variant);
    }
}

#[test]
async fn swarm_config_deserializes_with_defaults() {
    let toml_str = r#"
            agents = ["researcher", "writer"]
            strategy = "sequential"
        "#;
    let config: SwarmConfig = toml::from_str(toml_str).expect("deserialize");
    assert_eq!(config.agents, vec!["researcher", "writer"]);
    assert_eq!(config.strategy, SwarmStrategy::Sequential);
    assert!(config.router_prompt.is_none());
    assert!(config.description.is_none());
    assert_eq!(config.timeout_secs, 300);
}

#[test]
async fn swarm_config_deserializes_full() {
    let toml_str = r#"
            agents = ["a", "b", "c"]
            strategy = "router"
            router_prompt = "Pick the best."
            description = "Multi-agent router"
            timeout_secs = 120
        "#;
    let config: SwarmConfig = toml::from_str(toml_str).expect("deserialize");
    assert_eq!(config.agents.len(), 3);
    assert_eq!(config.strategy, SwarmStrategy::Router);
    assert_eq!(config.router_prompt.as_deref(), Some("Pick the best."));
    assert_eq!(config.description.as_deref(), Some("Multi-agent router"));
    assert_eq!(config.timeout_secs, 120);
}

#[test]
async fn config_with_swarms_section_deserializes() {
    let toml_str = r#"
            [agents.researcher]
            provider = "ollama"
            model = "llama3"

            [agents.writer]
            provider = "openrouter"
            model = "claude-sonnet"

            [swarms.pipeline]
            agents = ["researcher", "writer"]
            strategy = "sequential"
        "#;
    let config = parse_test_config(toml_str);
    assert_eq!(config.agents.len(), 2);
    assert_eq!(config.swarms.len(), 1);
    assert!(config.swarms.contains_key("pipeline"));
}

#[tokio::test]
async fn nevis_client_secret_encrypt_decrypt_roundtrip() {
    let dir = std::env::temp_dir().join(format!(
        "operant_test_nevis_secret_{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).await.unwrap();

    let plaintext_secret = "nevis-test-client-secret-value";

    let mut config = Config {
        workspace_dir: dir.join("workspace"),
        config_path: dir.join("config.toml"),
        ..Default::default()
    };
    config.security.nevis.client_secret = Some(plaintext_secret.into());

    // Save (triggers encryption)
    config.save().await.unwrap();

    // Read raw TOML and verify plaintext secret is NOT present
    let raw_toml = tokio::fs::read_to_string(&config.config_path)
        .await
        .unwrap();
    assert!(
        !raw_toml.contains(plaintext_secret),
        "Saved TOML must not contain the plaintext client_secret"
    );

    // Parse stored TOML and verify the value is encrypted
    let stored: Config = toml::from_str(&raw_toml).unwrap();
    let stored_secret = stored.security.nevis.client_secret.as_ref().unwrap();
    assert!(
        crate::secrets::SecretStore::is_encrypted(stored_secret),
        "Stored client_secret must be marked as encrypted"
    );

    // Decrypt and verify it matches the original plaintext
    let store = crate::secrets::SecretStore::new(&dir, true);
    assert_eq!(store.decrypt(stored_secret).unwrap(), plaintext_secret);

    // Simulate a full load: deserialize then decrypt (mirrors load_or_init logic)
    let mut loaded: Config = toml::from_str(&raw_toml).unwrap();
    loaded.config_path = dir.join("config.toml");
    let load_store = crate::secrets::SecretStore::new(&dir, loaded.secrets.encrypt);
    loaded.decrypt_secrets(&load_store).unwrap();
    assert_eq!(
        loaded.security.nevis.client_secret.as_deref().unwrap(),
        plaintext_secret,
        "Loaded client_secret must match the original plaintext after decryption"
    );

    let _ = fs::remove_dir_all(&dir).await;
}

// ══════════════════════════════════════════════════════════
// Nevis config validation tests
// ══════════════════════════════════════════════════════════

#[test]
async fn nevis_config_validate_disabled_accepts_empty_fields() {
    let cfg = NevisConfig::default();
    assert!(!cfg.enabled);
    assert!(cfg.validate().is_ok());
}

#[test]
async fn nevis_config_validate_rejects_empty_instance_url() {
    let cfg = NevisConfig {
        enabled: true,
        instance_url: String::new(),
        client_id: "test-client".into(),
        ..NevisConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.contains("instance_url"));
}

#[test]
async fn nevis_config_validate_rejects_empty_client_id() {
    let cfg = NevisConfig {
        enabled: true,
        instance_url: "https://nevis.example.com".into(),
        client_id: String::new(),
        ..NevisConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.contains("client_id"));
}

#[test]
async fn nevis_config_validate_rejects_empty_realm() {
    let cfg = NevisConfig {
        enabled: true,
        instance_url: "https://nevis.example.com".into(),
        client_id: "test-client".into(),
        realm: String::new(),
        ..NevisConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.contains("realm"));
}

#[test]
async fn nevis_config_validate_rejects_local_without_jwks() {
    let cfg = NevisConfig {
        enabled: true,
        instance_url: "https://nevis.example.com".into(),
        client_id: "test-client".into(),
        token_validation: "local".into(),
        jwks_url: None,
        ..NevisConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.contains("jwks_url"));
}

#[test]
async fn nevis_config_validate_rejects_zero_session_timeout() {
    let cfg = NevisConfig {
        enabled: true,
        instance_url: "https://nevis.example.com".into(),
        client_id: "test-client".into(),
        token_validation: "remote".into(),
        session_timeout_secs: 0,
        ..NevisConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.contains("session_timeout_secs"));
}

#[test]
async fn nevis_config_validate_accepts_valid_enabled_config() {
    let cfg = NevisConfig {
        enabled: true,
        instance_url: "https://nevis.example.com".into(),
        realm: "master".into(),
        client_id: "test-client".into(),
        token_validation: "remote".into(),
        session_timeout_secs: 3600,
        ..NevisConfig::default()
    };
    assert!(cfg.validate().is_ok());
}

#[test]
async fn nevis_config_validate_rejects_invalid_token_validation() {
    let cfg = NevisConfig {
        enabled: true,
        instance_url: "https://nevis.example.com".into(),
        realm: "master".into(),
        client_id: "test-client".into(),
        token_validation: "invalid_mode".into(),
        session_timeout_secs: 3600,
        ..NevisConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(
        err.contains("invalid value 'invalid_mode'"),
        "Expected invalid token_validation error, got: {err}"
    );
}

#[test]
async fn nevis_config_debug_redacts_client_secret() {
    let cfg = NevisConfig {
        client_secret: Some("super-secret".into()),
        ..NevisConfig::default()
    };
    let debug_output = format!("{:?}", cfg);
    assert!(
        !debug_output.contains("super-secret"),
        "Debug output must not contain the raw client_secret"
    );
    assert!(
        debug_output.contains("[REDACTED]"),
        "Debug output must show [REDACTED] for client_secret"
    );
}

#[test]
async fn telegram_config_ack_reactions_false_deserializes() {
    let toml_str = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
            ack_reactions = false
        "#;
    let cfg: TelegramConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.ack_reactions, Some(false));
}

#[test]
async fn telegram_config_ack_reactions_true_deserializes() {
    let toml_str = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
            ack_reactions = true
        "#;
    let cfg: TelegramConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.ack_reactions, Some(true));
}

#[test]
async fn telegram_config_ack_reactions_missing_defaults_to_none() {
    let toml_str = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
        "#;
    let cfg: TelegramConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.ack_reactions, None);
}

#[test]
async fn telegram_config_ack_reactions_channel_overrides_top_level() {
    let tg_toml = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
            ack_reactions = false
        "#;
    let tg: TelegramConfig = toml::from_str(tg_toml).unwrap();
    let top_level_ack = true;
    let effective = tg.ack_reactions.unwrap_or(top_level_ack);
    assert!(
        !effective,
        "channel-level false must override top-level true"
    );
}

#[test]
async fn telegram_config_ack_reactions_falls_back_to_top_level() {
    let tg_toml = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
        "#;
    let tg: TelegramConfig = toml::from_str(tg_toml).unwrap();
    let top_level_ack = false;
    let effective = tg.ack_reactions.unwrap_or(top_level_ack);
    assert!(
        !effective,
        "must fall back to top-level false when channel omits field"
    );
}

#[test]
async fn google_workspace_allowed_operations_deserialize_from_toml() {
    let toml_str = r#"
            enabled = true

            [[allowed_operations]]
            service = "gmail"
            resource = "users"
            sub_resource = "drafts"
            methods = ["create", "update"]
        "#;

    let cfg: GoogleWorkspaceConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.allowed_operations.len(), 1);
    assert_eq!(cfg.allowed_operations[0].service, "gmail");
    assert_eq!(cfg.allowed_operations[0].resource, "users");
    assert_eq!(
        cfg.allowed_operations[0].sub_resource.as_deref(),
        Some("drafts")
    );
    assert_eq!(
        cfg.allowed_operations[0].methods,
        vec!["create".to_string(), "update".to_string()]
    );
}

#[test]
async fn google_workspace_allowed_operations_deserialize_without_sub_resource() {
    let toml_str = r#"
            enabled = true

            [[allowed_operations]]
            service = "drive"
            resource = "files"
            methods = ["list", "get"]
        "#;

    let cfg: GoogleWorkspaceConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.allowed_operations[0].sub_resource, None);
}

#[test]
async fn config_validate_accepts_google_workspace_allowed_operations() {
    let mut cfg = Config::default();
    cfg.google_workspace.enabled = true;
    cfg.google_workspace.allowed_services = vec!["gmail".into()];
    cfg.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
        service: "gmail".into(),
        resource: "users".into(),
        sub_resource: Some("drafts".into()),
        methods: vec!["create".into(), "update".into()],
    }];

    cfg.validate().unwrap();
}

#[test]
async fn config_validate_rejects_duplicate_google_workspace_allowed_operations() {
    let mut cfg = Config::default();
    cfg.google_workspace.enabled = true;
    cfg.google_workspace.allowed_services = vec!["gmail".into()];
    cfg.google_workspace.allowed_operations = vec![
        GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("drafts".into()),
            methods: vec!["create".into()],
        },
        GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("drafts".into()),
            methods: vec!["update".into()],
        },
    ];

    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("duplicate service/resource/sub_resource entry"));
}

#[test]
async fn config_validate_rejects_operation_service_not_in_allowed_services() {
    let mut cfg = Config::default();
    cfg.google_workspace.enabled = true;
    cfg.google_workspace.allowed_services = vec!["gmail".into()];
    cfg.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
        service: "drive".into(), // drive is not in allowed_services
        resource: "files".into(),
        sub_resource: None,
        methods: vec!["list".into()],
    }];

    let err = cfg.validate().unwrap_err().to_string();
    assert!(
        err.contains("not in the effective allowed_services"),
        "expected not-in-allowed_services error, got: {err}"
    );
}

#[test]
async fn config_validate_accepts_default_service_when_allowed_services_empty() {
    // When allowed_services is empty the validator uses DEFAULT_GWS_SERVICES.
    // A known default service must pass.
    let mut cfg = Config::default();
    cfg.google_workspace.enabled = true;
    // allowed_services deliberately left empty (falls back to defaults)
    cfg.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
        service: "drive".into(),
        resource: "files".into(),
        sub_resource: None,
        methods: vec!["list".into()],
    }];

    assert!(cfg.validate().is_ok());
}

#[test]
async fn config_validate_rejects_unknown_service_when_allowed_services_empty() {
    // Even with allowed_services empty (using defaults), an operation whose
    // service is not in DEFAULT_GWS_SERVICES must fail validation — not silently
    // pass through to be rejected at runtime.
    let mut cfg = Config::default();
    cfg.google_workspace.enabled = true;
    // allowed_services deliberately left empty
    cfg.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
        service: "not_a_real_service".into(),
        resource: "files".into(),
        sub_resource: None,
        methods: vec!["list".into()],
    }];

    let err = cfg.validate().unwrap_err().to_string();
    assert!(
        err.contains("not in the effective allowed_services"),
        "expected effective-allowed_services error, got: {err}"
    );
}

// ── Bootstrap files ─────────────────────────────────────

#[tokio::test]
async fn ensure_bootstrap_files_creates_missing_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let _: () = tokio::fs::create_dir_all(&ws).await.unwrap();

    ensure_bootstrap_files(&ws).await.unwrap();

    let soul: String = tokio::fs::read_to_string(ws.join("SOUL.md")).await.unwrap();
    let identity: String = tokio::fs::read_to_string(ws.join("IDENTITY.md"))
        .await
        .unwrap();
    assert!(soul.contains("SOUL.md"));
    assert!(identity.contains("IDENTITY.md"));
}

#[tokio::test]
async fn ensure_bootstrap_files_does_not_overwrite_existing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let _: () = tokio::fs::create_dir_all(&ws).await.unwrap();

    let custom = "# My custom SOUL";
    let _: () = tokio::fs::write(ws.join("SOUL.md"), custom).await.unwrap();

    ensure_bootstrap_files(&ws).await.unwrap();

    let soul: String = tokio::fs::read_to_string(ws.join("SOUL.md")).await.unwrap();
    assert_eq!(
        soul, custom,
        "ensure_bootstrap_files must not overwrite existing files"
    );

    // IDENTITY.md should still be created since it was missing
    let identity: String = tokio::fs::read_to_string(ws.join("IDENTITY.md"))
        .await
        .unwrap();
    assert!(identity.contains("IDENTITY.md"));
}

// ── PacingConfig serde defaults ─────────────────────────────

#[test]
async fn pacing_config_serde_defaults_match_manual_default() {
    // Deserialise an empty TOML table and verify the loop-detection
    // fields receive the same defaults as `PacingConfig::default()`.
    let from_toml: PacingConfig = toml::from_str("").unwrap();
    let manual = PacingConfig::default();

    assert_eq!(
        from_toml.loop_detection_enabled,
        manual.loop_detection_enabled
    );
    assert_eq!(
        from_toml.loop_detection_window_size,
        manual.loop_detection_window_size
    );
    assert_eq!(
        from_toml.loop_detection_max_repeats,
        manual.loop_detection_max_repeats
    );

    // Verify concrete values so a silent change to the defaults is caught.
    assert!(from_toml.loop_detection_enabled, "default should be true");
    assert_eq!(from_toml.loop_detection_window_size, 20);
    assert_eq!(from_toml.loop_detection_max_repeats, 3);
}

// ── Docker baked config template ────────────────────────────

/// The TOML template baked into Docker images (Dockerfile + Dockerfile.debian).
/// Kept here so changes to the Dockerfiles can be validated by `cargo test`.
const DOCKER_CONFIG_TEMPLATE: &str = r#"
workspace_dir = "/operant-data/workspace"
config_path = "/operant-data/.operant/config.toml"
api_key = ""
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4-20250514"
default_temperature = 0.7

[gateway]
port = 42617
host = "[::]"
allow_public_bind = true

[autonomy]
level = "supervised"
auto_approve = ["file_read", "file_write", "file_edit", "memory_recall", "memory_store", "web_search_tool", "web_fetch", "calculator", "glob_search", "content_search", "image_info", "weather", "git_operations"]
"#;

#[test]
async fn docker_config_template_is_parseable() {
    let cfg: Config = toml::from_str(DOCKER_CONFIG_TEMPLATE)
        .expect("Docker baked config.toml must be valid TOML that deserialises into Config");

    // The [autonomy] section must be present and contain the expected tools.
    let auto = &cfg.autonomy.auto_approve;
    for tool in &[
        "file_read",
        "file_write",
        "file_edit",
        "memory_recall",
        "memory_store",
        "web_search_tool",
        "web_fetch",
        "calculator",
        "glob_search",
        "content_search",
        "image_info",
        "weather",
        "git_operations",
    ] {
        assert!(
            auto.iter().any(|t| t == tool),
            "Docker config auto_approve missing expected tool: {tool}"
        );
    }
}

#[test]
async fn cost_enforcement_config_defaults() {
    let config = CostEnforcementConfig::default();
    assert_eq!(config.mode, "warn");
    assert_eq!(config.route_down_model, None);
    assert_eq!(config.reserve_percent, 10);
}

#[test]
async fn cost_config_includes_enforcement() {
    let config = CostConfig::default();
    assert_eq!(config.enforcement.mode, "warn");
    assert_eq!(config.enforcement.reserve_percent, 10);
}

// ── Configurable macro tests ──

#[test]
async fn matrix_secret_fields_discovered() {
    let mx = MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "tok".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };
    let fields = mx.secret_fields();
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0].name, "channels.matrix.access-token");
    assert_eq!(fields[0].category, "Channels");
    assert!(fields[0].is_set);
    assert_eq!(fields[1].name, "channels.matrix.recovery-key");
    assert!(!fields[1].is_set);
    assert_eq!(fields[2].name, "channels.matrix.password");
    assert!(!fields[2].is_set);
}

#[test]
async fn matrix_secret_fields_empty_not_set() {
    let mx = MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: String::new(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };
    let fields = mx.secret_fields();
    assert!(!fields[0].is_set);
}

#[test]
async fn set_secret_updates_field() {
    let mut mx = MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "old".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };
    mx.set_secret("channels.matrix.access-token", "new-token".into())
        .unwrap();
    assert_eq!(mx.access_token, "new-token");
}

#[test]
async fn set_secret_unknown_name_fails() {
    let mut mx = MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "tok".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };
    assert!(
        mx.set_secret("channels.matrix.nonexistent", "val".into())
            .is_err()
    );
}

#[test]
async fn config_tree_traversal_discovers_nested_secrets() {
    let mut config = Config::default();
    // Set api_key on fallback provider
    if let Some(name) = config.providers.fallback.clone() {
        if let Some(entry) = config.providers.models.get_mut(&name) {
            entry.api_key = Some("test-key".into());
        }
    } else {
        config.providers.fallback = Some("default".into());
        config.providers.models.insert(
            "default".into(),
            ModelProviderConfig {
                api_key: Some("test-key".into()),
                ..Default::default()
            },
        );
    }
    config.channels.matrix = Some(MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "mx-tok".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    });

    let fields = config.secret_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
    assert!(names.contains(&"channels.matrix.access-token"));
    assert!(names.contains(&"channels.matrix.recovery-key"));
}

#[test]
async fn config_set_secret_dispatches_to_child() {
    let mut config = Config::default();
    config.channels.matrix = Some(MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "old".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    });

    config
        .set_secret("channels.matrix.access-token", "new".into())
        .unwrap();
    assert_eq!(config.channels.matrix.as_ref().unwrap().access_token, "new");
}

#[test]
async fn config_set_secret_dispatches_to_matrix_child() {
    let mut config = Config::default();
    config.channels.matrix = Some(MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "old".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        mention_only: false,
        recovery_key: None,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    });
    config
        .set_secret("channels.matrix.access-token", "sk-test".into())
        .unwrap();
    assert_eq!(
        config.channels.matrix.as_ref().unwrap().access_token,
        "sk-test"
    );
}

#[test]
async fn config_set_secret_unknown_fails() {
    let mut config = Config::default();
    assert!(
        config
            .set_secret("nonexistent.field", "val".into())
            .is_err()
    );
}

#[test]
async fn encrypt_decrypt_roundtrip_via_macro() {
    let dir = TempDir::new().unwrap();
    let store = crate::secrets::SecretStore::new(dir.path(), true);

    let mut mx = MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "plaintext-token".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };

    // Encrypt
    mx.encrypt_secrets(&store).unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(&mx.access_token));
    assert_ne!(mx.access_token, "plaintext-token");

    // Decrypt
    mx.decrypt_secrets(&store).unwrap();
    assert_eq!(mx.access_token, "plaintext-token");
}

#[test]
async fn encrypt_skips_already_encrypted() {
    let dir = TempDir::new().unwrap();
    let store = crate::secrets::SecretStore::new(dir.path(), true);

    let mut mx = MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "plaintext-token".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };

    mx.encrypt_secrets(&store).unwrap();
    let first_encrypted = mx.access_token.clone();

    // Encrypt again — should be idempotent
    mx.encrypt_secrets(&store).unwrap();
    assert_eq!(mx.access_token, first_encrypted);
}

#[test]
async fn encrypt_no_op_on_disabled_store() {
    let dir = TempDir::new().unwrap();
    let store = crate::secrets::SecretStore::new(dir.path(), false);

    let mut mx = MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "plaintext-token".into(),
        user_id: None,
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    };

    mx.encrypt_secrets(&store).unwrap();
    // With encryption disabled, value should stay plaintext
    assert_eq!(mx.access_token, "plaintext-token");
}

// ── Property method tests ──

fn test_matrix_config() -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver: "https://m.org".into(),
        access_token: "tok".into(),
        user_id: Some("@bot:m.org".into()),
        device_id: None,
        allowed_users: vec![],
        allowed_rooms: vec!["!r:m".into()],
        interrupt_on_new_message: false,
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1500,
        multi_message_delay_ms: 800,
        recovery_key: None,
        mention_only: false,
        password: None,
        approval_timeout_secs: 300,
        reply_in_thread: true,
        ack_reactions: true,
    }
}

#[test]
async fn prop_fields_returns_typed_entries() {
    let mx = test_matrix_config();
    let fields = mx.prop_fields();
    let by_name: std::collections::HashMap<&str, &crate::traits::PropFieldInfo> =
        fields.iter().map(|f| (f.name.as_str(), f)).collect();

    // Bool field
    let enabled = by_name["channels.matrix.enabled"];
    assert_eq!(enabled.type_hint, "bool");
    assert_eq!(enabled.display_value, "true");
    assert!(!enabled.is_secret);
    assert!(!enabled.is_enum());

    // String field
    let homeserver = by_name["channels.matrix.homeserver"];
    assert_eq!(homeserver.type_hint, "String");
    assert_eq!(homeserver.display_value, "https://m.org");

    // Option<String> — set
    let user_id = by_name["channels.matrix.user-id"];
    assert_eq!(user_id.type_hint, "Option<String>");
    assert_eq!(user_id.display_value, "@bot:m.org");

    // Option<String> — unset
    let device_id = by_name["channels.matrix.device-id"];
    assert_eq!(device_id.display_value, "<unset>");

    // u64 field
    let interval = by_name["channels.matrix.draft-update-interval-ms"];
    assert_eq!(interval.type_hint, "u64");
    assert_eq!(interval.display_value, "1500");

    // Enum field
    let stream = by_name["channels.matrix.stream-mode"];
    assert!(stream.is_enum());
    assert!(stream.enum_variants.is_some());

    // Secret field — masked
    let token = by_name["channels.matrix.access-token"];
    assert!(token.is_secret);
    assert_eq!(token.display_value, "****");

    // All fields have correct category
    for field in &fields {
        assert_eq!(field.category, "Channels");
    }
}

#[test]
async fn get_prop_returns_values_by_path() {
    let mx = test_matrix_config();

    assert_eq!(
        mx.get_prop("channels.matrix.homeserver").unwrap(),
        "https://m.org"
    );
    assert_eq!(mx.get_prop("channels.matrix.enabled").unwrap(), "true");
    assert_eq!(
        mx.get_prop("channels.matrix.draft-update-interval-ms")
            .unwrap(),
        "1500"
    );
    assert_eq!(
        mx.get_prop("channels.matrix.user-id").unwrap(),
        "@bot:m.org"
    );
    assert_eq!(mx.get_prop("channels.matrix.device-id").unwrap(), "<unset>");
    // Secrets return masked value
    assert_eq!(
        mx.get_prop("channels.matrix.access-token").unwrap(),
        "**** (encrypted)"
    );
}

#[test]
async fn get_prop_unknown_path_fails() {
    let mx = test_matrix_config();
    assert!(mx.get_prop("channels.matrix.nonexistent").is_err());
}

#[test]
async fn set_prop_string() {
    let mut mx = test_matrix_config();
    mx.set_prop("channels.matrix.homeserver", "https://new.org")
        .unwrap();
    assert_eq!(mx.homeserver, "https://new.org");
}

#[test]
async fn set_prop_bool() {
    let mut mx = test_matrix_config();
    mx.set_prop("channels.matrix.interrupt-on-new-message", "true")
        .unwrap();
    assert!(mx.interrupt_on_new_message);
}

#[test]
async fn set_prop_bool_rejects_invalid() {
    let mut mx = test_matrix_config();
    let err = mx.set_prop("channels.matrix.enabled", "yes").unwrap_err();
    assert!(err.to_string().contains("bool"));
}

#[test]
async fn set_prop_u64() {
    let mut mx = test_matrix_config();
    mx.set_prop("channels.matrix.draft-update-interval-ms", "3000")
        .unwrap();
    assert_eq!(mx.draft_update_interval_ms, 3000);
}

#[test]
async fn set_prop_u64_rejects_invalid() {
    let mut mx = test_matrix_config();
    assert!(
        mx.set_prop("channels.matrix.draft-update-interval-ms", "abc")
            .is_err()
    );
}

#[test]
async fn set_prop_option_string_set_and_clear() {
    let mut mx = test_matrix_config();
    mx.set_prop("channels.matrix.user-id", "@new:m.org")
        .unwrap();
    assert_eq!(mx.user_id.as_deref(), Some("@new:m.org"));

    // Empty string clears Option
    mx.set_prop("channels.matrix.user-id", "").unwrap();
    assert!(mx.user_id.is_none());
}

#[test]
async fn set_prop_enum() {
    let mut mx = test_matrix_config();
    mx.set_prop("channels.matrix.stream-mode", "partial")
        .unwrap();
    assert_eq!(mx.stream_mode, StreamMode::Partial);

    mx.set_prop("channels.matrix.stream-mode", "multi_message")
        .unwrap();
    assert_eq!(mx.stream_mode, StreamMode::MultiMessage);
}

#[test]
async fn set_prop_enum_rejects_invalid() {
    let mut mx = test_matrix_config();
    let err = mx
        .set_prop("channels.matrix.stream-mode", "invalid")
        .unwrap_err();
    assert!(err.to_string().contains("expected one of"));
}

#[test]
async fn set_prop_unknown_path_fails() {
    let mut mx = test_matrix_config();
    assert!(mx.set_prop("channels.matrix.nonexistent", "val").is_err());
}

#[test]
async fn prop_is_secret_static_check() {
    assert!(MatrixConfig::prop_is_secret("channels.matrix.access-token"));
    assert!(MatrixConfig::prop_is_secret("channels.matrix.recovery-key"));
    assert!(!MatrixConfig::prop_is_secret("channels.matrix.homeserver"));
    assert!(!MatrixConfig::prop_is_secret("channels.matrix.enabled"));
}

#[test]
async fn prop_is_secret_routes_through_hashmap_keyed_paths() {
    // Regression: the macro's HashMap<String, T> arm previously passed the
    // full materialised path (e.g. `providers.models.openrouter.api-key`)
    // straight to the inner type's `prop_is_secret`, which then matched on
    // its own configurable_prefix and returned false. Result: the CLI's
    // `config set --json` and the gateway's PropResponse both took the
    // non-secret branch and emitted `{value}` instead of `{populated}` for
    // any secret on a map-keyed nested type.
    assert!(Config::prop_is_secret(
        "providers.models.openrouter.api-key"
    ));
    assert!(Config::prop_is_secret("providers.models.default.api-key"));
    assert!(!Config::prop_is_secret(
        "providers.models.openrouter.endpoint"
    ));
    assert!(!Config::prop_is_secret(
        "providers.models.openrouter.context-window"
    ));
}

#[test]
async fn hashmap_property_paths_preserve_url_like_keys() {
    let dir = TempDir::new().unwrap();
    let mut config = Config {
        config_path: dir.path().join("config.toml"),
        workspace_dir: dir.path().join("workspace"),
        ..Default::default()
    };
    let provider_key = "custom:https://api.example.invalid/v1";
    config
        .providers
        .models
        .insert(provider_key.to_string(), ModelProviderConfig::default());

    let api_key_path = format!("providers.models.{provider_key}.api-key");
    let base_url_path = format!("providers.models.{provider_key}.base-url");
    let model_path = format!("providers.models.{provider_key}.model");
    let temperature_path = format!("providers.models.{provider_key}.temperature");

    assert!(
        Config::prop_is_secret(&api_key_path),
        "url-like provider keys must still route secret metadata"
    );

    config.set_prop(&api_key_path, "sk-test-custom").unwrap();
    config
        .set_prop(&base_url_path, "https://api.example.invalid/v1")
        .unwrap();
    config.set_prop(&model_path, "local-large").unwrap();
    config.set_prop(&temperature_path, "0.2").unwrap();

    let provider = config
        .providers
        .models
        .get(provider_key)
        .expect("custom provider key should be preserved exactly");
    assert_eq!(provider.api_key.as_deref(), Some("sk-test-custom"));
    assert_eq!(
        provider.base_url.as_deref(),
        Some("https://api.example.invalid/v1")
    );
    assert_eq!(provider.model.as_deref(), Some("local-large"));
    assert_eq!(provider.temperature, Some(0.2));

    assert_eq!(config.get_prop(&api_key_path).unwrap(), "**** (encrypted)");
    assert_eq!(
        config.get_prop(&base_url_path).unwrap(),
        "https://api.example.invalid/v1"
    );

    config.save().await.unwrap();
    let raw_toml = tokio::fs::read_to_string(&config.config_path)
        .await
        .unwrap();
    assert!(
        raw_toml.contains(provider_key),
        "saved TOML should preserve the exact URL-like provider key"
    );
    assert!(
        !raw_toml.contains("sk-test-custom"),
        "saved TOML must not contain the plaintext custom provider API key"
    );

    let mut loaded: Config = toml::from_str::<crate::migration::V1Compat>(&raw_toml)
        .unwrap()
        .into_config();
    loaded.config_path = config.config_path.clone();
    loaded.workspace_dir = config.workspace_dir.clone();
    let store = crate::secrets::SecretStore::new(dir.path(), loaded.secrets.encrypt);
    loaded.decrypt_secrets(&store).unwrap();
    let loaded_provider = loaded
        .providers
        .models
        .get(provider_key)
        .expect("saved custom provider key should reload exactly");
    assert_eq!(
        loaded.providers.fallback.as_deref(),
        None,
        "property round-trip should not invent a fallback provider"
    );
    assert_eq!(loaded_provider.api_key.as_deref(), Some("sk-test-custom"));
    assert_eq!(
        loaded_provider.base_url.as_deref(),
        Some("https://api.example.invalid/v1")
    );
    assert_eq!(loaded_provider.model.as_deref(), Some("local-large"));
    assert_eq!(loaded_provider.temperature, Some(0.2));
}

#[test]
async fn enum_variants_callback_returns_values() {
    let mx = test_matrix_config();
    let fields = mx.prop_fields();
    let stream_field = fields
        .iter()
        .find(|f| f.name == "channels.matrix.stream-mode")
        .unwrap();
    let variants = (stream_field.enum_variants.unwrap())();
    assert!(variants.contains(&"off".to_string()));
    assert!(variants.contains(&"partial".to_string()));
    assert!(variants.contains(&"multi_message".to_string()));
}

#[test]
async fn map_key_sections_discovers_providers_models() {
    // The Configurable derive walks #[nested] HashMap<String, T> fields
    // and exposes them via map_key_sections(). Without this enumeration,
    // the dashboard has no way to know `providers.models.<name>` is an
    // addable shape — it only sees fields that already exist.
    let sections = Config::map_key_sections();
    let providers_models = sections
        .iter()
        .find(|s| s.path == "providers.models")
        .expect("providers.models must be discoverable as a map-keyed section");
    assert_eq!(providers_models.kind, crate::traits::MapKeyKind::Map);
    assert_eq!(providers_models.value_type, "ModelProviderConfig");

    // agents is also #[nested] HashMap on root Config.
    assert!(
        sections.iter().any(|s| s.path == "agents"),
        "agents map should be discoverable"
    );

    // mcp.servers is a Vec<McpServerConfig> with #[nested] — should
    // surface as a List-kind section so the dashboard's "+ Add MCP
    // server" affordance picks it up. Without this, dashboard users
    // hit a silent dead-end and have to hand-edit config.toml. Pinned
    // here so a regression that drops the #[nested] annotation or the
    // Configurable derive on McpServerConfig fails CI.
    let mcp_servers = sections
        .iter()
        .find(|s| s.path == "mcp.servers")
        .expect("mcp.servers must be discoverable as a list-shaped section");
    assert_eq!(mcp_servers.kind, crate::traits::MapKeyKind::List);
    assert_eq!(mcp_servers.value_type, "McpServerConfig");
}

#[test]
async fn create_map_key_inserts_default_mcp_server() {
    // Round-trip: `POST /api/config/map-key?path=mcp.servers&key=github`.
    // The new entry's `name` field is initialized to the supplied key
    // by the macro's List-kind insertion logic.
    let mut config = Config::default();
    assert!(config.mcp.servers.is_empty());

    let created = config
        .create_map_key("mcp.servers", "github")
        .expect("mcp.servers should accept new list entries");
    assert!(created, "first add should report created=true");
    assert_eq!(config.mcp.servers.len(), 1);
    assert_eq!(
        config.mcp.servers[0].name, "github",
        "new entry must carry the supplied key as its name field"
    );
}

#[test]
async fn create_map_key_inserts_default_provider() {
    // Round-trip: `+ Add anthropic provider` from the dashboard.
    let mut config = Config::default();
    assert!(!config.providers.models.contains_key("anthropic"));

    let created = config
        .create_map_key("providers.models", "anthropic")
        .expect("providers.models should accept new map keys");
    assert!(created, "first add should report created=true");
    assert!(config.providers.models.contains_key("anthropic"));

    // Idempotent: second add returns false, doesn't error.
    let again = config
        .create_map_key("providers.models", "anthropic")
        .expect("second add still resolves the section");
    assert!(!again, "duplicate add should report created=false");
}

#[test]
async fn create_map_key_rejects_unknown_section() {
    let mut config = Config::default();
    let err = config
        .create_map_key("not.a.real.section", "anything")
        .expect_err("unknown section path should error");
    assert!(err.contains("not.a.real.section"));
}

#[test]
async fn init_defaults_instantiates_none_sections() {
    let mut config = Config::default();
    assert!(config.channels.matrix.is_none());

    let initialized = config.init_defaults(Some("channels.matrix"));
    assert!(initialized.contains(&"channels.matrix"));
    assert!(config.channels.matrix.is_some());
}

#[test]
async fn deserialized_matrix_set_prop_round_trips_vec_string() {
    // Mirror the real-world daemon flow: config loaded from disk where
    // [channels.matrix] is present (possibly with all default fields),
    // then a PATCH from the dashboard hits set_prop.
    let toml_src = r#"
schema_version = 2

[channels.matrix]
enabled = false
homeserver = ""
access_token = ""
allowed_rooms = []
allowed_users = []
"#;
    let mut config: Config = toml::from_str(toml_src).expect("parse toml");
    assert!(
        config.channels.matrix.is_some(),
        "matrix must be Some after deserialize"
    );

    config
        .set_prop("channels.matrix.allowed-rooms", r#"["alice","bob"]"#)
        .expect("set_prop should succeed against deserialized matrix");
    assert_eq!(
        config.channels.matrix.as_ref().unwrap().allowed_rooms,
        vec!["alice".to_string(), "bob".to_string()],
    );
}

#[test]
async fn init_defaults_then_set_prop_round_trips_vec_string() {
    // Regression for #6175 Channels picker → form → save:
    // 1. init_defaults creates channels.matrix = Some(MatrixConfig::default())
    // 2. set_prop on channels.matrix.allowed-rooms must accept a JSON-array
    //    string (the shape coerce_for_set_prop emits for Vec<String>).
    // 3. get_prop reads it back.
    let mut config = Config::default();
    let initialized = config.init_defaults(Some("channels.matrix"));
    assert!(initialized.contains(&"channels.matrix"));
    assert!(config.channels.matrix.is_some());

    // prop_fields must surface the kebab path so the form can render it.
    let has_field = config
        .prop_fields()
        .iter()
        .any(|f| f.name == "channels.matrix.allowed-rooms");
    assert!(
        has_field,
        "channels.matrix.allowed-rooms must appear in prop_fields after init"
    );

    // set_prop with the JSON-array string the gateway PATCH path produces.
    config
        .set_prop("channels.matrix.allowed-rooms", r#"["alice","bob"]"#)
        .expect("set_prop should accept JSON-array string for Vec<String>");
    assert_eq!(
        config.channels.matrix.as_ref().unwrap().allowed_rooms,
        vec!["alice".to_string(), "bob".to_string()],
    );
}

#[test]
async fn mcp_servers_addable_via_create_map_key_and_per_entry_props() {
    // `mcp.servers` is a `Vec<McpServerConfig>` with `#[nested]`, so the
    // `Configurable` derive surfaces it as a List section (not an
    // ObjectArray prop) — operators add servers via
    // `POST /api/config/map-key?path=mcp.servers&key=<name>` and edit
    // each server's fields via per-prop GET/PUT.
    //
    // This replaces the prior model where the entire Vec round-tripped
    // through set_prop("mcp.servers", "<json-array>"). The List model
    // matches the rest of the schema (`providers.models`, `agents`,
    // etc.) and gives the dashboard a per-field editor instead of a
    // monolithic JSON blob.
    let mut config = Config::default();

    // The List section is discoverable.
    let sections = Config::map_key_sections();
    assert!(
        sections
            .iter()
            .any(|s| s.path == "mcp.servers" && s.kind == crate::traits::MapKeyKind::List),
        "mcp.servers should surface as a List section in map_key_sections()"
    );

    // create_map_key inserts a default-valued entry and seeds its
    // `name` field from the supplied key.
    config
        .create_map_key("mcp.servers", "fs")
        .expect("mcp.servers should accept new list entries via create_map_key");
    assert_eq!(config.mcp.servers.len(), 1);
    assert_eq!(config.mcp.servers[0].name, "fs");

    // Per-entry fields are mutated via standard set_prop on the inner
    // path (the same call site the per-prop PUT handler uses); the
    // McpServerConfig schema's `#[prefix = "mcp.servers"]` makes the
    // path resolution work without hand-table dispatch.
    // (Wider per-entry path routing through Vec<T> requires a
    // future generalization of route_hashmap_path-equivalent for
    // List sections; tracked as future work.)
}

#[test]
async fn ensure_default_mcp_servers_injects_agentmemory_when_backend_selects_it() {
    // Mirrors operant_core::config::ensure_default_mcp_servers (AppConfig
    // path) for the schema Config used by the runtime daemon/gateway.
    let mut config = Config::default();
    config.mcp.enabled = true;
    config.memory.backend = "agentmemory".to_string();

    config.ensure_default_mcp_servers();

    assert_eq!(config.mcp.servers.len(), 1);
    let server = &config.mcp.servers[0];
    assert_eq!(server.name, "agentmemory");
    assert_eq!(server.transport, McpTransport::Stdio);
    assert_eq!(server.command, "npx");
    assert_eq!(
        server.args,
        vec!["-y".to_string(), "@agentmemory/mcp".to_string()]
    );
    assert_eq!(
        server.env.get("AGENTMEMORY_URL").map(String::as_str),
        Some("http://localhost:3111")
    );

    // Idempotent: a second call must not duplicate the server.
    config.ensure_default_mcp_servers();
    assert_eq!(config.mcp.servers.len(), 1);
}

#[test]
async fn ensure_default_mcp_servers_noop_without_agentmemory_backend() {
    let mut config = Config::default();
    config.mcp.enabled = true;
    config.memory.backend = "sqlite".to_string();

    config.ensure_default_mcp_servers();
    assert!(config.mcp.servers.is_empty());

    // Disabled MCP also prevents injection even with the backend set.
    config.memory.backend = "agentmemory".to_string();
    config.mcp.enabled = false;
    config.ensure_default_mcp_servers();
    assert!(config.mcp.servers.is_empty());
}

#[test]
async fn ensure_default_mcp_servers_skips_when_user_configured() {
    let mut config = Config::default();
    config.mcp.enabled = true;
    config.memory.backend = "agentmemory".to_string();
    config.mcp.servers.push(McpServerConfig {
        name: "agentmemory".to_string(),
        transport: McpTransport::Stdio,
        url: None,
        command: "custom".to_string(),
        args: vec![],
        env: std::collections::HashMap::new(),
        headers: std::collections::HashMap::new(),
        tool_timeout_secs: None,
    });

    config.ensure_default_mcp_servers();

    // User's own agentmemory server is preserved untouched.
    assert_eq!(config.mcp.servers.len(), 1);
    assert_eq!(config.mcp.servers[0].command, "custom");
}

#[test]
async fn init_defaults_skips_already_set() {
    let mut config = Config::default();
    config.channels.matrix = Some(test_matrix_config());

    let initialized = config.init_defaults(Some("channels.matrix"));
    // Already set — should not re-initialize
    assert!(!initialized.contains(&"channels.matrix"));
    // Original value preserved
    assert_eq!(
        config.channels.matrix.as_ref().unwrap().homeserver,
        "https://m.org"
    );
}

#[test]
async fn nested_get_set_prop_traverses_config_tree() {
    let mut config = Config::default();
    config.channels.matrix = Some(test_matrix_config());

    // get_prop traverses Config → ChannelsConfig → MatrixConfig
    assert_eq!(
        config.get_prop("channels.matrix.homeserver").unwrap(),
        "https://m.org"
    );

    // set_prop traverses the same path
    config
        .set_prop("channels.matrix.homeserver", "https://new.org")
        .unwrap();
    assert_eq!(
        config.channels.matrix.as_ref().unwrap().homeserver,
        "https://new.org"
    );
}

#[test]
async fn hashmap_nested_encrypt_decrypt_traverses_values() {
    let dir = TempDir::new().unwrap();
    let store = crate::secrets::SecretStore::new(dir.path(), true);

    let mut config = Config::default();
    config.agents.insert(
        "test-agent".into(),
        DelegateAgentConfig {
            api_key: Some("secret-key".into()),
            ..Default::default()
        },
    );

    config.encrypt_secrets(&store).unwrap();
    let encrypted_key = config.agents["test-agent"].api_key.as_ref().unwrap();
    assert!(crate::secrets::SecretStore::is_encrypted(encrypted_key));

    config.decrypt_secrets(&store).unwrap();
    assert_eq!(
        config.agents["test-agent"].api_key.as_deref(),
        Some("secret-key")
    );
}

#[test]
async fn vec_secret_encrypt_decrypt_traverses_elements() {
    let dir = TempDir::new().unwrap();
    let store = crate::secrets::SecretStore::new(dir.path(), true);

    let mut config = Config::default();
    config.gateway.paired_tokens = vec!["token-a".into(), "token-b".into()];

    config.encrypt_secrets(&store).unwrap();
    for token in &config.gateway.paired_tokens {
        assert!(crate::secrets::SecretStore::is_encrypted(token));
    }

    config.decrypt_secrets(&store).unwrap();
    assert_eq!(config.gateway.paired_tokens, vec!["token-a", "token-b"]);
}

/// Walk every property on a default Config: get_prop must succeed,
/// and set_prop must round-trip for non-secret, non-enum scalar fields.
#[test]
async fn every_prop_is_gettable_and_settable() {
    let mut config = Config::default();
    // Initialize all Option<T> sections so their fields are reachable
    config.init_defaults(None);

    let fields = config.prop_fields();
    assert!(
        fields.len() > 50,
        "Expected 50+ props, got {} — macro may be skipping fields",
        fields.len()
    );

    for field in &fields {
        // get_prop must not panic or error
        let get_result = config.get_prop(&field.name);
        assert!(
            get_result.is_ok(),
            "get_prop failed for '{}': {}",
            field.name,
            get_result.unwrap_err()
        );

        // set_prop: round-trip the display value back through set_prop.
        // Skip secrets (masked), enums (need valid variant), and <unset> Options.
        if field.is_secret || field.is_enum() || field.display_value == "<unset>" {
            continue;
        }

        let set_result = config.set_prop(&field.name, &field.display_value);
        assert!(
            set_result.is_ok(),
            "set_prop failed for '{}' with value '{}': {}",
            field.name,
            field.display_value,
            set_result.unwrap_err()
        );

        // Value should survive the round-trip
        let after = config.get_prop(&field.name).unwrap();
        assert_eq!(
            after, field.display_value,
            "round-trip mismatch for '{}': set '{}', got '{}'",
            field.name, field.display_value, after
        );
    }
}

/// Every enum field must have a working enum_variants callback, and
/// set_prop must accept each variant it advertises.
#[test]
async fn every_enum_variant_is_settable() {
    let mut config = Config::default();
    config.init_defaults(None);

    for field in config.prop_fields() {
        if !field.is_enum() {
            continue;
        }
        let get_variants = field
            .enum_variants
            .unwrap_or_else(|| panic!("enum field '{}' has no enum_variants callback", field.name));
        let variants = get_variants();
        assert!(
            !variants.is_empty(),
            "enum field '{}' returned no variants",
            field.name
        );

        for variant in &variants {
            let result = config.set_prop(&field.name, variant);
            assert!(
                result.is_ok(),
                "set_prop('{}', '{}') failed: {}",
                field.name,
                variant,
                result.unwrap_err()
            );
        }
    }
}

#[test]
async fn backfill_enabled_activates_channel_without_explicit_enabled() {
    let toml = r#"
[channels.matrix]
homeserver = "https://matrix.org"
access_token = "tok"
allowed_rooms = ["!r:m"]
allowed_users = ["@u:m"]
"#;
    let mut config: Config = toml::from_str(toml).unwrap();
    assert!(!config.channels.matrix.as_ref().unwrap().enabled);

    config.channels.backfill_enabled(toml);
    assert!(config.channels.matrix.as_ref().unwrap().enabled);
}

#[test]
async fn backfill_enabled_respects_explicit_false() {
    let toml = r#"
[channels.matrix]
homeserver = "https://matrix.org"
access_token = "tok"
allowed_rooms = ["!r:m"]
allowed_users = ["@u:m"]
enabled = false
"#;
    let mut config: Config = toml::from_str(toml).unwrap();
    config.channels.backfill_enabled(toml);
    assert!(
        !config.channels.matrix.as_ref().unwrap().enabled,
        "explicit enabled=false must not be overwritten"
    );
}

#[test]
async fn backfill_enabled_no_op_when_section_absent() {
    let toml = r#"
api_key = "sk-test"
"#;
    let mut config: Config = toml::from_str(toml).unwrap();
    config.channels.backfill_enabled(toml);
    assert!(config.channels.telegram.is_none());
}

#[test]
async fn backfill_enabled_works_with_toml_comments() {
    let toml = r#"
# My matrix setup
[channels.matrix]
homeserver = "https://matrix.org"  # production server
access_token = "tok"
allowed_rooms = ["!r:m"]
allowed_users = ["@u:m"]
# enabled intentionally omitted
"#;
    let mut config: Config = toml::from_str(toml).unwrap();
    assert!(!config.channels.matrix.as_ref().unwrap().enabled);

    config.channels.backfill_enabled(toml);
    assert!(
        config.channels.matrix.as_ref().unwrap().enabled,
        "backfill should activate channel even when config has comments"
    );
}

#[test]
async fn channel_approval_timeout_secs_defaults_to_300() {
    // Omitting approval_timeout_secs from each config should deserialize to 300
    let discord: DiscordConfig =
        serde_json::from_str(r#"{"bot_token":"tok","enabled":true}"#).unwrap();
    assert_eq!(discord.approval_timeout_secs, 300);

    let slack: SlackConfig = serde_json::from_str(r#"{"bot_token":"tok","enabled":true}"#).unwrap();
    assert_eq!(slack.approval_timeout_secs, 300);

    let signal: SignalConfig =
        serde_json::from_str(r#"{"http_url":"http://localhost","account":"+1","enabled":true}"#)
            .unwrap();
    assert_eq!(signal.approval_timeout_secs, 300);

    let matrix: MatrixConfig = serde_json::from_str(
            r#"{"homeserver":"https://matrix.org","access_token":"tok","enabled":true,"allowed_users":[]}"#,
        )
        .unwrap();
    assert_eq!(matrix.approval_timeout_secs, 300);

    let whatsapp: WhatsAppConfig = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
    assert_eq!(whatsapp.approval_timeout_secs, 300);
}

#[test]
async fn channel_approval_timeout_secs_explicit_override() {
    let discord: DiscordConfig =
        serde_json::from_str(r#"{"bot_token":"tok","enabled":true,"approval_timeout_secs":60}"#)
            .unwrap();
    assert_eq!(discord.approval_timeout_secs, 60);

    let slack: SlackConfig =
        serde_json::from_str(r#"{"bot_token":"tok","enabled":true,"approval_timeout_secs":120}"#)
            .unwrap();
    assert_eq!(slack.approval_timeout_secs, 120);

    let signal: SignalConfig = serde_json::from_str(
            r#"{"http_url":"http://localhost","account":"+1","enabled":true,"approval_timeout_secs":90}"#,
        )
        .unwrap();
    assert_eq!(signal.approval_timeout_secs, 90);

    let matrix: MatrixConfig = serde_json::from_str(
            r#"{"homeserver":"https://matrix.org","access_token":"tok","enabled":true,"allowed_users":[],"approval_timeout_secs":45}"#,
        )
        .unwrap();
    assert_eq!(matrix.approval_timeout_secs, 45);

    let whatsapp: WhatsAppConfig =
        serde_json::from_str(r#"{"enabled":true,"approval_timeout_secs":180}"#).unwrap();
    assert_eq!(whatsapp.approval_timeout_secs, 180);
}

// ── combined_pricing: per-provider + top-level merge (#6251) ──────

fn pricing(input: f64, output: f64) -> ModelPricing {
    ModelPricing { input, output }
}

fn config_with_provider(
    provider_id: &str,
    model: Option<&str>,
    per_provider_pricing: Option<ModelPricing>,
) -> Config {
    let mut config = Config::default();
    // Start clean: Config::default() seeds CostConfig::default() which calls
    // get_default_pricing(); for these tests we want a deterministic baseline.
    config.cost.prices.clear();
    config.providers.models.insert(
        provider_id.to_string(),
        ModelProviderConfig {
            model: model.map(ToString::to_string),
            pricing: per_provider_pricing,
            ..ModelProviderConfig::default()
        },
    );
    config
}

#[test]
async fn combined_pricing_passes_through_when_no_per_provider_pricing() {
    let mut config = config_with_provider("openai", Some("gpt-4o"), None);
    config
        .cost
        .prices
        .insert("openai/gpt-4o".into(), pricing(2.5, 10.0));

    let combined = config.combined_pricing();
    assert_eq!(combined.len(), 1);
    let entry = combined.get("openai/gpt-4o").expect("top-level entry");
    assert_eq!(entry.input, 2.5);
    assert_eq!(entry.output, 10.0);
}

#[test]
async fn combined_pricing_merges_per_provider_into_provider_slash_model_key() {
    let config = config_with_provider(
        "anthropic",
        Some("claude-sonnet-4-5"),
        Some(pricing(3.0, 15.0)),
    );

    let combined = config.combined_pricing();
    let entry = combined
        .get("anthropic/claude-sonnet-4-5")
        .expect("per-provider pricing keyed as <provider_id>/<model>");
    assert_eq!(entry.input, 3.0);
    assert_eq!(entry.output, 15.0);
}

#[test]
async fn combined_pricing_top_level_wins_on_conflict() {
    let mut config = config_with_provider(
        "anthropic",
        Some("claude-sonnet-4-5"),
        Some(pricing(3.0, 15.0)),
    );
    // Operator pinned a different rate at the top level — must survive.
    config
        .cost
        .prices
        .insert("anthropic/claude-sonnet-4-5".into(), pricing(2.0, 8.0));

    let combined = config.combined_pricing();
    let entry = combined.get("anthropic/claude-sonnet-4-5").unwrap();
    assert_eq!(
        entry.input, 2.0,
        "top-level [cost.prices] override must not be silently shadowed by per-provider pricing"
    );
    assert_eq!(entry.output, 8.0);
}

#[test]
async fn combined_pricing_skips_provider_with_no_model() {
    // Provider has pricing but no `model` set — we cannot synthesize the
    // <provider_id>/<model> key, so the entry must be skipped (not crash,
    // not produce a malformed key).
    let config = config_with_provider("openrouter", None, Some(pricing(1.0, 2.0)));

    let combined = config.combined_pricing();
    assert!(
        combined.is_empty(),
        "per-provider pricing without `model` is silently skipped, got {combined:?}"
    );
}

#[test]
async fn combined_pricing_skips_provider_with_empty_model() {
    let config = config_with_provider("openrouter", Some(""), Some(pricing(1.0, 2.0)));

    let combined = config.combined_pricing();
    assert!(
        combined.is_empty(),
        "empty model string must be treated the same as missing, got {combined:?}"
    );
}

// ── set_prop round-trip for `pricing` (#6357 review) ──────────────
//
// Dashboard / JSON-patch callers send `pricing` as a JSON object
// (`{"input": 1.0, "output": 2.5}`). Before #6357 review, `pricing` was
// classified as `PropKind::String`, which meant `parse_prop_value`
// inserted the JSON text as a TOML string and serde failed to
// deserialize it back into `Option<ModelPricing>`. The fix classifies
// `ModelPricing` as `PropKind::Object` and routes through `json_to_toml`
// so the value lands as a typed inline table. These tests pin that
// contract end-to-end through `Config::set_prop` and the wire-form
// coercion at `coerce_for_set_prop`.

#[test]
async fn set_prop_round_trips_per_provider_pricing_object() {
    let mut config = config_with_provider("openai", Some("gpt-4o"), None);

    // Caller hits `Config::set_prop` directly with the JSON-stringified
    // object that the dashboard / CLI hand off after type coercion.
    config
        .set_prop(
            "providers.models.openai.pricing",
            r#"{"input": 1.5, "output": 6.0}"#,
        )
        .expect("set_prop must accept a JSON object for pricing");

    // Round-trip 1: typed access on the struct must reflect the write.
    let pricing = config
        .providers
        .models
        .get("openai")
        .and_then(|m| m.pricing.clone())
        .expect("pricing must round-trip back into a typed ModelPricing, not a string");
    assert_eq!(pricing.input, 1.5);
    assert_eq!(pricing.output, 6.0);

    // Round-trip 2: the merged pricing map (the runtime/channel cost
    // contexts consume this) must show the new values keyed under
    // `<provider_id>/<model>`.
    let combined = config.combined_pricing();
    let entry = combined
        .get("openai/gpt-4o")
        .expect("set_prop write must surface in combined_pricing");
    assert_eq!(entry.input, 1.5);
    assert_eq!(entry.output, 6.0);
}

#[test]
async fn set_prop_pricing_rejects_non_object_string_payload() {
    // Sanity: a quoted JSON string (which is what the buggy
    // `PropKind::String` path would have silently accepted by writing
    // the raw text into a TOML string field) must NOT round-trip into
    // a typed `ModelPricing`. The fix is `PropKind::Object`, which
    // requires a JSON object; anything else fails set_prop and leaves
    // the field unchanged. We assert the failure is observable AND
    // that no garbage state was written.
    let mut config = config_with_provider("openai", Some("gpt-4o"), None);

    let result = config.set_prop(
        "providers.models.openai.pricing",
        "\"{\\\"input\\\":1.5,\\\"output\\\":6.0}\"",
    );
    assert!(
        result.is_err(),
        "a JSON string in place of an object must be rejected by set_prop"
    );
    // Pricing must still be `None` — no partial / garbage write.
    let pricing_after = config
        .providers
        .models
        .get("openai")
        .and_then(|m| m.pricing.clone());
    assert!(
        pricing_after.is_none(),
        "rejected set_prop must not mutate pricing, got {pricing_after:?}"
    );
}

#[test]
async fn coerce_for_set_prop_object_round_trips_pricing_payload() {
    use crate::traits::PropKind;
    use crate::typed_value::coerce_for_set_prop;

    // Dashboard / CLI sends a real JSON object; the coercion layer
    // must hand it back as a JSON-stringified object (the wire form
    // that `parse_prop_value`'s `Object` arm consumes).
    let coerced = coerce_for_set_prop(
        &serde_json::json!({"input": 1.5, "output": 6.0}),
        Some(PropKind::Object),
    )
    .expect("JSON object payload must coerce successfully for an Object field");
    let parsed: serde_json::Value =
        serde_json::from_str(&coerced).expect("coerced output must remain valid JSON");
    assert_eq!(parsed, serde_json::json!({"input": 1.5, "output": 6.0}));
}

#[test]
async fn coerce_for_set_prop_object_rejects_non_object() {
    use crate::traits::PropKind;
    use crate::typed_value::coerce_for_set_prop;

    let err = coerce_for_set_prop(
        &serde_json::json!("{\"input\":1.5}"),
        Some(PropKind::Object),
    )
    .expect_err("a JSON string is not a JSON object — must be rejected");
    assert!(
        err.message.contains("object"),
        "rejection message must name the object requirement, got: {}",
        err.message
    );
}
