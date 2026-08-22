//! `helpers` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::{Context, Result};
use directories::UserDirs;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::*;

pub(crate) fn default_workspaces_dir() -> String {
    default_path_under_config_dir("workspaces")
}

/// Used by `#[serde(skip_serializing_if)]` on plain `bool` fields to omit
/// them from TOML output when they carry their struct-level default (`false`).
/// Keeps fresh provider entries clean — a default-constructed
/// `ModelProviderConfig` for one provider family shouldn't write flag fields
/// that only apply to a different family.
pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn default_delegate_timeout_secs() -> u64 {
    DEFAULT_DELEGATE_TIMEOUT_SECS
}

pub(crate) fn default_delegate_agentic_timeout_secs() -> u64 {
    DEFAULT_DELEGATE_AGENTIC_TIMEOUT_SECS
}

pub(crate) const DEFAULT_SWARM_TIMEOUT_SECS: u64 = 300;

pub(crate) fn default_swarm_timeout_secs() -> u64 {
    DEFAULT_SWARM_TIMEOUT_SECS
}

/// Default delegate tool timeout for non-agentic calls: 120 seconds.
pub const DEFAULT_DELEGATE_TIMEOUT_SECS: u64 = 120;

/// Default delegate tool timeout for agentic runs: 300 seconds.
pub const DEFAULT_DELEGATE_AGENTIC_TIMEOUT_SECS: u64 = 300;

/// Validate that a temperature value is within the allowed range.
pub fn validate_temperature(value: f64) -> std::result::Result<f64, String> {
    if TEMPERATURE_RANGE.contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "temperature {value} is out of range (expected {}..={})",
            TEMPERATURE_RANGE.start(),
            TEMPERATURE_RANGE.end()
        ))
    }
}

pub(crate) fn default_max_depth() -> u32 {
    3
}

pub(crate) fn default_max_tool_iterations() -> usize {
    10
}

pub(crate) fn default_baud_rate() -> u32 {
    115_200
}

// ── Transcription ────────────────────────────────────────────────

pub(crate) fn default_transcription_api_url() -> String {
    "https://api.groq.com/openai/v1/audio/transcriptions".into()
}

pub(crate) fn default_transcription_model() -> String {
    "whisper-large-v3-turbo".into()
}

pub(crate) fn default_transcription_max_duration_secs() -> u64 {
    120
}

pub(crate) fn default_transcription_provider() -> String {
    "groq".into()
}

pub(crate) fn default_openai_stt_model() -> String {
    "whisper-1".into()
}

pub(crate) fn default_deepgram_stt_model() -> String {
    "nova-2".into()
}

pub(crate) fn default_google_stt_language_code() -> String {
    "en-US".into()
}

pub(crate) fn default_deferred_loading() -> bool {
    true
}

pub(crate) fn default_vi_strictness() -> String {
    "strict".to_owned()
}

pub(crate) fn default_max_nodes() -> usize {
    16
}

// ── TTS (Text-to-Speech) ─────────────────────────────────────────

pub(crate) fn default_tts_provider() -> String {
    "openai".into()
}

pub(crate) fn default_tts_voice() -> String {
    "alloy".into()
}

pub(crate) fn default_tts_format() -> String {
    "mp3".into()
}

pub(crate) fn default_tts_max_text_length() -> usize {
    4096
}

pub(crate) fn default_openai_tts_model() -> String {
    "tts-1".into()
}

pub(crate) fn default_openai_tts_speed() -> f64 {
    1.0
}

pub(crate) fn default_elevenlabs_model_id() -> String {
    "eleven_monolingual_v1".into()
}

pub(crate) fn default_elevenlabs_stability() -> f64 {
    0.5
}

pub(crate) fn default_elevenlabs_similarity_boost() -> f64 {
    0.5
}

pub(crate) fn default_google_tts_language_code() -> String {
    "en-US".into()
}

pub(crate) fn default_edge_tts_binary_path() -> String {
    "edge-tts".into()
}

pub(crate) fn default_piper_tts_api_url() -> String {
    "http://127.0.0.1:5000/v1/audio/speech".into()
}

pub(crate) fn default_local_whisper_max_audio_bytes() -> usize {
    25 * 1024 * 1024
}

pub(crate) fn default_local_whisper_timeout_secs() -> u64 {
    300
}

pub(crate) fn default_inject_system_prompt() -> bool {
    true
}

pub(crate) fn default_max_tool_result_chars() -> usize {
    50_000
}

pub(crate) fn default_keep_tool_context_turns() -> usize {
    2
}

pub(crate) fn default_agent_max_tool_iterations() -> usize {
    10
}

pub(crate) fn default_memory_nudge_interval() -> usize {
    10
}

pub(crate) fn default_creation_nudge_interval() -> usize {
    10
}

pub(crate) fn default_agent_max_history_messages() -> usize {
    50
}

pub(crate) fn default_agent_max_context_tokens() -> usize {
    32_000
}

pub(crate) fn default_agent_tool_dispatcher() -> String {
    "auto".into()
}

pub(crate) fn default_max_system_prompt_chars() -> usize {
    0
}

pub(crate) fn default_loop_detection_enabled() -> bool {
    true
}

pub(crate) fn default_loop_detection_window_size() -> usize {
    20
}

pub(crate) fn default_loop_detection_max_repeats() -> usize {
    3
}

pub(crate) fn default_skill_improvement_cooldown() -> u64 {
    3600
}

pub(crate) fn default_pipeline_max_steps() -> usize {
    20
}

pub(crate) fn default_multimodal_max_images() -> usize {
    4
}

pub(crate) fn default_multimodal_max_image_size_mb() -> usize {
    5
}

pub(crate) fn default_identity_format() -> String {
    "openclaw".into()
}

pub(crate) fn default_cost_enforcement_mode() -> String {
    "warn".to_string()
}

pub(crate) fn default_reserve_percent() -> u8 {
    10
}

pub(crate) fn default_daily_limit() -> f64 {
    10.0
}

pub(crate) fn default_monthly_limit() -> f64 {
    100.0
}

pub(crate) fn default_warn_percent() -> u8 {
    80
}

pub(crate) fn default_cost_enabled() -> bool {
    true
}

pub(crate) fn default_peripheral_transport() -> String {
    "serial".into()
}

pub(crate) fn default_peripheral_baud() -> u32 {
    115_200
}

pub(crate) fn default_gateway_port() -> u16 {
    42617
}

pub(crate) fn default_gateway_host() -> String {
    "127.0.0.1".into()
}

pub(crate) fn default_pair_rate_limit() -> u32 {
    10
}

pub(crate) fn default_webhook_rate_limit() -> u32 {
    60
}

pub(crate) fn default_idempotency_ttl_secs() -> u64 {
    300
}

pub(crate) fn default_gateway_rate_limit_max_keys() -> usize {
    10_000
}

pub(crate) fn default_gateway_idempotency_max_keys() -> usize {
    10_000
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_false() -> bool {
    false
}

pub(crate) fn default_pairing_code_length() -> usize {
    8
}

pub(crate) fn default_pairing_ttl() -> u64 {
    3600
}

pub(crate) fn default_max_pending_codes() -> usize {
    3
}

pub(crate) fn default_max_failed_attempts() -> u32 {
    5
}

pub(crate) fn default_pairing_lockout_secs() -> u64 {
    300
}

pub(crate) fn default_node_transport_enabled() -> bool {
    true
}

pub(crate) fn default_max_request_age() -> i64 {
    300
}

pub(crate) fn default_require_https() -> bool {
    true
}

pub(crate) fn default_connection_pool_size() -> usize {
    4
}

pub(crate) fn default_entity_id() -> String {
    "default".into()
}

pub(crate) fn default_ms365_auth_flow() -> String {
    "client_credentials".to_string()
}

pub(crate) fn default_ms365_scopes() -> Vec<String> {
    vec!["https://graph.microsoft.com/.default".to_string()]
}

pub(crate) fn default_browser_computer_use_endpoint() -> String {
    "http://127.0.0.1:8787/v1/actions".into()
}

pub(crate) fn default_browser_computer_use_timeout_ms() -> u64 {
    15_000
}

pub(crate) fn default_browser_allowed_domains() -> Vec<String> {
    vec!["*".into()]
}

pub(crate) fn default_browser_backend() -> String {
    "agent_browser".into()
}

pub(crate) fn default_browser_webdriver_url() -> String {
    "http://127.0.0.1:9515".into()
}

pub(crate) fn default_http_max_response_size() -> usize {
    1_000_000 // 1MB
}

pub(crate) fn default_http_timeout_secs() -> u64 {
    30
}

pub(crate) fn default_firecrawl_api_key_env() -> String {
    "FIRECRAWL_API_KEY".into()
}

pub(crate) fn default_firecrawl_api_url() -> String {
    "https://api.firecrawl.dev/v1".into()
}

pub(crate) fn default_web_fetch_max_response_size() -> usize {
    500_000 // 500KB
}

pub(crate) fn default_web_fetch_timeout_secs() -> u64 {
    30
}

pub(crate) fn default_web_fetch_allowed_domains() -> Vec<String> {
    vec!["*".into()]
}

pub(crate) fn default_link_enricher_max_links() -> usize {
    3
}

pub(crate) fn default_link_enricher_timeout_secs() -> u64 {
    10
}

pub(crate) fn default_text_browser_timeout_secs() -> u64 {
    30
}

pub(crate) fn default_shell_tool_timeout_secs() -> u64 {
    60
}

pub(crate) fn default_web_search_provider() -> String {
    "duckduckgo".into()
}

pub(crate) fn default_web_search_max_results() -> usize {
    5
}

pub(crate) fn default_web_search_timeout_secs() -> u64 {
    15
}

pub(crate) fn default_project_intel_language() -> String {
    "en".into()
}

pub(crate) fn default_project_intel_report_dir() -> String {
    default_path_under_config_dir("project-reports")
}

pub(crate) fn default_project_intel_risk_sensitivity() -> String {
    "medium".into()
}

pub(crate) fn default_backup_max_keep() -> usize {
    10
}

pub(crate) fn default_backup_include_dirs() -> Vec<String> {
    vec![
        "config".into(),
        "memory".into(),
        "audit".into(),
        "knowledge".into(),
    ]
}

pub(crate) fn default_backup_destination_dir() -> String {
    "state/backups".into()
}

pub(crate) fn default_retention_days() -> u64 {
    90
}

// ── Google Workspace ─────────────────────────────────────────────

/// Built-in default service allowlist for the `google_workspace` tool.
///
/// Applied when `allowed_services` is empty. Defined here (not in the tool layer)
/// so that config validation can cross-check `allowed_operations` entries against
/// the effective service set in all cases, including when the operator relies on
/// the default.
pub const DEFAULT_GWS_SERVICES: &[&str] = &[
    "drive",
    "sheets",
    "gmail",
    "calendar",
    "docs",
    "slides",
    "tasks",
    "people",
    "chat",
    "classroom",
    "forms",
    "keep",
    "meet",
    "events",
];

/// Google Workspace CLI (`gws`) tool configuration (`[google_workspace]` section).
///
/// ## Defaults
/// - `enabled`: `false` (tool is not registered unless explicitly opted-in).
/// - `allowed_services`: empty vector, which grants access to the full default
///   service set: `drive`, `sheets`, `gmail`, `calendar`, `docs`, `slides`,
///   `tasks`, `people`, `chat`, `classroom`, `forms`, `keep`, `meet`, `events`.
/// - `credentials_path`: `None` (uses default `gws` credential discovery).
/// - `default_account`: `None` (uses the `gws` active account).
/// - `rate_limit_per_minute`: `60`.
/// - `timeout_secs`: `30`.
/// - `audit_log`: `false`.
/// - `credentials_path`: `None` (uses default `gws` credential discovery).
/// - `default_account`: `None` (uses the `gws` active account).
/// - `rate_limit_per_minute`: `60`.
/// - `timeout_secs`: `30`.
/// - `audit_log`: `false`.
///
/// ## Compatibility
/// Configs that omit the `[google_workspace]` section entirely are treated as
/// `GoogleWorkspaceConfig::default()` (disabled, all defaults allowed). Adding
/// the section is purely opt-in and does not affect other config sections.
///
/// ## Rollback / Migration
/// To revert, remove the `[google_workspace]` section from the config file (or
/// set `enabled = false`). No data migration is required; the tool simply stops
/// being registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct GoogleWorkspaceAllowedOperation {
    /// Google Workspace service ID (for example `gmail` or `drive`).
    pub service: String,
    /// Top-level resource name for the service (for example `users` for Gmail or `files` for Drive).
    pub resource: String,
    /// Optional sub-resource for 4-segment gws commands
    /// (for example `messages` or `drafts` under `gmail users`).
    /// When present, the entry only matches calls that include this exact sub_resource.
    /// When absent, the entry only matches calls with no sub_resource.
    #[serde(default)]
    pub sub_resource: Option<String>,
    /// Allowed methods for the service/resource/sub_resource combination.
    #[serde(default)]
    pub methods: Vec<String>,
}

pub(crate) fn default_gws_rate_limit() -> u32 {
    60
}

pub(crate) fn default_gws_timeout_secs() -> u64 {
    30
}

pub(crate) fn default_knowledge_db_path() -> String {
    default_path_under_config_dir("knowledge.db")
}

pub(crate) fn default_knowledge_max_nodes() -> usize {
    100_000
}

pub(crate) fn default_linkedin_api_version() -> String {
    "202602".to_string()
}

pub(crate) fn default_signature_mode() -> String {
    "disabled".to_string()
}

pub(crate) fn default_plugins_dir() -> String {
    default_path_under_config_dir("plugins")
}

pub(crate) fn default_max_plugins() -> usize {
    50
}

pub(crate) fn default_image_providers() -> Vec<String> {
    vec![
        "stability".into(),
        "imagen".into(),
        "dalle".into(),
        "flux".into(),
    ]
}

pub(crate) fn default_card_accent_color() -> String {
    "#0A66C2".into()
}

pub(crate) fn default_image_temp_dir() -> String {
    "linkedin/images".into()
}

pub(crate) fn default_stability_api_key_env() -> String {
    "STABILITY_API_KEY".into()
}

pub(crate) fn default_stability_model() -> String {
    "stable-diffusion-xl-1024-v1-0".into()
}

pub(crate) fn default_imagen_api_key_env() -> String {
    "GOOGLE_VERTEX_API_KEY".into()
}

pub(crate) fn default_imagen_project_id_env() -> String {
    "GOOGLE_CLOUD_PROJECT".into()
}

pub(crate) fn default_imagen_region() -> String {
    "us-central1".into()
}

pub(crate) fn default_dalle_api_key_env() -> String {
    "OPENAI_API_KEY".into()
}

pub(crate) fn default_dalle_model() -> String {
    "dall-e-3".into()
}

pub(crate) fn default_dalle_size() -> String {
    "1024x1024".into()
}

pub(crate) fn default_flux_api_key_env() -> String {
    "FAL_API_KEY".into()
}

pub(crate) fn default_flux_model() -> String {
    "fal-ai/flux/schnell".into()
}

pub(crate) fn default_image_gen_model() -> String {
    "fal-ai/flux/schnell".into()
}

pub(crate) fn default_image_gen_api_key_env() -> String {
    "FAL_API_KEY".into()
}

pub(crate) fn default_claude_code_timeout_secs() -> u64 {
    600
}

pub(crate) fn default_claude_code_allowed_tools() -> Vec<String> {
    vec!["Read".into(), "Edit".into(), "Bash".into(), "Write".into()]
}

pub(crate) fn default_claude_code_max_output_bytes() -> usize {
    2_097_152
}

pub(crate) fn default_claude_code_runner_tmux_prefix() -> String {
    "zc-claude-".into()
}

pub(crate) fn default_claude_code_runner_session_ttl() -> u64 {
    3600
}

pub(crate) fn default_codex_cli_timeout_secs() -> u64 {
    600
}

pub(crate) fn default_codex_cli_max_output_bytes() -> usize {
    2_097_152
}

pub(crate) fn default_gemini_cli_timeout_secs() -> u64 {
    600
}

pub(crate) fn default_gemini_cli_max_output_bytes() -> usize {
    2_097_152
}

pub(crate) fn default_opencode_cli_timeout_secs() -> u64 {
    600
}

pub(crate) fn default_opencode_cli_max_output_bytes() -> usize {
    2_097_152
}

pub(crate) fn normalize_comma_values(values: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    for value in values {
        for part in value.split(',') {
            let normalized = part.trim();
            if normalized.is_empty() {
                continue;
            }
            output.push(normalized.to_string());
        }
    }
    output.sort_unstable();
    output.dedup();
    output
}

pub(crate) fn default_storage_schema() -> String {
    "public".into()
}

pub(crate) fn default_storage_table() -> String {
    "memories".into()
}

pub(crate) fn default_qdrant_collection() -> String {
    "operant_memories".into()
}

pub(crate) fn default_retrieval_stages() -> Vec<String> {
    vec!["cache".into(), "fts".into(), "vector".into()]
}

pub(crate) fn default_rerank_threshold() -> usize {
    5
}

pub(crate) fn default_fts_early_return_score() -> f64 {
    0.85
}

pub(crate) fn default_namespace() -> String {
    "default".into()
}

pub(crate) fn default_conflict_threshold() -> f64 {
    0.85
}

pub(crate) fn default_audit_retention_days() -> u32 {
    30
}

pub(crate) fn default_pgvector_dimensions() -> usize {
    1536
}

pub(crate) fn default_embedding_provider() -> String {
    "none".into()
}

pub(crate) fn default_hygiene_enabled() -> bool {
    true
}

pub(crate) fn default_archive_after_days() -> u32 {
    7
}

pub(crate) fn default_purge_after_days() -> u32 {
    30
}

pub(crate) fn default_conversation_retention_days() -> u32 {
    30
}

pub(crate) fn default_embedding_model() -> String {
    // Empty by default: no external embedding model is ever assumed, so
    // operant runs with zero embedding dependencies out of the box (hermes
    // parity — hermes has no embedding provider at all; semantic memory is
    // server/plugin-side). Users opt into one explicitly.
    String::new()
}

pub(crate) fn default_embedding_dims() -> usize {
    0
}

pub(crate) fn default_vector_weight() -> f64 {
    0.7
}

pub(crate) fn default_keyword_weight() -> f64 {
    0.3
}

pub(crate) fn default_min_relevance_score() -> f64 {
    0.4
}

pub(crate) fn default_cache_size() -> usize {
    10_000
}

pub(crate) fn default_chunk_size() -> usize {
    512
}

pub(crate) fn default_response_cache_ttl() -> u32 {
    60
}

pub(crate) fn default_response_cache_max() -> usize {
    5_000
}

pub(crate) fn default_response_cache_hot_entries() -> usize {
    256
}

pub(crate) fn default_runtime_trace_mode() -> String {
    "none".to_string()
}

pub(crate) fn default_runtime_trace_path() -> String {
    "state/runtime-trace.jsonl".to_string()
}

pub(crate) fn default_runtime_trace_max_entries() -> usize {
    200
}

pub(crate) fn default_max_args_bytes() -> u64 {
    4096
}

pub(crate) fn default_shell_timeout_secs() -> u64 {
    60
}

pub(crate) fn default_auto_approve() -> Vec<String> {
    vec![
        "file_read".into(),
        "memory_recall".into(),
        "web_search_tool".into(),
        "web_fetch".into(),
        "calculator".into(),
        "glob_search".into(),
        "content_search".into(),
        "image_info".into(),
        "weather".into(),
        "browser".into(),
        "browser_open".into(),
    ]
}

pub(crate) fn default_always_ask() -> Vec<String> {
    vec![]
}

pub(crate) fn default_runtime_kind() -> String {
    "native".into()
}

pub(crate) fn default_docker_image() -> String {
    "alpine:3.20".into()
}

pub(crate) fn default_docker_network() -> String {
    "none".into()
}

pub(crate) fn default_docker_memory_limit_mb() -> Option<u64> {
    Some(512)
}

pub(crate) fn default_docker_cpu_limit() -> Option<f64> {
    Some(1.0)
}

pub(crate) fn default_provider_retries() -> u32 {
    2
}

pub(crate) fn default_provider_backoff_ms() -> u64 {
    500
}

pub(crate) fn default_channel_backoff_secs() -> u64 {
    2
}

pub(crate) fn default_channel_backoff_max_secs() -> u64 {
    60
}

pub(crate) fn default_scheduler_poll_secs() -> u64 {
    15
}

pub(crate) fn default_scheduler_retries() -> u32 {
    2
}

pub(crate) fn default_scheduler_enabled() -> bool {
    true
}

pub(crate) fn default_scheduler_max_tasks() -> usize {
    64
}

pub(crate) fn default_scheduler_max_concurrent() -> usize {
    4
}

pub(crate) fn default_heartbeat_interval() -> u32 {
    30
}

pub(crate) fn default_two_phase() -> bool {
    true
}

pub(crate) fn default_heartbeat_min_interval() -> u32 {
    5
}

pub(crate) fn default_heartbeat_max_interval() -> u32 {
    120
}

pub(crate) fn default_heartbeat_max_run_history() -> u32 {
    100
}

pub(crate) fn default_heartbeat_task_timeout() -> u64 {
    600
}

pub(crate) fn default_job_type_decl() -> String {
    "shell".to_string()
}

pub(crate) fn default_delivery_mode() -> String {
    "none".to_string()
}

pub(crate) fn default_max_run_history() -> u32 {
    50
}

pub(crate) fn default_openvpn_timeout() -> u64 {
    30
}

pub(crate) fn default_channel_message_timeout_secs() -> u64 {
    300
}

pub(crate) fn default_session_backend() -> String {
    "sqlite".into()
}

pub(crate) fn default_draft_update_interval_ms() -> u64 {
    1000
}

pub(crate) fn default_multi_message_delay_ms() -> u64 {
    800
}

pub(crate) fn default_telegram_approval_timeout_secs() -> u64 {
    120
}

pub(crate) fn default_channel_approval_timeout_secs() -> u64 {
    300
}

pub(crate) fn default_matrix_draft_update_interval_ms() -> u64 {
    1500
}

pub(crate) fn default_telegram_dm_topic_name() -> String {
    "General".to_string()
}

pub(crate) fn default_telegram_typing_cooldown_secs() -> f64 {
    30.0
}

pub(crate) fn default_slack_draft_update_interval_ms() -> u64 {
    1200
}

pub(crate) fn default_wati_api_url() -> String {
    "https://live-mt-server.wati.io".to_string()
}

pub(crate) fn default_mqtt_qos() -> u8 {
    1
}

pub(crate) fn default_mqtt_keep_alive_secs() -> u64 {
    30
}

pub(crate) fn default_irc_port() -> u16 {
    6697
}

pub(crate) fn default_line_webhook_port() -> u16 {
    8443
}

pub(crate) fn default_webauthn_rp_id() -> String {
    "localhost".into()
}

pub(crate) fn default_webauthn_rp_origin() -> String {
    "http://localhost:42617".into()
}

pub(crate) fn default_webauthn_rp_name() -> String {
    "Operant".into()
}

pub(crate) fn default_otp_token_ttl_secs() -> u64 {
    30
}

pub(crate) fn default_otp_cache_valid_secs() -> u64 {
    300
}

pub(crate) fn default_otp_challenge_max_attempts() -> u32 {
    3
}

pub(crate) fn default_otp_gated_actions() -> Vec<String> {
    vec![
        "shell".to_string(),
        "file_write".to_string(),
        "browser_open".to_string(),
        "browser".to_string(),
        "memory_forget".to_string(),
    ]
}

pub(crate) fn default_estop_state_file() -> String {
    default_path_under_config_dir("estop-state.json")
}

pub(crate) fn default_nevis_realm() -> String {
    "master".into()
}

pub(crate) fn default_nevis_token_validation() -> String {
    "local".into()
}

pub(crate) fn default_nevis_session_timeout_secs() -> u64 {
    3600
}

pub(crate) fn default_max_memory_mb() -> u32 {
    512
}

pub(crate) fn default_max_cpu_time_seconds() -> u64 {
    60
}

pub(crate) fn default_max_subprocesses() -> u32 {
    10
}

pub(crate) fn default_memory_monitoring_enabled() -> bool {
    true
}

pub(crate) fn default_audit_enabled() -> bool {
    true
}

pub(crate) fn default_audit_log_path() -> String {
    "audit.log".to_string()
}

pub(crate) fn default_audit_max_size_mb() -> u32 {
    100
}

pub(crate) fn default_mochat_poll_interval() -> u64 {
    5
}

#[cfg(feature = "voice-wake")]
pub(crate) fn default_voice_wake_word() -> String {
    "hey operant".into()
}

#[cfg(feature = "voice-wake")]
pub(crate) fn default_voice_wake_silence_timeout_ms() -> u32 {
    2000
}

#[cfg(feature = "voice-wake")]
pub(crate) fn default_voice_wake_energy_threshold() -> f32 {
    0.01
}

#[cfg(feature = "voice-wake")]
pub(crate) fn default_voice_wake_max_capture_secs() -> u32 {
    30
}

#[cfg(feature = "channel-nostr")]
/// Default public Nostr relay URLs used when the user omits `channels.nostr.relays`.
pub fn default_nostr_relays() -> Vec<String> {
    vec![
        "wss://relay.damus.io".to_string(),
        "wss://nos.lol".to_string(),
        "wss://relay.primal.net".to_string(),
        "wss://relay.snort.social".to_string(),
    ]
}

pub(crate) fn default_notion_poll_interval() -> u64 {
    5
}

pub(crate) fn default_notion_status_prop() -> String {
    "Status".into()
}

pub(crate) fn default_notion_input_prop() -> String {
    "Input".into()
}

pub(crate) fn default_notion_result_prop() -> String {
    "Result".into()
}

pub(crate) fn default_notion_max_concurrent() -> usize {
    4
}

pub(crate) fn default_notion_recover_stale() -> bool {
    true
}

pub(crate) fn default_jira_allowed_actions() -> Vec<String> {
    vec!["get_ticket".to_string()]
}

pub(crate) fn default_jira_timeout_secs() -> u64 {
    30
}

pub(crate) fn default_cloud_ops_cloud() -> String {
    "aws".into()
}

pub(crate) fn default_cloud_ops_supported_clouds() -> Vec<String> {
    vec!["aws".into(), "azure".into(), "gcp".into()]
}

pub(crate) fn default_cloud_ops_iac_tools() -> Vec<String> {
    vec!["terraform".into()]
}

pub(crate) fn default_cloud_ops_cost_threshold() -> f64 {
    100.0
}

pub(crate) fn default_cloud_ops_waf() -> Vec<String> {
    vec!["aws-waf".into()]
}

// ── Conversational AI ──────────────────────────────────────────────

pub(crate) fn default_conversational_ai_language() -> String {
    "en".into()
}

pub(crate) fn default_conversational_ai_supported_languages() -> Vec<String> {
    vec!["en".into(), "de".into(), "fr".into(), "it".into()]
}

pub(crate) fn default_conversational_ai_escalation_threshold() -> f64 {
    0.3
}

pub(crate) fn default_conversational_ai_max_turns() -> usize {
    50
}

pub(crate) fn default_conversational_ai_timeout_secs() -> u64 {
    1800
}

pub(crate) fn default_playbooks_dir() -> String {
    default_path_under_config_dir("playbooks")
}

pub(crate) fn default_require_approval() -> bool {
    true
}

pub(crate) fn default_max_auto_severity() -> String {
    "low".into()
}

pub(crate) fn default_report_output_dir() -> String {
    default_path_under_config_dir("security-reports")
}

pub(crate) fn default_config_and_workspace_dirs() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("workspace")))
}

pub(crate) fn default_config_dir() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("OPERANT_CONFIG_DIR") {
        let custom = custom.trim();
        if !custom.is_empty() {
            return Ok(expand_tilde_path(custom));
        }
    }

    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home).join(".operant"));
    }

    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home.join(".operant"))
}

/// Build a default path string by joining `relative` onto the resolved
/// platform config dir. The form sees the resolved absolute path
/// (`/home/<user>/.operant/<relative>` on Linux,
/// `C:\Users\<user>\.operant\<relative>` on Windows, etc.) instead of a
/// literal `~/...` token that doesn't expand on Windows. Falls back to
/// `~/.operant/<relative>` if the platform dir can't be resolved (rare —
/// e.g. no HOME and `directories::UserDirs` returns None); the runtime's
/// `expand_tilde_path()` handles that literal at use-time.
///
/// Switching to platform-native config locations (`~/Library/Application
/// Support/operant/` on macOS, `%APPDATA%\operant\` on Windows) is the
/// schema-v3 follow-up tracked in #5947 — that needs a migration to move
/// existing users' configs.
pub(crate) fn default_path_under_config_dir(relative: &str) -> String {
    match default_config_dir() {
        Ok(dir) => dir.join(relative).to_string_lossy().into_owned(),
        Err(_) => format!("~/.operant/{relative}"),
    }
}

pub(crate) fn default_sop_execution_mode() -> String {
    "supervised".to_string()
}

pub(crate) fn default_sop_max_concurrent_total() -> usize {
    4
}

pub(crate) fn default_sop_approval_timeout_secs() -> u64 {
    300
}

pub(crate) fn default_sop_max_finished_runs() -> usize {
    100
}
