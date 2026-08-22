//! `ops` — extracted verbatim from the former schema/core.rs monolith.
//! Re-exported from `schema` so every path is unchanged.

use super::*;
use crate::validation_bail;
use anyhow::Result;
use operant_macros::Configurable;
use serde::{Deserialize, Serialize};

///
/// Controls the read-only cloud transformation analysis tools:
/// IaC review, migration assessment, cost analysis, and architecture review.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "cloud-ops"]
pub struct CloudOpsConfig {
    /// Enable cloud operations tools. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Default cloud provider for analysis context. Default: "aws".
    #[serde(default = "default_cloud_ops_cloud")]
    pub default_cloud: String,
    /// Supported cloud providers. Default: [`aws`, `azure`, `gcp`].
    #[serde(default = "default_cloud_ops_supported_clouds")]
    pub supported_clouds: Vec<String>,
    /// Supported IaC tools for review. Default: \[`terraform`\].
    #[serde(default = "default_cloud_ops_iac_tools")]
    pub iac_tools: Vec<String>,
    /// Monthly USD threshold to flag cost items. Default: 100.0.
    #[serde(default = "default_cloud_ops_cost_threshold")]
    pub cost_threshold_monthly_usd: f64,
    /// Well-Architected Frameworks to check against. Default: \[`aws-waf`\].
    #[serde(default = "default_cloud_ops_waf")]
    pub well_architected_frameworks: Vec<String>,
}

impl Default for CloudOpsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_cloud: default_cloud_ops_cloud(),
            supported_clouds: default_cloud_ops_supported_clouds(),
            iac_tools: default_cloud_ops_iac_tools(),
            cost_threshold_monthly_usd: default_cloud_ops_cost_threshold(),
            well_architected_frameworks: default_cloud_ops_waf(),
        }
    }
}

impl CloudOpsConfig {
    /// Validate the cloud-ops config, bailing on empty required fields when enabled.
    pub fn validate(&self) -> Result<()> {
        if self.enabled {
            if self.default_cloud.trim().is_empty() {
                anyhow::bail!(
                    "cloud_ops.default_cloud must not be empty when cloud_ops is enabled"
                );
            }
            if self.supported_clouds.is_empty() {
                anyhow::bail!(
                    "cloud_ops.supported_clouds must not be empty when cloud_ops is enabled"
                );
            }
            for (i, cloud) in self.supported_clouds.iter().enumerate() {
                if cloud.trim().is_empty() {
                    validation_bail!(
                        RequiredFieldEmpty,
                        format!("cloud_ops.supported_clouds[{i}]"),
                        "cloud_ops.supported_clouds[{i}] must not be empty"
                    );
                }
            }
            if !self.supported_clouds.contains(&self.default_cloud) {
                anyhow::bail!(
                    "cloud_ops.default_cloud '{}' is not in cloud_ops.supported_clouds {:?}",
                    self.default_cloud,
                    self.supported_clouds
                );
            }
            if self.cost_threshold_monthly_usd < 0.0 {
                anyhow::bail!(
                    "cloud_ops.cost_threshold_monthly_usd must be non-negative, got {}",
                    self.cost_threshold_monthly_usd
                );
            }
            if self.iac_tools.is_empty() {
                anyhow::bail!("cloud_ops.iac_tools must not be empty when cloud_ops is enabled");
            }
        }
        Ok(())
    }
}

/// Conversational AI agent builder configuration (`[conversational_ai]` section).
///
/// **Status: Reserved for future use.** This configuration is parsed but not yet
/// consumed by the runtime. Setting `enabled = true` will produce a startup warning.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "conversational-ai"]
pub struct ConversationalAiConfig {
    /// Enable conversational AI features. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Default language for conversations (BCP-47 tag). Default: "en".
    #[serde(default = "default_conversational_ai_language")]
    pub default_language: String,
    /// Supported languages for conversations. Default: [`en`, `de`, `fr`, `it`].
    #[serde(default = "default_conversational_ai_supported_languages")]
    pub supported_languages: Vec<String>,
    /// Automatically detect user language from message content. Default: true.
    #[serde(default = "default_true")]
    pub auto_detect_language: bool,
    /// Intent confidence below this threshold triggers escalation. Default: 0.3.
    #[serde(default = "default_conversational_ai_escalation_threshold")]
    pub escalation_confidence_threshold: f64,
    /// Maximum conversation turns before auto-ending. Default: 50.
    #[serde(default = "default_conversational_ai_max_turns")]
    pub max_conversation_turns: usize,
    /// Conversation timeout in seconds (inactivity). Default: 1800.
    #[serde(default = "default_conversational_ai_timeout_secs")]
    pub conversation_timeout_secs: u64,
    /// Enable conversation analytics tracking. Default: false (privacy-by-default).
    #[serde(default)]
    pub analytics_enabled: bool,
    /// Optional tool name for RAG-based knowledge base lookup during conversations.
    #[serde(default)]
    pub knowledge_base_tool: Option<String>,
}

impl ConversationalAiConfig {
    /// Returns `true` when the feature is disabled (the default).
    ///
    /// Used by `#[serde(skip_serializing_if)]` to omit the entire
    /// `[conversational_ai]` section from newly-generated config files,
    /// avoiding user confusion over an undocumented / experimental section.
    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }
}

impl Default for ConversationalAiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_language: default_conversational_ai_language(),
            supported_languages: default_conversational_ai_supported_languages(),
            auto_detect_language: true,
            escalation_confidence_threshold: default_conversational_ai_escalation_threshold(),
            max_conversation_turns: default_conversational_ai_max_turns(),
            conversation_timeout_secs: default_conversational_ai_timeout_secs(),
            analytics_enabled: false,
            knowledge_base_tool: None,
        }
    }
}

// ── Security ops config ─────────────────────────────────────────

/// Managed Cybersecurity Service (MCSS) dashboard agent configuration (`[security_ops]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "security-ops"]
pub struct SecurityOpsConfig {
    /// Enable security operations tools.
    #[serde(default)]
    pub enabled: bool,
    /// Directory containing incident response playbook definitions (JSON).
    #[serde(default = "default_playbooks_dir")]
    pub playbooks_dir: String,
    /// Automatically triage incoming alerts without user prompt.
    #[serde(default)]
    pub auto_triage: bool,
    /// Require human approval before executing playbook actions.
    #[serde(default = "default_require_approval")]
    pub require_approval_for_actions: bool,
    /// Maximum severity level that can be auto-remediated without approval.
    /// One of: "low", "medium", "high", "critical". Default: "low".
    #[serde(default = "default_max_auto_severity")]
    pub max_auto_severity: String,
    /// Directory for generated security reports.
    #[serde(default = "default_report_output_dir")]
    pub report_output_dir: String,
    /// Optional SIEM webhook URL for alert ingestion.
    #[serde(default)]
    pub siem_integration: Option<String>,
}

impl Default for SecurityOpsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            playbooks_dir: default_playbooks_dir(),
            auto_triage: false,
            require_approval_for_actions: true,
            max_auto_severity: default_max_auto_severity(),
            report_output_dir: default_report_output_dir(),
            siem_integration: None,
        }
    }
}
