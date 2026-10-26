//! Gateway Runner
//!
//! Manages a singleton Gateway instance that persists across CLI commands
//! within a single process. Provides start/stop/restart lifecycle management
//! backed by configuration and the `operant_core::gateway` module.

use anyhow::{Context, Result};
use base64::Engine;
use operant_core::agent::{AgentEvent, OperantAgent};
use operant_core::config::AppConfig;
use operant_core::gateway::{
    DiscordAdapter, EmailAdapter, Gateway, GatewayConfig, IncomingMessage, MessageHandler,
    OutgoingMessage, PlatformAdapter, SlackAdapter, SmsAdapter, TelegramAdapter, WebhookAdapter,
    WhatsAppAdapter,
};
use operant_core::gateway_pipeline::{MessagePipeline, PipelineAction};
use operant_core::interrupt::InterruptFlag;
use operant_core::redaction::redact_sensitive_text_if_enabled;

use crate::gateway_commands::{
    CommandContext, CommandMatch, handle_command, resolve_command, telegram_bot_commands,
};
use operant_core::mcp::McpManager;
use operant_core::observer::ObserverEvent;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

/// The active gateway turn's target: (platform, channel_id, telegram
/// thread_id). Thread id is None for non-forum platforms/chat.
type CurrentChannel = (String, String, Option<i64>);

/// Map key for per-thread state: (platform, channel_id, telegram thread_id).
/// Thread id is None for non-forum platforms/chat. Keying stream/progress
/// state by thread keeps concurrent forum topics isolated (hermes session
/// isolation parity).
type ThreadKey = (String, String, Option<i64>);

/// (platform, channel_id, thread_id) -> streamed Telegram message id. Set by
/// the event receiver when the first streaming chunk is delivered; the
/// dispatch loop consumes it after route_message so the final response is
/// not sent a second time.
type StreamDeliveryMap = Arc<Mutex<HashMap<ThreadKey, String>>>;

/// (platform, channel_id, thread_id) -> exit reason of the turn that just
/// completed (BUGS.md S5). The event receiver writes it on
/// `AgentEvent::Done`; the handler's turn-end block reads it so a
/// grace-call or circuit-breaker exit surfaces as an operator-visible
/// "stopped early" notice instead of looking like a full answer (the
/// 25-day silence-after-degenerate-warning class).
type ExitReasonMap = Arc<Mutex<HashMap<ThreadKey, operant_core::agent::TurnExitReason>>>;

/// iter-632 (Wave-4 metering wire): one turn's accumulated model usage,
/// folded per model call by the event receiver and drained ONCE by the
/// turn-end block into the session-store accumulator — the row
/// `employee_window_usage` rolls up for budget enforcement.
#[derive(Debug, Clone, Copy, Default)]
struct TurnUsage {
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
}

type TurnUsageMap = Arc<Mutex<HashMap<ThreadKey, TurnUsage>>>;

/// D-2 (owner ruling 2026-10-08): the verdict for an UNGOVERNED unattended
/// permission request that has no interactive approver to answer it, under
/// `[bylaws].unattended_posture`. This knob, and only this knob, manages the
/// unattended posture — governed seats never reach it (their policy row
/// already clamps unattended runs in `agent/stream.rs`, F2).
///
/// Returns `None` to auto-approve (`yolo`, byte-identical legacy behavior)
/// or `Some(reason)` to deny with the reason. A typo'd spelling FAILS
/// CLOSED — the same read-error discipline as `seat_policies`: a typo'd
/// `lockdonw` silently re-opening unattended dangerous tools would be the
/// exact widening the bylaws exists to prevent.
/// Whether a streamed message counts as DELIVERED for the dispatch loop.
///
/// The dispatch loop skips its full send when the streaming path already
/// delivered the answer. That suppression is only correct for a platform
/// that can EDIT the message it sent: on Discord and Slack (before
/// iter-1063) the later chunks' edits were no-ops, so the user received
/// the FIRST CHUNK of the answer and never the rest.
///
/// Only Telegram overrode `send_message_return_id`; the default sends the
/// message and returns an EMPTY id, which means "this platform cannot
/// edit". So an empty id must NOT suppress the dispatch loop's full send.
///
/// The empty id is still kept as the streaming path's `stream_msg_id`
/// sentinel — it keeps the later ticks on the EDIT branch instead of
/// sending a new partial message every 500ms. This function decides only
/// whether to record the DELIVERY.
fn should_record_stream_delivery(message_id: &str) -> bool {
    !message_id.is_empty()
}

fn unattended_no_channel_verdict(posture_raw: &str) -> Option<String> {
    use operant_core::org::seat_policy::SeatMode;
    use std::str::FromStr;
    match SeatMode::from_str(posture_raw) {
        Ok(SeatMode::Yolo) => None,
        Ok(mode) => Some(format!(
            "Permission denied by unattended posture: \
             [bylaws].unattended_posture is `{}` and no interactive \
             approver is attached to this run. Give the seat a governed \
             policy (`operant cron create --seat-mode` or /grant) so its \
             approver can mint a grant between runs, or set the posture \
             back to `yolo` to restore auto-approval",
            mode.as_str()
        )),
        Err(e) => Some(format!(
            "Permission denied (fail-closed): the unattended posture is \
             unreadable — {}. Fix [bylaws].unattended_posture; `yolo` \
             restores auto-approval",
            e
        )),
    }
}

// Tool preview helpers (module-scope so the preview line is testable alone)

fn tool_emoji(name: &str) -> &'static str {
    match name {
        // ── Terminal / Code ──
        "terminal" | "bash" | "shell" | "aft_bash" | "process" | "docker" => "\u{1F4BB}",
        "code_execution" | "echo" | "local" | "slow" | "fast" => "\u{1F4BB}",
        // ── Web / Search ──
        "web_search" | "web_search_" | "tavily" | "tavily_search" | "sourcehound" => "\u{1F50D}",
        "web_fetch" | "web_scrape" | "tavily_extract" | "tavily_crawl" | "web_extract"
        | "web_crawl" => "\u{1F310}",
        "search" | "find" | "file_search" => "\u{1F50E}",
        // ── File I/O ──
        "read" | "glob" | "grep" | "ast_grep" | "look_at" => "\u{1F4D6}",
        "file_read"
        | "file_list"
        | "aft_read"
        | "aft_outline"
        | "aft_grep"
        | "aft_glob"
        | "aft_search"
        | "aft_inspect"
        | "aft_status"
        | "aft_callers"
        | "aft_zoom"
        | "aft_ast_search"
        | "aft_checkpoint_paths"
        | "aft_list_checkpoints" => "\u{1F4D6}",
        "write" | "edit" | "create" | "patch" => "\u{270F}\u{FE0F}",
        "file_write" | "aft_write" | "aft_edit" | "aft_apply_patch" | "aft_ast_replace" => {
            "\u{270F}\u{FE0F}"
        }
        // ── Memory ──
        "memory" | "memory_search" | "memory_store" | "memory_recall" => "\u{1F9E0}",
        // ── LCM (Lossless Context Memory) ──
        "lcm_recall" | "lcm_stats" | "lcm_load_session" | "lcm_recent" | "lcm_vector_recall"
        | "lcm_recall_round" | "lcm_assert" | "lcm_doctor" => "\u{1F9E0}",
        // ── Skills ──
        "skills_list" | "skill_view" | "skill_manage" | "learning_manage" => "\u{1F4DA}",
        // ── Session ──
        "session_search" | "session_insights" => "\u{1F4CB}",
        // ── Config ──
        "config_manage" => "\u{2699}\u{FE0F}",
        // ── Cron / Scheduling ──
        "cron" | "todo" | "kanban" => "\u{23F0}",
        // ── Browser ──
        "browser" | "browser_dialog" | "browser_cdp" => "\u{1F5A5}\u{FE0F}",
        // ── Approval / Clarify ──
        "approval_request" | "clarify" | "verify_task" => "\u{2753}",
        // ── Communication ──
        "chat" | "send_message" | "message" | "notify" | "react_to_message" => "\u{1F4AC}",
        "discord" | "discord_admin" | "homeassistant" => "\u{1F4AC}",
        // ── HTTP / API ──
        "http_request" => "\u{1F310}",
        // ── Image / Media ──
        "image" | "screenshot" | "draw" | "image_generate" | "vision_analyze" | "video_analyze" => {
            "\u{1F5BC}\u{FE0F}"
        }
        "transcribe_audio" | "text_to_speech" => "\u{1F3A4}",
        // ── Spotify ──
        "spotify_playback" | "spotify_search" | "spotify_devices" | "spotify_albums"
        | "spotify_playlists" | "spotify_library" | "spotify_queue" => "\u{1F3B5}",
        // ── SSH / Remote ──
        "ssh" => "\u{1F510}",
        // ── Checkpoint / Git ──
        "checkpoint" | "aft_checkpoint" | "aft_restore_checkpoint" | "aft_undo" => "\u{1F4BE}",
        "github" | "gh" | "git" => "\u{1F5A5}\u{FE0F}",
        // ── Thinking ──
        "think" | "reason" | "sequentialthinking" => "\u{1F4AD}",
        "plan" | "strategy" => "\u{1F9F0}",
        // ── Debug / Env ──
        "debug_env" | "debug_system" | "env_probe" | "osv_check" | "tool_backend" => "\u{1F527}",
        // ── Errors / Done ──
        "error" | "fail" => "\u{274C}",
        "done" | "complete" | "finish" => "\u{2705}",
        _ => "\u{2699}\u{FE0F}",
    }
}

fn extract_tool_arg(name: &str, args: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(args) {
        let keys: &[&str] = match name {
            // Terminal / Code
            "terminal" | "bash" | "shell" | "aft_bash" | "code_execution" | "process" | "echo"
            | "docker" => &["command"],
            // Web / Search
            "web_search" | "web_search_" | "tavily" | "tavily_search" | "sourcehound" => &["query"],
            "web_fetch" | "web_scrape" | "tavily_extract" | "tavily_crawl" | "web_extract"
            | "web_crawl" | "http_request" => &["url"],
            // File read
            "read" | "look_at" | "file_read" | "aft_read" | "aft_outline" | "aft_inspect"
            | "aft_status" => &["file_path", "path", "pattern"],
            "glob" | "file_list" | "aft_glob" | "aft_list_checkpoints" | "aft_checkpoint_paths" => {
                &["pattern", "file_path"]
            }
            // File search
            "grep" | "search" | "ast_grep" | "file_search" | "aft_grep" | "aft_search"
            | "aft_callers" | "aft_zoom" | "aft_ast_search" => &["pattern", "query", "file_path"],
            // File write
            "write" | "create" | "file_write" | "aft_write" | "patch" | "aft_apply_patch"
            | "aft_ast_replace" => &["file_path", "path", "content"],
            "edit" | "aft_edit" => &["file_path", "path", "old_string"],
            // Memory
            "memory" | "memory_search" | "memory_recall" | "lcm_recall" | "lcm_vector_recall"
            | "lcm_recall_round" => &["query", "text"],
            "memory_store" | "lcm_assert" => &["content", "text"],
            // LCM
            "lcm_stats" | "lcm_load_session" | "lcm_recent" | "lcm_doctor" => {
                &["session_id", "action"]
            }
            // Session
            "session_search" | "session_insights" => &["query", "session_id"],
            // Skills
            "skills_list" | "skill_view" | "skill_manage" | "learning_manage" => {
                &["action", "skill_name", "name"]
            }
            // Config
            "config_manage" => &["action", "key", "path"],
            // Cron / Tasks
            "cron" => &["action", "schedule", "command"],
            "todo" | "kanban" => &["action", "task_id", "title"],
            // Browser
            "browser" => &["action", "url", "command"],
            "browser_dialog" | "browser_cdp" => &["action", "command"],
            // Approval / Clarify
            "approval_request" | "verify_task" => &["tool_name", "description"],
            "clarify" => &["question", "choices"],
            // Communication
            "chat" | "send_message" | "notify" | "react_to_message" => {
                &["message", "content", "text"]
            }
            // Image / Media
            "image" | "screenshot" | "image_generate" | "vision_analyze" => {
                &["path", "url", "prompt"]
            }
            "video_analyze" | "transcribe_audio" | "text_to_speech" => &["path", "url", "text"],
            // Spotify
            "spotify_playback" | "spotify_search" | "spotify_devices" | "spotify_albums"
            | "spotify_playlists" | "spotify_library" | "spotify_queue" => &["query", "action"],
            // Checkpoint / Git
            "checkpoint" | "aft_checkpoint" | "aft_restore_checkpoint" | "aft_undo" => {
                &["action", "path"]
            }
            "github" | "gh" => &["query", "command", "repo"],
            // Thinking
            "think" | "sequentialthinking" | "reason" => &["thought"],
            // Debug
            "debug_env" | "debug_system" | "env_probe" | "osv_check" | "tool_backend" => {
                &["variable", "key"]
            }
            // SSH
            "ssh" => &["command", "host"],
            _ => &[
                "query",
                "command",
                "text",
                "url",
                "path",
                "file_path",
                "input",
                "content",
                "message",
            ],
        };
        for key in keys {
            if let Some(val) = json.get(*key).and_then(|v| v.as_str()) {
                let truncated = if val.len() > 100 {
                    format!("{}...", &val[..100])
                } else {
                    val.to_string()
                };
                return Some(truncated.replace('\n', "\\n"));
            }
        }
        // Fallback: show first string value from any key
        if let Some(obj) = json.as_object() {
            for (_key, val) in obj.iter() {
                if let Some(s) = val.as_str() {
                    let truncated = if s.len() > 100 {
                        format!("{}...", &s[..100])
                    } else {
                        s.to_string()
                    };
                    return Some(truncated.replace('\n', "\\n"));
                }
            }
        }
    }
    None
}

fn tool_preview_line(name: &str, args: &str) -> String {
    let emoji = tool_emoji(name);
    let parsed = serde_json::from_str::<serde_json::Value>(args).ok();
    // The model's display label (see `operant_core::schema::TOOL_INTENT_KEY`)
    // is WHAT this call is doing, so it leads the line; the technical
    // argument follows. A call that carries no intent falls back to the
    // technical argument alone — what this printed before.
    let intent = parsed.as_ref().and_then(|json| {
        json.get(operant_core::schema::TOOL_INTENT_KEY)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|intent| !intent.is_empty())
            .map(|intent| truncate_preview_value(intent, 120))
    });
    let technical = extract_tool_arg(name, args).map(|arg| truncate_preview_value(&arg, 120));
    match (intent, technical) {
        (Some(intent), Some(arg)) if arg != intent => {
            format!("{} {} · {}: {}", emoji, name, intent, arg)
        }
        (Some(intent), _) => format!("{} {} · {}", emoji, name, intent),
        (None, Some(arg)) => format!("{} {}: {}", emoji, name, arg),
        (None, None) => format!("{} {}...", emoji, name),
    }
}

/// Cap a tool preview's segment so a long command, path or intent cannot
/// overflow a channel's message limit. Byte-truncation on a char boundary
/// is fine: this is a preview, and the fallback is an ellipsis.
fn truncate_preview_value(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let head: String = value.chars().take(max).collect();
    format!("{}...", head)
}

#[cfg(test)]
mod unattended_posture_tests {
    use super::unattended_no_channel_verdict;

    #[test]
    fn yolo_keeps_the_legacy_auto_approve() {
        // The shipped default MUST stay byte-identical: `None` means the
        // receiver falls through to the legacy AllowSession arm.
        assert!(unattended_no_channel_verdict("yolo").is_none());
    }

    #[test]
    fn every_governing_mode_denies_with_the_knob_in_the_reason() {
        for mode in ["standard", "scoped", "lockdown"] {
            let Some(reason) = unattended_no_channel_verdict(mode) else {
                panic!("{mode} must deny");
            };
            assert!(
                reason.contains(mode),
                "the deny reason must name the posture: {reason}"
            );
            assert!(
                reason.contains("unattended_posture"),
                "the deny reason must name the knob: {reason}"
            );
        }
    }

    #[test]
    fn a_typo_fails_closed_instead_of_silently_reopening_tools() {
        let Some(reason) = unattended_no_channel_verdict("lockdonw") else {
            panic!("a typo'd posture must deny");
        };
        assert!(reason.contains("fail-closed"), "got: {reason}");
    }
}

/// iter-632: drain one turn's accumulated usage into the session store's
/// metering accumulator (`gateway_sessions.total_tokens` /
/// `estimated_cost_usd` — the columns Wave 4's budget rollup sums).
/// Fail-open on telemetry (bylaws invariant): a metering write must never
/// fail a turn, so the store error is swallowed and logged by the caller.
/// The periodic sweep that drops idle gateway sessions.
///
/// A NAMED function at module scope, not an inline closure, so the wiring
/// is a thing a reader can find and a test can drive. The period is a
/// parameter so a test can run the REAL sweep on a real store instead of
/// re-implementing the prune — a test that calls `prune_old_entries`
/// directly proves the prune works, not that the gateway ever calls it,
/// and that difference is the whole defect.
async fn session_retention_sweep(
    store: std::sync::Arc<operant_core::PersistentSessionStore>,
    retention_days: u32,
    period: std::time::Duration,
) {
    let mut ticker = tokio::time::interval(period);
    // The first tick fires immediately; skip it so a just-started gateway
    // does not prune before it has seen a single message.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let removed = store.prune_old_entries(retention_days);
        if removed > 0 {
            tracing::info!(removed, retention_days, "Pruned idle gateway sessions");
        }
    }
}

/// How often the gateway sweeps its session store.
const SESSION_RETENTION_SWEEP_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(6 * 60 * 60);

async fn drain_turn_usage_into_store(
    store: &operant_core::gateway_session::PersistentSessionStore,
    map: &TurnUsageMap,
    key: ThreadKey,
    session_key: &str,
    seat: Option<&str>,
) {
    let Some(usage) = map.lock().await.remove(&key) else {
        return;
    };
    if usage.input_tokens == 0 && usage.output_tokens == 0 && usage.cost_usd == 0.0 {
        return;
    }
    if let Err(e) = store.update_tokens(
        session_key,
        usage.input_tokens,
        usage.output_tokens,
        0,
        0,
        usage.cost_usd,
    ) {
        tracing::warn!(%session_key, %e, "Wave-4 metering write failed (fail-open)");
    }
    // GW-1: the spend is charged to the seat the turn started under.
    if let Some(seat) = seat
        && let Err(e) = store.record_seat_spend(
            seat,
            usage.input_tokens + usage.output_tokens,
            usage.cost_usd,
        )
    {
        tracing::warn!(%session_key, %e, "seat spend ledger write failed (fail-open)");
    }
}

/// (platform, channel_id, thread_id) -> (message_id, tool_lines) for the
/// per-segment tool-progress timeline. Each TOOL GROUP (a run of tool calls
/// with no intervening text) owns one message; the group is closed when a
/// content bubble lands on the platform or the turn ends, so the next
/// group starts a fresh chronological message BELOW the content (hermes
/// `__reset__` linearization parity).
///
/// The per-turn status bubble is CAPPED: during degenerate model loops the
/// same tool can start thousands of times; keeping every line would blow
/// past Telegram's edit limit and turn one status message into a wall of
/// repeated lines. Beyond [`MAX_STATUS_LINES`] the oldest lines roll off
/// and a `… +N earlier` header counts them; identical consecutive calls
/// collapse into `(×N)` counters first.
type ProgressMap = HashMap<ThreadKey, GroupState>;

/// Max tool lines kept in the live status bubble before older ones roll off.
const MAX_STATUS_LINES: usize = 15;

/// State of the live tool-status bubble for one thread.
struct GroupState {
    msg_id: String,
    /// (line, ×count) — count >= 1. Repeated identical tool lines collapse
    /// into a ×N counter on the last line (hermes `__dedup__` parity)
    /// instead of piling duplicate rows during repetitive tool loops.
    lines: Vec<(String, usize)>,
    /// Lines rolled off the top by the MAX_STATUS_LINES cap.
    dropped: usize,
}

impl GroupState {
    fn push(&mut self, line: &str) {
        if let Some((l, n)) = self.lines.last_mut()
            && l == line
        {
            *n += 1;
            return;
        }
        self.lines.push((line.to_string(), 1));
        if self.lines.len() > MAX_STATUS_LINES {
            self.lines.remove(0);
            self.dropped += 1;
        }
    }

    fn render(&self) -> String {
        let mut out = String::new();
        if self.dropped > 0 {
            out.push_str(&format!("… +{} earlier\n", self.dropped));
        }
        for (i, (line, n)) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(line);
            if *n > 1 {
                out.push_str(&format!(" (×{})", n));
            }
        }
        out
    }
}
use tokio::sync::Mutex;
use tokio::sync::mpsc;

/// What the event receiver must do with a tool line on `ToolStart`.
#[derive(Debug, PartialEq, Eq)]
enum ProgressAction {
    /// No message exists for this tool group yet — send `body` as a NEW
    /// message, then call [`ToolProgressTracker::attach_message_id`] with
    /// the returned id so subsequent tool starts append to it.
    New { body: String },
    /// A message exists for this group — edit `msg_id` to `body` (the full
    /// group so far).
    Edit { msg_id: String, body: String },
    /// This tool call id was already announced (it is re-announced once its
    /// worker permit is acquired) — leave the platform message untouched.
    Skip,
}

/// Tracks per-thread tool-progress messages with hermes' linearization
/// rules (`send_progress_messages` + `__reset__` parity):
///   * one status bubble per TOOL GROUP — a group closes when a content
///     bubble lands on the platform, so the next tool opens a fresh bubble
///     BELOW the content instead of editing the stale bubble above it;
///   * repeated identical tool lines collapse into `(×N)` counters;
///   * the line cap keeps degenerate loops from growing the bubble forever.
struct ToolProgressTracker {
    msgs: ProgressMap,
    /// Tool call ids already turned into a status line. The agent announces
    /// each call twice (queued, then running on permit acquire); without this
    /// the second announcement would inflate the group's `×N` counter.
    announced: HashSet<String>,
}

impl ToolProgressTracker {
    fn new() -> Self {
        Self {
            msgs: HashMap::new(),
            announced: HashSet::new(),
        }
    }

    /// Record a tool line for `key`. Returns `New` when this starts a fresh
    /// group (no message yet — the previous group was closed by an
    /// intervening text/iteration/turn boundary), or `Edit` with the full
    /// accumulated group when the group already has a message. A repeated
    /// `tool_call_id` (the agent re-announces a call once its worker permit
    /// is acquired) is a no-op — `Skip`.
    fn on_tool_start(&mut self, key: &ThreadKey, tool_call_id: &str, line: &str) -> ProgressAction {
        if !self.announced.insert(tool_call_id.to_string()) {
            return ProgressAction::Skip;
        }
        match self.msgs.get_mut(key) {
            Some(g) => {
                g.push(line);
                ProgressAction::Edit {
                    msg_id: g.msg_id.clone(),
                    body: g.render(),
                }
            }
            None => ProgressAction::New {
                body: line.to_string(),
            },
        }
    }

    /// Id-less convenience for callers that have no tool call id (every call
    /// is then its own tool call, so the line is always recorded).
    #[cfg(test)]
    fn on_tool_start_anon(&mut self, key: &ThreadKey, line: &str) -> ProgressAction {
        let anon = format!("anon-{}", self.announced.len());
        self.on_tool_start(key, &anon, line)
    }

    /// After a `New` send succeeds, attach the message id + first line so
    /// the next tool start edits that message. Called only on success — on
    /// failure the group is left untracked (the old code did the same).
    fn attach_message_id(&mut self, key: &ThreadKey, msg_id: String, first_line: &str) {
        self.msgs.insert(
            key.clone(),
            GroupState {
                msg_id,
                lines: vec![(first_line.to_string(), 1)],
                dropped: 0,
            },
        );
    }

    /// Close the current tool group for `key`. The already-sent message
    /// stays in the chat; the NEXT `on_tool_start` for this key returns
    /// `New` and starts a fresh message. Idempotent.
    fn close_group(&mut self, key: &ThreadKey) {
        self.msgs.remove(key);
    }
}

/// Byte offset to split an oversized streaming bubble at: last newline
/// within `budget` (hermes `_safe_limit` rfind parity) so sealed heads never
/// cut mid-word/mid-table-row; hard cut at a char boundary as fallback.
fn seal_split_point(text: &str, budget: usize) -> usize {
    let byte_budget = text
        .char_indices()
        .nth(budget)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    text[..byte_budget]
        .rfind('\n')
        .filter(|&i| i > budget / 2)
        .unwrap_or(byte_budget)
}

/// Pending permission requests, keyed by channel_id: outer std Mutex guards
/// the inner tokio `Mutex<HashMap>`, shared via Arc so multiple tasks can
/// insert/remove while the gateway runner holds the store.
///
/// `pub(crate)` so gateway_commands can type the resolve helper that
/// borrows the Arc (never `.take()`s it).
pub(crate) type PendingPermissions = std::sync::Mutex<
    Option<Arc<Mutex<HashMap<String, operant_core::agent::ToolPermissionRequest>>>>,
>;

/// Pending user-question reply senders, keyed by channel_id (same shape as
/// PendingPermissions).
/// Pending clarify() question entry: the reply oneshot plus the original
/// choice list (so a button tap can map its index back to the option text).
type PendingUserQuestion = (tokio::sync::oneshot::Sender<String>, Option<Vec<String>>);

type PendingUserQuestions =
    std::sync::Mutex<Option<Arc<Mutex<HashMap<String, PendingUserQuestion>>>>>;

/// Global store of pending permission requests, keyed by channel_id.
pub static PENDING_PERMISSIONS: OnceLock<PendingPermissions> = OnceLock::new();

/// F2: the seat approver the gateway's `/approve` / `/deny` commands and the
/// permission receiver's arms resolve queued seat escalations through.
/// Resolution-only (G+1): an approval or denial unblocks the in-flight
/// ask — there is no grant to mint and no durable row to record; Holon's
/// board owns the durable approvals. Same
/// OnceLock-over-Mutex<Option<..>> shape as [`PENDING_PERMISSIONS`]
/// because it solves the same problem: gateway_commands is sync and has no
/// handle back into `start_gateway`'s locals. `None` until the gateway
/// starts — commands then degrade the same way the pending-permissions
/// store does.
pub static SEAT_APPROVERS: OnceLock<
    std::sync::Mutex<Option<Arc<operant_core::org::seat_authority::SeatApprover>>>,
> = OnceLock::new();

/// Install the gateway's seat approver (F2). Mirrors
/// [`store_pending_permissions`]: overwrite-once at gateway start.
pub fn store_seat_approver(approver: Arc<operant_core::org::seat_authority::SeatApprover>) {
    let cell = SEAT_APPROVERS.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(approver);
    }
}

/// The installed seat approver, if the gateway started one. Clones the `Arc`;
/// never blocks longer than a mutex grab.
pub(crate) fn seat_approver() -> Option<Arc<operant_core::org::seat_authority::SeatApprover>> {
    let cell = SEAT_APPROVERS.get_or_init(|| std::sync::Mutex::new(None));
    let guard = cell.lock().ok()?;
    guard.clone()
}

/// P3: the concrete policy store the `/grant` family reads and seats rows
/// through. Same shape as [`SEAT_APPROVERS`] for the same reason —
/// `gateway_commands` is sync and has no handle back into `start_gateway`'s
/// locals, and the agents carry the policy source as `Arc<dyn
/// SeatPolicySource>`, which cannot seat rows. `None` until the gateway
/// starts — commands then say so instead of guessing.
pub static SEAT_POLICY_DBS: OnceLock<
    std::sync::Mutex<Option<Arc<operant_core::org::seat_policy_db::SeatPolicyDb>>>,
> = OnceLock::new();

/// Wave 4 (ORGANISM-ARCHITECTURE §5): the per-seat budget override store.
/// Same shape as the other org-store globals — the turn prologue resolves
/// a seat's effective budget through it (per-field precedence over
/// `[bylaws].budget`) without a handle back into `start_gateway`'s locals.
pub static SEAT_BUDGET_DBS: OnceLock<
    std::sync::Mutex<Option<Arc<operant_core::org::seat_budgets::SeatBudgetDb>>>,
> = OnceLock::new();

/// Install the seat budget store (Wave 4). Overwrite-once at gateway start.
pub fn store_seat_budget_db(db: Arc<operant_core::org::seat_budgets::SeatBudgetDb>) {
    let cell = SEAT_BUDGET_DBS.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(db);
    }
}

/// The installed seat budget store, if the gateway started one.
pub(crate) fn seat_budget_db() -> Option<Arc<operant_core::org::seat_budgets::SeatBudgetDb>> {
    let cell = SEAT_BUDGET_DBS.get_or_init(|| std::sync::Mutex::new(None));
    let guard = cell.lock().ok()?;
    guard.clone()
}

/// Install the gateway's concrete seat-policy store (P3). Overwrite-once
/// at gateway start, mirroring [`store_seat_approver`].
pub fn store_seat_policy_db(db: Arc<operant_core::org::seat_policy_db::SeatPolicyDb>) {
    let cell = SEAT_POLICY_DBS.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(db);
    }
}

/// The installed policy store, if the gateway started one.
pub(crate) fn seat_policy_db() -> Option<Arc<operant_core::org::seat_policy_db::SeatPolicyDb>> {
    let cell = SEAT_POLICY_DBS.get_or_init(|| std::sync::Mutex::new(None));
    let guard = cell.lock().ok()?;
    guard.clone()
}

/// Resolve the delivery target for a background event (advisor/recap/
/// self-improvement note), given the session's pinned channel and the
/// global last-active slot. Order: the session's own pin wins; a session
/// without a pin (empty tag, or entry expired with its turn) falls back
/// to the slot; nothing anywhere = drop (cold boot). Extracted pure so the
/// routing contract is testable without a live gateway (iter-812).
fn resolve_background_target(
    session_id: &str,
    pinned: Option<CurrentChannel>,
    slot: Option<CurrentChannel>,
) -> Option<CurrentChannel> {
    if session_id.is_empty() {
        return slot;
    }
    pinned.or(slot)
}

/// Derive the gateway agent's stable session id (and therefore the chat's
/// SEAT under the bylaws) from a routing key. Extracted from the turn
/// dispatcher so `/permissions` computes the SAME seat the running agent
/// actually consults — a third copy of this derivation is exactly the
/// class of drift the `/stop` command's comment warns about.
pub(crate) fn derived_gateway_session_id(session_key: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_key.hash(&mut hasher);
    format!("gw_{:x}", hasher.finish())
}

/// RT-175 — the session-mutating runtime commands, executed through
/// [`GatewayMessageHandler::run_session_scoped_command`] so each runs
/// under the turn lock against the INVOKING chat's loaded conversation.
/// Returns `None` for names that are not runtime commands (the dispatch
/// loop then falls through to the sync registry handler).
///
/// These five replaced metadata theater (flags no runtime ever read)
/// with the real agent APIs the TUI's twins call:
/// - `/undo [N]` — `conversation_rewind_marks` + `rewind_to_turn` (the
///   fork the TUI's /rewind drives)
/// - `/retry` — the same rewind of 1 turn, then the prompt is re-sent as
///   a fresh turn so the answer is re-run
/// - `/compress` — `compact_history_now` (the TUI's /compact)
/// - `/reload-mcp` — the TUI's reconnect task (remove+re-add configured
///   servers, then `sync_tools_to_registry`)
/// - `/reload-skills` — `SkillManager` rescan + `agent.set_skill_manager`
///
/// `handler` is the dispatch loop's live handler (the one holding the
/// turn lock and the agent), `name` the canonical command name, `args`
/// the raw argument text, `session_key` the invoking chat's routing key.
async fn handle_runtime_command(
    handler: &GatewayMessageHandler,
    name: &str,
    args: &str,
    session_key: &str,
) -> Option<String> {
    match name {
        // ── /undo [N] — fork the conversation back N user turns ──────
        "undo" => {
            let n: usize = args.trim().parse().unwrap_or(1).max(1);
            let reply = handler
                .run_session_scoped_command(session_key, move |agent, session_id| async move {
                    // Conversation-scoped marks ONLY (the process-global
                    // list names every chat this agent ever served).
                    let marks = agent.conversation_rewind_marks().await;
                    if marks.is_empty() {
                        return "↩️ Nothing to undo — this conversation has no completed turns.".to_string();
                    }
                    let n = n.min(marks.len());
                    let Some(mark) = marks.iter().rev().nth(n - 1) else {
                        return "↩️ Nothing to undo.".to_string();
                    };
                    let turn_id = mark.turn_id;
                    match agent.rewind_to_turn(turn_id).await {
                        Ok(outcome) => {
                            tracing::info!(
                                session_id = %session_id,
                                new_session = %outcome.new_session_id,
                                removed_turns = outcome.removed_turns,
                                "Gateway /undo: forked at the chosen turn"
                            );
                            format!(
                                "↩️ Undid {} turn(s) — rewound to the turn that asked: `{}`.\n\
                                 The conversation forked: the old path is intact under `{}`, and this chat now continues on `{}`.\n\
                                 Send the prompt again (or /retry to re-run it).",
                                outcome.removed_turns,
                                truncate_for_reply(&outcome.prompt_text),
                                outcome.parent_session_id,
                                outcome.new_session_id
                            )
                        }
                        Err(e) => format!("↩️ Undo failed: {e}"),
                    }
                })
                .await;
            Some(reply)
        }

        // ── /retry — undo 1 turn, then re-run its prompt as a fresh turn ──
        "retry" => {
            let reply = handler
                .run_session_scoped_command(session_key, |agent, session_id| async move {
                    let marks = agent.conversation_rewind_marks().await;
                    let Some(mark) = marks.last() else {
                        return "🔁 Nothing to retry — this conversation has no completed turns.".to_string();
                    };
                    let turn_id = mark.turn_id;
                    match agent.rewind_to_turn(turn_id).await {
                        Ok(outcome) => {
                            if outcome.prompt_text.trim().is_empty() {
                                return format!(
                                    "🔁 Undid the last turn (forked onto `{}`), but its prompt was empty — nothing to re-run.",
                                    outcome.new_session_id
                                );
                            }
                            tracing::info!(
                                session_id = %session_id,
                                new_session = %outcome.new_session_id,
                                "Gateway /retry: rewound one turn, re-running its prompt"
                            );
                            // Re-run the prompt as a REAL turn on the forked
                            // conversation. agent.run() is the same entry the
                            // dispatch loop's turn path uses; the reply is the
                            // agent's own answer, delivered as the /retry reply.
                            match agent.run(outcome.prompt_text.clone()).await {
                                Ok(response) => {
                                    format!("🔁 Retrying the last message:\n\n{}", response.content)
                                }
                                Err(e) => format!(
                                    "🔁 The turn was rewound (forked onto `{}`), but the re-run failed: {}",
                                    outcome.new_session_id,
                                    e.user_message()
                                ),
                            }
                        }
                        Err(e) => format!("🔁 Retry failed at the rewind step: {e}"),
                    }
                })
                .await;
            Some(reply)
        }

        // ── /compress — fold older completed exchanges (the TUI /compact) ──
        "compress" => {
            let reply = handler
                .run_session_scoped_command(session_key, |agent, session_id| async move {
                    let (before, after) = agent.compact_history_now().await;
                    tracing::info!(
                        session_id = %session_id,
                        before,
                        after,
                        "Gateway /compress: compacted the session's history"
                    );
                    if after < before {
                        format!(
                            "📦 Compacted context: {before} → {after} messages (recent turns kept verbatim)."
                        )
                    } else {
                        "📦 Nothing to compact yet — the history is all recent turns.".to_string()
                    }
                })
                .await;
            Some(reply)
        }

        // ── /reload-mcp — the TUI's reconnect task, reported synchronously ──
        "reload-mcp" => {
            // McpManager is Clone-over-Arc: one clone reaches the live
            // server set. The reconnect runs OUTSIDE the turn lock — it
            // touches transports and the registry, not the conversation,
            // and holding the lock through 60s server spawns would block
            // every chat's turns for a tools refresh.
            let agent = Arc::clone(&handler.agent);
            let manager = handler.mcp_manager.clone();
            let servers = handler.mcp_server_configs.clone();
            let started = std::time::Instant::now();
            let mut reconnected = Vec::new();
            let mut failures = Vec::new();
            for (name, transport, url, auth_token, command, args_v, env, enabled) in servers {
                if !enabled {
                    continue;
                }
                let _ = manager.remove_server(&name).await;
                let result = connect_configured_mcp_server(
                    &manager, &name, transport, url, auth_token, command, args_v, env,
                )
                .await;
                match result {
                    Ok(()) => reconnected.push(name),
                    Err(e) => failures.push(format!("{name}: {e}")),
                }
            }
            // Materialize the tools into the LIVE registry (the same
            // shared-map handle the agent reads per iteration).
            let tools = manager.sync_tools_to_registry(&agent.registry()).await;
            let elapsed = started.elapsed().as_secs_f64();
            let mut msg = format!(
                "🔄 MCP reload complete in {elapsed:.1}s — {} server(s) reconnected: {}.",
                reconnected.len(),
                if reconnected.is_empty() {
                    "none".to_string()
                } else {
                    reconnected.join(", ")
                }
            );
            msg.push_str(&format!(" {tools} tool(s) synced to the live registry."));
            if !failures.is_empty() {
                msg.push_str(&format!(" Failed: {}", failures.join("; ")));
            }
            Some(msg)
        }

        // ── /reload-skills — rescan + runtime swap (set_skill_manager) ──
        "reload-skills" => {
            let agent = Arc::clone(&handler.agent);
            let skills_dir = handler.skills_root_dir.clone();
            let mut mgr = operant_core::skills::SkillManager::new(skills_dir.clone());
            // Ponytail phase 3: keep the extra root across reloads — a
            // rescan that silently dropped integration skills would make
            // /skill ponytail-review fail right after a reload.
            mgr.add_extra_root(operant_core::ponytail::installed_skills_dir());
            match mgr.load_all() {
                Ok(loaded) => {
                    let count = loaded.len();
                    agent.set_skill_manager(Some(mgr));
                    tracing::info!(
                        skills = count,
                        dir = %skills_dir.display(),
                        "Gateway /reload-skills: rescanned and swapped the live skill manager"
                    );
                    Some(format!(
                        "🔄 Skills reloaded: {count} skill(s) from `{}` — the agent's next message sees the fresh list.",
                        skills_dir.display()
                    ))
                }
                Err(e) => Some(format!(
                    "🔄 Skill reload failed — the previous list stays active. Error: {e}"
                )),
            }
        }

        // ── /effort [level] — the TUI's /effort picker, over a channel ──
        // RT-179 plan item 2.3. The TUI funnels its picker through
        // `pending_effort` into `OperantAgent::set_reasoning_effort`
        // (adapter_types/tui_app.rs:1171) and that is the ONLY live
        // reader/writer of the wire field — there was no channel way in,
        // so a Telegram operator could not lower spend on a runaway turn.
        // The same setter here is the honest twin, not a metadata echo:
        // R17's `/yolo` lesson says a key nothing reads is a lie, so this
        // arm writes the agent's own state and the reply names the value
        // now in force.
        //
        // `off` maps to `None` (the field is not sent), matching the
        // TUI's empty-string branch.
        "effort" => {
            let value = args.trim().to_ascii_lowercase();
            if value == "status" {
                // Read-only report of what the agent will send next.
                // No session scope needed: the getter only touches the
                // agent's own cell, not the conversation.
                let held = handler.agent.reasoning_effort();
                return Some(match held {
                    Some(e) => format!(
                        "🧮 Reasoning effort for this session: `{e}`.\nChange with `/effort off|minimal|low|medium|high|xhigh`."
                    ),
                    None => "🧮 Reasoning effort for this session: not set (the field is not sent).\nChange with `/effort off|minimal|low|medium|high|xhigh`.".to_string(),
                });
            }
            // The wire values, exactly the TUI's own valid set — an empty
            // value (bare `/effort`) means the same as `off` here rather
            // than a toggle, because a channel has no display to show a
            // cycle result in.
            let valid = ["", "off", "minimal", "low", "medium", "high", "xhigh"];
            if !valid.contains(&value.as_str()) {
                return Some(format!(
                    "Unknown effort `{value}` — use off|minimal|low|medium|high|xhigh"
                ));
            }
            let reply = handler
                .run_session_scoped_command(session_key, move |agent, session_id| async move {
                    let wire = if value == "off" || value.is_empty() {
                        None
                    } else {
                        Some(value.clone())
                    };
                    agent.set_reasoning_effort(wire);
                    let shown = if value == "off" || value.is_empty() {
                        "off".to_string()
                    } else {
                        value.clone()
                    };
                    tracing::info!(
                        session_id = %session_id,
                        effort = %shown,
                        "Gateway /effort: set the live reasoning effort"
                    );
                    format!(
                        "🧮 Reasoning effort: `{shown}` for this session — the next request carries it."
                    )
                })
                .await;
            Some(reply)
        }

        _ => None,
    }
}

/// First ~80 chars of a prompt, for reply text that quotes what was undone.
fn truncate_for_reply(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= 80 {
        t.to_string()
    } else {
        let head: String = t.chars().take(77).collect();
        format!("{head}…")
    }
}

/// Connect one configured MCP server by transport kind (the shared shape
/// of the TUI's start_mcp_reconnect task, extracted so /reload-mcp and the
/// boot path cannot drift).
#[expect(
    clippy::too_many_arguments,
    reason = "the McpServerConfig field set, flattened: one arg per connect input. The TUI's start_mcp_reconnect carries the same 8-field tuple (aliased ServerCfg) for the same reason."
)]
async fn connect_configured_mcp_server(
    manager: &operant_core::mcp::McpManager,
    name: &str,
    transport: operant_core::config::McpTransportKind,
    url: Option<String>,
    auth_token: Option<String>,
    command: Option<String>,
    args: Vec<String>,
    env: std::collections::HashMap<String, String>,
) -> operant_core::Result<()> {
    use operant_core::config::McpTransportKind;
    match transport {
        McpTransportKind::Http => match url {
            Some(url) => manager.add_server(name, url, auth_token).await,
            None => Err(operant_core::error::Error::Agent(format!(
                "no URL configured for {name}",
            ))),
        },
        McpTransportKind::StreamableHttp => match url {
            Some(url) => manager.add_streamable_server(name, url, auth_token).await,
            None => Err(operant_core::error::Error::Agent(format!(
                "no URL configured for {name}",
            ))),
        },
        McpTransportKind::Stdio => match command {
            Some(command) => manager.add_stdio_server(name, command, args, env).await,
            None => Err(operant_core::error::Error::Agent(format!(
                "no command configured for stdio server {name}",
            ))),
        },
    }
}

/// Record a dispatcher-side DENIAL of a seat-policy escalation (the YOLO
/// arm): the approver's verdict on the queued row, best-effort — the run
/// hears `Deny` from the arm either way, so a resolution failure only
/// costs audit, never enforcement. Ungoverned requests carry no escalation
/// and are a no-op.
fn resolve_seat_escalation_on_deny(req: &operant_core::agent::ToolPermissionRequest) {
    let Some(ask) = req.seat_escalation.as_ref() else {
        return;
    };
    let Some(approver) = seat_approver() else {
        tracing::warn!(request = %ask.request_id, "no seat approver installed — the denial is not recorded");
        return;
    };
    if let Err(e) = approver.deny(ask) {
        tracing::warn!(request = %ask.request_id, error = %e, "recording the denial failed");
    }
}

/// Record the 60s interactive lapse of a seat-policy escalation as an
/// expiry (§8.3): the row survives for audit, the resolver is the operator
/// literal. Best-effort, for the same reason as
/// [`resolve_seat_escalation_on_deny`].
fn resolve_seat_escalation_on_expire(req: &operant_core::agent::ToolPermissionRequest) {
    let Some(ask) = req.seat_escalation.as_ref() else {
        return;
    };
    let Some(approver) = seat_approver() else {
        tracing::warn!(request = %ask.request_id, "no seat approver installed — the lapse is not recorded");
        return;
    };
    if let Err(e) = approver.expire(ask) {
        tracing::warn!(request = %ask.request_id, error = %e, "recording the lapse failed");
    }
}

/// F2 approval: mint the grant a queued seat escalation confers and resolve
/// its row. `Ok(grant_id)` — the caller tells the waiting run `AllowSession`
/// (or `AllowAlways`). `Err(refusal)` — `issue_grant` refused (the
/// approver's scope does not cover the grant, a seat is vacant, …): the row
/// is ALREADY recorded `Denied` by the approver, the refusal text is
/// returned so the caller surfaces it, and the waiting run must hear
/// `Deny`. Never bypass: the refusal IS the resolution.
/// `None` — not a seat escalation (ungoverned prompt): no minting, today's
/// behaviour exactly.
pub(crate) fn approve_seat_escalation(
    req: &operant_core::agent::ToolPermissionRequest,
) -> Option<Result<String, String>> {
    let ask = req.seat_escalation.as_ref()?;
    let approver = seat_approver()?;
    Some(approver.approve(ask))
}

/// Global store of pending user-question replies, keyed by channel_id.
/// When the clarify tool asks a question, we store the reply_tx here.
/// The next incoming message from that channel is routed as the reply
/// instead of being sent to the agent. (iter-161)
pub static PENDING_USER_QUESTIONS: OnceLock<PendingUserQuestions> = OnceLock::new();

/// Live YOLO-mode channels: keys are `"{platform}:{channel_id}"` for channels
/// that opted into skipping approval prompts (via `/yolo` in the gateway). The
/// permission receiver consults this set before prompting, so YOLO mode
/// actually takes effect. (R17: `/yolo` previously only wrote a metadata key
/// that nothing read — mirroring the TUI's
/// PermissionMode::BypassPermissions.)
pub static YOLO_CHANNELS: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();

/// Live plan-mode channels: keys are `"{platform}:{channel_id}"` for
/// channels that asked for read-only agent behaviour (via `/plan` in the
/// gateway). RT-179 plan item 2.3 — the TUI's twin is
/// `PermissionMode::Plan`, which the permission drain enforces by sending
/// `Deny` for every gated tool (tui/app/channel_drains.rs:389). The
/// gateway had NO equivalent, so a channel's only posture choices were
/// prompt-every-tool or YOLO-everything.
///
/// Same in-memory-shape lesson as `YOLO_CHANNELS` (R17): the set is what
/// the permission receiver READS, so the toggle is real rather than a
/// metadata echo. Plan mode is deliberately not persisted — the TUI's
/// own mode lives in settings.json and is re-picked per process, and a
/// read-only posture that silently survived a restart would surprise
/// more than a lost toggle does.
pub static PLAN_CHANNELS: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();

/// Redact secrets (API keys, bot tokens, passwords) from an error/display
/// string before it reaches logs. Hermes `_redact_telegram_error_text` /
/// `_redact_discord_error_text` parity: gateway error paths echo upstream
/// API bodies and reqwest URLs (which embed bot tokens in the path), so
/// every error-formatted log line must be scrubbed. (T9 channel audit)
pub(crate) fn redact_err(e: &(impl std::fmt::Display + ?Sized)) -> String {
    redact_sensitive_text_if_enabled(&e.to_string())
}

/// Short, content-free class name for a failed turn.
///
/// `operant_core::Error` has no accessor for its discriminant, and its `Display`
/// is the whole upstream body — too coarse to tell "the provider died" from "a
/// tool timed out" when reading a log back. The catch-all is deliberate: the
/// enum lives in `operant-core` and is free to grow variants, so naming a
/// subset of the classes that actually end turns keeps this file compiling
/// across those additions instead of breaking on the next one.
fn error_class(e: &operant_core::Error) -> &'static str {
    use operant_core::Error as E;
    match e {
        E::Network(_) => "network",
        E::StreamDied { .. } => "stream_died",
        E::Provider { .. } => "provider",
        E::RateLimited { .. } => "rate_limited",
        E::Authentication(_) => "auth",
        E::ContextLengthExceeded => "context_length",
        E::MaxIterationsExceeded { .. } => "max_iterations",
        E::IncompleteSseMessage => "incomplete_sse",
        E::MissingApiKey => "missing_api_key",
        E::Config(_) => "config",
        E::Agent(_) => "agent",
        E::ToolTimeout { .. } => "tool_timeout",
        E::ToolCancelled { .. } => "tool_cancelled",
        E::ToolExecution { .. } => "tool_execution",
        E::JsonDecode(_) | E::ParseResponse(_) => "decode",
        E::Io(_) => "io",
        _ => "other",
    }
}

fn yolo_key(platform: &str, channel_id: &str) -> String {
    format!("{platform}:{channel_id}")
}

/// Returns true when the given channel has YOLO mode enabled.
pub fn yolo_enabled(platform: &str, channel_id: &str) -> bool {
    YOLO_CHANNELS
        .get()
        .and_then(|s| s.lock().ok())
        .map(|guard| guard.contains(&yolo_key(platform, channel_id)))
        .unwrap_or(false)
}

/// Set (true) or clear (false) YOLO mode for a channel.
pub fn set_yolo(platform: &str, channel_id: &str, enabled: bool) {
    let cell = YOLO_CHANNELS.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    if let Ok(mut guard) = cell.lock() {
        let key = yolo_key(platform, channel_id);
        if enabled {
            guard.insert(key);
        } else {
            guard.remove(&key);
        }
    }
}

/// Returns true when the given channel asked for plan (read-only) mode.
/// Read by the permission receiver BEFORE it prompts: a plan-mode channel
/// never prompts at all — the request is denied outright, which is the
/// TUI's `PermissionMode::Plan` behaviour, not a stricter prompt.
///
/// Ordering against YOLO is deliberate: a channel in BOTH sets keeps plan
/// mode winning. Read-only is the stance that cannot be widened by an
/// accident of two toggles, while YOLO is the stance an operator can
/// always re-assert manually.
pub fn plan_mode_enabled(platform: &str, channel_id: &str) -> bool {
    PLAN_CHANNELS
        .get()
        .and_then(|s| s.lock().ok())
        .map(|guard| guard.contains(&yolo_key(platform, channel_id)))
        .unwrap_or(false)
}

/// Set (true) or clear (false) plan mode for a channel.
pub fn set_plan_mode(platform: &str, channel_id: &str, enabled: bool) {
    let cell = PLAN_CHANNELS.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    if let Ok(mut guard) = cell.lock() {
        let key = yolo_key(platform, channel_id);
        if enabled {
            guard.insert(key);
        } else {
            guard.remove(&key);
        }
    }
}

/// Initialize the globals (call once at startup).
fn init_pending_permissions() {
    PENDING_PERMISSIONS.get_or_init(|| std::sync::Mutex::new(None));
    PENDING_USER_QUESTIONS.get_or_init(|| std::sync::Mutex::new(None));
}

/// Store the shared pending-permission map into the global.
///
/// NB: must OVERWRITE the inner `Option` rather than
/// `get_or_init(|| Mutex::new(Some(..)))`: `init_pending_permissions()` has
/// already initialized the static with `None` by the time the permission
/// receiver builds its Arc, and `OnceLock::get_or_init` never overwrites —
/// so the old pattern silently left the store `None` for the whole process.
/// Every static-based lookup then failed ("Approval expired — no command
/// was waiting" / "No pending permission request found") while the 60s
/// timeout task — holding its own Arc clone — still found and auto-denied
/// the entry. That is exactly what the live logs showed: a button tap 5s
/// after the prompt found nothing, the timeout 60s later removed it.
pub fn store_pending_permissions(
    store: Arc<Mutex<HashMap<String, operant_core::agent::ToolPermissionRequest>>>,
) {
    let cell = PENDING_PERMISSIONS.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(store);
    }
}

/// True when `channel_id` has an unresolved permission request in the
/// shared store. Mirrors `resolve_pending_permission`'s bounded retry so a
/// transient collision with an in-flight insert can't make a live button
/// tap look expired.
pub fn pending_permission_exists(channel_id: &str) -> bool {
    let Some(cell) = PENDING_PERMISSIONS.get() else {
        return false;
    };
    let Ok(guard) = cell.lock() else {
        return false;
    };
    let Some(pending) = guard.as_ref() else {
        return false;
    };
    for _ in 0..50 {
        if let Ok(map) = pending.try_lock() {
            return map.contains_key(channel_id);
        }
        std::thread::yield_now();
    }
    false
}

/// Returns the path to the gateway PID file used for cross-process status checks.
fn pid_file_path() -> std::path::PathBuf {
    operant_core::platform::operant_home().join("gateway.pid")
}

/// The exclusive daemon lock beside the pid file. A SECOND `gateway run`
/// used to start unconditionally: `start_gateway` checked only the
/// in-process `RUNNER` static, never the pid file, so two daemons
/// double-polled Telegram/Discord (fighting over getUpdates offsets),
/// double-held model credentials, and double the RSS. `flock` is the
/// fix's shape (the jcode daemon lock, server/socket.rs): the kernel
/// releases it when the holder dies, so a crashed gateway never wedges
/// the next start, and a pid-file race can never let two through.
struct GatewayDaemonLock {
    _file: std::fs::File,
    path: std::path::PathBuf,
}

impl Drop for GatewayDaemonLock {
    fn drop(&mut self) {
        // Best effort: the flock is released by the fd close regardless, and
        // a leftover lock file is harmless (the next gateway re-locks it).
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The guards that have been taken and MUST STAY for the process's life.
///
/// iter-1345 (live verification): binding the lock to a local `let` in
/// `start_gateway` dropped it at that function's return, closed the flock
/// fd, and the kernel released the lock while the daemon went on serving
/// every platform. A second `gateway run` then acquired it happily, which is
/// the exact duplicate-daemon bug the lock exists to prevent (verified live:
/// the running daemon held zero fds on gateway.lock). A Vec: in-process
/// restarts push new guards and the older ones stay held until process exit,
/// which is itself never a second daemon. A process exit or crash closes the
/// fds and the kernel releases the flock, so the leak has no staleness cost;
/// the lock file itself is removed by `Drop` and a leftover file is harmless.
static GATEWAY_DAEMON_LOCKS: std::sync::Mutex<Vec<GatewayDaemonLock>> =
    std::sync::Mutex::new(Vec::new());

/// Take the exclusive gateway daemon lock, or report that a live gateway
/// holds it. `Ok(None)` means REFUSE TO START — the caller must not boot a
/// second daemon.
fn acquire_gateway_daemon_lock() -> Result<Option<GatewayDaemonLock>> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        let path = operant_core::platform::operant_home().join("gateway.lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open gateway lock {}", path.display()))?;
        let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
        Ok(taken.then_some(GatewayDaemonLock { _file: file, path }))
    }
    #[cfg(not(unix))]
    {
        // Windows has no flock; the pid file remains the (best-effort)
        // duplicate signal there. The gateway daemon is a systemd/foreground
        // Unix deployment surface in practice.
        Ok(Some(GatewayDaemonLock {
            _file: std::fs::File::create(std::path::PathBuf::from("NUL"))
                .context("windows no-op lock placeholder")?,
            path: std::path::PathBuf::new(),
        }))
    }
}
/// How often (in queued turns) the gateway logs the aggregate queue-wait
/// summary. 50 keeps the summary visible without flooding the log on a
/// busy multi-chat deployment — the per-turn line still records every
/// occurrence.
const QUEUE_WAIT_SUMMARY_EVERY: u64 = 50;

/// The shared-agent queue-wait measurement (iter-1338).
///
/// The one-agent design serializes every chat's turn behind `turn_lock`.
/// This is the durable record of whether that ever costs a user anything:
/// how often a turn queued behind another, the total and worst wait, and
/// the sessions that waited most. It is the ponytail gate the lock's own
/// comment names — the agent-per-seat upgrade path is taken when these
/// numbers say a busy chat starves others, not before.
#[derive(Debug, Default)]
struct QueueWaitStats {
    /// Turns that had to wait for another chat's turn.
    events: u64,
    /// Total wall time spent waiting across those turns (ms).
    total_ms: u64,
    /// Worst single wait (ms) — the tail a user actually feels.
    worst_ms: u64,
    /// Per-session event count, so a noisy chat is visible.
    per_session: HashMap<String, u64>,
}

impl QueueWaitStats {
    fn record(&mut self, waited: std::time::Duration, session_key: &str) {
        let ms = waited.as_millis() as u64;
        self.events += 1;
        self.total_ms += ms;
        self.worst_ms = self.worst_ms.max(ms);
        *self.per_session.entry(session_key.to_string()).or_insert(0) += 1;
    }

    /// One human-readable line for the logs and `gateway status --deep`.
    fn line(&self) -> String {
        if self.events == 0 {
            return "queued turns: 0 (no chat has waited yet)".to_string();
        }
        let top = self
            .per_session
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(k, n)| format!("{k} ×{n}"))
            .unwrap_or_else(|| "none".to_string());
        format!(
            "queued turns: {} (total {} ms, worst {} ms, busiest chat: {top})",
            self.events, self.total_ms, self.worst_ms
        )
    }
}

/// Sticky per-session resume pins, set by the `/resume` command.
///
/// The agent's session id is derived from the routing key
/// (`platform:channel:user[:thread]`), so a chat can only ever continue the one
/// transcript its key hashes to. `/resume <session-id>` needs to make the next
/// turn in a chat continue a DIFFERENT transcript, which means overriding that
/// derivation — and the reload machinery that actually repopulates the agent's
/// context from the database already exists in
/// `GatewayMessageHandler::handle`. This map is the missing half: it carries
/// the override from the command handler to the turn handler.
///
/// Sticky rather than one-shot on purpose. A one-shot pin would leave the
/// second message after a `/resume` silently back on the chat's own transcript,
/// which is the same class of lie this change exists to remove — the user was
/// told the session resumed, and then it did not.
///
/// A `std` mutex, not a tokio one, because `handle_command` is synchronous: the
/// critical section is a single map operation and never spans an await, so
/// there is nothing to be gained by making it async.
#[derive(Default)]
pub struct SessionPins {
    pins: std::sync::Mutex<HashMap<String, String>>,
}

impl SessionPins {
    /// Route `session_key` at `session_id` from now on, replacing any previous
    /// pin. `session_id` is the agent's session id (`gw_<hash>`), the same
    /// namespace the `messages` table is keyed by.
    pub fn set(&self, session_key: &str, session_id: &str) {
        let Ok(mut pins) = self.pins.lock() else {
            return;
        };
        pins.insert(session_key.to_string(), session_id.to_string());
    }

    /// The pinned session id for `session_key`, if any.
    pub fn get(&self, session_key: &str) -> Option<String> {
        self.pins.lock().ok()?.get(session_key).cloned()
    }
}

/// Compact human-readable duration for the stopped-early operator notice
/// (BUGS.md S5): "4m 37s" above a minute, "42s" below it.
fn format_turn_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// BUGS.md S5 — the operator-visible content for a completed turn.
///
/// Extracted from `GatewayMessageHandler::handle`'s turn-end block so the
/// operator-facing wording is unit-testable without a handler harness
/// (none exists in this crate). Semantics are verbatim from the former
/// inline cascade:
/// - empty content + early stop → stopped-early status line (A1's grace
///   gate returns the empty message for degenerate grace output, so this
///   is the operator-visible form of the reasoning-leak artifact class);
/// - empty + reasoning fallback and the provider-overload wording keep
///   their pre-S5 meaning so hermes-parity expectations keep passing;
/// - non-empty + early stop → the partial answer, labelled with elapsed
///   time and the real exit reason;
/// - a normal `TextResponse` turn (`early_stop: None`) passes through
///   unmodified.
fn turn_end_content(
    content: &str,
    reasoning: Option<&str>,
    early_stop: Option<operant_core::agent::TurnExitReason>,
    elapsed: std::time::Duration,
) -> String {
    if content.trim().is_empty() {
        if let Some(reason) = early_stop {
            // Early-stop exit with nothing (or only degenerate text) to
            // show: one-line status + resume hint.
            format!(
                "⚠️ I stopped early after {} — {}: no summary could be produced. \
                 Reply 'continue' to resume the task.",
                format_turn_elapsed(elapsed),
                reason,
            )
        } else if let Some(r) = reasoning {
            if !r.trim().is_empty() {
                tracing::info!(
                    "Content empty but reasoning available, using as fallback (len={})",
                    r.len()
                );
                r.to_string()
            } else {
                tracing::warn!("Agent returned empty response, no reasoning available");
                "⚠️ My model provider returned an empty response after retries \
                 (provider overload). Reply 'continue' to resume the task."
                    .to_string()
            }
        } else {
            tracing::warn!("Agent returned empty response, no reasoning available");
            "⚠️ My model provider returned an empty response after retries \
             (provider overload). Reply 'continue' to resume the task."
                .to_string()
        }
    } else if let Some(reason) = early_stop {
        // Early-stop exit with a non-degenerate partial answer: keep the
        // partial but label it as one, with the elapsed time and the real
        // reason.
        format!(
            "⚠️ I stopped early after {} — {}: {}",
            format_turn_elapsed(elapsed),
            reason,
            content
        )
    } else {
        content.to_string()
    }
}

/// The endpoint and model as `operant.toml` holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigEndpoint {
    base_url: String,
    api_key: Option<String>,
    model: String,
}

/// The file's endpoint as of the last check. The gateway is a separate process
/// from the TUI, so `/connect` and `/model` reach it only through the file.
static CONFIG_ENDPOINT: std::sync::Mutex<Option<ConfigEndpoint>> = std::sync::Mutex::new(None);

/// The endpoint in the config file this process loaded. `None` when there is no
/// file or it does not parse; the caller then keeps what it has.
fn config_endpoint_on_disk() -> Option<ConfigEndpoint> {
    let path = operant_core::config::loaded_config_path()?;
    let config = operant_core::config::parse_config_file(&path).ok()?;
    Some(ConfigEndpoint {
        base_url: config.client.base_url,
        api_key: config.client.api_key,
        model: config.agent.model,
    })
}

/// Record `now` as the last-seen endpoint. Returns it only when a previous
/// observation exists and differs, so the first observation is a baseline.
fn endpoint_changed_since(
    last: &mut Option<ConfigEndpoint>,
    now: ConfigEndpoint,
) -> Option<ConfigEndpoint> {
    let changed = last.as_ref().is_some_and(|prev| *prev != now);
    *last = Some(now.clone());
    changed.then_some(now)
}

/// (P0.4) The endpoint the file gained since the last check, if any. The baseline
/// is taken when the handler is built, so an env override is not replaced until
/// the file itself changes.
fn endpoint_changed_on_disk() -> Option<ConfigEndpoint> {
    let now = config_endpoint_on_disk()?;
    let mut last = CONFIG_ENDPOINT.lock().ok()?;
    endpoint_changed_since(&mut last, now)
}

/// Message handler that processes incoming gateway messages through the Operant agent.
///
/// Long-term memory is handled by the agent itself: `OperantAgent` now
/// carries a `MemoryProvider` (attached in `create_runtime_agent`), so
/// prefetch injection and sync_turn happen inside `run()` — this handler
/// no longer needs its own provider copy (audit gap F1, docs/AUDIT_2026-08-02.md).
/// RT-175 — the send-safe MCP server config tuple (name, transport,
/// url, auth_token, command, args, env, enabled): the same 8-field shape
/// the TUI's start_mcp_reconnect aliases as ServerCfg. Aliased so the
/// handler field and the reconnect loop share one name, not a repeated
/// 8-line type.
type ServerConfigTuple = (
    String,
    operant_core::config::McpTransportKind,
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<String>,
    std::collections::HashMap<String, String>,
    bool,
);

struct GatewayMessageHandler {
    agent: Arc<OperantAgent>,
    /// `gateway.admins`. A turn from a sender outside this list is not retained
    /// as memory (KB-6).
    admins: Vec<String>,
    /// Tracks the currently-active session ID so we only reload
    /// conversation history when switching sessions (not on every
    /// message within the same session). This preserves prompt prefix
    /// caching — previously clear_history() was called every message.
    current_session_id: tokio::sync::Mutex<Option<String>>,
    /// Reference to the gateway so the handler can read session metadata
    /// (model_override, yolo_mode, etc.) before each agent.run() call.
    /// (Bug #3 from iter-98 audit — 15 slash commands wrote to metadata
    /// but the runner never read it back.) Set after gateway construction
    /// via set_gateway() because the handler is created before the gateway.
    gateway: tokio::sync::Mutex<Option<Arc<Gateway>>>,
    /// Per-session resume overrides set by `/resume`. Shared with the dispatch
    /// loop's `CommandContext` so the command can pin a session this handler
    /// then honours.
    session_pins: Arc<SessionPins>,
    /// Per-thread exit reason of the most recent turn, written by the event
    /// receiver on `AgentEvent::Done` (S5). Read by the turn-end block so a
    /// non-`TextResponse` exit becomes an operator-visible stopped-early
    /// notice.
    exit_reasons: ExitReasonMap,
    /// iter-632: per-thread turn usage accumulator written by the event
    /// receiver (`AgentEvent::Usage`/`Cost`) and drained at turn end.
    turn_usage: TurnUsageMap,
    /// Wave 3 (ORGANISM-ARCHITECTURE §3): the continuation-summary TTL
    /// (`[bylaws].session_summary_ttl_minutes`, captured at gateway start
    /// — the handler has no config handle of its own and the value is not
    /// hot-reloadable).
    summary_ttl_minutes: i64,
    /// Wave 4 (ORGANISM-ARCHITECTURE §5): the org-wide default budget
    /// (`[bylaws].budget`, captured at gateway start for the same reason).
    /// Per-seat overrides resolve through the SEAT_BUDGET_DBS global at
    /// turn time.
    default_budget: operant_core::config::BudgetSettings,
    /// iter-683 (gap 5 phase 1b): the DM/feed context injector — the
    /// gateway turn's mirror of the scheduler's cron seam. `None` = no
    /// injection, turns byte-identical (dark-mergeable).
    context_injection: Option<Arc<operant_core::org::context_injection::ContextInjector>>,
    /// One gateway turn at a time on the shared agent (2026-10-16 audit,
    /// plan 06A). Every chat shares ONE `Arc<OperantAgent>` whose hot
    /// conversation slot is swapped by `set_session_id`; two concurrent
    /// turns retarget each other mid-run and one chat's tool results land
    /// in the other's persisted transcript. Held from the retarget to the
    /// end of `handle`.
    // ponytail: one turn at a time. Upgrade path: an agent per seat (the
    // create_runtime_agent seam) if a busy chat starves others — measure
    // queue wait first.
    turn_lock: tokio::sync::Mutex<()>,
    /// The session whose turn currently holds `turn_lock` — the `/stop`
    /// attach gate. The interrupt flag is ONE object shared by every
    /// chat, so handing it to a non-running session's command context
    /// would let that session stop someone else's turn.
    running_session: std::sync::Mutex<Option<String>>,
    /// Sessions whose still-queued turn the user `/stop`ped (W3-2). Written
    /// and consumed under the `running_session` mutex, so a stop decided
    /// "not running yet" can never race the turn's claim of the lock.
    cancelled_queued: std::sync::Mutex<std::collections::HashSet<String>>,
    /// RT-175 — the live MCP manager handle for `/reload-mcp`. Clone-over-
    /// Arc, so this reaches the same server set the agent's tools read.
    mcp_manager: operant_core::mcp::McpManager,
    /// RT-175 — send-safe copies of the configured MCP servers for
    /// `/reload-mcp`'s reconnect loop (the TUI's start_mcp_reconnect
    /// built the same tuple list for the same reason: the AppConfig is
    /// not `Send`-safe to move into a task).
    mcp_server_configs: Vec<ServerConfigTuple>,
    /// iter-1338: how often a turn queued behind another chat's turn, and
    /// for how long. The ponytail gate for the agent-per-seat upgrade path
    /// (see the turn_lock site). Read by the stop-time log and
    /// `gateway status --deep`.
    queue_waits: tokio::sync::Mutex<QueueWaitStats>,
    /// RT-175 — the configured skills root for `/reload-skills`' rescan.
    skills_root_dir: std::path::PathBuf,
}

impl GatewayMessageHandler {
    /// RT-175 — execute a session-mutating runtime command against THIS
    /// chat's own conversation, safely on the shared agent.
    ///
    /// `/undo`, `/retry`, `/compress` mutate the agent's loaded
    /// conversation. The dispatch loop intercepts commands BEFORE any turn
    /// runs, and the shared agent holds whichever conversation turned
    /// LAST — so executing against whatever happens to be loaded would
    /// rewind/compact a STRANGER's history (the exact cross-session
    /// corruption `turn_lock` exists to prevent). This helper is the safe
    /// shape, mirroring `handle`'s own turn entry:
    ///
    /// 1. acquire `turn_lock` (a running turn finishes first — one
    ///    mutation surface at a time);
    /// 2. resolve THIS chat's session id the same way `handle` does
    ///    (pin → persisted row → derived), then `set_session_id` it, so
    ///    the loaded conversation is the invoker's;
    /// 3. run the operation; `set_session_id`'s swap persists the
    ///    outgoing conversation, so the previous chat loses nothing;
    /// 4. reply text back to the dispatch loop's send path.
    ///
    /// `op` is the operation: `&self.agent` + the loaded session id in,
    /// reply string out. Splitting it keeps the lock/resolve scaffolding
    /// in ONE place — a second mutating command cannot forget the guard.
    async fn run_session_scoped_command<F, Fut>(&self, session_key: &str, op: F) -> String
    where
        F: FnOnce(Arc<OperantAgent>, String) -> Fut,
        Fut: std::future::Future<Output = String>,
    {
        // 1. One mutation surface at a time — same lock a turn takes.
        let _turn = self.turn_lock.lock().await;

        // 2. THIS chat's session id, resolved exactly as `handle` does.
        // The gateway lock is a tokio Mutex held by no other command-path
        // code at interception time (handle() takes it later in ITS own
        // flow, under this same turn_lock), so a plain await here cannot
        // deadlock.
        let derived = derived_gateway_session_id(session_key);
        let persisted = {
            let gw_guard = self.gateway.lock().await;
            gw_guard
                .as_ref()
                .and_then(|gw| gw.get_persistent_sessions())
                .and_then(|store| store.conversation_id(session_key).ok().flatten())
        };
        let session_id = self
            .session_pins
            .get(session_key)
            .or(persisted)
            .unwrap_or(derived);
        // Load THIS chat's transcript. The swap persists the outgoing
        // conversation back to the store, so no other chat loses context.
        self.agent.set_session_id(session_id.clone());

        // 3. The operation, under the lock, against the invoker's session.
        op(Arc::clone(&self.agent), session_id).await
    }

    /// Plan 06A: `/stop` may only stop the turn that currently holds
    /// `turn_lock`. The interrupt flag is one object shared by every
    /// chat, so attaching it for any other session would let that
    /// session's `/stop` kill someone else's running turn.
    fn stop_flag_if_running(
        &self,
        turn_inflight: &HashMap<String, InterruptFlag>,
        session_key: &str,
    ) -> Option<InterruptFlag> {
        let running = self
            .running_session
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if running.as_deref() == Some(session_key) {
            turn_inflight.get(session_key).cloned()
        } else {
            None
        }
    }

    /// W3-2: `/stop` for a session whose turn is registered in
    /// `turn_inflight` but has not claimed `turn_lock` yet. Marks it so the
    /// turn exits the moment it would start. `false` = nothing queued (the
    /// session is idle or its turn is the running one).
    fn cancel_if_queued(
        &self,
        turn_inflight: &HashMap<String, InterruptFlag>,
        session_key: &str,
    ) -> bool {
        let running = self
            .running_session
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if running.as_deref() != Some(session_key) && turn_inflight.contains_key(session_key) {
            self.cancelled_queued
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(session_key.to_string());
            true
        } else {
            false
        }
    }
}

/// Clears `running_session` when the turn ends, on every return path — a
/// stale "running" marker would send every later `/stop` into the void.
struct RunningSessionGuard<'a>(&'a std::sync::Mutex<Option<String>>);
impl Drop for RunningSessionGuard<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// The memory scope an unbound gateway turn starts in (KB-8). The owner's
/// personal bank is reachable only from a sender named in `gateway.admins`;
/// every other sender gets a bank of their own.
fn unbound_scope_for(
    admins: &[String],
    platform: &str,
    user_id: &str,
) -> operant_core::memory_wire::MemoryScope {
    use operant_core::memory_wire::MemoryScope;
    if admins.iter().any(|admin| admin == user_id) {
        MemoryScope::Personal
    } else {
        MemoryScope::Sender {
            key: format!("{platform}:{user_id}"),
        }
    }
}

/// Whether a turn from `user_id` is retained as memory (KB-6). Only a named
/// admin is retained: an empty admin list admits anyone, so its senders are not
/// trusted to write memory.
fn retains_turn_for(admins: &[String], user_id: &str) -> bool {
    admins.iter().any(|admin| admin == user_id)
}

#[async_trait::async_trait]
impl MessageHandler for GatewayMessageHandler {
    /// Feed capture (plan-2026-10-09 §3): route a channel/group post to its
    /// seat's feed. Seat = `feed_seat_map[channel]`, default `premiere` —
    /// same default as the DM tap's employee binding. Fail-open on every
    /// error: a failed feed row must never break routing.
    fn record_feed(&self, message: &IncomingMessage) {
        let Some(injector) = self.context_injection.as_ref() else {
            return;
        };
        let seat = injector
            .settings()
            .feed_seat_map
            .get(&message.channel_id)
            .cloned()
            .unwrap_or_else(|| "premiere".to_string());
        injector.record_feed_item(&seat, &message.username, &message.content);
    }

    async fn handle(&self, mut message: IncomingMessage) -> operant_core::Result<OutgoingMessage> {
        // (iter-1132) A webhook subscription with `deliver_only` wants the
        // EVENT delivered, not an answer about it: no agent run, the event
        // body IS the message. The delivery target (if any) is applied by
        // the gateway when the response comes back.
        if message.deliver_only {
            return Ok(OutgoingMessage::new(
                message.channel_id.clone(),
                message.content.clone(),
            ));
        }
        // (iter-161: Check for pending user-question reply before routing
        // to the agent. If there's a pending clarify() question for this
        // channel, route the user's message as the reply instead.)
        let channel_key = format!("{}:{}", message.platform, message.channel_id);
        let pending_map_clone = {
            let global_uq = crate::gateway_runner::PENDING_USER_QUESTIONS.get();
            global_uq
                .and_then(|g| g.lock().ok())
                .and_then(|mut guard| guard.take())
        };
        if let Some(pending_map) = pending_map_clone {
            let reply_tx = {
                let mut pending = pending_map.lock().await;
                pending.remove(&channel_key)
            };
            if let Some((reply_tx, _choices)) = reply_tx {
                tracing::info!(
                    channel = %channel_key,
                    "Routing message as user-question reply (intercepted)"
                );
                let _ = reply_tx.send(message.content.clone());
                // Restore the map for future questions
                if let Some(g) = crate::gateway_runner::PENDING_USER_QUESTIONS.get()
                    && let Ok(mut guard) = g.lock()
                {
                    *guard = Some(pending_map);
                }
                return Ok(OutgoingMessage::new(
                    message.channel_id.clone(),
                    "✅ Reply received — resuming...",
                )
                .no_markdown());
            }
            // No pending question — restore the map and continue to agent
            if let Some(g) = crate::gateway_runner::PENDING_USER_QUESTIONS.get()
                && let Ok(mut guard) = g.lock()
            {
                *guard = Some(pending_map);
            }
        }

        // Build session key: per-user for DMs, per-channel for groups, and
        // thread-aware so each Telegram forum topic / Discord thread gets its
        // own conversation — a new topic starts a FRESH session instead of
        // continuing the parent channel's history. (hermes build_session_key
        // parity: threads are separate sessions within a parent chat.)
        let session_key = match (message.is_group_chat, message.thread_id) {
            (true, Some(tid)) => {
                format!("{}:{}:thread:{}", message.platform, message.channel_id, tid)
            }
            (true, None) => format!("{}:{}", message.platform, message.channel_id),
            (false, Some(tid)) => format!(
                "{}:{}:{}:thread:{}",
                message.platform, message.channel_id, message.user_id, tid
            ),
            (false, None) => format!(
                "{}:{}:{}",
                message.platform, message.channel_id, message.user_id
            ),
        };

        // Derive stable session_id from key hash — the shared helper, so
        // `/permissions` computes the same seat the agent actually runs as.
        let derived_session_id = derived_gateway_session_id(&session_key);

        // A `/resume` pin overrides the derivation. Everything downstream —
        // `save_message`, the reload below, the trajectory namespace — is
        // already keyed on `session_id`, so re-pointing that one value is the
        // whole of resume. The reload then fires on its own, because
        // `current_session_id` no longer matches.
        // GW-6/GW-7: a conversation pinned in memory (`/resume`) or persisted on
        // the session row (`/new`, a seat change, `/resume` across a restart)
        // overrides the derivation. The row is read once per turn.
        let persisted = {
            let gw_guard = self.gateway.lock().await;
            gw_guard
                .as_ref()
                .and_then(|gw| gw.get_persistent_sessions())
                .and_then(|store| store.conversation_id(&session_key).ok().flatten())
        };
        let (session_id, resumed) = match self.session_pins.get(&session_key).or(persisted) {
            Some(conversation) => (conversation, true),
            None => (derived_session_id.clone(), false),
        };
        if resumed {
            tracing::info!(
                session_key = %session_key,
                session_id = %session_id,
                derived_session_id = %derived_session_id,
                "Gateway turn is running on a pinned conversation"
            );
        }

        // ── One turn at a time on the shared agent (2026-10-16 audit,
        // plan 06A) ────────────────────────────────────────────────
        // `set_session_id` below swaps the agent's single hot conversation:
        // the outgoing session's turns go back to the store and the
        // incoming session's transcript is rehydrated in their place. Two
        // concurrent turns on this one agent therefore retarget each other
        // mid-run — one chat's tool results and answer persist under the
        // OTHER chat's session id. The lock is held until `handle`
        // returns; other chats queue here instead of corrupting the
        // running turn.
        // ponytail: one turn at a time. Upgrade path: an agent per seat
        // (the create_runtime_agent seam) if a busy chat starves others —
        // measure queue wait first.
        //
        // That measurement (iter-1338, audit operationalization step 4):
        // the counter below is the starvation signal, exposed through the
        // log line at turn end and `gateway status --deep`. It answers
        // "do concurrent chats queue, and for how long?" with a number, so
        // agent-per-seat is a decision made on evidence rather than a
        // guess. When the sum moved to seconds instead of milliseconds,
        // take the upgrade path.
        let queue_wait_start = std::time::Instant::now();
        let mut queued = false;
        // Queue visibility (plan 06A): if another chat's turn holds the
        // lock, show the platform typing indicator while this one waits.
        if self.turn_lock.try_lock().is_err() {
            queued = true;
            let gw = self.gateway.lock().await;
            if let Some(ref g) = *gw {
                let _ = g.send_typing(&message.platform, &message.channel_id, message.thread_id);
            }
        }
        let _turn = self.turn_lock.lock().await;
        let queue_wait = queue_wait_start.elapsed();
        if queued {
            self.queue_waits
                .lock()
                .await
                .record(queue_wait, &session_key);
        }

        let _running_marker = RunningSessionGuard(&self.running_session);
        {
            let mut running = self
                .running_session
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            // W3-2: the user `/stop`ped this turn while it was queued.
            if self
                .cancelled_queued
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&session_key)
            {
                return Err(operant_core::Error::Interrupted);
            }
            *running = Some(session_key.clone());
        }
        // Arm a clean interrupt latch at TURN START. This used to run in
        // the dispatch loop, where a second chat's dispatch could wipe a
        // `/stop` aimed at the RUNNING chat (plan 06A failure case 4).
        // Inside the lock the reset can only ever affect this turn.
        self.agent.interrupt_flag().reset();
        // Process-global per-turn state moves inside the lock for the same
        // reason (it used to be set in the dispatch closure, where two
        // overlapping chats could interleave their origins).
        operant_core::tools::cron_tool::set_cron_origin(Some(
            operant_core::tools::cron_tool::CronOrigin {
                platform: message.platform.clone(),
                chat_id: message.channel_id.clone(),
                thread_id: message.thread_id.map(|t| t.to_string()),
            },
        ));

        // Point the agent at THIS session before it runs. Without it the agent
        // minted a fresh `sess_<uuid>` per turn, so the tool trajectory was
        // orphaned into an id the reload below never reads — which is why a bare
        // `continue` after a restart came back to five prompts and no answers.
        // `set_session_id` takes `&self` and works through `Arc<OperantAgent>`,
        // the same shape as the `set_model` call further down.
        self.agent.set_session_id(session_id.clone());

        // Ensure session exists in DB
        let now = chrono::Utc::now().to_rfc3339();
        let _ = self
            .agent
            .db()
            .save_session(&session_id, None, "gateway", &now, &now);

        // Load conversation history only when the session changes.
        //
        // This block used to call `clear_history()` and manually re-add the
        // last 20 messages on EVERY gateway message. That broke Anthropic
        // prompt prefix caching (the message array was rebuilt from scratch
        // each turn, so the cache never hit) and, worse, it made two
        // concurrent gateway chats wipe each other's turns: they share one
        // agent, so switching sessions destroyed the other chat's context.
        //
        // `set_session_id` above now performs the swap itself — the outgoing
        // session's turns go back to the session store and the incoming
        // session's transcript is rehydrated in their place — so this block
        // only tracks which session is active for bridge state. Calling
        // `clear_history()` here would now be actively wrong: it discards the
        // addressed session from the store, destroying the transcript
        // `set_session_id` just rehydrated.
        let needs_reload = {
            let current = self.current_session_id.lock().await;
            *current != Some(session_id.clone())
        };
        if needs_reload {
            let mut current = self.current_session_id.lock().await;
            *current = Some(session_id.clone());
        }

        // Long-term memory recall is injected by the agent itself inside
        // run() (build_messages → provider.prefetch under <memory_context>).
        //
        // Attachments: the gateway cached platform attachments to local paths
        // (message.media_urls). Inject them into the query so the agent can
        // inspect them with native tools — vision_analyze for images,
        // file_read / STT for documents and audio. Hermes passes
        // `event.media_urls` to the agent the same way.
        //
        // RT-175 — the /goal + /subgoal runtime read runs BEFORE the query
        // is built below, because it mutates `message.content`: after the
        // binding, the mutation would reach neither the query nor the
        // persisted transcript. The commands write `goal`/`subgoal` + a
        // `goal_pending_inject` flag on the session row; this block injects
        // them into this turn's query (the same "[session goal]" marker the
        // TUI submits as a real user message) and consumes the flag, so the
        // goal reaches the agent exactly once. Without this read the
        // commands were theater: metadata written, reply claiming
        // success, agent never told.
        {
            let gw_guard = self.gateway.lock().await;
            if let Some(ref gw) = *gw_guard {
                let store = gw.get_session_store();
                if let Some(session) =
                    store.find_session(&message.platform, &message.user_id, &message.channel_id)
                {
                    let pending = session
                        .metadata
                        .get("goal_pending_inject")
                        .map(|v| v == "true")
                        .unwrap_or(false);
                    if pending {
                        let goal = session.metadata.get("goal").cloned().unwrap_or_default();
                        let subgoals = session.metadata.get("subgoal").cloned().unwrap_or_default();
                        if !goal.is_empty() {
                            let mut inject = format!("[session goal] {goal}");
                            if !subgoals.is_empty() {
                                inject.push_str(&format!("\n[session subgoals] {subgoals}"));
                            }
                            tracing::info!(
                                platform = %message.platform,
                                channel = %message.channel_id,
                                "Injecting pending session goal into this turn"
                            );
                            message.content = format!("{}\n\n{}", inject, message.content);
                        }
                        // Consume the flag whether or not there was goal text
                        // (a cleared goal must not re-inject forever).
                        store.update_session_metadata(
                            &message.platform,
                            &message.user_id,
                            &message.channel_id,
                            &[("goal_pending_inject".to_string(), "false".to_string())],
                        );
                    }
                }
            }
        }

        let query = if message.media_urls.is_empty() {
            message.content.clone()
        } else {
            let paths: Vec<String> = message
                .media_urls
                .iter()
                .map(|p| format!("- {p}"))
                .collect();
            format!(
                "{}\n\n[The user attached {} file(s), cached locally by the gateway:]\n{}\n\nInspect them with the appropriate tool: vision_analyze for images (pass the file path as image_url), file_read for text/documents, and the STT/audio tools for voice messages. Report what you find.",
                message.content,
                message.media_urls.len(),
                paths.join("\n")
            )
        };

        let user_content = message.content.clone();

        // (P0.4) Apply a `/connect` or `/model` made in the TUI since the last turn.
        if let Some(endpoint) = endpoint_changed_on_disk() {
            tracing::info!(
                base_url = %endpoint.base_url,
                model = %endpoint.model,
                "operant.toml endpoint changed; retargeting the gateway agent"
            );
            // The key the client is CURRENTLY using, when the config file
            // does not carry one. `parse_config_file` does NOT apply the
            // environment overrides, so in the documented setup (the key
            // lives in `~/.operant/.env`) `endpoint.api_key` is None — and
            // `set_endpoint(url, None)` EMPTIES a pool-backed client's
            // credentials. The agent then answers "no available keys"
            // after the operator changed the model, with a key that was
            // present the whole time. The `.env` rotation path already
            // has this fallback; this path needed the same one.
            let current_key = self.agent.current_api_key();
            self.agent.set_endpoint(
                &endpoint.base_url,
                endpoint.api_key.as_deref().or(current_key.as_deref()),
            );
            self.agent.set_model(endpoint.model);
        }

        // Read session metadata and apply overrides before running the agent.
        // (Bug #3 from iter-98 audit — 15 slash commands wrote to metadata
        // but the runner never read it back. Now /model, /yolo, /fast etc.
        // actually take effect.)
        let model_override: Option<String> = {
            let gw_guard = self.gateway.lock().await;
            if let Some(ref gw) = *gw_guard {
                let store = gw.get_session_store();
                store
                    .find_session(&message.platform, &message.user_id, &message.channel_id)
                    .and_then(|s| s.metadata.get("model_override").cloned())
                    .filter(|s| !s.is_empty())
            } else {
                None
            }
        };
        if let Some(ref model) = model_override {
            // (iter-162: apply model override via agent.set_model(). Was
            // previously read but not applied — the agent's config.model
            // was private with no setter API.)
            if *model != self.agent.model() {
                tracing::info!(model_override = %model, "Applying model override from session metadata");
                self.agent.set_model(model.clone());
            }
        }

        // Wave 2 (ORGANISM-ARCHITECTURE §2): apply the session's employee
        // binding — the bylaws SEAT this turn runs under, and the charter
        // the frozen prefix carries. The binding lives on the PERSISTENT
        // session store (survives restarts); the in-memory SessionStore is
        // metadata-only and holds no employee state. The persistent store
        // backfills `premiere` on load (owner ruling: default conversations
        // talk to the premiere), so `employee_id` is Some in every
        // post-migration row; the None arm is the pre-migration transient.
        //
        // Wave 3 reuses `bound_employee` twice below: the continuation
        // injection (a reloaded session begins from the rolling summary)
        // and the post-turn summary write. `store_session_key` carries the
        // STORE's canonical key (build_session_key: `agent:main:…`) to the
        // Wave-4 metering drain — the handler's `session_key`
        // (`{platform}:{channel}:{user}`) is a turn-lease namespace the store
        // never files under, so looking rows up by it always missed
        // ("Session not found", and the DM tap could never fire).
        let mut bound_employee: Option<String> = None;
        let mut store_session_key: Option<String> = None;
        // GW-3: every turn starts unseated; the binding below sets the seat
        // again only when this chat has an employee.
        self.agent.clear_seat_id();
        // KB-8: every turn starts in its unbound scope, so a sender with no
        // session row never inherits the previous turn's scope. A bound seat
        // overrides it below.
        let unbound_scope = unbound_scope_for(&self.admins, &message.platform, &message.user_id);
        if let Some(p) = self.agent.memory_provider() {
            p.set_scope(&unbound_scope);
        }
        {
            let gw_guard = self.gateway.lock().await;
            if let Some(ref gw) = *gw_guard
                && let Some(store) = gw.get_persistent_sessions()
            {
                // Resolve through the origin triple — the same seam the
                // `/session` command uses — instead of `get_entry` with
                // the turn-lease key. The gateway files every session
                // under `build_session_key(source)` at routing time
                // (gateway/mod.rs), so the triple always finds it.
                // ponytail: threads/topics with the same origin triple
                // resolve to the first matching entry — same ambiguity
                // `/session` has always had; revisit when
                // telegram_dm_topics_enabled is flipped on.
                if let Some(entry) =
                    store.entry_for_source(&message.platform, &message.user_id, &message.channel_id)
                {
                    store_session_key = Some(entry.session_key.clone());
                    // G+1: seat binding names a Holon employee id in org
                    // mode; ungoverned (None) in personal mode. No charter
                    // injection — job descriptions are Holon's now.
                    if let Some(emp) = entry.employee_id.clone() {
                        self.agent.set_seat_id(emp.clone());
                        // (plan 06B) the turn's memory bank follows the
                        // seat binding — inside the turn lock, so a
                        // concurrent chat's seat can never interleave: a
                        // seat's turn recalls and retains in `emp:<id>`
                        // and never sees the personal bank or another
                        // seat's (the owner's unbound chats keep the
                        // personal bank).
                        if let Some(p) = self.agent.memory_provider() {
                            p.set_scope(&operant_core::memory_wire::MemoryScope::Seat {
                                identity: emp,
                            })
                        }
                    }
                    self.agent.set_charter(None);
                    tracing::debug!(
                        session_key = %session_key,
                        "session runs as its bound employee"
                    );
                    bound_employee = entry.employee_id.clone();
                }
            }
        }

        // iter-684 (gap 5 phase 2, capture-at-inbound): tap this DM into the
        // seat's durable context store — the seat's LATER cycles (cron and
        // socialization) then see what the human asked, ranked under the Dm
        // quota. This turn itself never sees it echoed back: the transcript
        // check excludes the inbound message's content hash. Fail-open.
        if let (Some(injector), Some(seat)) =
            (self.context_injection.as_ref(), bound_employee.as_ref())
        {
            injector.record_inbound_dm(seat, &message.username, &message.content);
        }

        // Wave 3 (ORGANISM-ARCHITECTURE §3): a CONTINUATION begins from the
        // bound employee's rolling summary. Only when the transcript
        // actually reloaded (restart, session switch, /resume) — a live
        // conversation keeps its full in-memory context and must not see
        // the block, or the model would double-count prior turns.
        // TTL-guarded (read-time): an expired roll means a fresh start —
        // "always rolling, never aggregating old rolls".
        let query = if needs_reload && bound_employee.is_some() {
            let employee_id = bound_employee.clone().unwrap_or_default();
            let summary = {
                let gw_guard = self.gateway.lock().await;
                gw_guard
                    .as_ref()
                    .and_then(|gw| gw.get_persistent_sessions())
                    .and_then(|store| {
                        store
                            // GW-7: a roll belongs to a transcript, so it is keyed by the
                            // conversation id. A `/resume` onto another transcript reads none.
                            .valid_summary(&employee_id, &session_id, self.summary_ttl_minutes)
                            .ok()
                            .flatten()
                    })
            };
            match summary {
                Some((text, tokens)) if !text.is_empty() => {
                    tracing::debug!(
                        session_key = %session_key,
                        summary_tokens = tokens,
                        "Wave 3: continuation begins from the rolling summary"
                    );
                    format!("<continuation_summary>\n{text}\n</continuation_summary>\n\n{query}")
                }
                _ => query,
            }
        } else {
            query
        };

        // Wave 4 (ORGANISM-ARCHITECTURE §5): the budget gate + the
        // remaining-budget injection. Resolution: per-seat override row
        // (per-field precedence) over `[bylaws].budget`; cap <= 0 =
        // ungoverned, nothing is computed or injected. Metering READS the
        // existing per-session accumulator rolled up per employee over the
        // window — no second counter. Enforcement is TURN-BOUNDARY v1: a
        // turn that STARTS over a HARD cap is refused with a clear notice
        // (the owner's "stop and provide the summary"); soft = the remainder
        // is injected every turn so the agent self-economizes BEFORE the
        // cap, and past it, warns and continues. Mid-flight enforcement is
        // a recorded follow-up.
        // iter-633 (Wave-4 mid-flight): the per-turn HARD-cap envelope for
        // the loop's iteration-boundary check. Token basis only — usd-basis
        // seats keep turn-start enforcement (per-call catalog cost inside
        // the loop is a recorded follow-up). Set ONLY when the turn is
        // allowed to run: a refused turn (hard cap already blown at turn
        // start) never reaches the loop.
        let mut seat_envelope: Option<operant_core::agent::SeatBudgetEnvelope> = None;
        let query = if let Some(employee_id) = bound_employee.as_ref() {
            let seat_override = seat_budget_db().and_then(|db| db.get(employee_id).ok().flatten());
            let effective = operant_core::org::seat_budgets::resolve_budget(
                seat_override.as_ref(),
                &self.default_budget,
            );
            match effective {
                None => query,
                Some(budget) => {
                    let since = operant_core::org::seat_budgets::window_start(&budget.window);
                    let usage = {
                        let gw_guard = self.gateway.lock().await;
                        gw_guard
                            .as_ref()
                            .and_then(|gw| gw.get_persistent_sessions())
                            .map(|store| {
                                store
                                    .employee_window_usage(employee_id, &since)
                                    .map_err(|e| e.to_string())
                            })
                    };
                    match usage {
                        None => query,
                        // GW-8: an unreadable meter cannot show that a hard cap
                        // is unspent, so a hard cap refuses. Soft mode runs the
                        // turn and only loses the injected remainder.
                        Some(Err(err)) if budget.mode == "hard" => {
                            return Ok(OutgoingMessage::new(
                                message.channel_id.clone(),
                                format!(
                                    "🛑 The budget for `{employee_id}` could not be read ({err}), \
                                     so the turn was not run. Retry shortly."
                                ),
                            ));
                        }
                        Some(Err(err)) => {
                            tracing::warn!(
                                seat = %employee_id,
                                error = %err,
                                "budget usage unreadable — soft mode, running the turn"
                            );
                            query
                        }
                        Some(Ok((tokens_used, usd_used))) => {
                            let (used, remaining, unit) = if budget.basis == "usd" {
                                (usd_used, budget.cap - usd_used, "USD")
                            } else {
                                (
                                    tokens_used as f64,
                                    budget.cap - tokens_used as f64,
                                    "tokens",
                                )
                            };
                            if remaining <= 0.0 && budget.mode == "hard" {
                                tracing::warn!(
                                    seat = %employee_id,
                                    basis = %budget.basis,
                                    cap = budget.cap,                                    used = used,
                                    "Wave 4: hard budget cap — refusing the turn"
                                );
                                return Ok(OutgoingMessage::new(
                                    message.channel_id.clone(),
                                    format!(
                                        "🛑 Budget cap reached for `{employee_id}` — {used:.0} of \
                                         {cap:.0} {unit} used this {window}. The turn was not run. \
                                         Ask `{owner}` to raise the `seat_budgets` cap, or wait for the \
                                         window to roll.",
                                        cap = budget.cap,
                                        unit = unit,
                                        window = budget.window,
                                        owner = "hrmaster",
                                    ),
                                ));
                            }
                            // iter-633: arm the mid-flight envelope for a
                            // HARD token cap the turn still fits under — the
                            // loop stops the turn at its next iteration
                            // boundary once this turn's spend crosses the
                            // window line. Soft mode self-economizes via the
                            // injection above and never stops mid-flight;
                            // usd basis defers to turn-start enforcement.
                            if budget.mode == "hard" && budget.basis == "tokens" && remaining > 0.0
                            {
                                seat_envelope = Some(operant_core::agent::SeatBudgetEnvelope {
                                    cap_tokens: budget.cap,
                                    used_at_turn_start: tokens_used as f64,
                                });
                            }
                            let posture = if remaining <= 0.0 {
                                "OVER the cap (soft mode: continue, but say so)".to_string()
                            } else {
                                format!(
                                    "{remaining:.0} of {cap:.0} {unit} remain",
                                    cap = budget.cap,
                                    unit = unit
                                )
                            };
                            format!(
                                "<budget_state>\nEmployee `{employee_id}` budget — {basis} window {window}, mode {mode}: {posture}. Be mindful of consumption; prefer efficient tool use and concise reasoning while the budget is tight.\n</budget_state>\n\n{query}",
                                basis = budget.basis,
                                window = budget.window,
                                mode = budget.mode,
                            )
                        }
                    }
                }
            }
        } else {
            query
        };

        // iter-683 (gap 5 phase 1b): the DM/feed context section — the
        // gateway mirror of the scheduler's seam. The bound employee is
        // the reader; the agent's charter is the affinity corpus; the
        // inbound message is the transcript check (never echo it back).
        // None/empty/disabled → byte-identical (the iter-666 contract).
        let query = if let (Some(injector), Some(employee_id)) =
            (self.context_injection.as_ref(), bound_employee.as_ref())
        {
            let charter = self.agent.charter();
            let exclude = [operant_core::org::context_injection::content_hash(
                &message.content,
            )];
            injector.render_section(employee_id, charter.as_deref(), &query, &exclude)
        } else {
            query
        };

        // Turn start. `reloaded` is logged because it is the single fact that
        // explains what the model can see this turn: `true` means the
        // conversation was rebuilt from the last 20 stored messages of
        // `session_id` just above, `false` means the agent kept the transcript
        // it already had in memory.
        let turn_started = std::time::Instant::now();
        tracing::info!(
            session_key = %session_key,
            session_id = %session_id,
            agent_session_id = ?self.agent.session_id(),
            platform = %message.platform,
            channel = %message.channel_id,
            thread_id = ?message.thread_id,
            reloaded = needs_reload,
            has_media = !message.media_urls.is_empty(),
            "Gateway turn start"
        );

        // iter-633: scope the turn inside its seat-budget envelope when one
        // is armed — the loop's iteration-boundary check reads the task-local.
        // Ungoverned turns (no envelope) take the byte-identical legacy call.
        let retain = retains_turn_for(&self.admins, &message.user_id);
        let run_turn = |q: String| async move {
            operant_core::agent::TURN_RETENTION
                .scope(retain, async {
                    match seat_envelope {
                        Some(envelope) => {
                            operant_core::agent::SEAT_BUDGET_ENVELOPE
                                .scope(envelope, self.agent.run(q))
                                .await
                        }
                        None => self.agent.run(q).await,
                    }
                })
                .await
        };
        let mut run_result = run_turn(query.clone()).await;
        // Plan 06 P1: the context could not fit even after folding. Compact the
        // stored history and retry the turn once. The user message is already
        // stored, and `run` skips re-adding it, so the retry does not duplicate it.
        if matches!(
            run_result,
            Err(operant_core::error::Error::ContextExhausted { .. })
        ) {
            let (before, after) = self.agent.compact_history_now().await;
            tracing::warn!(
                messages_before = before,
                messages_after = after,
                "Context exhausted — compacted the stored history and retrying the turn once"
            );
            run_result = run_turn(query).await;
        }
        // Wave-4 metering wire (iter-632): drain the turn's accumulated
        // model usage into the session accumulator AFTER the turn (the
        // events fire during run()) but BEFORE the outcome arms, so error
        // and early-stop turns are metered too and no entry leaks into the
        // next turn on this thread. Before this wire nothing in production
        // called `update_tokens` — the budget rollup read permanently-zero
        // columns and the hard cap could never trip on real spend.
        {
            let usage_key = (
                message.platform.clone(),
                message.channel_id.clone(),
                message.thread_id,
            );
            let store = {
                let gw_guard = self.gateway.lock().await;
                gw_guard
                    .as_ref()
                    .and_then(|gw| gw.get_persistent_sessions())
                    .cloned()
            };
            if let Some(store) = store {
                // The store files rows under the canonical `build_session_key`
                // namespace — drain by that key (captured at the Wave-2
                // resolution above). Fallback keeps the legacy key when no
                // entry resolved (unseen-session edge), preserving the
                // fail-open warn instead of hard-failing the turn.
                let meter_key = store_session_key
                    .clone()
                    .unwrap_or_else(|| session_key.clone());
                drain_turn_usage_into_store(
                    &store,
                    &self.turn_usage,
                    usage_key,
                    &meter_key,
                    bound_employee.as_deref(),
                )
                .await;
            }
        }
        match run_result {
            Ok(response) => {
                let used_fallback = response.content.trim().is_empty();
                // S5 (BUGS.md) — the exit reason stashed by the event
                // receiver's Done handler says whether this was a real
                // answer (`TextResponse`) or an early stop (`GraceCall` /
                // `CircuitBreaker`). An early stop must read as one to the
                // operator: before this, a degenerate grace summary arrived
                // looking like the final answer and the agent then fell
                // silent (the 25-day-silence specimen).
                //
                // `remove` consumes the entry so the reason cannot leak into
                // the next turn on this thread.
                let exit_reason = {
                    let mut map = self.exit_reasons.lock().await;
                    map.remove(&(
                        message.platform.clone(),
                        message.channel_id.clone(),
                        message.thread_id,
                    ))
                };
                let early_stop =
                    exit_reason.filter(|r| *r != operant_core::agent::TurnExitReason::TextResponse);
                if let Some(reason) = early_stop {
                    tracing::warn!(
                        %reason,
                        session_key = %session_key,
                        "Turn ended early — substituting stopped-early notice for the operator"
                    );
                }
                let content = turn_end_content(
                    &response.content,
                    response.reasoning.as_deref(),
                    early_stop,
                    turn_started.elapsed(),
                );

                // Audit finding 3: the daemon's bursty per-turn allocations
                // (transcript rehydration, tool outputs, streaming buffers)
                // land in glibc arenas that RETAIN freed pages as RSS.
                // Return them to the OS once the turn settles — debounced
                // inside `trim()` so a burst of turns costs one sweep, not
                // one per turn.
                allocator::trim();

                // iter-1338: the queue-wait signal. The per-turn line names the
                // turn; the aggregate line (every 50th queued turn) is the
                // durable summary an operator greps the gateway log for —
                // "queued turns: N (total X ms, worst Y ms, busiest chat: Z)"
                // is the number the agent-per-seat decision needs, and the
                // turn_lock comment names this exact measurement.
                if queued {
                    tracing::info!(
                        session_key = %session_key,
                        queue_wait_ms = queue_wait.as_millis() as u64,
                        "Chat queued behind the running turn on the shared agent (audit gate for agent-per-seat)"
                    );
                    let summary_snapshot = {
                        let stats = self.queue_waits.lock().await;
                        stats.events % QUEUE_WAIT_SUMMARY_EVERY == 0
                    };
                    if summary_snapshot {
                        let line = self.queue_waits.lock().await.line();
                        tracing::info!("Shared-agent queue wait summary — {line}");
                    }
                }

                // Turn persistence to long-term memory happens inside
                // agent.run() via the agent's memory provider (sync_turn).

                // Save user message and assistant response to DB
                let _ = self
                    .agent
                    .db()
                    .save_message(&session_id, "user", &user_content, &now);
                let _ = self
                    .agent
                    .db()
                    .save_message(&session_id, "assistant", &content, &now);

                // Wave 3 (ORGANISM-ARCHITECTURE §3): write the ROLLING
                // summary — every completed turn upserts ONE row per
                // (employee, session): the previous roll plus this turn's
                // doings. Deterministic v1 (no extra LLM call): the roll is
                // the prior roll + the turn's final response, head-bounded
                // so a long-lived session cannot grow it without limit —
                // "rolling, never aggregating". An LLM-compressed roll is
                // a follow-up knob, not this wave's contract.
                if let Some(employee_id) = bound_employee.as_ref() {
                    let prior = self
                        .gateway
                        .lock()
                        .await
                        .as_ref()
                        .and_then(|gw| gw.get_persistent_sessions())
                        .and_then(|store| {
                            store
                                .valid_summary(employee_id, &session_id, self.summary_ttl_minutes)
                                .ok()
                                .flatten()
                        });
                    const MAX_SUMMARY_CHARS: usize = 8_000;
                    let mut roll = match &prior {
                        Some((text, _)) => format!("{text}\n\n"),
                        None => String::new(),
                    };
                    roll.push_str("Latest turn:\n");
                    roll.push_str(&content);
                    let roll = if roll.len() > MAX_SUMMARY_CHARS {
                        // Keep the TAIL: the latest turns are what a
                        // continuation most needs. Snap to a UTF-8 char
                        // boundary — a blind byte slice panics mid-character.
                        let mut start = roll.len() - MAX_SUMMARY_CHARS;
                        while !roll.is_char_boundary(start) {
                            start += 1;
                        }
                        roll[start..].to_string()
                    } else {
                        roll
                    };
                    let tokens = (roll.len() / 4).max(1) as i64; // ~4 chars/token estimate
                    let upserted = self
                        .gateway
                        .lock()
                        .await
                        .as_ref()
                        .and_then(|gw| gw.get_persistent_sessions())
                        .map(|store| store.upsert_summary(employee_id, &session_id, &roll, tokens));
                    match upserted {
                        Some(Ok(())) => {}
                        Some(Err(e)) => tracing::warn!(
                            session_key = %session_key,
                            error = %e,
                            "Wave 3: summary write failed — continuation falls back to raw transcript"
                        ),
                        None => {}
                    }
                }

                tracing::info!(
                    session_key = %session_key,
                    session_id = %session_id,
                    duration_ms = turn_started.elapsed().as_millis(),
                    response_len = content.len(),
                    empty_response_fallback = used_fallback,
                    "Gateway turn end: answered"
                );

                Ok(OutgoingMessage::new(message.channel_id, content)
                    .with_thread_id(message.thread_id))
            }
            Err(e) => {
                // A provider death used to reach the log only as the redacted
                // `route_message` failure far downstream, by which point the
                // session key and duration were gone. `error_class` makes the
                // failure legible as an event; the redacted `Display` is the same
                // scrubbed path every other error line in this file uses, and
                // carries no user content.
                tracing::error!(
                    session_key = %session_key,
                    session_id = %session_id,
                    error_class = error_class(&e),
                    duration_ms = turn_started.elapsed().as_millis(),
                    detail = %redact_err(&e),
                    "Gateway turn end: agent run failed"
                );

                // Still save the user message on error
                let _ = self
                    .agent
                    .db()
                    .save_message(&session_id, "user", &user_content, &now);

                // `user_message()`, not `Display`. `Display` double-prefixed the
                // text ("Error: Agent error: ...") and, for a provider failure,
                // surfaced the raw transport string with nothing the user could
                // act on. `StreamDied`'s own rendering already says the message
                // was not lost and to reply 'continue' to retry it.
                let text = e.user_message();

                // Persist the explanation too. Saving only the user row is what
                // made a failed turn look like a message the bot ignored: the
                // reloaded session showed prompts and no answers, so the history
                // actively misrepresented what had happened.
                let _ = self
                    .agent
                    .db()
                    .save_message(&session_id, "assistant", &text, &now);

                tracing::error!(
                    session_key = %session_key,
                    session_id = %session_id,
                    error_class = error_class(&e),
                    duration_ms = turn_started.elapsed().as_millis(),
                    "Gateway turn end: answered with error notice"
                );

                Ok(OutgoingMessage::new(&message.channel_id, text)
                    .with_thread_id(message.thread_id))
            }
        }
    }
}

/// Global gateway instance managed by the runner
static RUNNER: OnceLock<Mutex<Option<Arc<Gateway>>>> = OnceLock::new();

fn runner() -> &'static Mutex<Option<Arc<Gateway>>> {
    RUNNER.get_or_init(|| Mutex::new(None))
}

/// Build platform adapters from config
/// Platform adapter factory function type.
type AdapterFactory = fn(&GatewayConfig) -> Option<Arc<dyn PlatformAdapter>>;

/// Platform registry entry — maps a platform name to its factory function
/// and metadata. This replaces the hardcoded if/elif chain and allows
/// new platforms to be added by simply appending to the registry.
struct PlatformEntry {
    factory: AdapterFactory,
}

/// All supported platform adapters. To add a new platform:
/// 1. Implement the PlatformAdapter trait
/// 2. Add an entry here with a factory function
/// 3. Add config fields to GatewayConfig
///
/// The factory returns `Option<Arc<dyn PlatformAdapter>>` — None means the
/// platform is disabled in config, Some means it's enabled and constructed.
fn platform_registry() -> Vec<PlatformEntry> {
    vec![
        PlatformEntry {
            factory: |config| {
                if config.telegram_enabled {
                    Some(Arc::new(TelegramAdapter::with_config(
                        config.telegram_token.clone(),
                        config.telegram_bot_username.clone(),
                        config.telegram_dm_topics_enabled,
                        config.telegram_proxy.as_deref(),
                    )))
                } else {
                    None
                }
            },
        },
        PlatformEntry {
            factory: |config| {
                if config.discord_enabled {
                    Some(Arc::new(DiscordAdapter::new(config.discord_token.clone())))
                } else {
                    None
                }
            },
        },
        PlatformEntry {
            factory: |config| {
                if config.slack_enabled {
                    Some(Arc::new(SlackAdapter::new(config.slack_token.clone())))
                } else {
                    None
                }
            },
        },
        PlatformEntry {
            factory: |config| {
                if config.whatsapp_enabled {
                    Some(Arc::new(
                        WhatsAppAdapter::new(config.whatsapp_enabled)
                            .with_token(config.whatsapp_token.clone())
                            .with_phone_number_id(config.whatsapp_phone_number_id.clone()),
                    ))
                } else {
                    None
                }
            },
        },
        PlatformEntry {
            factory: |config| {
                if config.email_enabled {
                    Some(Arc::new(EmailAdapter::new(config.email_enabled).with_smtp(
                        config.email_smtp_host.clone(),
                        config.email_smtp_user.clone(),
                        config.email_smtp_pass.clone(),
                    )))
                } else {
                    None
                }
            },
        },
        PlatformEntry {
            factory: |config| {
                if config.sms_twilio_enabled {
                    Some(Arc::new(SmsAdapter::new(config.sms_twilio_enabled)))
                } else {
                    None
                }
            },
        },
        PlatformEntry {
            factory: |config| {
                if config.webhooks_enabled {
                    let addr = config
                        .webhooks_addr
                        .clone()
                        .unwrap_or_else(|| WebhookAdapter::DEFAULT_LISTEN_ADDR.to_string());
                    Some(Arc::new(
                        WebhookAdapter::new(config.webhooks_enabled)
                            .with_addr(addr)
                            .with_secret(config.webhooks_secret.clone())
                            .with_holon_opt(config.holon_notion.clone()),
                    ))
                } else {
                    None
                }
            },
        },
    ]
}

/// Build platform adapters from config using the platform registry.
/// Iterates the registry, calls each factory, and collects enabled adapters.
fn build_adapters(config: &GatewayConfig) -> Vec<Arc<dyn PlatformAdapter>> {
    platform_registry()
        .into_iter()
        .filter_map(|entry| (entry.factory)(config))
        .collect()
}

/// Map an `AppConfig`'s `[gateway]` section onto a `GatewayConfig`.
/// Extracted so `start_gateway` and one-shot sends share one mapping —
/// previously every literal was hand-maintained and drifted (e.g. the
/// `webhooks_secret` field was missing for a full round, silently disabling
/// signature verification).
/// docs/agentic-os phase F: derive the webhook adapter's Holon notion hook
/// from `[holon]` — mounted only when holon is enabled AND a notion adapter
/// script exists where the kernel's repo expects it. The script path is
/// `[holon] home`'s sibling `adapters/notion/holon_notion.py` when the home
/// is a checkout, else `~/.config/holon/notion/holon_notion.py` (installed).
fn holon_notion_hook(app_config: &AppConfig) -> Option<operant_core::gateway::HolonHook> {
    if !app_config.holon.enabled || app_config.holon.home.as_os_str().is_empty() {
        return None;
    }
    let home = app_config.holon.home.clone();
    // explicit [holon] notion_script wins; else the kernel checkout that
    // `bin` points into (<bin>/../adapters/notion/holon_notion.py —
    // O-16: the live instance home holds notion.json, not the script),
    // else the instance-home location for installed instances.
    let bin_fallback = app_config
        .holon
        .bin
        .as_deref()
        .and_then(|b| std::path::Path::new(b).parent())
        .map(|p| p.join("adapters").join("notion").join("holon_notion.py"));
    let home_fallback = home.join("adapters").join("notion").join("holon_notion.py");
    let fallback = bin_fallback.filter(|p| p.exists()).unwrap_or(home_fallback);
    let script = match &app_config.holon.notion_script {
        Some(p) if p.exists() => p.clone(),
        _ if fallback.exists() => fallback,
        _ => {
            tracing::warn!(
                home = %home.display(),
                "[holon] enabled but no notion adapter script found — webhook notion arm stays off"
            );
            return None;
        }
    };
    // O-08: the arm mounts at the instance's SECRET path — read once at
    // gateway start through the adapter itself (`webhook-path` creates
    // and persists a random route on first call). Unreadable → the arm
    // stays off entirely: a literal "notion" route is exactly the
    // guessable path the token-rotation guard exists to prevent.
    let wp = std::process::Command::new("python3")
        .arg(&script)
        .arg("--holon-home")
        .arg(&home)
        .arg("webhook-path")
        .output();
    let webhook_path = match wp {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => {
            tracing::warn!(
                home = %home.display(),
                "notion webhook-path unreadable — the secret-path arm stays off (O-08)"
            );
            return None;
        }
    };
    Some(operant_core::gateway::HolonHook {
        home,
        script,
        python_path: None,
        webhook_path,
    })
}

fn gateway_config_from_app(app_config: &AppConfig) -> GatewayConfig {
    GatewayConfig {
        telegram_enabled: app_config.gateway.telegram_enabled,
        telegram_token: app_config.gateway.telegram_token.clone(),
        discord_enabled: app_config.gateway.discord_enabled,
        discord_token: app_config.gateway.discord_token.clone(),
        slack_enabled: app_config.gateway.slack_enabled,
        slack_token: app_config.gateway.slack_token.clone(),
        whatsapp_enabled: app_config.gateway.whatsapp_enabled,
        whatsapp_token: app_config.gateway.whatsapp_token.clone(),
        whatsapp_phone_number_id: app_config.gateway.whatsapp_phone_number_id.clone(),
        email_enabled: app_config.gateway.email_enabled,
        email_smtp_host: app_config.gateway.email_smtp_host.clone(),
        email_smtp_user: app_config.gateway.email_smtp_user.clone(),
        email_smtp_pass: app_config.gateway.email_smtp_pass.clone(),
        sms_twilio_enabled: app_config.gateway.sms_twilio_enabled,
        webhooks_enabled: app_config.gateway.webhooks_enabled,
        webhooks_addr: app_config.gateway.webhooks_addr.clone(),
        webhooks_secret: app_config.gateway.webhooks_secret.clone(),
        holon_notion: holon_notion_hook(app_config),
        admins: app_config.gateway.admins.clone(),
        streaming_transport: app_config.gateway.streaming_transport.clone(),
        telegram_proxy: app_config.gateway.telegram_proxy.clone(),
        telegram_bot_username: app_config.gateway.telegram_bot_username.clone(),
        telegram_dm_topics_enabled: app_config.gateway.telegram_dm_topics_enabled,
        max_concurrent_sessions: app_config.gateway.max_concurrent_sessions,
    }
}

/// Send a message to a configured platform, using the running gateway when
/// available or a fresh one-shot gateway built from config otherwise.
///
/// Wired so `operant channel send` actually delivers instead of printing a
/// success stub (R15-2). Returns the platform's response/error text.
pub async fn send_channel_message(
    app_config: &AppConfig,
    platform: &str,
    recipient: &str,
    message: &str,
) -> Result<String> {
    // Prefer the live gateway so one-shot sends ride the same adapter
    // instances (tokens, proxy, streaming config) as the running service.
    let gw = runner().lock().await.clone();
    let msg = OutgoingMessage::new(recipient, message).no_markdown();
    if let Some(gw) = gw
        && gw.is_running().await
    {
        return gw
            .send_to_platform(platform, msg)
            .await
            .map(|_| format!("Sent via {platform} to {recipient}."))
            .map_err(anyhow::Error::from);
    }

    // No running gateway — build a one-shot gateway with the enabled
    // adapters and send directly (stateless platform APIs like Telegram's
    // sendMessage work without the listener running).
    let gw_config = gateway_config_from_app(app_config);
    let adapters = build_adapters(&gw_config);
    if adapters.is_empty() {
        anyhow::bail!(
            "No channels are enabled. Set gateway.{}_enabled = true in operant.toml first",
            platform
        );
    }
    let mut one_shot = Gateway::new(gw_config);
    for adapter in adapters {
        one_shot = one_shot.with_adapter(adapter);
    }
    one_shot
        .send_to_platform(platform, msg)
        .await
        .map(|_| format!("Sent via {platform} to {recipient}."))
        .map_err(anyhow::Error::from)
}

/// Delete a message operant sent earlier, on one configured platform.
///
/// Same two paths as `send_channel_message`: the live gateway first (its
/// adapters carry the tokens, proxy and per-platform settings), and a one-shot
/// gateway when the service is not running — Telegram's `deleteMessage` is a
/// stateless API call.
pub async fn delete_channel_message(
    app_config: &AppConfig,
    platform: &str,
    channel_id: &str,
    message_id: &str,
) -> Result<()> {
    let gw = runner().lock().await.clone();
    if let Some(gw) = gw
        && gw.is_running().await
    {
        return gw
            .delete_message_on_platform(platform, channel_id, message_id)
            .await
            .map_err(anyhow::Error::from);
    }

    let gw_config = gateway_config_from_app(app_config);
    let adapters = build_adapters(&gw_config);
    if adapters.is_empty() {
        anyhow::bail!(
            "No channels are enabled. Set gateway.{}_enabled = true in operant.toml first",
            platform
        );
    }
    let mut one_shot = Gateway::new(gw_config);
    for adapter in adapters {
        one_shot = one_shot.with_adapter(adapter);
    }
    one_shot
        .delete_message_on_platform(platform, channel_id, message_id)
        .await
        .map_err(anyhow::Error::from)
}

fn build_session_context(
    platform: &str,
    channel_id: &str,
    thread_id: Option<i64>,
    app_config: &AppConfig,
) -> String {
    let mut ctx = String::new();
    ctx.push_str(&format!("Connected platform: {}\n", platform));
    // Temporal grounding: without the wall-clock time the model cannot
    // resolve relative scheduling requests ("in 2 minutes") — R38 live-test
    // observed a cron job scheduled a YEAR out because 'now' was unknown.
    ctx.push_str(&format!(
        "Current time: {} UTC\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
    ));
    ctx.push_str(&format!(
        "Channel: {}\n",
        operant_core::redaction::redact_chat_id(channel_id)
    ));
    if let Some(tid) = thread_id {
        ctx.push_str(&format!("Topic/thread: {tid}\n"));
    }
    if app_config.gateway.telegram_enabled {
        ctx.push_str("Available: Telegram\n");
    }
    if app_config.gateway.discord_enabled {
        ctx.push_str("Available: Discord\n");
    }
    if app_config.gateway.slack_enabled {
        ctx.push_str("Available: Slack\n");
    }
    ctx
}

/// Start the gateway with the provided application configuration.
///
/// Constructs a `GatewayConfig` from `app_config.gateway.*` fields,
/// builds the enabled platform adapters, creates the gateway session
/// handler, and starts polling for incoming messages.
pub async fn start_gateway(app_config: &AppConfig) -> Result<String> {
    // Audit finding 3, first half: cap the arena count BEFORE the tokio
    // runtime spawns its workers. glibc's default is one arena per core;
    // on a 12-core box that is up to 12 × 64 MiB-aligned retained regions
    // for a daemon whose hot set is one conversation. `MALLOC_ARENA_MAX`
    // in the operator's environment still wins.
    allocator::configure();

    // W4 (audit O-12, O-13): the resident organs. When [holon] supervise
    // is on, the gateway runs the supervisor loop as a task — one claim
    // loop over supervise_employees under a max_concurrent_runs
    // semaphore, with backoff on error. When [holon] clock is on, it
    // fires `holon tick` every 2 minutes (the kernel's schedules and
    // lease sweeps — the clock belongs to the resident runtime).
    operant_os::organs::spawn_if_configured(app_config);

    // Finding 1 of the 2026-10-26 persistent-gateway audit: refuse to
    // become a SECOND daemon. The in-process RUNNER check below only covers
    // this process; the flock covers the MACHINE. `None` = a live gateway
    // holds the lock — say so and do not start.
    let lock_guard = match acquire_gateway_daemon_lock()? {
        Some(lock) => lock,
        None => {
            return Ok(
                "Gateway is already running on this machine — refusing to start a second daemon. \
                 Run `operant gateway status` to see it, or `operant gateway stop` first."
                    .to_string(),
            );
        }
    };
    // Leak the guard into the process-global slot so it outlives THIS
    // function (the iter-1345 defect: a local `let` dropped at this
    // return released the flock while the daemon kept serving).
    if let Ok(mut held) = GATEWAY_DAEMON_LOCKS.lock() {
        held.push(lock_guard);
    }

    init_pending_permissions();
    let mut guard = runner().lock().await;

    if guard.is_some()
        && let Some(gw) = guard.as_ref()
        && gw.is_running().await
    {
        return Ok("Gateway is already running.".to_string());
    }

    let gw_config = gateway_config_from_app(app_config);
    let dispatch_config = gw_config.clone();
    let adapters = build_adapters(&gw_config);

    let mut gateway = Gateway::new(gw_config);
    for adapter in adapters {
        gateway = gateway.with_adapter(adapter);
    }
    // v6 outline §2 P1-A: the per-channel dispatch breaker, on the app
    // database beside the session store (same file, separate table — a
    // hot channel must not get a fresh breaker by restarting the
    // gateway). Fail-open on an unusable store: a broken breaker file
    // logs and the gateway runs unbreaked rather than refusing to boot.
    match operant_core::gateway::dispatch_breaker::DispatchBreaker::init(
        app_config.database_path.clone(),
    ) {
        Ok(breaker) => {
            gateway = gateway.with_dispatch_breaker(std::sync::Arc::new(breaker));
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "dispatch breaker store unavailable — outbound sends run UNBREAKED"
            );
        }
    }

    // Attach persistent session store for cross-restart session tracking
    let session_db_path = app_config.database_path.clone();
    // iter-678 (gap 1): hoist the handle — the cron scheduler's usage drain
    // writes through the SAME store the budget rollup reads.
    let persistent_session_store: Option<std::sync::Arc<operant_core::PersistentSessionStore>> =
        session_db_path
            .to_str()
            .and_then(|path| operant_core::PersistentSessionStore::open(path).ok())
            .map(std::sync::Arc::new);
    if let Some(store) = persistent_session_store.clone() {
        gateway = gateway.with_persistent_sessions(store.clone());
        // The session store has grown without bound: `prune_old_entries`
        // existed since it was written and was called ONLY from its own
        // tests, so every conversation the gateway ever saw stayed in the
        // database and in the in-memory map forever. This is the periodic
        // caller it was missing.
        //
        // The task is detached (it dies with the process) and sweeps every
        // 6 hours. A sweep is a cheap scan of the in-memory map plus one
        // DELETE for the rows it removes.
        let retention_days = app_config.gateway.session_retention_days;
        if retention_days > 0 {
            tokio::spawn(session_retention_sweep(
                store,
                retention_days,
                SESSION_RETENTION_SWEEP_INTERVAL,
            ));
        }
    }

    let mcp_manager = McpManager::new();

    // Create event channel for tool progress previews
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    // (platform, channel_id, telegram thread_id) — thread_id is forwarded so
    // all outbound progress/stream/error messages land in the same forum
    // topic the user sent from (hermes topic-reply parity).
    let current_channel: Arc<Mutex<Option<CurrentChannel>>> = Arc::new(Mutex::new(None));

    // session_id -> (platform, channel_id, thread_id) for BACKGROUND events
    // (advisor/recap/self-improvement notes). These fire after the turn
    // ends, when the global `current_channel` slot may already point at
    // another session's thread — the registry pins each session's notes to
    // its own topic (iter-812, fixes cross-thread note delivery).
    let session_channels: Arc<Mutex<HashMap<String, CurrentChannel>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // (platform, channel_id) -> streamed message id. Set by the event
    // receiver when the first streaming chunk is delivered; the dispatch
    // loop consumes it after route_message so the final response is NOT
    // sent a second time (the streamed message was already edited to the
    // full text by AgentEvent::Done). This kills the duplicate-reply bug
    // where every gateway turn produced two identical messages.
    // Keyed by (platform, channel_id, thread_id) so concurrent forum topics
    // never collide on each other's stream/dedup markers (hermes session
    // isolation parity).
    let stream_delivery: StreamDeliveryMap = Arc::new(Mutex::new(HashMap::new()));
    let exit_reasons: ExitReasonMap = Arc::new(Mutex::new(HashMap::new()));
    let turn_usage: TurnUsageMap = Arc::new(Mutex::new(HashMap::new()));

    // Create permission channel for tool-approval flow (Bug #1 from iter-98
    // audit — gateway never called with_permissions, so bash/file_write ran
    // silently without approval). The TUI wires this at adapter_types.rs:2267;
    // the gateway now mirrors it.
    let (permission_tx, mut permission_rx) =
        mpsc::channel::<operant_core::agent::ToolPermissionRequest>(8);

    // Create user-question channel for clarify/AskUser flow (Bug #2 from
    // iter-98 audit — gateway never called set_user_question_sender, so the
    // clarify tool hung forever). The TUI wired this in iter-97; the gateway
    // now mirrors it.
    let (uq_tx, mut uq_rx) =
        mpsc::unbounded_channel::<operant_core::user_question::UserQuestionRequest>();
    let _ = operant_core::user_question::set_user_question_sender(uq_tx);

    // BUGS.md D-5 — build the memory pair ONCE for the whole gateway
    // process and thread the SAME pair into BOTH agent builds below (the
    // gateway agent and cron's). Every `build_agent_core` invocation used
    // to load a fresh pair: two `MemoryManager`s whole-file-writing one
    // MEMORY.md (bare `fs::write`, memory.rs), the second build repointing
    // the global `ACTIVE_MEMORY_MANAGER` that the first agent's
    // already-attached memory tools read, and cron getting a second
    // provider instance over the same store.
    let memory = crate::load_repo_memory_manager().await?;
    // P1/P2 seat authority (permission-bylaws wave-2 slices E + F2): the
    // bylaws's three stores over the shared org app db (the same
    // `database.db` derivation the PersistentSessionStore above uses; the
    // `seat_policies`, `authority_grants` and `pending_requests` tables are
    // created idempotently inside it), threaded into BOTH agent builds
    // below — the gateway agent consults it keyed by the chat session id,
    // cron by its derived employee id (`set_session_id(derive_employee_id(
    // &job.id))` in the scheduler). A seat with no row stays ungoverned
    // (today's behavior), so wiring the authority alone changes nothing
    // until a policy row is written. `?` (not a soft skip): a store that
    // fails to open must fail loudly — silently running ungoverned is the
    // widening the bylaws exists to make impossible.
    //
    // G+1 (docs/agentic-os/, owner order 2026-10-15): the org layer's
    // retired stores are gone — the grant ledger, the escalation queue,
    // the employee registry, the hierarchy edges, the cast. What the
    // gateway keeps is the enforcement machinery the boundary assigns
    // operant: the personal-mode seat policies (the enforcement surface)
    // and the seat budgets (window math; caps from policy.json.budget in
    // org mode). The approver is resolution-only now — an interactive
    // approval or denial resolves the in-flight ask; there is no grant
    // to mint and no durable row to record.
    let org_db = app_config
        .database_path
        .parent()
        .map(|p| p.join("operant_kanban.db"))
        .unwrap_or_else(|| std::path::PathBuf::from("operant_kanban.db"));
    let seat_policy_store = Arc::new(operant_core::org::seat_policy_db::SeatPolicyDb::init(
        org_db.clone(),
    )?);
    // The concrete store `/permissions` reads posture rows through; the
    // agents carry the `Arc<dyn SeatPolicySource>` view below.
    store_seat_policy_db(Arc::clone(&seat_policy_store));
    let seat_policies: Arc<dyn operant_core::org::seat_policy::SeatPolicySource> =
        seat_policy_store;
    let seat_budget_store = Arc::new(operant_core::org::seat_budgets::SeatBudgetDb::init(
        &org_db,
    )?);
    store_seat_budget_db(Arc::clone(&seat_budget_store));
    let seat_authority = Arc::new(operant_core::org::seat_authority::SeatAuthority::new(
        Arc::clone(&seat_policies),
    ));
    // The approver the dispatcher's `/approve` / `/deny` and the 60s
    // timeout resolve queued asks through. Resolution-only (G+1).
    let seat_approver = Arc::new(operant_core::org::seat_authority::SeatApprover::new(
        "gateway",
    ));
    store_seat_approver(seat_approver);
    let agent = crate::create_runtime_agent_with(
        app_config,
        &app_config.agent,
        None,
        event_tx.clone(),
        &mcp_manager,
        &app_config.skills.root_dir,
        None, // gateway has no status bar; the agent keeps its internal registry
        // `true`: this is the process's long-lived agent, so its LCM
        // maintenance workers must spawn (cron below passes `false`).
        true,
        // BUGS.md D-5 — the hoisted pair; see the comment above.
        Some(memory.clone()),
    )
    .await?;
    // Idle recap + advisor on the GATEWAY agent too (iters 793/794 wired
    // the TUI only — the flagship surface for both is a Telegram user
    // stepping away, so the gateway agent is where the engines earn their
    // keep). Delivery rides the same event channel the consumer task below
    // already drains; its BackgroundReview arm (added with this change)
    // sends the note/recap to the ACTIVE channel — no new plumbing.
    //
    // Recap activity: the handler's inbound-message path notifies the
    // handle on every user message (cancel-on-activity, omp parity).
    //
    // Cron agent: deliberately NOT given the engines. Its turns are
    // unattended seat cycles — no user is watching a cron transcript, a
    // 240s-idle recap would fire into an empty channel, and the advisor
    // would double LLM spend on every org seat tick. The TUI and the live
    // gateway chat are the interactive surfaces; that is where the
    // engines run.
    let mut agent = agent;
    let idle = app_config.agent.recap_idle_seconds;
    let gateway_recap_activity = agent.attach_recap_engine(
        std::time::Duration::from_secs(idle),
        app_config.recap.clone(),
        crate::client_config(app_config),
    );
    agent.attach_advisor(app_config.advisor.clone(), crate::client_config(app_config));
    // Wire the permission channel so the agent can request approval for
    // dangerous tools (bash, file_write, file_edit). Without this, the
    // agent runs tools silently. (Bug #1 from iter-98 audit.)
    // Clone before the gateway agent consumes the sender. Cron gets the same
    // channel so its permission posture is UNCHANGED by this commit (see below).
    let cron_permission_tx = permission_tx.clone();
    let agent = Arc::new(
        agent
            .with_permissions(permission_tx)
            .with_seat_authority(Some(Arc::clone(&seat_authority))),
    );

    // Cron gets its OWN agent, not a clone of the gateway's.
    //
    // They used to share one `Arc<OperantAgent>`, which is BUGS.md D-1 and
    // D-1b: `run_agent_job` called `clear_history()` on the shared agent, which
    // clears the hot conversation AND discards the persisted transcript from the
    // SessionStore, so a cron tick destroyed a live user's context
    // irrecoverably. Worse, a cron tool call that needed approval fell into the
    // gateway's no-active-channel branch (gateway_runner.rs:1938-1944), which
    // sends `AllowSession` — mutating the SHARED session_allowlist and granting
    // the human user's own bash/file_write without ever prompting them.
    //
    // The fresh `McpManager` is required, not cosmetic (BUGS.md D-3a):
    // `McpManager` is `Clone` over `Arc` and `sync_tools_to_registry` unregisters
    // every name in its shared `registered_tool_names` list from WHICHEVER
    // registry it is handed. Sharing it would mean cron's build unregistering
    // the gateway's live MCP tools out of the gateway's registry. A fresh
    // manager has no servers and its own empty list, so cron gets MCP management
    // tools only and cannot displace anything.
    //
    // PERMISSION POSTURE: cron is wrapped with the SAME permission channel the
    // gateway uses, and deliberately so. Before this change `cron_agent` was
    // `agent.clone()` of the ALREADY-WRAPPED agent, so `permission_tx` was
    // `Some` and the dangerous-tool guard at `agent/stream.rs:737` ran for cron
    // calls too. Omitting `with_permissions` here would set the Option to `None`
    // (default, `agent/builders.rs:67`), skip that guard entirely, and quietly
    // REMOVE a human veto: with a chat active, someone could previously be
    // prompted and refuse a cron's dangerous tool call. That is an
    // authorisation change disguised as a refactor, so this commit preserves the
    // existing posture instead.
    //
    // This closes D-1b regardless, because `session_allowlist` is per-agent
    // (`agent/builders.rs:68`): a cron `AllowSession` now inserts into CRON's
    // set, never the user's.
    //
    // Whether unattended cron *should* be granted dangerous tools is still open,
    // and is the owner's decision - see BUGS.md D-2. Note it is NOT solved by
    // handing cron a channel nobody drains: `stream.rs:788-791` sleeps 120s per
    // dangerous call before denying, so that stalls every job instead of failing
    // closed. A real fail-closed path needs deny-without-prompting (new code).
    // iter-1339 (audit delete: finding): the cron agent shares the
    // gateway's McpManager — the D-5 shape. `config.mcp.autoload` connects
    // every configured server INSIDE create_runtime_agent_with
    // (main.rs:1462), so two managers meant 2N MCP server subprocesses and
    // two stdio watchdog tasks for the same N configured servers (memory
    // bloat and duplicate child processes — the audit's fragmentation
    // family). The memory-pair hoist already proved this exact swap safe
    // for the gateway's two agents (iter-98, BUGS D-5); MCP config is
    // process-wide, so both agents' registries want the same tools.
    let cron_mcp_manager = mcp_manager.clone();
    // iter-678 (gap 1, metering): the cron agent gets its OWN event channel.
    // Sharing the gateway's had two defects: usage events accumulated under
    // the *current DM channel's* key and drained into a stranger's rollup
    // (the wave-4 trap — cron spend was invisible to every seat's budget),
    // and tool-progress previews from a 03:00 job landed in whatever human
    // chat happened to be active (the D-1b class). This drain task folds only
    // Usage/Cost into the scheduler's meter — the serial tick loop's
    // pre/post snapshots turn it into per-run deltas. Everything else drops.
    let (cron_event_tx, mut cron_event_rx) = mpsc::channel::<AgentEvent>(256);
    let cron_usage_meter = Arc::new(std::sync::Mutex::new(
        operant_core::org::worklog::UsageAccumulator::new(),
    ));
    {
        let meter = cron_usage_meter.clone();
        tokio::spawn(async move {
            while let Some(event) = cron_event_rx.recv().await {
                match event {
                    AgentEvent::Usage {
                        input_tokens,
                        output_tokens,
                        ..
                    } => {
                        if let Ok(mut acc) = meter.lock() {
                            acc.record(input_tokens, output_tokens);
                        }
                    }
                    AgentEvent::Cost { cost_usd, .. } => {
                        if let (Some(cost), Ok(mut acc)) = (cost_usd, meter.lock()) {
                            acc.record_cost(cost);
                        }
                    }
                    _ => {}
                }
            }
        });
    }
    let cron_agent = Arc::new(
        crate::create_runtime_agent_with(
            app_config,
            &app_config.agent,
            None,
            cron_event_tx,
            &cron_mcp_manager,
            &app_config.skills.root_dir,
            None,
            // `false`: the gateway agent above already spawned this process's
            // LCM maintenance workers (rollups + assertion extraction, each with
            // its own OpenAIClient). Passing `true` here would start a SECOND
            // pair over the same context engine - duplicated LLM spend on every
            // rollup tick and racing rollups. See create_runtime_agent_with.
            false,
            // BUGS.md D-5 — the SAME hoisted pair the gateway agent was built
            // with; constructing a fresh one here is what clobbered the
            // gateway's memory backend.
            Some(memory),
        )
        .await?
        .with_permissions(cron_permission_tx)
        .with_seat_authority(Some(Arc::clone(&seat_authority)))
        // F2: cron is the unattended build — a governed Escalate is queued
        // and denied THIS run (`agent/stream.rs`), never parked on the
        // interactive 120s stall; the next tick consults the grant the
        // approver minted between ticks. The ungoverned path ignores the
        // flag and keeps the channel posture this block preserves.
        .with_unattended(true),
    );

    // Long-term memory is handled by the hoisted pair above: BOTH agents
    // attach the SAME MemoryManager + provider (BUGS.md D-5) — cron never
    // constructs its own memory backend.

    // Baseline for the per-turn endpoint check; nothing to apply yet.
    endpoint_changed_on_disk();

    let handler = Arc::new(GatewayMessageHandler {
        agent,
        admins: app_config.gateway.admins.clone(),
        current_session_id: tokio::sync::Mutex::new(None),
        gateway: tokio::sync::Mutex::new(None),
        session_pins: Arc::new(SessionPins::default()),
        exit_reasons: exit_reasons.clone(),
        turn_usage: turn_usage.clone(),
        summary_ttl_minutes: app_config.bylaws.session_summary_ttl_minutes,
        default_budget: app_config.bylaws.budget.clone(),
        // iter-683: same injector the scheduler holds — one settings
        // surface for both seams. Fail-open: an injector that cannot open
        // leaves turns byte-identical.
        context_injection: operant_core::org::context_injection::ContextInjector::open(
            &app_config.database_path,
            app_config.context_injection.clone(),
        )
        .ok()
        .map(Arc::new),
        turn_lock: tokio::sync::Mutex::new(()),
        running_session: std::sync::Mutex::new(None),
        cancelled_queued: Default::default(),
        // RT-175: the runtime-command handles. The MCP manager is the
        // SAME clone-over-Arc instance the agent's tools live behind;
        // the server configs are send-safe tuples (the same shape the
        // TUI's reconnect task builds); the skills root is what
        // /reload-skills rescans.
        mcp_manager: mcp_manager.clone(),
        mcp_server_configs: app_config
            .mcp
            .servers
            .iter()
            .map(|s| {
                (
                    s.name.clone(),
                    s.transport.clone(),
                    s.url.clone(),
                    s.auth_token.clone(),
                    s.command.clone(),
                    s.args.clone(),
                    s.env.clone(),
                    s.enabled,
                )
            })
            .collect(),
        skills_root_dir: app_config.skills.root_dir.clone(),
        queue_waits: tokio::sync::Mutex::new(QueueWaitStats::default()),
    });
    gateway = gateway.with_handler(handler.clone());

    let gateway = Arc::new(gateway);

    // Now that the gateway is constructed, set the gateway reference on the
    // handler so it can read session.metadata before each agent.run() call.
    // (Bug #3 from iter-98 audit.)
    {
        let mut gw_guard = handler.gateway.lock().await;
        *gw_guard = Some(gateway.clone());
    }

    // Check for interrupted turns from previous session.
    //
    // The scan used to be called as a bare statement: it logged a WARN naming
    // the channel and then discarded the (channel_id, timestamp) pairs. The
    // user got nothing — a turn killed mid-flight by a restart simply vanished
    // from their side. Observed 2026-09-29: a turn started at 11:15:51, the
    // process died 5.2s later, and the next boot logged the warning to the
    // server log while Telegram showed 18 minutes of silence.
    //
    // Interrupted-turn detection. The scan runs here (it is pure I/O and must
    // happen before anything is started), but the NOTICE is deferred until
    // after `start_with_channel` below. Sending it from here would take
    // `send_channel_message`'s one-shot branch, build throwaway adapters, and
    // in practice return Ok without the message ever reaching Telegram --
    // verified against the live gateway on 2026-09-30, with no
    // "Failed to deliver" line because nothing errored.
    //
    // Deferring it rides the live polling adapter, which is the path the rest
    // of the gateway uses.
    let interrupted = collect_interrupted_turns();
    if !interrupted.is_empty() {
        tracing::warn!(
            count = interrupted.len(),
            "Detected interrupted turns from previous session"
        );
        // iter-691 — the CLOSE path. The turn is dead the moment this
        // process boots (the old process is gone; no executor can still
        // finish it), so each pending row flips terminal NOW, at detection,
        // not after a channel notice succeeds. Gating the close on delivery
        // meant a dead channel (the current Telegram hop) left rows
        // `pending` forever — the "still in-flight after restart" state,
        // re-detected and re-failed on every boot. The notice below is
        // best-effort and no longer guards the close; a late honest terminal
        // write from a draining old process would simply overwrite this, and
        // that is the right order of truth.
        for (channel_id, ts, platform) in &interrupted {
            save_turn_state_in_for(
                &operant_core::platform::operant_home(),
                channel_id,
                "interrupted",
                platform,
                Some(operant_core::gateway::lifecycle::TurnExitReason::Interrupted),
                Some("gateway restarted before the turn finished"),
            );
            tracing::info!(
                channel_id = %channel_id,
                pending_since = %ts,
                platform = %platform,
                "Closed orphaned turn as interrupted (restart reaper)"
            );
        }
    }

    let (message_tx, mut message_rx) = mpsc::unbounded_channel::<IncomingMessage>();
    // Clone kept by the dispatch loop so a queued message can be re-injected
    // into the channel once its session's in-flight turn completes (the
    // message arm re-runs interception and spawns the turn).
    let message_tx_for_reinject = message_tx.clone();
    gateway
        .start_with_channel(message_tx)
        .await
        .context("Failed to start gateway channel")?;

    // Now that the adapters are live, deliver the deferred interrupted-turn
    // notices.
    //
    // Send through the LOCAL `gateway` handle, NOT `send_channel_message`.
    // That helper takes `runner().lock().await` (:757), and `start_gateway`
    // holds that same mutex via `let mut guard = runner().lock().await` (:836)
    // for its entire body -- the guard is never dropped before this point. So
    // the call self-deadlocks: no error, no timeout, no log line. Measured on
    // 2026-09-30 as "detection logged, then silence" -- `Delivered` and
    // `Failed to deliver` both 0 across the whole log, process alive and
    // polling, because the dispatch loop is separate from this one.
    //
    // The local handle is alive here (start_with_channel just ran on it) and
    // needs no global lock.
    for (channel_id, _ts, platform) in &interrupted {
        let notice = "⚠️ Your previous turn was interrupted — the agent restarted \
                       before it finished. Nothing was sent back. Please re-send \
                       your request.";
        let msg = OutgoingMessage::new(channel_id, notice).no_markdown();
        // Best-effort only (iter-691): the row was already closed as
        // `interrupted` at detection time, above — a failed send must never
        // resurrect it.
        match gateway.send_to_platform(platform, msg).await {
            Ok(_) => tracing::info!(
                channel_id = %channel_id,
                platform = %platform,
                "Delivered interrupted-turn notice"
            ),
            Err(e) => tracing::error!(
                channel_id = %channel_id,
                platform = %platform,
                error = %e,
                "Failed to deliver interrupted-turn notice (row already closed)"
            ),
        }
    }

    let gw = gateway.clone();
    let admins_configured = !dispatch_config.admins.is_empty();
    let admins = dispatch_config.admins.clone();
    let telegram_token = dispatch_config.telegram_token.clone();

    if let Some(ref token) = telegram_token {
        // The CONFIGURED base, not the public API: a deployment that
        // reaches Telegram through a proxy (or a compatible endpoint)
        // must register its command menu there too — this is the one
        // request that used to go to the real API while every message
        // went to the configured endpoint.
        let api_base = operant_core::tools::platform_bases::telegram_api_base();
        let cmd_url = format!("{}/bot{}/setMyCommands", api_base, token);
        let cmd_body = format!(r#"{{"commands":{}}}"#, telegram_bot_commands());
        // Bounded retry: this fires once during boot — exactly when the
        // network may still be settling (the same race that kills adapter
        // starts). Without retries the command menu silently stays
        // unregistered until the next full gateway restart.
        // The same proxy the adapter's client uses. Without it the base
        // was right but the transport was not: a deployment that reaches
        // Telegram through a proxy could send messages (the adapter's
        // client is proxied) and could not register its command menu
        // (this client was not), so the menu silently stayed
        // unregistered.
        let mut client_builder = reqwest::Client::builder();
        if let Some(proxy) = operant_core::runtime_config()
            .gateway
            .telegram_proxy
            .as_deref()
            .filter(|p| !p.trim().is_empty())
        {
            match reqwest::Proxy::all(proxy) {
                Ok(proxy_obj) => {
                    client_builder = client_builder.proxy(proxy_obj);
                    tracing::info!("Telegram command registration using proxy: {}", proxy);
                }
                Err(e) => {
                    tracing::warn!("Invalid telegram_proxy '{}', ignoring: {}", proxy, e);
                }
            }
        }
        let client = client_builder
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        const MAX_ATTEMPTS: u32 = 5;
        for attempt in 1..=MAX_ATTEMPTS {
            match client
                .post(&cmd_url)
                .header("Content-Type", "application/json")
                .body(cmd_body.clone())
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status().is_success() {
                        tracing::info!("Telegram bot commands registered successfully");
                        break;
                    } else {
                        tracing::warn!(
                            "Failed to register Telegram commands: HTTP {} (attempt {}/{}),",
                            resp.status(),
                            attempt,
                            MAX_ATTEMPTS
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to register Telegram commands: {} (attempt {}/{}),",
                        redact_err(&e),
                        attempt,
                        MAX_ATTEMPTS
                    );
                }
            }
            if attempt < MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_secs(u64::from(attempt) * 5)).await;
            }
        }
    }

    /// True when a `Content` event is an agent-emitted meta-notice rather
    /// than model-generated prose. These are surfaced to the user (never
    /// filtered out) but as their OWN small message — they must not glue
    /// into the active writing block and turn the first message into a
    /// dumping ground.
    fn is_agent_meta_notice(text: &str) -> bool {
        text.starts_with("Empty assistant response — retrying")
            || text.starts_with("↻ Response truncated — requesting continuation")
            || text.starts_with("⚠ The model's thinking phase may have exceeded")
            || text.starts_with("🛑 Seat budget cap reached mid-turn")
            || (text.starts_with("⚠ Tool '") && text.contains("identical arguments"))
    }

    // Spawn tool progress event receiver — tool calls stream into a
    // per-group chronological message, and the group is CLOSED whenever the
    // agent emits text, finishes an iteration, or the turn ends, so the
    // next tool group (and every mid-stream text response) arrives as its
    // OWN message in chronological order (hermes `on_segment_break`
    // parity). Also drains AgentEvent::Content for streaming
    // progressive-edit (iter-100 — closes Bug #12 from iter-98 audit).
    let gw_for_events = gw.clone();
    let session_channels_for_events = session_channels.clone();
    let current_channel_for_events = current_channel.clone();
    let stream_delivery_for_events = stream_delivery.clone();
    let exit_reasons_for_events = exit_reasons.clone();
    let turn_usage_for_events = turn_usage.clone();
    tokio::spawn(async move {
        // Per-segment tool-progress messages (closed on text boundaries).
        let mut progress_tracker = ToolProgressTracker::new();
        // Streaming state: accumulated text + message_id for progressive edit.
        let mut stream_text = String::new();
        let mut stream_msg_id: Option<String> = None;
        let mut last_edit_time = std::time::Instant::now();
        let mut last_stream_sent = String::new();
        tracing::info!("Tool progress + streaming event receiver started");
        while let Some(event) = event_rx.recv().await {
            let (platform, channel_id, thread_id) =
                match current_channel_for_events.lock().await.as_ref() {
                    Some((p, c, t)) => (p.clone(), c.clone(), *t),
                    None => {
                        tracing::debug!(
                            "Dropping tool event (no active channel): {:?}",
                            std::mem::discriminant(&event)
                        );
                        continue;
                    }
                };
            let key = (platform.clone(), channel_id.clone(), thread_id);

            match event {
                AgentEvent::ToolStart {
                    tool_call_id,
                    name,
                    arguments,
                } => {
                    let line = tool_preview_line(&name, &arguments);

                    // Single-bubble-per-turn rendering (hermes
                    // stream_consumer parity): the content bubble stays open
                    // across the whole turn and is progressively edited;
                    // tool progress lives in ONE status bubble per turn,
                    // edited in place. The old segment-break behavior
                    // (finalize text + fresh message per tool group) is what
                    // shredded alternating text/tool output into dozens of
                    // fragments during degenerate model loops.

                    match progress_tracker.on_tool_start(&key, &tool_call_id, &line) {
                        ProgressAction::New { body } => {
                            // First tool of a group — send a NEW message.
                            let msg = OutgoingMessage::new(&channel_id, &body)
                                .no_markdown()
                                .with_thread_id(thread_id);
                            match gw_for_events.send_message_return_id(&platform, msg).await {
                                Ok(msg_id) => {
                                    progress_tracker.attach_message_id(&key, msg_id, &line);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %redact_err(&e),
                                        "Failed to send first progress message"
                                    );
                                }
                            }
                        }
                        ProgressAction::Edit { msg_id, body } => {
                            // Append to the group's existing chronological
                            // message (only while no text intervened).
                            let msg = OutgoingMessage::new(&channel_id, &body)
                                .no_markdown()
                                .with_thread_id(thread_id);
                            let _ = gw_for_events
                                .edit_message(&platform, &channel_id, &msg_id, msg)
                                .await;
                        }
                        ProgressAction::Skip => {}
                    }
                }
                AgentEvent::ToolComplete { result } => {
                    // Intercept TTS audio results and send as voice message
                    if result.name == "text_to_speech"
                        && result.success
                        && let Ok(data) = serde_json::from_str::<serde_json::Value>(&result.content)
                        && let Some(audio_b64) = data.get("audio").and_then(|a| a.as_str())
                    {
                        let format = data.get("format").and_then(|f| f.as_str()).unwrap_or("wav");
                        if let Ok(audio_bytes) =
                            base64::engine::general_purpose::STANDARD.decode(audio_b64)
                            && let Err(e) = gw_for_events
                                .send_voice(&platform, &channel_id, &audio_bytes, format)
                                .await
                        {
                            let _ = thread_id;
                            tracing::warn!(
                                error = %redact_err(&e),
                                "Failed to send TTS voice message"
                            );
                        }
                    }
                }
                AgentEvent::Content { text } => {
                    // Agent-emitted meta-notices (empty-response retries,
                    // truncation continuations, thinking timeouts, identical
                    // tool-call warnings) arrive as Content events. They must
                    // stay VISIBLE (never filtered out) but must never glue
                    // into the active writing block: finalize the current
                    // block, send the notice as its own small message, and
                    // reset the stream so the next real content starts fresh.
                    if is_agent_meta_notice(&text) {
                        if let Some(ref msg_id) = stream_msg_id {
                            if !stream_text.is_empty() {
                                let msg = OutgoingMessage::new(&channel_id, &stream_text)
                                    .with_thread_id(thread_id);
                                let _ = gw_for_events
                                    .edit_message(&platform, &channel_id, msg_id, msg)
                                    .await;
                            }
                            stream_text.clear();
                            stream_msg_id = None;
                            last_stream_sent.clear();
                        }
                        // Meta notices land between content — reset the tool
                        // group so subsequent tools open below them too.
                        progress_tracker.close_group(&key);
                        let notice =
                            OutgoingMessage::new(&channel_id, &text).with_thread_id(thread_id);
                        if let Err(e) = gw_for_events.send_to_platform(&platform, notice).await {
                            // A notice that could not be delivered is
                            // invisible otherwise: the result is
                            // discarded, and the delivery ledger is not
                            // something an operator watches. Log it.
                            tracing::warn!(
                                platform = %platform,
                                error = %e,
                                "could not deliver a gateway notice"
                            );
                        }
                    } else {
                        // Streaming progressive edit (iter-100 — closes Bug #12).
                        // ONE bubble per turn, edited in place every 500ms
                        // (hermes stream_consumer parity). The old per-block
                        // finalize made alternating text/tool output shred
                        // into dozens of messages.
                        stream_text.push_str(&text);
                        // Length guard: Telegram edits cap at 4096 chars —
                        // seal the current bubble and start a fresh one when
                        // the accumulated text approaches the limit. Split at
                        // the last newline inside the budget (hermes
                        // `_safe_limit` rfind parity) so sealed heads never
                        // cut mid-word/mid-table-row.
                        if stream_text.len() > 3500 {
                            let split_at = seal_split_point(&stream_text, 3500);
                            if let Some(ref msg_id) = stream_msg_id {
                                let head = stream_text[..split_at].to_string();
                                let msg = OutgoingMessage::new(&channel_id, &head)
                                    .with_thread_id(thread_id);
                                let _ = gw_for_events
                                    .edit_message(&platform, &channel_id, msg_id, msg)
                                    .await;
                                last_stream_sent = head;
                            }
                            stream_text =
                                stream_text[split_at..].trim_start_matches('\n').to_string();
                            stream_msg_id = None;
                            // A sealed head is delivered content below any
                            // tool chrome — same __reset__ rule as a fresh
                            // content bubble.
                            progress_tracker.close_group(&key);
                        }
                        // Debounce: only edit if 500ms have passed since the
                        // last edit, and skip no-op edits (hermes
                        // `_last_sent_text` parity — identical bodies are
                        // rejected as "message is not modified" anyway).
                        if last_edit_time.elapsed() > std::time::Duration::from_millis(500)
                            && last_stream_sent != stream_text
                        {
                            last_edit_time = std::time::Instant::now();
                            let msg = OutgoingMessage::new(&channel_id, &stream_text)
                                .with_thread_id(thread_id);
                            if let Some(ref msg_id) = stream_msg_id {
                                let _ = gw_for_events
                                    .edit_message(&platform, &channel_id, msg_id, msg)
                                    .await;
                            } else {
                                // First chunk — send a new message and store the id.
                                match gw_for_events.send_message_return_id(&platform, msg).await {
                                    Ok(msg_id) => {
                                        stream_msg_id = Some(msg_id.clone());
                                        last_stream_sent = stream_text.clone();
                                        // Record delivery so the dispatch loop
                                        // skips the duplicate final send —
                                        // but ONLY for a platform that can
                                        // EDIT the message it just sent.
                                        //
                                        // Only Telegram overrides
                                        // `send_message_return_id`; the
                                        // default sends the message and
                                        // returns an EMPTY id, because
                                        // the platform cannot edit. On
                                        // Discord and Slack the later
                                        // chunks' edits are no-ops, so the
                                        // user received the FIRST CHUNK of
                                        // the answer and never the rest —
                                        // and the marker suppressed the
                                        // dispatch loop's full send.
                                        //
                                        // So: an empty id means "this
                                        // platform cannot edit", and the
                                        // delivery is NOT recorded. The
                                        // dispatch loop then sends the
                                        // whole answer as a fresh message.
                                        //
                                        // The EMPTY id is still kept as
                                        // `stream_msg_id` — it is the
                                        // sentinel that keeps the later
                                        // ticks on the EDIT branch instead
                                        // of sending a new partial
                                        // message every 500ms.
                                        if should_record_stream_delivery(&msg_id) {
                                            let mut map = stream_delivery_for_events.lock().await;
                                            map.insert(
                                                (platform.clone(), channel_id.clone(), thread_id),
                                                msg_id,
                                            );
                                        }
                                        // Hermes `__reset__` linearization: a
                                        // content bubble just landed — close the
                                        // current tool-progress bubble so the
                                        // next tool opens a fresh bubble BELOW
                                        // the content, not the stale one above.
                                        progress_tracker.close_group(&key);
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %redact_err(&e),
                                            "Failed to send first stream chunk"
                                        );
                                    }
                                }
                            }
                            if stream_msg_id.is_some() {
                                last_stream_sent = stream_text.clone();
                            }
                        }
                    }
                }
                AgentEvent::Reasoning { text: _ } | AgentEvent::Thinking { content: _ } => {
                    // Reasoning/thinking tokens — don't surface to the user
                    // in gateway mode (the TUI shows them via /reasoning toggle,
                    // but gateway channels don't have that toggle). Just log.
                    tracing::debug!(
                        "Reasoning/thinking token received (not surfaced in gateway mode)"
                    );
                }
                AgentEvent::Done { reason, .. } => {
                    // S5: stash the turn's exit reason for the handler's
                    // turn-end block — a grace/circuit-breaker exit must
                    // surface as a stopped-early notice, not read as a full
                    // answer.
                    {
                        let mut map = exit_reasons_for_events.lock().await;
                        map.insert((platform.clone(), channel_id.clone(), thread_id), reason);
                    }
                    // Final message — do a final edit with the complete text
                    // of the last writing block. If the last block never got
                    // a message (debounce race: content arrived, then Done
                    // fired within 500ms), send it as a fresh message now.
                    if !stream_text.is_empty() {
                        let msg = OutgoingMessage::new(&channel_id, &stream_text)
                            .with_thread_id(thread_id);
                        if let Some(ref msg_id) = stream_msg_id {
                            // If the final edit fails (e.g. the accumulated
                            // stream exceeded Telegram's 4096-char edit limit
                            // after tool lines were merged in), drop the
                            // stream-delivery marker so the dispatch loop
                            // falls back to sending the full response as a
                            // fresh message instead of losing the answer.
                            let edit_ok = gw_for_events
                                .edit_message(&platform, &channel_id, msg_id, msg)
                                .await
                                .is_ok();
                            if !edit_ok {
                                let mut map = stream_delivery_for_events.lock().await;
                                map.remove(&(platform.clone(), channel_id.clone(), thread_id));
                            }
                        } else {
                            // No message yet — deliver the final block as a
                            // fresh message.
                            match gw_for_events.send_message_return_id(&platform, msg).await {
                                Ok(msg_id) => {
                                    // An EMPTY id means the platform cannot
                                    // edit (only Telegram overrides
                                    // `send_message_return_id`), so the
                                    // dispatch loop must still send the
                                    // full answer — the marker is only
                                    // recorded for a platform that can
                                    // edit the message it just sent.
                                    if should_record_stream_delivery(&msg_id) {
                                        let mut map = stream_delivery_for_events.lock().await;
                                        map.insert(
                                            (platform.clone(), channel_id.clone(), thread_id),
                                            msg_id,
                                        );
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %redact_err(&e),
                                        "Failed to send final stream block"
                                    );
                                }
                            }
                        }
                        // Reset streaming state for the next turn.
                        stream_text.clear();
                        stream_msg_id = None;
                    } else {
                        // Final writing block was EMPTY (provider returned an
                        // empty completion after the mid-turn blocks already
                        // finalized): nothing user-visible was delivered for
                        // the actual answer. Drop the stream-delivery marker
                        // so dispatch sends the fallback/response instead of
                        // suppressing it — suppressing here is what produced
                        // the silent-stall failure mode (R31 follow-up).
                        stream_text.clear();
                        stream_msg_id = None;
                        let mut map = stream_delivery_for_events.lock().await;
                        map.remove(&(platform.clone(), channel_id.clone(), thread_id));
                    }
                    // Turn ended — close the tool group so the next turn
                    // starts a fresh chronological message.
                    progress_tracker.close_group(&key);
                }
                AgentEvent::BackgroundReview {
                    summary,
                    session_id,
                } => {
                    // Advisor notes (⚑) and idle recaps (※) arriving on the
                    // GATEWAY: deliver to the session's OWN thread — not the
                    // global last-active slot. These fire AFTER the turn
                    // ends; on multi-thread chats (Telegram forum topics,
                    // concurrent sessions) the slot can already point at
                    // another session's topic, which delivered notes into
                    // the wrong thread (observed live 2026-10-14, iter-812).
                    //
                    // Resolution order: (1) the session registry entry the
                    // turn wrote at its start; (2) fall back to the current
                    // slot (legacy emitters with an empty tag, single-DM
                    // setups where both are identical).
                    let pinned_channel = session_channels_for_events
                        .lock()
                        .await
                        .get(&session_id)
                        .cloned();
                    let slot_channel = current_channel_for_events.lock().await.clone();
                    let target =
                        resolve_background_target(&session_id, pinned_channel, slot_channel);
                    match target {
                        Some((platform, channel_id, thread_id)) => {
                            let msg = OutgoingMessage::new(&channel_id, &summary)
                                .with_thread_id(thread_id);
                            let _ = gw_for_events.send_message_return_id(&platform, msg).await;
                        }
                        None => {
                            // No session entry and no active channel: the
                            // session never had a turn this boot. Drop —
                            // same cold-boot contract as before.
                            tracing::debug!(
                                session = %session_id,
                                "BackgroundReview dropped: no channel for session"
                            );
                        }
                    }
                }
                AgentEvent::IterationComplete { .. } => {
                    // Segment break (hermes MessageStop/_NEW_SEGMENT parity):
                    // finalize this writing block as its own message and
                    // reset stream state, so post-tool text starts a fresh
                    // message BELOW the tool chrome instead of being edited
                    // into the block above it. Chronological interleaving:
                    // [text] [tools] [text] [tools] … [final]. The R33
                    // single-bubble merge destroyed this ordering; the R33
                    // shredding it fixed is now contained by the circuit
                    // breaker + ×N dedup + line cap instead.
                    if !stream_text.is_empty() {
                        if let Some(ref msg_id) = stream_msg_id {
                            let msg = OutgoingMessage::new(&channel_id, &stream_text)
                                .with_thread_id(thread_id);
                            let _ = gw_for_events
                                .edit_message(&platform, &channel_id, msg_id, msg)
                                .await;
                        }
                        stream_text.clear();
                        stream_msg_id = None;
                        last_stream_sent.clear();
                    }
                }
                AgentEvent::Usage {
                    input_tokens,
                    output_tokens,
                    ..
                } => {
                    // iter-632 (Wave-4 metering wire): fold every model
                    // call's tokens into the per-thread TurnUsage; the
                    // turn-end block drains it into the session-store
                    // accumulator the budget rollup reads. Still not
                    // surfaced to the user — the TUI uses these for
                    // /stats overlays, the gateway counts them instead.
                    let mut map = turn_usage_for_events.lock().await;
                    let entry = map.entry(key).or_default();
                    entry.input_tokens += u64::from(input_tokens);
                    entry.output_tokens += u64::from(output_tokens);
                }
                // iter-632: the cost half of the metering wire — emitted right
                // after Usage from the models_dev catalog rates. `None` (model
                // not in catalog) is a legitimate zero-cost estimate, so it
                // matches no arm.
                AgentEvent::Cost {
                    cost_usd: Some(cost),
                    ..
                } => {
                    let mut map = turn_usage_for_events.lock().await;
                    map.entry(key).or_default().cost_usd += cost;
                }
                AgentEvent::Error { error } => {
                    // Surface errors to the user.
                    let body = format!("❌ Error: {}", error);
                    let msg = OutgoingMessage::new(&channel_id, &body)
                        .no_markdown()
                        .with_thread_id(thread_id);
                    if let Err(e) = gw_for_events.send_to_platform(&platform, msg).await {
                        // An ERROR message that could not be delivered is
                        // the worst kind of silent failure: the operator
                        // sees nothing at all, and the turn's error is
                        // lost with it.
                        tracing::warn!(
                            platform = %platform,
                            error = %e,
                            "could not deliver an error notice to the user"
                        );
                    }
                    // Reset streaming state + drop the dedup marker so the
                    // dispatch loop isn't left suppressing a future send.
                    stream_text.clear();
                    stream_msg_id = None;
                    progress_tracker.close_group(&key);
                    let mut map = stream_delivery_for_events.lock().await;
                    map.remove(&(platform.clone(), channel_id.clone(), thread_id));
                }
                AgentEvent::ToolPermissionRequest { .. } => {
                    // Permission requests are drained by the dedicated
                    // permission_rx task (iter-99). The event_rx copy is
                    // a duplicate — skip it.
                }
                _ => {}
            }
        }
        tracing::warn!("Tool progress + streaming event receiver exited (event_rx closed)");
    });

    // Drain permission requests from the agent. When the agent calls a tool
    // that needs approval (bash, file_write, file_edit), it sends a
    // ToolPermissionRequest. We surface it to the user via the chat platform
    // and await their /approve or /deny response. (Bug #1 from iter-98 audit.)
    //
    // ToolPermissionRequest. We surface it to the user via the chat platform
    // and await their /approve or /deny response. (iter-160 — replaces the
    // auto-approve mode with real per-platform prompts.)
    //
    // The flow:
    // 1. Permission request arrives → send "🔧 Tool: X — description. Reply
    //    /approve to allow, /deny to cancel" to the current channel.
    // 2. Store the pending request (response_tx) in a shared HashMap keyed
    //    by channel_id.
    // 3. When /approve or /deny arrives (handled in gateway_commands.rs),
    //    it looks up the pending request and sends AllowSession/Deny.
    // 4. If no response within 60s, auto-deny (safety timeout).
    let gw_for_perm = gw.clone();
    let current_channel_for_perm = current_channel.clone();
    // D-2 (owner ruling 2026-10-08): the unattended posture, captured at
    // start — the receiver consults it when nobody can answer a prompt.
    let unattended_posture_for_perm = app_config.bylaws.unattended_posture.clone();
    let pending_permissions: Arc<
        Mutex<HashMap<String, operant_core::agent::ToolPermissionRequest>>,
    > = Arc::new(Mutex::new(HashMap::new()));
    let pending_permissions_for_cmd = pending_permissions.clone();
    let pending_permissions_for_perm = pending_permissions.clone();

    // Spawn the permission receiver task
    tokio::spawn(async move {
        tracing::info!("Permission request receiver started (prompt mode)");
        while let Some(req) = permission_rx.recv().await {
            tracing::info!(
                tool = %req.tool_name,
                description = %req.description,
                "Permission request received — prompting user"
            );

            // Get the current channel to send the prompt to
            let channel_info = current_channel_for_perm.lock().await.clone();

            if let Some((platform, channel_id, thread_id)) = &channel_info {
                // RT-179 plan mode: a channel that asked for read-only
                // behaviour never prompts — the gated tool is denied
                // outright, exactly as the TUI's PermissionMode::Plan
                // drain does (tui/app/channel_drains.rs:389). Checked
                // BEFORE YOLO so a channel in both sets stays read-only:
                // read-only is the posture that must never widen by an
                // accident of two toggles.
                //
                // A seat-policy escalation is refused with the SAME audit
                // recording as the YOLO arm (F2) — plan mode is a posture
                // choice, not authority over a governed seat, and the
                // queued row must survive for the approver.
                if crate::gateway_runner::plan_mode_enabled(platform, channel_id) {
                    if req.seat_escalation.is_some() {
                        tracing::warn!(
                            channel = %channel_id,
                            tool = %req.tool_name,
                            "plan mode denies a seat-policy escalation — the ask stays queued for the approver"
                        );
                        resolve_seat_escalation_on_deny(&req);
                    }
                    tracing::info!(
                        channel = %channel_id,
                        tool = %req.tool_name,
                        "plan mode — denying the gated tool without prompting"
                    );
                    let _ = req
                        .response_tx
                        .send(operant_core::agent::ToolPermissionResponse::Deny);
                    continue;
                }
                // YOLO mode: the channel opted into skipping approval prompts
                // via /yolo. (R17: /yolo previously wrote a metadata key that
                // nothing read — the receiver now honors the live set, mirroring
                // the TUI's PermissionMode::BypassPermissions.)
                if yolo_enabled(platform, channel_id) {
                    // F2: a channel's YOLO toggle is the operator's wish to
                    // skip PROMPTS — it is not authority over a governed
                    // seat's policy. A seat-policy escalation is refused
                    // here (its queue row already stands; the approver
                    // resolves it), while ungoverned prompts keep the
                    // auto-approve byte-for-byte.
                    if req.seat_escalation.is_some() {
                        tracing::warn!(
                            channel = %channel_id,
                            tool = %req.tool_name,
                            "YOLO mode cannot approve a seat-policy escalation — denying; the ask stays queued"
                        );
                        resolve_seat_escalation_on_deny(&req);
                        let _ = req
                            .response_tx
                            .send(operant_core::agent::ToolPermissionResponse::Deny);
                        continue;
                    }
                    tracing::info!(
                        channel = %channel_id,
                        "YOLO mode — auto-approving tool permission"
                    );
                    let _ = req
                        .response_tx
                        .send(operant_core::agent::ToolPermissionResponse::AllowSession);
                    continue;
                }
                // Send the permission prompt. Platforms that support
                // interactive components (Telegram) render tappable
                // approve/deny buttons via their send_approval_prompt
                // override; the rest fall back to the plain-text /approve
                // /deny prompt. (hermes send_exec_approval parity.)
                let prompt_msg_id: Option<String> = if let Some(adapter) =
                    gw_for_perm.adapter_for(platform)
                {
                    adapter
                        .send_approval_prompt(
                            channel_id,
                            *thread_id,
                            &req.tool_name,
                            &req.description,
                        )
                        .await
                        .unwrap_or(None)
                } else {
                    let prompt = format!(
                        "🔧 Permission required: {} — {}\nReply /approve to allow, /deny to cancel (60s timeout)",
                        req.tool_name, req.description
                    );
                    let msg = OutgoingMessage::new(channel_id, &prompt)
                        .no_markdown()
                        .with_thread_id(*thread_id);
                    if let Err(e) = gw_for_perm.send_to_platform(platform, msg).await {
                        // A PERMISSION PROMPT that could not be delivered
                        // leaves the turn blocked forever, with no
                        // visible cause.
                        tracing::warn!(
                            platform = %platform,
                            error = %e,
                            "could not deliver a permission prompt to the user"
                        );
                    }
                    None
                };

                let timeout_tool_name = req.tool_name.clone();

                // Store the pending request keyed by channel_id
                pending_permissions_for_perm
                    .lock()
                    .await
                    .insert(channel_id.clone(), req);

                // Spawn a timeout task — auto-deny after 60s, then edit
                // the prompt to remove stale buttons (hermes parity).
                let pending_for_timeout = pending_permissions_for_perm.clone();
                let timeout_channel = channel_id.clone();
                let timeout_gw = gw_for_perm.clone();
                let timeout_platform = platform.clone();
                let timeout_thread_id = *thread_id;
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    let mut pending = pending_for_timeout.lock().await;
                    if let Some(req) = pending.remove(&timeout_channel) {
                        tracing::warn!(
                            channel = %timeout_channel,
                            "Permission request timed out — auto-denying"
                        );
                        // F2: a lapsed seat escalation is recorded as expired
                        // (§8.3 — the attempt is denied, the row survives
                        // for audit; nobody answered, so the resolver is the
                        // operator literal).
                        resolve_seat_escalation_on_expire(&req);
                        let _ = req
                            .response_tx
                            .send(operant_core::agent::ToolPermissionResponse::Deny);
                    }
                    // Edit the prompt to remove stale buttons and show
                    // expiry — prevents confusing stale-button taps.
                    if let Some(ref msg_id) = prompt_msg_id {
                        let expiry_msg = OutgoingMessage::new(
                            &timeout_channel,
                            format!(
                                "⏱️ Permission for `{}` timed out (auto-denied).",
                                timeout_tool_name
                            ),
                        )
                        .no_markdown()
                        .with_thread_id(timeout_thread_id);
                        let _ = timeout_gw
                            .edit_message(&timeout_platform, &timeout_channel, msg_id, expiry_msg)
                            .await;
                    }
                });
            } else {
                // No active channel — auto-approve (can't prompt). UNGOVERNED
                // requests only: this arm is exactly the pre-F2 behaviour and
                // stays byte-identical for them. A seat-policy escalation has
                // no interactive approver to fall back on — its queue row
                // already stands (§8.1), so this attempt is denied and the
                // next run consults the grant the approver mints.
                if req.seat_escalation.is_some() {
                    tracing::warn!(
                        tool = %req.tool_name,
                        "No active channel for a seat-policy escalation — denying; the ask stays queued"
                    );
                    let _ = req
                        .response_tx
                        .send(operant_core::agent::ToolPermissionResponse::Deny);
                    continue;
                }
                // D-2 (owner ruling 2026-10-08): an UNGOVERNED unattended
                // request with nobody to answer it consults
                // `[bylaws].unattended_posture` — the governance knob that,
                // per the ruling, solely manages the unattended posture.
                // `yolo` (the shipped default) keeps the auto-approve below
                // byte-identical; any other spelling denies with the
                // actionable reason. Attended requests never reach this
                // consult — their behavior is unchanged either way.
                if req.unattended
                    && let Some(_reason) =
                        unattended_no_channel_verdict(&unattended_posture_for_perm)
                {
                    tracing::warn!(
                        tool = %req.tool_name,
                        posture = %unattended_posture_for_perm,
                        "Unattended permission request with no active channel — denying by unattended posture"
                    );
                    let _ = req
                        .response_tx
                        .send(operant_core::agent::ToolPermissionResponse::Deny);
                    continue;
                }
                tracing::warn!("No active channel for permission prompt — auto-approving");
                let _ = req
                    .response_tx
                    .send(operant_core::agent::ToolPermissionResponse::AllowSession);
            }
        }
        tracing::warn!("Permission request receiver exited (permission_rx closed)");
    });

    // Store the pending_permissions Arc so gateway_commands can access it
    // when /approve or /deny is received. Overwrites the `None` that
    // `init_pending_permissions()` seeded above (see store_pending_permissions).
    store_pending_permissions(pending_permissions_for_cmd);

    // Drain user-question requests from the clarify tool. When the agent
    // calls clarify(), it pushes a UserQuestionRequest and blocks. We surface
    // the question to the user via the chat platform and await their reply.
    // (Bug #2 from iter-98 audit.)
    //
    // For now, we send the question as a plain text message and wait for the
    // user's next message as the reply. A future iteration will add
    // per-platform inline buttons (Telegram InlineKeyboardMarkup, Discord
    // View, Slack Block Kit, WhatsApp native poll).
    let gw_for_uq = gw.clone();
    let current_channel_for_uq = current_channel.clone();
    // Pending clarify() questions, keyed by `platform:channel_id` (the same
    // key the message handler uses to intercept replies).
    let pending_uq: Arc<Mutex<HashMap<String, PendingUserQuestion>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Store in the global so GatewayMessageHandler can check it
    if let Some(global_uq) = PENDING_USER_QUESTIONS.get()
        && let Ok(mut guard) = global_uq.lock()
    {
        *guard = Some(pending_uq.clone());
    }
    let pending_uq_for_task = pending_uq.clone();
    tokio::spawn(async move {
        tracing::info!("User question receiver started");
        while let Some(req) = uq_rx.recv().await {
            tracing::info!(question = %req.question, "User question received from clarify tool");
            // Surface the question to the user.
            if let Some((platform, channel_id, thread_id)) =
                current_channel_for_uq.lock().await.as_ref()
            {
                let channel_key = format!("{platform}:{channel_id}");
                // Platforms with interactive components render tappable
                // choice buttons (hermes `send_clarify` parity); the rest
                // fall back to a plain-text numbered list. Either way the
                // question is threaded into the ongoing topic.
                if let (Some(adapter), Some(choices)) =
                    (gw_for_uq.adapter_for(platform), req.choices.as_ref())
                {
                    let _ = adapter
                        .send_choice_prompt(channel_id, *thread_id, &req.question, choices)
                        .await;
                } else {
                    let body = if let Some(ref choices) = req.choices {
                        format!(
                            "❓ {}\n\n{}",
                            req.question,
                            choices
                                .iter()
                                .enumerate()
                                .map(|(i, c)| format!("{}. {}", i + 1, c))
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    } else {
                        format!("❓ {}", req.question)
                    };
                    let msg = OutgoingMessage::new(channel_id, &body)
                        .no_markdown()
                        .with_thread_id(*thread_id);
                    if let Err(e) = gw_for_uq.send_to_platform(platform, msg).await {
                        // A QUESTION that could not be delivered leaves the
                        // turn waiting for an answer that can never come.
                        tracing::warn!(
                            platform = %platform,
                            error = %e,
                            "could not deliver a question to the user"
                        );
                    }
                }

                // Store the reply_tx (plus the original choices, so a button
                // tap can map its index back to the option text) keyed by the
                // same `platform:channel_id` key the message handler uses to
                // intercept replies. (iter-161 — replaces the hardcoded
                // placeholder string with real reply interception.)
                pending_uq_for_task
                    .lock()
                    .await
                    .insert(channel_key.clone(), (req.reply_tx, req.choices));

                // Spawn a timeout — if no reply in 120s, send a timeout message
                let pending_uq_for_timeout = pending_uq_for_task.clone();
                let timeout_channel_key = channel_key.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                    let mut pending = pending_uq_for_timeout.lock().await;
                    if let Some((reply_tx, _choices)) = pending.remove(&timeout_channel_key) {
                        tracing::warn!(
                            channel = %timeout_channel_key,
                            "User question timed out — sending timeout reply"
                        );
                        let _ =
                            reply_tx.send("(no reply received within 120s — timeout)".to_string());
                    }
                });
            } else {
                // No active channel — can't prompt
                let _ = req
                    .reply_tx
                    .send("(no active channel for user question)".to_string());
            }
        }
        tracing::warn!("User question receiver exited (uq_rx closed)");
    });

    let app_config_clone = app_config.clone();
    let handler_for_stop = handler.clone();
    let stream_delivery_for_dispatch = stream_delivery.clone();
    tokio::spawn(async move {
        // ── Turn dispatch: per-session serialization with live dialogs ────────
        // Only one agent turn runs per session at a time; further messages for
        // a busy session are FIFO-queued and run when the current turn finishes
        // (hermes per-session queue). Dialog messages — approval / choice
        // button taps, /approve /deny, clarify replies — are resolved INLINE
        // below and never queued, so a long-running turn can never deadlock an
        // interactive prompt. Previously the loop awaited the whole turn
        // inline: a tap or reply queued behind the very turn that spawned the
        // dialog, and the tool's timeout killed it before the user could
        // respond (clarify / approval_request always timed out).
        let gw_for_turns = gw.clone();
        let app_config_for_turns = app_config_clone.clone();
        let stream_delivery_for_turns = stream_delivery_for_dispatch.clone();
        let current_channel_for_turns = current_channel.clone();
        let telegram_token_for_turns = telegram_token.clone();
        // Idle-recap cancel handle (omp parity): cloned per turn so the
        // dispatch loop's original stays valid — same Arc-free clone
        // discipline as the channel clones above.
        let recap_activity_for_turns = gateway_recap_activity.clone();
        // A live handle on each running turn's interrupt flag, so `/stop` has
        // real machinery to signal: the agent's run loop already polls this
        // exact flag. A `()` here — which is what this used to be — meant there
        // was nothing to signal at all, and `/stop` reported a cancellation that
        // never happened.
        let mut turn_inflight: HashMap<String, InterruptFlag> = HashMap::new();
        // Same handle the turn handler consults, so `/resume` can re-point this
        // chat at a stored session before the next turn derives its own id.
        let session_pins = handler_for_stop.session_pins.clone();
        let mut turn_queue: VecDeque<(String, IncomingMessage)> = VecDeque::new();
        let (turn_done_tx, mut turn_done_rx) = mpsc::unbounded_channel::<String>();

        loop {
            tokio::select! {
                    maybe_msg = message_rx.recv() => {
                        let Some(mut msg) = maybe_msg else { break };
                        // Turn observability: the gateway log carried tool
                        // registration, poll heartbeats and warnings, but no
                        // inbound-message event at all — a turn was invisible in
                        // the agent's own telemetry. Identifiers, booleans and
                        // counts only; `msg.content` is user text and is never
                        // logged.
                        tracing::info!(
                            user = %msg.user_id,
                            platform = %msg.platform,
                            channel = %msg.channel_id,
                            is_group = msg.is_group_chat,
                            thread_id = ?msg.thread_id,
                            media_count = msg.media_urls.len(),
                            "Gateway inbound message"
                        );
                        let platform = msg.platform.clone();
                        let channel_id = msg.channel_id.clone();
                        // Thread id captured before `msg` is moved into the turn —
                        // stream/dedup markers and session keys are per-topic.
                        let msg_thread = msg.thread_id;

                        // Per-session routing key. Hoisted above both the command
                        // branch and turn routing: `/stop` needs it to find the
                        // running turn for THIS session, and it used to be derived
                        // inside turn routing only. Deriving it in both places
                        // would let them drift, and a drifted key means `/stop`
                        // silently never matches.
                        let session_key = match (msg.is_group_chat, msg.thread_id) {
                            (true, Some(tid)) => {
                                format!("{}:{}:thread:{}", msg.platform, msg.channel_id, tid)
                            }
                            (true, None) => format!("{}:{}", msg.platform, msg.channel_id),
                            (false, Some(tid)) => format!(
                                "{}:{}:{}:thread:{}",
                                msg.platform, msg.channel_id, msg.user_id, tid
                            ),
                            (false, None) => {
                                format!("{}:{}:{}", msg.platform, msg.channel_id, msg.user_id)
                            }
                        };

                        // ── 1. Dialog interception (never queued) ────────────────
                        // `true` = handled here (no agent turn); `false` = turn.
                        let handled: anyhow::Result<bool> = async {
                            // ── 1. Admin allowlist ────────────────────────────────
                            if !admins.is_empty() && !admins.contains(&msg.user_id) {
                                tracing::warn!(
                                    "Message from unauthorized user {} on {} rejected",
                                    msg.user_id,
                                    msg.platform
                                );
                                let response = OutgoingMessage::new(
                                    &msg.channel_id,
                                    "You are not authorized to use this bot.",
                                )
                                .with_thread_id(msg.thread_id);
                                gw.send_to_platform(&platform, response).await?;
                                return Ok(true);
                            }
                            tracing::info!(
                                "Message from {} passed admin allowlist check",
                                msg.user_id
                            );

                            // ── 1.5 Command interception ──────────────────────────

                            // (Approval button taps are synthesized as `/approve` /
                            // `/deny` commands — see handle_callback_update in
                            // core. The tap also carries an `approval_callback`
                            // raw marker so we can edit the prompt message once
                            // the outcome is known, exactly like hermes'
                            // `query.edit_message_text` after a tap.)
                            let approval_tap = msg.raw.get("approval_callback").cloned();
                            let approval_was_pending = approval_tap.is_some()
                                && pending_permission_exists(&msg.channel_id);
                            let is_admin = admins.is_empty() || admins.contains(&msg.user_id);
                            match resolve_command(&msg.content) {
                            CommandMatch::Found(cmd_def, cmd_args) => {
                                // `has_args` is a boolean on purpose: the args
                                // string carries whatever the user typed after the
                                // command name (`/title <text>`), so it is
                                // classified but never logged.
                                tracing::info!(
                                    user = %msg.user_id,
                                    platform = %platform,
                                    channel = %channel_id,
                                    session_key = %session_key,
                                    command = cmd_def.name,
                                    category = cmd_def.category,
                                    admin_only = cmd_def.admin_only,
                                    is_admin,
                                    has_args = !cmd_args.is_empty(),
                                    "Gateway command dispatch"
                                );
                                if cmd_def.admin_only && !is_admin {
                                    let response = OutgoingMessage::new(
                                        &msg.channel_id,
                                        "This command is admin-only.",
                                    )
                                    .with_thread_id(msg.thread_id);
                                    gw.send_to_platform(&platform, response).await?;
                                } else if cmd_def.name == "stop"
                                    && handler_for_stop.cancel_if_queued(&turn_inflight, &session_key)
                                {
                                    // W3-2: the turn is waiting on another
                                    // chat's turn — cancel it before it starts.
                                    let response = OutgoingMessage::new(
                                        &msg.channel_id,
                                        "⏹️ **Cancelled your queued message** (it was waiting behind another chat's turn).",
                                    )
                                    .with_thread_id(msg.thread_id);
                                    gw.send_to_platform(&platform, response).await?;
                                } else if let Some(reply) = handle_runtime_command(
                                    &handler_for_stop,
                                    cmd_def.name,
                                    cmd_args,
                                    &session_key,
                                )
                                .await
                                {
                                    // RT-175: session-mutating commands execute
                                    // against THIS chat's conversation under the
                                    // turn lock — the sync handle_command arms
                                    // for them are unreachable honesty stubs
                                    // kept for non-runtime contexts (tests).
                                    let response =
                                        OutgoingMessage::new(&msg.channel_id, reply)
                                            .with_thread_id(msg.thread_id);
                                    gw.send_to_platform(&platform, response).await?;
                                } else {
                                    let ctx = CommandContext::new(
                                        Some(&gw),
                                        &app_config_clone,
                                        is_admin,
                                        &msg.user_id,
                                        &platform,
                                        &msg.channel_id,
                                    )
                                    .with_stop_flag(
                                        handler_for_stop
                                            .stop_flag_if_running(&turn_inflight, &session_key),
                                    )
                                    .with_session_pins(Some(session_pins.clone()))
                                    .with_session_key(&session_key)
                                    // RT-175: the live steer queue off the
                                    // handler's agent — the same queue
                                    // drain_steers empties at the iteration
                                    // boundary, so /steer from a channel
                                    // reaches the RUNNING turn like the TUI's.
                                    .with_steer_queue(Some(
                                        handler_for_stop.agent.steer_queue_handle(),
                                    ));
                                    if let Some(response_text) =
                                        handle_command(cmd_def.name, cmd_args, &ctx)
                                    {
                                        let response = OutgoingMessage::new(
                                            &msg.channel_id,
                                            response_text,
                                        )
                                        .with_thread_id(msg.thread_id);
                                        gw.send_to_platform(&platform, response).await?;
                                    }
                                }
                                // Reflect the tap outcome on the prompt message
                                // itself (hermes parity: "✅ Approved by X" /
                                // "⌛ expired").
                                if let Some(cb) = approval_tap {
                                    edit_approval_prompt_message(
                                        &gw,
                                        &platform,
                                        &cb,
                                        approval_was_pending,
                                    )
                                    .await;
                                }
                                return Ok(true);
                            }
                            CommandMatch::Unknown { token } => {
                                // The single most valuable line for the
                                // mistyped-command class: the token that failed to
                                // resolve, and the fact that it was REJECTED here
                                // rather than forwarded to the model. `warn!`
                                // rather than `info!` because in a log dominated by
                                // poll heartbeats an unrecognized command is an
                                // anomaly that must not drown in the noise — and
                                // before `resolve_command` returned three states,
                                // exactly this case fell through as prose, the
                                // model improvised a help reply, and nothing
                                // recorded that the command surface had been
                                // missed.
                                tracing::warn!(
                                    user = %msg.user_id,
                                    platform = %platform,
                                    channel = %channel_id,
                                    session_key = %session_key,
                                    token = %token,
                                    "Gateway command dispatch: unknown token rejected, not sent to model"
                                );
                                let response = OutgoingMessage::new(
                                    &msg.channel_id,
                                    format!(
                                        "Unknown command `/{}`.\nUse /help to list the available commands.",
                                        token
                                    ),
                                )
                                .with_thread_id(msg.thread_id);
                                gw.send_to_platform(&platform, response).await?;
                                return Ok(true);
                            }
                            // Prose: fall through to the agent turn below.
                            CommandMatch::NotACommand => {
                                // Classification companion to the inbound line:
                                // this is a TURN, not a command. A pending dialog
                                // below can still intercept it (a clarify reply),
                                // so this records the classification and not the
                                // outcome — the authoritative "a turn actually
                                // started" event is the one in
                                // `GatewayMessageHandler::handle`.
                                // ── W5 (audit O-14): the Chair's channel ──
                                // When org mode is on, a message from one of
                                // Holon's configured Chair identities is the
                                // OWNER speaking to the organization: classify
                                // it through the kernel (Holon owns the
                                // classifier), and a chair-command goes to the
                                // kernel instead of the model. Anyone else
                                // stays chat — exactly as before.
                                if app_config_clone.holon.enabled {
                                    match operant_os::chair::is_chair(
                                        &app_config_clone.holon.home,
                                        app_config_clone.holon.bin.as_deref(),
                                        &platform,
                                        &msg.user_id,
                                    )
                                    .await
                                    {
                                        Ok(true) => {
                                            let reply = operant_os::chair::dispatch_message(
                                                &app_config_clone.holon.home,
                                                app_config_clone.holon.bin.as_deref(),
                                                &msg.content,
                                            )
                                            .await;
                                            let text = match reply {
                                                Ok(t) => t,
                                                Err(e) => format!("chair command failed: {e}"),
                                            };
                                            let response = OutgoingMessage::new(&msg.channel_id, text)
                                                .with_thread_id(msg.thread_id);
                                            gw.send_to_platform(&platform, response).await?;
                                            return Ok(true);
                                        }
                                        Ok(false) => {} // not the Chair: chat as usual
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "chair identity check degraded — treating sender as chat"
                                            );
                                        }
                                    }

                                    // W5 (O-15): every channel message is an
                                    // event source. The kernel's event
                                    // triggers match against it; a match fires
                                    // its routine (payload on stdin, cooldown
                                    // enforced kernel-side). Fire-and-forget:
                                    // a trigger problem never blocks the turn.
                                    let ev_home = app_config_clone.holon.home.clone();
                                    let ev_bin = app_config_clone.holon.bin.clone();
                                    let ev_text = msg.content.clone();
                                    tokio::spawn(async move {
                                        match operant_os::events::match_and_fire(
                                            &ev_home,
                                            ev_bin.as_deref(),
                                            &ev_text,
                                        )
                                        .await
                                        {
                                            Ok(fired) if fired > 0 => tracing::info!(
                                                fired,
                                                "event triggers fired for an inbound message"
                                            ),
                                            Ok(_) => {}
                                            Err(e) => tracing::warn!(
                                                error = %e,
                                                "event intake degraded"
                                            ),
                                        }
                                    });
                                }

                                tracing::info!(
                                    user = %msg.user_id,
                                    platform = %platform,
                                    channel = %channel_id,
                                    session_key = %session_key,
                                    "Gateway command dispatch: prose, offered to the agent as a turn"
                                );
                            }
                        }

                            // ── 1.6 Clarify choice taps ──────────────────────────
                            // A `choice:` inline-button tap resolves the pending
                            // clarify() question with the selected option. The
                            // adapter synthesized this message with a raw
                            // `choice_callback` marker — resolve it here so it
                            // never reaches the agent as a stray message, and
                            // edit the prompt to show the selection (hermes
                            // `send_clarify` tap parity).
                            if let Some(cc) = msg.raw.get("choice_callback").cloned() {
                                let pending_map_clone = {
                                    let global_uq =
                                        crate::gateway_runner::PENDING_USER_QUESTIONS.get();
                                    global_uq
                                        .and_then(|g| g.lock().ok())
                                        .and_then(|mut guard| guard.take())
                                };
                                let mut selected: Option<String> = None;
                                if let Some(pending_map) = pending_map_clone {
                                    let key = format!("{platform}:{channel_id}");
                                    let idx = cc
                                        .get("idx")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as usize;
                                    if let Some((reply_tx, choices)) =
                                        pending_map.lock().await.remove(&key)
                                    {
                                        let text = choices
                                            .and_then(|c| c.get(idx).cloned())
                                            .unwrap_or_else(|| format!("option {}", idx + 1));
                                        let _ = reply_tx.send(text.clone());
                                        selected = Some(text);
                                    }
                                    if let Some(g) =
                                        crate::gateway_runner::PENDING_USER_QUESTIONS.get()
                                        && let Ok(mut guard) = g.lock()
                                    {
                                        *guard = Some(pending_map);
                                    }
                                }
                                // Edit the question message to show the selection
                                // and remove the buttons.
                                if let (Some(adapter), Some(chat), Some(mid)) = (
                                    gw.adapter_for(&platform),
                                    cc.get("chat_id").and_then(|v| v.as_str()),
                                    cc.get("message_id").and_then(|v| v.as_i64()),
                                ) {
                                    let text = match selected {
                                        Some(t) => format!("✅ Selected: {t}"),
                                        None => "⌛ This question has already been answered or timed out."
                                            .to_string(),
                                    };
                                    let _ = adapter
                                        .edit_message(
                                            chat,
                                            &mid.to_string(),
                                            &OutgoingMessage::new(chat, &text).no_markdown(),
                                        )
                                        .await;
                                }
                            return Ok(true);
                        }

                        // ── 1.7 Pending clarify() text replies ────────────────
                        // A plain-text reply while a clarify() /
                        // approval_request() dialog is pending resolves the
                        // question with the user's text. Checked here (in
                        // addition to the handler) so the reply is never
                        // queued behind the very turn that spawned the
                        // dialog — the dialog would otherwise deadlock until
                        // its 120s timeout.
                        if !msg.content.trim_start().starts_with('/') {
                            let channel_key = format!("{platform}:{channel_id}");
                            let pending_map_clone = {
                                let global_uq =
                                    crate::gateway_runner::PENDING_USER_QUESTIONS.get();
                                global_uq
                                    .and_then(|g| g.lock().ok())
                                    .and_then(|mut guard| guard.take())
                            };
                            if let Some(pending_map) = pending_map_clone {
                                let reply_tx = {
                                    let mut pending = pending_map.lock().await;
                                    pending.remove(&channel_key)
                                };
                                if let Some((reply_tx, _choices)) = reply_tx {
                                    tracing::info!(
                                        channel = %channel_key,
                                        "Routing message as user-question reply (dispatch)"
                                    );
                                    let _ = reply_tx.send(msg.content.clone());
                                    if let Some(g) =
                                        crate::gateway_runner::PENDING_USER_QUESTIONS.get()
                                        && let Ok(mut guard) = g.lock()
                                    {
                                        *guard = Some(pending_map);
                                    }
                                    // Acknowledge so the user knows the
                                    // reply landed (hermes parity).
                                    let ack = OutgoingMessage::new(
                                        &msg.channel_id,
                                        "✅ Reply received — resuming...",
                                    )
                                    .no_markdown()
                                    .with_thread_id(msg.thread_id);
                                    if let Err(e) = gw.send_to_platform(&platform, ack).await {
                                        tracing::warn!(
                                            platform = %platform,
                                            error = %e,
                                            "could not deliver an acknowledgement"
                                        );
                                    }
                                    return Ok(true);
                                }
                                if let Some(g) =
                                    crate::gateway_runner::PENDING_USER_QUESTIONS.get()
                                    && let Ok(mut guard) = g.lock()
                                {
                                    *guard = Some(pending_map);
                                }
                            }
                        }
                        Ok(false)
                    }
                    .await;
                        match handled {
                            Ok(true) => continue,
                            Err(e) => {
                                tracing::error!(
                                    "Error intercepting message on {}: {}",
                                    channel_id,
                                    redact_err(&e)
                                );
                                continue;
                            }
                            Ok(false) => {}
                        }

                        // ── 2. Turn routing (per-session FIFO queue) ─────────────
                        if turn_inflight.contains_key(&session_key) {
                            tracing::debug!(
                                session = %session_key,
                                "Queuing message behind in-flight turn"
                            );
                            turn_queue.push_back((session_key, msg));
                            continue;
                        }
                        // Insert the shared interrupt latch for this turn
                        // BEFORE spawning, so `/stop` can find the flag while
                        // the turn waits for or holds the turn lock. The
                        // RESET moved into `handle` (plan 06A): resetting
                        // here let one chat's dispatch wipe a `/stop` aimed
                        // at the RUNNING chat, and a triggered flag that is
                        // never cleared would brick the agent for the rest
                        // of the session — the reset inside the lock keeps
                        // both invariants.
                        let stop_flag = handler_for_stop.agent.interrupt_flag();
                        turn_inflight.insert(session_key.clone(), stop_flag);
                        let gw = gw_for_turns.clone();
                        let app_config_clone = app_config_for_turns.clone();
                        let stream_delivery_for_dispatch = stream_delivery_for_turns.clone();
                        let current_channel = current_channel_for_turns.clone();
                        let telegram_token = telegram_token_for_turns.clone();
                        let done_tx = turn_done_tx.clone();
                        let recap_activity_for_turn = recap_activity_for_turns.clone();
                        let session_pins_for_turns = session_pins.clone();
                        let session_channels_for_turns = session_channels.clone();
                        // The credential rotation below pushes a rotated
                        // `.env` key into the live client, so this turn needs
                        // its own handle to the handler.
                        let handler_for_turn = handler.clone();
                        tokio::spawn(async move {
                    // Wrap the turn body for error resilience — a single message
                    // processing error will not crash the whole gateway.
                    let result: anyhow::Result<()> = async {
                    // ── 2. Message enrichment ─────────────────────────────────────
                    if platform == "telegram"
                        && let Some(token) = &telegram_token
                    {
                        // Photo enrichment: download largest photo and note it
                        if let Some(desc) = enrich_photo(&msg.raw, token).await {
                            msg.content = format!("{}\n{}", desc, msg.content);
                        }
                        // Voice enrichment: download, transcribe, prepend
                        if let Some(transcript) = enrich_voice(&msg.raw, token).await {
                            msg.content = format!("{}\n{}", transcript, msg.content);
                        }
                        // Document enrichment: extract filename + caption
                        if let Some(info) = enrich_document(&msg.raw) {
                            msg.content = if msg.content.is_empty() {
                                info
                            } else {
                                format!("{}\n{}", msg.content, info)
                            };
                        }
                    }
                    tracing::info!(
                        "Message enrichment done, final content length: {}",
                        msg.content.len()
                    );

                    // ── 3. Message pipeline ───────────────────────────────────────
                    let pipeline = MessagePipeline::new();
                    let action = pipeline.process(&msg);
                    tracing::info!("Message pipeline action: {:?}", action);
                    match action {
                        PipelineAction::Block(reason) => {
                            let response = OutgoingMessage::new(&msg.channel_id, reason)
                                .with_thread_id(msg.thread_id);
                            gw.send_to_platform(&platform, response).await?;
                            return Ok(());
                        }
                        PipelineAction::Queue => {
                            // Queue not yet wired — allow through for now.
                        }
                        PipelineAction::Allow => {}
                    }

                    // ── 4. Session management ─────────────────────────────────────
                    if msg.is_group_chat {
                        match gw.get_session_store().find_or_create_shared_session(
                            &msg.platform,
                            &msg.channel_id,
                            msg_thread,
                        ) {
                            Ok(s) => tracing::info!(
                                "Shared session {} for channel {}",
                                s.session_id,
                                msg.channel_id
                            ),
                            Err(e) => {
                                tracing::warn!("Failed to get shared session: {}", redact_err(&e))
                            }
                        }
                        msg.content = format!("[{}]: {}", msg.username, msg.content);
                    } else {
                        let existing = gw.get_session_store().find_session(
                            &msg.platform,
                            &msg.user_id,
                            &msg.channel_id,
                        );
                        if let Some(s) = existing {
                            tracing::info!(
                                "Found existing session {} for {}@{}",
                                s.session_id,
                                msg.user_id,
                                msg.platform
                            );
                            let _ = gw.get_session_store().update_activity(&s.session_id);
                        } else {
                            match gw.get_session_store().create_session(
                                &msg.platform,
                                &msg.user_id,
                                &msg.channel_id,
                            ) {
                                Ok(s) => tracing::info!(
                                    "Created new session {} for {}@{}",
                                    s.session_id,
                                    msg.user_id,
                                    msg.platform
                                ),
                                Err(e) => {
                                    tracing::warn!("Failed to create session: {}", redact_err(&e))
                                }
                            }
                        }
                    }

                    // ── 5. Typing indicator ───────────────────────────────────────
                    // Send typing indicator on ALL platforms, not just Telegram.
                    // The PlatformAdapter::send_typing default is a no-op, so
                    // non-supporting platforms skip silently. (Bug #13 from
                    // iter-98 audit — Discord and Slack users saw no 'bot is
                    // typing' indicator, making long silences look like the bot
                    // was dead.)
                    let typing_platform = platform.clone();
                    let typing_thread_id = msg.thread_id;
                    let typing_handle = {
                        let gw = gw.clone();
                        let ch = channel_id.clone();
                        Some(tokio::spawn(async move {
                            // iter-814: first indicator fires IMMEDIATELY — the
                            // old loop consumed the interval's first tick, so a
                            // 4s window at every turn start showed nothing and
                            // users reported 'typing indicator not showing'.
                            if gw.send_typing(&typing_platform, &ch, typing_thread_id).is_err() {
                                return;
                            }
                            let mut interval = tokio::time::interval(std::time::Duration::from_secs(4));
                            // Skip first immediate tick, send first after 4s
                            interval.tick().await;
                            loop {
                                interval.tick().await;
                                if gw
                                    .send_typing(&typing_platform, &ch, typing_thread_id)
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }))
                    };

                    // ── 5.5 Keepalive notification ─────────────────────────────────
                    // Send "Still working..." periodically for long-running operations
                    let keepalive_handle = if platform == "telegram" {
                        let gw = gw.clone();
                        let ch = channel_id.clone();
                        let thread_id = msg.thread_id;
                        Some(tokio::spawn(async move {
                            let start = std::time::Instant::now();
                            // Wait 3 minutes before first notification (matches Python behavior)
                            tokio::time::sleep(std::time::Duration::from_secs(180)).await;
                            loop {
                                let elapsed = start.elapsed().as_secs();
                                let minutes = elapsed / 60;
                                let body =
                                    format!("\u{23F3} Still working... ({}m elapsed...)", minutes);
                                let msg = OutgoingMessage::new(&ch, &body)
                                    .no_markdown()
                                    .with_thread_id(thread_id);
                                if gw.send_to_platform("telegram", msg).await.is_err() {
                                    break;
                                }
                                // Wait 3 minutes between notifications
                                tokio::time::sleep(std::time::Duration::from_secs(180)).await;
                            }
                        }))
                    } else {
                        None
                    };

                    // ── 5.5 Set current channel for tool progress ────────────────
                    tracing::debug!(
                        "Setting current_channel to {}@{} for tool progress",
                        platform,
                        channel_id
                    );
                    *current_channel.lock().await =
                        Some((platform.clone(), channel_id.clone(), msg.thread_id));

                    // 5.5a Pin this session's delivery target for BACKGROUND
                    // events (advisor/recap/self-improvement notes — iter-812).
                    // They fire after the turn ends, when the global slot above
                    // may already point at another session's thread; the
                    // registry routes each note to its OWN topic.
                    let turn_session_id = session_pins_for_turns
                        .get(&session_key)
                        .unwrap_or_else(|| derived_gateway_session_id(&session_key));
                    session_channels_for_turns.lock().await.insert(
                        turn_session_id.clone(),
                        (platform.clone(), channel_id.clone(), msg.thread_id),
                    );

                    // ── 5.5b Cancel any pending idle recap ─────────────────
                    // User activity arrived: the recap exists to greet a
                    // returning user, never to interrupt one (omp parity —
                    // same contract as the TUI's submit_user_message).
                    recap_activity_for_turn.notify();
                    // (notify is a try_send on the shared channel — the per-turn
                    // clone keeps the dispatch loop's original valid.)

                    // ── 5.6 Per-turn .env reload for credential rotation ─────────
                    // Writing the process environment is not enough: the
                    // running client captured its key when it was built and
                    // the credential pool seeded its keys at startup, so a
                    // value that only lands in the environment is read by
                    // nobody and the rotation never took effect (RT-6c).
                    // The reloaded values are pushed into the live agent
                    // through the same seam `/connect` uses.
                    let rotated = operant_core::env_passthrough::reload_dotenv();
                    if let Some(key) = rotated
                        .api_key
                        .as_deref()
                        .filter(|k| !k.trim().is_empty())
                    {
                        tracing::info!("credential rotated from .env; updating the live client");
                        handler_for_turn.agent.set_api_key(key);
                    }
                    if let Some(base_url) = rotated
                        .base_url
                        .as_deref()
                        .filter(|u| !u.trim().is_empty())
                    {
                        // Retarget ONLY on a change. `set_endpoint` with no
                        // key empties a pool-backed client's credentials, so
                        // calling it every turn for an endpoint that did not
                        // change would leave the agent with no credential at
                        // all and the next request would fail with "no
                        // available keys".
                        let current = handler_for_turn.agent.base_url();
                        if current.as_deref() != Some(base_url) {
                            tracing::info!("endpoint rotated from .env; updating the live client");
                            // Pass the key the client is USING when the file
                            // does not carry one: a `.env` with only a base
                            // URL (the key lives in `operant.toml`) would
                            // otherwise empty the pool and the agent would
                            // have no credential at all.
                            let key = rotated
                                .api_key
                                .as_deref()
                                .filter(|k| !k.trim().is_empty())
                                .map(str::to_string)
                                .or_else(|| handler_for_turn.agent.current_api_key());
                            handler_for_turn.agent.set_endpoint(base_url, key.as_deref());
                        }
                    }

                    // ── 5.7 Turn state: mark pending ─────────────────────────────
                    save_turn_state(&channel_id, "pending");

                    // ── 5.8 Session context injection ────────────────────────────
                    let ctx =
                        build_session_context(&platform, &channel_id, msg_thread, &app_config_clone);
                    msg.content = format!("[context: {}]\n{}", ctx.trim(), msg.content);

                    // ── 6. Route message ──────────────────────────────────────
                    match gw.route_message(msg).await {
                        Ok(Some(response)) => {
                            tracing::info!(
                                "Message routed successfully, response length: {}",
                                response.content.len()
                            );
                            // If streaming already delivered the response as a
                            // progressive-edited message (AgentEvent::Content →
                            // AgentEvent::Done), skip the duplicate full send.
                            // (Kills the double-reply bug on gateway turns.)
                            // R37: the event-consumer task drains queued events
                            // (ToolStart/Usage/Content…) BEFORE Done, so its
                            // marker insert can lag the routed return by
                            // hundreds of ms on fast turns — an instant check
                            // raced it and both paths delivered (duplicate
                            // replies). Poll briefly for the marker before
                            // falling back to the full send.
                            let marker_key = (platform.clone(), channel_id.clone(), msg_thread);
                            let mut streamed = false;
                            for _ in 0..40 {
                                let hit = {
                                    let mut map = stream_delivery_for_dispatch.lock().await;
                                    map.remove(&marker_key).is_some()
                                };
                                if hit {
                                    streamed = true;
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                            }
                            if streamed {
                                tracing::info!(
                                    "Stream already delivered response to {}; skipping duplicate send",
                                    channel_id
                                );
                            } else {
                                // (iter-1132) A webhook subscription's
                                // `deliver` target names the platform the
                                // answer goes to. Without one, the answer goes
                                // back to where the message came from.
                                let send_platform = response
                                    .target_platform
                                    .clone()
                                    .unwrap_or_else(|| platform.clone());
                                if let Err(e) = gw.send_to_platform(&send_platform, response).await {
                                    tracing::error!(
                                        "Failed to send response on {} to {}: {}",
                                        send_platform,
                                        channel_id,
                                        redact_err(&e)
                                    );
                                }
                            }
                            save_turn_state_exit(
                                &channel_id,
                                "complete",
                                operant_core::gateway::lifecycle::TurnExitReason::Complete,
                                "turn returned a final response",
                            );
                        }
                        Ok(None) => {
                            // Even when the handler returns no response, clear any
                            // leftover stream marker so the next turn isn't
                            // wrongly suppressed.
                            let mut map = stream_delivery_for_dispatch.lock().await;
                            map.remove(&(platform.clone(), channel_id.clone(), msg_thread));
                            save_turn_state_exit(
                                &channel_id,
                                "complete",
                                operant_core::gateway::lifecycle::TurnExitReason::Complete,
                                "turn ended without a response",
                            );
                        }
                         Err(e) => {
                             tracing::error!(
                                 "Failed to route message on {}: {}",
                                 platform,
                                 redact_err(&e)
                             );
                             let mut map = stream_delivery_for_dispatch.lock().await;
                             map.remove(&(platform.clone(), channel_id.clone(), msg_thread));
                             // iter-691: classify before the close so the
                             // journal's exit_code column carries the class a
                             // retry/breaker can branch on (kill markers are
                             // never auth — see classify_turn_error).
                             let exit_code =
                                 operant_core::gateway::lifecycle::classify_turn_error(&e.to_string());
                             save_turn_state_exit(
                                 &channel_id,
                                 "failed",
                                 exit_code,
                                 &e.to_string(),
                             );
                         }
                    }

                    if let Some(handle) = typing_handle {
                        handle.abort();
                    }
                    if let Some(handle) = keepalive_handle {
                        handle.abort();
                    }
                    // ── Turn end: drop this session's background-event entry
                    // (iter-812). The channel binding lives exactly as long as
                    // the session has produced a turn this boot; a note firing
                    // after cleanup falls back to the current slot — identical
                    // to the pre-812 behavior for cold sessions.
                    session_channels_for_turns
                        .lock()
                        .await
                        .remove(&turn_session_id);

                    Ok(())
                }
                .await;
                if let Err(e) = result {
                    tracing::error!(
                        "Error processing message from {} on {}: {}",
                        channel_id,
                        platform,
                        redact_err(&e)
                    );
                }
                // Free the session slot so the next queued message runs.
                let _ = done_tx.send(session_key);
            });
            continue;
                    }
                    Some(done_key) = turn_done_rx.recv() => {
                        turn_inflight.remove(&done_key);
                        // Run the next queued message for this session by
                        // re-injecting it — the message arm re-runs interception
                        // and spawns the turn.
                        if let Some(pos) = turn_queue.iter().position(|(k, _)| *k == done_key)
                            && let Some((_, msg)) = turn_queue.remove(pos)
                        {
                            let _ = message_tx_for_reinject.send(msg);
                        }
                    }
                }
        }
    });

    let platform_count = gateway.adapter_count();
    *guard = Some(gateway.clone());

    // ── Spawn Cron Scheduler ─────────────────────────────────────────────
    let cron_db_path = operant_core::platform::operant_home().join("operant_cron.db");
    if let Ok(cron_db) = operant_core::cronjobs::CronDb::init(cron_db_path) {
        let cron_db = Arc::new(cron_db);
        // Row 1 (outline v5 §1): the delivery consumer settles the ledger
        // row the scheduler handed off. `cron_db` is MOVED into the
        // scheduler below (both arms of the WriteBarrier match), so the
        // sender loop takes its own clone of the same handle — without it
        // pending rows linger until the 2h reclaim replays them.
        let cron_db_consumer = Arc::clone(&cron_db);
        let (cron_tx, mut cron_rx) =
            tokio::sync::mpsc::unbounded_channel::<operant_core::cronjobs::CronDelivery>();

        // v6 outline row 5 (the queued seam, now closed): the config-driven
        // cron observer. `[observability] backend` selects it — the
        // core-local factory (plan 10 step 1; the runtime crate's factory
        // is gone with the crate). Default (`none`) yields the no-op
        // observer: the scheduler mounts an observer unconditionally and
        // an unconfigured gateway stays byte-identical.
        let cron_observer: Arc<dyn operant_core::observer::Observer> =
            operant_core::observer::observer_from_config(&app_config.observability);
        // B′ (NEXT-IMPLEMENTATION-OUTLINE §3): the scheduled-run write
        // barrier, over the same app database the session store uses (:1027
        // derives it the same way). Every completed scheduled run now leaves
        // exactly one worklog row, attributable via the stable per-job
        // session id; a barrier write failure flips the run to failure
        // instead of silently succeeding unattributed.
        //
        // Fail-open with a warn: if the org stores cannot even be opened
        // (disk error, not the normal idempotent-init path), the org's
        // heartbeat keeps running — attribution is the audit layer, and
        // killing scheduled work because the worklog is unavailable is the
        // worse failure.
        let scheduler = operant_core::cronjobs::CronScheduler::new(cron_db, cron_agent.clone())
            .with_observer(Arc::clone(&cron_observer))
            .with_seat_memory_root(
                app_config
                    .database_path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| app_config.database_path.clone()),
            )
            .with_delivery(cron_tx);
        // iter-678 (gap 1): mount the cron budget envelope — the usage meter
        // (fed by the cron agent's own event channel), the session store the
        // rollup reads, and the seat-budget resolution (overrides +
        // [bylaws].budget default). All dark-mergeable: cap 0 = ungoverned
        // keeps every run byte-identical until an operator sets a cap.
        // iter-679 (gap 5, phase 1): the DM/feed context injector over the
        // app db (worklog reads + the context_watermarks cursors).
        // Fail-open: an injector that cannot open degrades to no injection
        // (prompts byte-identical) — injection must never block a boot.
        let context_injector: Option<Arc<operant_core::org::context_injection::ContextInjector>> =
            operant_core::org::context_injection::ContextInjector::open(
                &app_config.database_path,
                app_config.context_injection.clone(),
            )
            .ok()
            .map(Arc::new);
        if context_injector.is_none() {
            tracing::warn!("context injection unavailable — seat prompts stay byte-identical");
        }
        let mut scheduler = scheduler;
        if let Some(store) = persistent_session_store.clone() {
            scheduler = scheduler.with_usage_store(store);
        }
        let mut scheduler = scheduler
            .with_usage_meter(cron_usage_meter.clone())
            .with_budgets(
                Arc::clone(&seat_budget_store),
                app_config.bylaws.budget.clone(),
            );
        if let Some(injector) = context_injector {
            scheduler = scheduler.with_context_injection(injector);
        }
        tokio::spawn(async move { scheduler.start().await });

        // Delivery receiver — sends cron results to platforms
        let gw_for_cron = gateway.clone();
        // v6 outline row 5: the sender loop is the `CronDeliveryOutcome`
        // emitter — the one cron lifecycle event core never emits (the
        // consumer-side report contract, iter-720). Same observer the
        // scheduler holds, so the quartet (Queued/Parked/Reclaimed/Outcome)
        // lands on one backend.
        let cron_observer_sender = Arc::clone(&cron_observer);
        tokio::spawn(async move {
            while let Some(delivery) = cron_rx.recv().await {
                let msg = OutgoingMessage::new(&delivery.chat_id, &delivery.content);
                let send_result = gw_for_cron.send_to_platform(&delivery.platform, msg).await;
                // Row 1 (outline v5 §1): close the ledger loop — settle
                // the row the scheduler handed off. Success is terminal
                // ('delivered'); failure records the error and leaves the
                // row pending, where the 2h reclaim replays it. The
                // attempt was already counted at handoff, so this only
                // reports the outcome.
                if let Some(delivery_id) = delivery.delivery_id {
                    // The outcome event rides alongside the ledger
                    // report — redacted text (it reaches the console
                    // observer's logs), while the ledger keeps the raw
                    // error for forensics.
                    cron_observer_sender.record_event(&ObserverEvent::CronDeliveryOutcome {
                        job_id: delivery.job_id.clone(),
                        delivery_id,
                        success: send_result.is_ok(),
                        error: send_result
                            .as_ref()
                            .err()
                            .map(|e| redact_err(e).to_string()),
                    });
                    let err_text = send_result.as_ref().err().map(|e| e.to_string());
                    match cron_db_consumer.mark_delivery_outcome(
                        delivery_id,
                        send_result.is_ok(),
                        err_text.as_deref(),
                    ) {
                        Ok(true) => {}
                        Ok(false) => tracing::debug!(
                            delivery_id,
                            "cron delivery row already settled — duplicate report ignored"
                        ),
                        Err(e) => tracing::warn!(
                            error = %e,
                            delivery_id,
                            "failed to settle cron delivery row"
                        ),
                    }
                }
                if let Err(e) = send_result {
                    tracing::warn!(
                        error = %redact_err(&e),
                        "Failed to deliver cron result"
                    );
                }
            }
        });
        tracing::info!("Cron scheduler started");
    }

    // Write PID file for cross-process status checks (the flock above is
    // the duplicate-prevention mechanism; the pid file only names the
    // process for status/stop).
    let pid_str = std::process::id().to_string();
    let pid_path = pid_file_path();
    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&pid_path, &pid_str);

    // An empty admins list means every user who finds the bot can drive the
    // agent, with this process's model credentials. `operant gateway status`
    // says so, but only that command -- `gateway run` started an open bot
    // silently, and that is the one people actually type. Say it here.
    let admin_note = if admins_configured {
        String::new()
    } else {
        // The gate compares msg.user_id with no platform scoping, so a Discord
        // or Slack id in this list will never match a Telegram user and would
        // lock Telegram out silently. Worth knowing before you fill it in.
        [
            "",
            "  WARNING: no admins configured -- ANY user who can reach this bot",
            "  can start agent turns.",
            "  Fix: set admins in ~/.operant/operant.toml under [gateway] to your",
            "  numeric id on the platform you are using. Ids are per-platform: a",
            "  Discord id will not match a Telegram user.",
        ]
        .join("\n")
    };

    Ok(format!(
        "Gateway started with {} platform(s).{admin_note}",
        platform_count
    ))
}

/// Stop the gateway
pub async fn stop_gateway() -> Result<String> {
    let mut guard = runner().lock().await;

    match guard.take() {
        Some(gateway) => {
            gateway.stop().await.context("Failed to stop gateway")?;
            let _ = std::fs::remove_file(pid_file_path());
            Ok("Gateway stopped.".to_string())
        }
        None => Ok("Gateway is not running.".to_string()),
    }
}

/// Check if gateway is running
pub async fn is_running() -> bool {
    let guard = runner().lock().await;
    match guard.as_ref() {
        Some(gw) => gw.is_running().await,
        None => false,
    }
}

/// After an approval button tap is resolved, edit the original prompt
/// message to show which choice was made and by whom, and remove the inline
/// keyboard (hermes `query.edit_message_text` parity — `"✅ Approved by X"`
/// with `reply_markup=None`). `was_pending` reflects whether a permission
/// request was actually waiting for this channel when the tap arrived; a
/// stale tap (already timed out or resolved elsewhere) gets an honest
/// "expired" note instead of a false "approved".
async fn edit_approval_prompt_message(
    gw: &Gateway,
    platform: &str,
    cb: &serde_json::Value,
    was_pending: bool,
) {
    let Some(adapter) = gw.adapter_for(platform) else {
        return;
    };
    let Some(chat_id) = cb.get("chat_id").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(message_id) = cb.get("message_id").and_then(|v| v.as_i64()) else {
        return;
    };
    let user = cb.get("user").and_then(|v| v.as_str()).unwrap_or("User");
    let action = cb.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let text = if was_pending {
        match action {
            "approve" => format!("✅ Approved by {user} (button tap)"),
            "always" => format!("✅✅ Always allowed by {user} (button tap)"),
            "deny" => format!("❌ Denied by {user} (button tap)"),
            _ => "⚠️ Unknown action".to_string(),
        }
    } else {
        "⌛ Approval expired — no command was waiting (already timed out or resolved elsewhere)."
            .to_string()
    };
    // edit_message also clears the inline keyboard (Telegram impl sends an
    // empty InlineKeyboardMarkup), so the buttons disappear on resolution.
    let _ = adapter
        .edit_message(
            chat_id,
            &message_id.to_string(),
            &OutgoingMessage::new(chat_id, &text).no_markdown(),
        )
        .await;
}

// ── Message enrichment + turn-state tracking (split 2026-10-25) ─────────────
// The Telegram media enrichment helpers live in `enrichment.rs`; the
// turn-state journal (auto-continue / interruption recovery) lives in
// `turn_state.rs` with its own tests. The glob imports below keep every
// call site in this file on its unqualified name — the split moved code,
// it did not rewrite call sites.
pub(crate) mod allocator;
mod enrichment;
mod turn_state;

use enrichment::*;
pub use turn_state::collect_interrupted_turns;
use turn_state::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod pending_permissions_tests {
    use super::*;
    use operant_core::agent::ToolPermissionRequest;
    use operant_core::agent::ToolPermissionResponse;

    fn make_request() -> ToolPermissionRequest {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        ToolPermissionRequest {
            tool_name: "code_execution".into(),
            tool_id: "t-1".into(),
            description: "Execute code".into(),
            danger_explanation: "Runs arbitrary code".into(),
            input_preview: None,
            seat_escalation: None,
            unattended: false,
            response_tx: tx,
        }
    }

    /// Serialises the tests in this module that INSTALL a store.
    ///
    /// `PENDING_PERMISSIONS` is a process-global `OnceLock`, and Rust's test
    /// harness runs tests in parallel threads within one process. Two tests
    /// that each call `store_pending_permissions` therefore race: one can
    /// overwrite the global between the other's store and its assert, and the
    /// assert then reads the wrong store and fails.
    ///
    /// This was not theoretical. `store_overwrites_early_none_init_and_taps_see_
    /// entries` failed exactly once across a long stretch of full-suite runs,
    /// while passing 3/3 in isolation and on re-run both with and without an
    /// unrelated change in flight — the signature of an order-dependent test.
    /// A full green run therefore proves nothing about this module, which is
    /// worse than a known failure: the next unrelated change that trips it will
    /// be blamed on that change.
    ///
    /// Cleanup at the end of each test is NOT sufficient on its own, and the
    /// original cleanup comment assumed it was: emptying the map leaves the
    /// global still pointing at the previous test's `Arc`, so a concurrent
    /// reader sees a valid store with the wrong contents. Only mutual
    /// exclusion fixes that.
    static PERMISSIONS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Acquire the module lock, recovering from poisoning rather than
    /// panicking: a panic in one test must not cascade into unrelated ones.
    fn permissions_test_guard() -> std::sync::MutexGuard<'static, ()> {
        PERMISSIONS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Regression: `init_pending_permissions()` seeds the static with `None`
    /// BEFORE the permission receiver builds its Arc. The old code then did
    /// `PENDING_PERMISSIONS.get_or_init(|| Mutex::new(Some(arc)))`, which is a
    /// no-op on an already-initialized OnceLock — the store stayed `None` for
    /// the whole process, so every button tap reported "Approval expired — no
    /// command was waiting" while the 60s timeout (own Arc clone) still
    /// auto-denied. `store_pending_permissions` must OVERWRITE the inner
    /// Option so the static reflects the live store.
    #[test]
    fn store_overwrites_early_none_init_and_taps_see_entries() {
        let _serialised = permissions_test_guard();
        // Reproduce the production init order: seed the globals first.
        init_pending_permissions();

        let store: Arc<Mutex<HashMap<String, ToolPermissionRequest>>> =
            Arc::new(Mutex::new(HashMap::new()));
        store
            .blocking_lock()
            .insert("ch_91838".into(), make_request());

        // The fix: overwrite, not get_or_init.
        store_pending_permissions(store);

        // A button tap / /approve lookup on the waiting channel finds it...
        assert!(
            pending_permission_exists("ch_91838"),
            "pending tap lookup must see the stored request"
        );
        // ...and an unrelated channel does not.
        assert!(!pending_permission_exists("other_channel"));

        // Cleanup: leave the global with an empty live store so other tests
        // in this binary aren't affected.
        store_pending_permissions(Arc::new(Mutex::new(HashMap::new())));
        assert!(!pending_permission_exists("ch_91838"));
    }

    #[test]
    fn exists_returns_false_when_store_never_seeded() {
        // A store that was never installed (or was seeded but never
        // overwritten by the receiver) must read as "no pending" rather
        // than panicking or looping.
        //
        // Deliberately does NOT take `PERMISSIONS_TEST_LOCK`: it installs
        // nothing, and it queries a key ("whatever") that no sibling test ever
        // stores, so a concurrent installer cannot change its answer. Taking
        // the lock here would serialise the module for no gain and would
        // obscure which tests actually mutate the global.
        assert!(!pending_permission_exists("whatever"));
    }

    #[test]
    fn response_tx_resolves_from_stored_request() {
        let _serialised = permissions_test_guard();
        // End-to-end of the resolution contract: the request stored in the
        // shared map must deliver its ToolPermissionResponse when resolved.
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let req = ToolPermissionRequest {
            tool_name: "process".into(),
            tool_id: "t-2".into(),
            description: "Run a process".into(),
            danger_explanation: "Spawns a process".into(),
            input_preview: None,
            seat_escalation: None,
            unattended: false,
            response_tx: tx,
        };
        let store: Arc<Mutex<HashMap<String, ToolPermissionRequest>>> =
            Arc::new(Mutex::new(HashMap::new()));
        store.blocking_lock().insert("dm_5297486612".into(), req);
        store_pending_permissions(store);

        assert!(pending_permission_exists("dm_5297486612"));
        let cell = PENDING_PERMISSIONS.get().unwrap();
        let guard = cell.lock().unwrap();
        let pending = guard.as_ref().unwrap();
        let removed = pending
            .blocking_lock()
            .remove("dm_5297486612")
            .expect("entry must be present");
        removed
            .response_tx
            .send(ToolPermissionResponse::AllowSession)
            .ok();
        drop(guard);

        // oneshot delivery is synchronous — no runtime needed to observe it.
        let response = rx
            .try_recv()
            .expect("response must be delivered synchronously");
        assert!(matches!(response, ToolPermissionResponse::AllowSession));

        store_pending_permissions(Arc::new(Mutex::new(HashMap::new())));
    }
}

#[cfg(test)]
mod tool_progress_tests {
    use super::*;

    fn key() -> ThreadKey {
        ("telegram".into(), "-100abc".into(), Some(91738))
    }

    #[test]
    fn consecutive_tool_starts_append_to_one_message() {
        let mut tracker = ToolProgressTracker::new();
        let k = key();

        // First tool of the group: New (send a fresh message).
        assert_eq!(
            tracker.on_tool_start_anon(&k, "⚙️ session_search: 5"),
            ProgressAction::New {
                body: "⚙️ session_search: 5".into()
            }
        );
        // After the message id is attached, the next tool appends.
        tracker.attach_message_id(&k, "msg_1".into(), "⚙️ session_search: 5");
        assert_eq!(
            tracker.on_tool_start_anon(&k, "⚙️ echo: hi"),
            ProgressAction::Edit {
                msg_id: "msg_1".into(),
                body: "⚙️ session_search: 5\n⚙️ echo: hi".into(),
            }
        );
        assert_eq!(
            tracker.on_tool_start_anon(&k, "🧠 memory_store: fact"),
            ProgressAction::Edit {
                msg_id: "msg_1".into(),
                body: "⚙️ session_search: 5\n⚙️ echo: hi\n🧠 memory_store: fact".into(),
            }
        );
    }

    #[test]
    fn text_boundary_closes_group_and_next_tool_starts_fresh() {
        let mut tracker = ToolProgressTracker::new();
        let k = key();

        tracker.on_tool_start_anon(&k, "⚙️ tool_a");
        tracker.attach_message_id(&k, "msg_1".into(), "⚙️ tool_a");
        tracker.on_tool_start_anon(&k, "⚙️ tool_b");

        // A mid-stream text response arrives → the group is closed. The
        // already-sent message stays in the chat; the next tool call must
        // start a NEW message (hermes parity: separate chronological
        // messages around the text).
        tracker.close_group(&k);
        assert_eq!(
            tracker.on_tool_start_anon(&k, "⚙️ tool_c"),
            ProgressAction::New {
                body: "⚙️ tool_c".into()
            },
            "tool call after text must start a fresh message, not append"
        );
    }

    #[test]
    fn close_group_is_idempotent() {
        let mut tracker = ToolProgressTracker::new();
        let k = key();

        tracker.close_group(&k);
        tracker.close_group(&k);
        assert_eq!(
            tracker.on_tool_start_anon(&k, "⚙️ tool_a"),
            ProgressAction::New {
                body: "⚙️ tool_a".into()
            }
        );
    }

    #[test]
    fn seal_split_point_prefers_newline_and_is_char_safe() {
        // Newline inside budget → split there.
        let text = format!("{}\n{}", "a".repeat(2000), "b".repeat(2000));
        assert_eq!(seal_split_point(&text, 3500), 2000);
        // No newline → hard cut at a char boundary (multibyte-safe).
        // "a"*3000 + "é"*1000: the 3501st char is the 501st 'é' at
        // byte 3000 + 500*2 = 4000.
        let text = format!("{}{}", "a".repeat(3000), "é".repeat(1000));
        assert_eq!(seal_split_point(&text, 3500), 4000);
        assert!(text.is_char_boundary(seal_split_point(&text, 3500)));
        // Newline too early (< half budget) → ignored, hard cut.
        let text = format!("x\n{}", "y".repeat(4000));
        assert_eq!(seal_split_point(&text, 3500), 3500);
    }

    #[test]
    fn repeated_identical_lines_collapse_to_x_counter() {
        // Hermes `__dedup__` parity: identical consecutive tool lines
        // collapse into a ×N counter on the last line instead of piling
        // duplicate rows (degenerate-loop spam containment).
        let mut tracker = ToolProgressTracker::new();
        let k = key();

        let _ = tracker.on_tool_start_anon(&k, "⏰ todo");
        tracker.attach_message_id(&k, "m".into(), "⏰ todo");
        for _ in 0..4 {
            assert!(matches!(
                tracker.on_tool_start_anon(&k, "⏰ todo"),
                ProgressAction::Edit { .. }
            ));
        }
        if let ProgressAction::Edit { msg_id, body } = tracker.on_tool_start_anon(&k, "⏰ todo") {
            assert_eq!(msg_id, "m");
            assert_eq!(body, "⏰ todo (×6)");
        } else {
            panic!("expected Edit");
        }
        // A different tool breaks the streak and renders normally.
        if let ProgressAction::Edit { body, .. } = tracker.on_tool_start_anon(&k, "🔧 debug_env")
        {
            assert!(body.ends_with("\n🔧 debug_env"));
        } else {
            panic!("expected Edit");
        }
    }

    #[test]
    fn status_bubble_caps_at_max_lines() {
        let mut tracker = ToolProgressTracker::new();
        let k = key();

        tracker.on_tool_start_anon(&k, "t0");
        tracker.attach_message_id(&k, "m".into(), "t0");
        for i in 1..(MAX_STATUS_LINES + 5) {
            tracker.on_tool_start_anon(&k, &format!("t{}", i));
        }
        if let ProgressAction::Edit { body, .. } = tracker.on_tool_start_anon(&k, "final") {
            assert!(body.starts_with("… +6 earlier\n"));
            assert!(body.lines().count() == MAX_STATUS_LINES + 1);
            assert!(body.ends_with("\nfinal"));
        } else {
            panic!("expected Edit");
        }
    }

    #[test]
    fn failed_first_send_leaves_group_untracked() {
        let mut tracker = ToolProgressTracker::new();
        let k = key();

        // Simulate a failed first send: on_tool_start returned New but
        // attach_message_id was never called. The next tool start must
        // retry a fresh send rather than editing a phantom message.
        assert_eq!(
            tracker.on_tool_start_anon(&k, "⚙️ tool_a"),
            ProgressAction::New {
                body: "⚙️ tool_a".into()
            }
        );
        assert_eq!(
            tracker.on_tool_start_anon(&k, "⚙️ tool_b"),
            ProgressAction::New {
                body: "⚙️ tool_b".into()
            },
            "group must stay untracked until attach_message_id succeeds"
        );
    }

    #[test]
    fn different_threads_are_isolated() {
        let mut tracker = ToolProgressTracker::new();
        let k1 = key();
        let k2: ThreadKey = ("telegram".into(), "-100abc".into(), Some(91739));

        tracker.on_tool_start_anon(&k1, "⚙️ a");
        tracker.attach_message_id(&k1, "m1".into(), "⚙️ a");
        // Topic 2 starts its own group.
        assert_eq!(
            tracker.on_tool_start_anon(&k2, "⚙️ b"),
            ProgressAction::New {
                body: "⚙️ b".into()
            }
        );
        // Closing topic 1 does not affect topic 2.
        tracker.close_group(&k1);
        tracker.attach_message_id(&k2, "m2".into(), "⚙️ b");
        assert_eq!(
            tracker.on_tool_start_anon(&k2, "⚙️ c"),
            ProgressAction::Edit {
                msg_id: "m2".into(),
                body: "⚙️ b\n⚙️ c".into(),
            }
        );
    }

    // ── the gateway shows WHAT each tool call is doing ────────────────────
    //
    // A channel's progress line used to carry only the technical argument
    // (`⚙️ read: src/main.rs`), so a recipient following the run had no idea
    // why the call was made. The model's `intent` (see
    // `operant_core::schema::TOOL_INTENT_KEY`) is the description, and it
    // leads the line.

    #[test]
    fn the_preview_line_leads_with_the_models_intent() {
        let args = r#"{"file_path":"src/parser.rs","intent":"Locate the entry point"}"#;
        let line = tool_preview_line("read", args);
        assert!(
            line.contains("read · Locate the entry point"),
            "the intent leads the line: {line}"
        );
        assert!(
            line.contains("src/parser.rs"),
            "the technical argument still rides along: {line}"
        );
        assert!(line.starts_with("📖"), "the tool's emoji stays: {line}");
    }

    /// A call that carries no intent prints exactly what it did before: the
    /// tool name plus its technical argument.
    #[test]
    fn the_preview_line_without_an_intent_is_unchanged() {
        let line = tool_preview_line("read", r#"{"file_path":"src/main.rs"}"#);
        assert!(line.starts_with("\u{1F4D6}"), "{line}");
        assert!(line.contains("read: src/main.rs"), "{line}");
        assert!(!line.contains(" · "), "no separator when no intent: {line}");
    }

    /// An empty or whitespace intent is not an intent: it must not produce a
    /// dangling separator.
    #[test]
    fn a_blank_intent_is_treated_as_no_intent() {
        let line = tool_preview_line("read", r#"{"file_path":"a.rs","intent":"   "}"#);
        assert!(line.contains("read: a.rs"), "{line}");
        assert!(!line.contains(" · "), "{line}");
    }

    /// A long intent or argument is capped, so one verbose call cannot push a
    /// channel message over its limit.
    #[test]
    fn a_long_intent_or_argument_is_capped() {
        let long = "x".repeat(400);
        let args = format!(r#"{{"file_path":"{}","intent":"{}"}}"#, long, long);
        let line = tool_preview_line("read", &args);
        assert!(
            line.chars().count() < 300,
            "line is capped: {} chars",
            line.chars().count()
        );
        assert!(line.contains("..."), "an ellipsis marks the cut: {line}");
    }

    /// An intent identical to the technical argument is not repeated: the
    /// line must not read `bash · cargo test: cargo test`.
    #[test]
    fn an_intent_equal_to_the_argument_is_not_repeated() {
        let args = r#"{"command":"cargo test","intent":"cargo test"}"#;
        let line = tool_preview_line("bash", args);
        assert_eq!(
            line.matches("cargo test").count(),
            1,
            "the value appears once: {line}"
        );
    }
}

#[cfg(test)]
mod turn_retention_tests {
    use super::retains_turn_for;

    #[test]
    fn only_a_named_admin_has_its_turns_retained() {
        // KB-6: an open bot (empty admin list) admits anyone, so it retains nothing.
        assert!(retains_turn_for(&["42".to_string()], "42"));
        assert!(!retains_turn_for(&["42".to_string()], "7"));
        assert!(!retains_turn_for(&[], "42"));
    }
}

#[cfg(test)]
mod unbound_scope_tests {
    use super::unbound_scope_for;
    use operant_core::memory_wire::{MemoryScope, bank_for};

    #[test]
    fn only_a_named_admin_reaches_the_personal_bank() {
        // KB-8: an unbound stranger recalls and retains in a bank of their own,
        // and two strangers never share one.
        let admins = vec!["owner".to_string()];
        let personal = bank_for(&MemoryScope::Personal);

        assert_eq!(
            bank_for(&unbound_scope_for(&admins, "telegram", "owner")),
            personal
        );
        let stranger = bank_for(&unbound_scope_for(&admins, "telegram", "stranger"));
        assert_ne!(stranger, personal);
        assert_ne!(
            stranger,
            bank_for(&unbound_scope_for(&admins, "telegram", "other"))
        );
    }
}
