//! Helpers, tunables, attachment parsing, and poll-recovery state extracted
//! verbatim from the former telegram.rs monolith.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Telegram's maximum message length for text messages
pub(crate) const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;
/// Reserve space for continuation markers added by send_text_chunks:
/// worst case is "(continued)\n\n" + chunk + "\n\n(continues...)" = 30 extra chars
pub(crate) const TELEGRAM_CONTINUATION_OVERHEAD: usize = 30;
pub(crate) const TELEGRAM_ACK_REACTIONS: &[&str] = &["⚡️", "👌", "👀", "🔥", "👍"];

/// Metadata for an incoming document or photo attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IncomingAttachment {
    pub(crate) file_id: String,
    pub(crate) file_name: Option<String>,
    pub(crate) file_size: Option<u64>,
    pub(crate) caption: Option<String>,
    pub(crate) kind: IncomingAttachmentKind,
}

/// The kind of incoming attachment (document vs photo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IncomingAttachmentKind {
    Document,
    Photo,
}
pub(crate) const TELEGRAM_BIND_COMMAND: &str = "/bind";
/// Telegram Bot API allows at most 100 commands via setMyCommands.
pub(crate) const TELEGRAM_MAX_BOT_COMMANDS: usize = 100;
/// Telegram command names: 1-32 lowercase a-z, 0-9, and underscore.
pub(crate) const TELEGRAM_COMMAND_NAME_MAX_LEN: usize = 32;
/// Telegram command descriptions nominally allow up to 256 characters per the API docs,
/// but empirical testing shows the API returns errors for descriptions substantially
/// longer than 100 characters. This conservative cap avoids that in practice.
pub(crate) const TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN: usize = 100;

// ── T4 polling-resilience constants (hermes `_polling_heartbeat_loop`,
// `_probe_pending_updates`, `_verify_polling_after_reconnect` parity) ────────
/// Seconds between `getMe` connectivity probes on the general path. Catches
/// dead TCP sockets (CLOSE-WAIT) that a long-poll waiting for the 30-second
/// Telegram window never surfaces. Hermes default: 90.
pub(crate) const POLLING_HEARTBEAT_INTERVAL_SECS: u64 = 90;
/// Per-probe deadline for the heartbeat `getMe` before the path is declared
/// suspect. Hermes default: 15s.
pub(crate) const POLLING_HEARTBEAT_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// Seconds between `getWebhookInfo` pending-update probes. Hermes schedules
/// these alongside the heartbeat.
pub(crate) const POLLING_PENDING_PROBE_INTERVAL_SECS: u64 = 60;
/// A pending_update_count at or above this while we believe we are polling
/// marks a probe as "stuck" (updates are queuing server-side but the consumer
/// is not draining them).
pub(crate) const POLLING_PENDING_STUCK_THRESHOLD: i64 = 2;
/// Consecutive stuck probes required before escalating to a polling restart,
/// so a single in-flight update between probes never trips a needless
/// recovery. Hermes: 2.
pub(crate) const POLLING_PENDING_STUCK_STRIKES: u32 = 2;
/// Client-side ceiling for one long-poll getUpdates request. Telegram holds
/// the request up to the API `timeout` (30s), so 45s only fires on a wedged
/// socket that the API-level timeout cannot surface.
pub(crate) const POLL_CLIENT_TIMEOUT_SECS: u64 = 45;
/// Minimum seconds between recovery triggers, so a wedged heartbeat and a
/// wedged consumer probing at the same time collapse into one restart.
pub(crate) const POLL_RECOVERY_DEBOUNCE_SECS: u64 = 30;
/// Backoff after a confirmed client-side poll timeout (wedged socket).
pub(crate) const POLL_WEDGE_BACKOFF_SECS: u64 = 5;

/// Classification of a getUpdates failure (hermes `_looks_like_network_error`
/// parity). `Fatal` errors must not churn the polling loop; `Conflict` means
/// another process holds the getUpdates slot; `Network` is transient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollErrorClass {
    Fatal,
    Conflict,
    Network,
}

impl PollErrorClass {
    pub(crate) fn from_error_code(error_code: i64) -> Self {
        match error_code {
            // Unauthorized / forbidden: the token itself is rejected. Retrying
            // cannot help; churn would just spam the API (hermes: auth errors
            // are not `_looks_like_network_error`).
            401 | 403 => PollErrorClass::Fatal,
            // Another process holds the getUpdates slot (terminated by other
            // getUpdates request).
            409 => PollErrorClass::Conflict,
            // Everything else (429 rate limit, 5xx, unknown) is transient.
            _ => PollErrorClass::Network,
        }
    }
}

/// Shared state between `listen()` and the watchdog loops (heartbeat +
/// pending-update probe). Cloneable so a spawned watchdog task can trigger
/// recovery without borrowing the whole channel.
#[derive(Clone)]
pub(crate) struct PollRecoveryState {
    /// Generation counter. `trigger()` bumps it; `listen()` subscribes a watch
    /// receiver and abandons the in-flight long-poll the moment it bumps.
    pub(crate) generation: tokio::sync::watch::Sender<u64>,
    /// Permanent receiver keeping the channel alive. tokio watch only stores a
    /// sent value when at least one receiver exists, so without this a watchdog
    /// firing before `listen()`'s first subscription would silently lose the
    /// bump (and the debounce timestamp would still be consumed).
    pub(crate) _generation_keep_alive: tokio::sync::watch::Receiver<u64>,
    /// Last observed `pending_update_count` (probe loop).
    pub(crate) last_pending_count: Arc<parking_lot::Mutex<i64>>,
    /// Consecutive stuck-probe strikes (probe loop).
    pub(crate) pending_stuck_strikes: Arc<parking_lot::Mutex<u32>>,
    /// Instant of the last recovery trigger, for debouncing.
    pub(crate) last_recovery_at: Arc<parking_lot::Mutex<std::time::Instant>>,
}

impl PollRecoveryState {
    pub(crate) fn new() -> Self {
        let (generation, _generation_keep_alive) = tokio::sync::watch::channel(0);
        Self {
            generation,
            _generation_keep_alive,
            last_pending_count: Arc::new(parking_lot::Mutex::new(0)),
            pending_stuck_strikes: Arc::new(parking_lot::Mutex::new(0)),
            // Initialized in the past so the first trigger is never debounced.
            last_recovery_at: Arc::new(parking_lot::Mutex::new(
                std::time::Instant::now() - Duration::from_secs(POLL_RECOVERY_DEBOUNCE_SECS + 1),
            )),
        }
    }

    /// Debounced recovery trigger (hermes `_schedule_polling_recovery`
    /// parity). Repeated triggers inside the debounce window collapse into
    /// one bump, so a wedged heartbeat and a wedged consumer probing at the
    /// same time restart the poll loop exactly once.
    pub(crate) fn trigger(&self, reason: &str) {
        let now = std::time::Instant::now();
        {
            let mut last = self.last_recovery_at.lock();
            if now.duration_since(*last) < Duration::from_secs(POLL_RECOVERY_DEBOUNCE_SECS) {
                tracing::debug!("Telegram poll recovery debounced: {reason}");
                return;
            }
            *last = now;
        }
        tracing::warn!("Telegram polling recovery triggered: {reason}");
        // NB: read the current value into a local first — `send()` takes the
        // channel's write lock, so holding the `Ref` from `borrow()` across
        // the call would deadlock (read lock + write lock).
        let next = self.generation.borrow().wrapping_add(1);
        let _ = self.generation.send(next);
    }
}

/// Pure escalation rule for the pending-update probe (hermes
/// `_probe_pending_updates` 2-strike parity): a probe observing a queue at or
/// above `threshold` counts a strike; `max_strikes` consecutive strikes return
/// `true` (escalate). Any healthy probe resets the strikes.
pub(crate) fn probe_pending_escalate(
    pending: i64,
    strikes: &mut u32,
    threshold: i64,
    max_strikes: u32,
) -> bool {
    if pending >= threshold {
        *strikes = strikes.saturating_add(1);
        if *strikes >= max_strikes {
            *strikes = 0;
            return true;
        }
    } else {
        *strikes = 0;
    }
    false
}

/// Host component of an API base URL (default `api.telegram.org`), used as the
/// resolution target when fallback IPs are configured.
pub(crate) fn api_host_of(base: &str) -> &str {
    base.strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(base)
}

/// Build the Telegram API client for one request, pinning the API host to a
/// configured fallback IP (rotated round-robin) when `fallback_ips` is
/// non-empty, else the regular proxied channel client.
pub(crate) fn build_telegram_api_client(
    api_base: &str,
    proxy_url: Option<&str>,
    fallback_ips: &[String],
    fallback_ip_index: &parking_lot::Mutex<usize>,
) -> reqwest::Client {
    if fallback_ips.is_empty() {
        return operant_config::schema::build_channel_proxy_client("channel.telegram", proxy_url);
    }
    let idx = {
        let mut index = fallback_ip_index.lock();
        let current = *index;
        *index = (*index + 1) % fallback_ips.len();
        current
    };
    let ip = &fallback_ips[idx];
    operant_config::schema::build_channel_proxy_client_resolved(
        "channel.telegram",
        proxy_url,
        api_host_of(api_base),
        ip,
    )
}

/// Sanitize a skill name into a valid Telegram command name.
/// Telegram commands must be 1-32 characters, lowercase a-z, 0-9, underscore only.
pub(crate) fn sanitize_telegram_command_name(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() {
            result.push(lower);
        } else if !result.ends_with('_') {
            // Replace non-alphanumeric with underscore, collapsing consecutive runs.
            result.push('_');
        }
    }

    let trimmed = result.trim_matches('_');
    if trimmed.len() <= TELEGRAM_COMMAND_NAME_MAX_LEN {
        trimmed.to_string()
    } else {
        trimmed[..TELEGRAM_COMMAND_NAME_MAX_LEN]
            .trim_end_matches('_')
            .to_string()
    }
}

/// Truncate a description to the conservative `TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN` cap.
/// The API nominally supports 256 characters, but empirical testing shows errors occur
/// for descriptions substantially longer than 100 characters.
pub(crate) fn truncate_telegram_command_description(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN {
        return trimmed.to_string();
    }
    let mut truncated: String = trimmed
        .chars()
        .take(TELEGRAM_COMMAND_DESCRIPTION_MAX_LEN - 1)
        .collect();
    truncated.push('…');
    truncated
}

/// Split a message into chunks that respect Telegram's 4096 character limit.
/// Tries to split at word boundaries when possible, and handles continuation.
/// The effective per-chunk limit is reduced to leave room for continuation markers.
pub(crate) fn split_message_for_telegram(message: &str) -> Vec<String> {
    if message.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return vec![message.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = message;
    let chunk_limit = TELEGRAM_MAX_MESSAGE_LENGTH - TELEGRAM_CONTINUATION_OVERHEAD;

    while !remaining.is_empty() {
        // If the remainder fits within the full limit, take it all (last chunk
        // or single chunk — continuation overhead is at most 14 chars).
        if remaining.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
            chunks.push(remaining.to_string());
            break;
        }

        // Find the byte offset for the Nth character boundary.
        let hard_split = remaining
            .char_indices()
            .nth(chunk_limit)
            .map_or(remaining.len(), |(idx, _)| idx);

        let chunk_end = if hard_split == remaining.len() {
            hard_split
        } else {
            // Try to find a good break point (newline, then space)
            let search_area = &remaining[..hard_split];

            // Prefer splitting at newline
            if let Some(pos) = search_area.rfind('\n') {
                // Don't split if the newline is too close to the start
                if search_area[..pos].chars().count() >= chunk_limit / 2 {
                    pos + 1
                } else {
                    // Try space as fallback
                    search_area.rfind(' ').unwrap_or(hard_split) + 1
                }
            } else if let Some(pos) = search_area.rfind(' ') {
                pos + 1
            } else {
                // Hard split at character boundary
                hard_split
            }
        };

        chunks.push(remaining[..chunk_end].to_string());
        remaining = &remaining[chunk_end..];
    }

    chunks
}

pub(crate) fn pick_uniform_index(len: usize) -> usize {
    debug_assert!(len > 0);
    let upper = len as u64;
    let reject_threshold = (u64::MAX / upper) * upper;

    loop {
        let value = rand::random::<u64>();
        if value < reject_threshold {
            #[allow(clippy::cast_possible_truncation)]
            return (value % upper) as usize;
        }
    }
}

pub(crate) fn random_telegram_ack_reaction() -> &'static str {
    TELEGRAM_ACK_REACTIONS[pick_uniform_index(TELEGRAM_ACK_REACTIONS.len())]
}

pub(crate) fn build_telegram_ack_reaction_request(
    chat_id: &str,
    message_id: i64,
    emoji: &str,
) -> serde_json::Value {
    serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "reaction": [{
            "type": "emoji",
            "emoji": emoji
        }]
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TelegramAttachmentKind {
    Image,
    Document,
    Video,
    Audio,
    Voice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TelegramAttachment {
    pub(crate) kind: TelegramAttachmentKind,
    pub(crate) target: String,
}

impl TelegramAttachmentKind {
    pub(crate) fn from_marker(marker: &str) -> Option<Self> {
        match marker.trim().to_ascii_uppercase().as_str() {
            "IMAGE" | "PHOTO" => Some(Self::Image),
            "DOCUMENT" | "FILE" => Some(Self::Document),
            "VIDEO" => Some(Self::Video),
            "AUDIO" => Some(Self::Audio),
            "VOICE" => Some(Self::Voice),
            _ => None,
        }
    }
}

/// Check whether a file path has a recognized image extension.
pub(crate) fn is_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
            )
        })
        .unwrap_or(false)
}

/// Build the user-facing content string for an incoming attachment.
///
/// Photos with a recognized image extension use `[IMAGE:/path]` so the
/// multimodal pipeline can validate vision capability. Non-image files
/// always use `[Document: name] /path` regardless of how Telegram
/// classified them.
pub(crate) fn format_attachment_content(
    kind: IncomingAttachmentKind,
    local_filename: &str,
    local_path: &Path,
) -> String {
    match kind {
        IncomingAttachmentKind::Photo | IncomingAttachmentKind::Document
            if is_image_extension(local_path) =>
        {
            format!("[IMAGE:{}]", local_path.display())
        }
        _ => {
            format!("[Document: {}] {}", local_filename, local_path.display())
        }
    }
}

pub(crate) fn is_http_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

pub(crate) fn infer_attachment_kind_from_target(target: &str) -> Option<TelegramAttachmentKind> {
    let normalized = target
        .split('?')
        .next()
        .unwrap_or(target)
        .split('#')
        .next()
        .unwrap_or(target);

    let extension = Path::new(normalized)
        .extension()
        .and_then(|ext| ext.to_str())?
        .to_ascii_lowercase();

    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => Some(TelegramAttachmentKind::Image),
        "mp4" | "mov" | "mkv" | "avi" | "webm" => Some(TelegramAttachmentKind::Video),
        "mp3" | "m4a" | "wav" | "flac" => Some(TelegramAttachmentKind::Audio),
        "ogg" | "oga" | "opus" => Some(TelegramAttachmentKind::Voice),
        "pdf" | "txt" | "md" | "csv" | "json" | "zip" | "tar" | "gz" | "doc" | "docx" | "xls"
        | "xlsx" | "ppt" | "pptx" => Some(TelegramAttachmentKind::Document),
        _ => None,
    }
}

pub(crate) fn parse_path_only_attachment(message: &str) -> Option<TelegramAttachment> {
    let trimmed = message.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }

    let candidate = trimmed.trim_matches(|c| matches!(c, '`' | '"' | '\''));
    if candidate.chars().any(char::is_whitespace) {
        return None;
    }

    let candidate = candidate.strip_prefix("file://").unwrap_or(candidate);
    let kind = infer_attachment_kind_from_target(candidate)?;

    if !is_http_url(candidate) && !Path::new(candidate).exists() {
        return None;
    }

    Some(TelegramAttachment {
        kind,
        target: candidate.to_string(),
    })
}

/// Delegate to the shared `strip_tool_call_tags` in the orchestrator module.
pub(crate) fn strip_tool_call_tags(message: &str) -> String {
    crate::orchestrator::strip_tool_call_tags(message)
}

pub(crate) fn find_matching_close(s: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn parse_attachment_markers(message: &str) -> (String, Vec<TelegramAttachment>) {
    let mut cleaned = String::with_capacity(message.len());
    let mut attachments = Vec::new();
    let mut cursor = 0;

    while cursor < message.len() {
        let Some(open_rel) = message[cursor..].find('[') else {
            cleaned.push_str(&message[cursor..]);
            break;
        };

        let open = cursor + open_rel;
        cleaned.push_str(&message[cursor..open]);

        let Some(close_rel) = find_matching_close(&message[open + 1..]) else {
            cleaned.push_str(&message[open..]);
            break;
        };

        let close = open + 1 + close_rel;
        let marker = &message[open + 1..close];

        let parsed = marker.split_once(':').and_then(|(kind, target)| {
            let kind = TelegramAttachmentKind::from_marker(kind)?;
            let target = target.trim();
            if target.is_empty() {
                return None;
            }
            Some(TelegramAttachment {
                kind,
                target: target.to_string(),
            })
        });

        if let Some(attachment) = parsed {
            attachments.push(attachment);
        } else {
            cleaned.push_str(&message[open..=close]);
        }

        cursor = close + 1;
    }

    (cleaned.trim().to_string(), attachments)
}

/// Telegram Bot API maximum file download size (20 MB).
pub(crate) const TELEGRAM_MAX_FILE_DOWNLOAD_BYTES: u64 = 20 * 1024 * 1024;

/// A pending multiple-choice question: the oneshot sender to resolve with
/// the chosen option text, plus the original option list so an index-based
/// `choice:` callback can be mapped back to text.
pub(crate) type PendingChoice = (tokio::sync::oneshot::Sender<String>, Vec<String>);
