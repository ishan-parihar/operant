//! Config types that were originally defined in their home modules (agent, channels, tools, trust)
//! but are needed by the config schema. Moved here to break circular dependencies.

use crate::traits::{ChannelConfig, HasPropKind, PropKind};
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Agent config types ──────────────────────────────────────────

/// How deeply the model should reason for a given message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    /// No reasoning effort; fastest, least capable.
    Off,
    /// Minimal reasoning effort.
    Minimal,
    /// Low reasoning effort.
    Low,
    #[default]
    /// Default reasoning effort.
    Medium,
    /// High reasoning effort.
    High,
    /// Maximum reasoning effort; slowest but most capable.
    Max,
}

impl HasPropKind for ThinkingLevel {
    const PROP_KIND: PropKind = PropKind::Enum;
}

impl ThinkingLevel {
    /// Parse a thinking level from a case-insensitive string (`"off"`, `"low"`, ...).
    pub fn from_str_insensitive(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "off" | "none" => Some(Self::Off),
            "minimal" | "min" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" | "med" | "default" => Some(Self::Medium),
            "high" => Some(Self::High),
            "max" | "maximum" => Some(Self::Max),
            _ => None,
        }
    }
}

/// Configuration for thinking/reasoning level control.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "agent.thinking"]
pub struct ThinkingConfig {
    /// Reasoning level applied to messages that do not request one explicitly.
    #[serde(default)]
    pub default_level: ThinkingLevel,
}

impl Default for ThinkingConfig {
    fn default() -> Self {
        Self {
            default_level: ThinkingLevel::Medium,
        }
    }
}

fn default_max_tokens() -> usize {
    8192
}
fn default_keep_recent() -> usize {
    4
}
fn default_collapse() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "agent.history-pruning"]
/// Prunes older turns from conversation history before it reaches the provider.
pub struct HistoryPrunerConfig {
    /// Whether history pruning is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Token ceiling above which older turns are pruned. Default: `8192`.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Number of most-recent turns always kept. Default: `4`.
    #[serde(default = "default_keep_recent")]
    pub keep_recent: usize,
    /// Collapse prior tool results to a single line to save tokens. Default: `true`.
    #[serde(default = "default_collapse")]
    pub collapse_tool_results: bool,
}

impl Default for HistoryPrunerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_tokens: 8192,
            keep_recent: 4,
            collapse_tool_results: true,
        }
    }
}

fn default_cost_optimized_hint() -> String {
    "cost-optimized".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "agent.auto-classify"]
/// Auto-classifies request complexity into routing hints for the model router.
pub struct AutoClassifyConfig {
    /// Prompt hint applied when a request is classified simple.
    #[serde(default)]
    pub simple_hint: Option<String>,
    /// Prompt hint applied when a request is classified standard.
    #[serde(default)]
    pub standard_hint: Option<String>,
    /// Prompt hint applied when a request is classified complex.
    #[serde(default)]
    pub complex_hint: Option<String>,
    /// Hint applied when the cost-optimized route is selected. Default: `"cost-optimized"`.
    #[serde(default = "default_cost_optimized_hint")]
    pub cost_optimized_hint: String,
}

impl Default for AutoClassifyConfig {
    fn default() -> Self {
        Self {
            simple_hint: None,
            standard_hint: None,
            complex_hint: None,
            cost_optimized_hint: default_cost_optimized_hint(),
        }
    }
}

fn default_min_quality_score() -> f64 {
    0.5
}
fn default_eval_max_retries() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "agent.eval"]
/// Quality evaluation of model responses before they are returned.
pub struct EvalConfig {
    /// Whether response evaluation is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Minimum quality score (0.0–1.0) a response must reach to pass. Default: `0.5`.
    #[serde(default = "default_min_quality_score")]
    pub min_quality_score: f64,
    /// Maximum evaluation retries before accepting the response. Default: `1`.
    #[serde(default = "default_eval_max_retries")]
    pub max_retries: u32,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_quality_score: default_min_quality_score(),
            max_retries: default_eval_max_retries(),
        }
    }
}

fn default_cc_enabled() -> bool {
    true
}
fn default_threshold_ratio() -> f64 {
    0.50
}
fn default_protect_first_n() -> usize {
    3
}
fn default_protect_last_n() -> usize {
    4
}
fn default_cc_max_passes() -> u32 {
    3
}
fn default_summary_max_chars() -> usize {
    4000
}
fn default_source_max_chars() -> usize {
    50_000
}
fn default_cc_timeout_secs() -> u64 {
    60
}
fn default_identifier_policy() -> String {
    "strict".to_string()
}
fn default_tool_result_retrim_chars() -> usize {
    2_000
}

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "agent.context-compression"]
/// Compresses context windows before the provider call to stay under limits.
pub struct ContextCompressionConfig {
    /// Whether context compression is enabled. Default: `true`.
    #[serde(default = "default_cc_enabled")]
    pub enabled: bool,
    /// Fraction of the context window at which compression triggers. Default: `0.50`.
    #[serde(default = "default_threshold_ratio")]
    pub threshold_ratio: f64,
    /// Leading turns protected from compression. Default: `3`.
    #[serde(default = "default_protect_first_n")]
    pub protect_first_n: usize,
    /// Trailing turns protected from compression. Default: `4`.
    #[serde(default = "default_protect_last_n")]
    pub protect_last_n: usize,
    /// Maximum compression passes per turn. Default: `3`.
    #[serde(default = "default_cc_max_passes")]
    pub max_passes: u32,
    /// Character ceiling for generated summaries. Default: `4000`.
    #[serde(default = "default_summary_max_chars")]
    pub summary_max_chars: usize,
    /// Character ceiling for source excerpts kept alongside summaries. Default: `50000`.
    #[serde(default = "default_source_max_chars")]
    pub source_max_chars: usize,
    /// Timeout (seconds) for a single compression pass. Default: `60`.
    #[serde(default = "default_cc_timeout_secs")]
    pub timeout_secs: u64,
    /// Model used to generate summaries; `None` reuses the route model.
    #[serde(default)]
    pub summary_model: Option<String>,
    /// Identifier policy applied when renaming identifiers in summaries. Default: `"strict"`.
    #[serde(default = "default_identifier_policy")]
    pub identifier_policy: String,
    /// Character budget for re-trimming tool results after compression. Default: `2000`.
    #[serde(default = "default_tool_result_retrim_chars")]
    pub tool_result_retrim_chars: usize,
    /// Tool-result keys exempt from re-trimming.
    #[serde(default)]
    pub tool_result_trim_exempt: Vec<String>,
}

impl Default for ContextCompressionConfig {
    fn default() -> Self {
        Self {
            enabled: default_cc_enabled(),
            threshold_ratio: default_threshold_ratio(),
            protect_first_n: default_protect_first_n(),
            protect_last_n: default_protect_last_n(),
            max_passes: default_cc_max_passes(),
            summary_max_chars: default_summary_max_chars(),
            source_max_chars: default_source_max_chars(),
            timeout_secs: default_cc_timeout_secs(),
            summary_model: None,
            identifier_policy: default_identifier_policy(),
            tool_result_retrim_chars: default_tool_result_retrim_chars(),
            tool_result_trim_exempt: Vec::new(),
        }
    }
}

fn default_precheck_enabled() -> bool {
    true
}
fn default_precheck_timeout_secs() -> u64 {
    5
}

/// Channel reply-intent precheck configuration.
///
/// The precheck runs a lightweight `REPLY` / `NO_REPLY` classifier before the
/// main agent loop so group-chat messages that are not addressed to the
/// assistant do not trigger a full tool-using turn. By default it reuses the
/// main route model, which can be unnecessarily slow on large reasoning
/// models — set `model` to a literal model name served by the same provider
/// to delegate the classification to a faster/cheaper model. A hard
/// `timeout_secs` keeps a slow provider from blocking the whole turn; on
/// timeout the precheck fails open to REPLY.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "agent.precheck"]
pub struct ChannelPrecheckConfig {
    /// When false, the precheck is skipped entirely and every channel message
    /// triggers the full agent loop. Default: `true`.
    #[serde(default = "default_precheck_enabled")]
    pub enabled: bool,
    /// Model used for the precheck classification call. When `None`, falls
    /// back to the route model used by the main agent turn. Must be a literal
    /// model name served by the same provider as the route model — the
    /// channel orchestrator does not resolve `hint:<name>` routing hints.
    /// Default: `None`.
    #[serde(default)]
    pub model: Option<String>,
    /// Hard ceiling (seconds) on the precheck LLM call. On timeout the
    /// precheck fails open to REPLY. Default: `5`.
    #[serde(default = "default_precheck_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for ChannelPrecheckConfig {
    fn default() -> Self {
        Self {
            enabled: default_precheck_enabled(),
            model: None,
            timeout_secs: default_precheck_timeout_secs(),
        }
    }
}

// ── Tools config types ──────────────────────────────────────────

fn default_browser_cli() -> String {
    "claude".into()
}
fn default_browser_task_timeout() -> u64 {
    120
}

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "browser-delegate"]
/// Drives a headless browser CLI for the browser-delegation tool.
pub struct BrowserDelegateConfig {
    /// Whether browser delegation is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Browser CLI binary used for delegation. Default: `"claude"`.
    #[serde(default = "default_browser_cli")]
    pub cli_binary: String,
    /// Chrome profile directory for persistent sessions; empty when unused.
    #[serde(default)]
    pub chrome_profile_dir: String,
    /// Domains the delegate is allowed to visit.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Domains the delegate must never visit.
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    /// Per-task timeout (seconds). Default: `120`.
    #[serde(default = "default_browser_task_timeout")]
    pub task_timeout_secs: u64,
}

impl Default for BrowserDelegateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cli_binary: default_browser_cli(),
            chrome_profile_dir: String::new(),
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
            task_timeout_secs: default_browser_task_timeout(),
        }
    }
}

// ── Trust config types ──────────────────────────────────────────

fn default_initial_score() -> f64 {
    0.8
}
fn default_decay_half_life() -> f64 {
    30.0
}
fn default_regression_threshold() -> f64 {
    0.5
}
fn default_correction_penalty() -> f64 {
    0.05
}
fn default_success_boost() -> f64 {
    0.01
}

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "trust"]
/// Scoring model for tool trust (used to gate/prioritize tool use).
pub struct TrustConfig {
    /// Initial trust score for new tools. Default: `0.8`.
    #[serde(default = "default_initial_score")]
    pub initial_score: f64,
    /// Half-life (days) for trust-score decay. Default: `30.0`.
    #[serde(default = "default_decay_half_life")]
    pub decay_half_life_days: f64,
    /// Score threshold below which a tool is considered regressed. Default: `0.5`.
    #[serde(default = "default_regression_threshold")]
    pub regression_threshold: f64,
    /// Score penalty applied when a correction follows a tool use. Default: `0.05`.
    #[serde(default = "default_correction_penalty")]
    pub correction_penalty: f64,
    /// Score boost applied on successful tool use. Default: `0.01`.
    #[serde(default = "default_success_boost")]
    pub success_boost: f64,
}

impl Default for TrustConfig {
    fn default() -> Self {
        Self {
            initial_score: default_initial_score(),
            decay_half_life_days: default_decay_half_life(),
            regression_threshold: default_regression_threshold(),
            correction_penalty: default_correction_penalty(),
            success_boost: default_success_boost(),
        }
    }
}

// ── Channel config types ────────────────────────────────────────

fn default_imap_port() -> u16 {
    993
}
fn default_smtp_port() -> u16 {
    465
}
fn default_imap_folder() -> String {
    "INBOX".into()
}
fn default_idle_timeout() -> u64 {
    1740
}
fn default_poll_interval_secs() -> u64 {
    60
}
fn default_true() -> bool {
    true
}
fn default_subject() -> String {
    "Operant Message".into()
}
fn default_max_attachment_bytes() -> usize {
    25 * 1024 * 1024
}

#[derive(Debug, Clone, Serialize, Deserialize, operant_macros::Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.email"]
/// Email channel (IMAP/SMTP) configuration.
pub struct EmailConfig {
    /// Whether the email channel is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// IMAP host for inbound mail.
    pub imap_host: String,
    /// IMAP port. Default: `993`.
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    /// IMAP folder to poll. Default: `"INBOX"`.
    #[serde(default = "default_imap_folder")]
    pub imap_folder: String,
    /// SMTP host for outbound mail.
    pub smtp_host: String,
    /// SMTP port. Default: `465`.
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    /// Whether SMTP uses TLS. Default: `true`.
    #[serde(default = "default_true")]
    pub smtp_tls: bool,
    /// IMAP/SMTP username.
    pub username: String,
    /// IMAP/SMTP password (secret).
    #[secret]
    pub password: String,
    /// From-address used on outbound mail.
    pub from_address: String,
    /// IMAP IDLE keep-alive timeout (seconds). Default: `1740`.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Polling interval used when the IMAP server does not advertise the IDLE
    /// capability (RFC 2177). Ignored when IDLE is available.
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Senders allowed to trigger agent turns; empty allows all.
    #[serde(default)]
    pub allowed_senders: Vec<String>,
    /// Default subject for outbound mail. Default: `"Operant Message"`.
    #[serde(default = "default_subject")]
    pub default_subject: String,
    /// Maximum accepted attachment size (bytes). Default: `25 MiB`.
    #[serde(default = "default_max_attachment_bytes")]
    pub max_attachment_bytes: usize,
}

impl ChannelConfig for EmailConfig {
    fn name() -> &'static str {
        "Email"
    }
    fn desc() -> &'static str {
        "Email over IMAP/SMTP"
    }
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            imap_host: String::new(),
            imap_port: default_imap_port(),
            imap_folder: default_imap_folder(),
            smtp_host: String::new(),
            smtp_port: default_smtp_port(),
            smtp_tls: true,
            username: String::new(),
            password: String::new(),
            from_address: String::new(),
            idle_timeout_secs: default_idle_timeout(),
            poll_interval_secs: default_poll_interval_secs(),
            allowed_senders: Vec::new(),
            default_subject: default_subject(),
            max_attachment_bytes: default_max_attachment_bytes(),
        }
    }
}

fn default_label_filter() -> Vec<String> {
    vec!["INBOX".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize, operant_macros::Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.gmail"]
/// Gmail Pub/Sub push notification channel configuration.
pub struct GmailPushConfig {
    /// Whether the Gmail channel is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Pub/Sub topic receiving Gmail push notifications.
    pub topic: String,
    /// Gmail labels that are forwarded. Default: `["INBOX"]`.
    #[serde(default = "default_label_filter")]
    pub label_filter: Vec<String>,
    /// OAuth token for Gmail API access (secret).
    #[serde(default)]
    #[secret]
    pub oauth_token: String,
    /// Senders allowed to trigger agent turns; empty allows all.
    #[serde(default)]
    pub allowed_senders: Vec<String>,
    /// Webhook URL receiving push payloads.
    #[serde(default)]
    pub webhook_url: String,
    /// Shared secret used to sign webhook payloads.
    #[serde(default)]
    pub webhook_secret: String,
}

impl ChannelConfig for GmailPushConfig {
    fn name() -> &'static str {
        "Gmail Push"
    }
    fn desc() -> &'static str {
        "Gmail Pub/Sub push notifications"
    }
}

impl Default for GmailPushConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            topic: String::new(),
            label_filter: default_label_filter(),
            oauth_token: String::new(),
            allowed_senders: Vec::new(),
            webhook_url: String::new(),
            webhook_secret: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, operant_macros::Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.clawdtalk"]
/// ClawdTalk telephony channel configuration.
pub struct ClawdTalkConfig {
    /// Whether the ClawdTalk channel is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// ClawdTalk API key (secret).
    #[secret]
    pub api_key: String,
    /// ClawdTalk connection identifier.
    pub connection_id: String,
    /// Source number for outbound messages.
    pub from_number: String,
    /// Destinations allowed to trigger agent turns; empty allows all.
    #[serde(default)]
    pub allowed_destinations: Vec<String>,
    /// Optional webhook signing secret (secret).
    #[serde(default)]
    #[secret]
    pub webhook_secret: Option<String>,
}

impl ChannelConfig for ClawdTalkConfig {
    fn name() -> &'static str {
        "ClawdTalk"
    }
    fn desc() -> &'static str {
        "ClawdTalk Channel"
    }
}

/// Which telephony provider to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum VoiceProvider {
    #[default]
    /// Twilio telephony provider.
    Twilio,
    /// Telnyx telephony provider.
    Telnyx,
    /// Plivo telephony provider.
    Plivo,
}

impl HasPropKind for VoiceProvider {
    const PROP_KIND: PropKind = PropKind::Enum;
}

impl fmt::Display for VoiceProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Twilio => write!(f, "twilio"),
            Self::Telnyx => write!(f, "telnyx"),
            Self::Plivo => write!(f, "plivo"),
        }
    }
}

fn default_webhook_port() -> u16 {
    8090
}
fn default_max_call_duration() -> u64 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.voice-call"]
/// Voice-call channel (Twilio/Telnyx/Plivo) configuration.
pub struct VoiceCallConfig {
    /// Whether the voice-call channel is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Telephony provider. Default: `twilio`.
    #[serde(default)]
    pub provider: VoiceProvider,
    /// Provider account identifier.
    pub account_id: String,
    /// Provider authentication token (secret).
    pub auth_token: String,
    /// Source number for outbound calls.
    pub from_number: String,
    /// Local port for inbound call webhooks. Default: `8090`.
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,
    /// Require explicit approval before placing outbound calls. Default: `true`.
    #[serde(default = "default_true")]
    pub require_outbound_approval: bool,
    /// Record and log call transcriptions. Default: `true`.
    #[serde(default = "default_true")]
    pub transcription_logging: bool,
    /// TTS voice identifier; `None` uses the provider default.
    #[serde(default)]
    pub tts_voice: Option<String>,
    /// Maximum call duration (seconds). Default: `3600`.
    #[serde(default = "default_max_call_duration")]
    pub max_call_duration_secs: u64,
    /// Public base URL for call webhooks; `None` derives from the gateway host.
    #[serde(default)]
    pub webhook_base_url: Option<String>,
}

impl Default for VoiceCallConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: VoiceProvider::default(),
            account_id: String::new(),
            auth_token: String::new(),
            from_number: String::new(),
            webhook_port: default_webhook_port(),
            require_outbound_approval: default_true(),
            transcription_logging: default_true(),
            tts_voice: None,
            max_call_duration_secs: default_max_call_duration(),
            webhook_base_url: None,
        }
    }
}
