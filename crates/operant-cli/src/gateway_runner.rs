//! Gateway Runner
//!
//! Manages a singleton Gateway instance that persists across CLI commands
//! within a single process. Provides start/stop/restart lifecycle management
//! backed by configuration and the `operant_core::gateway` module.

use anyhow::{Context, Result};
use base64::Engine;
use operant_core::agent::{AgentEvent, OperantAgent};
use operant_core::config::AppConfig;
use operant_core::context_management::estimate_tokens;
use operant_core::gateway::{
    DiscordAdapter, DrainOutcome, EmailAdapter, Gateway, GatewayConfig, IncomingMessage,
    MessageHandler, OutgoingMessage, PlatformAdapter, SlackAdapter, SmsAdapter, TelegramAdapter,
    WebhookAdapter, WhatsAppAdapter,
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

/// What a permission ask with NO channel to prompt on should get.
///
/// (iter-2366) Extracted so the decision has a test. This is the one arm that
/// decides whether an operator is asked before one of their tools runs, and it
/// had no test at all.
///
/// In this process the arm is reached by exactly one kind of turn: an ATTENDED
/// turn with no channel — a turn started over the session socket. A channel turn
/// always has `current_channel` set by the dispatch loop before it runs, and the
/// cron agent sets `unattended`, so it is decided by
/// [`unattended_no_channel_verdict`] instead.
///
/// An unattended ask keeps its posture consult (an operator who set `yolo`
/// accepted auto-approval for cron). An ATTENDED ask is DENIED: the old code
/// sent `AllowSession`, which meant a tool the same operator would be prompted
/// for on Telegram ran unapproved the moment they used `operant chat --attach`
/// — attaching silently removed the human veto.
fn no_channel_verdict(
    unattended: bool,
    posture_raw: &str,
) -> operant_core::agent::ToolPermissionResponse {
    use operant_core::agent::ToolPermissionResponse;
    if unattended {
        // The posture consult above already ran and did not object.
        return ToolPermissionResponse::AllowSession;
    }
    let _ = posture_raw;
    ToolPermissionResponse::Deny
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
    // The todo tool's payload is `{ todos: [ {id, content, status}, ... ],
    // merge, sessionId }` — every key holds an array or a bool, so
    // `extract_tool_arg`'s key lists have nothing to pull. Its FALLBACK
    // ("show first string value from any key") then grabs whatever string is
    // present, which on this payload is the model's own `intent` — a value
    // already printed as the intent half of the line. So for the todo tool the
    // summary must WIN rather than fill a gap: the counts are the only
    // technical content the payload actually carries, and the item text is
    // `/todos`' job (it reads the same store).
    let technical = todo_progress_summary(name, args).or(technical);
    match (intent, technical) {
        (Some(intent), Some(arg)) if arg != intent => {
            format!("{} {} · {}: {}", emoji, name, intent, arg)
        }
        (Some(intent), _) => format!("{} {} · {}", emoji, name, intent),
        (None, Some(arg)) => format!("{} {}: {}", emoji, name, arg),
        (None, None) => format!("{} {}...", emoji, name),
    }
}

/// A one-glance count summary for the todo tool's progress line.
///
/// The todo tool's payload is `{ todos: [{id, content, status}, ...], merge,
/// sessionId }`. `extract_tool_arg` exists to pull a STRING argument out of a
/// payload, and every key here holds an array or a bool, so it can never
/// surface anything — which is why a five-item plan printed as a bare
/// `⏰ todo...`. Counts by status are the honest preview: the item text is
/// `/todos`' job, and it reads the same store.
///
/// Returns `None` when there is nothing to summarise (no `todos` array, or an
/// empty one), so the caller keeps its `⏰ todo...` fallback rather than
/// printing `0 tasks` for a call that carried no list at all.
///
/// `name` gates the whole thing: a `todos` key on some other tool's payload
/// must not be read as a plan, and the check is cheaper than the parse it
/// guards.
fn todo_progress_summary(name: &str, args: &str) -> Option<String> {
    if name != "todo" {
        return None;
    }
    let json = serde_json::from_str::<serde_json::Value>(args).ok()?;
    let items = json.get("todos")?.as_array()?;
    if items.is_empty() {
        return None;
    }
    // `status` is normalised server-side (an unknown value becomes `pending`,
    // see todo_tool.rs), so count the four states the tool actually emits
    // and bucket anything else with `pending` rather than inventing a fifth.
    let mut done = 0usize;
    let mut active = 0usize;
    let mut cancelled = 0usize;
    for item in items {
        match item
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("pending")
        {
            "completed" => done += 1,
            "in_progress" => active += 1,
            "cancelled" => cancelled += 1,
            _ => {}
        }
    }
    let pending = items.len() - done - active - cancelled;
    let mut parts = Vec::with_capacity(4);
    if done > 0 {
        parts.push(format!("{} done", done));
    }
    if active > 0 {
        parts.push(format!("{} active", active));
    }
    if pending > 0 {
        parts.push(format!("{} to do", pending));
    }
    if cancelled > 0 {
        parts.push(format!("{} dropped", cancelled));
    }
    Some(format!("{} tasks · {}", items.len(), parts.join(" · ")))
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
    use super::{no_channel_verdict, unattended_no_channel_verdict};

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

    /// iter-2366: an ATTENDED ask with nobody to ask is DENIED.
    ///
    /// This is the arm a turn started over the session socket reaches. It used
    /// to send `AllowSession`, so a tool the same operator would be prompted for
    /// on a channel ran unapproved the moment they used `operant chat --attach`:
    /// attaching silently removed the human veto. The decision now lives in a
    /// named function with this test, because it is the one place that decides
    /// whether an operator is asked before one of their tools runs.
    #[test]
    fn an_attended_ask_with_no_channel_is_denied_not_auto_approved() {
        use operant_core::agent::ToolPermissionResponse;
        assert!(
            matches!(
                no_channel_verdict(false, "yolo"),
                ToolPermissionResponse::Deny
            ),
            "an attended ask with no channel must be DENIED, whatever the posture"
        );
        assert!(
            matches!(
                no_channel_verdict(false, "standard"),
                ToolPermissionResponse::Deny
            ),
            "the posture must not reopen an attended ask"
        );
    }

    /// An UNATTENDED ask keeps the posture consult: `yolo` is the shipped
    /// default and the operator who chose it accepted auto-approval for cron.
    #[test]
    fn an_unattended_ask_keeps_its_posture_behaviour() {
        use operant_core::agent::ToolPermissionResponse;
        assert!(
            matches!(
                no_channel_verdict(true, "yolo"),
                ToolPermissionResponse::AllowSession
            ),
            "yolo must still auto-approve an UNATTENDED ask"
        );
        // The receiver consults `unattended_no_channel_verdict` first and only
        // reaches this function when it did not object, so the governing modes
        // never arrive here. Asserted so the seam cannot be inverted silently.
        assert!(
            unattended_no_channel_verdict("standard").is_some(),
            "a governing posture must deny the unattended ask BEFORE this function"
        );
        let _ = ToolPermissionResponse::Deny;
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

/// (A3-gateway H-3) The per-event target resolution the progress/permission
/// receivers use.
///
/// Split out from the two call sites so the decision is testable. The callers
/// look up `pinned` for the event's own session and read `slot` from the
/// process-global cell; this decides between them. A turn that names its own
/// session can therefore never be routed to whichever chat most recently
/// wrote the global cell -- which is exactly what happened when both readers
/// read the cell alone.
fn resolve_turn_channel(
    event_session: Option<&str>,
    pinned: Option<CurrentChannel>,
    slot: Option<CurrentChannel>,
) -> Option<CurrentChannel> {
    match event_session {
        Some(id) if !id.is_empty() => resolve_background_target(id, pinned, slot),
        _ => slot,
    }
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
/// How many skills one picker page offers.
const SKILL_PICKER_PAGE: usize = 8;

/// The vendor skill roots — the installed content integrations plus the
/// plugin roots captured at start: ONE list, the same one the SkillManager
/// and the in-loop tools read.
fn vendor_skill_roots_for(handler: &GatewayMessageHandler) -> Vec<std::path::PathBuf> {
    #[cfg(feature = "plugins-wasm")]
    let host = handler.wasm_plugin_skill_roots.clone();
    #[cfg(not(feature = "plugins-wasm"))]
    let host: Vec<std::path::PathBuf> = Vec::new();
    operant_core::skills::vendor_skill_roots(&host)
}

/// Every skill root this gateway may offer: the vendor roots PLUS the
/// operator's `[skills] extra_roots`.
///
/// ONE helper, because the defect it replaces was a drift: three call sites
/// (the `$` expansion, the browse, the invoke) extended the vendor list with
/// `extra_roots` and one -- the bare `/skills` picker -- passed the vendor list
/// alone. Over a channel, browse therefore omitted every configured extra root
/// while search and invoke included them, so a skill could be listed and
/// rejected in the same session.
fn skill_roots_for(handler: &GatewayMessageHandler) -> Vec<std::path::PathBuf> {
    extend_with_extra_roots(vendor_skill_roots_for(handler), &handler.skills_extra_roots)
}

/// Append `extra` to `roots`, de-duplicating by path.
///
/// Extracted so the merge has ONE testable definition. The defect this replaces
/// was not in the merge logic — it was that one call site did not call it at
/// all.
fn extend_with_extra_roots(
    mut roots: Vec<std::path::PathBuf>,
    extra: &[std::path::PathBuf],
) -> Vec<std::path::PathBuf> {
    for root in extra {
        if !roots.contains(root) {
            roots.push(root.clone());
        }
    }
    roots
}

/// Offer the skills matching `query` to a chat as a button plane.
///
/// The pending list lives in `PENDING_SKILL_PICKERS`, keyed by chat: the
/// buttons are index-based, so the list IS the index space and a re-render
/// keeps every earlier button honest.
///
/// Returns false when there is nothing to offer or the platform cannot
/// render a picker — the caller then falls back to the plain-text listing.
#[expect(
    clippy::too_many_arguments,
    reason = "the picker's destination (platform, chat, thread, the message it \
              replaces), its pool (skills dir, vendor roots), its query and its \
              page — one message, one argument each"
)]
async fn offer_skill_picker(
    gw: &Gateway,
    platform: &str,
    channel_id: &str,
    thread_id: Option<i64>,
    message_id: Option<&str>,
    skills_dir: &std::path::Path,
    vendor_roots: &[std::path::PathBuf],
    query: &str,
    page: usize,
) -> bool {
    let entries = if query.trim().is_empty() {
        operant_core::skill_mentions::loadable_skills(skills_dir, vendor_roots)
    } else {
        operant_core::skill_mentions::search_skills(skills_dir, vendor_roots, query, 100)
    };
    if entries.is_empty() {
        return false;
    }
    let Some(adapter) = gw.adapter_for(platform) else {
        return false;
    };
    let title = if query.trim().is_empty() {
        format!(
            "🧩 Skills — pick one to run it ({} available)",
            entries.len()
        )
    } else {
        format!("🧩 Skills matching `{query}` ({} found)", entries.len())
    };
    let Ok(sent_id) = adapter
        .send_picker(
            channel_id,
            thread_id,
            message_id,
            &title,
            &entries,
            page,
            SKILL_PICKER_PAGE,
        )
        .await
    else {
        // The picker never landed: there are no buttons to answer, so there
        // is nothing to remember either.
        return false;
    };
    // The pending list is stored AFTER the send: the message id is what a tap
    // is matched against, and a picker that never landed has no id.
    if let Ok(mut guard) = PENDING_SKILL_PICKERS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
    {
        guard.insert(
            format!("{platform}:{channel_id}"),
            PendingSkillPicker {
                entries,
                query: query.to_string(),
                message_id: sent_id,
            },
        );
    }
    true
}

/// What a skill-picker tap asks for, before any I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SkillTapAction {
    /// Run the skill with this name.
    Invoke(String),
    /// Re-render the picker at this page.
    Page(usize),
    /// Dismiss the picker.
    Cancel,
}

/// Read the pending picker and decide what a tap asks for.
///
/// Pure apart from the pending-list read, so the decision is testable without
/// a live platform: the caller does the I/O. `None` means the tap names
/// nothing — the picker expired — so the caller should offer a fresh one.
fn skill_tap_action(marker: &serde_json::Value, key: &str) -> Option<SkillTapAction> {
    let action = marker.get("action")?;
    let kind = action.get("kind").and_then(|k| k.as_str())?;
    let tapped_message = marker
        .get("message_id")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string());
    let mut pending = PENDING_SKILL_PICKERS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .ok()?;
    let entry = pending.get(key).cloned();
    // A tap belongs to the picker message it was rendered into. A chat can
    // have TWO pickers open (the operator ran `$` again while the first was
    // still on screen), and the callback carries an INDEX — which means
    // nothing without the list it was drawn from. A tap from any other
    // picker is STALE: it must not invoke the wrong skill, and it must not
    // consume the picker that IS on offer.
    if let Some(entry) = &entry
        && let (Some(pending_mid), Some(tapped_mid)) = (&entry.message_id, &tapped_message)
        && pending_mid != tapped_mid
    {
        return None;
    }
    match kind {
        "invoke" => {
            let idx = action.get("idx").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            let name = entry?.entries.get(idx).map(|(name, _)| name.clone())?;
            pending.remove(key);
            Some(SkillTapAction::Invoke(name))
        }
        "page" => Some(SkillTapAction::Page(
            action.get("page").and_then(|p| p.as_u64()).unwrap_or(0) as usize,
        )),
        "cancel" => {
            pending.remove(key);
            Some(SkillTapAction::Cancel)
        }
        _ => None,
    }
}

/// What an inbound message's `$skill` surface asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SkillSurface {
    /// The message names a skill: this is the invocation message to send.
    Invoke(String),
    /// The operator asked for the picker: a bare `$`, or a name that
    /// resolves to nothing. The string is the query.
    Picker(String),
    /// The message carries no `$` surface at all.
    None,
}

/// Read a message's `$skill` surface.
///
/// The decision is pure enough to pin: a message that names a skill expands
/// into the invocation message the agent follows (the SAME message
/// `build_skill_invocation_message_in` builds for `/skill <name>`), a bare
/// `$` asks for the picker, and a name that resolves to nothing asks for the
/// picker with that name as its query.
fn skill_surface_for(
    handler: &GatewayMessageHandler,
    session_key: &str,
    content: &str,
) -> SkillSurface {
    let mentions = operant_core::skill_mentions::find_skill_mentions(content);
    if mentions.is_empty() {
        // A bare `$` is the "show me the skills" gesture.
        return if content.trim() == "$" {
            SkillSurface::Picker(String::new())
        } else {
            SkillSurface::None
        };
    }
    // ONE list, via the helper, so `$skill` over a channel resolves the SAME
    // set the TUI's `$` popup offers and the `/skills` browse lists.
    let roots = skill_roots_for(handler);
    let session = derived_gateway_session_id(session_key);
    let expansion = operant_core::skill_mentions::expand_skill_mentions_in(
        &handler.skills_root_dir,
        content,
        Some(session.as_str()),
        &roots,
    );
    match expansion.message {
        Some(text) => SkillSurface::Invoke(text),
        // An unknown name is better answered by offering the skills that DO
        // exist than by sending `$typo` to the agent.
        None => SkillSurface::Picker(
            expansion
                .unresolved
                .first()
                .cloned()
                .unwrap_or_else(|| mentions[0].name.clone()),
        ),
    }
}

/// The pending picker for a chat, if one is open.
fn pending_skill_picker(key: &str) -> Option<PendingSkillPicker> {
    PENDING_SKILL_PICKERS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|guard| guard.get(key).cloned())
}

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
            // Materialize the tools into EVERY slot's LIVE registry (RT-508:
            // one manager, several agents — the MCP CHILDREN are shared, the
            // registries are not, so each one needs the tools).
            let mut tools = 0usize;
            for slot in handler.slots.iter() {
                tools += manager.sync_tools_to_registry(&slot.agent.registry()).await;
            }
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
            msg.push_str(&format!(" {tools} tool(s) synced to the live registries."));
            if !failures.is_empty() {
                msg.push_str(&format!(" Failed: {}", failures.join("; ")));
            }
            Some(msg)
        }

        // ── /reload-skills — rescan + runtime swap (set_skill_manager) ──
        "reload-skills" => {
            // The off switch is sticky here too: the operator set
            // `[skills] autoload = false` to keep skills off, and a reload
            // is a PEEK at the roots, not a reason to resurrect the
            // surface. This is the only runtime path in the gateway that
            // re-adds skills to an agent.
            if !handler.skills_autoload {
                Some(
                    "🔇 `[skills] autoload = false` — the skill surface stays off, and /reload-skills does not                      resurrect it. Set autoload = true and restart to re-enable."
                        .to_string(),
                )
            } else {
                let skills_dir = handler.skills_root_dir.clone();
                // (RT-508) EVERY slot gets its own SkillManager: the field is
                // owned by each agent, and a manager swapped into one slot
                // would leave the other slots on the stale list.
                let build = || {
                    let mut mgr = operant_core::skills::SkillManager::new(skills_dir.clone());
                    // The SAME roots skill_roots_for offers every other gateway
                    // surface: the vendor roots (installed integrations + the
                    // handler's captured plugin roots) AND the operator's
                    // `[skills] extra_roots`. The manual two-step this used to
                    // perform is now the ONE helper, so a rescan that dropped an
                    // extra root — making /skill ponytail-review fail right after
                    // a reload — is no longer possible.
                    for root in skill_roots_for(handler) {
                        mgr.add_extra_root(Some(root));
                    }
                    mgr.load_all().map(|loaded| (mgr, loaded.len()))
                };
                match build() {
                    Ok((first, count)) => {
                        let slots = handler.slots.iter().count();
                        handler.slots.slots[0].agent.set_skill_manager(Some(first));
                        let mut swapped = 1usize;
                        for slot in &handler.slots.slots[1..] {
                            if let Ok((mgr, _)) = build() {
                                slot.agent.set_skill_manager(Some(mgr));
                                swapped += 1;
                            }
                        }
                        tracing::info!(
                            skills = count,
                            slots = swapped,
                            dir = %skills_dir.display(),
                            "Gateway /reload-skills: rescanned and swapped the live skill managers"
                        );
                        Some(format!(
                            "🔄 Skills reloaded: {count} skill(s) from `{}` on {swapped} of {slots} agent slot(s) — the next message sees the fresh list.",
                            skills_dir.display()
                        ))
                    }
                    Err(e) => Some(format!(
                        "🔄 Skill reload failed — the previous list stays active. Error: {e}"
                    )),
                }
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
                let held = handler.slots.slot_for(session_key).agent.reasoning_effort();
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

        // ── /context — the status half of the TUI's context view ─────────
        // RT-179: `/context`'s mutation half already exists as `/compress`;
        // this is the read-only half — how full the session's context is.
        //
        // READ-ONLY, and reported from the SAME figures the TUI's footer
        // divides by, for the iter-1207 reason spelled out there: two
        // surfaces quoting two denominators is a lie about how close the
        // window is to full. So the window is the AGENT's own
        // (`OperantAgent::context_window` — the configured one, lowered by a
        // learned overflow limit), which is what the core budgets against and
        // what the TUI's footer now reads, and the token count uses the same
        // `estimate_total_tokens` estimator the context manager itself uses.
        //
        // It takes the session scope (turn lock + this chat's conversation)
        // rather than reading a cell, because the thing being reported is
        // the conversation's own store — the same reason /undo and
        // /compress do.
        "context" => {
            let reply = handler
                .run_session_scoped_command(session_key, |agent, session_id| async move {
                    let messages = agent.conversation().await;
                    let estimated =
                        operant_core::context_management::estimate_total_tokens(&messages);
                    let model = agent.model();
                    // (iter-1779) The agent's OWN window — the configured one,
                    // lowered by a limit a prior overflow taught us — is the
                    // denominator the core budgets against. The gateway used
                    // to resolve its own (models.dev cache + per-provider
                    // fallbacks), and the TUI's footer used the configured
                    // value: two surfaces, two denominators, the iter-1207 lie
                    // again. `OperantAgent::context_window` is public for
                    // exactly this ("CLI and TUI surfaces must report the same
                    // denominator the agent uses").
                    let window = agent.context_window();
                    tracing::debug!(
                        session_id = %session_id,
                        messages = messages.len(),
                        estimated,
                        window,
                        "Gateway /context: reported the session's context occupancy"
                    );
                    format!(
                        "📐 *Context for this session:*\n\
                         • Messages: {}\n\
                         • Estimated tokens: ~{} (the same estimator the context manager uses)\n\
                         • Window: {} for `{}`\n\
                         {}\n\
                         Shrink it with `/compress` — it folds older completed exchanges and keeps the recent tail.",
                        messages.len(),
                        estimated,
                        window,
                        model,
                        context_headroom_line(estimated, window)
                    )
                })
                .await;
            Some(reply)
        }

        // ── /export [path] — the TUI's /transcript twin ───────────────
        // RT-179 plan item 3.4. The channel twin of the TUI's
        // `/transcript` (tui/app/commands.rs:786), which writes the same
        // markdown to `~/.operant/transcripts/` and opens it in the
        // external editor. A channel has no editor to open, so the reply
        // names the file path instead — the artifact is the deliverable.
        //
        // It goes through `run_session_scoped_command` for the SAME reason
        // /compress does: the conversation being exported is THIS chat's
        // loaded transcript, and reading the agent's cell without the
        // scope would export whichever conversation happened to be loaded
        // (the cross-session corruption the turn lock prevents).
        //
        // Markdown shape is `export_dialog::export_as_markdown`'s, kept in
        // ONE place by reusing the function rather than re-deriving a
        // second format that would drift from the TUI's.
        "export" => {
            let requested = args.trim().to_string();
            let reply = handler
                .run_session_scoped_command(session_key, move |agent, session_id| async move {
                    let messages = agent.conversation().await;
                    if messages.is_empty() {
                        return "📄 Nothing to export — this conversation has no messages yet."
                            .to_string();
                    }
                    // The TUI's own formatter, so both surfaces produce one
                    // format. Its Message carries only (role, content) with
                    // content as Text or Blocks — core's Message carries the
                    // wire fields — so the conversion is explicit and
                    // mechanical: text content maps straight through, and
                    // anything with tool calls or reasoning becomes a Text
                    // carrying what a reader of a transcript needs (the
                    // reasoning is kept, the tool plumbing is not: a
                    // transcript is prose, and `get_all_text` on a Blocks
                    // message would re-derive the same choice anyway).
                    use crate::tui::adapter_types::types::MessageContent;
                    let adapted: Vec<crate::tui::adapter_types::types::Message> = messages
                        .iter()
                        .map(|m| {
                            // A Tool message has no TUI role, and the honest
                            // choice is not to drop it: a transcript that
                            // silently omits what the tools did is a
                            // misleading artifact. It is kept as a System
                            // block naming the tool, so the reader sees the
                            // call happened and can go to the raw session
                            // for the payload.
                            let role = match m.role {
                                operant_core::client::Role::User => {
                                crate::tui::adapter_types::types::Role::User
                            }
                            operant_core::client::Role::Assistant => {
                                crate::tui::adapter_types::types::Role::Assistant
                            }
                            operant_core::client::Role::System
                            | operant_core::client::Role::Tool => {
                                crate::tui::adapter_types::types::Role::System
                            }
                            };
                            let mut text = m.content.clone();
                            if m.role == operant_core::client::Role::Tool {
                                let tool_name = m.name.as_deref().unwrap_or("tool");
                                text = format!("_[tool result: {tool_name}]_\n\n{text}");
                            }
                            if let Some(reasoning) = &m.reasoning
                                && !reasoning.trim().is_empty()
                            {
                                text.push_str("\n\n> ");
                                text.push_str(reasoning.trim());
                            }
                            crate::tui::adapter_types::types::Message {
                                role,
                                content: MessageContent::Text(text),
                            }
                        })
                        .collect();
                    let markdown =
                        crate::tui::export_dialog::export_as_markdown(&adapted, None);

                    // The path: `/export <path>` writes exactly there (an
                    // operator who wants a specific file names it); a bare
                    // `/export` uses the TUI's timestamped default in the
                    // same directory.
                    let dir = operant_core::platform::operant_home().join("transcripts");
                    let path = if requested.is_empty() {
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or_default();
                        dir.join(format!("transcript-{stamp}.md"))
                    } else {
                        std::path::PathBuf::from(&requested)
                    };
                    if let Some(parent) = path.parent()
                        && let Err(e) = std::fs::create_dir_all(parent)
                    {
                        return format!("📄 Could not create `{}`: {e}", parent.display());
                    }
                    match std::fs::write(&path, markdown) {
                        Ok(()) => {
                            tracing::info!(
                                session_id = %session_id,
                                path = %path.display(),
                                messages = adapted.len(),
                                "Gateway /export: wrote the session transcript"
                            );
                            format!(
                                "📄 Exported {} message(s) to `{}`\nSend me a message and I can keep going from there.",
                                adapted.len(),
                                path.display()
                            )
                        }
                        Err(e) => format!(
                            "📄 Could not write `{}`: {e}\nThe conversation is intact — nothing was lost.",
                            path.display()
                        ),
                    }
                })
                .await;
            Some(reply)
        }

        // ── /memory [query] — the live memory engine, over a channel ──
        // RT-179 plan item 3.2. The TUI's twin SHELLS OUT to `operant
        // memory` (tui/app/commands.rs:387), which is a second process with
        // its own store view — so the channel twin is FREE to be better and
        // go straight at the agent's own provider, the same engine the
        // turn's prefetch reads.
        //
        // It is deliberately built on the TRAIT surface (`name`,
        // `is_available`, `check_health`, `write_bank`, `prefetch`) rather
        // than downcasting to `MemoryWireProvider`. `recall_scored` is a
        // concrete-only method and there is no `as_any` seam on the trait,
        // so a downcast would compile and silently fail for any provider
        // that is not memory-wire — the exact shape of the defect this
        // slice exists to close. The trait surface is what EVERY provider
        // guarantees, so `/memory` works on memory-wire, on builtin, and on
        // whatever replaces them, and it degrades honestly on
        // `builtin`/`disabled` by saying memory is OFF.
        //
        // `prefetch` is the recall path: it returns the SAME `[memory]`
        // block the agent's own turn prefetch injects, so the channel and
        // the turn agree about what is in there.
        "memory" => {
            let query = args.trim().to_string();
            let provider = handler.slots.slot_for(session_key).agent.memory_provider();
            let Some(provider) = provider else {
                return Some(
                    "🧠 No memory backend is attached to this session.\nSet `[memory] provider` in the config (the default `memory-wire` is the in-process engine) and restart.".to_string(),
                );
            };
            // Status is cheap and always honest, and it is the answer to a
            // bare `/memory`.
            let name = provider.name().to_string();
            let available = provider.is_available();
            let persistent = provider.is_persistent();
            let bank = provider.write_bank();
            let healthy = provider.check_health().await;
            let mut status = format!(
                "🧠 Memory: `{name}` — {}\nPersistent: {} · Bank: `{}`",
                if healthy {
                    "reachable"
                } else {
                    "NOT reachable"
                },
                persistent,
                bank.as_deref()
                    .unwrap_or("(none — this turn is not banked)"),
            );
            if !available {
                status.push_str(
                    "\n⚠️ The engine is configured but not available — the agent runs without memory this turn.",
                );
            }
            if query.is_empty() {
                status.push_str(
                    "\nAsk me a question, or `/memory <text>` to recall matching entries directly.",
                );
                return Some(status);
            }
            // The recall itself. `prefetch` formats the `[memory]` block
            // the turn would see, so this is the REAL answer rather than a
            // second query path that could drift.
            let recalled = provider.prefetch(&query).await;
            if recalled.trim().is_empty() {
                status.push_str(&format!("\n🔍 Nothing recalled for `{query}`."));
            } else {
                status.push_str(&format!("\n🔍 Recall for `{query}`:\n"));
                // Bounded: a channel message has a practical ceiling, and a
                // full bank dump through a chat is a worse experience than
                // the head of one. The tool surface
                // (`operant memory search`) is where an unbounded query
                // belongs, and it is named so the operator knows where to
                // go for more.
                const CHANNEL_RECALL_HEAD_CHARS: usize = 1200;
                let shown: String = recalled.chars().take(CHANNEL_RECALL_HEAD_CHARS).collect();
                status.push_str(shown.trim_end());
                if recalled.chars().count() > CHANNEL_RECALL_HEAD_CHARS {
                    status.push_str(&format!(
                        "\n…(truncated — `operant memory search \"{query}\"` for the full list)"
                    ));
                }
            }
            Some(status)
        }

        // ── /skill [name [instruction]] — the installed skill surface ─────
        // RT-179 plan item 3.3. /skills (plural) already EXISTS on the
        // gateway as the ledger point of contact for this surface, and it
        // opens the TUI's browse panel — which is exactly what a channel
        // cannot render. /skill (singular) is the INVOKE form the TUI has
        // (tui/app/commands.rs:394): `/skill gitcrawl summarize the repo`.
        //
        // There is one real seam and it is shared deliberately:
        // `build_skill_invocation_message_in` is THE function the TUI
        // injects into `pending_user_message`, so both surfaces expand a
        // skill through one code path and produce one message shape.
        //
        // What /skill does NOT do is silently spawn a turn. The TUI injects
        // its built message into the loop's own pending-user-message slot,
        // a cell only the TUI process owns; the gateway dispatch loop
        // returns a reply string and has no equivalent slot (adding one
        // would be an architectural change to a peer-owned area). So the
        // honest channel answer is to EXPAND the skill and hand back the
        // full invocation message the agent will follow, so the operator
        // can send it and the skill actually runs — that is a real seam
        // against the alternative, which is a `/skill` that replies and
        // does nothing.
        // BOTH spellings — the registry's canonical `skills` and its alias
        // `skill` — so the dispatch reaches ONE implementation instead of
        // two near-copies that only one of them can reach (iter-2159).
        "skill" | "skills" => {
            let rest = args.trim();
            let skills_dir = handler.skills_root_dir.clone();
            // The vendor roots AND the configured extra roots, via the ONE
            // helper: the browse, the invoke and the picker all resolve
            // against the same set the TUI and `operant doctor` report.
            let roots = skill_roots_for(handler);
            if rest.is_empty() {
                // The browse half, over a channel: the REAL installed list,
                // not a canned "see the TUI". A root with no skills says so
                // rather than reading as an empty reply.
                let mut mgr = operant_core::skills::SkillManager::new(skills_dir);
                for root in &roots {
                    mgr.add_extra_root(Some(root.clone()));
                }
                let loaded = mgr.load_all().unwrap_or_default();
                let rows: Vec<(String, String)> = mgr
                    .list()
                    .into_iter()
                    .filter(|(name, _)| {
                        // Only what can actually run: a skill whose
                        // prerequisites are unmet is listed by name so the
                        // operator is not told it is usable when it is not.
                        mgr.get(name).map(|s| mgr.is_available(s)).unwrap_or(false)
                    })
                    .collect();
                let mut reply = format!("🧩 {} installed skills:", loaded.len());
                if rows.is_empty() {
                    reply.push_str(
                        "\n(none loadable — check the skills directory paths in `[skills]`)",
                    );
                } else {
                    // Bounded head + count, the same shape /memory uses: a
                    // 107-skill dump through a chat message is a worse
                    // answer than the head of the list plus its size.
                    const CHANNEL_SKILL_HEAD: usize = 30;
                    for (name, desc) in rows.iter().take(CHANNEL_SKILL_HEAD) {
                        reply.push_str(&format!("\n• **{name}** — {desc}"));
                    }
                    if rows.len() > CHANNEL_SKILL_HEAD {
                        reply.push_str(&format!(
                            "\n…(and {} more — `operant skills list` for all)",
                            rows.len() - CHANNEL_SKILL_HEAD
                        ));
                    }
                    reply.push_str(
                        "\nSend `$<name>` to run one inline, or `/skill <name> [instruction]`.",
                    );
                }
                return Some(reply);
            }
            // `/skill <name> [instruction]` — optional trailing instruction
            // after the name, split exactly as the TUI splits it (on the
            // first whitespace) so both surfaces invoke the same way.
            let (skill_name, instruction) = match rest.split_once(char::is_whitespace) {
                Some((n, i)) => (n, i.trim().to_string()),
                None => (rest, String::new()),
            };
            match operant_core::agent::skill_preprocessing::build_skill_invocation_message_in(
                &skills_dir,
                skill_name,
                &instruction,
                // The agent's session id for this chat, derived exactly as
                // `run_session_scoped_command` derives it (a PINNED or
                // persisted session id differs; the derived one is the
                // default this arm can resolve without the gateway lock).
                Some(derived_gateway_session_id(session_key).as_str()),
                &roots,
            ) {
                Some(msg) => {
                    // Bounded to the same 1200-char head /memory uses, with
                    // the marker made explicit so a truncated body is never
                    // mistaken for the whole skill.
                    const CHANNEL_SKILL_HEAD_CHARS: usize = 1200;
                    let total_chars = msg.chars().count();
                    let shown: String = msg.chars().take(CHANNEL_SKILL_HEAD_CHARS).collect();
                    let mut reply = format!(
                        "🧩 Skill `{skill_name}`{}\n\n{shown}",
                        if instruction.is_empty() {
                            String::new()
                        } else {
                            format!(" — with your instruction: {instruction}")
                        }
                    );
                    if total_chars > CHANNEL_SKILL_HEAD_CHARS {
                        reply.push_str(
                            "\n…(truncated — send this message to run the skill in full)",
                        );
                    }
                    Some(reply)
                }
                None => Some(format!(
                    "Skill `{skill_name}` not found — bare `/skill` lists what is installed."
                )),
            }
        }

        // ── /ponytail [lite|full|ultra|off] — the posture, for THIS chat ──
        // (iter-2211) Per-CHAT: the override lives on the session's agent,
        // not in a process-global cell, so one chat's posture cannot flip
        // every other chat in the daemon (upstream warns about exactly
        // that in its Hermes note: "runtime mode is process-local").
        // Upstream's rule: a bare `/ponytail` switches the posture ON at the
        // default level when it is off, and otherwise reports the level.
        "ponytail" => {
            let requested = args.trim().to_ascii_lowercase();
            let reply = handler
                .run_session_scoped_command(session_key, move |agent, _session_id| async move {
                    let configured = agent.ponytail_settings().clone();
                    match requested.as_str() {
                        "" => match operant_core::ponytail::bare_command_effect(
                            &configured,
                            agent.posture_override(),
                        ) {
                            operant_core::ponytail::BareCommand::Report(level) => format!(
                                "ponytail: {level} (the posture is on for this chat; \
                                 /ponytail off clears it)"
                            ),
                            operant_core::ponytail::BareCommand::SwitchOn(level) => {
                                if operant_core::ponytail::installed_ruleset().is_none() {
                                    "ponytail is not installed — run `operant update \
                                     --integrations` first; the posture needs the upstream \
                                     ruleset on disk."
                                        .to_string()
                                } else {
                                    agent.set_posture_override(Some(
                                        operant_core::ponytail::PostureOverride::Level(level),
                                    ));
                                    format!(
                                        "ponytail: {level} — switched ON for this chat (it \
                                         was off); the frozen prefix carries the ruleset from \
                                         the NEXT turn (/ponytail off clears it)."
                                    )
                                }
                            }
                        },
                        "off" => {
                            agent.set_posture_override(Some(
                                operant_core::ponytail::PostureOverride::Off,
                            ));
                            "ponytail: off for this chat — the config on disk is untouched."
                                .to_string()
                        }
                        "lite" | "full" | "ultra" => {
                            let level = match requested.as_str() {
                                "lite" => operant_core::config::PonytailLevel::Lite,
                                "ultra" => operant_core::config::PonytailLevel::Ultra,
                                _ => operant_core::config::PonytailLevel::Full,
                            };
                            if operant_core::ponytail::installed_ruleset().is_none() {
                                "ponytail is not installed — run `operant update \
                                 --integrations` first; the posture needs the upstream ruleset on \
                                 disk."
                                    .to_string()
                            } else {
                                agent.set_posture_override(Some(
                                    operant_core::ponytail::PostureOverride::Level(level),
                                ));
                                format!(
                                    "ponytail: {level} — the frozen prefix carries the ruleset \
                                     from the NEXT turn for THIS chat (a level switch \
                                     invalidates the prompt cache, like any config change)."
                                )
                            }
                        }
                        other => {
                            format!("Unknown ponytail level '{other}' — use lite|full|ultra|off")
                        }
                    }
                })
                .await;
            Some(reply)
        }

        // ── /search <query> — long-term conversation recall ──────────────
        // RT-179 plan item 3.5. The plan said "resolve which search first":
        // resolved. The TUI's `/search` (`commands.rs:373`, aliased `/find`)
        // opens the ripgrep GLOBAL SEARCH dialog — a file search over
        // whatever CWD the TUI was launched in. A gateway daemon has no
        // project root (it is a systemd unit started from `$HOME`), so the
        // file twin would be searching a directory the operator never chose
        // and never sees — a literal that runs, an answer to the wrong
        // question, which is the theater the R17 /yolo lesson names.
        //
        // What the agent ITSELF searches when a channel user asks "have we
        // talked about this?" is `session_search` — FTS5 over every session
        // with browse/discovery/scroll modes. That is the twin: the SAME
        // tool the model calls, driven from the command.
        //
        // It rides the storage-window contract, though: a fresh turn-scoped
        // session is created on the fly if this chat has no history yet
        // (`notify_sessions_can_appear_after_a_fresh_empty_turn` pins WHY) —
        // so a bare `/search` must not read as "nothing anywhere". It
        // reports the STORE, not this chat, and says so.
        "search" | "find" => {
            // The operator's disable policy is a listing filter on the registry
            // path but nothing at all here: this arm constructs
            // `SessionSearchTool` itself and calls `OperantTool::execute` on it,
            // so `operant tools disable session_search` used to leave the
            // channel command searching. Every direct-execute arm asks first
            // (RT-3's rule, `enrichment.rs:213`).
            if direct_call_is_disabled("session_search") {
                return Some(
                    "🔎 `session_search` is disabled by operator policy — re-enable it with \
                     `operant tools enable session_search`."
                        .to_string(),
                );
            }
            let reply = handler
                .run_session_scoped_command(session_key, move |agent, session_id| async move {
                    let db = agent.session_store().db();
                    let tool = operant_core::tools::session_search_tool::SessionSearchTool::new(db);
                    // The trait owns `execute`; without it in scope the
                    // inherent-method lookup fails.
                    use operant_core::tools::OperantTool as _;
                    let query = args.trim();
                    // Bare /search = browse: the recent sessions, exactly the
                    // tool's no-query mode. Zero LLM cost, and it is the
                    // shape a channel user wants first.
                    let args_value = if query.is_empty() {
                        serde_json::json!({ "limit": 10 })
                    } else {
                        serde_json::json!({ "query": query, "limit": 10 })
                    };
                    let result = tool
                        .execute(args_value, operant_core::tools::ToolContext::default())
                        .await;
                    tracing::debug!(
                        session_id = %session_id,
                        query = %query,
                        success = result.success,
                        "Gateway /search: drove the session_search tool"
                    );
                    if !result.success {
                        return format!("🔎 Search failed: {}", result.content);
                    }
                    // The tool already formats its own browse/listing output;
                    // pass it through rather than re-deriving a second
                    // renderer that could drift from what the model sees.
                    // Only the head is surfaced, the same 1200-char bound
                    // /memory and /skill use, with a pointer to the CLI for
                    // the rest.
                    const CHANNEL_SEARCH_HEAD_CHARS: usize = 1200;
                    let total = result.content.chars().count();
                    let shown: String =
                        result.content.chars().take(CHANNEL_SEARCH_HEAD_CHARS).collect();
                    let mut reply = if query.is_empty() {
                        format!("🔎 Recent sessions (this agent's store, all chats):\n{shown}")
                    } else {
                        format!("🔎 Sessions matching `{query}`:\n{shown}")
                    };
                    if total > CHANNEL_SEARCH_HEAD_CHARS {
                        // The pointer is to ASK, not to a CLI flag: `operant
                        // sessions` lists but has no query mode (verified
                        // against the deployed binary — `sessions list`
                        // takes no `--query`), so pointing there would be a
                        // dead end. Asking the agent runs the search tool
                        // with the full result, which is the real answer.
                        reply.push_str(
                            "\n…(truncated — ask me to search further and I will read the full results)",
                        );
                    }
                    reply
                })
                .await;
            Some(reply)
        }

        _ => None,
    }
}

/// The one-line share-of-window sentence for `/context`.
///
/// Split out so the arithmetic (and its ceiling) is testable directly, and so
/// the reply reads as a fraction rather than a bare percentage — a channel
/// user cannot see the footer bar, so the numbers have to carry the shape.
pub(crate) fn context_headroom_line(estimated: usize, window: usize) -> String {
    if window == 0 {
        return "• Share of window: unknown (no window is resolved for this model).".to_string();
    }
    let pct = (estimated as f64 / window as f64 * 100.0).round() as u64;
    let pct = pct.min(100);
    format!(
        "• Share of window: ~{pct}% full — roughly {} of {} tokens used.",
        estimated.min(window),
        window
    )
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
    // RT-427: the denial now carries its resolution (the resolver's name);
    // this path records it rather than dropping it.
    match approver.deny(ask) {
        Ok(resolution) => {
            tracing::info!(request = %ask.request_id, resolution = %resolution, "seat escalation denied");
        }
        Err(e) => {
            tracing::warn!(request = %ask.request_id, error = %e, "recording the denial failed");
        }
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
    match approver.expire(ask) {
        Ok(resolution) => {
            tracing::info!(request = %ask.request_id, resolution = %resolution, "seat escalation lapsed");
        }
        Err(e) => {
            tracing::warn!(request = %ask.request_id, error = %e, "recording the lapse failed");
        }
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

/// The skill picker a chat is looking at.
///
/// One entry per chat, keyed `"{platform}:{channel_id}"`. The picker's
/// buttons are INDEX-based, so this list IS the index space: a re-rendered
/// page keeps every earlier button honest.
#[derive(Debug, Clone)]
pub struct PendingSkillPicker {
    /// The skills on offer: `(name, description)`.
    pub entries: Vec<(String, String)>,
    /// The query that produced them ("" = browse everything).
    pub query: String,
    /// The message the rows were rendered into.
    ///
    /// A chat can have TWO pickers open (the operator ran `$` again while the
    /// first was still on screen), and a callback carries an INDEX — which
    /// means nothing without the list it was drawn from. The id is what says
    /// "this tap belongs to the picker still on offer"; a tap from any other
    /// picker is stale, and must not invoke the wrong skill.
    pub message_id: Option<String>,
}

/// The per-chat skill pickers.
pub static PENDING_SKILL_PICKERS: OnceLock<std::sync::Mutex<HashMap<String, PendingSkillPicker>>> =
    OnceLock::new();

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
        E::RequestTimeout { .. } => "request_timeout",
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

/// Where the daemon records the socket path it actually bound (iter-2366).
///
/// A sidecar beside the pid file rather than a field inside it: the pid file is
/// read by `kill -0` callers that parse it as a bare integer, and widening that
/// format would break every one of them for a fact only `gateway status` wants.
pub(crate) fn socket_path_file() -> std::path::PathBuf {
    operant_core::platform::operant_home().join("gateway.sock")
}

/// The home this daemon resolved, as recorded at start (iter-2366).
///
/// Detection, not prevention. The per-home `flock` only stops a second daemon
/// that resolves the SAME home, and `operant_home()` follows `$HERMES_HOME` and
/// the mutable active profile — so a daemon started from a shell with a
/// different `HERMES_HOME` than the systemd unit's produces two daemons that
/// each believe they hold the machine. Recording the home lets `gateway status`
/// say which daemon this CLI is actually talking to, instead of silently
/// describing the wrong one. A machine-wide flock would prevent it outright and
/// was tried and reverted: it is one fixed path, so it serialised the gateway
/// tests that start several daemons in one process with distinct homes.
pub(crate) fn home_file() -> std::path::PathBuf {
    operant_core::platform::operant_home().join("gateway.home")
}

/// The home the running daemon recorded, or `None` when it predates this file.
pub(crate) fn recorded_home() -> Option<String> {
    std::fs::read_to_string(home_file())
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

/// The socket path this daemon bound, as recorded at start.
///
/// Falls back to the default path when there is no sidecar: a daemon started
/// before this file existed, or one whose write failed, is served at the
/// default, which is what every client recomputes anyway.
pub(crate) fn recorded_socket_path() -> std::path::PathBuf {
    let path = socket_path_file();
    match std::fs::read_to_string(&path) {
        Ok(raw) if !raw.trim().is_empty() => std::path::PathBuf::from(raw.trim()),
        _ => operant_core::session_client::default_socket_path(),
    }
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
    /// The open file descriptor that HOLDS the flock. Dropping the guard
    /// closes it and the kernel releases the lock.
    ///
    /// (RT-526) The lock file is NOT unlinked. `flock` protects the open
    /// INODE, so removing the path let a second starter create a NEW inode
    /// and lock that one — two daemons holding "the lock", the exact
    /// duplicate the flock was added for. A leftover file is harmless: the
    /// next starter opens the SAME inode and locks it.
    _file: std::fs::File,
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
/// the lock file itself is NOT removed (RT-526: `flock` protects the inode, so
/// unlinking the path would let a second daemon lock a NEW inode).
static GATEWAY_DAEMON_LOCKS: std::sync::Mutex<Vec<GatewayDaemonLock>> =
    std::sync::Mutex::new(Vec::new());

/// Take the exclusive gateway daemon lock, or report that a live gateway
/// holds it. `Ok(None)` means REFUSE TO START — the caller must not boot a
/// second daemon.
fn acquire_gateway_daemon_lock() -> Result<Option<GatewayDaemonLock>> {
    acquire_gateway_daemon_lock_at(operant_core::platform::operant_home().join("gateway.lock"))
}

/// The same lock, at an explicit path (RT-526) — the seam a test uses, so
/// the test does not need the process's operant home.
fn acquire_gateway_daemon_lock_at(path: std::path::PathBuf) -> Result<Option<GatewayDaemonLock>> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

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
        Ok(taken.then_some(GatewayDaemonLock { _file: file }))
    }
    #[cfg(not(unix))]
    {
        // Windows has no flock; the pid file remains the (best-effort)
        // duplicate signal there. The gateway daemon is a systemd/foreground
        // Unix deployment surface in practice.
        Ok(Some(GatewayDaemonLock {
            _file: std::fs::File::create(std::path::PathBuf::from("NUL"))
                .context("windows no-op lock placeholder")?,
        }))
    }
}
/// Whether another message may be queued for this chat (RT-521).
///
/// The queue is per session: a chat that floods while its turn is busy must
/// not grow the queue without limit.
fn queue_has_room(queue: &VecDeque<(String, IncomingMessage)>, session_key: &str) -> bool {
    queue.iter().filter(|(key, _)| key == session_key).count() < MAX_QUEUED_MESSAGES_PER_SESSION
}

/// How many messages one chat may have QUEUED behind its own in-flight turn
/// (RT-521). The queue was unbounded, so a chat that kept talking while its
/// turn was busy grew the queue until the process died. The drop is logged:
/// a dropped chat message is a real loss.
const MAX_QUEUED_MESSAGES_PER_SESSION: usize = 32;

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

    /// The numbers for the daemon's state file (RT-523).
    fn snapshot(&self) -> (u64, u64, u64) {
        (self.events, self.total_ms, self.worst_ms)
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
    // The NON-recording parse: this only READS the file the process already
    // loaded, and a read must never redirect the surgical writer's target
    // (the RT-488 incident).
    let config = operant_core::config::parse_config_file_without_recording(&path).ok()?;
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

/// The memory provider for ONE agent slot (RT-508 red-team, finding 1).
///
/// The provider holds the memory SCOPE — the bank the turn reads from and
/// writes to — and the scope is set per turn. One shared provider would
/// mean one shared scope cell: with the slots running turns at the same
/// time, one slot could set the scope while another slot's turn was
/// between its recall and its retain, and a seat's turn would recall from
/// another seat's bank. That is exactly the leak the old one-turn-at-a-time
/// lock prevented.
///
/// The providers share the STORE (the same memory-wire file ✓ — one memory,
/// not four) and differ only in the scope cell.
fn slot_memory_provider() -> Option<Arc<operant_core::memory_wire::MemoryWireProvider>> {
    operant_core::memory_wire::MemoryWireProvider::new(operant_core::platform::operant_home())
        .ok()
        .map(Arc::new)
}

use operant_core::session_protocol::WireMessage;
use tokio::sync::broadcast;

/// The gateway's session bus (RT-544).
///
/// One broadcast sender per SESSION. The slot that is serving a session
/// publishes the turn's events under that session's id, and a client that
/// attaches to the session subscribes by id — so two clients on one session see
/// the same turn, and a client on another session does not.
struct SessionBus {
    senders: std::sync::Mutex<std::collections::HashMap<String, broadcast::Sender<WireMessage>>>,
}

impl SessionBus {
    fn new() -> Self {
        Self {
            senders: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Subscribe a client to a session, dropping the senders nobody reads.
    ///
    /// This is the ONLY place a sender is created (iter-2366). The map is keyed
    /// by a session id ANY client can name (a `LoadHistory` for a made-up id
    /// subscribes to it), so without the `retain` below it grows for as long as
    /// the daemon lives and each entry is a 256-slot ring. A client joining is
    /// rare; a turn emitting an event is not.
    fn subscribe(&self, session_id: &str) -> broadcast::Receiver<WireMessage> {
        // A poisoned bus lock must not take the gateway down: the map is
        // recoverable state, so drop the poison like every other lock site
        // in this file (`unwrap_or_else(|e| e.into_inner())`).
        let mut senders = self.senders.lock().unwrap_or_else(|e| e.into_inner());
        senders.retain(|_, sender| sender.receiver_count() > 0);
        let sender = senders
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(256).0);
        sender.subscribe()
    }

    /// Publish one agent event to the session's subscribers.
    ///
    /// (iter-2366) A READ-ONLY lookup, not an insert. `sender_for` used to be
    /// called from here, so every session id that ever emitted an event got a
    /// 256-slot broadcast ring that nothing ever removed unless a client
    /// happened to call `subscribe` — and a channel-only deployment, which is
    /// the normal one, never does. 500 sessions × one turn each retained 500
    /// rings for the life of the daemon, with no receiver ever attached to any
    /// of them. An entry now exists only while someone is subscribed to it.
    fn publish(&self, session_id: &str, event: AgentEvent) {
        // A closed channel means nobody is attached; that is not an error.
        let senders = match self.senders.lock() {
            Ok(senders) => senders,
            Err(_) => return,
        };
        let Some(sender) = senders.get(session_id) else {
            return;
        };
        let _ = sender.send(WireMessage::Event {
            session_id: session_id.to_string(),
            event,
        });
    }
}

/// The gateway's answer on the session socket (RT-542, RT-544).
///
/// The gateway OWNS the runtime — this handler only speaks the protocol on its
/// behalf. A prompt is run in the SLOT that owns that session (the deterministic
/// hash), and the slot's events flow back to every client attached to the
/// session through the bus.
struct GatewaySessionHandler {
    /// The gateway's own message handler, which owns the slots and the agents.
    ///
    /// `None` in a test that exercises the socket and the handshake without a
    /// gateway: a prompt is then answered with a named error rather than a
    /// silent turn.
    handler: Option<std::sync::Arc<GatewayMessageHandler>>,
    bus: std::sync::Arc<SessionBus>,
    /// Sessions with a socket turn ADMITTED but not yet holding a slot (W3-2
    /// for the socket). A cancel needs to tell "running now" from "waiting for
    /// a slot": only the first can be stopped by the interrupt flag, and only
    /// the second can be dropped from the queue.
    socket_inflight: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

/// A session's claim on the concurrency cap, released when it drops.
///
/// The socket used to `acquire` and never `release`, so the tracker's lock file
/// outlived the turn and was only reclaimed after `STALE_AFTER_SECS` (six
/// hours): with a cap of four, four distinct socket sessions exhausted it for
/// the rest of the day while the channels kept working. The guard is what makes
/// the release unconditional — a finished turn, an error, or a panic all give
/// the slot back.
struct SessionLease {
    tracker: operant_core::active_sessions::ActiveSessionTracker,
    session_id: String,
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        self.tracker.release(&self.session_id);
    }
}

/// The channel key holding `tool_call_id`, if any (read-only).
fn pending_permission_channel(tool_call_id: &str) -> Option<String> {
    let store = PENDING_PERMISSIONS.get()?;
    let guard = store.lock().ok()?;
    let pending = guard.as_ref()?;
    let map = pending.try_lock().ok()?;
    map.iter()
        .find_map(|(channel, request)| (request.tool_id == tool_call_id).then(|| channel.clone()))
}

/// Resolve a pending tool-permission ask by its tool-call id (the session
/// socket's `/approve` and `/deny`).
///
/// The channel path keys its pending map by CHANNEL, and a socket client has no
/// channel; the tool-call id is the identity the wire carries, so the map is
/// scanned for it. Best-effort by design: a request that is not found (already
/// answered, already timed out) reports false rather than inventing an approval.
fn resolve_pending_permission(
    tool_call_id: &str,
    allow: bool,
    requester_session: Option<&str>,
) -> bool {
    use operant_core::agent::ToolPermissionResponse;
    // (iter-2366) Two kinds of key now live in the pending map.
    //
    // A CHANNEL key belongs to a chat turn: the socket must not answer it, or a
    // local process could approve a tool waiting for a human on Telegram (the
    // RT-550 hardening, kept intact below).
    //
    // A SESSION key (`session:<id>`) belongs to a turn the socket started, and
    // the ONLY client that may answer it is one attached to THAT session. The
    // Approve/Deny arms carry the asking client's session id for exactly this
    // comparison, so a socket client can answer its own ask and nobody else's.
    if let Some(channel) = pending_permission_channel(tool_call_id)
        && !channel.is_empty()
    {
        let mine = requester_session
            .map(|session| channel == format!("session:{session}"))
            .unwrap_or(false);
        if !mine {
            tracing::warn!(
                channel = %channel,
                "an approval for a pending request was refused on the socket: \
                 it belongs to another session or to a channel"
            );
            return false;
        }
    }
    let Some(store) = PENDING_PERMISSIONS.get() else {
        return false;
    };
    let Ok(guard) = store.lock() else {
        return false;
    };
    let Some(pending) = guard.as_ref() else {
        return false;
    };
    let Ok(mut map) = pending.try_lock() else {
        return false;
    };
    let found = map
        .iter()
        .find_map(|(channel, request)| (request.tool_id == tool_call_id).then(|| channel.clone()));
    let Some(channel) = found else {
        return false;
    };
    let Some(request) = map.remove(&channel) else {
        return false;
    };
    let response = if allow {
        ToolPermissionResponse::AllowSession
    } else {
        ToolPermissionResponse::Deny
    };
    request.response_tx.send(response).is_ok()
}

impl GatewaySessionHandler {
    /// Stop the session's turn, wherever it is: running, or waiting for a slot.
    ///
    /// One rule, two callers (`Cancel` and `/stop`). The interrupt flag belongs
    /// to the SLOT and ends a RUNNING turn; a turn still waiting for a slot has
    /// no flag to raise, so it is marked in `cancelled_queued` and the turn
    /// entry returns before it runs. The channel path already works this way
    /// (`W3-2`); the socket path had the first half only, so a queued prompt
    /// could not be stopped at all.
    fn stop_turn(&self, session_id: &str) -> &'static str {
        let Some(handler) = self.handler.as_ref().map(std::sync::Arc::clone) else {
            return "this server has no agent slots to run a turn in";
        };
        let running = handler
            .running_session
            .lock()
            .map(|running| running.contains_key(session_id))
            .unwrap_or(false);
        if running {
            handler
                .slots
                .slot_for(session_id)
                .agent
                .interrupt_flag()
                .trigger();
            return "the turn was cancelled";
        }
        let queued = self
            .socket_inflight
            .lock()
            .map(|inflight| inflight.contains(session_id))
            .unwrap_or(false);
        if queued {
            if let Ok(mut cancelled) = handler.cancelled_queued.lock() {
                cancelled.insert(session_id.to_string());
            }
            // It will not run, so it is no longer in flight: leaving the mark
            // behind made a second cancel report a turn that was already gone.
            if let Ok(mut inflight) = self.socket_inflight.lock() {
                inflight.remove(session_id);
            }
            return "the queued turn was cancelled";
        }
        "there was no turn to cancel"
    }
}

impl operant_core::session_server::SessionHandler for GatewaySessionHandler {
    fn handle(
        &self,
        message: operant_core::session_protocol::WireMessage,
    ) -> Vec<operant_core::session_protocol::WireMessage> {
        use operant_core::session_protocol::WireMessage;
        match message {
            WireMessage::Hello => {
                // The daemon's OWN record of what it can serve. The presence
                // directory is written by the TUI alone, so reading it here
                // always answered zero and the client made up an id.
                let sessions = self
                    .handler
                    .as_ref()
                    .map(|handler| handler.live_session_ids())
                    .unwrap_or_default();
                vec![WireMessage::HelloOk { sessions }]
            }
            // A client changing session: the connection re-subscribes to any
            // session a message names (see `serve_client`), so the switch needs
            // no state of its own — the answer is what the new session holds.
            WireMessage::Switch { session_id } => {
                self.handle(WireMessage::LoadHistory { session_id })
            }
            // A prompt runs the turn in the SLOT that owns this session.
            //
            // The reply is not the answer: the answer arrives as events on
            // the client's subscription. The turn is spawned so this client's
            // connection stays free to receive them.
            WireMessage::Prompt { session_id, text } => {
                let Some(handler) = self.handler.as_ref().map(std::sync::Arc::clone) else {
                    return vec![WireMessage::Error {
                        session_id: None,
                        message: "this server has no agent slots to run a turn in".to_string(),
                    }];
                };
                // Commands first, before any gate. `/stop` is what an operator
                // reaches for when a turn is stuck, so a pause or a full cap
                // must not make it answer "operant is paused" instead of
                // stopping. A command is a slash plus a KNOWN name, not any
                // leading slash: a prompt about `/etc/hosts` is a prompt.
                match text.split_whitespace().next().unwrap_or("") {
                    "/stop" => {
                        let message = self.stop_turn(&session_id);
                        return vec![WireMessage::Error {
                            session_id: Some(session_id),
                            message: message.to_string(),
                        }];
                    }
                    "/sessions" => {
                        return vec![WireMessage::HelloOk {
                            sessions: handler.live_session_ids(),
                        }];
                    }
                    _ => {}
                }
                // (iter-2366) The drain's admission gate. A stop interrupts the
                // turns already running and waits for their slot locks; a turn
                // admitted after that point would queue on the very lock being
                // waited for and then reset the interrupt on acquiring it, so
                // the drain could never complete and the process would exit
                // with that turn dropped mid-await. Refuse it here, where the
                // client can still be told why.
                if handler.stopping.load(std::sync::atomic::Ordering::SeqCst) {
                    return vec![WireMessage::Error {
                        session_id: Some(session_id),
                        message: "the gateway is stopping — the turn was not started".to_string(),
                    }];
                }
                // A turn over the socket obeys the SAME gates a channel turn
                // does. ESTOP is a pause, not a channel-only setting.
                if operant_core::estop::is_engaged() {
                    return vec![WireMessage::Error {
                        session_id: Some(session_id),
                        message: "operant is paused — send /unpause or run `operant resume`"
                            .to_string(),
                    }];
                }
                // Another PROCESS already owns this session (a TUI that has it
                // open writes the presence registry). Two writers on one
                // transcript is the corruption the slot design exists to
                // prevent, so the prompt is refused rather than interleaved.
                if operant_core::active_sessions::session_is_live(&session_id) {
                    let message = format!(
                        "another live process owns session {session_id} — close it first \
                         (`operant sessions` lists them)"
                    );
                    return vec![WireMessage::Error {
                        session_id: Some(session_id),
                        message,
                    }];
                }
                // The cap bounds CONCURRENT TURNS, so it is counted, not
                // reserved: a prompt is admitted while fewer than `max` turns
                // are running, and takes its lease only once it holds a slot.
                // Holding the lease while queued let one busy slot consume the
                // whole cap — four sessions behind one long turn refused the
                // fifth although three slots were idle.
                let cap = handler
                    .max_concurrent_sessions
                    .map(|max| max.min(handler.slots.slots.len().max(1)));
                if let Some(max) = cap {
                    let tracker = operant_core::active_sessions::ActiveSessionTracker::new(
                        handler.session_tracker_dir.clone(),
                        Some(max),
                    );
                    // (iter-2366) Still synchronous, and deliberately so: this
                    // arm is inside `SessionHandler::handle`, which is a sync
                    // trait method, so there is no `.await` here. The blocking
                    // directory pass now costs ONE walk instead of two (see
                    // `ActiveSessionTracker::scan`), and moving it off the
                    // runtime entirely means making the trait async or wrapping
                    // the whole `handle` call in `spawn_blocking` at the
                    // `session_server` dispatch site.
                    let running = tracker.count().unwrap_or(0);
                    if running >= max {
                        return vec![WireMessage::Error {
                            session_id: Some(session_id),
                            message: format!(
                                "too many concurrent sessions (limit {max}) — the turn was not started"
                            ),
                        }];
                    }
                }
                let bus = std::sync::Arc::clone(&self.bus);
                let task_session = session_id.clone();
                if let Ok(mut inflight) = self.socket_inflight.lock() {
                    inflight.insert(session_id.clone());
                }
                let inflight = std::sync::Arc::clone(&self.socket_inflight);
                tokio::spawn(async move {
                    let slot = handler.slots.slot_for(&task_session);
                    // ONE turn at a time in this slot (the same lock every
                    // other gateway turn takes).
                    let _guard = slot.lock.lock().await;
                    // Cancelled while it waited: drop it before it runs, the
                    // same check the channel path makes at turn start.
                    if let Ok(mut cancelled) = handler.cancelled_queued.lock()
                        && cancelled.remove(&task_session)
                    {
                        if let Ok(mut inflight) = inflight.lock() {
                            inflight.remove(&task_session);
                        }
                        return;
                    }
                    // (iter-2366) The SECOND half of the drain's admission
                    // gate. The `Prompt` arm refuses a turn that arrives after
                    // the stop begins, but a turn can already be queued on this
                    // slot lock when the drain starts — and it reaches this
                    // point only when the lock is finally free, which is after
                    // the interrupted turn has released it. Letting it run
                    // would reset the interrupt the drain is relying on and
                    // hold the lock the drain is waiting for, so the drain
                    // times out and this turn is dropped mid-await anyway.
                    // Dropping it here is the difference between "refused" and
                    // "started, ran, and was silently lost".
                    if handler.stopping.load(std::sync::atomic::Ordering::SeqCst) {
                        if let Ok(mut inflight) = inflight.lock() {
                            inflight.remove(&task_session);
                        }
                        bus.publish(
                            &task_session,
                            operant_core::agent::AgentEvent::Error {
                                error: "the gateway was stopping before this turn got a slot — \
                                        it was not started"
                                    .to_string(),
                            },
                        );
                        return;
                    }
                    // Now that the slot is held the turn is real, so it takes a
                    // cap slot — and if the cap filled while it waited, it says
                    // so instead of running over the limit.
                    let _lease = match cap {
                        Some(max) => {
                            let tracker = operant_core::active_sessions::ActiveSessionTracker::new(
                                handler.session_tracker_dir.clone(),
                                Some(max),
                            );
                            // Same reason as the count above: one blocking
                            // directory pass off the runtime thread (iter-2366).
                            let blocking_tracker = tracker.clone();
                            let blocking_session = task_session.clone();
                            let admitted = tokio::task::spawn_blocking(move || {
                                blocking_tracker.acquire(&blocking_session).unwrap_or(true)
                            })
                            .await
                            .unwrap_or(true);
                            if !admitted {
                                if let Ok(mut inflight) = inflight.lock() {
                                    inflight.remove(&task_session);
                                }
                                bus.publish(
                                    &task_session,
                                    operant_core::agent::AgentEvent::Error {
                                        error: format!(
                                            "too many concurrent sessions (limit {max}) — the turn was not started"
                                        ),
                                    },
                                );
                                return;
                            }
                            Some(SessionLease {
                                tracker,
                                session_id: task_session.clone(),
                            })
                        }
                        None => None,
                    };
                    // The same turn bookkeeping a channel turn does: the
                    // session claims its slot (so a cancel can tell this
                    // session's turn from a neighbour's) and arms a clean
                    // interrupt latch, released when the turn ends.
                    let _running = RunningSessionGuard {
                        running: &handler.running_session,
                        session_key: task_session.clone(),
                    };
                    if let Ok(mut running) = handler.running_session.lock() {
                        running
                            .insert(task_session.clone(), handler.slots.index_for(&task_session));
                    }
                    slot.agent.interrupt_flag().reset();
                    // The slot's agent must be pointed at THIS session before
                    // the turn runs — the same binding the channel path does,
                    // or the transcript lands in the previous chat's history
                    // (or in a throwaway `sess_<uuid>`).
                    handler
                        .bind_slot_to_session(slot, &task_session, "session-socket")
                        .await;
                    // (A3-gateway H-2) Reset the per-session agent state before
                    // this turn runs. See `reset_slot_for_session`.
                    handler.reset_slot_for_session(slot, &task_session).await;
                    // (A3-gateway H-1) The socket turn must also carry a cron
                    // origin. Without one, `cron create` from
                    // `operant chat --attach` writes the row with NULL origin
                    // columns, and the scheduler's delivery arm has no target:
                    // the result is computed and then dropped with a `warn!`.
                    // The channel path set it and the socket path did not, so
                    // a reminder created locally could NEVER fire. The test
                    // at `gateway_runner/tests.rs` asserts the channel path
                    // only, so it stayed green.
                    slot.agent
                        .set_cron_origin(Some(socket_cron_origin(&task_session)));
                    // A message the operator QUEUED arrives WITH this turn,
                    // ahead of the draft they just submitted — the same rule
                    // the channel path applies at its turn entry.
                    let text = {
                        let parked = handler.take_queued_messages(&task_session);
                        if parked.is_empty() {
                            text
                        } else {
                            let marked: Vec<String> =
                                parked.iter().map(|m| format!("[QUEUED] {m}")).collect();
                            format!("{}\n\n{}", marked.join("\n\n"), text)
                        }
                    };
                    let agent = std::sync::Arc::clone(&slot.agent);
                    if let Err(error) = agent.run(text).await {
                        // A client must not sit in silence after a failed
                        // turn: the events are its only channel back.
                        bus.publish(
                            &task_session,
                            AgentEvent::Error {
                                error: error.user_message(),
                            },
                        );
                    }
                    // The session no longer has a turn in flight, running or
                    // queued: this is what a later cancel reads.
                    if let Ok(mut inflight) = inflight.lock() {
                        inflight.remove(&task_session);
                    }
                });
                vec![]
            }
            // RT-544: a client joining a live session is told what it has.
            //
            // The conversation is the SLOT's, and the slot is the daemon's, so
            // it outlives every client: the session never died with the client
            // that started it, and a client that attaches later reads it back.
            WireMessage::LoadHistory { session_id } => {
                let Some(handler) = self.handler.as_ref().map(std::sync::Arc::clone) else {
                    return vec![WireMessage::Error {
                        session_id: Some(session_id),
                        message: "this server has no agent slots to read a session from"
                            .to_string(),
                    }];
                };
                let slot = handler.slots.slot_for(&session_id);
                let agent = std::sync::Arc::clone(&slot.agent);
                let messages = agent.transcript_lines(&session_id);
                vec![WireMessage::History {
                    session_id,
                    messages,
                }]
            }
            // RT-550: a cancel STOPS the turn — with the INTERRUPT, not an
            // abort.
            //
            // An abort drops the future at its next await point, which skips
            // the turn's own tail: no interrupted marker, no clean commit, and
            // an in-flight tool row left saying its side effects are UNKNOWN.
            // The reload path then strips that row, so the model re-issues a
            // write that already happened (the RT-510 defect). The interrupt
            // flag is how `/stop` and the drain end a turn: the loop exits, the
            // tail is committed, and the slot's lock is released by its guard.
            WireMessage::Cancel { session_id, .. } => {
                let message = self.stop_turn(&session_id);
                vec![WireMessage::Error {
                    session_id: Some(session_id),
                    message: message.to_string(),
                }]
            }
            // RT-550's siblings: an approval the client can actually give.
            // The pending map is keyed by CHANNEL, and a socket client has no
            // channel — so the request is matched by its tool-call id, which
            // the wire already carries.
            WireMessage::Approve {
                session_id,
                tool_call_id,
            } => vec![WireMessage::Error {
                session_id: Some(session_id.clone()),
                message: match resolve_pending_permission(
                    &tool_call_id,
                    true,
                    Some(session_id.as_str()),
                ) {
                    true => "the tool call was approved".to_string(),
                    false => format!("no pending tool call named {tool_call_id}"),
                },
            }],
            WireMessage::Deny {
                session_id,
                tool_call_id,
                reason,
            } => {
                // The response enum carries no reason, so it is reported back
                // to the client that typed it rather than silently dropped.
                if let Some(reason) = reason.as_deref().filter(|r| !r.trim().is_empty()) {
                    tracing::info!(session = %session_id, tool_call_id = %tool_call_id, reason, "tool call denied over the session socket");
                }
                vec![WireMessage::Error {
                    session_id: Some(session_id.clone()),
                    message: match resolve_pending_permission(
                        &tool_call_id,
                        false,
                        Some(session_id.as_str()),
                    ) {
                        true => "the tool call was denied".to_string(),
                        false => format!("no pending tool call named {tool_call_id}"),
                    },
                }]
            }
            // A QUEUE parks a message for the NEXT turn in this session — the
            // same store `/queue` writes on a channel, keyed by session id.
            WireMessage::Queue { session_id, text } => {
                let Some(handler) = self.handler.as_ref().map(std::sync::Arc::clone) else {
                    return vec![WireMessage::Error {
                        session_id: Some(session_id),
                        message: "this server has no agent slots".to_string(),
                    }];
                };
                if operant_core::estop::is_engaged() {
                    return vec![WireMessage::Error {
                        session_id: Some(session_id),
                        message: "operant is paused — the message was not queued".to_string(),
                    }];
                }
                // (iter-2366) A queue is only meaningful for a session the
                // daemon has actually loaded. The per-session cap of 32 bounds
                // ONE session's queue; nothing bounded the NUMBER of sessions,
                // and this arm accepted any id a client cared to name — so a
                // client could park 32 messages under a million invented ids
                // and size the daemon's memory. `take_queued_messages` removes
                // an entry only when a turn consumes it, so a name that never
                // prompts leaves its `Vec` resident forever. A session no slot
                // has loaded has no next turn to queue FOR.
                let known = handler.live_session_ids().contains(&session_id);
                if !known {
                    return vec![WireMessage::Error {
                        session_id: Some(session_id.clone()),
                        message: format!(
                            "no such live session: {session_id} — start it with a prompt before \
                             queueing for it"
                        ),
                    }];
                }
                let depth = match handler.next_turn_queue.lock() {
                    Ok(mut map) => {
                        // A queue with no bound is a memory sink a client
                        // controls: park a million messages and the daemon
                        // holds them all. Past the bound the answer is a
                        // refusal, not a longer queue.
                        if map.get(&session_id).map(|q| q.len()).unwrap_or(0)
                            >= MAX_QUEUED_PER_SESSION
                        {
                            return vec![WireMessage::Error {
                                session_id: Some(session_id),
                                message: format!(
                                    "the queue holds {MAX_QUEUED_PER_SESSION} messages already — send it after the next turn"
                                ),
                            }];
                        }
                        map.entry(session_id.clone()).or_default().push(text);
                        map.get(&session_id).map(|q| q.len()).unwrap_or(0)
                    }
                    Err(_) => {
                        return vec![WireMessage::Error {
                            session_id: Some(session_id),
                            message:
                                "the queue is being read right now — send it again in a moment"
                                    .to_string(),
                        }];
                    }
                };
                vec![WireMessage::Error {
                    session_id: Some(session_id),
                    message: format!("queued for the next turn ({depth} waiting)"),
                }]
            }
            // A steer reaches the RUNNING turn between iterations — the same
            // queue the channel path pushes into (`agent.steer_queue_handle`).
            WireMessage::Steer { session_id, text } => {
                if operant_core::estop::is_engaged() {
                    return vec![WireMessage::Error {
                        session_id: Some(session_id),
                        message: "operant is paused — the message was not sent".to_string(),
                    }];
                }
                let slot = self
                    .handler
                    .as_ref()
                    .map(|handler| handler.slots.slot_for(&session_id));
                let Some(slot) = slot else {
                    return vec![WireMessage::Error {
                        session_id: Some(session_id),
                        message: "this server has no agent slots".to_string(),
                    }];
                };
                let queue = slot.agent.steer_queue_handle();
                let message = match queue.try_lock() {
                    Ok(mut queue) => {
                        queue.push(text);
                        "steered the running turn".to_string()
                    }
                    Err(_) => {
                        "the agent is reading its steer queue right now — send it again in a moment"
                            .to_string()
                    }
                };
                vec![WireMessage::Error {
                    session_id: Some(session_id),
                    message,
                }]
            }
            other => vec![WireMessage::Error {
                session_id: None,
                message: format!("the session protocol does not serve this yet: {other:?}"),
            }],
        }
    }

    fn subscribe(
        &self,
        session_id: &str,
    ) -> Option<tokio::sync::broadcast::Receiver<operant_core::session_protocol::WireMessage>> {
        Some(self.bus.subscribe(session_id))
    }
}

/// How many messages one session may park for the next turn (W3-2).
///
/// The queue is a `HashMap<session, Vec<String>>` a client can write to over
/// the socket, so without a bound the daemon's memory is the client's to size.
const MAX_QUEUED_PER_SESSION: usize = 32;

/// The default number of agent slots the gateway runs (RT-508).
///
/// One slot per concurrent conversation: a slot is a full agent with its
/// OWN conversation, so a turn in one slot cannot retarget another slot's
/// turn — the corruption the single global lock existed to prevent.
const DEFAULT_AGENT_SLOT_COUNT: usize = 4;

/// The ceiling on the slot count (RT-548).
///
/// A slot is a full agent, so the count is a RAM decision. MEASURED on the
/// operator's box with this build: one daemon at 1 / 4 / 16 / 64 slots is
/// 37 / 43 / 69 / 173 MiB RSS — about **2.1 MiB per idle slot**, not the
/// ~45 MiB an earlier note claimed. The ceiling is therefore set by the file
/// descriptor and scheduler budget a thousand sessions imply, not by RAM, and
/// the honest caveat is that an ACTIVE turn costs more than an idle slot
/// (conversation, compressor and caches) — a number nobody has measured yet.
const MAX_AGENT_SLOT_COUNT: usize = 256;

/// How many slots the gateway runs for this config (RT-548).
///
/// The slot count and the session cap are the SAME number: a cap of 32 with
/// four slots meant 32 sessions were admitted and four ran at a time, silently.
/// The cap is the operator's statement of how many concurrent conversations
/// this gateway serves, so it is the slot count too.
fn slot_count_for(max_concurrent_sessions: Option<usize>) -> usize {
    max_concurrent_sessions
        .unwrap_or(DEFAULT_AGENT_SLOT_COUNT)
        .clamp(1, MAX_AGENT_SLOT_COUNT)
}

/// One agent slot (RT-508).
///
/// The slot owns the agent AND the one-turn-at-a-time lock that used to
/// be global. The lock is per slot because the agent is: two turns in the
/// SAME slot serialize (they share the conversation), and two turns in
/// DIFFERENT slots run concurrently.
struct AgentSlot {
    /// The slot's agent. Its conversation belongs to the sessions that
    /// land in this slot.
    agent: Arc<OperantAgent>,
    /// The agent's recap-engine activity handle (iters 793/794): the
    /// dispatch notifies it on every user message so the idle recap is
    /// cancelled by activity. It belongs to the AGENT, so it belongs to
    /// the slot. `None` = the agent has no recap engine.
    recap_activity: Option<operant_core::recap::RecapActivity>,
    /// The reasoning effort the GATEWAY starts from (RT-535): the
    /// per-session reset restores it, so a chat without an effort of its
    /// own does not inherit the previous chat's.
    default_effort: Option<String>,
    /// ONE turn at a time IN THIS SLOT.
    lock: tokio::sync::Mutex<()>,
    /// The session this slot currently holds (the red-team's finding 7).
    ///
    /// The reload decision belongs to the SLOT: each slot has ONE agent and
    /// ONE conversation, and a session key maps to one slot (a fixed hash),
    /// so the cell belongs with the agent. One shared cell let a slot see
    /// the OTHER slot's session id and decide to reload a conversation it
    /// already had — a redundant rehydrate per turn.
    loaded_session: tokio::sync::Mutex<Option<String>>,
    /// The session this slot is serving RIGHT NOW (RT-544).
    ///
    /// The slot's event forwarder publishes under this session id, so a client
    /// attached to the session sees the turn it asked for. It is set by the
    /// turn before the agent runs and read by the forwarder on every event.
    session_cell: Option<std::sync::Arc<std::sync::Mutex<Option<String>>>>,
}

/// The gateway's agent slots (RT-508).
///
/// The gateway used to run ONE agent for the whole machine, so every
/// chat's turn took the same global lock: a busy chat blocked every other
/// chat, every platform and the cron agent, and the queue wait was
/// measured but never acted on ("an agent per seat — measure queue wait
/// first"). The slots are that upgrade path.
struct AgentSlots {
    slots: Vec<AgentSlot>,
}

impl AgentSlots {
    /// The slot index for a session key.
    ///
    /// DETERMINISTIC, so a session always lands in the same slot and its
    /// conversation is not swapped by another session between turns. The
    /// hash is `DefaultHasher`, which is fixed-seeded — the mapping does
    /// not change between runs, so a session keeps its slot across a
    /// gateway restart.
    fn index_for(&self, session_key: &str) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        session_key.hash(&mut hasher);
        (hasher.finish() as usize) % self.slots.len().max(1)
    }

    /// The slot for a session key.
    fn slot_for(&self, session_key: &str) -> &AgentSlot {
        &self.slots[self.index_for(session_key)]
    }

    /// Every slot, for the drain (RT-508): a stopping gateway interrupts
    /// every running turn, not just one.
    fn iter(&self) -> impl Iterator<Item = &AgentSlot> {
        self.slots.iter()
    }
}

struct GatewayMessageHandler {
    /// The agent slots this gateway runs turns on (RT-508).
    slots: AgentSlots,
    /// `gateway.admins`. A turn from a sender outside this list is not retained
    /// as memory (KB-6).
    admins: Vec<String>,
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
    /// (RT-508) The one-turn-at-a-time lock MOVED into the agent slots:
    /// every chat used to share ONE `Arc<OperantAgent>` whose hot
    /// conversation slot is swapped by `set_session_id`, so two concurrent
    /// turns retarget each other mid-run and one chat's tool results land
    /// in the other's persisted transcript. Each slot now owns an agent
    /// AND its lock, so turns in different slots run concurrently and the
    /// corruption the global lock prevented cannot happen.
    /// The sessions whose turn currently holds a SLOT's lock, and the slot
    /// each holds — the `/stop` attach gate. The interrupt flag is ONE
    /// object per slot, so handing it to a non-running session's command
    /// context would let that session stop someone else's turn.
    ///
    /// (RT-508) It is a MAP, not a single option: the slots run turns
    /// concurrently, so several sessions can be running at once.
    running_session: std::sync::Mutex<std::collections::HashMap<String, usize>>,
    /// Messages QUEUED for the NEXT turn, per session.
    ///
    /// (2026-11-12) A QUEUE is not a STEER. The agent's steer queue is
    /// drained BETWEEN iterations, so a message parked there reaches the
    /// model inside the RUNNING turn — which is why `/queue` used to be an
    /// alias for `/steer` and a "queued" message landed in the turn the
    /// operator was trying to steer. This store waits for the next turn in
    /// the SAME chat.
    next_turn_queue: crate::gateway_commands::NextTurnQueue,
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
    /// (iter-2366) Set for the rest of the process's life the moment a stop
    /// begins, and read at every turn's ADMISSION.
    ///
    /// The drain used to interrupt the running turns and then wait for their
    /// slot locks — but nothing stopped NEW work arriving, and the socket's
    /// accept loop kept serving for the whole 30 s bound. A turn admitted
    /// during the drain queued on the same slot lock the drain was waiting
    /// for, and its first act on acquiring that lock was
    /// `interrupt_flag().reset()` — erasing the drain's interrupt. The drain
    /// then timed out, the process exited, and the newly-admitted turn's
    /// future was dropped mid-await with no interrupted tail committed: the
    /// exact loss the drain exists to prevent. A stopping gateway refuses new
    /// turns and drops ones still queued.
    stopping: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// RT-175 — the configured skills root for `/reload-skills`' rescan.
    skills_root_dir: std::path::PathBuf,
    /// `[skills] extra_roots` — the roots the operator configured beyond the
    /// pool. The handler has no config handle, so the list is captured at
    /// start like `skills_root_dir`; without it the gateway's picker and
    /// `/reload-skills` listed the pool only, while `operant doctor` and the
    /// in-loop tools saw every root.
    skills_extra_roots: Vec<std::path::PathBuf>,
    /// `[skills] autoload` captured at gateway start — the off switch the
    /// operator set. `/reload-skills` re-reads roots to repair a stale
    /// manager; it must not resurrect the skill surface the operator turned
    /// off.
    skills_autoload: bool,
    /// iter-1720 — skill-plugin `skills/` dirs captured at gateway start
    /// (the handler has no config handle, same reason as
    /// `summary_ttl_minutes`). `/reload-skills` re-adds them so a rescan
    /// does not silently drop plugin skills; plugin INSTALL/REMOVE still
    /// needs a gateway restart, matching how plugin tools behave (the
    /// registry is built once at startup).
    #[cfg(feature = "plugins-wasm")]
    wasm_plugin_skill_roots: Vec<std::path::PathBuf>,
    /// `gateway.max_concurrent_sessions`, captured at gateway start (the
    /// handler has no config handle of its own). The session socket enforces
    /// the SAME cap the channel path does — a prompt path that skips it makes
    /// the cap decorative.
    max_concurrent_sessions: Option<usize>,
    /// Where the session cap's lock files live.
    ///
    /// A field, not a call to `default_dir()` at the use site, for two reasons:
    /// production must resolve the operator's data directory, and a TEST must
    /// not — a cap test that writes into the live `~/.operant/sessions/active`
    /// can refuse a real session, and two cap tests running in parallel share
    /// one directory and refuse each other.
    session_tracker_dir: std::path::PathBuf,
}

impl GatewayMessageHandler {
    /// Reset a slot agent's PER-SESSION state before a turn that did not come
    /// from a channel message, then bind it to the session that owns it.
    ///
    /// (A3-gateway H-2) A slot serves many sessions. The channel path clears
    /// the seat, the model override, the reasoning effort, the posture, the
    /// charter and the memory scope on EVERY turn (RT-535, iter-2211, KB-8),
    /// then re-reads that session's own overrides. The socket path did none of
    /// that, so a local turn landing on a slot that last served Telegram ran as
    /// THAT chat's employee, against that seat's memory bank, on that chat's
    /// model -- and nothing in the turn said so.
    ///
    /// The order is the channel path's: clear everything, THEN apply what this
    /// session actually has. A session's own employee binding wins over the
    /// unbound scope.
    ///
    /// `default_effort` is the slot's own configured effort, not a per-session
    /// one -- the channel path passes the same value for the same reason.
    ///
    /// Kept as one method rather than inlined at the call site because the
    /// reset is the kind of rule that gets half-copied: the next entry point
    /// to appear should call this, not re-derive which fields to clear.
    async fn reset_slot_for_session(&self, slot: &AgentSlot, session_id: &str) {
        slot.agent.clear_seat_id();
        slot.agent.clear_model_override();
        slot.agent.set_reasoning_effort(slot.default_effort.clone());
        slot.agent.set_posture_override(None);
        slot.agent.set_charter(None);
        // KB-8: unbound until this session proves it has a seat.
        //
        // A socket client is the local operator -- it has no
        // `platform`/`user_id` triple to derive a sender scope from, and
        // treating it as a stranger would silently move every local turn into
        // a throwaway per-sender bank. The unbound owner scope is `Personal`,
        // which is what the channel path yields for an admin.
        if let Some(p) = slot.agent.memory_provider() {
            p.set_scope(&operant_core::memory_wire::MemoryScope::Personal);
        }
        // A session that carries its own employee binding still runs as that
        // employee, with that seat's bank. `lookup_by_session_id` is the
        // store's own seam -- a socket session is filed under the id the
        // client named.
        let gw_guard = self.gateway.lock().await;
        if let Some(ref gw) = *gw_guard
            && let Some(store) = gw.get_persistent_sessions()
            && let Some(entry) = store.lookup_by_session_id(session_id)
            && let Some(emp) = entry.employee_id.clone()
        {
            slot.agent.set_seat_id(emp.clone());
            if let Some(p) = slot.agent.memory_provider() {
                p.set_scope(&operant_core::memory_wire::MemoryScope::Seat { identity: emp });
            }
        }
    }

    /// Point a SLOT at a session before a turn runs in it — the ONE rule
    /// every turn entry obeys.
    ///
    /// The channel path and the session socket each used to do this their own
    /// way, and only the channel path did it: a prompt over the socket never
    /// retargeted the slot's agent, so `run()` either rehydrated the PREVIOUS
    /// chat's conversation into the turn or minted a fresh `sess_<uuid>` and
    /// wrote the transcript there. MEASURED: a prompt sent to session
    /// `attached` landed as `sess_9d985cd8-…`, so `LoadHistory` for the named
    /// session was empty and a resumed session could not be found.
    ///
    /// Returns whether this CHANGED the slot's session, which is the reload
    /// signal the channel path logs and reports.
    async fn bind_slot_to_session(&self, slot: &AgentSlot, session_id: &str, source: &str) -> bool {
        // The swap itself: the outgoing session's turns go back to the store
        // and the incoming session's transcript is rehydrated in their place
        // (`set_session_id`, agent/events.rs). Without it the agent keeps
        // whatever conversation turned last.
        slot.agent.set_session_id(session_id.to_string());
        let now = chrono::Utc::now().to_rfc3339();
        let _ = slot
            .agent
            .db()
            .save_session(session_id, None, source, &now, &now);
        // The slot's forwarder publishes the turn's events under this id, so
        // it must be written on EVERY turn — a stale cell publishes one
        // chat's events to another chat's subscribers.
        if let Some(cell) = slot.session_cell.as_ref()
            && let Ok(mut held) = cell.lock()
        {
            *held = Some(session_id.to_string());
        }
        let changed = {
            let mut current = slot.loaded_session.lock().await;
            let changed = *current != Some(session_id.to_string());
            if changed {
                *current = Some(session_id.to_string());
            }
            changed
        };
        if changed {
            tracing::info!(session = %session_id, source, "the slot is bound to a session");
        }
        changed
    }

    /// Write the daemon's state file (RT-523).
    ///
    /// Best-effort: a status file that cannot be written must never fail a
    /// turn.
    async fn write_status_file(&self) {
        let (deliveries, failed_deliveries) = {
            let gw = self.gateway.lock().await;
            match gw.as_ref() {
                Some(gw) => gw.delivery_counts().await,
                None => (0, 0),
            }
        };
        let (queue_waits, queue_wait_total_ms, queue_wait_worst_ms) =
            self.queue_waits.lock().await.snapshot();
        let status = operant_core::gateway::status::GatewayStatus {
            updated_at: chrono::Utc::now().to_rfc3339(),
            deliveries: deliveries as u64,
            failed_deliveries: failed_deliveries as u64,
            queue_waits,
            queue_wait_total_ms,
            queue_wait_worst_ms,
            agent_slots: self.slots.iter().count(),
        };
        // The state file sits NEXT TO THE DATABASE it describes. That is
        // the operant home in production (`<home>/database.db`), and it is
        // the test's temp directory in a test — so a test that runs a turn
        // never writes the operator's live state file (RT-536).
        let path = self
            .slots
            .slots
            .first()
            .and_then(|slot| slot.agent.db().path())
            .and_then(|db| {
                db.parent()
                    .map(|dir| dir.join(operant_core::gateway::status::STATUS_FILE_NAME))
            })
            .unwrap_or_else(operant_core::gateway::status::status_file_path);
        if let Err(e) = operant_core::gateway::status::write_status_file(&path, &status) {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "the gateway status file could not be written"
            );
        }
    }

    /// The sessions this daemon can serve right now (the `Hello` answer).
    ///
    /// Read from the SLOTS, which is the one record of what a daemon holds: the
    /// slot's `loaded_session` is written by the same binding that runs the
    /// turn. The set this replaced was insert-only — a client could mint session
    /// ids forever and every `Hello` shipped the whole list.
    ///
    /// `try_lock`, not `lock`: the caller is a synchronous `handle` inside an
    /// async connection task, and a blocking lock there would panic. A slot that
    /// is mid-turn is simply absent from this snapshot.
    fn live_session_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .slots
            .slots
            .iter()
            .filter_map(|slot| slot.loaded_session.try_lock().ok().and_then(|h| h.clone()))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

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
        // 1. One mutation surface at a time — the SLOT's lock, the same
        // lock a turn in that slot takes (RT-508). A command for a chat
        // in another slot is not blocked by this one.
        let slot = self.slots.slot_for(session_key);
        let _turn = slot.lock.lock().await;

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
        slot.agent.set_session_id(session_id.clone());

        // (iter-2582) A command-scoped turn must not inherit the slot's last
        // cron origin. Slots are hash-assigned across session keys
        // (`index_for`), so several chats share one agent; the channel and
        // socket turn paths each stamp the origin at turn start, but this path
        // never did. A `/retry` therefore ran `agent.run()` with whatever
        // chat used the slot last, and a `cron create` in the retried turn
        // wrote THAT chat's platform/chat_id as its delivery target -- so the
        // scheduled result was delivered into a different conversation, which
        // is a cross-chat leak and not merely a lost result.
        //
        // Cleared rather than guessed: a command has no `message` to read a
        // platform/channel from, and inventing one would route results to a
        // chat that never asked. `None` is the cron tool's "local" case, which
        // the scheduler records and does not deliver anywhere.
        slot.agent.set_cron_origin(None);

        // 3. The operation, under the lock, against the invoker's session.
        op(Arc::clone(&slot.agent), session_id).await
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
        if running.contains_key(session_key) {
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
        if !running.contains_key(session_key) && turn_inflight.contains_key(session_key) {
            self.cancelled_queued
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(session_key.to_string());
            true
        } else {
            false
        }
    }

    /// (2026-11-12) True when THIS session's turn is currently live.
    ///
    /// Per-session, NOT per-slot: `running_session` is a MAP (RT-508) so the
    /// slots run turns concurrently, and another chat's turn holding the same
    /// slot must not make this chat's message a steer.
    fn session_is_running(&self, session_key: &str) -> bool {
        self.running_session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(session_key)
    }

    /// True when a `clarify()` question is waiting for an answer in this
    /// channel.
    ///
    /// A LOOKUP, not the interception: `handle()` takes the entry and routes
    /// the message to the tool. This reads the same map WITHOUT consuming it,
    /// so the dispatch loop can tell an ANSWER from a MESSAGE before it
    /// decides what to do with the prose.
    fn has_pending_question_reply(&self, message: &IncomingMessage) -> bool {
        let channel_key = format!("{}:{}", message.platform, message.channel_id);
        let Some(global) = crate::gateway_runner::PENDING_USER_QUESTIONS.get() else {
            return false;
        };
        let Ok(guard) = global.lock() else {
            return false;
        };
        let Some(map) = guard.as_ref() else {
            return false;
        };
        // `try_lock`: the interception holds this lock only for
        // microseconds. A FAILURE means "cannot know", and the safe direction
        // is to answer "a question may be pending" — that sends the message
        // down the normal path to `handle()`'s own interception instead of
        // stealing it with a steer.
        map.try_lock()
            .map(|pending| pending.contains_key(&channel_key))
            .unwrap_or(true)
    }

    /// (iter-2173) A DIRECT prose message while THIS session's turn is live
    /// is a STEER: it reaches the model at the agent's next iteration
    /// boundary.
    ///
    /// Returns the notice to send back, or `None` when the message is NOT a
    /// steer and must fall through to the normal turn path.
    ///
    /// What this path must NOT steal:
    ///
    ///   1. a reply to a pending `clarify()` question. The interception for
    ///      that lives at the TOP of `handle()`, and the turn blocked on the
    ///      question is exactly the turn that makes `session_is_running`
    ///      true — so without this check the ANSWER became a steer and the
    ///      tool timed out (the regression iter-2172 shipped).
    ///   2. a message for a session that is not running: it waits for the
    ///      slot lock and starts a turn of its own.
    ///
    /// Per-session, NOT per-slot: another chat's turn holding the same slot
    /// does not make this chat's message a steer.
    fn steer_prose_into_running_turn(
        &self,
        message: &IncomingMessage,
        session_key: &str,
    ) -> Option<String> {
        if !self.session_is_running(session_key) {
            return None;
        }
        if self.has_pending_question_reply(message) {
            tracing::info!(
                channel = %message.channel_id,
                session_key = %session_key,
                "Gateway prose: a question is pending for this channel — the message is an answer, not a steer"
            );
            return None;
        }
        let slot = self.slots.slot_for(session_key);
        let queue = slot.agent.steer_queue_handle();
        let depth = match queue.try_lock() {
            Ok(mut q) => {
                q.push(message.content.clone());
                q.len()
            }
            Err(_) => {
                return Some(
                    "🧭 The agent is reading its steer queue right now — send it again in a moment. Nothing was lost."
                        .to_string(),
                );
            }
        };
        tracing::info!(
            user = %message.user_id,
            platform = %message.platform,
            channel = %message.channel_id,
            session_key = %session_key,
            depth,
            "Gateway command dispatch: prose steered into the running turn"
        );
        Some(
            "🧭 **Steered** — it reaches the model at its next iteration boundary.\nUse `/queue <text>` to hold a message for the next turn instead."
                .to_string(),
        )
    }

    /// Take the messages this session QUEUED for the next turn (FIFO).
    ///
    /// Read by the turn end so the messages are delivered WITH the turn that
    /// just started, ahead of the draft the operator sent. Read by `/queue`
    /// too (peeked, not taken) so the operator can see the queue.
    fn take_queued_messages(&self, session_key: &str) -> Vec<String> {
        self.next_turn_queue
            .lock()
            .ok()
            .and_then(|mut map| map.remove(session_key))
            .unwrap_or_default()
    }
}

/// Clears `running_session` when the turn ends, on every return path — a
/// stale "running" marker would send every later `/stop` into the void.
struct RunningSessionGuard<'a> {
    running: &'a std::sync::Mutex<std::collections::HashMap<String, usize>>,
    session_key: String,
}
impl Drop for RunningSessionGuard<'_> {
    fn drop(&mut self) {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.session_key);
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

    /// (RT-509) Drain the turn in flight before the process goes away.
    ///
    /// A restart used to abort the turn: the tokio runtime dropped every
    /// spawned turn, so a half-finished tool call left the transcript
    /// claiming the tool never ran, and the operator's next message
    /// started from a lie.
    ///
    /// The drain INTERRUPTS the running turn and waits (bounded) for it to
    /// end, so the agent commits its interrupted tail (the interrupted
    /// marker and the DB flag are written by the turn itself — RT-511)
    /// while the runtime is still alive.
    ///
    /// Only ONE turn runs at a time (the global `turn_lock`), so the
    /// shared interrupt flag can only reach the turn that is actually
    /// running — the same reasoning the `/stop` command uses.
    async fn drain(&self, timeout: Duration) -> DrainOutcome {
        // (iter-2366) Close the door BEFORE opening the drain. Nothing below
        // can complete while new turns keep arriving: an admitted turn queues
        // on the slot lock this function is waiting for, and its first act on
        // getting it is to reset the interrupt that is supposed to end the
        // previous one.
        self.stopping
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // (RT-508) The slots run turns concurrently, so a stopping gateway
        // must drain EVERY slot, not one turn.
        let mut running_slots: Vec<usize> = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.lock.try_lock().is_err() {
                running_slots.push(index);
                // The interrupt is what makes the turn END — and it goes
                // only to the slot that IS running: a slot with no turn
                // must keep its flag clean, because the next turn in it
                // would otherwise start already-interrupted.
                slot.agent.interrupt_flag().trigger();
            }
        }
        if running_slots.is_empty() {
            return DrainOutcome::Idle;
        }
        tracing::info!(
            slots = ?running_slots,
            timeout_secs = timeout.as_secs(),
            "Draining the turns in flight before the gateway stops"
        );
        // Each slot's turn holds its lock for the whole run, so acquiring
        // the lock means the turn ENDED. One deadline for all the slots:
        // the bound keeps a wedged turn from holding the daemon's
        // shutdown hostage.
        let deadline = tokio::time::Instant::now() + timeout;
        for index in running_slots {
            let slot = &self.slots.slots[index];
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                tracing::warn!(slot = index, "The drain bound expired; stopping anyway");
                return DrainOutcome::TimedOut;
            }
            if tokio::time::timeout(remaining, slot.lock.lock())
                .await
                .is_err()
            {
                tracing::warn!(
                    slot = index,
                    "The turn in flight did not end inside the drain bound; stopping anyway"
                );
                return DrainOutcome::TimedOut;
            }
        }
        tracing::info!("The turns in flight were drained");
        DrainOutcome::Drained
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
        // (RT-508) The slot is the unit of concurrency: the lock is the
        // slot's, and the agent is the slot's.
        let slot = self.slots.slot_for(&session_key);
        if slot.lock.try_lock().is_err() {
            queued = true;
            let gw = self.gateway.lock().await;
            if let Some(ref g) = *gw {
                let _ = g.send_typing(&message.platform, &message.channel_id, message.thread_id);
            }
        }
        let _turn = slot.lock.lock().await;

        // (RT-508) The turn runs on ITS slot's agent: this conversation
        // belongs to this slot, so no other chat can retarget it mid-run.
        let agent = &slot.agent;
        let queue_wait = queue_wait_start.elapsed();
        if queued {
            self.queue_waits
                .lock()
                .await
                .record(queue_wait, &session_key);
        }

        let _running_marker = RunningSessionGuard {
            running: &self.running_session,
            session_key: session_key.clone(),
        };
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
            running.insert(session_key.clone(), self.slots.index_for(&session_key));
        }
        // Arm a clean interrupt latch at TURN START. This used to run in
        // the dispatch loop, where a second chat's dispatch could wipe a
        // `/stop` aimed at the RUNNING chat (plan 06A failure case 4).
        // Inside the lock the reset can only ever affect this turn.
        agent.interrupt_flag().reset();
        // Process-global per-turn state moves inside the lock for the same
        // reason (it used to be set in the dispatch closure, where two
        // overlapping chats could interleave their origins).
        // (RT-534) The cron origin rides the TURN, on the slot's agent — a
        // process global would let a concurrent chat's turn overwrite this
        // chat's delivery target between the turn's start and its tool call.
        agent.set_cron_origin(Some(operant_core::tools::cron_tool::CronOrigin {
            platform: message.platform.clone(),
            chat_id: message.channel_id.clone(),
            thread_id: message.thread_id.map(|t| t.to_string()),
        }));

        // Point the agent at THIS session before it runs — the ONE binding
        // every turn entry makes, shared with the session socket's prompt path.
        // Without it the agent keeps whatever conversation turned last and
        // mints a fresh `sess_<uuid>`, so the tool trajectory is orphaned and
        // a restart rehydrates a text-only skeleton.
        //
        // `set_session_id` performs the swap itself: the outgoing session's
        // turns go back to the store and the incoming session's transcript
        // is rehydrated in their place. `clear_history` here would be actively
        // wrong — it discards the addressed session from the store.
        let needs_reload = self
            .bind_slot_to_session(self.slots.slot_for(&session_key), &session_id, "gateway")
            .await;
        // The timestamp a failed turn's explanation is written with. It was
        // computed beside the session binding before that moved into the
        // shared helper, so the stamp stays here where it is read.
        let now = chrono::Utc::now().to_rfc3339();

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

        // The raw operator text is no longer written to the transcript here:
        // the core persists the user message it actually runs on (before the
        // model call), so a second copy would show the operator's message
        // twice. `message.content` is what the query is built from above.

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
            let current_key = agent.current_api_key();
            agent.set_endpoint(
                &endpoint.base_url,
                endpoint.api_key.as_deref().or(current_key.as_deref()),
            );
            agent.set_model(endpoint.model);
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
            if *model != agent.model() {
                tracing::info!(model_override = %model, "Applying model override from session metadata");
                agent.set_model(model.clone());
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
        agent.clear_seat_id();
        // (RT-535) The slot's agent carries the PREVIOUS session's
        // per-session fields. A slot serves several sessions now, so a chat
        // with no override of its own must not inherit the previous chat's
        // model, reasoning effort or charter. The reset happens BEFORE this
        // session's own overrides are read below.
        agent.clear_model_override();
        agent.set_reasoning_effort(slot.default_effort.clone());
        // (iter-2211) the posture is per-session too: the next chat that
        // takes this slot starts from the config, not from this chat's level.
        agent.set_posture_override(None);
        agent.set_charter(None);
        // KB-8: every turn starts in its unbound scope, so a sender with no
        // session row never inherits the previous turn's scope. A bound seat
        // overrides it below.
        let unbound_scope = unbound_scope_for(&self.admins, &message.platform, &message.user_id);
        if let Some(p) = agent.memory_provider() {
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
                        agent.set_seat_id(emp.clone());
                        // (plan 06B) the turn's memory bank follows the
                        // seat binding — inside the turn lock, so a
                        // concurrent chat's seat can never interleave: a
                        // seat's turn recalls and retains in `emp:<id>`
                        // and never sees the personal bank or another
                        // seat's (the owner's unbound chats keep the
                        // personal bank).
                        if let Some(p) = agent.memory_provider() {
                            p.set_scope(&operant_core::memory_wire::MemoryScope::Seat {
                                identity: emp,
                            })
                        }
                    }
                    agent.set_charter(None);
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
            let charter = agent.charter();
            let exclude = [operant_core::org::context_injection::content_hash(
                &message.content,
            )];
            injector.render_section(employee_id, charter.as_deref(), &query, &exclude)
        } else {
            query
        };

        // (2026-11-12) A message the chat QUEUED (channel `/queue`) is
        // delivered WITH this turn, ahead of the draft that just arrived —
        // they queued first. Taken here, under the slot lock, so the next
        // turn in THIS chat picks up whatever is left.
        //
        // The queue is this handler's own store, NOT the agent's steer
        // queue: the agent drains that one BETWEEN iterations, which would
        // inject the message into the running turn instead of the next one.
        let query = {
            let queued = self.take_queued_messages(&session_key);
            if queued.is_empty() {
                query
            } else {
                tracing::info!(
                    session_key = %session_key,
                    depth = queued.len(),
                    "Delivering the chat's queued messages with this turn"
                );
                // Each parked message carries a marker: the merged row is one
                // prompt, so without it neither the operator nor the model can
                // tell what this chat QUEUED from what it just sent. The same
                // merge happens on the TUI side (adapter_types/tui_app.rs).
                let marked: Vec<String> =
                    queued.iter().map(|m| format!("[QUEUED] {}", m)).collect();
                format!("{}\n\n{}", marked.join("\n\n"), query)
            }
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
            agent_session_id = ?agent.session_id(),
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
                                .scope(envelope, agent.run(q))
                                .await
                        }
                        None => agent.run(q).await,
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
            Err(operant_core::error::Error::ContextExhausted { required: 0, .. })
        ) {
            let (before, after) = agent.compact_history_now().await;
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
                //
                // The TRANSCRIPT is the core's job: `agent.run()` persists the
                // user message (before the model call) and every assistant
                // message it produces — the answer, the grace summary, the
                // circuit-breaker notice. The gateway used to write the same
                // two rows again, so every turn showed the operator's message
                // and the bot's answer TWICE in the resumed session.

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
                    let tokens = estimate_tokens(&roll).max(1) as i64; // canonical chars/4 estimate
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

                // (RT-523) The daemon's state file, written on every turn
                // end: the delivery counts and the queue-wait numbers are
                // RAM-only, so after a restart nobody could say how many
                // replies had failed or how many turns had waited.
                self.write_status_file().await;

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

                // The prompt is already in the transcript: the core persists
                // the user message BEFORE the model call, so a failed turn
                // still shows what the operator asked. Writing it again here
                // duplicated it.

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
                let _ = agent
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

/// The cron origin a SESSION SOCKET turn runs under.
///
/// A cron job's delivery target comes from the TURN's origin (RT-534): the
/// agent hands `CronOrigin` to `cron_tool::handle_create`, which writes
/// `origin_platform` / `origin_chat_id` onto the row. The scheduler only
/// delivers when BOTH are non-NULL, and `cron create` with no origin writes
/// NULLs and falls back to `deliver: "local"` -- a job that fires and then
/// throws its result away with a `warn!`.
///
/// The channel turn sets an origin (see its own `set_cron_origin` call). The
/// socket turn did not, so a reminder created through `operant chat --attach`
/// could never be delivered.
///
/// `platform` names a route the cron CONSUMER understands: it publishes on the
/// session bus rather than going through an adapter. There is no
/// `session-socket` adapter in `platform_registry`, so `send_to_platform` would
/// answer "Unknown platform" -- which is why the consumer branches on this
/// string. `chat_id` is the session the turn addressed, exactly the channel an
/// attached client is subscribed to.
fn socket_cron_origin(task_session: &str) -> operant_core::tools::cron_tool::CronOrigin {
    operant_core::tools::cron_tool::CronOrigin {
        platform: "session-socket".to_string(),
        chat_id: task_session.to_string(),
        thread_id: None,
    }
}

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
pub async fn start_gateway(
    app_config: &AppConfig,
    session_socket: Option<std::path::PathBuf>,
) -> Result<String> {
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
    //
    // (iter-2366) NOT HERE. This call used to sit ABOVE the flock, so a
    // REFUSED second daemon still started a supervisor thread: it claimed a
    // kernel run (`holon … --prepare`), then lost the lock, printed the
    // refusal, and the process exited — leaving that run leased with nobody to
    // heartbeat or finish it, plus an orphaned `holon` child. Repeated under
    // the unit's `Restart=always` that is a claim storm against the kernel.
    // The organs belong to a daemon that actually owns the machine, so they
    // start below, once the lock is held.

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

    // (iter-2366) The machine is ours now, so the resident organs may claim
    // work on its behalf. Everything this daemon owns is created below this
    // line; nothing above it survives the refusal path.
    operant_os::organs::spawn_if_configured(app_config);

    init_pending_permissions();

    // (plan 2026-10-28 iter-E, G8) The read-only session API: same daemon,
    // same flock, one axum route group for the OTHER surfaces. The lock is
    // held HERE, so spawning after it means the API shares the daemon's
    // single-instance discipline.
    crate::gateway_api::spawn_if_enabled(&app_config.gateway, &app_config.database_path).await;

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

    // The session bus (RT-544): a client that attaches to a session receives
    // that session's agent events. One broadcast per session, published by the
    // slot's own event forwarder.
    let session_bus = std::sync::Arc::new(SessionBus::new());

    // Create event channel for tool progress previews. RT-544 moved the
    // channel INTO the slot (one per slot), because an event has to know which
    // session it belongs to before it can be published on the bus — and the
    // shared channel told every consumer only "an event happened". The slots
    // merge into the one consumer the rest of the gateway already reads, so
    // nothing downstream changed.
    //
    // (A3-gateway H-3) The merge carried the event ALONE, so the session it
    // came from was lost at exactly the point where it was still known, and
    // the reader below resolved its reply target from one process-global cell.
    // Two chats talking at once overwrote that cell each turn, so a turn's
    // progress, permission prompts and questions were answered on whichever
    // thread had most recently written it. The per-slot channel already knows
    // the session (the bus publish below uses it) -- it just did not pass it
    // on. Carrying `(session, event)` through the merge keeps the one consumer
    // and gives it the identity it needs to resolve the way the background
    // path already does.
    let (event_tx, mut event_rx) = mpsc::channel::<(Option<String>, AgentEvent)>(256);
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
    // (iter-2366) The bus, cloned for the permission receiver so a socket
    // turn's ask can be announced to the client attached to that session.
    let session_bus_for_perm = Some(session_bus.clone());
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
    let seat_budget_store = Arc::new(
        operant_core::org::seat_budgets::SeatBudgetDb::init_org_sibling(&app_config.database_path)?,
    );
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
    // (RT-508) The agent SLOTS: one agent per slot, each with its OWN
    // conversation. The gateway used to run ONE agent for the whole machine,
    // so every chat's turn took the same global lock — a busy chat blocked
    // every other chat, every platform and the cron agent, and the queue
    // wait was measured but never acted on. The slots are that upgrade
    // path, and they share the hoisted pair (memory), the MCP manager (so
    // the MCP CHILDREN are shared) and the permission channel.
    let slot_count = slot_count_for(app_config.gateway.max_concurrent_sessions);
    tracing::info!(slots = slot_count, "the gateway's agent slots");
    let mut slots: Vec<AgentSlot> = Vec::with_capacity(slot_count);
    // (iter-2366) ONE model client for the whole slot pool.
    //
    // Each slot used to build its own `OpenAIClient` inside
    // `create_runtime_agent_with`, and that client owns its own `RateLimiter`
    // (`client.rs:179`) plus two reqwest clients with their own connection
    // pools. So N slots enforced the CONFIGURED provider rate limit
    // independently: an operator who wrote 20 requests a minute got 640 at 32
    // slots, and the limiter protected nothing. The client carries no
    // conversation state, so one instance serves every slot — the same hoist the
    // memory pair already has (BUGS.md D-5).
    //
    // The per-slot `model_client`/`pool_registry` inside the builder is a
    // separate hoist: it changes pool scope per provider, so it is not attempted
    // here.
    let shared_slot_client =
        operant_core::client::OpenAIClient::new(crate::client_config(app_config));
    for slot_index in 0..slot_count {
        // (RT-508 red-team, finding 1) The slot's OWN memory provider: the
        // scope cell is per provider, so two slots cannot swap each other's
        // bank mid-turn.
        let slot_memory = match &memory.1 {
            Some(_) => Some((
                memory.0.clone(),
                slot_memory_provider().map(|provider| {
                    provider as Arc<dyn operant_core::memory_provider::MemoryProvider>
                }),
            )),
            None => None,
        };
        // ONE channel per slot (RT-544). The slot's session is known to the
        // slot (loaded_session), so the forwarder can publish the event on the
        // bus under the session it belongs to — and still merge into the
        // process's single consumer.
        let (slot_event_tx, mut slot_event_rx) = mpsc::channel::<AgentEvent>(256);
        let merge = event_tx.clone();
        let bus = std::sync::Arc::clone(&session_bus);
        let session_cell: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let cell_for_forwarder = std::sync::Arc::clone(&session_cell);
        tokio::spawn(async move {
            while let Some(event) = slot_event_rx.recv().await {
                // Publish under the session this slot is serving, so a client
                // attached to that session sees the event (RT-544).
                let session = cell_for_forwarder.lock().ok().and_then(|held| held.clone());
                if let Some(id) = session.as_deref() {
                    bus.publish(id, event.clone());
                }
                // The single consumer the rest of the gateway reads. The
                // session travels WITH the event (A3-gateway H-3) so that
                // consumer can resolve the reply target per event instead of
                // reading one process-global slot that another chat's turn
                // overwrote.
                if merge.send((session.clone(), event)).await.is_err() {
                    break;
                }
            }
        });
        let mut agent = crate::create_runtime_agent_with_client(
            app_config,
            &app_config.agent,
            None,
            slot_event_tx,
            &mcp_manager,
            &app_config.skills.root_dir,
            None, // gateway has no status bar; the agent keeps its internal registry
            // (iter-2366) ONLY THE FIRST slot spawns the LCM maintenance pair.
            //
            // This was an unconditional `true`, so a 32-slot gateway built 32
            // long-term-consolidation engines and 32 rollup/extractor worker
            // PAIRS — each with its own model client, each able to make LLM
            // calls over the same conversations. That is duplicated spend and
            // duplicated write contention on one memory store, not capacity:
            // the maintenance is a property of the STORE, and every slot writes
            // the same store.
            //
            // The cron agent already passes `false` for exactly this reason, and
            // `create_runtime_agent_with`'s own documentation says the flag must
            // be `false` for any SECOND agent in a process. The slot loop was
            // the one caller that never read that.
            slot_index == 0,
            // BUGS.md D-5 — the hoisted pair; see the comment above. The
            // slot's provider is its OWN (the scope is per provider).
            slot_memory,
            // iter-2366: the ONE client built above the loop, so every slot
            // shares a single rate limiter and connection pool.
            Some(&shared_slot_client),
        )
        .await?;
        let idle = app_config.agent.recap_idle_seconds;
        let gateway_recap_activity = agent.attach_recap_engine(
            std::time::Duration::from_secs(idle),
            app_config.recap.clone(),
            crate::client_config(app_config),
        );
        agent.attach_advisor(app_config.advisor.clone(), crate::client_config(app_config));
        slots.push(AgentSlot {
            agent: Arc::new(
                agent
                    .with_permissions(permission_tx.clone())
                    .with_seat_authority(Some(Arc::clone(&seat_authority))),
            ),
            recap_activity: Some(gateway_recap_activity),
            default_effort: (!app_config.agent.reasoning_effort.is_empty())
                .then(|| app_config.agent.reasoning_effort.clone()),
            lock: tokio::sync::Mutex::new(()),
            loaded_session: tokio::sync::Mutex::new(None),
            session_cell: Some(session_cell),
        });
    }
    let agent_count = slots.len();
    tracing::info!(
        slots = agent_count,
        "The gateway agent slots are ready — each slot runs one conversation at \
         a time, and the slots run concurrently"
    );
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
    // Wire the permission channel so the agent can request approval for
    // dangerous tools (bash, file_write, file_edit). Without this, the
    // agent runs tools silently. (Bug #1 from iter-98 audit.)
    // Clone before the gateway agent consumes the sender. Cron gets the same
    // channel so its permission posture is UNCHANGED by this commit (see below).
    // Cron gets the SAME permission channel (see the posture note below).
    let cron_permission_tx = permission_tx.clone();

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
        slots: AgentSlots { slots },
        admins: app_config.gateway.admins.clone(),
        max_concurrent_sessions: app_config.gateway.max_concurrent_sessions,
        session_tracker_dir: operant_core::active_sessions::ActiveSessionTracker::default_dir(),
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
        running_session: std::sync::Mutex::new(std::collections::HashMap::new()),
        next_turn_queue: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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
        skills_extra_roots: app_config.skills.extra_roots.clone(),
        skills_autoload: app_config.skills.autoload,
        #[cfg(feature = "plugins-wasm")]
        wasm_plugin_skill_roots: crate::plugin_tools::plugin_skill_roots(app_config),
        queue_waits: tokio::sync::Mutex::new(QueueWaitStats::default()),
        stopping: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    gateway = gateway.with_handler(handler.clone());

    // RT-542: the session socket, bound once the handler exists — a prompt on
    // it runs a turn in the slot that owns the session, so the socket needs
    // the slots. It is bound while this process holds the daemon lock, so the
    // socket and the channels live and die together.
    // An owned clone, so the spawned server task can own the path without
    // borrowing `session_socket`, which is still read at the pid-file write below.
    if let Some(socket) = session_socket.clone() {
        let session_handler = std::sync::Arc::new(GatewaySessionHandler {
            handler: Some(std::sync::Arc::clone(&handler)),
            bus: std::sync::Arc::clone(&session_bus),
            socket_inflight: std::sync::Arc::default(),
        });
        tokio::spawn(async move {
            if let Err(e) =
                operant_core::session_server::run_session_socket(session_handler, socket).await
            {
                tracing::error!(error = %e, "the session socket server stopped");
            }
        });
        tracing::info!("the gateway will serve the session socket for attaching clients");
    }

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

    // (RT-521) The ingress channel is BOUNDED. It was unbounded, so a
    // burst (a webhook loop, a flood, a reconnect storm) grew the queue
    // until the process died. The capacity is generous enough for a burst
    // and small enough to fail loudly.
    const INGRESS_CAPACITY: usize = 1024;
    let (message_tx, mut message_rx) = mpsc::channel::<IncomingMessage>(INGRESS_CAPACITY);
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
        let cmd_body = format!(
            r#"{{"commands":{}}}"#,
            telegram_bot_commands(&crate::skill_roots::vendor_skill_command_rows(app_config))
        );
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
        while let Some((event_session, event)) = event_rx.recv().await {
            // (A3-gateway H-3) Resolve the target for THIS event, not from a
            // process-global slot. The session arrives with the event; its
            // pinned channel is what the turn itself recorded at :8281. A
            // second chat's turn writing the old global cell could not move
            // this one's progress, because it is never consulted when the
            // event names its own session. Same resolution order as the
            // background path (`resolve_background_target`): pinned first,
            // the shared slot as the fallback for single-DM setups where
            // both are identical.
            let pinned = match event_session.as_deref() {
                Some(id) if !id.is_empty() => {
                    session_channels_for_events.lock().await.get(id).cloned()
                }
                _ => None,
            };
            let slot_channel = current_channel_for_events.lock().await.clone();
            let target = resolve_turn_channel(event_session.as_deref(), pinned, slot_channel);
            let (platform, channel_id, thread_id) = match target {
                Some((p, c, t)) => (p, c, t),
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

                    // The summary carries TWO different things: the CARD the
                    // operator reads (one block per note) and, for a review
                    // that should continue the run, the marker plus the
                    // machine batch the MODEL must receive. The TUI splits at
                    // the marker (tui/app/agent_events.rs); the gateway sent
                    // the whole string verbatim, so a Telegram user got the
                    // boundary guidance and the raw `<advisory severity="...">`
                    // XML as their card. The card is everything BEFORE the
                    // marker, exactly as the TUI reads it.
                    let marker = operant_core::advisor::CONTINUATION_TAG;
                    // Recaps ride the same BackgroundReview channel with a
                    // "※" prefix; the tag inside a recap must NOT be read as
                    // an advisor boundary. Phoning home a phantom
                    // "continuation produced" warning for every tag mention
                    // in a codebase discussion is exactly the kind of false
                    // alarm that teaches operators to ignore the log.
                    let (card, continuation) = if summary.starts_with('⚑') {
                        match summary.find(marker) {
                            Some(pos) => (
                                summary[..pos].trim_end().to_string(),
                                Some(summary[pos..].trim().to_string()),
                            ),
                            None => (summary.clone(), None),
                        }
                    } else {
                        // Not an advisor summary: deliver verbatim.
                        (summary.clone(), None)
                    };
                    if let Some(batch) = continuation
                        && !batch.is_empty()
                    {
                        // There is no gateway path that submits a follow-up
                        // turn: the agent handle is not in this event loop's
                        // scope (only the TUI's loop can continue a run). Say
                        // so, rather than dropping it silently — an undelivered
                        // continuation is the difference between "the advisor
                        // advised and nothing happened" and "nobody knows".
                        tracing::warn!(
                            session = %session_id,
                            "advisor: a continuation batch was produced but the gateway has no                              turn-submission path — the notes are delivered as a card only"
                        );
                    }

                    match target {
                        Some((platform, channel_id, thread_id)) => {
                            let msg =
                                OutgoingMessage::new(&channel_id, &card).with_thread_id(thread_id);
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
                // (A1-gateway MED-1) Four events the operator can act on used
                // to fall through `_ => {}` and vanish. Each one explains a
                // gap the operator would otherwise read as a stall or a
                // silent substitution:
                //
                //   Compaction*  the turn is working, not hung -- the old
                //                behaviour showed nothing at all while the
                //                context was rebuilt.
                //   Retry*       the request failed and is being reissued;
                //                without this a classified failure looks like
                //                a slow model.
                //   Fallback*    a DIFFERENT model answered. That is a
                //                billing and behaviour fact the operator never
                //                agreed to, and it was invisible.
                AgentEvent::CompactionStarted { tokens_before } => {
                    let line = format!("compacting context ({tokens_before} tokens)");
                    let msg = OutgoingMessage::new(&channel_id, &line)
                        .no_markdown()
                        .with_thread_id(thread_id);
                    let _ = gw_for_events.send_message_return_id(&platform, msg).await;
                }
                AgentEvent::CompactionCompleted {
                    tokens_before,
                    tokens_after,
                    ..
                } => {
                    let line =
                        format!("context compacted: {tokens_before} -> {tokens_after} tokens");
                    let msg = OutgoingMessage::new(&channel_id, &line)
                        .no_markdown()
                        .with_thread_id(thread_id);
                    let _ = gw_for_events.send_message_return_id(&platform, msg).await;
                }
                AgentEvent::RetryScheduled {
                    attempt,
                    max_attempts,
                    reason,
                } => {
                    let line = format!("retry {attempt}/{max_attempts}: {reason}");
                    let msg = OutgoingMessage::new(&channel_id, &line)
                        .no_markdown()
                        .with_thread_id(thread_id);
                    let _ = gw_for_events.send_message_return_id(&platform, msg).await;
                }
                // A tool that FAILED. `ToolComplete` is emitted for the
                // successes the renderer cares about (TTS audio), so a
                // failure had no channel output at all: the operator saw the
                // tool row stop and no reason.
                AgentEvent::ToolError { name, error, .. } => {
                    let line = format!("tool failed: {name}: {error}");
                    let msg = OutgoingMessage::new(&channel_id, &line)
                        .no_markdown()
                        .with_thread_id(thread_id);
                    let _ = gw_for_events.send_message_return_id(&platform, msg).await;
                }
                // A 429 classified mid-turn. Without this the turn looks
                // stalled while the provider is rate-limiting it.
                AgentEvent::RateLimitNotice { retry_after_secs } => {
                    let line = match retry_after_secs {
                        Some(secs) => format!("rate limited, retrying in {secs}s"),
                        None => "rate limited, retrying".to_string(),
                    };
                    let msg = OutgoingMessage::new(&channel_id, &line)
                        .no_markdown()
                        .with_thread_id(thread_id);
                    let _ = gw_for_events.send_message_return_id(&platform, msg).await;
                }
                AgentEvent::ModelFallback { from, to, reason } => {
                    let line = format!("model fallback: {from} -> {to} ({reason})");
                    let msg = OutgoingMessage::new(&channel_id, &line)
                        .no_markdown()
                        .with_thread_id(thread_id);
                    let _ = gw_for_events.send_message_return_id(&platform, msg).await;
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
    let session_channels_for_perm = session_channels.clone();
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

            // Get the channel to send the prompt to -- resolved for THIS ask,
            // not from the process-global slot (A3-gateway H-3).
            //
            // `req.session_id` has been on the request since iter-2366 for
            // exactly this reason (its doc names the failure: one chat's reply
            // approving a tool that runs for a different session). Until now
            // it was read only to tell a socket ask from a channel ask; the
            // TARGET still came from the global cell, which another chat's
            // turn overwrote. A permission prompt for session A landing in
            // chat B is not a cosmetic mix-up: the operator approves a tool
            // they never saw requested, on a turn they are not watching.
            let pinned = match req.session_id.as_deref() {
                Some(id) if !id.is_empty() => {
                    session_channels_for_perm.lock().await.get(id).cloned()
                }
                _ => None,
            };
            let slot_channel = current_channel_for_perm.lock().await.clone();
            let channel_info =
                resolve_turn_channel(req.session_id.as_deref(), pinned, slot_channel);

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
                // (iter-2366) An ATTENDED ask with nobody to ask is DENIED.
                // See `no_channel_verdict` — the old `AllowSession` here let a
                // session-socket turn run an operator's gated tools with no
                // prompt at all.
                if !req.unattended {
                    // (iter-2366) A socket turn's ask now has a route of its own.
                    //
                    // With the session on the request the receiver can tell this
                    // ask apart from a channel's, so instead of denying blind it
                    // stores the ask under `session:<id>`, tells the attached
                    // client which tool call is waiting, and waits 60s — the same
                    // shape as the channel prompt. The client answers with
                    // Approve/Deny carrying its session id, and
                    // `resolve_pending_permission` allows exactly that pairing
                    // while still refusing every channel ask.
                    //
                    // No session on the ask: deny. An ask nobody can answer must
                    // not hang the turn.
                    let ask_session = req.session_id.clone();
                    match ask_session.as_deref() {
                        Some(session) if !session.is_empty() => {
                            let tool = req.tool_name.clone();
                            let tool_call_id = req.tool_id.clone();
                            tracing::info!(
                                session = %session,
                                tool = %tool,
                                "a session-socket turn needs approval — queued for its \
                                 attached client (60s)"
                            );
                            if let Some(bus) = session_bus_for_perm.as_ref() {
                                bus.publish(
                                    session,
                                    operant_core::agent::AgentEvent::Error {
                                        error: format!(
                                            "permission needed for `{tool}` — send \
                                             `/approve {tool_call_id}` or \
                                             `/deny {tool_call_id}` within 60s"
                                        ),
                                    },
                                );
                            }
                            pending_permissions_for_perm
                                .lock()
                                .await
                                .insert(format!("session:{session}"), req);
                            let pending_for_timeout = pending_permissions_for_perm.clone();
                            let timeout_tool = tool;
                            let timeout_session = session.to_string();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(60)).await;
                                let mut map = pending_for_timeout.lock().await;
                                if let Some(request) =
                                    map.remove(&format!("session:{timeout_session}"))
                                {
                                    tracing::warn!(
                                        session = %timeout_session,
                                        tool = %timeout_tool,
                                        "a session permission ask timed out — denying"
                                    );
                                    if request.seat_escalation.is_some() {
                                        resolve_seat_escalation_on_deny(&request);
                                    }
                                    let _ = request
                                        .response_tx
                                        .send(operant_core::agent::ToolPermissionResponse::Deny);
                                }
                            });
                        }
                        _ => {
                            tracing::warn!(
                                tool = %req.tool_name,
                                "No active channel and no session for an ATTENDED permission \
                                 prompt — denying. Auto-approving would grant the operator's own \
                                 tools without the prompt a channel turn would have shown."
                            );
                            let _ = req
                                .response_tx
                                .send(no_channel_verdict(false, &unattended_posture_for_perm));
                        }
                    }
                    continue;
                }
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
    let session_channels_for_uq = session_channels.clone();
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
            // Surface the question to the user -- resolved for the session that
            // ASKED, not from the process-global slot (A3-gateway H-3, the
            // third and last receiver). The request carries `session_id`
            // because the asking tool read it off its own `ToolContext`; the
            // two receivers that could resolve per event were fixed in
            // iter-2528. Until now a question raised inside session A was
            // delivered to whichever chat ran a turn last, and A's turn sat
            // blocked on an answer nobody was shown.
            let pinned = match req.session_id.as_deref() {
                Some(id) if !id.is_empty() => session_channels_for_uq.lock().await.get(id).cloned(),
                _ => None,
            };
            let slot_channel = current_channel_for_uq.lock().await.clone();
            let channel_info =
                resolve_turn_channel(req.session_id.as_deref(), pinned, slot_channel);
            if let Some((platform, channel_id, thread_id)) = channel_info.as_ref() {
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
        // (RT-508) The recap engine belongs to the AGENT, so the handle
        // comes from the session's slot — the dispatch looks it up per
        // turn.
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
                                            } else if cmd_def.name == "skills"
                                                && cmd_args.trim().is_empty()
                                                && offer_skill_picker(
                                                    &gw,
                                                    &platform,
                                                    &msg.channel_id,
                                                    msg.thread_id,
                                                    None,
                                                    &handler_for_stop.skills_root_dir,
                                                    // The SAME roots invoke and
                                                    // search use. This passed the
                                                    // bare vendor list, so a
                                                    // configured extra root was
                                                    // browsable-but-not-invocable
                                                    // in one session.
                                                    &skill_roots_for(&handler_for_stop),
                                                    "",
                                                    0,
                                                )
                                                .await
                                            {
                                                // The channel's skill SEARCH: the picker WAS
                                                // rendered, so the command is fully handled
                                                // here. A platform that cannot render buttons
                                                // (or a skills dir with nothing loadable) falls
                                                // through to the plain-text listing below.
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
                                                    handler_for_stop.slots.slot_for(&session_key).agent.steer_queue_handle(),
                                                ))
                                                // (2026-11-12) The NEXT-turn queue: where
                                                // `/queue` parks a message. Separate from
                                                // the steer queue above — that one is
                                                // drained BETWEEN iterations, so a message
                                                // parked there reaches the model inside the
                                                // RUNNING turn.
                                                .with_next_turn_queue(Some(
                                                    std::sync::Arc::clone(&handler_for_stop.next_turn_queue),
                                                ))
                                                // (2026-11-12) A steer steers a RUNNING
                                                // turn; with no turn running the message
                                                // belongs in the next-turn queue.
                                                .with_turn_running(
                                                    handler_for_stop.session_is_running(&session_key),
                                                );
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
                                            // A skill NAME is a command: the vendor skills
                                            // are published as commands (`/ponytail-audit`
                                            // RUNS the skill), and Telegram's menu spells a
                                            // hyphen as an underscore, so a token that is no
                                            // built-in may still be a skill — try both
                                            // spellings before rejecting (iter-2159).
                                            // Via the ONE helper, so this sees the
                                            // configured extra roots too: a skill
                                            // published only from an extra root is
                                            // still a command.
                                            let vendor_roots =
                                                skill_roots_for(&handler_for_stop);
                                            let instruction = msg
                                                .content
                                                .trim()
                                                .strip_prefix('/')
                                                .and_then(|rest| rest.split_once(char::is_whitespace))
                                                .map(|(_, args)| args.trim())
                                                .unwrap_or("");
                                            let skill_msg = crate::gateway_commands::skill_command_names(token)
                                                .into_iter()
                                                .find_map(|name| {
                                                    operant_core::agent::skill_preprocessing::build_skill_invocation_message_in(
                                                        &app_config_clone.skills.root_dir,
                                                        &name,
                                                        instruction,
                                                        None,
                                                        &vendor_roots,
                                                    )
                                                });
                                            if let Some(injected) = skill_msg {
                                                tracing::info!(
                                                    user = %msg.user_id,
                                                    platform = %platform,
                                                    channel_id = %channel_id,
                                                    token = %token,
                                                    "Gateway dispatch: an unknown token is a vendor skill command — running it"
                                                );
                                                const CHANNEL_SKILL_HEAD_CHARS: usize = 1200;
                                                let total = injected.chars().count();
                                                let shown: String =
                                                    injected.chars().take(CHANNEL_SKILL_HEAD_CHARS).collect();
                                                let mut reply = format!("🧩 Skill `{token}`\n\n{shown}");
                                                if total > CHANNEL_SKILL_HEAD_CHARS {
                                                    reply.push_str(
                                                        "\n…(truncated — send this message to run the skill in full)",
                                                    );
                                                }
                                                gw.send_to_platform(
                                                    &platform,
                                                    OutgoingMessage::new(&msg.channel_id, reply)
                                                        .with_thread_id(msg.thread_id),
                                                )
                                                .await?;
                                                return Ok(true);
                                            }
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

                                 // ── (2026-11-12, iter-2173) A DIRECT message while THIS
                                 // session's turn runs is a STEER ───────────
                                 //
                                 // The operator typed it while the model was
                                 // working; the only useful reading is "change
                                 // what you are doing now". The agent's steer
                                 // queue delivers exactly that at the next
                                 // iteration boundary.
                                 //
                                 // Before this, the message waited for the slot
                                 // lock and then started a SECOND turn — so a
                                 // channel could not steer at all, and queueing
                                 // was the only behaviour it had. On a channel
                                 // there is no Ctrl+Enter, so `/queue` is the
                                 // ONLY way to park a message for the next turn.
                                 //
                                 // The decision — and the two things it must NOT
                                 // steal, a pending question's ANSWER and a
                                 // message for a session that is not running —
                                 // lives in the handler, because this branch sits
                                 // AHEAD of `handle()`'s own interceptions.
                                 if let Some(reply) = handler_for_stop
                                     .steer_prose_into_running_turn(&msg, &session_key)
                                 {
                                     let response = OutgoingMessage::new(&msg.channel_id, reply)
                                         .with_thread_id(msg.thread_id);
                                     gw.send_to_platform(&platform, response).await?;
                                     return Ok(true);
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

                                    // ── 1.8 The skill picker and `$skill` mentions ────────
                                    // A channel has no composer, so the `$` grammar is
                                    // how a chat names a skill. Two shapes reach it:
                                    //
                                    //   * an inline mention ("apply $ponytail here") —
                                    //     expanded into the SAME invocation message the
                                    //     TUI's composer builds, so the skill's content
                                    //     reaches the turn;
                                    //   * a bare `$`, or a name that resolves to nothing —
                                    //     answered by the picker, the utility surface a
                                    //     channel can render (tappable rows + navigation).
                                    // The ONE helper: vendor roots + extra roots.
                                    let skill_vendor_roots = skill_roots_for(&handler_for_stop);
                                    if let Some(marker) = msg.raw.get("skill_callback").cloned() {
                                        let key = format!("{platform}:{}", msg.channel_id);
                                        match skill_tap_action(&marker, &key) {
                                            Some(SkillTapAction::Invoke(name)) => {
                                                let session = derived_gateway_session_id(&session_key);
                                                match operant_core::agent::skill_preprocessing::build_skill_invocation_message_in(
                                                    &handler_for_stop.skills_root_dir,
                                                    &name,
                                                    "",
                                                    Some(session.as_str()),
                                                    &skill_vendor_roots,
                                                ) {
                                                    // The tap IS a request to run the
                                                    // skill: the expansion becomes the turn
                                                    // body and the ordinary turn path below
                                                    // runs it.
                                                    Some(text) => msg.content = text,
                                                    None => {
                                                        let response = OutgoingMessage::new(
                                                            &msg.channel_id,
                                                            format!("Skill `{name}` is no longer loadable."),
                                                        )
                                                        .no_markdown()
                                                        .with_thread_id(msg.thread_id);
                                                        gw.send_to_platform(&platform, response).await?;
                                                        return Ok(true);
                                                    }
                                                }
                                            }
                                            Some(SkillTapAction::Page(page)) => {
                                                // Navigation replaces the picker message IN
                                                // PLACE, so a chat does not collect one
                                                // message per page.
                                                let picker = pending_skill_picker(&key);
                                                let message_id = marker
                                                    .get("message_id")
                                                    .and_then(|m| m.as_i64())
                                                    .map(|m| m.to_string());
                                                let query = picker.as_ref().map(|p| p.query.clone()).unwrap_or_default();
                                                offer_skill_picker(
                                                    &gw,
                                                    &platform,
                                                    &msg.channel_id,
                                                    msg.thread_id,
                                                    message_id.as_deref(),
                                                    &handler_for_stop.skills_root_dir,
                                                    &skill_vendor_roots,
                                                    &query,
                                                    page,
                                                )
                                                .await;
                                                return Ok(true);
                                            }
                                            Some(SkillTapAction::Cancel) => {
                                                // The pending entry is already gone
                                                // (`skill_tap_action` consumed it); clear
                                                // the buttons so a dismissed picker cannot
                                                // be re-tapped.
                                                if let Some(adapter) = gw.adapter_for(&platform)
                                                    && let Some(chat) =
                                                        marker.get("chat_id").and_then(|c| c.as_str())
                                                    && let Some(mid) =
                                                        marker.get("message_id").and_then(|m| m.as_i64())
                                                {
                                                    let _ = adapter
                                                        .edit_message(
                                                            chat,
                                                            &mid.to_string(),
                                                            &OutgoingMessage::new(
                                                                chat,
                                                                "🧩 Picker dismissed.",
                                                            )
                                                            .no_markdown(),
                                                        )
                                                        .await;
                                                }
                                                return Ok(true);
                                            }
                                            None => {
                                                // The tap names no live picker: it
                                                // expired, or it belongs to a
                                                // picker this chat has since
                                                // replaced. Edit the tapped
                                                // message so its buttons die
                                                // with it, and offer a fresh
                                                // picker rather than leaving a
                                                // dead button.
                                                if let Some(adapter) = gw.adapter_for(&platform)
                                                    && let Some(chat) = marker
                                                        .get("chat_id")
                                                        .and_then(|c| c.as_str())
                                                    && let Some(mid) = marker
                                                        .get("message_id")
                                                        .and_then(|m| m.as_i64())
                                                {
                                                    let _ = adapter
                                                        .edit_message(
                                                            chat,
                                                            &mid.to_string(),
                                                            &OutgoingMessage::new(
                                                                chat,
                                                                "🧩 This picker was replaced — here is the current one.",
                                                            )
                                                            .no_markdown(),
                                                        )
                                                        .await;
                                                }
                                                offer_skill_picker(
                                                    &gw,
                                                    &platform,
                                                    &msg.channel_id,
                                                    msg.thread_id,
                                                    None,
                                                    &handler_for_stop.skills_root_dir,
                                                    &skill_vendor_roots,
                                                    "",
                                                    0,
                                                )
                                                .await;
                                                return Ok(true);
                                            }
                                        }
                                    } else {
                                        // The prose path: a message that NAMES a skill is expanded into
                                        // the invocation message the agent follows; a bare `$` (or a name
                                        // that resolves to nothing) asks for the picker.
                                        match skill_surface_for(&handler_for_stop, &session_key, &msg.content) {
                                            SkillSurface::Invoke(text) => msg.content = text,
                                            SkillSurface::Picker(query) => {
                                                let offered = offer_skill_picker(
                                                    &gw,
                                                    &platform,
                                                    &msg.channel_id,
                                                    msg.thread_id,
                                                    None,
                                                    &handler_for_stop.skills_root_dir,
                                                    &skill_vendor_roots,
                                                    &query,
                                                    0,
                                                )
                                                .await;
                                                if offered {
                                                    return Ok(true);
                                                }
                                                // No picker possible: the raw text goes to the agent, which
                                                // can say what it knows.
                                            }
                                            SkillSurface::None => {}
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
                                        // (RT-521) The per-session queue is BOUNDED: a
                                        // chat that floods while its turn is busy used
                                        // to grow the queue without limit. The drop is
                                        // visible — a dropped chat message is a real
                                        // loss, so it is logged, not silent.
                                        if !queue_has_room(&turn_queue, &session_key) {
                                            tracing::warn!(
                                                session = %session_key,
                                                limit = MAX_QUEUED_MESSAGES_PER_SESSION,
                                                "the session's message queue is FULL — the message \
                                                 is dropped"
                                            );
                                            continue;
                                        }
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
                                    // (RT-508) The interrupt flag belongs to the
                                    // session's slot's agent.
                                    let stop_flag = handler_for_stop
                                        .slots
                                        .slot_for(&session_key)
                                        .agent
                                        .interrupt_flag();
                                    turn_inflight.insert(session_key.clone(), stop_flag);
                                    let gw = gw_for_turns.clone();
                                    let app_config_clone = app_config_for_turns.clone();
                                    let stream_delivery_for_dispatch = stream_delivery_for_turns.clone();
                                    let current_channel = current_channel_for_turns.clone();
                                    let telegram_token = telegram_token_for_turns.clone();
                                    let done_tx = turn_done_tx.clone();
                                    // (RT-508) The recap engine belongs to the AGENT,
                                    // so the handle is the session's SLOT's.
                                    let recap_activity_for_turn = handler_for_stop
                                        .slots
                                        .slot_for(&session_key)
                                        .recap_activity
                                        .clone();
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
                                    // Replace is tool-seam semantics; the message
                                    // pipeline has nothing to replace here.
                                    PipelineAction::Replace(_) => {}
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
                                if let Some(activity) = &recap_activity_for_turn {
                                    activity.notify();
                                }
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
                                    handler_for_turn
                                        .slots
                                        .slot_for(&session_key)
                                        .agent
                                        .set_api_key(key);
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
                                    let current = handler_for_turn
                                        .slots
                                        .slot_for(&session_key)
                                        .agent
                                        .base_url();
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
                                            .or_else(|| {
                                    handler_for_turn
                                        .slots
                                        .slot_for(&session_key)
                                        .agent
                                        .current_api_key()
                                });
                                        handler_for_turn
                                    .slots
                                    .slot_for(&session_key)
                                    .agent
                                    .set_endpoint(base_url, key.as_deref());
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
                                        // (RT-521) The reinject WAITS for room: a queued
                                        // chat message must not be dropped because the
                                        // ingress queue was momentarily full.
                                        if let Err(e) = message_tx_for_reinject.send(msg).await {
                                            tracing::warn!(
                                                error = %e,
                                                "a queued message could not be re-injected into the \
                                                 ingress queue — it is dropped"
                                            );
                                        }
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
            .with_retry(app_config.agent.retry.clone())
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
        // (A3-gateway H-1, the other half) A cron job created from a SESSION
        // SOCKET turn carries `platform: "session-socket"` and the session
        // id as its chat id. That platform has no adapter in the registry —
        // `send_to_platform` would answer "Unknown platform" and the result
        // would be just as dead as before. What those jobs need is the
        // session bus, the same channel a turn's own events ride, so an
        // attached client receives the reminder as an event.
        let bus_for_cron = std::sync::Arc::clone(&session_bus);
        // v6 outline row 5: the sender loop is the `CronDeliveryOutcome`
        // emitter — the one cron lifecycle event core never emits (the
        // consumer-side report contract, iter-720). Same observer the
        // scheduler holds, so the quartet (Queued/Parked/Reclaimed/Outcome)
        // lands on one backend.
        let cron_observer_sender = Arc::clone(&cron_observer);
        tokio::spawn(async move {
            while let Some(delivery) = cron_rx.recv().await {
                // A socket-originated delivery bypasses the adapter path
                // entirely. `platform` is a plain string chosen by
                // `set_cron_origin`, so the scheduler cannot know it means
                // "publish on this session's bus" — the consumer has to.
                let send_result = if delivery.platform == "session-socket" {
                    // `Content`, not `Error`: this is a delivered reminder,
                    // and rendering it as a failure card would tell the
                    // operator their job failed when it worked.
                    bus_for_cron.publish(
                        &delivery.chat_id,
                        AgentEvent::Content {
                            text: delivery.content.clone(),
                        },
                    );
                    Ok(())
                } else {
                    let msg = OutgoingMessage::new(&delivery.chat_id, &delivery.content);
                    gw_for_cron.send_to_platform(&delivery.platform, msg).await
                };
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
    // (iter-2366) Record the socket this daemon actually bound.
    //
    // `gateway run --socket PATH` moves the socket alone: the client recomputes
    // the DEFAULT path and `gateway status` printed only the default, so a
    // daemon started on a non-default path was UNDISCOVERABLE — status said
    // "(not bound)" while it was serving, and an operator had no way to learn
    // the path to pass to `--attach`. The resolved path is a sidecar next to
    // the pid file, so a reader that knows where the pid file is can find it.
    let _ = std::fs::write(
        home_file(),
        operant_core::platform::operant_home().display().to_string(),
    );
    if let Some(socket) = session_socket.as_ref() {
        let _ = std::fs::write(socket_path_file(), socket.display().to_string());
    }

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
/// How long a drain waits for the turn in flight to end (RT-509).
///
/// Below systemd's default `TimeoutStopSec` (90 s), so a drain never
/// makes the unit look like it failed to stop.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn stop_gateway() -> Result<String> {
    // Take the gateway out of the slot FIRST, so a concurrent status
    // check does not block for the length of the drain.
    let gateway = {
        let mut guard = runner().lock().await;
        guard.take()
    };

    match gateway {
        Some(gateway) => {
            // (RT-509) Drain BEFORE the adapters stop and before the
            // runtime drops the spawned turns: the turn in flight is
            // interrupted and given time to END, so its transcript is
            // complete and the next start can offer the resume. Without
            // this, a restart aborted the turn mid-tool.
            let outcome = gateway.drain(DRAIN_TIMEOUT).await;
            if !outcome.is_drained() {
                tracing::warn!(
                    ?outcome,
                    "The gateway is stopping with a turn still in flight"
                );
            }
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
            // (iter-2366) The session this ask belongs to.
            session_id: None,
            response_tx: tx,
        }
    }

    /// iter-2364: the socket may not approve a CHANNEL's pending tool ask.
    ///
    /// The pending map is keyed by channel and only channel turns write it, so
    /// matching by tool-call id alone gave any local process the authority to
    /// approve a tool waiting for a human on Telegram.
    #[tokio::test]
    async fn a_channel_owned_approval_is_refused_on_the_socket() {
        let _guard = permissions_test_guard();
        let store = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let (tx, _rx) = tokio::sync::oneshot::channel();
        store.lock().await.insert(
            "telegram:42".to_string(),
            ToolPermissionRequest {
                // (iter-2366) The session this ask belongs to.
                session_id: None,
                response_tx: tx,
                ..make_request()
            },
        );
        store_pending_permissions(store.clone());

        assert!(
            !resolve_pending_permission("t-1", true, None),
            "a channel's pending ask must not be approvable over the socket"
        );
        // (iter-2366) A SESSION ask is the mirror image: approvable by the
        // session it belongs to, and by no other. Reuses the store above, which
        // the receiver would hold either way.
        let (tx2, _rx2) = tokio::sync::oneshot::channel();
        store.lock().await.insert(
            "session:s1".to_string(),
            ToolPermissionRequest {
                session_id: Some("s1".to_string()),
                tool_id: "t-session".to_string(),
                response_tx: tx2,
                ..make_request()
            },
        );

        assert!(
            !resolve_pending_permission("t-session", true, Some("s2")),
            "a session's ask must not be approvable by a DIFFERENT session"
        );
        assert!(
            resolve_pending_permission("t-session", true, Some("s1")),
            "the session an ask belongs to must be able to approve it"
        );
        assert!(
            !resolve_pending_permission("t-session", true, None),
            "and a client with no session must not"
        );
        // The ask was consumed by the one legitimate approval.
        assert!(
            store.lock().await.get("session:s1").is_none(),
            "a resolved ask must leave the pending map"
        );
        // Re-insert so the channel assertion below still has its ask.
        let (tx3, _rx3) = tokio::sync::oneshot::channel();
        store.lock().await.insert(
            "telegram:42".to_string(),
            ToolPermissionRequest {
                session_id: None,
                response_tx: tx3,
                ..make_request()
            },
        );

        // And it is still there for the channel to answer.
        let still = store
            .lock()
            .await
            .get("telegram:42")
            .map(|r| r.tool_id.clone());
        assert_eq!(
            still.as_deref(),
            Some("t-1"),
            "the refusal must not consume the request"
        );
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
            // (iter-2366) The session this ask belongs to.
            session_id: None,
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

    /// The todo tool's payload is a JSON ARRAY of item objects, so
    /// `extract_tool_arg` can never pull a string out of it and the line used
    /// to read as a bare `⏰ todo...`. Counts by status are the preview, and
    /// the item text stays `/todos`' job.
    ///
    /// MUTATION PROVEN: deleting the `.or_else(|| todo_progress_summary(args))`
    /// term in `tool_preview_line` turns this test red — the line falls back
    /// to `⏰ todo...` with nothing after it, exactly the reported gap.
    #[test]
    fn a_todo_call_previews_its_counts_not_a_bare_dots_line() {
        let args = serde_json::json!({
            "todos": [
                { "id": "1", "content": "write tests", "status": "completed" },
                { "id": "2", "content": "fix bug", "status": "in_progress" },
                { "id": "3", "content": "ship", "status": "pending" },
            ]
        })
        .to_string();
        let line = tool_preview_line("todo", &args);
        assert!(line.contains("3 tasks"), "the total must show: {line}");
        assert!(line.contains("1 done"), "done count: {line}");
        assert!(line.contains("1 active"), "active count: {line}");
        assert!(line.contains("1 to do"), "pending count: {line}");
        assert!(
            !line.ends_with("..."),
            "the fallback ellipsis must be replaced by real content: {line}"
        );
    }

    /// An item with no `status` field counts as `pending`, which is what the
    /// tool normalises it to anyway — not as an extra unlabelled bucket.
    #[test]
    fn a_statusless_todo_item_counts_as_pending() {
        let args = serde_json::json!({
            "todos": [{ "id": "1", "content": "no status here" }]
        })
        .to_string();
        let line = tool_preview_line("todo", &args);
        assert!(line.contains("1 to do"), "it is pending: {line}");
        assert!(line.contains("1 tasks"), "and the total is still 1: {line}");
    }

    /// A cancelled item is counted on its own line rather than folded into
    /// `pending`, because "1 dropped" tells the operator something `pending`
    /// does not.
    #[test]
    fn a_cancelled_todo_item_is_counted_separately() {
        let args = serde_json::json!({
            "todos": [
                { "id": "1", "content": "a", "status": "cancelled" },
                { "id": "2", "content": "b", "status": "completed" },
            ]
        })
        .to_string();
        let line = tool_preview_line("todo", &args);
        assert!(line.contains("1 dropped"), "cancelled shows: {line}");
        assert!(
            !line.contains("to do"),
            "and is not miscounted as pending: {line}"
        );
    }

    /// A call that clears the list (an empty `todos` array) has nothing to
    /// summarise, so the line falls back to the ellipsis rather than claiming
    /// `0 tasks` — which would be a lie, since the call is a WRITE.
    #[test]
    fn an_empty_todo_payload_keeps_the_fallback_line() {
        let line = tool_preview_line("todo", r#"{"todos":[]}"#);
        assert!(
            line.ends_with("..."),
            "nothing to count means nothing to claim: {line}"
        );
        assert!(!line.contains("0 tasks"), "nor a false zero: {line}");
    }

    /// A `todos` key on some OTHER tool's payload must not be read as a plan:
    /// the summary is gated on the tool name, so an unrelated call keeps its
    /// own argument unchanged.
    #[test]
    fn a_todos_key_on_another_tool_is_not_read_as_a_plan() {
        let args = serde_json::json!({
            "command": "ls",
            "todos": [{ "id": "1", "content": "unrelated", "status": "pending" }]
        })
        .to_string();
        let line = tool_preview_line("bash", &args);
        assert!(line.contains("ls"), "the command still previews: {line}");
        assert!(
            !line.contains("tasks"),
            "and the todo shape is not re-read here: {line}"
        );
    }

    /// The model's display intent still leads the line when it supplied one:
    /// the counts are the technical half, not a replacement for the intent.
    ///
    /// MUTATION PROVEN, and for the DIRECTION of the fix: written first as an
    /// `.or_else` ("fill only a gap"), this test was red — the extractor's
    /// "first string value" fallback had already returned the intent itself,
    /// so the counts never reached the line. The summary must WIN.
    #[test]
    fn an_intent_still_leads_a_todo_line_with_counts() {
        let args = serde_json::json!({
            "intent": "plan the migration",
            "todos": [
                { "id": "1", "content": "a", "status": "pending" },
                { "id": "2", "content": "b", "status": "completed" },
            ]
        })
        .to_string();
        let line = tool_preview_line("todo", &args);
        assert!(
            line.contains("plan the migration"),
            "the intent must not be displaced: {line}"
        );
        assert!(
            line.contains("2 tasks"),
            "counts still show beside it: {line}"
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
