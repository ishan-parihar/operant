// Vendored from jcode (crates/operant-base/src/usage/model.rs + crates/operant-usage-types/src/lib.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — see operant_app/mod.rs for scope.
//! Included: OpenAIUsageWindow (model.rs :187), OpenAIUsageData (:196, plus its
//! impl), OpenAiResetCredits (usage-types :4). [port-excision] the rest of
//! operant-usage-types is not ported.
use std::time::{Duration, Instant};

// [port-decision] leaf ports from operant-base/src/usage: the staleness consts
// (usage.rs:50,53,56) and usage_reset_passed + its parse_reset_timestamp
// (usage/display.rs:132-141, 120-130) — used by OpenAIUsageData::is_stale.
/// Cache duration (refresh every 5 minutes - usage data is slow-changing)
const CACHE_DURATION: Duration = Duration::from_secs(300);

/// Error backoff duration (wait 5 minutes before retrying after auth/credential errors)
const ERROR_BACKOFF: Duration = Duration::from_secs(300);

/// Rate limit backoff duration (wait 15 minutes before retrying after 429 errors)
const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(900);

fn parse_reset_timestamp(timestamp: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(reset) = chrono::DateTime::parse_from_rfc3339(timestamp) {
        Some(reset.with_timezone(&chrono::Utc))
    } else if let Ok(reset) =
        chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.fZ")
    {
        Some(reset.and_utc())
    } else {
        None
    }
}

pub(crate) fn usage_reset_passed<'a>(
    timestamps: impl IntoIterator<Item = Option<&'a str>>,
) -> bool {
    let now = chrono::Utc::now();
    timestamps
        .into_iter()
        .flatten()
        .filter_map(parse_reset_timestamp)
        .any(|reset| reset <= now)
}

// ─── Combined usage for /usage command ───────────────────────────────────────

/// Normalized OpenAI/Codex usage window info used by the TUI widget.
#[derive(Debug, Clone, Default)]
pub struct OpenAIUsageWindow {
    pub name: String,
    /// Utilization as a fraction in [0.0, 1.0].
    pub usage_ratio: f32,
    pub resets_at: Option<String>,
}

/// Cached OpenAI/Codex usage snapshot for info widgets.
#[derive(Debug, Clone, Default)]
pub struct OpenAIUsageData {
    pub five_hour: Option<OpenAIUsageWindow>,
    pub seven_day: Option<OpenAIUsageWindow>,
    pub spark: Option<OpenAIUsageWindow>,
    pub hard_limit_reached: bool,
    pub openai_reset_credits: Option<OpenAiResetCredits>,
    pub fetched_at: Option<Instant>,
    pub last_error: Option<String>,
}

impl OpenAIUsageData {
    /// Recommend a reset only with fresh, account-matched availability and an
    /// actually reached limit. The general `exhausted` heuristic's 99% threshold
    /// is useful for failover, but must not encourage spending a reset early.
    pub fn banked_reset_available_for_account(&self, account_label: Option<&str>) -> bool {
        if self.is_stale() || self.last_error.is_some() {
            return false;
        }
        let Some(credits) = &self.openai_reset_credits else {
            return false;
        };
        if credits.available_count == 0 || credits.account_label.as_deref() != account_label {
            return false;
        }
        // OpenAI can report rounded 100% usage while still allowing requests.
        // Conversely an enforced limit need not have a corresponding window.
        if let Some(allowed) = credits.ordinary_usage_allowed {
            return !allowed;
        }
        self.hard_limit_reached
            || [self.five_hour.as_ref(), self.seven_day.as_ref()]
                .into_iter()
                .flatten()
                .any(|window| window.usage_ratio >= 1.0)
    }

    pub fn is_stale(&self) -> bool {
        if usage_reset_passed([
            self.five_hour.as_ref().and_then(|w| w.resets_at.as_deref()),
            self.seven_day.as_ref().and_then(|w| w.resets_at.as_deref()),
            self.spark.as_ref().and_then(|w| w.resets_at.as_deref()),
        ]) {
            return true;
        }

        match self.fetched_at {
            Some(t) => {
                let ttl = if self.is_rate_limited() {
                    RATE_LIMIT_BACKOFF
                } else if self.last_error.is_some() {
                    ERROR_BACKOFF
                } else {
                    CACHE_DURATION
                };
                t.elapsed() > ttl
            }
            None => true,
        }
    }

    fn is_rate_limited(&self) -> bool {
        self.last_error
            .as_ref()
            .map(|e| e.contains("429") || e.contains("rate limit") || e.contains("Rate limited"))
            .unwrap_or(false)
    }
}

/// Read-only banked reset metadata from the ChatGPT usage response, pinned to
/// the login whose usage was fetched. `None` label denotes the default scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiResetCredits {
    pub available_count: u64,
    /// One expiry per available reset, in RFC3339. Missing entries are unknown.
    pub available_expirations: Vec<Option<String>>,
    pub account_label: Option<String>,
    pub ordinary_usage_allowed: Option<bool>,
}

/// Read-only Claude session-limit reset offer (the `/limit-reset` program),
/// pinned to the login whose usage was fetched. `None` label is the default
/// scope. Only fetched at the five-hour wall, where the offer applies.
/// Verbatim from crates/operant-usage-types/src/lib.rs:16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicLimitResetOffer {
    pub account_label: Option<String>,
    /// The server says a reset can be claimed right now.
    pub available: bool,
    /// RFC3339 time the next reset becomes available when one was spent.
    pub next_available_at: Option<String>,
    pub resets_per_week: u64,
}

/// Verbatim from crates/operant-usage-types/src/lib.rs:26.
#[derive(Debug, Clone, Default)]
pub struct ProviderUsage {
    pub provider_name: String,
    pub limits: Vec<UsageLimit>,
    pub extra_info: Vec<(String, String)>,
    pub hard_limit_reached: bool,
    pub openai_reset_credits: Option<OpenAiResetCredits>,
    pub anthropic_limit_reset: Option<AnthropicLimitResetOffer>,
    pub error: Option<String>,
    /// When jcode last successfully used this login/credential (unix seconds).
    /// Drives most-recently-used-first ordering in `/usage`. `None` sorts last.
    pub last_used_unix_secs: Option<u64>,
}

/// Verbatim from crates/operant-usage-types/src/lib.rs:40.
#[derive(Debug, Clone)]
pub struct UsageLimit {
    pub name: String,
    pub usage_percent: f32,
    pub resets_at: Option<String>,
}
