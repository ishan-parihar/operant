// Vendored from jcode (crates/jcode-base/src/usage/model.rs + crates/jcode-usage-types/src/lib.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — see jcode_app/mod.rs for scope.
//! Included: OpenAIUsageWindow (model.rs :187), OpenAIUsageData (:196, plus its
//! impl), OpenAiResetCredits (usage-types :4). [port-excision] the rest of
//! jcode-usage-types is not ported.
use std::time::{Duration, Instant};

// [port-decision] leaf ports from jcode-base/src/usage: the staleness consts
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

    pub fn age_ms(&self) -> Option<u128> {
        self.fetched_at.map(|t| t.elapsed().as_millis())
    }

    pub fn freshness_state(&self) -> &'static str {
        if self.fetched_at.is_none() {
            "unknown"
        } else if self.is_stale() {
            "stale"
        } else {
            "fresh"
        }
    }

    pub fn exhausted(&self) -> bool {
        if self.hard_limit_reached {
            return true;
        }

        if !self.has_limits() {
            return false;
        }

        let mut windows = [self.five_hour.as_ref(), self.seven_day.as_ref()]
            .into_iter()
            .flatten()
            .peekable();
        windows.peek().is_some() && windows.all(|window| window.usage_ratio >= 0.99)
    }

    pub fn diagnostic_fields(&self) -> String {
        let fmt_ratio = |window: Option<&OpenAIUsageWindow>| {
            window
                .map(|w| format!("{:.1}%", w.usage_ratio * 100.0))
                .unwrap_or_else(|| "unknown".to_string())
        };

        format!(
            "freshness={} age_ms={} exhausted={} hard_limit_reached={} has_limits={} five_hour={} seven_day={} spark={} last_error={}",
            self.freshness_state(),
            self.age_ms()
                .map(|age| age.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            self.exhausted(),
            self.hard_limit_reached,
            self.has_limits(),
            fmt_ratio(self.five_hour.as_ref()),
            fmt_ratio(self.seven_day.as_ref()),
            fmt_ratio(self.spark.as_ref()),
            self.last_error.as_deref().unwrap_or("none")
        )
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

    pub fn has_limits(&self) -> bool {
        self.five_hour.is_some() || self.seven_day.is_some() || self.spark.is_some()
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
