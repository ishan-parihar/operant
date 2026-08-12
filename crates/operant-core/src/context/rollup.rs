//! Temporal LLM rollups over the lossless DAG (hermes-lcm `rollup_builder.py`
//! + `rollup_store.py` parity).
//!
//! Rollups summarize DAG content below the D0 fresh-tail frontier into
//! day / week / month summaries stored in the `lcm_rollups` table. The
//! summarizer is injected (`Summarizer = Callable` in hermes) so the core is
//! fully testable with a fake, while the CLI wires the real model client.
//!
//! Hermes keeps this deliberately separate from the engine ("not wired into
//! the LCM engine yet"): rollups are *derived* state on top of the lossless
//! store — the store itself always keeps verbatim nodes.

use std::future::Future;

use chrono::{Datelike, Duration, Months, NaiveDate, Utc};

use crate::context::lcm::LcmContextEngine;
use crate::error::Result;

/// Rollup aggregation period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollupPeriod {
    /// UTC calendar day.
    Day,
    /// UTC ISO week (Monday start).
    Week,
    /// UTC calendar month.
    Month,
}

impl RollupPeriod {
    /// Stable kind string stored in `lcm_rollups.period_kind`.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    /// (start, end) UTC-day boundaries for the period containing `anchor`,
    /// in unix millis. `end` is exclusive.
    pub fn window(self, anchor: NaiveDate) -> (i64, i64) {
        let start = self.period_start(anchor);
        let end = match self {
            Self::Day => start + Duration::days(1),
            Self::Week => start + Duration::days(7),
            Self::Month => start.checked_add_months(Months::new(1)).unwrap_or(start),
        };
        (
            start
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis(),
            end.and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis(),
        )
    }

    /// First date of the period containing `anchor` (UTC): the day itself,
    /// the ISO week's Monday, or the 1st of the month.
    fn period_start(self, anchor: NaiveDate) -> NaiveDate {
        match self {
            Self::Day => anchor,
            Self::Week => anchor - Duration::days(anchor.weekday().num_days_from_monday() as i64),
            Self::Month => {
                NaiveDate::from_ymd_opt(anchor.year(), anchor.month(), 1).unwrap_or(anchor)
            }
        }
    }
}

impl std::str::FromStr for RollupPeriod {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "day" => Ok(Self::Day),
            "week" => Ok(Self::Week),
            "month" => Ok(Self::Month),
            other => Err(format!(
                "invalid rollup period '{other}' (expected day | week | month)"
            )),
        }
    }
}

/// One stored rollup summary row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollupSummary {
    /// `day` | `week` | `month`.
    pub period_kind: String,
    /// ISO-8601 date (YYYY-MM-DD) of the period start.
    pub period_start: String,
    /// LLM summary text.
    pub summary: String,
    /// Number of DAG source nodes condensed by this rollup.
    pub source_count: usize,
    /// Unix millis when the rollup was (re)built.
    pub created_at: i64,
}

/// Maximum number of DAG nodes fed into a single rollup summarization.
pub const MAX_ROLLUP_SOURCES: usize = 200;

/// Maximum total characters of source text sent to the summarizer (hard cap
/// to keep rollup LLM calls bounded; mirror hermes `_deterministic_truncate`).
pub const MAX_ROLLUP_SOURCE_CHARS: usize = 24_000;

/// Build (or refresh) the rollup for `period` containing `anchor` (default:
/// today UTC) for `session_id`.
///
/// Fetches the session's DAG nodes inside the period window (excluding
/// rollup/derived nodes), truncates deterministically, calls the injected
/// summarizer, and upserts the result into `lcm_rollups` (idempotent: the
/// same period is refreshed, never duplicated). Returns `None` when the
/// window contains no source nodes.
///
/// `summarizer` receives the bounded source transcript and returns the
/// summary text — hermes `Summarizer = Callable[..., tuple[str, int]]`.
pub async fn build_rollup<F, Fut>(
    engine: &LcmContextEngine,
    session_id: &str,
    period: RollupPeriod,
    anchor: Option<NaiveDate>,
    summarizer: F,
) -> Result<Option<RollupSummary>>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let anchor = anchor.unwrap_or_else(|| Utc::now().date_naive());
    let (start_ms, end_ms) = period.window(anchor);
    let nodes = engine.nodes_in_window(session_id, start_ms, end_ms, MAX_ROLLUP_SOURCES)?;
    if nodes.is_empty() {
        return Ok(None);
    }

    // Deterministic bound: join newest-first, cap by source count and chars.
    let mut transcript = String::new();
    for content in nodes.iter().rev() {
        if transcript.len() >= MAX_ROLLUP_SOURCE_CHARS {
            break;
        }
        transcript.push_str(content);
        transcript.push('\n');
    }
    if transcript.len() > MAX_ROLLUP_SOURCE_CHARS {
        // Byte-cap must land on a char boundary or String::truncate panics
        // on multi-byte UTF-8 content. Walk back to the nearest boundary.
        let mut bound = MAX_ROLLUP_SOURCE_CHARS;
        while !transcript.is_char_boundary(bound) {
            bound -= 1;
        }
        transcript.truncate(bound);
    }

    let summary = summarizer(transcript).await?;
    let summary = summary.trim();
    if summary.is_empty() {
        // The model produced nothing useful — don't store an empty rollup
        // that would shadow the period in listings.
        return Ok(None);
    }
    let period_start = period.period_start(anchor);
    let created_at = engine.upsert_rollup(
        session_id,
        period.kind(),
        &period_start.format("%Y-%m-%d").to_string(),
        summary.trim(),
        nodes.len(),
    )?;
    Ok(Some(RollupSummary {
        period_kind: period.kind().to_string(),
        period_start: period_start.format("%Y-%m-%d").to_string(),
        summary: summary.trim().to_string(),
        source_count: nodes.len(),
        created_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Message;
    use crate::context::ContextEngine;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_engine() -> (LcmContextEngine, String) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("operant_rollup_{}_{}", std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("rollup-test.db");
        let engine = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
        })
        .unwrap();
        (engine, format!("{}", db_path.display()))
    }
    async fn fake_summarizer(text: String) -> Result<String> {
        Ok(format!(
            "SUMMARY[{}, {} chars]",
            text.lines().count(),
            text.len()
        ))
    }

    #[tokio::test]
    async fn build_rollup_upserts_and_is_idempotent() {
        let (engine, _) = test_engine();
        let turn = vec![
            Message::user("release cadence is biweekly"),
            Message::assistant("deploys happen every two weeks on wednesday"),
        ];
        engine.ingest_turn("sess_rollup", &turn).await.unwrap();

        let first = build_rollup(
            &engine,
            "sess_rollup",
            RollupPeriod::Day,
            None,
            fake_summarizer,
        )
        .await
        .unwrap()
        .expect("day has sources");
        assert_eq!(first.period_kind, "day");
        assert_eq!(first.source_count, 2);
        assert!(first.summary.starts_with("SUMMARY["));

        // Idempotent rebuild: same period is refreshed, not duplicated.
        let second = build_rollup(
            &engine,
            "sess_rollup",
            RollupPeriod::Day,
            None,
            fake_summarizer,
        )
        .await
        .unwrap()
        .expect("day has sources");
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.period_start, first.period_start);

        let all = engine.list_rollups("sess_rollup").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].period_kind, "day");
    }

    #[tokio::test]
    async fn build_rollup_empty_window_returns_none() {
        let (engine, _) = test_engine();
        let out = build_rollup(&engine, "empty", RollupPeriod::Day, None, fake_summarizer)
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn build_rollup_scopes_to_session() {
        let (engine, _) = test_engine();
        engine
            .ingest_turn("sess_a", &[Message::user("alpha content")])
            .await
            .unwrap();
        engine
            .ingest_turn("sess_b", &[Message::user("beta content")])
            .await
            .unwrap();

        // Echoing summarizer: proves which session's text reached the model.
        let echo = |t: String| async move { Ok(format!("ECHO[{t}]")) };

        let out_a = build_rollup(&engine, "sess_a", RollupPeriod::Day, None, echo)
            .await
            .unwrap()
            .expect("sess_a has sources");
        assert!(out_a.summary.contains("alpha"));
        assert!(!out_a.summary.contains("beta"));
        let out_b = build_rollup(&engine, "sess_b", RollupPeriod::Day, None, echo)
            .await
            .unwrap()
            .expect("sess_b has sources");
        assert!(out_b.summary.contains("beta"));
        assert_eq!(engine.list_rollups("sess_a").unwrap().len(), 1);
        assert_eq!(engine.list_rollups("sess_b").unwrap().len(), 1);
    }

    #[tokio::test]
    async fn truncation_handles_multi_byte_utf8_without_panic() {
        let (engine, _) = test_engine();
        // 4-byte emoji content — truncating mid-sequence would panic.
        let turn: Vec<Message> = (0..60)
            .map(|i| {
                if i % 2 == 0 {
                    Message::user(format!(
                        "emoji line {i} 🚀🚀🚀 with repeated multibyte content"
                    ))
                } else {
                    Message::assistant(format!("response {i} ✓✓✓ also multibyte 🎉"))
                }
            })
            .collect();
        engine.ingest_turn("sess_utf8", &turn).await.unwrap();
        let out = build_rollup(
            &engine,
            "sess_utf8",
            RollupPeriod::Day,
            None,
            fake_summarizer,
        )
        .await
        .unwrap()
        .expect("has sources");
        assert!(out.source_count > 0);
        assert!(out.summary.starts_with("SUMMARY["));
    }

    #[test]
    fn period_windows_are_contiguous() {
        // 2026-08-13 is a Thursday.
        let thursday = NaiveDate::from_ymd_opt(2026, 8, 13).unwrap();
        let (day_start, day_end) = RollupPeriod::Day.window(thursday);
        assert_eq!(day_end - day_start, 24 * 3600 * 1000);

        // ISO week starts Monday 2026-08-10.
        let (week_start, week_end) = RollupPeriod::Week.window(thursday);
        assert_eq!(week_end - week_start, 7 * 24 * 3600 * 1000);
        let monday = chrono::DateTime::from_timestamp_millis(week_start)
            .unwrap()
            .date_naive();
        assert_eq!(monday, NaiveDate::from_ymd_opt(2026, 8, 10).unwrap());

        // Month starts the 1st.
        let (month_start, month_end) = RollupPeriod::Month.window(thursday);
        let first = chrono::DateTime::from_timestamp_millis(month_start)
            .unwrap()
            .date_naive();
        assert_eq!(first, NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
        assert_eq!(month_end - month_start, 31 * 24 * 3600 * 1000);
    }
}
