//! Slack tunables, permalink/emoji helpers extracted verbatim.

pub(crate) const SLACK_HISTORY_MAX_RETRIES: u32 = 3;
pub(crate) const SLACK_HISTORY_DEFAULT_RETRY_AFTER_SECS: u64 = 1;
pub(crate) const SLACK_HISTORY_MAX_BACKOFF_SECS: u64 = 120;
pub(crate) const SLACK_HISTORY_MAX_JITTER_MS: u64 = 500;
pub(crate) const SLACK_SOCKET_MODE_INITIAL_BACKOFF_SECS: u64 = 3;
pub(crate) const SLACK_SOCKET_MODE_MAX_BACKOFF_SECS: u64 = 120;
pub(crate) const SLACK_SOCKET_MODE_MAX_JITTER_MS: u64 = 500;
pub(crate) const SLACK_USER_CACHE_TTL_SECS: u64 = 6 * 60 * 60;
pub(crate) const SLACK_ATTACHMENT_IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const SLACK_ATTACHMENT_IMAGE_INLINE_FALLBACK_MAX_BYTES: usize = 512 * 1024;
pub(crate) const SLACK_ATTACHMENT_TEXT_DOWNLOAD_MAX_BYTES: usize = 256 * 1024;
pub(crate) const SLACK_ATTACHMENT_TEXT_INLINE_MAX_CHARS: usize = 12_000;
pub(crate) const SLACK_MARKDOWN_BLOCK_MAX_CHARS: usize = 12_000;
pub(crate) const SLACK_BLOCK_TEXT_MAX_CHARS: usize = 3_000;
pub(crate) const SLACK_MAX_BLOCKS_PER_MESSAGE: usize = 50;
pub(crate) const SLACK_ATTACHMENT_FILENAME_MAX_CHARS: usize = 128;
pub(crate) const SLACK_USER_CACHE_MAX_ENTRIES: usize = 1000;
pub(crate) const SLACK_ATTACHMENT_SAVE_SUBDIR: &str = "slack_files";
pub(crate) const SLACK_ATTACHMENT_MAX_FILES_PER_MESSAGE: usize = 8;
pub(crate) const SLACK_PERMALINK_MAX_LINKS_PER_MESSAGE: usize = 3;
pub(crate) const SLACK_PERMALINK_THREAD_MAX_REPLIES: usize = 20;
pub(crate) const SLACK_PERMALINK_TEXT_MAX_CHARS: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackPermalinkRef {
    pub(crate) url: String,
    pub(crate) channel_id: String,
    pub(crate) message_ts: String,
    pub(crate) thread_ts_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlackPermalinkLookup {
    Message(serde_json::Value),
    AccessDenied(String),
    NotFound,
}

/// Extract the Slack message timestamp from a Operant message ID.
///
/// Message IDs follow the format `slack_{channel_id}_{ts}` where `ts`
/// contains a dot (e.g. `"1234567890.123456"`). If the format is
/// unrecognised the raw `message_id` is returned as-is.
pub(crate) fn extract_slack_ts(message_id: &str) -> &str {
    message_id
        .strip_prefix("slack_")
        .and_then(|rest| {
            rest.find('.').map(|dot_pos| {
                let underscore = rest[..dot_pos].rfind('_').unwrap_or(0);
                &rest[underscore + 1..]
            })
        })
        .unwrap_or(message_id)
}

/// Map a Unicode emoji to its Slack short-name.
///
/// The orchestration layer passes Unicode characters (e.g. `"\u{1F440}"`).
/// Slack's reactions API expects colon-free short-names (`"eyes"`).
pub(crate) fn unicode_emoji_to_slack_name(emoji: &str) -> &str {
    match emoji {
        "\u{1F440}" => "eyes",                        // 👀
        "\u{2705}" => "white_check_mark",             // ✅
        "\u{26A0}\u{FE0F}" | "\u{26A0}" => "warning", // ⚠️
        "\u{274C}" => "x",                            // ❌
        "\u{1F44D}" => "thumbsup",                    // 👍
        "\u{1F44E}" => "thumbsdown",                  // 👎
        "\u{2B50}" => "star",                         // ⭐
        "\u{1F389}" => "tada",                        // 🎉
        "\u{1F914}" => "thinking_face",               // 🤔
        "\u{1F525}" => "fire",                        // 🔥
        _ => emoji.trim_matches(':'),
    }
}
/// Default minimum interval between Slack draft edits.
/// Slack's `chat.update` is rate-limited to ~1 req/sec per channel.
pub(crate) const SLACK_DRAFT_UPDATE_INTERVAL_MS: u64 = 1200;

/// Maximum text length for a single Slack message (approx 40k chars).
pub(crate) const SLACK_MESSAGE_MAX_CHARS: usize = 40_000;

/// Prefix for lazy draft IDs that haven't been posted to Slack yet.
pub(crate) const LAZY_DRAFT_PREFIX: &str = "lazy:";

pub(crate) const SLACK_ATTACHMENT_RENDER_CONCURRENCY: usize = 3;
pub(crate) const SLACK_POLL_ACTIVE_THREAD_MAX: usize = 50;
pub(crate) const SLACK_POLL_THREAD_EXPIRE_SECS: u64 = 24 * 60 * 60;
pub(crate) const SLACK_MEDIA_REDIRECT_MAX_HOPS: usize = 5;
pub(crate) const SLACK_ALLOWED_MEDIA_HOST_SUFFIXES: &[&str] =
    &["slack.com", "slack-edge.com", "slack-files.com"];
pub(crate) const SLACK_SUPPORTED_IMAGE_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
];
