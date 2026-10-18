use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform;

// Re-export the `[providers]` section types so consumers (CLI run path) can
// name provider profiles without depending on operant-config directly.
pub use operant_config::providers::ProvidersConfig;
pub use operant_config::schema::{FallbackProviderConfig, ModelProviderConfig};

static RUNTIME_CONFIG: OnceLock<RwLock<AppConfig>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Configuration format version. None = v0 (legacy, no version field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    pub client: ClientSettings,
    /// Outbound HTTP proxy for provider and tool clients (`[proxy]`), applied at startup.
    pub proxy: operant_config::schema::ProxyConfig,
    pub agent: BehaviorSettings,
    pub autonomous: AutonomousSettings,
    pub logging: LoggingSettings,
    pub tui: TuiSettings,
    pub mcp: McpSettings,
    pub skills: SkillsSettings,
    pub gateway: GatewaySettings,
    pub plugins: PluginSettings,
    pub tools: ToolSettings,
    pub tts: TtsSettings,
    pub memory: MemorySettings,
    pub browser: BrowserSettings,
    pub vision: VisionSettings,
    pub credential_pool: CredentialPoolSettings,
    pub terminal_backend: TerminalBackend,
    pub auxiliary_models: AuxiliaryModels,
    pub moa: MoaSettings,
    /// Always-on transcript watchdog (`[advisor]` section, omp
    /// `coding-agent/src/advisor/` parity). Distinct from MoA: MoA is a
    /// per-turn fan-out amplifier; the advisor is an incremental reviewer
    /// that notes concerns after each completed turn without joining the
    /// acting agent's context.
    pub advisor: crate::advisor::AdvisorConfig,
    pub checkpoints: CheckpointsSettings,
    /// Plan 016 / Phase 0 config surface for `operant-harness`. Dark-merged:
    /// `enabled` defaults to `false` and no provider is mounted unless the
    /// user opts in. Phase 2+ wires the kernel into the live
    /// `ToolRegistry` / `HookRunner` behind this flag.
    pub harness: HarnessSettings,
    /// Plan 015 / Phase 0 config surface for the Prime Kernel sidecar.
    /// Dark-merged: `enabled` defaults to `false`; the sidecar is not
    /// even spawned until a Phase 1+ rollout flips this and provides
    /// a working `sidecar_script` path.
    pub pk: PkSettings,
    pub database_path: PathBuf,
    /// Provider profiles + ordered cross-provider fallback chain
    /// (`[providers]`, hermes `fallback_providers` parity). Consumed by the
    /// run path to build a [`crate::agent::provider_registry::ProviderRegistry`]
    /// so auth/billing failures can switch providers; same-provider model
    /// fallback is driven by `agent.fallback_models`.
    #[serde(default)]
    pub providers: operant_config::providers::ProvidersConfig,
    /// Tool-approval allowlist patterns (hermes `command_allowlist` parity).
    /// Patterns match tool names exactly or via `*`/`?` globs (e.g.
    /// `"file_*"`). A matching tool skips the permission prompt in every
    /// session. `Always allow` choices made interactively are appended here
    /// (via the sidecar `approval_allowlist.json`) and honored across
    /// restarts.
    #[serde(default)]
    pub command_allowlist: Vec<String>,
    /// Permission-genome knobs (`[genome]`, wave-2 slice F2). `grant_ttl_days`
    /// is LIVE: the TTL an approval mints for a non-CEO approver
    /// (`SeatApprover`). The other two are declared surface for the named
    /// later slices — see `GenomeSettings` for exactly which.
    #[serde(default)]
    pub genome: GenomeSettings,
    /// DM/feed context injection (`[context_injection]`, gap 5 phase 1).
    #[serde(default)]
    pub context_injection: ContextInjectionSettings,
    /// Daily seat-pairing sessions (`[socialization]`, gap 6).
    #[serde(default)]
    pub socialization: SocializationSettings,
    /// Observability backend selection (`[observability]`, schema parity,
    /// v6 outline row 5). The gateway mounts the observer this section
    /// selects on the cron scheduler and delivery sender loop — the
    /// CronDeliveryQueued/Parked/Reclaimed/Outcome quartet (iter-720).
    /// Default (`backend = "none"`) is the schema default: dark-merged,
    /// no observer, byte-identical behavior.
    #[serde(default)]
    pub observability: operant_config::schema::ObservabilityConfig,
    /// `[holon]` — org-mode surface (docs/agentic-os/, phase B+). Off by
    /// default: invariant 5, the personal agent is byte-identical.
    #[serde(default)]
    pub holon: HolonSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        let database_path = dirs::home_dir()
            .map(|h| h.join(".operant").join("database.db"))
            .unwrap_or_else(|| PathBuf::from("database.db"));

        Self {
            version: Some(2),
            client: ClientSettings::default(),
            proxy: Default::default(),
            agent: BehaviorSettings::default(),
            autonomous: AutonomousSettings::default(),
            logging: LoggingSettings::default(),
            tui: TuiSettings::default(),
            mcp: McpSettings::default(),
            skills: SkillsSettings::default(),
            gateway: GatewaySettings::default(),
            plugins: PluginSettings::default(),
            tools: ToolSettings::default(),
            tts: TtsSettings::default(),
            memory: MemorySettings::default(),
            browser: BrowserSettings::default(),
            vision: VisionSettings::default(),
            credential_pool: CredentialPoolSettings::default(),
            terminal_backend: TerminalBackend::Local,
            checkpoints: CheckpointsSettings::default(),
            harness: HarnessSettings::default(),
            genome: GenomeSettings::default(),
            context_injection: ContextInjectionSettings::default(),
            socialization: SocializationSettings::default(),
            observability: operant_config::schema::ObservabilityConfig::default(),
            pk: PkSettings::default(),
            auxiliary_models: AuxiliaryModels::default(),
            moa: MoaSettings::default(),
            advisor: crate::advisor::AdvisorConfig::default(),
            database_path,
            providers: operant_config::providers::ProvidersConfig::default(),
            command_allowlist: Vec::new(),
            holon: HolonSettings::default(),
        }
    }
}

/// `[genome]` — the permission-genome knobs (wave-2 slice F2,
/// `docs/PERMISSION-SCOPING-PLAN.md` §5 P2 + §11's owner defaults).
///
/// All knobs are live — the repo's `lifeos` rule forbids keys that
/// gate nothing, so each field below states exactly which slice reads it.
///
/// - `grant_ttl_days` — LIVE (F2): the TTL in days a department-head
///   approval mints (`SeatApprover::mint`). `0`/negative = never-standing
///   for non-CEO approvals: the grant is recorded already-lapsed, so the
///   approval is auditable while conferring nothing. CEO+ (org-lead)
///   approvals mint standing grants (`expires_at = None`) regardless.
/// - `unrestricted_default` — consumed by the `/grant` command (P3, live
///   since iter-598) as the mode a newly-seated policy row carries when the
///   operator does not name one. The RUNTIME default for a seat without a
///   policy row stays ungoverned (byte-identical to mainline — which is
///   NOT `yolo`: dangerous tools still prompt), per
///   `org/seat_policy.rs` rule 2.
/// - `unattended_posture` — D-2 (owner ruling 2026-10-08): the posture an
///   UNGOVERNED unattended run takes on a dangerous tool when nobody can
///   answer the prompt (no active channel). `yolo` (the default) keeps the
///   legacy auto-approve byte-identical; any other `SeatMode` spelling
///   denies with a reason naming the knob; a typo'd spelling fails CLOSED.
///   Governed seats are NOT affected — their policy row already clamps
///   unattended runs (F2, `agent/stream.rs`). This knob, and only this
///   knob, manages the unattended posture: the governance config surface
///   the owner ruled it into.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GenomeSettings {
    /// Default seat mode for operator-seated policy rows that do not name
    /// one (P3 consumer; see the struct docs).
    pub unrestricted_default: String,
    /// D-2: the posture for UNGOVERNED unattended permission requests that
    /// have no interactive approver (`SeatMode` spelling; see the struct
    /// docs). Default `yolo` = legacy auto-approve, byte-identical.
    pub unattended_posture: String,
    /// Days a department-head approval's grant stands. 0/negative =
    /// never-standing for non-CEO approvals (see the struct docs).
    pub grant_ttl_days: i64,
    /// Wave 3 (ORGANISM-ARCHITECTURE §3): minutes a session's rolling
    /// summary stays a valid continuation entry point after the turn that
    /// wrote it. 15–30 per the owner spec; default 30. A continuation
    /// (restart / session reload) within the window begins from the
    /// latest summary; after the window it starts fresh. The summary is
    /// UPSERTed per (employee, session) every completed turn — rolling,
    /// never aggregating old rolls.
    pub session_summary_ttl_minutes: i64,
    /// Wave 4 (ORGANISM-ARCHITECTURE §5): the ORG-WIDE default budget a
    /// seat runs under when no `seat_budgets` row overrides it. `cap = 0`
    /// (the default) = ungoverned — no budget is consulted at all, which
    /// is the genome's rule-2 posture (byte-identical legacy). Per-employee
    /// overrides live in the `seat_budgets` table (data, not config) so
    /// hrmaster can fine-tune one seat without a config edit + restart.
    pub budget: BudgetSettings,
}

/// `[context_injection]` — the DM/feed context-injection quotas and
/// ranking weights (plan-2026-10-08-dm-feed-context-injection.md, owner
/// rulings 2026-10-08). Phase 1 reads org-internal sources only; every
/// knob here governs the four aspect classes the owner named.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextInjectionSettings {
    /// Master switch. `false` = every prompt byte-identical (dark-mergeable).
    pub enabled: bool,
    /// Hard ceiling for the whole rendered section, chars.
    pub total_char_cap: usize,
    /// Per-aspect char quotas (the MEMORY.md budget model, per aspect).
    pub dm_quota: usize,
    /// Feed sources (channel/group posts captured at route time,
    /// plan-2026-10-09 §3): per-seat char quota, same budget model.
    pub feed_quota: usize,
    pub global_quota: usize,
    pub dept_quota: usize,
    pub self_quota: usize,
    /// Feed seat routing: platform chat id → the seat whose feed it is.
    /// Unmapped chats default to `premiere` (the operator's default
    /// binding, same rule as the DM tap).
    pub feed_seat_map: std::collections::HashMap<String, String>,
    /// Recency decay half-lives (hours): feeds go stale faster than DMs.
    pub recency_halflife_hours_feed: f64,
    pub recency_halflife_hours_dm: f64,
    /// Ranking weights: recency + lexical affinity + author authority.
    /// (Thread participation is the design doc's fourth component; phase-1
    /// items carry no thread refs, so it applies to zero until phase 2.)
    pub weight_recency: f64,
    pub weight_lexical: f64,
    pub weight_authority: f64,
}

impl Default for ContextInjectionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            total_char_cap: 2000,
            dm_quota: 800,
            feed_quota: 500,
            global_quota: 500,
            dept_quota: 400,
            self_quota: 300,
            feed_seat_map: std::collections::HashMap::new(),
            recency_halflife_hours_feed: 24.0,
            recency_halflife_hours_dm: 168.0,
            weight_recency: 1.0,
            weight_lexical: 1.0,
            weight_authority: 0.5,
        }
    }
}

/// `[socialization]` — the daily seat-pairing sessions (gap 6,
/// plan-2026-10-08-socialization-sessions.md). Pairs are DATA and ordered:
/// each entry is `[senior, junior]` — the senior seat initiates; junior
/// voice is an explicit grant, visibly exceptional.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SocializationSettings {
    /// Master switch. `false` (default) = the sessions never run —
    /// dark-mergeable like every wave.
    pub enabled: bool,
    /// cron expression, six fields (sec min hour dom mon dow), UTC.
    pub schedule: String,
    /// Shared turn budget per pair per day (the dm_threads envelope).
    pub turn_budget: u32,
    /// `[senior, junior]` pairs. Invalid entries (wrong arity, duplicate
    /// seats) are skipped with a warning, never fail the tick.
    pub pairs: Vec<Vec<String>>,
}

impl Default for SocializationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            schedule: "0 30 9 * * *".to_string(),
            turn_budget: 3,
            pairs: Vec::new(),
        }
    }
}

/// `[holon]` — operant as the runtime of a Holon organization
/// (docs/agentic-os/, boundary doc §4.1). Phase B: only `enabled` and `home`
/// are live (the read API behind `operant org`); later phases add the
/// supervisor/runner/broker and Chair-channel keys here. `enabled = false`
/// (default) is invariant 5: the personal agent is byte-identical, org
/// surfaces off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolonSettings {
    /// Master switch for every Holon-backed surface (org views, panels,
    /// enforcement, supervisor). Off = personal mode, byte-identical.
    #[serde(default)]
    pub enabled: bool,
    /// `$HOLON_HOME` — the Holon instance operant reads (and, from phase E,
    /// drives). Its `work.db` is opened read-only through the read API.
    #[serde(default)]
    pub home: PathBuf,
    /// Phase E: pin the kernel launcher (`bin/holon`). Empty = the PATH
    /// `holon` (or `$HOLON_BIN`).
    #[serde(default)]
    pub bin: Option<String>,
    /// Phase F: the kernel's notion adapter script the gateway's webhook
    /// route invokes per event (`adapters/notion/holon_notion.py` in the
    /// kernel's repo). Empty = `$HOLON_HOME/adapters/notion/holon_notion.py`.
    #[serde(default)]
    pub notion_script: Option<PathBuf>,
    /// Phase E: the run supervisor claims, spawns, heartbeats and hands in
    /// (CONTRACT §11) for the employees `supervise_employees` names.
    #[serde(default)]
    pub supervise: bool,
    /// Employees the supervisor serves, e.g. ["emp_01…"]. Required when
    /// `supervise` is on.
    #[serde(default)]
    pub supervise_employees: Vec<String>,
    /// Bodies the supervisor runs at once (the kernel enforces the
    /// employee's own max_concurrency too).
    #[serde(default = "default_max_concurrent_runs")]
    pub max_concurrent_runs: usize,
    /// Phase E: fire `holon tick` on the no-agent cron (every 2 minutes)
    /// instead of a systemd timer — the clock belongs to the resident
    /// runtime. When off, the kernel's own timer keeps ticking.
    #[serde(default)]
    pub clock: bool,
}

fn default_max_concurrent_runs() -> usize {
    2
}

/// O-26: expand a leading `~` in a [holon] path. The plan's own example
/// writes `home = "~/holon"` — without this it reaches the API as a
/// literal `~` and fails with "work database not found at ~/holon/…".
fn expand_holon_path(p: &std::path::Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::Path::new(&home).join(rest);
    }
    if s == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home);
    }
    p.to_path_buf()
}

impl HolonSettings {
    /// O-26: resolve `~` in every [holon] path field, once, at load.
    pub fn expand_tilde(mut self) -> Self {
        self.home = expand_holon_path(&self.home);
        if let Some(b) = self.bin.clone() {
            self.bin = Some(
                expand_holon_path(&PathBuf::from(b))
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        if let Some(s) = self.notion_script.clone() {
            self.notion_script = Some(expand_holon_path(&s));
        }
        self
    }
}

impl Default for HolonSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            home: PathBuf::new(),
            bin: None,
            notion_script: None,
            supervise: false,
            supervise_employees: Vec::new(),
            max_concurrent_runs: default_max_concurrent_runs(),
            clock: false,
        }
    }
}

/// `[genome].budget` — Wave 4. Basis is `tokens` or `usd` (metering has
/// both; tokens is model-agnostic, USD maps 1:1 to paid-model invoices).
/// Window is `daily` | `weekly` | `monthly`, UTC boundaries. Mode `hard`
/// refuses the turn at the cap (with a clear notice); `soft` warns and
/// continues. Enforcement is TURN-BOUNDARY v1: a turn that STARTS over a
/// hard cap is refused; in-flight turns complete and their usage lands in
/// the accumulator for the next boundary check. Mid-flight enforcement is
/// a recorded follow-up, not this contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BudgetSettings {
    /// `tokens` (default) or `usd` — what `cap` counts.
    pub basis: String,
    /// `daily` (default) | `weekly` | `monthly` — the rollup window.
    pub window: String,
    /// The cap on the basis within the window. 0 = ungoverned (no budget).
    pub cap: f64,
    /// `soft` (default: warn + continue) or `hard` (refuse at the cap).
    pub mode: String,
}

impl Default for BudgetSettings {
    fn default() -> Self {
        Self {
            basis: "tokens".to_string(),
            window: "daily".to_string(),
            cap: 0.0,
            mode: "soft".to_string(),
        }
    }
}

impl Default for GenomeSettings {
    fn default() -> Self {
        Self {
            unrestricted_default: "yolo".to_string(),
            unattended_posture: "yolo".to_string(),
            grant_ttl_days: 7,
            session_summary_ttl_minutes: 30,
            budget: BudgetSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClientSettings {
    pub base_url: String,
    pub api_key: Option<String>,
    /// Additional API keys for credential pool / multi-key rotation.
    #[serde(default)]
    pub additional_api_keys: Vec<String>,
    pub timeout_secs: u64,
    pub max_context_length: usize,
    /// Rate limit configuration for outbound API requests.
    pub rate_limit: RateLimitSettings,
    /// TTL for provider prompt-cache breakpoints ("5m" or "1h"). Drives the
    /// `cache_control` markers applied by the OpenAI/OpenRouter and Anthropic
    /// adapters (zeroclaw port: was hardcoded to the 5-minute default).
    #[serde(default)]
    pub prompt_cache_ttl: crate::agent::clients::prompt_caching::CacheTtl,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: None,
            additional_api_keys: Vec::new(),
            timeout_secs: 60,
            max_context_length: 128_000,
            rate_limit: RateLimitSettings::default(),
            prompt_cache_ttl: Default::default(),
        }
    }
}

/// Settings for token-bucket rate limiting of outbound API requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitSettings {
    /// Maximum number of retry attempts after a 429 or transient error.
    pub max_retries: u32,
    /// Base delay (seconds) for exponential backoff.
    pub base_delay_secs: u64,
    /// Maximum delay (seconds) for exponential backoff.
    pub max_delay_secs: u64,
    /// Default token-bucket capacity (max requests in a burst).
    pub bucket_capacity: u32,
    /// Default token refill rate (tokens per second).
    pub bucket_refill_rate: f64,
}

impl Default for RateLimitSettings {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_secs: 5,
            max_delay_secs: 60,
            bucket_capacity: 60,
            bucket_refill_rate: 1.0,
        }
    }
}

/// How tool execution progress is reported to the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ToolProgressMode {
    PerStep,
    FinalOnly,
    Streaming,
    #[default]
    Auto,
}

/// When the conversation session should be automatically reset.
/// Serde validator for `BehaviorSettings::reasoning_effort`: empty (don't
/// send) or one of minimal|low|medium|high|xhigh (plan 03). Same set as
/// operant-config's `normalize_reasoning_effort`.
fn deserialize_validated_reasoning_effort<'de, D>(
    deserializer: D,
) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: String = String::deserialize(deserializer)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let normalized = trimmed.to_ascii_lowercase();
    match normalized.as_str() {
        "minimal" | "low" | "medium" | "high" | "xhigh" => Ok(normalized),
        _ => Err(serde::de::Error::custom(format!(
            "reasoning_effort {raw:?} is invalid (expected \"\" or one of: minimal, low, medium, high, xhigh)"
        ))),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SessionResetMode {
    #[default]
    Never,
    OnSystemPromptChange,
    OnToolChange,
    Always,
}

/// How `[agent] reasoning_effort` is serialized per endpoint family
/// (2026-10-14 batch, plan 03).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffortParam {
    /// Pick the shape from the base_url/model: openrouter/kilo → `reasoning`
    /// object; api.openai.com or o*/gpt-5*/codex models → `reasoning_effort`;
    /// native Anthropic → `thinking` budget; everything else (proxies) →
    /// `reasoning_effort`.
    #[default]
    Auto,
    /// Always send `"reasoning_effort": "<e>"`.
    ReasoningEffort,
    /// Always send `"reasoning": {"effort": "<e>"}`.
    ReasoningObject,
    /// Always send `"thinking": {"type": "enabled", "budget_tokens": B}`
    /// (Anthropic-native; drops temperature per their contract).
    AnthropicThinking,
    /// Never send an effort field.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BehaviorSettings {
    pub model: String,
    pub max_iterations: usize,
    pub tool_timeout_secs: u64,
    pub request_timeout_secs: u64,
    pub system_prompt: Option<String>,
    pub stream: bool,
    pub context_window: usize,
    pub max_healing_attempts: usize,
    pub show_reasoning: bool,
    pub tool_progress: ToolProgressMode,
    pub session_reset: SessionResetMode,
    pub context_compression: bool,
    pub context_compression_threshold: f64,
    /// Maximum share of `context_window` that a SINGLE tool result may occupy
    /// before operant withholds the bulk of it. Expressed as a fraction
    /// (0.0-1.0) rather than an absolute token count so the bound tracks the
    /// configured model's window instead of assuming one. When the ceiling is
    /// exceeded the model is told the result's size, its share of the window
    /// and its estimated input price, and only a marked prefix is inlined.
    /// Set to 0.0 to disable the guard.
    #[serde(default = "default_max_tool_result_share")]
    pub max_tool_result_share: f64,
    /// Max consecutive tool-only iterations before force-answer kicks in.
    /// When the LLM calls tools N times in a row without producing text,
    /// the agent omits tools from the request to force a textual response.
    /// Set to 0 to disable (use max_iterations as the only limit).
    pub max_consecutive_tool_only: usize,

    /// Ordered list of fallback models to try when the primary model fails
    /// with a retryable provider error (5xx, 429, network error).
    /// The models are tried in order until one succeeds or all are exhausted.
    /// Fallback is per-request — the next request starts again with the primary model.
    #[serde(default)]
    pub fallback_models: Vec<String>,

    /// Tool names exempt from the guardrail rungs (ping-pong /
    /// no-progress — `ToolGuardrailTracker::add_exempt_tools`,
    /// hermes `guardrail_exempt_tools` parity). Exact names or `*`/`?`
    /// globs (e.g. `"read_*"`). Empty by default: every tool is guarded.
    /// Flows to the tracker via `AgentConfig::guardrail_exempt_tools`.
    #[serde(default)]
    pub guardrail_exempt_tools: Vec<String>,

    /// Guardian-LLM approval arm (iter-713) as a config-file DEFAULT:
    /// applies only when the `OPERANT_GUARDIAN_LLM` env var is UNSET —
    /// the env var (and the `with_guardian_llm` builder) still win when
    /// present. Off by default.
    #[serde(default)]
    pub guardian_llm: bool,

    /// Micro-compaction (iter-714) as a config-file DEFAULT: applies only
    /// when the `OPERANT_MICRO_COMPACTION` env var is UNSET — the env var
    /// (and the `with_micro_compaction` builder) still win when present.
    /// Off by default.
    #[serde(default)]
    pub micro_compaction: bool,

    /// Whether to automatically fall back to `fallback_models` on provider errors.
    /// Default: `true`. Set to `false` to disable fallback behavior entirely.
    #[serde(default = "default_fallback_on_errors")]
    pub fallback_on_errors: bool,

    /// Turns between memory reviews (0 = disabled). Mirrors hermes-agent's
    /// `memory_nudge_interval` (default 10).
    #[serde(default = "default_memory_nudge_interval")]
    pub memory_nudge_interval: usize,
    /// Reasoning effort sent on every main-turn request (2026-10-14 batch,
    /// plan 03). Empty = don't send; one of minimal|low|medium|high|xhigh.
    /// Invalid values fail config load with the allowed set named.
    #[serde(default, deserialize_with = "deserialize_validated_reasoning_effort")]
    pub reasoning_effort: String,
    /// Wire shape for the effort field (plan 03); see ReasoningEffortParam.
    #[serde(default)]
    pub reasoning_effort_param: ReasoningEffortParam,

    /// Iterations between skill nudges (0 = disabled). Mirrors hermes-agent's
    /// `creation_nudge_interval` (default 10).
    #[serde(default = "default_creation_nudge_interval")]
    pub creation_nudge_interval: usize,

    /// Idle-recap (omp `recap.*` parity): when the terminal/gateway has been
    /// idle this many seconds after a completed turn, an ephemeral
    /// side-channel LLM call renders a ~40-word "where the session stands"
    /// recap over a snapshot of the transcript (never the live context) and
    /// surfaces it as a dim `※ recap:` line in the TUI (and as a message on
    /// gateway channels). Any user activity cancels the pending recap.
    /// 0 = disabled. Default 240s (omp's `recap.idleSeconds`).
    #[serde(default = "default_recap_idle_seconds")]
    pub recap_idle_seconds: u64,

    /// Context engine used to assemble the per-call message list.
    ///   - `"compact"` (default): deterministic decay + `evict_to_budget`
    ///     (current behavior — lossy when over budget).
    ///   - `"lcm"`: lossless DAG + fresh-tail assembly (hermes-lcm parity;
    ///     see docs/HERMES_LCM_INTEGRATION.md). Opt-in — the default is
    ///     unchanged until rollups ship in P1.
    #[serde(default = "default_context_engine")]
    pub context_engine: String,
    /// SQLite path for the LCM lossless DAG (default `~/.operant/lcm.db`).
    #[serde(default)]
    pub context_lcm_db: Option<PathBuf>,
    /// Fresh-tail (D0) token budget kept verbatim by the LCM engine.
    /// Older messages are compacted into the DAG and recallable verbatim.
    #[serde(default = "default_context_lcm_tail_tokens")]
    pub context_lcm_tail_tokens: usize,
    /// P3 adaptive auto-recall: when the LCM engine assembles the context,
    /// run one bounded retrieval round against the latest user message and
    /// inject the top hits as a system "pre-answer evidence" block (hermes
    /// `adaptive_retrieval.py` / pre-answer evidence parity). Default on —
    /// costs one FTS query per turn.
    #[serde(default = "default_true")]
    pub context_lcm_auto_recall: bool,
    /// Max nodes auto-recalled and injected per assemble (default 3).
    #[serde(default = "default_lcm_auto_recall_limit")]
    pub context_lcm_auto_recall_limit: usize,
    /// Hard cap on the injected evidence block, in characters (default 4000).
    #[serde(default = "default_lcm_auto_recall_max_chars")]
    pub context_lcm_auto_recall_max_chars: usize,
    /// P1 rollup-in-compaction: when the LCM engine compacts an over-budget
    /// context, inject stored day/week/month rollup summaries instead of a
    /// bare placeholder marker. Default on — rollups are only injected when
    /// they already exist (built via `operant context rollup`).
    #[serde(default = "default_true")]
    pub context_lcm_rollups_inject: bool,
    /// P1 background rollup maintenance: interval in minutes between
    /// automatic maintenance passes that build missing day/week/month
    /// rollups for all DAG sessions (hermes `_RollupMaintenanceScheduler`
    /// parity). `0` (default) disables the background task — run
    /// `operant context rollup-maintenance` manually instead.
    #[serde(default = "default_lcm_rollup_interval_minutes")]
    pub context_lcm_rollup_interval_minutes: u64,
    /// P3 vector recall: embedding model for `lcm_vector_recall` (hermes
    /// `embedding_provider.py` parity). Empty (default) disables the vector
    /// tool — the provider must expose the OpenAI-compatible `/embeddings`
    /// endpoint.
    #[serde(default)]
    pub context_lcm_embedding_model: Option<String>,
    /// Base URL for the embeddings endpoint when it differs from the chat
    /// provider (e.g. a local Ollama at `http://localhost:11434/v1`). Empty
    /// (default) reuses the chat provider's base URL and key.
    #[serde(default)]
    pub context_lcm_embedding_base_url: Option<String>,
    /// P3 LLM-driven assertion extraction (hermes `assertion_extraction.py`
    /// `ModelAssertionExtractor` parity, opt-in): when true, the
    /// `lcm_assert` tool's `action = "extract"` mines durable
    /// (subject, predicate, object) facts out of the most recent DAG nodes
    /// and persists them to the assertion store. Off by default — costs one
    /// LLM call per extract invocation.
    #[serde(default)]
    pub context_lcm_assertion_extraction: bool,
    /// hermes-lcm `ignore_session_patterns` parity: glob patterns (fnmatch
    /// `*`/`?`/`[...]`) of sessions to skip in global DAG recall (FTS,
    /// vector, recent). Explicit per-session recall is unaffected. Empty
    /// (default) recalls across every session.
    #[serde(default)]
    pub context_lcm_ignore_session_patterns: Vec<String>,
    /// hermes-lcm `read_only` session scopes parity: session ids that must
    /// never be mutated — ingest is a no-op so archived transcripts stay
    /// byte-for-byte stable. Empty (default) allows all sessions to write.
    #[serde(default)]
    pub context_lcm_readonly_sessions: Vec<String>,
}

fn default_fallback_on_errors() -> bool {
    true
}

fn default_memory_nudge_interval() -> usize {
    10
}

fn default_creation_nudge_interval() -> usize {
    10
}

fn default_recap_idle_seconds() -> u64 {
    240
}

fn default_context_engine() -> String {
    "compact".to_string()
}

fn default_context_lcm_tail_tokens() -> usize {
    12_000
}

fn default_lcm_auto_recall_limit() -> usize {
    3
}

fn default_lcm_auto_recall_max_chars() -> usize {
    4_000
}

fn default_lcm_rollup_interval_minutes() -> u64 {
    0
}

/// Default for `max_tool_result_share` — 5% of the context window. See
/// `context_management::DEFAULT_MAX_TOOL_RESULT_SHARE` for the rationale.
fn default_max_tool_result_share() -> f64 {
    crate::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            model: "gpt-4".to_string(),
            max_iterations: 90,
            tool_timeout_secs: 30,
            request_timeout_secs: 120,
            system_prompt: None,
            stream: true,
            context_window: 128_000,
            max_healing_attempts: 3,
            show_reasoning: true,
            reasoning_effort: String::new(),
            reasoning_effort_param: ReasoningEffortParam::Auto,
            tool_progress: ToolProgressMode::Auto,
            session_reset: SessionResetMode::Never,
            context_compression: false,
            context_compression_threshold: 0.5,
            max_tool_result_share: default_max_tool_result_share(),
            max_consecutive_tool_only: 90,
            fallback_models: Vec::new(),
            guardrail_exempt_tools: Vec::new(),
            guardian_llm: false,
            micro_compaction: false,
            fallback_on_errors: true,
            memory_nudge_interval: 10,
            creation_nudge_interval: 10,
            recap_idle_seconds: 240,
            context_engine: "compact".to_string(),
            context_lcm_db: None,
            context_lcm_tail_tokens: 12_000,
            context_lcm_auto_recall: true,
            context_lcm_auto_recall_limit: 3,
            context_lcm_auto_recall_max_chars: 4_000,
            context_lcm_rollups_inject: true,
            context_lcm_rollup_interval_minutes: 0,
            context_lcm_embedding_model: None,
            context_lcm_embedding_base_url: None,
            context_lcm_assertion_extraction: false,
            context_lcm_ignore_session_patterns: Vec::new(),
            context_lcm_readonly_sessions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutonomousSettings {
    pub interval_secs: u64,
    pub todo_path: PathBuf,
    pub status_path: PathBuf,
    pub test_command: String,
    pub git_remote: String,
    pub git_branch: String,
    pub commit_message: String,
    pub command_timeout_secs: u64,
    pub max_failures_per_state: usize,
}

impl Default for AutonomousSettings {
    fn default() -> Self {
        Self {
            interval_secs: 300,
            todo_path: PathBuf::from("TODO.md"),
            status_path: PathBuf::from("autonomous-status.toml"),
            test_command: "cargo test --workspace".to_string(),
            git_remote: "origin".to_string(),
            git_branch: "agent-dev".to_string(),
            commit_message: "Auto-commit by operant-rs".to_string(),
            command_timeout_secs: 900,
            max_failures_per_state: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingSettings {
    pub level: String,
    pub format: String,
    pub log_file: Option<String>,
    pub with_target: bool,
    pub with_thread_ids: bool,
    pub with_file: bool,
    pub with_line_number: bool,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "pretty".to_string(),
            log_file: None,
            with_target: false,
            with_thread_ids: false,
            with_file: false,
            with_line_number: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TuiSettings {
    pub theme: String,
    pub rich_output: bool,
    /// P5-1 terminal-scrollback mode (2026-10-10 design doc): render in the
    /// main buffer with an inline bottom viewport — no alternate screen, so
    /// the terminal's native scrollback owns the history (tmux copy-mode and
    /// native selection reach everything). Default off until live-verified.
    #[serde(default)]
    pub terminal_scroll_mode: bool,
    pub show_tool_calls: bool,
    pub show_iterations: bool,
    pub landing_title: String,
    pub prompt_placeholder: String,
    /// Accepted for config compatibility (`deny_unknown_fields`) but unread:
    /// the TUI paces frames adaptively (`tui::redraw::redraw_interval`).
    pub refresh_rate_ms: u64,
    pub compact_width: u16,
    pub medium_width: u16,
    /// Command the composer opens with Ctrl+G (the prompt text is edited in
    /// a temp file passed as the last argument). Empty = `$VISUAL`, then
    /// `$EDITOR`, then `vi`. Arguments are whitespace-separated, e.g.
    /// `"code --wait"`.
    pub external_editor: String,
    /// (plan 05) What a DOUBLE Esc on an empty composer does when not
    /// streaming: "rewind" opens the fullscreen rewind selector (the
    /// default), "none" leaves Esc with its single-press meanings only.
    #[serde(default = "default_double_escape_action")]
    pub double_escape_action: String,
}

fn default_double_escape_action() -> String {
    "rewind".to_string()
}

impl Default for TuiSettings {
    fn default() -> Self {
        Self {
            theme: "opencode".to_string(),
            rich_output: true,
            terminal_scroll_mode: false,
            show_tool_calls: true,
            show_iterations: true,
            landing_title: "HERMES".to_string(),
            prompt_placeholder: "Ask anything... \"Fix a TODO in the codebase\"".to_string(),
            refresh_rate_ms: 80,
            compact_width: 96,
            medium_width: 120,
            external_editor: String::new(),
            double_escape_action: default_double_escape_action(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportKind {
    #[default]
    Http,
    Stdio,
    /// Streamable-HTTP transport (MCP spec 2025-06-18). Uses HTTP POST
    /// for client→server requests and SSE (Server-Sent Events) for
    /// server→client streaming. This is the modern recommended HTTP
    /// transport, superseding the old plain-POST HTTP + legacy SSE.
    /// (iter-137 — closes ponytail-audit gap B2.)
    StreamableHttp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportKind,
    pub url: Option<String>,
    pub auth_token: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub enabled: bool,
    /// Connect lazily: skip the eager autoload connect at agent startup.
    /// The server stays available on demand via `operant mcp` / the MCP
    /// tooling. Used for the injected agentmemory server so an operant
    /// invocation never spawns `npx @agentmemory/mcp` unless the user
    /// actually connects it (hermes lazy-MCP parity).
    pub deferred: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: McpTransportKind::Http,
            url: None,
            auth_token: None,
            command: None,
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            enabled: true,
            deferred: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpSettings {
    pub autoload: bool,
    /// Master switch for progressive tool disclosure of MCP tools (hermes
    /// `mcp.deferred_loading` parity). When `false`, MCP tool schemas are
    /// always loaded eagerly and `tools.tool_search.enabled` is forced to
    /// `"off"`. Defaults to `true` (defer behind the tool_search bridge
    /// when MCP tools are present).
    #[serde(default = "default_true")]
    pub deferred_loading: bool,
    /// Stdio MCP watchdog sweep interval in seconds (hermes
    /// `mcp_stdio_watchdog.py` parity). A background task periodically
    /// checks every connected stdio MCP server's child process and
    /// auto-reconnects one that has exited (crash), re-syncing its tools
    /// into the registry. `0` disables the watchdog. Default 30s.
    #[serde(default = "default_mcp_watchdog_interval")]
    pub watchdog_interval_secs: u64,
    pub servers: Vec<McpServerConfig>,
}

fn default_mcp_watchdog_interval() -> u64 {
    30
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            autoload: true,
            deferred_loading: true,
            watchdog_interval_secs: default_mcp_watchdog_interval(),
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SkillsSettings {
    pub root_dir: PathBuf,
    pub memory_dir: PathBuf,
    pub autoload: bool,
    pub template_name: String,
    pub template_description: String,
    /// Substitute `${OPERANT_SKILL_DIR}` / `${OPERANT_SESSION_ID}` in
    /// SKILL.md before injection (hermes `template_vars`, default true).
    #[serde(default = "default_skill_template_vars")]
    pub template_vars: bool,
    /// Expand `` !`cmd` `` inline-shell snippets in SKILL.md at load time
    /// (hermes `inline_shell`, default false — off unless opted in).
    #[serde(default)]
    pub inline_shell: bool,
    /// Timeout (seconds) for each inline-shell snippet.
    #[serde(default = "default_skill_inline_shell_timeout")]
    pub inline_shell_timeout: u64,
}

fn default_skill_template_vars() -> bool {
    true
}
fn default_skill_inline_shell_timeout() -> u64 {
    10
}

impl Default for SkillsSettings {
    fn default() -> Self {
        let root_dir = platform::operant_skills_dir();
        let memory_dir = root_dir.parent().unwrap_or(&root_dir).join("memory");
        Self {
            root_dir,
            memory_dir,
            autoload: true,
            template_name: "new-skill".to_string(),
            template_description: "Describe what this skill does.".to_string(),
            template_vars: true,
            inline_shell: false,
            inline_shell_timeout: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GatewaySettings {
    pub telegram_enabled: bool,
    pub telegram_token: Option<String>,
    pub telegram_api_base: String,
    pub discord_enabled: bool,
    pub discord_token: Option<String>,
    pub discord_api_base: String,
    pub slack_enabled: bool,
    pub slack_token: Option<String>,
    pub slack_api_base: String,
    pub whatsapp_enabled: bool,
    pub whatsapp_token: Option<String>,
    /// WhatsApp Cloud API phone number ID (Meta Business Manager).
    /// Required for outbound sends — without it the Graph API URL has no
    /// `phone_number_id` segment and returns 404. (R22)
    pub whatsapp_phone_number_id: Option<String>,
    pub email_enabled: bool,
    pub email_smtp_host: Option<String>,
    pub email_smtp_user: Option<String>,
    pub email_smtp_pass: Option<String>,
    pub webhooks_enabled: bool,
    pub webhooks_addr: Option<String>,
    /// Shared secret for HMAC-SHA256 verification of inbound webhook
    /// signatures (GitHub `x-hub-signature-256`, Stripe `Stripe-Signature`,
    /// Slack `x-slack-signature`, or the custom `x-webhook-signature` header).
    /// When set, unsigned or mismatched webhook requests are rejected.
    pub webhooks_secret: Option<String>,
    pub sms_twilio_enabled: bool,
    pub admins: Vec<String>,
    pub streaming_transport: String,
    /// HTTP/SOCKS5 proxy URL for Telegram API requests (e.g. "socks5://127.0.0.1:1080")
    pub telegram_proxy: Option<String>,
    /// Bot username for @mention detection in groups
    pub telegram_bot_username: Option<String>,
    /// Enable DM topic creation for private chats (Bot API 9.4+)
    pub telegram_dm_topics_enabled: bool,
    /// Cap on concurrent gateway sessions (hermes `max_concurrent_sessions`
    /// parity). When reached, new sessions get a refusal reply while existing
    /// holders keep their slots. `None` = unlimited.
    pub max_concurrent_sessions: Option<usize>,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            telegram_enabled: false,
            telegram_token: None,
            telegram_api_base: "https://api.telegram.org".to_string(),
            discord_enabled: false,
            discord_token: None,
            discord_api_base: "https://discord.com/api/v10".to_string(),
            slack_enabled: false,
            slack_token: None,
            slack_api_base: "https://slack.com/api".to_string(),
            whatsapp_enabled: false,
            whatsapp_token: None,
            whatsapp_phone_number_id: None,
            email_enabled: false,
            email_smtp_host: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            webhooks_enabled: false,
            webhooks_addr: None,
            webhooks_secret: None,
            sms_twilio_enabled: false,
            admins: Vec::new(),
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
            max_concurrent_sessions: None,
        }
    }
}

/// Plugin system configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginSettings {
    /// Directories to scan for plugin manifests (`plugin.toml` / `plugin.yaml`).
    pub plugin_dirs: Vec<PathBuf>,
    /// Signature enforcement: "disabled" (default), "permissive", or
    /// "strict". Parsed by `PluginHost::parse_signature_mode`. Applies to
    /// the tool bridge AND the kernel swap watcher, so the two can never
    /// disagree about whether code is verified.
    #[serde(default = "default_plugin_signature_mode")]
    pub signature_mode: String,
    /// Hex-encoded Ed25519 public keys of trusted plugin publishers.
    /// Required by both strict mode (tool bridge) and the kernel swap path
    /// (which always verifies — see `spawn_wasm_watcher`). Mirrors the
    /// schema world's `[plugins.security].trusted_publisher_keys`.
    #[serde(default)]
    pub trusted_publisher_keys: Vec<String>,
}

/// Signature-mode default; mirrors the schema world's
/// `[plugins.security].signature_mode` default.
fn default_plugin_signature_mode() -> String {
    "disabled".to_string()
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            plugin_dirs: vec![platform::operant_home().join("plugins")],
            signature_mode: default_plugin_signature_mode(),
            trusted_publisher_keys: Vec::new(),
        }
    }
}

/// Checkpoint (filesystem snapshot) settings.
///
/// Checkpoints are opt-in: when `enabled`, the agent snapshots working
/// directories into an isolated shadow git store under `base_dir` before
/// mutating operations, and the `checkpoint` tool becomes functional. Hermes
/// parity: hermes gates checkpoints behind a `checkpoints` config flag and
/// stores snapshots in an isolated git store so no git state leaks into the
/// user's project repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointsSettings {
    /// Whether filesystem checkpoints are enabled.
    pub enabled: bool,
    /// Base directory for the checkpoint shadow store.
    /// Defaults to `~/.operant/checkpoints`.
    pub base_dir: Option<PathBuf>,
    /// Maximum snapshots kept per working directory.
    pub max_snapshots: usize,
}

/// Plan 016 / Phase 0 config surface for the `operant-harness` kernel.
/// Dark-merged: `enabled` defaults to `false` and no provider is mounted
/// unless the user opts in. Phase 2+ wires the kernel into the live
/// `ToolRegistry` / `HookRunner` behind this flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessSettings {
    /// Master switch. When `false` (the default), the harness is
    /// constructed but no provider is mounted — the live agent loop
    /// is unaffected. Set to `true` once a Phase 2 dispatch path is
    /// plumbed and the composition layer is loaded.
    pub enabled: bool,
    /// Optional path to an `architecture.toml` (Phase 3). When unset,
    /// the kernel starts empty. The path is resolved relative to
    /// `~/.operant/` if relative.
    pub architecture_toml: Option<PathBuf>,
    /// Cap on the number of simultaneously active providers. Default
    /// 64. Mirrors the 016 plan's "kernel" guardrail; keeps a single
    /// `Harness` instance from absorbing unbounded WASM modules.
    pub max_active_providers: usize,
    /// C2 — watch the plugin directory and hot-swap verified WASM
    /// providers into the kernel. Default `false`: the watcher is a
    /// permanent background poll (and every scan re-instantiates the
    /// module), so it must be an explicit opt-in rather than a surprise
    /// cost on every harness-enabled boot. Requires the plugin host to run
    /// in `SignatureMode::Strict` with trusted keys.
    #[serde(default)]
    pub watch_wasm: bool,
}

impl Default for HarnessSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            architecture_toml: None,
            max_active_providers: 64,
            watch_wasm: false,
        }
    }
}

/// Plan 015 / Phase 0 config surface for the Prime Kernel sidecar.
/// Dark-merged: `enabled` defaults to `false`; the sidecar is not even
/// spawned until a Phase 1+ rollout flips this and provides a working
/// `sidecar_script` path. The fields are independent so Phase 1 can
/// light up one knob at a time (e.g. just `enabled` first, the
/// `ndjson_socket` later when the supervisor lands).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PkSettings {
    /// Master switch. When `false` (the default), the sidecar is
    /// never spawned; the existing `code_execution` tempfile path is
    /// the only Python surface.
    pub enabled: bool,
    /// Path to the sidecar entrypoint (the supervisor / NDJSON server).
    /// Defaults to `pk-sidecar/main.py` relative to the operant repo
    /// root. Phase 1 will switch this to a vendored python venv.
    pub sidecar_script: Option<PathBuf>,
    /// Unix socket path the supervisor binds to (Phase 1). When `None`,
    /// the supervisor picks a default under `~/.operant/run/pk.sock`.
    pub ndjson_socket: Option<PathBuf>,
    /// Maximum wall-clock seconds an `exec` call may take before the
    /// supervisor kills the kernel. Default 30. Per the 015 plan the
    /// kernel reuses this; we expose it for the operator.
    pub exec_timeout_secs: u64,
}

impl Default for PkSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            sidecar_script: None,
            ndjson_socket: None,
            exec_timeout_secs: 30,
        }
    }
}

impl Default for CheckpointsSettings {
    fn default() -> Self {
        Self {
            // On by default: the checkpoint tool is registered for every
            // agent, and an inert-by-default tool confused the live loop
            // ("Disabled in config"). Snapshots live in an isolated shadow
            // git store under ~/.operant/checkpoints — zero git-state leak
            // into the user's project — so the safety net costs nothing.
            enabled: true,
            base_dir: None,
            max_snapshots: 20,
        }
    }
}

/// Terminal execution backends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TerminalBackend {
    #[default]
    Local,
    Docker,
    Modal,
    Ssh,
    Daytona,
    VercelSandbox,
    Singularity,
}

impl std::fmt::Display for TerminalBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Docker => write!(f, "docker"),
            Self::Modal => write!(f, "modal"),
            Self::Ssh => write!(f, "ssh"),
            Self::Daytona => write!(f, "daytona"),
            Self::VercelSandbox => write!(f, "vercel_sandbox"),
            Self::Singularity => write!(f, "singularity"),
        }
    }
}

impl std::str::FromStr for TerminalBackend {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "docker" => Ok(Self::Docker),
            "modal" => Ok(Self::Modal),
            "ssh" => Ok(Self::Ssh),
            "daytona" => Ok(Self::Daytona),
            "vercel_sandbox" => Ok(Self::VercelSandbox),
            "singularity" => Ok(Self::Singularity),
            _ => Err(format!("Unknown terminal backend: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolSettings {
    pub registry_timeout_secs: u64,
    pub event_channel_size: usize,
    /// Accepted but ignored: the only local browser backend is `sourcehound`,
    /// which is resolved from `SOURCEHOUND_BINARY` / `PATH`, not from a path in
    /// the config. Kept so an existing `operant.toml` still parses.
    pub browser_binary_path: Option<PathBuf>,
    pub web: WebToolSettings,
    pub http: HttpToolSettings,
    pub terminal: TerminalSettings,
    pub code_execution: CodeExecutionSettings,
    /// Prime Kernel (plan 015): persistent stateful Python kernel + continual
    /// harness, hosted in the kernel-sidecar subprocess over NDJSON stdio.
    #[serde(alias = "prime_kernel")]
    pub kernel: KernelSettings,
    pub stt: SttSettings,
    pub disabled_tools: Vec<String>,
    pub disabled_toolsets: Vec<String>,
    /// Whether to enable AFT (Agent File Tools) IDE-grade coding tools.
    /// When true, operant registers 18 aft_* tools that communicate with
    /// an aft subprocess (auto-downloaded from GitHub releases when not
    /// installed). Defaults to true; the CLI verifies the bridge is live
    /// before hiding the native file/terminal tools, so a broken/missing
    /// aft always falls back to the built-in tools.
    #[serde(default = "default_true")]
    pub aft_enabled: bool,
    // NOTE: `lifeos_enabled` used to live here. It was removed, together with
    // its line in `operant.example.toml`, because it gated nothing: a
    // repo-wide search found the identifier in exactly three places — this
    // struct, its own `Default`, and the example file — and no LifeOS tool
    // module or `lifeos` cargo feature exists in the tree to gate. A flag
    // that reads as a switch for 22 Notion-backed tools but reads back
    // whatever the user typed is worse than an absent key: it invites the
    // belief that the tools are present and gated when neither is true.
    // (`docs/CHANGELOG.md` and `AGENTS.md` still describe LifeOS as a
    // feature-gated integration; that documentation drift is pre-existing and
    // was left alone here.) Locked by `lifeos_key_is_gone_from_config`.
    /// Progressive tool disclosure (hermes `tools.tool_search` parity).
    ///
    /// When active, MCP server tools (`mcp_*` names) are replaced in the
    /// model-visible tools array by three bridge tools — `tool_search`,
    /// `tool_describe`, `tool_call` — and surfaced on demand. Native
    /// builtin tools never defer; Tier 0 (no MCP tools) is a pure
    /// passthrough. See `tools/tool_search.rs` for the full design.
    #[serde(default)]
    pub tool_search: ToolSearchSettings,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolSearchSettings {
    /// `"auto"` | `"on"` | `"off"`. `"auto"` (default) activates the
    /// bridge only when deferrable (MCP) tools are present — a pure
    /// passthrough otherwise, so zero-behavior-change for native-only
    /// configs. `"on"` forces the bridge; `"off"` disables it even when
    /// MCP tools exist.
    #[serde(default = "default_tool_search_enabled")]
    pub enabled: String,
    /// Listing budget as a percentage of the model's context window that
    /// the embedded catalog listing may consume before disclosure degrades
    /// (full listing -> names-only -> bare bridge). Default 5.0.
    #[serde(default = "default_tool_search_threshold_pct")]
    pub threshold_pct: f64,
    /// `"auto"` | `"on"` | `"off"` — whether the `tool_search` bridge
    /// description embeds a skills-style catalog listing of every deferred
    /// tool (name + short description). `"auto"` = include when it fits
    /// the listing budget, else names-only, else none (bare bridge).
    #[serde(default = "default_tool_search_listing")]
    pub listing: String,
    /// Absolute cap on the embedded listing, regardless of context size.
    /// Effective budget = min(listing_max_tokens, threshold_pct% of
    /// context). Default 4000.
    #[serde(default = "default_tool_search_listing_max_tokens")]
    pub listing_max_tokens: usize,
    /// Default `limit` for `tool_search` when the model omits it. Default 10.
    #[serde(default = "default_tool_search_default_limit")]
    pub search_default_limit: usize,
    /// Hard cap on a single `tool_search` `limit`. Default 25.
    #[serde(default = "default_tool_search_max_limit")]
    pub max_search_limit: usize,
}

fn default_tool_search_enabled() -> String {
    "auto".to_string()
}

fn default_tool_search_threshold_pct() -> f64 {
    5.0
}

fn default_tool_search_listing() -> String {
    "auto".to_string()
}

fn default_tool_search_listing_max_tokens() -> usize {
    4000
}

fn default_tool_search_default_limit() -> usize {
    10
}

fn default_tool_search_max_limit() -> usize {
    25
}

impl Default for ToolSearchSettings {
    fn default() -> Self {
        Self {
            enabled: default_tool_search_enabled(),
            threshold_pct: default_tool_search_threshold_pct(),
            listing: default_tool_search_listing(),
            listing_max_tokens: default_tool_search_listing_max_tokens(),
            search_default_limit: default_tool_search_default_limit(),
            max_search_limit: default_tool_search_max_limit(),
        }
    }
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            registry_timeout_secs: 30,
            event_channel_size: 100,
            browser_binary_path: None,
            web: WebToolSettings::default(),
            http: HttpToolSettings::default(),
            terminal: TerminalSettings::default(),
            code_execution: CodeExecutionSettings::default(),
            kernel: KernelSettings::default(),
            stt: SttSettings::default(),
            disabled_tools: Vec::new(),
            disabled_toolsets: Vec::new(),
            aft_enabled: true,
            tool_search: ToolSearchSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SttSettings {
    pub groq_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub groq_model: String,
    pub openai_model: String,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            groq_api_key: None,
            openai_api_key: None,
            groq_model: "whisper-large-v3-turbo".to_string(),
            openai_model: "whisper-1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TtsSettings {
    /// TTS provider: kokoro|edge|elevenlabs|openai|minimax|mistral|gemini|xai|neutts|kittentts|piper
    pub provider: String,
    pub enabled: bool,
    /// Default voice for the selected provider (provider-specific identifier)
    pub voice: Option<String>,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            provider: "kokoro".to_string(),
            enabled: false,
            voice: None,
        }
    }
}

/// Memory provider configuration.
///
/// (plan 08) memory-wire is the one engine (in-process, ~/.operant/
/// memory_wire.sqlite). The `provider` key is accepted so existing files
/// keep loading; anything but the memory-wire family is ignored with a
/// one-line warning. The retired agentmemory keys stay accepted the same
/// way — `deny_unknown_fields` is on, and a config that parsed yesterday
/// must parse today.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemorySettings {
    /// Memory provider: memory-wire (the only engine; other names warn)
    pub provider: String,
    /// Whether long-term memory is enabled at all
    pub enabled: bool,
    /// (retired, plan 08) agentmemory server base URL — accepted, ignored.
    pub agentmemory_url: Option<String>,
    /// (retired, plan 08) agentmemory REST secret — accepted, ignored.
    pub agentmemory_secret: Option<String>,
    /// (retired, plan 08) agentmemory auto-spawn — accepted, ignored.
    pub agentmemory_auto_spawn: Option<bool>,
    /// (retired, plan 08) npm version pin — accepted, ignored.
    pub agentmemory_version: Option<String>,
    /// (retired, memory-factory era) `[memory] audit_enabled` — accepted, ignored.
    /// Without this field, `deny_unknown_fields` failed the whole config load.
    pub audit_enabled: Option<bool>,
    /// (retired, memory-factory era) `[memory] response_cache_enabled` — accepted, ignored.
    pub response_cache_enabled: Option<bool>,
}

impl MemorySettings {
    /// (plan 08) the retired-key warning contract: DEFAULTS ARE SILENT.
    /// A warning fires only for a value the file actually carries (the
    /// deserialized struct cannot tell "present" from "defaulted", so the
    /// retired keys default to `None`/`Some(false)` — the loud legacy
    /// default `Some(true)` is gone and every clean start is quiet).
    pub fn warn_legacy_keys(&self) {
        for line in self.legacy_warnings() {
            eprintln!("{line}");
        }
    }

    /// (plan 08) the warning contract as data — defaults ARE SILENT. A
    /// warning fires only for a value the file actually carries; the
    /// deserialized struct cannot tell "present" from "defaulted", so
    /// the retired agentmemory keys default to `None` — the loud legacy
    /// default `Some(true)` is gone and every clean start is quiet. The
    /// test pins this: `MemorySettings::default().legacy_warnings()` is
    /// empty.
    pub fn legacy_warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !matches!(
            self.provider.as_str(),
            "memory-wire" | "memory_wire" | "memory" | ""
        ) {
            out.push(format!(
                "warning: [memory] provider = {:?} is ignored — memory-wire is the only memory engine now. Remove the key from operant.toml.",
                self.provider
            ));
        }
        if self.agentmemory_auto_spawn.unwrap_or(false) {
            out.push(
                "warning: [memory] agentmemory_auto_spawn is removed — the agentmemory server era is over. Remove the key from operant.toml."
                    .to_string(),
            );
        }
        if self.audit_enabled.is_some() {
            out.push(
                "warning: [memory] audit_enabled is removed — audit logging is not wired into the live memory path. Remove the key from operant.toml."
                    .to_string(),
            );
        }
        if self.response_cache_enabled.is_some() {
            out.push(
                "warning: [memory] response_cache_enabled is removed — the response cache is not wired into the live memory path. Remove the key from operant.toml."
                    .to_string(),
            );
        }
        out
    }
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            provider: "memory-wire".to_string(),
            enabled: true,
            agentmemory_url: None,
            agentmemory_secret: None,
            agentmemory_auto_spawn: None,
            agentmemory_version: None,
            audit_enabled: None,
            response_cache_enabled: None,
        }
    }
}

/// Browser provider configuration.
///
/// `provider` selects the browser backend:
/// - `"sourcehound"` (default) — the sourcehound browser engine driven over
///   its MCP server (`cloakctl.navigate` / `cloakctl.read` / `cloakctl.act`)
/// - `"camofox"` — local anti-detection browser REST API (`CAMOFOX_URL`)
/// - `"browserbase"` — Browserbase cloud (`BROWSERBASE_API_KEY` + `BROWSERBASE_PROJECT_ID`)
/// - `"browser-use"` — Browser Use cloud agent (`BROWSER_USE_API_KEY`)
/// - `"firecrawl"` — Firecrawl scrape API (`FIRECRAWL_API_KEY`)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserSettings {
    /// Browser provider name — (plan 11C2) accepted for config
    /// compatibility and IGNORED: sourcehound is the only engine. A
    /// non-default value warns once at load.
    pub provider: String,
}

impl BrowserSettings {
    /// (plan 11C2 migration) warn once when a non-sourcehound provider is
    /// configured — the value is accepted but ignored.
    pub fn warn_legacy_keys(&self) {
        if !self.provider.eq_ignore_ascii_case("sourcehound") {
            eprintln!(
                "warning: [browser] provider = {:?} is ignored — sourcehound is the only browser engine now. Remove the key from operant.toml.",
                self.provider
            );
        }
    }
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            provider: "sourcehound".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct VisionSettings {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct CredentialPoolSettings {
    /// Global default selection strategy (fill_first|round_robin|random|least_used).
    pub strategy: Option<String>,
    /// Master switch — when false no credential pool is attached to agents.
    pub enabled: bool,
    /// Per-provider selection strategies (hermes `credential_pool_strategies`
    /// parity). Keys are provider names as inferred from the model (e.g.
    /// `opencode-zen`, `openai`, `anthropic`); each entry overrides the global
    /// `strategy` for that provider. Unknown or unparseable values fall back
    /// to `fill_first` (hermes `get_pool_strategy()` semantics).
    pub strategies: HashMap<String, String>,
}

impl CredentialPoolSettings {
    /// Resolve the pool selection strategy for a provider.
    ///
    /// Mirrors hermes-agent's `get_pool_strategy()`: per-provider
    /// `strategies` entry first, then the global `strategy`, defaulting to
    /// `fill_first` when nothing (or an unknown value) is configured.
    pub fn strategy_for(&self, provider: &str) -> crate::credential_pool::PoolStrategy {
        self.strategies
            .get(provider)
            .or(self.strategy.as_ref())
            .map(|s| crate::credential_pool::PoolStrategy::parse_strategy(s))
            .unwrap_or_default()
    }
}

/// Configuration for an auxiliary model assigned to a specific task slot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuxiliaryModelConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

/// Mixture-of-Agents (MoA) configuration (`[moa]` section, hermes
/// `hermes_cli/moa_config.py` + `agent/moa_loop.py` parity).
///
/// When `enabled`, a MoA turn fans out `references` (advisory models) over
/// the flattened conversation, then an `aggregator` model synthesizes their
/// advice into guidance that is injected into the acting agent's context
/// before it answers. Off by default — MoA costs N+1 LLM calls per turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct MoaSettings {
    /// Master switch. When false (default), MoA turns are a no-op.
    pub enabled: bool,
    /// Reference (advisor) models. Each is called with the flattened
    /// conversation plus an advisory system prompt; their outputs feed the
    /// aggregator. Empty (default) → MoA is inert even when enabled.
    pub references: Vec<AuxiliaryModelConfig>,
    /// Aggregator model that synthesizes the reference advice into concise
    /// guidance for the acting agent. When unset, the main agent model is
    /// used with the main provider client.
    pub aggregator: Option<AuxiliaryModelConfig>,
    /// Optional per-reference output cap (tokens). None → the model's own
    /// maximum. The aggregator synthesis is NEVER capped (hermes parity).
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Optional temperature for reference and aggregator calls. None → the
    /// provider default applies.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Per-call timeout in seconds for reference/aggregator calls
    /// (default 60).
    #[serde(default = "default_moa_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_moa_timeout_secs() -> u64 {
    60
}

/// Auxiliary model routing for specialized task slots.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuxiliaryModels {
    pub vision: Option<AuxiliaryModelConfig>,
    pub compression: Option<AuxiliaryModelConfig>,
    /// LLM post-processing slot for `web_extract`. Currently **inert**: the
    /// IGS-backed `web_extract` tool returns raw markdown directly (no LLM
    /// summarization step), so this slot only takes effect once an
    /// LLM-backed extractor is wired to it.
    pub web_extract: Option<AuxiliaryModelConfig>,
    pub image_gen: Option<AuxiliaryModelConfig>,
    pub embeddings: Option<AuxiliaryModelConfig>,
    pub search: Option<AuxiliaryModelConfig>,
    pub memory: Option<AuxiliaryModelConfig>,
    pub code_execution: Option<AuxiliaryModelConfig>,
    pub reasoning: Option<AuxiliaryModelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebToolSettings {
    pub search_timeout_secs: u64,
    pub fetch_timeout_secs: u64,
    pub user_agent: String,
    pub default_results: usize,
    pub max_results: usize,
    // ── (plan 11C1) retired provider keys, kept parsing for migration ──
    // Web search is sourcehound-only now. These fields exist ONLY so an
    // existing user config still loads (`deny_unknown_fields` would
    // otherwise fail on them); they are serialized away, read by no code,
    // and `warn_legacy_keys` names the replacement once per load.
    #[serde(skip_serializing)]
    pub search_url: Option<String>,
    #[serde(skip_serializing)]
    pub preferred_provider: Option<String>,
    #[serde(skip_serializing)]
    pub tavily_api_key: Option<String>,
    #[serde(skip_serializing)]
    pub exa_api_key: Option<String>,
    #[serde(skip_serializing)]
    pub searxng_base_url: Option<String>,
}

impl WebToolSettings {
    /// One warning per removed key with a value set (plan 11C1 migration).
    pub fn warn_legacy_keys(&self) {
        let pairs = [
            ("search_url", self.search_url.as_deref(), None::<&str>),
            (
                "preferred_provider",
                self.preferred_provider.as_deref(),
                Some("sourcehound"),
            ),
            ("tavily_api_key", self.tavily_api_key.as_deref(), None),
            ("exa_api_key", self.exa_api_key.as_deref(), None),
            ("searxng_base_url", self.searxng_base_url.as_deref(), None),
        ];
        for (key, value, replacement) in pairs {
            if value.is_some_and(|v| !v.trim().is_empty()) {
                let hint = replacement
                    .map(|_| " — web search is sourcehound-only now".to_string())
                    .unwrap_or_else(|| {
                        " — removed; web search is sourcehound-only now".to_string()
                    });
                eprintln!(
                    "warning: [tools.web] {key} is ignored{hint}. Remove it from operant.toml."
                );
            }
        }
    }
}

impl Default for WebToolSettings {
    fn default() -> Self {
        Self {
            search_timeout_secs: 15,
            fetch_timeout_secs: 30,
            user_agent: "Mozilla/5.0 (compatible; OperantAgent/0.1)".to_string(),
            default_results: 10,
            max_results: 20,
            search_url: None,
            preferred_provider: None,
            tavily_api_key: None,
            exa_api_key: None,
            searxng_base_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HttpToolSettings {
    pub timeout_secs: u64,
}

impl Default for HttpToolSettings {
    fn default() -> Self {
        Self { timeout_secs: 30 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalSettings {
    pub max_timeout_secs: u64,
    pub max_output_bytes: usize,
    pub docker: DockerTerminalSettings,
    pub ssh: SshTerminalSettings,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            max_timeout_secs: 300,
            max_output_bytes: 1_000_000,
            docker: DockerTerminalSettings::default(),
            ssh: SshTerminalSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DockerTerminalSettings {
    pub image: Option<String>,
    pub volumes: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    pub cpu: f64,
    pub memory_mb: u64,
}

impl Default for DockerTerminalSettings {
    fn default() -> Self {
        Self {
            image: Some("nikolaik/python-nodejs:python3.11-nodejs20".to_string()),
            volumes: Vec::new(),
            env: std::collections::HashMap::new(),
            cpu: 1.0,
            memory_mb: 5120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshTerminalSettings {
    pub host: Option<String>,
    pub user: Option<String>,
    pub port: u16,
    pub key_path: Option<String>,
}

impl Default for SshTerminalSettings {
    fn default() -> Self {
        Self {
            host: None,
            user: None,
            port: 22,
            key_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodeExecutionSettings {
    pub default_timeout_secs: u64,
    pub max_timeout_secs: u64,
}

impl Default for CodeExecutionSettings {
    fn default() -> Self {
        Self {
            default_timeout_secs: 60,
            max_timeout_secs: 300,
        }
    }
}

/// Phase-2.5 tool bridge: which tools model-authored kernel programs may call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KernelToolBridge {
    /// Master switch for `operant_tool()` availability inside kernel cells.
    pub enabled: bool,
    /// Deny-by-default allowlist of tool names callable from kernel programs.
    /// Bridged calls re-enter `check_tool_approval` with the TARGET tool's
    /// name/args — a program can never launder permissions a direct call
    /// would not have. Blocked/requires-approval verdicts return an error
    /// VALUE to the kernel (no human-in-loop channel mid-cell in v1).
    pub allowlist: Vec<String>,
    /// Hard cap on bridged calls per executed cell; excess yields structured
    /// partial results so long sweeps degrade instead of hanging.
    pub max_calls_per_exec: usize,
    pub per_call_timeout_secs: u64,
}

impl Default for KernelToolBridge {
    fn default() -> Self {
        // Read-only discovery set first; writes are deliberate opt-ins.
        let allowlist = vec![
            "file_list".into(),
            "file_read".into(),
            "file_search".into(),
            "web_search".into(),
            "web_scrape".into(),
            "http_request".into(),
            "datetime".into(),
            "session_insights".into(),
        ];
        Self {
            enabled: false,
            allowlist,
            max_calls_per_exec: 64,
            per_call_timeout_secs: 60,
        }
    }
}

/// Executable skills (plan 016, phase 6b): controls SKILL.toml [reference]
/// binding into the kernel. Default-off; allowlist is deny-by-default.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct KernelPyskill {
    /// Master switch for executable skill binding (default false).
    pub enabled: bool,
    /// Allowlist of import prefixes callable from SKILL.toml [reference].
    /// Supports glob `*` (e.g. `"my_pkg.*"`) and exact matches.
    /// Empty (default) ⇒ every reference is denied.
    pub allowed_imports: Vec<String>,
}

/// Persistent-kernel sidecar configuration (plan 015). Everything defaults
/// 017-A: `enabled` is the single gate for the unified python path.
/// `route_python_to_kernel` is deprecated (kept as alias so old configs
/// parse, but no longer gates anything).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KernelSettings {
    pub enabled: bool,
    /// Python ≥3.11 for the sidecar. None ⇒ platform::find_python() then PATH.
    pub python: Option<PathBuf>,
    /// vendored submodule root containing rlm runtime/src/rlm.
    /// None ⇒ `<repo>`/vendor/prime-agent resolved from CARGO_MANIFEST_DIR at
    /// build time with a ./vendor fallback.
    pub vendor_dir: Option<PathBuf>,
    /// Harness store root (`<state_dir>/<scope>/`). None ⇒
    /// ~/.local/share/operant/kernel/harness.
    pub state_dir: Option<PathBuf>,
    pub sidecar_idle_secs: u64,
    pub request_timeout_secs: u64,
    pub max_output_bytes: usize,
    /// Deprecated since 017-A: no longer gates anything. Kept so old
    /// `route_python_to_kernel = true` configs still parse. `enabled`
    /// is the sole gate since 017-A.
    #[serde(default)]
    pub route_python_to_kernel: bool,
    /// Phase 5: when true AND kernel.enabled, the existing background
    /// review fork may additionally emit prompt/subagent harness edits via
    /// kernel_refine. Default off until soaked.
    pub harness_auto_learn: bool,
    /// Session GC — prune per-session harness dirs older than this many hours
    /// (0 = disabled). Default 168h (7 days).
    pub session_gc_ttl_hours: u64,
    /// Executable skills (plan 016, phase 6b): SKILL.toml [reference] binding.
    #[serde(default)]
    pub pyskill: KernelPyskill,
    pub tool_bridge: KernelToolBridge,
}

impl Default for KernelSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            python: None,
            vendor_dir: None,
            state_dir: None,
            sidecar_idle_secs: 1800,
            request_timeout_secs: 120,
            max_output_bytes: 200_000,
            route_python_to_kernel: false,
            harness_auto_learn: false,
            session_gc_ttl_hours: 168,
            pyskill: KernelPyskill::default(),
            tool_bridge: KernelToolBridge::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: AppConfig,
    pub source: Option<PathBuf>,
}

pub fn install_runtime_config(config: AppConfig) {
    let store = RUNTIME_CONFIG.get_or_init(|| RwLock::new(AppConfig::default()));
    if let Ok(mut current) = store.write() {
        *current = config;
    }
}

pub fn runtime_config() -> AppConfig {
    let store = RUNTIME_CONFIG.get_or_init(|| RwLock::new(AppConfig::default()));
    store
        .read()
        .map(|config| config.clone())
        .unwrap_or_else(|_| AppConfig::default())
}

pub fn load_app_config(explicit: Option<&Path>) -> Result<LoadedConfig> {
    let (mut config, source) = if let Some(path) = explicit {
        if !path.exists() {
            return Err(Error::Config(format!(
                "Config file '{}' was not found. Pass a valid --config path or create operant.toml.",
                path.display()
            )));
        }
        (parse_config_file(path)?, Some(path.to_path_buf()))
    } else {
        let mut found = None;
        for path in default_config_paths() {
            if path.exists() {
                found = Some(path);
                break;
            }
        }
        match found {
            Some(path) => {
                // Plan 002: auto-tighten pre-R42 loose config files on read.
                let _ = crate::fs_secrets::tighten_if_loose(&path);
                (parse_config_file(&path)?, Some(path))
            }
            None => {
                // Nothing exists yet: fall back to defaults, but when
                // OPERANT_CONFIG_DIR is set, still claim that isolated path
                // as the config source so the `config_manage` tool and any
                // persistence (e.g. `operant channel add`) target the
                // isolated config home instead of the real `~/.operant`
                // config. A later explicit persist creates the file there.
                let source = operant_config_dir_override().map(|dir| dir.join("operant.toml"));
                (AppConfig::default(), source)
            }
        }
    };

    // Native-MCP registration: when the agentmemory memory provider is
    // active, ensure an `agentmemory` stdio MCP server exists in the config
    // so EVERY agent-construction path (CLI registry, runtime-agent
    // connect_all, gateway orchestrator) exposes the full 53-tool memory
    // surface. Users who configured their own server (or deliberately
    // disabled it) keep their entry untouched.

    // 018-U3: single harness — [harness].enabled and [tools.kernel].enabled
    // are now aliases for the same master switch. OR so either being true
    // enables the unified harness (one store, one injection). One release
    // as alias, then [tools.kernel].enabled is removed.
    if config.harness.enabled || config.tools.kernel.enabled {
        config.harness.enabled = true;
        config.tools.kernel.enabled = true;
    }

    // (plan 11C1) one-time migration warnings for the retired web-search
    // provider keys — the config still parses, the user is told once.
    config.tools.web.warn_legacy_keys();
    // (plan 11C2) [browser] provider is accepted-and-ignored now.
    config.browser.warn_legacy_keys();
    // (plan 11C3) [memory] provider is accepted-and-ignored now.
    config.memory.warn_legacy_keys();

    // O-26: [holon] paths arrive with `~` expanded exactly once.
    config.holon = config.holon.expand_tilde();

    // (plan 09) Record the file this process actually loaded — the one
    // `persist_config_key` writes into. Without it, every writer guessed
    // `~/.operant/operant.toml` and `operant -c <path>` runs wrote the
    // wrong file.
    if let Ok(mut slot) = LOADED_CONFIG_PATH.write() {
        *slot = source.clone();
    }

    Ok(LoadedConfig { config, source })
}
/// The config file this process actually loaded (`-c <path>`, the config-dir
/// override, or the discovered default). `None` until `load_app_config` runs,
/// or when no file exists at all — persistence then has no target, which
/// callers must surface instead of guessing a path.
static LOADED_CONFIG_PATH: std::sync::LazyLock<RwLock<Option<PathBuf>>> =
    std::sync::LazyLock::new(|| RwLock::new(None));

/// The config file the current process loaded — the one (and only one)
/// `persist_config_key` writes into.
pub fn loaded_config_path() -> Option<PathBuf> {
    LOADED_CONFIG_PATH
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

// ── (plan 09) Surgical config writes ────────────────────────────────────────────

/// Coerce a CLI-style raw string into the TOML scalar it means — `true`,
/// `42`, `3.5`, else a (quoted) string. Same coercion every setter used
/// ad hoc; now one copy, used by the leaf parser and the tests.
fn coerce_scalar(raw: &str) -> toml_edit::Item {
    let raw = raw.trim();
    if raw == "true" || raw == "false" {
        return toml_edit::value(raw == "true");
    }
    if let Ok(int) = raw.parse::<i64>() {
        return toml_edit::value(int);
    }
    if let Ok(float) = raw.parse::<f64>() {
        return toml_edit::value(float);
    }
    toml_edit::value(raw)
}

/// Set one dotted key (`agent.model`) in the config file the process
/// actually loaded, keeping every comment, every layout choice, and every
/// OTHER key byte-for-byte (2026-10-16 audit plan 09: the old writers
/// re-serialized the whole `AppConfig`, which stripped comments, resurrected
/// retired keys from the defaults, clobbered `client.base_url` with a
/// provider-guessed URL, and wrote env-sourced secrets to disk).
///
/// Validation: after writing, the file re-parses through the normal
/// loader; on failure the previous bytes are restored and the error is
/// returned (a typo from `operant config set` can never leave an
/// unloadable config behind). Returns the path written.
pub fn persist_config_key(dotted: &str, raw_value: &str) -> Result<PathBuf> {
    let path = loaded_config_path().ok_or_else(|| {
        crate::error::Error::Config(
            "no config file is loaded in this process; nothing to persist into".to_string(),
        )
    })?;
    let original = std::fs::read(&path)
        .map_err(|e| crate::error::Error::Config(format!("read {}: {e}", path.display())))?;

    let mut doc: toml_edit::DocumentMut = std::str::from_utf8(&original)
        .map_err(|e| crate::error::Error::Config(format!("{}: not UTF-8: {e}", path.display())))?
        .parse()
        .map_err(|e| {
            crate::error::Error::Config(format!("{}: not valid TOML: {e}", path.display()))
        })?;

    let segments: Vec<&str> = dotted.split('.').collect();
    if segments.is_empty() || segments.iter().any(|s| s.is_empty()) {
        return Err(crate::error::Error::Config(format!(
            "config key '{dotted}' must be a non-empty dotted path"
        )));
    }
    let (tables, leaf) = segments.split_at(segments.len() - 1);
    let mut table = doc.as_table_mut();
    for segment in tables {
        let entry = table
            .entry(segment)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        table = entry.as_table_mut().ok_or_else(|| {
            crate::error::Error::Config(format!(
                "config key '{dotted}': '{segment}' is not a table in {}",
                path.display()
            ))
        })?;
    }
    // Array literals (the setup wizard's `fallback_models`, for example)
    // arrive as raw TOML like `["a", "b"]` — parse them as the value they
    // are; plain strings fail this parse and fall through to the scalar
    // coercion (bare multi-word text is not a valid TOML value).
    let item = match raw_value.trim().parse::<toml_edit::Item>() {
        Ok(item) if !item.is_none() => item,
        _ => coerce_scalar(raw_value),
    };
    table.insert(leaf[0], item);

    // Write to a sibling temp file with the original's mode, validate it
    // through the REAL loader, and only then rename over — the user's
    // config is never left broken by a typo from `operant config set`, and
    // a reader never sees a half-written file. The mode (0600 where
    // secrets live by design) must survive the round-trip.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())
        .map_err(|e| crate::error::Error::Config(format!("write {}: {e}", tmp.display())))?;
    if let Ok(mode) = std::fs::metadata(&path).map(|m| m.permissions()) {
        let _ = std::fs::set_permissions(&tmp, mode);
    }
    if let Err(e) = parse_config_file(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(crate::error::Error::Config(format!(
            "config key '{dotted}' was rejected by the config loader ({e}); \
             {} left unchanged",
            path.display()
        )));
    }
    std::fs::rename(&tmp, &path)
        .map_err(|e| crate::error::Error::Config(format!("rename {}: {e}", path.display())))?;
    Ok(path)
}

/// Best-effort coercion of a CLI string argument into a typed JSON value.
/// Precedence: boolean → integer → float → string. The one copy (plan 09):
/// `operant config set` and the agent's config tool both walk through it.
pub fn coerce_json_value(raw: &str) -> serde_json::Value {
    match raw.to_ascii_lowercase().as_str() {
        "true" => return serde_json::Value::Bool(true),
        "false" => return serde_json::Value::Bool(false),
        _ => {}
    }
    if let Ok(n) = raw.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(f) = raw.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return serde_json::Value::Number(n);
    }
    serde_json::Value::String(raw.to_string())
}

/// Recursively navigate into a nested `serde_json::Value` object and set
/// the leaf value — the in-memory twin of `persist_config_key`, used by
/// `operant config set` and the agent's config tool (the config tool's
/// documented contract stays: in-memory only, never persisted).
pub fn set_nested_json(target: &mut serde_json::Value, segments: &[&str], raw: &str) -> Result<()> {
    if segments.is_empty() {
        return Err(crate::error::Error::Agent(
            "config key must not be empty".into(),
        ));
    }
    let head = segments[0];

    if segments.len() == 1 {
        let parsed = coerce_json_value(raw);
        let obj = target.as_object_mut().ok_or_else(|| {
            crate::error::Error::Agent(format!("cannot index into non-object with '{head}'"))
        })?;
        obj.insert(head.to_string(), parsed);
        return Ok(());
    }

    let tail = &segments[1..];
    let obj = target.as_object_mut().ok_or_else(|| {
        crate::error::Error::Agent(format!("cannot index into non-object with '{head}'"))
    })?;

    if !obj.contains_key(head) {
        obj.insert(
            head.to_string(),
            serde_json::Value::Object(serde_json::Map::new()),
        );
    }
    let child = obj
        .get_mut(head)
        .ok_or_else(|| crate::error::Error::Agent(format!("could not access key '{head}'")))?;

    set_nested_json(child, tail, raw)
}

/// The `OPERANT_CONFIG_DIR` override (if set and non-empty), tilde-expanded.
///
/// Mirrors the schema layer's `default_config_dir()` so EVERY load path —
/// CLI `run`/`gateway`/`cron`, the `config_manage` tool, and the gateway
/// service — resolves to the same isolated config the user asked for
/// instead of silently falling back to `~/.operant/operant.toml`.
pub(crate) fn operant_config_dir_override() -> Option<PathBuf> {
    let custom = std::env::var("OPERANT_CONFIG_DIR").ok()?;
    let custom = custom.trim();
    if custom.is_empty() {
        return None;
    }
    // Expand a leading `~` (bare, `~/`, or `~\`) to $HOME; leave
    // `~otheruser/...` untouched. Mirrors the schema layer's tilde handling
    // without pulling in shellexpand here.
    if let Some(rest) = custom.strip_prefix('~')
        && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
        && let Ok(home) = std::env::var("HOME")
    {
        return Some(PathBuf::from(home).join(rest.trim_start_matches(['/', '\\'])));
    }
    Some(PathBuf::from(custom))
}

/// Apply the `[proxy]` table before the tokio runtime is built.
///
/// Sets the runtime proxy config that the provider and tool clients read. With
/// `scope = "environment"` it also exports the variables for child processes.
/// `main` calls this before the runtime exists, so no runtime thread can read
/// the environment during the export.
pub fn apply_startup_proxy(explicit: Option<&Path>) -> Result<()> {
    let loaded = load_app_config(explicit)?;
    let proxy = loaded.config.proxy;
    proxy.validate().map_err(|e| Error::Config(e.to_string()))?;
    if proxy.enabled && proxy.scope == operant_config::schema::ProxyScope::Environment {
        proxy.apply_to_process_env();
    }
    operant_config::schema::set_runtime_proxy_config(proxy);
    Ok(())
}

pub fn default_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // OPERANT_CONFIG_DIR is the highest-precedence override after an
    // explicit `--config`: `<dir>/operant.toml`. Checked BEFORE the cwd
    // candidates so a stray local `operant.toml` can never shadow an
    // explicitly requested isolated config home (isolation guarantee).
    if let Some(dir) = operant_config_dir_override() {
        paths.push(dir.join("operant.toml"));
    }

    paths.push(PathBuf::from("operant.toml"));
    paths.push(PathBuf::from(".operant.toml"));

    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("operant").join("config.toml"));
    }

    if let Some(home_dir) = dirs::home_dir() {
        paths.push(home_dir.join(".operant").join("operant.toml"));
    }

    paths
}

pub fn parse_config_file(path: &Path) -> Result<AppConfig> {
    let raw = std::fs::read_to_string(path).map_err(|error| {
        Error::Config(format!(
            "Failed to read config file '{}': {}",
            path.display(),
            error
        ))
    })?;

    parse_config_str(&raw, path)
}

pub fn parse_config_str(raw: &str, source: &Path) -> Result<AppConfig> {
    toml::from_str(raw).map_err(|error| {
        let message = match error.span() {
            Some(span) => format!(
                "Invalid TOML in '{}': {} (bytes {}..{})",
                source.display(),
                error,
                span.start,
                span.end
            ),
            None => format!("Invalid TOML in '{}': {}", source.display(), error),
        };

        Error::Config(message)
    })
}

impl AppConfig {
    pub fn apply_env_overrides(&mut self) -> Result<()> {
        apply_string_option_override("OPENAI_API_KEY", &mut self.client.api_key)?;
        apply_string_value_override("OPENAI_BASE_URL", &mut self.client.base_url);
        apply_string_value_override("HERMES_MODEL", &mut self.agent.model);
        apply_usize_override("HERMES_MAX_ITERATIONS", &mut self.agent.max_iterations)?;
        apply_u64_override("HERMES_TOOL_TIMEOUT", &mut self.agent.tool_timeout_secs)?;
        apply_u64_override(
            "HERMES_REQUEST_TIMEOUT",
            &mut self.agent.request_timeout_secs,
        )?;
        apply_usize_override("HERMES_CONTEXT_WINDOW", &mut self.agent.context_window)?;
        apply_usize_override(
            "HERMES_MAX_HEALING_ATTEMPTS",
            &mut self.agent.max_healing_attempts,
        )?;
        apply_bool_override("HERMES_STREAM", &mut self.agent.stream)?;
        apply_string_option_override("HERMES_SYSTEM_PROMPT", &mut self.agent.system_prompt)?;
        apply_u64_override(
            "HERMES_AUTONOMOUS_INTERVAL",
            &mut self.autonomous.interval_secs,
        )?;
        apply_path_override("HERMES_AUTONOMOUS_TODO", &mut self.autonomous.todo_path)?;
        apply_path_override("HERMES_AUTONOMOUS_STATUS", &mut self.autonomous.status_path)?;
        apply_string_value_override(
            "HERMES_AUTONOMOUS_TEST_COMMAND",
            &mut self.autonomous.test_command,
        );
        apply_string_value_override(
            "HERMES_AUTONOMOUS_GIT_REMOTE",
            &mut self.autonomous.git_remote,
        );
        apply_string_value_override(
            "HERMES_AUTONOMOUS_GIT_BRANCH",
            &mut self.autonomous.git_branch,
        );
        apply_string_value_override(
            "HERMES_AUTONOMOUS_COMMIT_MESSAGE",
            &mut self.autonomous.commit_message,
        );
        apply_u64_override(
            "HERMES_AUTONOMOUS_COMMAND_TIMEOUT",
            &mut self.autonomous.command_timeout_secs,
        )?;
        apply_usize_override(
            "HERMES_AUTONOMOUS_MAX_FAILURES",
            &mut self.autonomous.max_failures_per_state,
        )?;
        apply_string_value_override("HERMES_LOG_LEVEL", &mut self.logging.level);
        apply_path_override("HERMES_SKILLS_DIR", &mut self.skills.root_dir)?;

        // Gateway env var overrides
        apply_string_option_override("TELEGRAM_BOT_TOKEN", &mut self.gateway.telegram_token)?;
        apply_bool_override(
            "HERMES_TELEGRAM_ENABLED",
            &mut self.gateway.telegram_enabled,
        )?;
        apply_string_option_override("DISCORD_BOT_TOKEN", &mut self.gateway.discord_token)?;
        apply_bool_override("HERMES_DISCORD_ENABLED", &mut self.gateway.discord_enabled)?;
        apply_string_option_override("SLACK_BOT_TOKEN", &mut self.gateway.slack_token)?;
        apply_bool_override("HERMES_SLACK_ENABLED", &mut self.gateway.slack_enabled)?;
        apply_string_option_override(
            "OPERANT_WHATSAPP_PHONE_NUMBER_ID",
            &mut self.gateway.whatsapp_phone_number_id,
        )?;

        Ok(())
    }
}

fn read_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn apply_string_option_override(name: &str, target: &mut Option<String>) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = Some(value);
    }
    Ok(())
}

fn apply_string_value_override(name: &str, target: &mut String) {
    if let Some(value) = read_env(name) {
        *target = value;
    }
}

fn apply_u64_override(name: &str, target: &mut u64) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = value.parse().map_err(|_| {
            Error::Config(format!(
                "Environment variable '{}' must be an unsigned integer.",
                name
            ))
        })?;
    }
    Ok(())
}

fn apply_usize_override(name: &str, target: &mut usize) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = value.parse().map_err(|_| {
            Error::Config(format!(
                "Environment variable '{}' must be an unsigned integer.",
                name
            ))
        })?;
    }
    Ok(())
}

fn apply_bool_override(name: &str, target: &mut bool) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => {
                return Err(Error::Config(format!(
                    "Environment variable '{}' must be a boolean.",
                    name
                )));
            }
        };
    }
    Ok(())
}

fn apply_path_override(name: &str, target: &mut PathBuf) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = PathBuf::from(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn proxy_table_parses_from_operant_toml() {
        let config: AppConfig = toml::from_str(
            "[proxy]\nenabled = true\nscope = \"environment\"\nhttp_proxy = \"http://127.0.0.1:7890\"\n",
        )
        .expect("valid [proxy] table");
        assert!(config.proxy.enabled);
        assert_eq!(
            config.proxy.scope,
            operant_config::schema::ProxyScope::Environment
        );
    }

    use super::*;
    use serial_test::serial;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    // ── (plan 09) surgical config writes ────────────────────────────────

    /// A minimal but complete-enough config the loader accepts. The
    /// loader requires nothing mandatory, so a couple of comments and two
    /// keys is a valid file.
    fn write_plan09_fixture(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("operant.toml");
        std::fs::write(
            &path,
            "# my header comment — must survive every persist\n\n[client]\nbase_url = \"http://localhost:20129/v1\"\n\n[agent]\nmodel = \"big-stack\"\n",
        )
        .expect("write fixture");
        path
    }

    #[test]
    #[serial]
    fn persist_config_key_keeps_comments_and_other_keys_byte_for_byte() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_plan09_fixture(dir.path());
        let loaded = load_app_config(Some(&path)).expect("load fixture");
        install_runtime_config(loaded.config);

        let written = persist_config_key("agent.model", "small-stack").expect("persist");
        assert_eq!(written, path, "the LOADED file is the one written");

        let text = std::fs::read_to_string(&path).expect("re-read");
        assert!(
            text.contains("# my header comment — must survive every persist"),
            "comments must survive: {text}"
        );
        assert!(
            text.contains("http://localhost:20129/v1"),
            "client.base_url must be untouched: {text}"
        );
        assert!(
            text.contains("model = \"small-stack\""),
            "the leaf must be set: {text}"
        );
        assert!(!text.contains("big-stack"), "the old value is replaced");
        // No defaults resurrected: the file holds ONLY what it held plus the
        // leaf (the old whole-file writer emitted [pk], [genome.budget], …).
        assert!(
            !text.contains("[pk]"),
            "retired default sections must not appear"
        );
        assert!(
            !text.contains("[genome"),
            "retired default sections must not appear"
        );
        // And it re-loads.
        let reloaded = parse_config_file(&path).expect("reload");
        assert_eq!(reloaded.agent.model, "small-stack");
        assert_eq!(
            reloaded.client.base_url, "http://localhost:20129/v1",
            "base_url must survive the persist unchanged"
        );
    }

    #[test]
    #[serial]
    fn persist_config_key_never_writes_secrets_that_were_never_in_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_plan09_fixture(dir.path());
        let loaded = load_app_config(Some(&path)).expect("load fixture");
        // Simulate an env-sourced secret in the RUNTIME config (the old
        // writer serialized the whole runtime config, dragging it to disk).
        let mut cfg = loaded.config;
        cfg.client.api_key = Some("sk-NEVER-DISK".to_string());
        install_runtime_config(cfg);

        persist_config_key("agent.model", "small-stack").expect("persist");

        let text = std::fs::read_to_string(&path).expect("re-read");
        assert!(
            !text.contains("sk-NEVER-DISK"),
            "a secret that lives in env/runtime must never land in the file: {text}"
        );
        assert!(
            !text.contains("api_key"),
            "the api_key key must not appear either: {text}"
        );
    }

    #[test]
    #[serial]
    fn persist_config_key_creates_intermediate_tables_and_coerces_scalars() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_plan09_fixture(dir.path());
        let loaded = load_app_config(Some(&path)).expect("load fixture");
        install_runtime_config(loaded.config);

        // New nested leaf (a valid schema path): genome.budget.cap is a
        // real f64 field, and the [genome.budget] table is absent from the
        // fixture — the walk must CREATE it.
        persist_config_key("genome.budget.cap", "2.5").expect("persist float");
        let reloaded = parse_config_file(&path).expect("reload");
        assert_eq!(reloaded.genome.budget.cap, 2.5);

        // Boolean coercion on a real key.
        persist_config_key("tui.terminal_scroll_mode", "true").expect("persist bool");
        let reloaded = parse_config_file(&path).expect("reload");
        assert!(reloaded.tui.terminal_scroll_mode);
    }

    #[test]
    #[serial]
    fn persist_config_key_rejects_a_broken_leaf_and_leaves_the_file_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_plan09_fixture(dir.path());
        let before = std::fs::read_to_string(&path).expect("read before");
        let loaded = load_app_config(Some(&path)).expect("load fixture");
        install_runtime_config(loaded.config);

        // `agent.model` is a STRING — walking INTO it as a table must fail
        // before any write happens.
        let err = persist_config_key("agent.model.leaf", "x").expect_err("collision");
        assert!(err.to_string().contains("not a table"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read after"),
            before,
            "the file must be byte-identical after a rejected write"
        );
        // No temp litter.
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn plugin_signature_policy_is_configurable_and_defaults_disabled() {
        // Both the tool bridge and the kernel swap watcher read ONE policy
        // surface, so they can never disagree about whether plugin code is
        // verified. Absent = disabled, matching the schema world's
        // [plugins.security] default.
        let plugins = PluginSettings::default();
        assert_eq!(plugins.signature_mode, "disabled");
        assert!(plugins.trusted_publisher_keys.is_empty());

        let parsed: PluginSettings = toml::from_str(
            "signature_mode = \"strict\"\ntrusted_publisher_keys = [\"ab\", \"cd\"]",
        )
        .expect("valid plugin policy");
        assert_eq!(parsed.signature_mode, "strict");
        assert_eq!(parsed.trusted_publisher_keys, vec!["ab", "cd"]);
    }

    #[test]
    fn gateway_toml_roundtrips_whatsapp_phone_number_id() {
        // R22: proves the headline fix path — a user setting
        // `whatsapp_phone_number_id` under [gateway] in operant.toml actually
        // reaches GatewaySettings (pure serde via parse_config_str).
        let raw = r#"
[gateway]
whatsapp_enabled = true
whatsapp_token = "wa-token"
whatsapp_phone_number_id = "123456789"
"#;
        let parsed = parse_config_str(raw, std::path::Path::new("test.toml")).expect("valid TOML");
        assert!(parsed.gateway.whatsapp_enabled);
        assert_eq!(parsed.gateway.whatsapp_token.as_deref(), Some("wa-token"));
        assert_eq!(
            parsed.gateway.whatsapp_phone_number_id.as_deref(),
            Some("123456789")
        );

        // And an old TOML without the key still parses (backward compat).
        let old = r#"
[gateway]
whatsapp_enabled = true
whatsapp_token = "wa-token"
"#;
        let parsed = parse_config_str(old, std::path::Path::new("old.toml")).expect("valid TOML");
        assert_eq!(parsed.gateway.whatsapp_phone_number_id, None);
    }

    /// Plan 016 Phase 0 / step 1: prove the [harness] TOML section
    /// round-trips through parse_config_str, and that a missing
    /// section produces the dark-merge default (enabled=false).
    #[test]
    fn harness_settings_roundtrip_and_default_is_dark() {
        // Section present: enabled=true + an architecture path + cap.
        let toml = r#"
[harness]
enabled = true
architecture_toml = "/etc/operant/arch.toml"
max_active_providers = 128
"#;
        let parsed = parse_config_str(toml, std::path::Path::new("harness.toml"))
            .expect("valid TOML with [harness] section");
        assert!(parsed.harness.enabled, "explicit enabled=true is honored");
        assert_eq!(
            parsed.harness.architecture_toml.as_deref(),
            Some(std::path::Path::new("/etc/operant/arch.toml"))
        );
        assert_eq!(parsed.harness.max_active_providers, 128);
        // C2 — the WASM watcher is opt-in: absent means off, so an
        // existing config never starts a background poller.
        assert!(!parsed.harness.watch_wasm);
        let watched: HarnessSettings = toml::from_str("watch_wasm = true").expect("watch flag");
        assert!(watched.watch_wasm);

        // Re-serialize: settings survive a write/read cycle (proves
        // the surface is stable for the Phase 3 composition layer).
        let round_tripped = toml::to_string(&parsed.harness).expect("HarnessSettings serializes");
        let reparsed: HarnessSettings =
            toml::from_str(&round_tripped).expect("HarnessSettings round-trips through TOML");
        assert!(reparsed.enabled, "round-trip preserves enabled=true");
        assert_eq!(reparsed.max_active_providers, 128);

        // Section absent: dark-merge default — `enabled` is `false`,
        // the kernel is constructed but no provider is mounted.
        let no_section = r#"
[client]
base_url = "https://api.openai.com/v1"
"#;
        let parsed = parse_config_str(no_section, std::path::Path::new("default.toml"))
            .expect("valid TOML without [harness] still parses");
        assert!(
            !parsed.harness.enabled,
            "missing [harness] section defaults to enabled=false (dark-merge)"
        );
        assert_eq!(
            parsed.harness.max_active_providers, 64,
            "default cap is 64 per the 016 plan's kernel guardrail"
        );
        assert!(parsed.harness.architecture_toml.is_none());

        // Unknown [harness] keys are rejected (deny_unknown_fields).
        let bogus = r#"
[harness]
enabled = true
wonderful_unknown_key = 42
"#;
        let err = parse_config_str(bogus, std::path::Path::new("bogus.toml"));
        assert!(
            err.is_err(),
            "deny_unknown_fields must reject unknown [harness] keys"
        );
    }

    #[test]
    fn client_prompt_cache_ttl_parses_and_defaults() {
        use crate::agent::clients::prompt_caching::CacheTtl;

        // zeroclaw CacheTtl port: an explicit "1h" must reach the adapters.
        let raw = r#"
[client]
base_url = "https://openrouter.ai/api/v1"
timeout_secs = 30
max_context_length = 1000
prompt_cache_ttl = "1h"
"#;
        let parsed = parse_config_str(raw, std::path::Path::new("ttl.toml"))
            .expect("valid TOML with prompt_cache_ttl parses");
        assert_eq!(parsed.client.prompt_cache_ttl, CacheTtl::OneHour);

        // Round-trip: the field survives a write/read cycle.
        let round_tripped = toml::to_string(&parsed.client).expect("ClientSettings serializes");
        let reparsed: ClientSettings =
            toml::from_str(&round_tripped).expect("ClientSettings round-trips through TOML");
        assert_eq!(reparsed.prompt_cache_ttl, CacheTtl::OneHour);

        // Absent → 5-minute default (previous hardcoded behavior).
        let no_ttl = r#"
[client]
base_url = "https://api.openai.com/v1"
"#;
        let parsed = parse_config_str(no_ttl, std::path::Path::new("default-ttl.toml"))
            .expect("valid TOML without prompt_cache_ttl parses");
        assert_eq!(parsed.client.prompt_cache_ttl, CacheTtl::FiveMinutes);
    }

    /// Plan 015 / Phase 0: prove the [pk] TOML section round-trips
    /// through parse_config_str, that the dark-merge default is
    /// `enabled = false`, and that deny_unknown_fields catches typos.
    #[test]
    fn pk_settings_roundtrip_and_default_is_dark() {
        // Section present: enabled=true + an override script + cap.
        let toml = r#"
[pk]
enabled = true
sidecar_script = "/opt/operant/pk/main.py"
ndjson_socket = "/run/operant/pk.sock"
exec_timeout_secs = 90
"#;
        let parsed = parse_config_str(toml, std::path::Path::new("pk.toml"))
            .expect("valid TOML with [pk] section");
        assert!(parsed.pk.enabled, "explicit enabled=true is honored");
        assert_eq!(
            parsed.pk.sidecar_script.as_deref(),
            Some(std::path::Path::new("/opt/operant/pk/main.py"))
        );
        assert_eq!(
            parsed.pk.ndjson_socket.as_deref(),
            Some(std::path::Path::new("/run/operant/pk.sock"))
        );
        assert_eq!(parsed.pk.exec_timeout_secs, 90);

        // Re-serialize: settings survive a write/read cycle (Phase 1
        // supervisor reads this from disk at boot).
        let round_tripped = toml::to_string(&parsed.pk).expect("PkSettings serializes");
        let reparsed: PkSettings =
            toml::from_str(&round_tripped).expect("PkSettings round-trips through TOML");
        assert!(reparsed.enabled, "round-trip preserves enabled=true");
        assert_eq!(reparsed.exec_timeout_secs, 90);

        // Section absent: dark-merge default — sidecar never spawned.
        let no_section = r#"
[client]
base_url = "https://api.openai.com/v1"
"#;
        let parsed = parse_config_str(no_section, std::path::Path::new("default.toml"))
            .expect("valid TOML without [pk] still parses");
        assert!(
            !parsed.pk.enabled,
            "missing [pk] section defaults to enabled=false (dark-merge)"
        );
        assert_eq!(
            parsed.pk.exec_timeout_secs, 30,
            "default exec timeout is 30s per the 015 plan"
        );
        assert!(parsed.pk.sidecar_script.is_none());
        assert!(parsed.pk.ndjson_socket.is_none());

        // Unknown [pk] keys are rejected.
        let bogus = r#"
[pk]
enabled = true
wonderful_unknown_key = 42
"#;
        let err = parse_config_str(bogus, std::path::Path::new("bogus.toml"));
        assert!(
            err.is_err(),
            "deny_unknown_fields must reject unknown [pk] keys"
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "operant_config_test_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn with_current_dir<T>(path: &Path, f: impl FnOnce() -> T) -> T {
        let current = std::env::current_dir().unwrap();
        std::env::set_current_dir(path).unwrap();
        let result = f();
        std::env::set_current_dir(current).unwrap();
        result
    }

    fn set_env(name: &str, value: &str) -> Option<OsString> {
        let previous = std::env::var_os(name);
        // SAFETY: test-only env mutation under exclusive lock
        unsafe { std::env::set_var(name, value) };
        previous
    }

    fn restore_env(name: &str, previous: Option<OsString>) {
        if let Some(value) = previous {
            // SAFETY: test-only env mutation under exclusive lock
            unsafe { std::env::set_var(name, value) };
        } else {
            // SAFETY: test-only env mutation under exclusive lock
            unsafe { std::env::remove_var(name) };
        }
    }

    #[test]
    fn example_toml_parses() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("operant.example.toml");
        let raw = std::fs::read_to_string(&root).unwrap();
        let config = parse_config_str(&raw, &root).unwrap();
        assert_eq!(config.agent.model, "gpt-4");
        assert!(config.tui.rich_output);
        assert_eq!(config.autonomous.git_branch, "agent-dev");
        assert_eq!(
            config.autonomous.status_path,
            PathBuf::from("autonomous-status.toml")
        );
    }

    /// The `[genome]` block (wave-2 slice F2) maps onto the real fields —
    /// pinned with NON-default values so the test fails if the keys stop
    /// binding (a commented-out block parses as defaults and would pass
    /// vacuously, which is exactly what the values below rule out).
    #[test]
    fn socialization_block_parses_into_fields() {
        let raw = "[socialization]\nenabled = true\nschedule = \"0 45 9 * * *\"\nturn_budget = 2\npairs = [[\"premiere\", \"chief-of-staff\"]]\n";
        let config = parse_config_str(raw, Path::new("memory://socialization-probe")).unwrap();
        assert!(config.socialization.enabled);
        assert_eq!(config.socialization.schedule, "0 45 9 * * *");
        assert_eq!(config.socialization.turn_budget, 2);
        assert_eq!(
            config.socialization.pairs,
            vec![vec!["premiere".to_string(), "chief-of-staff".to_string()]]
        );
    }

    /// iter-723: the `[agent]` config-file surface for the guardrail
    /// exemption list and the two env-armed knobs — pinned with
    /// NON-default values so the keys stop binding = test fails (the
    /// same anti-vacuous discipline as the socialization probe above).
    #[test]
    fn agent_block_parses_guardrail_exempts_and_env_knob_defaults() {
        let raw = "[agent]\nguardrail_exempt_tools = [\"read_*\", \"web_search\"]\nguardian_llm = true\nmicro_compaction = true\n";
        let config = parse_config_str(raw, Path::new("memory://agent-block-probe")).unwrap();
        assert_eq!(
            config.agent.guardrail_exempt_tools,
            vec!["read_*".to_string(), "web_search".to_string()]
        );
        assert!(config.agent.guardian_llm);
        assert!(config.agent.micro_compaction);
        // The exempt list flows to the agent config the guardrail
        // tracker consumes (the From<&BehaviorSettings> mapping).
        let agent_cfg = crate::agent::AgentConfig::from(&config.agent);
        assert_eq!(
            agent_cfg.guardrail_exempt_tools,
            config.agent.guardrail_exempt_tools
        );
    }

    #[test]
    fn agent_block_env_knob_defaults_stay_dark() {
        // No [agent] keys → every new knob off, list empty (dark-mergeable:
        // an unchanged config file cannot change any behavior).
        let config = parse_config_str("[agent]\n", Path::new("memory://agent-dark-probe")).unwrap();
        assert!(config.agent.guardrail_exempt_tools.is_empty());
        assert!(!config.agent.guardian_llm);
        assert!(!config.agent.micro_compaction);
    }

    #[test]
    fn genome_block_parses_into_fields() {
        let raw = "[genome]\nunrestricted_default = \"standard\"\nunattended_posture = \"lockdown\"\ngrant_ttl_days = 3\n";
        let config = parse_config_str(raw, Path::new("memory://genome-probe")).unwrap();
        assert_eq!(config.genome.unrestricted_default, "standard");
        assert_eq!(config.genome.unattended_posture, "lockdown");
        assert_eq!(config.genome.grant_ttl_days, 3);
        // A removed [genome] key MUST fail loudly (deny_unknown_fields):
        // a typo'd security knob silently falling back to defaults is the
        // worse failure mode, so the schema stays strict and removal is a
        // documented break.
        let stale = "[genome]\nqueued_cron_jobs_resolve_grants = true\n";
        let err = parse_config_str(stale, Path::new("memory://genome-stale"));
        assert!(
            err.is_err(),
            "removed genome key must hard-fail config load"
        );
        // And the shipped example carries the live default TTL of 7 days,
        // uncommented, where the gateway reads it.
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("operant.example.toml");
        let example = std::fs::read_to_string(&root).unwrap();
        assert!(
            example.contains("grant_ttl_days = 7"),
            "the example must ship the HoD 7-day default, uncommented"
        );
    }

    /// The inert `lifeos_enabled` key is gone from both the schema and the
    /// shipped example, and the two have to move together: `ToolSettings` is
    /// `deny_unknown_fields`, so a `lifeos_enabled` line left in
    /// `operant.example.toml` makes `example_toml_parses` fail outright. This
    /// test names that coupling so the next person adding a tools flag cannot
    /// reintroduce the split, and so a future LifeOS implementation re-adds
    /// the key deliberately rather than by accident.
    #[test]
    fn lifeos_key_is_gone_from_config() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("operant.example.toml");
        let raw = std::fs::read_to_string(&root).unwrap();
        assert!(
            !raw.contains("lifeos"),
            "operant.example.toml still advertises lifeos_enabled, which gates nothing"
        );
        // And a config that still carries it is now rejected rather than
        // silently accepted-and-ignored, which is the point of removing it.
        let with_key = "[tools]\nlifeos_enabled = true\n";
        assert!(
            parse_config_str(with_key, Path::new("memory://lifeos-probe")).is_err(),
            "a stale lifeos_enabled must not parse silently into a field that gates nothing"
        );
    }

    /// Pins the `[browser]` block of `operant.example.toml` to the code.
    ///
    /// The example previously advertised `igs (default)` while
    /// `BrowserSettings::default()` returned `obscura`, and omitted `obscura`
    /// from the list entirely — so the one provider a fresh install actually
    /// uses was undocumented, and the documented default was wrong. The
    /// (plan 11C2) The [browser] section documents sourcehound-only: the
    /// provider alternatives are gone, the section must not advertise a
    /// provider choice anymore.
    #[test]
    fn example_toml_documents_the_real_browser_default_and_providers() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("operant.example.toml");
        let raw = std::fs::read_to_string(&root).unwrap();

        let block = raw
            .split("[browser]")
            .nth(1)
            .expect("operant.example.toml has no [browser] section")
            .split("\n[")
            .next()
            .expect("[browser] section is empty");
        // No provider list line anymore — the section says sourcehound is
        // the only engine.
        assert!(
            !block
                .lines()
                .any(|l| l.trim_start().starts_with("# provider:")),
            "[browser] must not document a provider choice (11C2)"
        );
        assert!(
            block.contains("sourcehound"),
            "[browser] names the sourcehound engine"
        );
    }

    // Both tests mutate the process-wide current directory via
    // `with_current_dir`, which races any concurrently-running test that
    // reads `std::env::current_dir()` (e.g. the #[serial] LCM rollup test's
    // `load_workspace_context` walk). `#[serial]` shares the global serial
    // lock, making them mutually exclusive with other serial tests; the
    // env_lock serializes them against the other env-mutating config tests.
    #[serial]
    #[test]
    fn default_path_discovery_prefers_local_operant_toml() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("default_path");
        std::fs::write(
            dir.join("operant.toml"),
            "[agent]\nmodel = \"gpt-4.1-mini\"\n",
        )
        .unwrap();

        let loaded = with_current_dir(&dir, || load_app_config(None)).unwrap();
        assert_eq!(loaded.source.unwrap().file_name().unwrap(), "operant.toml");
        assert_eq!(loaded.config.agent.model, "gpt-4.1-mini");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[serial]
    #[test]
    fn explicit_path_overrides_defaults() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("explicit_path");
        let explicit = dir.join("custom.toml");
        std::fs::write(dir.join("operant.toml"), "[agent]\nmodel = \"wrong\"\n").unwrap();
        std::fs::write(&explicit, "[agent]\nmodel = \"right\"\n").unwrap();

        let loaded = with_current_dir(&dir, || load_app_config(Some(&explicit))).unwrap();
        assert_eq!(loaded.config.agent.model, "right");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_toml_returns_field_aware_error() {
        let path = PathBuf::from("broken.toml");
        let error = parse_config_str("[agent]\nmax_iterations = \"many\"\n", &path).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("Invalid TOML"));
        assert!(text.contains("expected"));
    }

    // (plan 08) agentmemory_version_pins_package_specs deleted with the
    // spec fns and the seeding — the agentmemory era is over.

    // (plan 08) deleted with the agentmemory seeding: ensure_default_mcp_servers_injects_agentmemory_when_provider_active

    // (plan 08) deleted with the agentmemory seeding: ensure_default_mcp_servers_respects_custom_url_and_existing_server

    // (plan 08) deleted with the agentmemory seeding: ensure_default_mcp_servers_noop_for_builtin_provider

    /// (plan 08) DEFAULTS ARE SILENT: a clean start must not print the
    /// legacy-key warnings. Pre-plan-08 the DEFAULT provider was
    /// "agentmemory" (not in the allowed list) and auto_spawn defaulted
    /// to Some(true), so every single start warned — the audit's
    /// "seen on every start" finding.
    #[test]
    fn memory_defaults_are_silent() {
        let warnings = MemorySettings::default().legacy_warnings();
        assert!(
            warnings.is_empty(),
            "clean defaults must not warn, got: {warnings:?}"
        );
        assert_eq!(MemorySettings::default().provider, "memory-wire");
        // A file that actually carries a retired key still warns once.
        let loud = MemorySettings {
            provider: "agentmemory".to_string(),
            enabled: true,
            agentmemory_url: None,
            agentmemory_secret: None,
            agentmemory_auto_spawn: Some(true),
            agentmemory_version: None,
            audit_enabled: None,
            response_cache_enabled: None,
        };
        assert_eq!(loud.legacy_warnings().len(), 2);
    }

    /// The keys the factory-era module docs told users to write. They were never
    /// fields of this struct, so `deny_unknown_fields` failed the whole config load
    /// (`Invalid TOML … unknown field audit_enabled`). They now load and warn once.
    #[test]
    fn retired_memory_factory_keys_load_and_warn() {
        let parsed: MemorySettings =
            toml::from_str("audit_enabled = true\nresponse_cache_enabled = false")
                .expect("retired keys must not fail the config load");
        let warnings = parsed.legacy_warnings();
        assert_eq!(warnings.len(), 2, "got: {warnings:?}");
        assert!(warnings[0].contains("audit_enabled"));
        assert!(warnings[1].contains("response_cache_enabled"));
    }

    #[test]
    fn env_overrides_apply_after_file_values() {
        let _guard = env_lock().lock().unwrap();
        let previous_model = set_env("HERMES_MODEL", "gpt-4.1");
        let previous_stream = set_env("HERMES_STREAM", "false");
        let previous_interval = set_env("HERMES_AUTONOMOUS_INTERVAL", "120");
        let previous_status = set_env("HERMES_AUTONOMOUS_STATUS", "runtime/autonomous-status.toml");

        let mut config = parse_config_str(
            "[agent]\nmodel = \"gpt-4o-mini\"\nstream = true\n",
            Path::new("env.toml"),
        )
        .unwrap();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.agent.model, "gpt-4.1");
        assert!(!config.agent.stream);
        assert_eq!(config.autonomous.interval_secs, 120);
        assert_eq!(
            config.autonomous.status_path,
            PathBuf::from("runtime/autonomous-status.toml")
        );

        restore_env("HERMES_MODEL", previous_model);
        restore_env("HERMES_STREAM", previous_stream);
        restore_env("HERMES_AUTONOMOUS_INTERVAL", previous_interval);
        restore_env("HERMES_AUTONOMOUS_STATUS", previous_status);
    }

    #[test]
    fn providers_section_roundtrips_into_app_config() {
        // Hermes `fallback_providers` parity: the run path must be able to
        // read `[providers]` profiles + the ordered fallback chain from
        // operant.toml (this was previously only loaded by the channels/ACP
        // config path, never by AppConfig).
        let raw = r#"
[client]
base_url = "https://zen.example/v1"
api_key = "sk-test"

[agent]
model = "laguna-s-2.1-free"
fallback_models = ["deepseek-v4-flash-free"]

[providers.models.opencode-zen]
base_url = "https://zen.example/v1"
model = "deepseek-v4-flash-free"

[[providers.fallback_chain]]
provider = "opencode-zen"
model = "deepseek-v4-flash-free"
"#;
        let parsed =
            parse_config_str(raw, std::path::Path::new("providers.toml")).expect("valid TOML");

        // Profiles load.
        assert!(parsed.providers.models.contains_key("opencode-zen"));
        assert_eq!(
            parsed.providers.models["opencode-zen"].model.as_deref(),
            Some("deepseek-v4-flash-free")
        );
        // Ordered chain loads in order.
        assert_eq!(parsed.providers.fallback_chain.len(), 1);
        assert_eq!(parsed.providers.fallback_chain[0].provider, "opencode-zen");
        assert_eq!(
            parsed.providers.fallback_chain[0].model,
            "deepseek-v4-flash-free"
        );
        // Same-provider model fallback loads.
        assert_eq!(parsed.agent.fallback_models, vec!["deepseek-v4-flash-free"]);
        // Absent section defaults to empty — never blocks boot.
        let minimal = parse_config_str("[agent]\nmodel = \"x\"\n", Path::new("min.toml")).unwrap();
        assert!(minimal.providers.models.is_empty());
        assert!(minimal.providers.fallback_chain.is_empty());
    }

    #[test]
    fn credential_pool_strategies_parse_and_resolve_per_provider() {
        // hermes `credential_pool_strategies` parity: the per-provider map
        // must round-trip through TOML and win over the global strategy.
        let raw = r#"
[credential_pool]
enabled = true
strategy = "fill_first"
strategies = { opencode-zen = "round_robin", openai = "least_used" }
"#;
        let parsed = parse_config_str(raw, std::path::Path::new("pool.toml")).expect("valid TOML");
        use crate::credential_pool::PoolStrategy;
        assert!(parsed.credential_pool.enabled);
        // Per-provider entries win.
        assert_eq!(
            parsed.credential_pool.strategy_for("opencode-zen"),
            PoolStrategy::RoundRobin
        );
        assert_eq!(
            parsed.credential_pool.strategy_for("openai"),
            PoolStrategy::LeastUsed
        );
        // Unconfigured provider falls back to the global strategy.
        assert_eq!(
            parsed.credential_pool.strategy_for("anthropic"),
            PoolStrategy::FillFirst
        );
    }

    #[test]
    fn credential_pool_strategy_fallback_semantics() {
        use crate::credential_pool::PoolStrategy;

        // No strategy configured anywhere → fill_first.
        let unset = CredentialPoolSettings::default();
        assert_eq!(unset.strategy_for("anything"), PoolStrategy::FillFirst);

        // Global-only: every provider inherits it.
        let global = CredentialPoolSettings {
            strategy: Some("round_robin".to_string()),
            ..Default::default()
        };
        assert_eq!(global.strategy_for("openai"), PoolStrategy::RoundRobin);
        assert_eq!(
            global.strategy_for("opencode-zen"),
            PoolStrategy::RoundRobin
        );

        // Unknown/invalid value → fill_first (hermes `get_pool_strategy()`
        // fail-closed default; never panics on a typo'd config).
        let invalid = CredentialPoolSettings {
            strategy: Some("banana".to_string()),
            strategies: {
                let mut m = std::collections::HashMap::new();
                m.insert("openai".to_string(), "round robin with spaces".to_string());
                m
            },
            ..Default::default()
        };
        assert_eq!(invalid.strategy_for("openai"), PoolStrategy::FillFirst);
        assert_eq!(invalid.strategy_for("other"), PoolStrategy::FillFirst);
    }
}
