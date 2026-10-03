// Vendored from jcode (crates/jcode-tui/src/tui/info_widget.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; PARTIAL —
// only the referenced symbols; the full info_widget surface (19 files) lands
// at W7 and replaces this partial. See jcode_app/mod.rs for scope.
//! Included: WidgetKind (:87), Side (:229, plus impl), WidgetPlacement (:259),
//! AuthMethod (:340), UsageProvider (:313), UsageInfo (:367), CacheHitInfo (:402),
//! MemoryInfo (:508), GitInfo (:555) + RecentCommit (:577) + DirtyFile (:592) +
//! is_interesting impl (:622), AmbientWidgetData (:633), InfoWidgetData (:651),
//! CompactionInfo (:722).
//! [port-excision] InfoWidgetData's `diagrams`/`workspace_rows` fields and the
//! `impl InfoWidgetData` block (is_empty/has_data_for — unreferenced) are dropped:
//! their payload types live in the cut mermaid engine and the unported
//! workspace_map module. `Margins` re-export (from info_widget_layout, upstream
//! :262) is not ported. `swarm_gallery` (info_widget_swarm_gallery.rs) is NOT
//! ported — its renderer (jcode_tui_render::swarm_gallery) is [port-excision] in
//! operant's jcode_render; render_swarm_chat_card_lines is recorded unresolved.
use crate::tui::jcode_app::ambient::AmbientStatus;
use crate::tui::jcode_app::memory::MemoryActivity;
use crate::tui::jcode_app::protocol::SwarmMemberStatus;
use crate::tui::jcode_app::prompt::ContextInfo;
use crate::tui::jcode_app::todo::{TodoGoal, TodoItem};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::time::Duration;


// [port-decision] dedup: two upstream sources each derived these traits;
// moved WidgetKind/UsageInfo derives onto their items per upstream :85-87,:363-365.

/// A placed widget with its location and type
#[derive(Debug, Clone)]
pub struct WidgetPlacement {
    pub kind: WidgetKind,
    pub rect: Rect,
    pub side: Side,
}

// [port-decision] leaf ports: ui_viewport's margin settlement needs these types.
// `Margins` verbatim from jcode-tui/src/tui/info_widget_layout.rs:47-70
// (upstream re-exported it from here at info_widget.rs:265 — the re-export is
// realized as a direct definition since info_widget_layout is not ported);
// `GraphNode`/`GraphEdge` verbatim from jcode-tui-core/src/graph_topology.rs:1-20
// (upstream re-exported via info_widget_graph.rs:3).
/// Margin information for layout calculation.
#[derive(Debug, Clone, Default)]
pub struct Margins {
    /// Free widths on the right side for each row.
    pub right_widths: Vec<u16>,
    /// Free widths on the left side for each row (only populated in centered mode).
    pub left_widths: Vec<u16>,
    /// Whether we're in centered mode.
    pub centered: bool,
    /// Look-ahead "reliable" free widths: per row, the width that stays free across
    /// a small band of upcoming/recent scroll lines. When non-empty these gate where
    /// *new* widgets may dock (Phase 2) so a freshly placed widget won't be covered
    /// by a long line a frame later. Pinned widgets (Phase 1) still size themselves
    /// to the instantaneous `right_widths`/`left_widths` for full coverage. Empty =
    /// fall back to the instantaneous widths (no look-ahead).
    pub right_reliable: Vec<u16>,
    pub left_reliable: Vec<u16>,
    /// Absolute transcript line shown on the first visible row this frame. Lets the
    /// placement engine translate a content-anchored widget by the scroll delta so
    /// it rides the transcript instead of holding a fixed screen row.
    pub scroll_top: usize,
}

#[derive(Debug, Clone)]
pub struct GraphNode {
    /// Stable node ID from memory graph (mem:*, tag:*, cluster:*)
    pub id: String,
    /// Human-readable display label
    pub label: String,
    /// Category: "fact", "preference", "correction", "tag"
    pub kind: String,
    /// Whether this node is a memory (vs tag/cluster)
    pub is_memory: bool,
    /// Whether this node is active (superseded memories are inactive)
    pub is_active: bool,
    /// Effective confidence score (0.0-1.0)
    pub confidence: f32,
    /// Number of connections (degree)
    pub degree: usize,
}

#[derive(Debug, Clone)]
pub struct GraphEdge {
    /// Source index into MemoryInfo::graph_nodes
    pub source: usize,
    /// Target index into MemoryInfo::graph_nodes
    pub target: usize,
    /// Edge kind (has_tag, supersedes, contradicts, ...)
    pub kind: String,
}

/// Types of info widgets that can be displayed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetKind {
    /// Combined overview to reduce scattered widgets
    Overview,
    /// Niri-style workspace map preview
    WorkspaceMap,
    /// Todo list with progress
    Todos,
    /// Memory sidecar activity
    MemoryActivity,
    /// Subagents/sessions status
    SwarmStatus,
    /// Background work indicator
    BackgroundTasks,
    /// Conversation context compaction status
    Compaction,
    /// Subscription quota bars
    UsageLimits,
    /// Session-level KV cache hit ratio
    KvCache,
    /// Runtime details: service tier, route, transport, throughput, session
    /// (the status line owns model, effort, provider, and auth)
    ModelInfo,
    /// Mermaid diagrams
    Diagrams,
    /// Ambient mode status
    AmbientMode,
    /// Rotating tips/shortcuts
    Tips,
    /// Changes: the dirty file list (the status line owns branch and counts)
    GitStatus,
    /// Commits: recent history on the current branch
    Commits,
}


/// Which side of the screen a widget is on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}


impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}


/// Which provider the usage info is for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageProvider {
    #[default]
    None,
    /// Anthropic/Claude OAuth (shows subscription usage)
    Anthropic,
    /// OpenAI/Codex OAuth (shows subscription usage)
    OpenAI,
    /// OpenRouter/API-key providers (shows token costs)
    CostBased,
    /// GitHub Copilot (shows session token counts, no cost)
    Copilot,
}


impl UsageProvider {
    pub fn label(&self) -> &'static str {
        match self {
            UsageProvider::None => "",
            UsageProvider::Anthropic => "Anthropic",
            UsageProvider::OpenAI => "OpenAI",
            UsageProvider::CostBased => "",
            UsageProvider::Copilot => "Copilot",
        }
    }
}


/// Swarm/subagent status for the info widget
#[derive(Debug, Default, Clone)]
pub struct SwarmInfo {
    /// Number of sessions in the same swarm (same working directory)
    pub session_count: usize,
    /// Current subagent status (from Task tool execution)
    pub subagent_status: Option<String>,
    /// Number of connected clients (server mode)
    pub client_count: Option<usize>,
    /// List of session names in the swarm
    pub session_names: Vec<String>,
    /// Swarm member lifecycle status updates
    pub members: Vec<SwarmMemberStatus>,
    /// Agents this session manages (spawn-subtree filtered), shown in the
    /// swarm dock widget. Empty = no dock.
    pub managed_members: Vec<SwarmMemberStatus>,
    /// Selected agent index in the dock (display order), mirrors the inline
    /// swarm panel selection so both surfaces agree.
    pub selected: usize,
    /// Whether the swarm panel/dock has keyboard focus.
    pub focused: bool,
    /// Swarm plan progress (completed, running, total), when a plan is active.
    pub plan_progress: Option<(u32, u32, u32)>,
    /// Spinner frame for animating active agents' status glyphs.
    pub spinner_frame: usize,
}


/// Background task status for the info widget
#[derive(Debug, Default, Clone)]
pub struct BackgroundInfo {
    /// Number of running background tasks
    pub running_count: usize,
    /// Names of running tasks (e.g., "bash", "task")
    pub running_tasks: Vec<String>,
    /// Compact summary of the most recent task progress
    pub progress_summary: Option<String>,
    /// Detailed display for the most recent task progress
    pub progress_detail: Option<String>,
    /// Memory agent status
    pub memory_agent_active: bool,
    /// Memory agent turn count
    pub memory_agent_turns: usize,
}

/// Subscription usage info for the info widget
#[derive(Debug, Default, Clone)]
pub struct UsageInfo {
    /// Which provider this usage is for
    pub provider: UsageProvider,
    /// Primary subscription window label. OpenAI reports this dynamically.
    pub primary_limit_label: Option<String>,
    /// Primary window utilization (0.0-1.0) - for OAuth providers
    pub five_hour: f32,
    /// Primary reset timestamp (RFC3339), if known
    pub five_hour_resets_at: Option<String>,
    /// Secondary subscription window label, when one exists.
    pub secondary_limit_label: Option<String>,
    /// Secondary window utilization (0.0-1.0) - for OAuth providers
    pub seven_day: f32,
    /// Secondary reset timestamp (RFC3339), if known
    pub seven_day_resets_at: Option<String>,
    /// Codex Spark window utilization (0.0-1.0), if available
    pub spark: Option<f32>,
    /// Codex Spark reset timestamp (RFC3339), if known
    pub spark_resets_at: Option<String>,
    /// Total cost in USD - for API-key providers (OpenRouter, direct API key)
    pub total_cost: f32,
    /// Input tokens used - for cost calculation
    pub input_tokens: u64,
    /// Output tokens used - for cost calculation
    pub output_tokens: u64,
    /// Cache read tokens (from cache, cheaper) - for API-key providers
    pub cache_read_tokens: Option<u64>,
    /// Cache write tokens (creating cache, more expensive) - for API-key providers
    pub cache_write_tokens: Option<u64>,
    /// Output tokens per second (live streaming)
    pub output_tps: Option<f32>,
    /// Whether data was successfully fetched / available to show
    pub available: bool,
}


/// Authentication method used to access the model
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMethod {
    #[default]
    Unknown,
    /// Generic API key auth for API-backed providers without a provider-specific auth widget variant
    ApiKey,
    /// Anthropic OAuth (Claude Code CLI style)
    AnthropicOAuth,
    /// Anthropic API key
    AnthropicApiKey,
    /// OpenAI OAuth (Codex style)
    OpenAIOAuth,
    /// OpenAI API key
    OpenAIApiKey,
    /// OpenRouter API key
    OpenRouterApiKey,
    /// OpenCode API key
    OpenCodeApiKey,
    /// GitHub Copilot OAuth
    CopilotOAuth,
    /// Google Gemini OAuth
    GeminiOAuth,
}


/// Session-level KV cache telemetry for providers that report cache usage.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheHitInfo {
    /// Sum of per-request full prompt sizes, never inferred from aggregate counters.
    pub prompt_tokens: Option<u64>,
    pub last_prompt_tokens: Option<u64>,
    /// Input tokens from completed API requests that included explicit cache telemetry.
    pub reported_input_tokens: u64,
    /// Tokens read from provider KV/prefix cache across this session.
    pub read_tokens: u64,
    /// Tokens written/created in provider cache across this session, when reported.
    pub creation_tokens: u64,
    /// Approximate reusable prefix tokens expected to be cache-readable.
    pub optimal_input_tokens: u64,
    /// Input tokens from the latest completed request with cache telemetry.
    pub last_reported_input_tokens: Option<u64>,
    /// Cached input tokens read on the latest completed request with cache telemetry.
    pub last_read_tokens: Option<u64>,
    /// Tokens written/created in provider cache on the latest completed request.
    pub last_creation_tokens: Option<u64>,
    /// Approximate reusable prefix tokens expected on the latest completed request.
    pub last_optimal_input_tokens: Option<u64>,
    /// Recent attributed misses with estimated cacheable tokens not read.
    pub miss_attributions: Vec<CacheMissAttribution>,
}

// [port-decision] leaf port: `CacheMissAttribution` verbatim from jcode-tui/src/tui/info_widget.rs:446-451
// (its definition was dropped when the concatenated sources were deduped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMissAttribution {
    pub turn_number: usize,
    pub call_index: u16,
    pub missed_tokens: u64,
    pub reason: String,
}


/// Memory statistics for the info widget
#[derive(Debug, Default, Clone)]
pub struct MemoryInfo {
    /// Total memory count (project + global)
    pub total_count: usize,
    /// Project-specific memory count
    pub project_count: usize,
    /// Global memory count
    pub global_count: usize,
    /// Count by category
    pub by_category: HashMap<String, usize>,
    /// Whether sidecar is available
    pub sidecar_available: bool,
    /// Whether the memory feature is disabled for this session.
    /// When true, stored counts are still shown but recall/extraction are off.
    pub disabled: bool,
    /// Selected sidecar model/backend label for memory work
    pub sidecar_model: Option<String>,
    /// Current memory activity
    pub activity: Option<MemoryActivity>,
    /// Graph topology for visualization (node positions + edges)
    pub graph_nodes: Vec<GraphNode>,
    /// Directed edges into graph_nodes
    pub graph_edges: Vec<GraphEdge>,
}


/// Git repository status for the info widget
#[derive(Debug, Clone, Default)]
pub struct GitInfo {
    pub branch: String,
    pub modified: usize,
    pub staged: usize,
    pub untracked: usize,
    pub ahead: usize,
    pub behind: usize,
    /// First few dirty paths with their porcelain status (capped).
    pub dirty_files: Vec<DirtyFile>,
    /// Total number of dirty paths, including those beyond the cap.
    pub dirty_total: usize,
    /// Lines added across all dirty files (text files only).
    pub added_total: usize,
    /// Lines removed across all dirty files (text files only).
    pub removed_total: usize,
    /// Absolute repository root, used to match agent-edited paths.
    pub repo_root: Option<std::path::PathBuf>,
    /// Most recent commits on HEAD, newest first.
    pub recent_commits: Vec<RecentCommit>,
}


/// One commit for the Commits widget.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecentCommit {
    /// Abbreviated hash.
    pub hash: String,
    pub subject: String,
    /// Committer time, seconds since the Unix epoch.
    pub timestamp: i64,
    /// Not yet on the upstream branch.
    pub unpushed: bool,
    pub added: Option<usize>,
    pub removed: Option<usize>,
}


/// One dirty path from `git status --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirtyFile {
    /// Single-letter status shown in the Changes widget: `M`, `A`, `D`, `R`,
    /// `U` (conflict), or `?` (untracked).
    pub status: char,
    pub path: String,
    /// Lines added, `None` for binary or unknown.
    pub added: Option<usize>,
    /// Lines removed, `None` for binary or unknown.
    pub removed: Option<usize>,
    /// Last modification time, used for newest-first ordering.
    pub modified_at: Option<std::time::SystemTime>,
}


impl GitInfo {
    pub fn is_interesting(&self) -> bool {
        self.modified > 0
            || self.staged > 0
            || self.untracked > 0
            || self.ahead > 0
            || self.behind > 0
    }
}


/// Ambient mode status data for the info widget
#[derive(Debug, Clone)]
pub struct AmbientWidgetData {
    pub show_widget: bool,
    pub status: AmbientStatus,
    pub queue_count: usize,
    pub next_queue_preview: Option<String>,
    pub reminder_count: usize,
    pub next_reminder_preview: Option<String>,
    pub last_run_ago: Option<String>,
    pub last_summary: Option<String>,
    pub next_wake: Option<String>,
    pub next_reminder_wake: Option<String>,
    pub budget_percent: Option<f32>,
}


/// Data to display in the info widget
#[derive(Debug, Default, Clone)]
pub struct InfoWidgetData {
    pub todos: Vec<TodoItem>,
    /// Goal-level assessments (closed feedback loop and objective)
    /// keyed by todo group (`group: None` covers the ungrouped list). Empty
    /// when the session has no recorded goals or `todos` is a swarm-plan
    /// projection.
    pub todo_goals: Vec<crate::tui::jcode_app::todo::TodoGoal>,
    /// True when `todos` is actually a projection of the shared swarm plan
    /// (task DAG) rather than this session's private todo list. The widget
    /// renders a "Plan" header instead of "Todos" so the two are not
    /// conflated.
    pub todos_are_swarm_plan: bool,
    pub context_info: Option<ContextInfo>,
    /// True when context state is being updated and no authoritative snapshot is available.
    pub context_info_stale: bool,
    pub queue_mode: Option<bool>,
    pub context_limit: Option<usize>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
    pub native_compaction_mode: Option<String>,
    pub native_compaction_threshold_tokens: Option<usize>,
    pub session_count: Option<usize>,
    pub session_name: Option<String>,
    /// Current working directory for this session.
    pub working_dir: Option<String>,
    pub client_count: Option<usize>,
    /// Memory system statistics
    pub memory_info: Option<MemoryInfo>,
    /// Swarm/subagent status
    pub swarm_info: Option<SwarmInfo>,
    /// Background tasks status
    pub background_info: Option<BackgroundInfo>,
    /// Subscription usage info
    pub usage_info: Option<UsageInfo>,
    /// Show consumed rather than remaining percentages in usage limits.
    pub usage_display_used: bool,
    /// Streaming output tokens per second (approximate)
    pub tokens_per_second: Option<f32>,
    /// Active provider name (openrouter/openai/anthropic/...)
    pub provider_name: Option<String>,
    /// Authentication method used to access the model
    pub auth_method: AuthMethod,
    /// Upstream provider (e.g., which OpenRouter provider served the request: fireworks, etc.)
    pub upstream_provider: Option<String>,
    /// Active connection type (websocket/https/etc.)
    pub connection_type: Option<String>,
    /// Mermaid diagrams to display
    // [port-excision] diagrams: Vec<DiagramInfo> — mermaid engine cut (W9)
    /// Visible Niri-style workspace rows
    // [port-excision] workspace_rows: workspace_map unported
    /// Lightweight animation tick for workspace map rendering
    pub workspace_animation_tick: u64,
    /// Ambient mode status
    pub ambient_info: Option<AmbientWidgetData>,
    /// Actual API-reported context tokens (from last streaming response)
    /// When available, this is more accurate than the char-based estimate in context_info
    pub observed_context_tokens: Option<u64>,
    /// Session-level cache read ratio, when the active provider reports cache telemetry.
    pub cache_hit_info: Option<CacheHitInfo>,
    /// Conversation compaction status, shown as a compact rounded status card.
    pub compaction_info: Option<CompactionInfo>,
    /// Whether background compaction is currently in progress
    pub is_compacting: bool,
    /// Git repository status
    pub git_info: Option<GitInfo>,
    /// Absolute paths the agent edited this session (edit-style tool calls),
    /// used to mark agent changes in the Changes widget.
    pub agent_edited: std::sync::Arc<std::collections::HashSet<std::path::PathBuf>>,
}


#[derive(Clone, Debug)]
pub struct CompactionInfo {
    pub is_compacting: bool,
    pub compacted_messages: usize,
    pub active_messages: usize,
    pub summary_chars: usize,
    pub mode: String,
}


// --- crates/jcode-tui/src/tui/info_widget_tips.rs (occasional_status_tip closure) -----
// Vendored from jcode (crates/jcode-tui/src/tui/info_widget_tips.rs +
// tui/info_widget_text.rs), MIT License, Copyright (c) 2025 Jeremy Huang. Ported
// verbatim @ 0a9dc7805; partial — only the occasional_status_tip closure the
// status line calls: consts (tips.rs :3-:6), Tip (:8), all_tips (:12),
// TIP_STATE (:37), current_tip (:39), occasional_status_tip (:58), plus
// truncate_smart/truncate_chars (text.rs :1-:26) which it calls.
// [port-excision] wrap_tip_text / render_tips_widget and the other text.rs
// helpers are not ported.
// [port-unresolved] all_tips still calls `jcode_tui_core::keybind::alt_label()`
// verbatim — the keybind leaf (jcode-tui-core/src/keybind.rs, 1,207 lines) is
// the integrator's to port or excise per plan; the same applies to the
// `crate::tui::keybind::*` call sites in ui_overlays.rs / ui_input.rs.
use std::sync::Mutex;
use std::time::Instant;

const TIP_CYCLE_SECONDS: u64 = 15;
const STATUS_TIP_PERIOD_SECONDS: u64 = 90;
const STATUS_TIP_OFFSET_SECONDS: u64 = 28;
const STATUS_TIP_SHOW_SECONDS: u64 = 12;


struct Tip {
    text: String,
}


fn all_tips() -> Vec<Tip> {
    let mut tips = vec![
        "Ctrl+J / Ctrl+K to jump between user prompts (Cmd+J / Cmd+K on macOS terminals that forward Command)",
        "Ctrl+Shift+J / Ctrl+Shift+K to scroll the chat down and up one line",
        "Ctrl+G to bookmark your scroll position - press again to teleport back",
        "Swarms form automatically when multiple sessions share a repo - they coordinate plans, share context, and track file conflicts",
        "Memories are stored in a graph with semantic embeddings - recall finds related facts even if you use different words",
        "Ambient mode runs background cycles while you're away - maintaining memories, compacting context, and doing proactive work",
        "Ambient cycles can email you a summary and you can reply with directives for the next run",
        "Alt+B moves a long-running tool to the background - the agent continues and can check on it later with the `bg` tool",
        "Most terminals can be configured to copy text on highlight - no Ctrl+C needed. Check your terminal's settings for 'copy on select'",
        "Alt+G (or /diff) cycles diff mode: Off, Inline, Pinned, File. Shift+Tab cycles favorited models. Pinned shows all diffs in a side pane. File shows the full file with changes highlighted, synced to your scroll position",
    ];
    if crate::tui::jcode_app::config_shim::config().features.mermaid {
        tips.insert(3, "```mermaid code blocks render as diagrams");
    }
    // Mac keyboards label this modifier ⌥, not Alt, so rewrite hints there.
    let alt = crate::tui::jcode_app::keybind::alt_label();
    tips.into_iter()
        .map(|text| Tip {
            text: text.replace("Alt+", &format!("{alt}+")),
        })
        .collect()
}


static TIP_STATE: Mutex<Option<(usize, Instant)>> = Mutex::new(None);

fn current_tip(_max_width: usize) -> Tip {
    let tips = all_tips();
    let mut guard = TIP_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let (idx, last) = guard.get_or_insert_with(|| (0, now));

    let should_advance = now.duration_since(*last).as_secs() >= TIP_CYCLE_SECONDS;
    if should_advance {
        *idx = (*idx + 1) % tips.len();
        *last = now;
    }

    let i = *idx % tips.len();
    drop(guard);
    Tip {
        text: tips[i].text.clone(),
    }
}

// [port-decision] dedup: info_widget.rs defined current_tip twice across
// concatenated sources; kept the first (identical) copy.


pub(crate) fn occasional_status_tip(max_width: usize, elapsed_secs: u64) -> Option<String> {
    if max_width < 16 {
        return None;
    }

    let cycle_pos = elapsed_secs % STATUS_TIP_PERIOD_SECONDS;
    let show_until = STATUS_TIP_OFFSET_SECONDS + STATUS_TIP_SHOW_SECONDS;
    if cycle_pos < STATUS_TIP_OFFSET_SECONDS || cycle_pos >= show_until {
        return None;
    }

    let prefix = "💡 ";
    let available = max_width.saturating_sub(prefix.chars().count());
    if available < 12 {
        return None;
    }

    let tip = current_tip(available);
    Some(format!(
        "{}{}",
        prefix,
        truncate_smart(&tip.text, available)
    ))
}

pub(crate) fn truncate_smart(s: &str, max_len: usize) -> String {
    let char_len = s.chars().count();
    if char_len <= max_len {
        return s.to_string();
    }
    if max_len <= 3 {
        return "...".to_string();
    }

    let target = max_len - 3;
    let prefix = truncate_chars(s, target);

    if let Some(pos) = prefix.rfind(' ') {
        let before = &prefix[..pos];
        let pos_chars = before.chars().count();
        if pos_chars > target / 2 {
            return format!("{}...", before);
        }
    }
    format!("{}...", prefix)
}


pub(crate) fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

