// Vendored from jcode (crates/operant-tui/src/tui/mod.rs + tui/redraw_schedule.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — the tui/mod.rs-level items the ported renderers reference.
// See operant_app/mod.rs for scope.
//! Included from tui/mod.rs: BackgroundTaskRowStatus (:12), BackgroundTaskRow
//! (:17), scheduled_notification_text (:88), TuiState trait (:395),
//! connection_type_icon (:923), CacheTtlInfo (:941, plus impl :984),
//! format_compact_age (:956), KvCacheProblemKind/KvCacheProblem (:1008-:1028),
//! detect_kv_cache_problem (:1102) with its private helpers, PickerKind (:1148),
//! PromptHistorySearchView (:1155), InlineViewState (:1291, plus impl),
//! InlineUiStateRef (:1317), InlineInteractiveLayout (:1272),
//! InlineInteractiveSchema (:1278), impl PickerKind (:1323), AccountPickerAction
//! (:1436), AgentModelTarget (:1444), PickerAction (:1453), InlineInteractiveState
//! (:1487, plus both impls and the private estimate_* helpers), PickerEntry (:1730,
//! plus impl), PickerOption (:1775), ssh_remote_host (:1785). Included from
//! tui/redraw_schedule.rs: LAST_FULL_FRAME_REDRAW_REASON (:157),
//! FULL_FRAME_REDRAW_REASONS (:160), last_full_frame_redraw_reason (:182).
//! [port-excision] from the TuiState trait: workspace_map_rows,
//! session_picker_overlay, login_picker_overlay, account_picker_overlay and
//! usage_overlay — their payload modules (workspace_map, the four overlay
//! pickers) are unported waves; no ported renderer calls them.
//! context_info/context_snapshot stay (ContextInfo and ContextSnapshot are
//! ported). Everything else in the trait is verbatim.
#[allow(unused_imports)] // re-export: TuiState trait method defaults consume at cutover
use crate::tui::operant_app::app::ProcessingStatus;
#[allow(unused_imports)] // re-export: TuiState trait method defaults consume at cutover
use crate::tui::operant_app::auth::ActiveCredential;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_app::bus::BatchProgress;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_app::info_widget::AmbientWidgetData;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_app::prompt::ContextInfo;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_app::protocol::SwarmMemberStatus;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_app::provider::ActiveProvider;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_app::session::RenderedImage;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_model::ContentPos;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_model::DisplayMessage;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_model::vendor_types::ToolCall;
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use crate::tui::operant_ui::copy_selection::{CopySelectionRange, CopySelectionStatus};
#[allow(unused_imports)] // vendored-verbatim / re-export for cutover consumers
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundTaskRowStatus {
    Running,
    Completed,
    Failed,
}

/// Compact presentation state for one retained background task.
#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundTaskRow {
    pub task_id: String,
    pub label: String,
    pub percent: Option<f32>,
    pub status: BackgroundTaskRowStatus,
    /// When a successful task stopped being actionable. Running and failed
    /// tasks remain visible until their state changes or the session closes.
    pub completed_at: Option<std::time::Instant>,
}

#[derive(Clone)]
pub struct ContextSnapshot {
    pub info: Option<crate::tui::operant_app::prompt::ContextInfo>,
    pub revision: u64,
    pub fresh: bool,
}

pub(crate) fn scheduled_notification_text(
    info: Option<&crate::tui::operant_app::info_widget::AmbientWidgetData>,
) -> Option<String> {
    let info = info?;
    if info.reminder_count == 0 {
        return None;
    }
    let next = info.next_reminder_wake.as_deref()?;
    let suffix = if info.reminder_count > 1 {
        format!(" · {} queued", info.reminder_count)
    } else {
        String::new()
    };
    Some(format!("⏰ next scheduled task {}{}", next, suffix))
}

// [port-decision] dedup: the TuiState trait was ported twice — here and in
// operant_app/tui_state.rs (the canonical verbatim :395-916 port). Deleted this
// copy; the `pub use` re-export at the bottom of this file provides the name.

pub(crate) fn connection_type_icon(connection_type: Option<&str>) -> Option<&'static str> {
    let normalized = connection_type?.trim().to_ascii_lowercase();
    if normalized.contains("websocket") || normalized == "ws" || normalized == "wss" {
        // 🔌 is a single emoji-default codepoint. The previous 🕸️ (U+1F578 +
        // VS16) is text-default and rendered as a monochrome outline/tofu in
        // macOS window titles (Ghostty/Terminal ignore the VS16 selector there).
        Some("🔌")
    } else if normalized.contains("http") {
        Some("🌐")
    } else {
        None
    }
}

/// Cache TTL information for the current provider
#[derive(Debug, Clone)]
pub struct CacheTtlInfo {
    /// Provider retention varies, so this countdown cannot establish a hit or expiry.
    pub is_estimate: bool,
    /// Seconds until the retention window ends (estimated for some providers)
    pub remaining_secs: u64,
    /// Total TTL for this provider in seconds
    pub ttl_secs: u64,
    /// Whether the retention window elapsed, not proof of eviction for estimates
    pub is_cold: bool,
    /// How long ago the retention window ended, in seconds (0 before it ends)
    pub cold_for_secs: u64,
    /// Estimated cached tokens (from last response's input tokens)
    pub cached_tokens: Option<u64>,
}

impl CacheTtlInfo {
    /// How long before expiry the `⏳ cache ...` countdown should appear.
    ///
    /// A fixed 60s window is fine for a 5-minute TTL but far too easy to miss
    /// on a 1-hour (or 24-hour) TTL where stepping away is exactly the failure
    /// mode. Scale with the TTL (10%) but keep it within 60s..10min so short
    /// TTLs keep their old behavior and long TTLs don't nag for hours.
    pub fn warn_window_secs(&self) -> u64 {
        (self.ttl_secs / 10).clamp(60, 600)
    }

    /// Whether the cache is warm but close enough to expiry that the
    /// countdown should be shown (and idle redraws kept alive).
    pub fn expiring_soon(&self) -> bool {
        !self.is_estimate && !self.is_cold && self.remaining_secs <= self.warn_window_secs()
    }

    /// Only known TTLs may drive proactive expiry UI. An estimate or minimum
    /// lifetime says nothing about when the provider will actually evict cache.
    pub fn expiry_notification_active(&self) -> bool {
        !self.is_estimate && (self.is_cold || self.expiring_soon())
    }
}

/// Compact human age like `30s`, `5m`, `1h 1m`, `2d 3h` for "went cold N ago"
/// annotations. Keeps at most two units so it stays glanceable.
pub(crate) fn format_compact_age(secs: u64) -> String {
    if secs < 60 {
        return format!("{}s", secs);
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{}m", mins);
    }
    let hours = mins / 60;
    let rem_mins = mins % 60;
    if hours < 24 {
        return if rem_mins == 0 {
            format!("{}h", hours)
        } else {
            format!("{}h {}m", hours, rem_mins)
        };
    }
    let days = hours / 24;
    let rem_hours = hours % 24;
    if rem_hours == 0 {
        format!("{}d", days)
    } else {
        format!("{}d {}h", days, rem_hours)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KvCacheProblemKind {
    /// The provider explicitly reported new cache creation on a turn where we expected
    /// an already-warm cache to be read instead.
    UnexpectedCacheCreation,
    /// The provider explicitly reported zero cached input tokens on a turn where this
    /// provider family should report cached tokens for a warm, cacheable conversation.
    ExpectedCacheReadMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KvCacheProblem {
    pub kind: KvCacheProblemKind,
    pub affected_tokens: Option<u64>,
}

impl KvCacheProblem {
    pub(crate) fn log_reason(self) -> &'static str {
        match self.kind {
            KvCacheProblemKind::UnexpectedCacheCreation => "unexpected_cache_creation",
            KvCacheProblemKind::ExpectedCacheReadMissing => "expected_cache_read_missing",
        }
    }
}

fn normalized_provider_matches(provider: &str, needle: &str) -> bool {
    provider.trim().to_ascii_lowercase().contains(needle)
}

fn provider_stack_contains(provider: &str, upstream_provider: Option<&str>, needle: &str) -> bool {
    let needle = &needle.to_ascii_lowercase();
    normalized_provider_matches(provider, needle)
        || upstream_provider
            .map(|upstream| normalized_provider_matches(upstream, needle))
            .unwrap_or(false)
}

fn cache_expected_warm(cache_ttl: Option<&CacheTtlInfo>) -> bool {
    cache_ttl
        .map(|info| !info.is_cold && !info.is_estimate)
        .unwrap_or(false)
}

/// Detect a KV/prompt-cache problem that is reliable enough to surface in the UI.
///
/// This intentionally does **not** warn merely because a cache-hit metric is absent. A warning
/// requires all of the following:
/// - a multi-turn conversation where cache reuse should be possible;
/// - a prior completed turn still within the provider's expected cache TTL;
/// - explicit provider telemetry showing either a cache rewrite without a read, or an explicit
///   zero cache-read count from a known cache-reporting provider family;
/// - enough input tokens to be cacheable for read-only providers.
pub(crate) fn detect_kv_cache_problem(
    provider: &str,
    upstream_provider: Option<&str>,
    user_turn_count: usize,
    input_tokens: u64,
    cache_read: Option<u64>,
    cache_creation: Option<u64>,
    cache_ttl: Option<&CacheTtlInfo>,
) -> Option<KvCacheProblem> {
    if user_turn_count <= 2 || !cache_expected_warm(cache_ttl) {
        return None;
    }

    let cache_read_tokens = cache_read.unwrap_or(0);
    let cache_creation_tokens = cache_creation.unwrap_or(0);

    // Strongest signal: the provider explicitly says it created cache but read none.
    if cache_creation_tokens > 0 && cache_read_tokens == 0 {
        return Some(KvCacheProblem {
            kind: KvCacheProblemKind::UnexpectedCacheCreation,
            affected_tokens: Some(cache_creation_tokens),
        });
    }

    // Read-only telemetry providers (OpenAI/Gemini and known OpenRouter upstreams) do not expose
    // cache creation tokens. For those, an explicit zero read on a warm, cacheable conversation is
    // the reliable signal. Absence of the metric is ignored.
    if cache_read != Some(0) {
        return None;
    }

    if !supports_reliable_zero_cache_read_warning(provider, upstream_provider) {
        return None;
    }

    if input_tokens < min_cacheable_input_tokens(provider, upstream_provider) {
        return None;
    }

    Some(KvCacheProblem {
        kind: KvCacheProblemKind::ExpectedCacheReadMissing,
        affected_tokens: Some(input_tokens),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    Account,
    Login,
    Usage,
}

/// Render snapshot of the Ctrl+R reverse prompt-history search overlay.
/// `matches` are single-line previews, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptHistorySearchView {
    pub query: String,
    pub matches: Vec<String>,
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineInteractiveLayout {
    Compact,
    ThreeColumn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineInteractiveSchema {
    pub layout: InlineInteractiveLayout,
    pub primary_label: &'static str,
    pub secondary_label: &'static str,
    pub secondary_preview_label: &'static str,
    pub tertiary_label: &'static str,
    pub preview_submit_hint: &'static str,
    pub active_submit_hint: &'static str,
    pub shows_default_shortcut_hint: bool,
    pub preview_activation_column: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineViewState {
    pub title: String,
    pub status: Option<String>,
    pub lines: Vec<String>,
}

impl InlineViewState {
    pub fn debug_memory_profile(&self) -> serde_json::Value {
        let title_bytes = self.title.capacity();
        let status_bytes = self
            .status
            .as_ref()
            .map(|value| value.capacity())
            .unwrap_or(0);
        let lines_bytes: usize = self.lines.iter().map(|value| value.capacity()).sum();
        serde_json::json!({
            "lines_count": self.lines.len(),
            "title_bytes": title_bytes,
            "status_bytes": status_bytes,
            "lines_bytes": lines_bytes,
            "total_estimate_bytes": title_bytes + status_bytes + lines_bytes,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum InlineUiStateRef<'a> {
    View(&'a InlineViewState),
    Interactive(&'a InlineInteractiveState),
}

impl PickerKind {
    pub fn schema(&self) -> InlineInteractiveSchema {
        match self {
            Self::Model => InlineInteractiveSchema {
                layout: InlineInteractiveLayout::ThreeColumn,
                primary_label: "MODEL",
                secondary_label: "PROVIDER",
                secondary_preview_label: "PROVIDER",
                tertiary_label: "METHOD",
                preview_submit_hint: "  ↵ open",
                active_submit_hint: "  ↑↓ ←→ ↵ Esc",
                shows_default_shortcut_hint: true,
                preview_activation_column: 2,
            },
            Self::Account => InlineInteractiveSchema {
                layout: InlineInteractiveLayout::Compact,
                primary_label: "ACCOUNT",
                secondary_label: "STATE",
                secondary_preview_label: "STATE",
                tertiary_label: "",
                preview_submit_hint: "  ↵ select",
                active_submit_hint: "  ↑↓/jk ↵ Esc",
                shows_default_shortcut_hint: false,
                preview_activation_column: 0,
            },
            Self::Login => InlineInteractiveSchema {
                layout: InlineInteractiveLayout::ThreeColumn,
                primary_label: "ITEM",
                secondary_label: "PROVIDER",
                secondary_preview_label: "PROVIDER",
                tertiary_label: "ACTION",
                preview_submit_hint: "  ↵ open",
                active_submit_hint: "  ↑↓ ←→ ↵ Esc",
                shows_default_shortcut_hint: true,
                preview_activation_column: 2,
            },
            Self::Usage => InlineInteractiveSchema {
                layout: InlineInteractiveLayout::ThreeColumn,
                primary_label: "ITEM",
                secondary_label: "STATUS",
                secondary_preview_label: "ITEM",
                tertiary_label: "WINDOW",
                preview_submit_hint: "  ↵ inspect",
                active_submit_hint: "  ↑↓ ←→ ↵ Esc",
                shows_default_shortcut_hint: false,
                preview_activation_column: 2,
            },
        }
    }

    pub fn uses_compact_navigation(&self) -> bool {
        self.schema().layout == InlineInteractiveLayout::Compact
    }

    pub fn filter_text(&self, entry: &PickerEntry) -> String {
        match self {
            Self::Account => {
                let provider = entry
                    .active_option()
                    .map(|option| option.provider.as_str())
                    .unwrap_or("");
                let state = entry.account_state_label().unwrap_or("");
                format!("{} {} {}", entry.name, provider, state)
            }
            Self::Login => {
                let auth_kind = entry
                    .active_option()
                    .map(|option| option.provider.as_str())
                    .unwrap_or("");
                let state = entry
                    .active_option()
                    .map(|option| option.api_method.as_str())
                    .unwrap_or("");
                let detail = entry
                    .active_option()
                    .map(|option| option.detail.as_str())
                    .unwrap_or("");
                format!("{} {} {} {}", entry.name, auth_kind, state, detail)
            }
            Self::Usage => {
                let status = entry
                    .active_option()
                    .map(|option| option.provider.as_str())
                    .unwrap_or("");
                let window = entry
                    .active_option()
                    .map(|option| option.api_method.as_str())
                    .unwrap_or("");
                let detail = entry
                    .active_option()
                    .map(|option| option.detail.as_str())
                    .unwrap_or("");
                format!("{} {} {} {}", entry.name, status, window, detail)
            }
            Self::Model => {
                let route = entry.active_option();
                let provider = route.map(|option| option.provider.as_str()).unwrap_or("");
                let method = route.map(|option| option.api_method.as_str()).unwrap_or("");
                let detail = route.map(|option| option.detail.as_str()).unwrap_or("");
                // Include the pretty name so a query like "opus 4.8" matches
                // the row even though the underlying id is `claude-opus-4-8`.
                let pretty =
                    crate::tui::operant_app::helpers::model_names::pretty_known_model_family(
                        &entry.name,
                    )
                    .unwrap_or_default();
                format!(
                    "{} {} {} {} {}",
                    entry.name, pretty, provider, method, detail
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountPickerAction {
    Switch { provider_id: String, label: String },
    Add { provider_id: String },
    Replace { provider_id: String, label: String },
    OpenCenter { provider_filter: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentModelTarget {
    Swarm,
    Review,
    Judge,
    Memory,
    Ambient,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    Model,
    Account(AccountPickerAction),
    // [port-excision] Login(LoginProviderDescriptor) — operant-provider-metadata
    // cascade not ported; no ported renderer constructs this variant.
    /// Native SSH actions never dispatch through laptop-local authentication.
    RemoteLogin {
        provider: &'static str,
        import: bool,
    },
    /// Explicit remote import offer/consent, never a local authentication action.
    RemoteImportDecision {
        accept: bool,
    },
    // [port-excision] Logout(LoginProviderDescriptor) — same cascade.
    LogoutAll,
    // [port-excision] Usage { .. } — operant-tui-usage-overlay not ported; no
    // ported renderer constructs this variant.
    AgentTarget(AgentModelTarget),
    AgentModelChoice {
        target: AgentModelTarget,
        clear_override: bool,
    },
    SubagentModelChoice {
        inherit: bool,
    },
}

/// Unified inline picker with three columns.
#[derive(Debug, Clone)]
pub struct InlineInteractiveState {
    /// Which inline picker is currently active.
    pub kind: PickerKind,
    /// All visible picker entries and their available actions/options.
    pub entries: Vec<PickerEntry>,
    /// Filtered indices into `entries`.
    pub filtered: Vec<usize>,
    /// Selected row in filtered list
    pub selected: usize,
    /// Active column: 0=primary item, 1=secondary option, 2=tertiary option.
    pub column: usize,
    /// Filter text applied to the picker kind's searchable text.
    pub filter: String,
    /// Preview mode: picker is visible but input stays in main text box
    pub preview: bool,
}

impl InlineInteractiveState {
    pub fn debug_memory_profile(&self) -> serde_json::Value {
        let entries_bytes: usize = self.entries.iter().map(estimate_picker_entry_bytes).sum();
        let filtered_bytes = self.filtered.capacity() * std::mem::size_of::<usize>();
        let filter_bytes = self.filter.capacity();
        serde_json::json!({
            "entries_count": self.entries.len(),
            "filtered_count": self.filtered.len(),
            "entries_bytes": entries_bytes,
            "filtered_bytes": filtered_bytes,
            "filter_bytes": filter_bytes,
            "total_estimate_bytes": entries_bytes + filtered_bytes + filter_bytes,
        })
    }
}

fn estimate_picker_action_bytes(action: &PickerAction) -> usize {
    match action {
        PickerAction::Model
        | PickerAction::RemoteLogin { .. }
        | PickerAction::RemoteImportDecision { .. }
        | PickerAction::AgentTarget(_)
        | PickerAction::AgentModelChoice { .. }
        | PickerAction::SubagentModelChoice { .. }
        | PickerAction::LogoutAll => 0,
        PickerAction::Account(AccountPickerAction::Switch { provider_id, label }) => {
            provider_id.capacity() + label.capacity()
        }
        PickerAction::Account(AccountPickerAction::Add { provider_id }) => provider_id.capacity(),
        PickerAction::Account(AccountPickerAction::Replace { provider_id, label }) => {
            provider_id.capacity() + label.capacity()
        }
        PickerAction::Account(AccountPickerAction::OpenCenter { provider_filter }) => {
            provider_filter
                .as_ref()
                .map(|value| value.capacity())
                .unwrap_or(0)
        } // [port-excision] Login/Logout arm — variants excised above.
          // [port-excision] Usage arm — variant excised above.
    }
}

fn estimate_picker_option_bytes(option: &PickerOption) -> usize {
    option.provider.capacity() + option.api_method.capacity() + option.detail.capacity()
}

fn estimate_picker_entry_bytes(entry: &PickerEntry) -> usize {
    entry.name.capacity()
        + entry
            .options
            .iter()
            .map(estimate_picker_option_bytes)
            .sum::<usize>()
        + estimate_picker_action_bytes(&entry.action)
        + entry
            .created_date
            .as_ref()
            .map(|value| value.capacity())
            .unwrap_or(0)
        + entry
            .effort
            .as_ref()
            .map(|value| value.capacity())
            .unwrap_or(0)
}

/// A reusable picker entry with one or more available actions/options.
#[derive(Debug, Clone)]
pub struct PickerEntry {
    pub name: String,
    pub options: Vec<PickerOption>,
    pub action: PickerAction,
    pub selected_option: usize,
    pub is_current: bool,
    pub is_default: bool,
    pub is_favorite: bool,
    pub recommended: bool,
    pub recommendation_rank: usize,
    pub usage_score: u32,
    pub old: bool,
    /// Human-readable created date (e.g. "Jan 2026") for OpenRouter models
    pub created_date: Option<String>,
    pub effort: Option<String>,
}

impl PickerEntry {
    pub fn active_option(&self) -> Option<&PickerOption> {
        self.options.get(self.selected_option)
    }

    pub fn active_option_mut(&mut self) -> Option<&mut PickerOption> {
        self.options.get_mut(self.selected_option)
    }

    pub fn option_count(&self) -> usize {
        self.options.len()
    }

    pub fn account_state_label(&self) -> Option<&'static str> {
        match &self.action {
            PickerAction::Account(AccountPickerAction::Switch { .. }) => {
                Some(if self.is_current { "active" } else { "saved" })
            }
            PickerAction::Account(AccountPickerAction::Add { .. }) => Some("add"),
            PickerAction::Account(AccountPickerAction::Replace { .. }) => Some("replace"),
            PickerAction::Account(AccountPickerAction::OpenCenter { .. }) => Some("manage"),
            _ => None,
        }
    }
}

/// A single available option for a picker entry.
#[derive(Debug, Clone)]
pub struct PickerOption {
    pub provider: String,
    pub api_method: String,
    pub available: bool,
    pub detail: String,
    pub estimated_reference_cost_micros: Option<u64>,
}

/// An SSH-backed socket is not a shared-filesystem local daemon. Keep this
/// distinct from `App::is_remote`, which also describes ordinary local clients.
pub(crate) fn ssh_remote_host() -> Option<String> {
    std::env::var("OPERANT_SSH_REMOTE")
        .ok()
        .filter(|host| !host.trim().is_empty())
}

// --- crates/operant-tui/src/tui/redraw_schedule.rs ------------------------------------

/// Stored as an index into [`FULL_FRAME_REDRAW_REASONS`] in an atomic, so the
/// redraw hot path records it without locking (and without an error to ignore).
static LAST_FULL_FRAME_REDRAW_REASON: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(usize::MAX);

const FULL_FRAME_REDRAW_REASONS: &[&str] = &[
    "processing",
    "streaming",
    "tail_catchup",
    "status_notice",
    "learn_hint",
    "mouse_scroll_animation",
    "copy_autoscroll",
    "notification",
    "rate_limit_countdown",
    "remote_startup",
    "status_animation",
    "swarm_spinner",
    "session_picker_spinner",
];

pub(crate) fn last_full_frame_redraw_reason() -> Option<&'static str> {
    FULL_FRAME_REDRAW_REASONS
        .get(LAST_FULL_FRAME_REDRAW_REASON.load(std::sync::atomic::Ordering::Relaxed))
        .copied()
}

pub use crate::tui::operant_app::tui_state::TuiState;

// Verbatim from operant-tui/src/tui/mod.rs:1035-1086 (kv_cache_problem's two
// predicates plus the three fns they stand on; nothing here depends on app
// state — these are pure and gated on the same upstream definitions).

fn provider_stack_contains_any(
    provider: &str,
    upstream_provider: Option<&str>,
    needles: &[&str],
) -> bool {
    needles
        .iter()
        .any(|needle| provider_stack_contains(provider, upstream_provider, needle))
}

fn supports_reliable_zero_cache_read_warning(
    provider: &str,
    upstream_provider: Option<&str>,
) -> bool {
    if provider_stack_contains_any(
        provider,
        upstream_provider,
        &["openai", "anthropic", "claude", "gemini", "google"],
    ) {
        return true;
    }

    // OpenRouter/Jcode-subscription routes can only be treated as reliable for zero-read
    // warnings once the upstream provider identifies a known cache-reporting family.
    // A bare OpenRouter route with cached_tokens=0 is not enough: some upstreams simply
    // do not implement prompt caching, and warning on those would make the UI untrustworthy.
    false
}

fn min_cacheable_input_tokens(provider: &str, upstream_provider: Option<&str>) -> u64 {
    if provider_stack_contains_any(provider, upstream_provider, &["gemini", "google"]) {
        // Be conservative for Gemini-style implicit caching. Several Gemini models have
        // higher minimums than OpenAI/Anthropic; a higher UI threshold avoids warning on
        // prompts that might legitimately be below the provider's cacheable size.
        4_096
    } else {
        1_024
    }
}
impl InlineInteractiveState {
    pub fn schema(&self) -> InlineInteractiveSchema {
        if self.is_agent_target_picker() {
            InlineInteractiveSchema {
                layout: InlineInteractiveLayout::ThreeColumn,
                primary_label: "TARGET",
                secondary_label: "MODEL",
                secondary_preview_label: "MODEL",
                tertiary_label: "CONFIG",
                preview_submit_hint: "  ↵ open",
                active_submit_hint: "  ↑↓ ←→ ↵ Esc",
                shows_default_shortcut_hint: false,
                preview_activation_column: 2,
            }
        } else {
            self.kind.schema()
        }
    }

    pub fn selected_entry_index(&self) -> Option<usize> {
        self.filtered.get(self.selected).copied()
    }

    pub fn selected_entry(&self) -> Option<&PickerEntry> {
        self.selected_entry_index()
            .and_then(|index| self.entries.get(index))
    }

    pub fn selected_entry_mut(&mut self) -> Option<&mut PickerEntry> {
        self.selected_entry_index()
            .and_then(|index| self.entries.get_mut(index))
    }

    pub fn is_agent_target_picker(&self) -> bool {
        self.kind == PickerKind::Model
            && !self.entries.is_empty()
            && self
                .entries
                .iter()
                .all(|entry| matches!(entry.action, PickerAction::AgentTarget(_)))
    }

    pub fn uses_compact_navigation(&self) -> bool {
        self.schema().layout == InlineInteractiveLayout::Compact
    }

    pub fn preview_submit_hint(&self) -> &'static str {
        self.schema().preview_submit_hint
    }

    pub fn active_submit_hint(&self) -> &'static str {
        self.schema().active_submit_hint
    }

    pub fn preview_activation_column(&self) -> usize {
        self.schema().preview_activation_column
    }

    pub fn max_navigable_column(&self) -> usize {
        match self.schema().layout {
            InlineInteractiveLayout::Compact => 0,
            InlineInteractiveLayout::ThreeColumn => 2,
        }
    }

    pub fn header_layout(&self, preview: bool) -> ([&'static str; 3], [usize; 3]) {
        if self.uses_compact_navigation() {
            (
                [self.primary_label(), self.secondary_label(preview), ""],
                [0, 0, 0],
            )
        } else if preview {
            (
                [
                    self.secondary_label(true),
                    self.primary_label(),
                    self.tertiary_label(),
                ],
                [1, 0, 2],
            )
        } else {
            (
                [
                    self.primary_label(),
                    self.secondary_label(false),
                    self.tertiary_label(),
                ],
                [0, 1, 2],
            )
        }
    }

    pub fn filter_text(&self, entry: &PickerEntry) -> String {
        if self.is_agent_target_picker() {
            let model = entry
                .active_option()
                .map(|option| option.provider.as_str())
                .unwrap_or("");
            let config = entry
                .active_option()
                .map(|option| option.api_method.as_str())
                .unwrap_or("");
            let detail = entry
                .active_option()
                .map(|option| option.detail.as_str())
                .unwrap_or("");
            format!("{} {} {} {}", entry.name, model, config, detail)
        } else {
            self.kind.filter_text(entry)
        }
    }

    pub fn primary_label(&self) -> &'static str {
        self.schema().primary_label
    }

    pub fn secondary_label(&self, preview: bool) -> &'static str {
        let schema = self.schema();
        if preview {
            schema.secondary_preview_label
        } else {
            schema.secondary_label
        }
    }

    pub fn tertiary_label(&self) -> &'static str {
        self.schema().tertiary_label
    }

    pub fn shows_default_shortcut_hint(&self) -> bool {
        self.schema().shows_default_shortcut_hint
    }
}
