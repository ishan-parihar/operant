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

/// Report from a rollup maintenance pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MaintenanceReport {
    /// Sessions scanned for missing rollups.
    pub sessions_scanned: usize,
    /// Day rollups built.
    pub days_built: usize,
    /// Week rollups built.
    pub weeks_built: usize,
    /// Month rollups built.
    pub months_built: usize,
    /// Periods already present and skipped (idempotent).
    pub skipped_existing: usize,
    /// Periods with no source content (nothing to summarize).
    pub skipped_empty: usize,
    /// Summarizer failures swallowed (one bad LLM call never aborts a pass).
    pub errors: usize,
}

impl MaintenanceReport {
    pub fn total_built(&self) -> usize {
        self.days_built + self.weeks_built + self.months_built
    }
}

/// Run one rollup maintenance pass across every session in the DAG (or a
/// single session when `only_session` is `Some`): build missing day/week/month
/// rollups for recent periods, skipping periods that already have a rollup
/// (hermes `run_rollup_maintenance` dedup semantics).
///
/// `lookback_days`: how far back (UTC) to cover, applied per period kind
/// (e.g. 7 → today's day-rollup plus the last 6 days). Summarizer errors for
/// a single period are swallowed and counted — one bad LLM call never aborts
/// the whole pass.
pub async fn run_rollup_maintenance<F, Fut>(
    engine: &LcmContextEngine,
    only_session: Option<&str>,
    lookback_days: u32,
    summarizer: F,
) -> Result<MaintenanceReport>
where
    F: Fn(String) -> Fut + Clone,
    Fut: Future<Output = Result<String>>,
{
    let today = Utc::now().date_naive();
    let mut report = MaintenanceReport::default();

    let sessions = match only_session {
        Some(sid) => vec![(sid.to_string(), 0, 0_i64)],
        None => engine.list_sessions()?,
    };
    for (session_id, _count, _last_ts) in sessions {
        report.sessions_scanned += 1;

        // Day: each of the last `lookback_days` days.
        for offset in 0..lookback_days.max(1) {
            let anchor = today - Duration::days(offset as i64);
            report = build_missing(
                engine,
                &session_id,
                RollupPeriod::Day,
                anchor,
                summarizer.clone(),
                report,
            )
            .await?;
        }
        // Week: Mondays of the last `lookback_days`/7 weeks (at least 1).
        for w in 0..(lookback_days / 7).max(1) {
            let anchor = today - Duration::days(w as i64 * 7);
            report = build_missing(
                engine,
                &session_id,
                RollupPeriod::Week,
                anchor,
                summarizer.clone(),
                report,
            )
            .await?;
        }
        // Month: 1sts of the last `lookback_days`/30 calendar months (at
        // least 1). Uses checked_sub_months — a fixed 30-day stride would
        // skip short months (e.g. Feb when today is Mar 1) or revisit the
        // same month (Jan 31 → Jan 1 twice).
        for m in 0..(lookback_days / 30).max(1) {
            let anchor = today
                .checked_sub_months(chrono::Months::new(m))
                .unwrap_or(today);
            report = build_missing(
                engine,
                &session_id,
                RollupPeriod::Month,
                anchor,
                summarizer.clone(),
                report,
            )
            .await?;
        }
    }
    Ok(report)
}

/// Build one period if missing; fold the outcome into `report`.
async fn build_missing<F, Fut>(
    engine: &LcmContextEngine,
    session_id: &str,
    period: RollupPeriod,
    anchor: chrono::NaiveDate,
    summarizer: F,
    mut report: MaintenanceReport,
) -> Result<MaintenanceReport>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let period_start = period.period_start(anchor);
    let start_key = period_start.format("%Y-%m-%d").to_string();
    if engine
        .has_rollup(session_id, period.kind(), &start_key)
        .unwrap_or(false)
    {
        report.skipped_existing += 1;
        return Ok(report);
    }
    match build_rollup(engine, session_id, period, Some(anchor), summarizer).await {
        Ok(Some(_)) => match period {
            RollupPeriod::Day => report.days_built += 1,
            RollupPeriod::Week => report.weeks_built += 1,
            RollupPeriod::Month => report.months_built += 1,
        },
        Ok(None) => report.skipped_empty += 1,
        Err(_) => report.errors += 1, // one bad period never aborts the pass
    }
    Ok(report)
}

/// Spawn a background rollup maintenance task (hermes
/// `_RollupMaintenanceScheduler` parity, bounded): one immediate pass, then
/// every `interval`. Each pass scans all DAG sessions and builds missing
/// day/week/month rollups; empty windows skip the summarizer, existing
/// periods are skipped, and a bad pass is logged and swallowed — it never
/// aborts the loop. Abort the returned `JoinHandle` to stop.
///
/// `lookback_days` feeds `run_rollup_maintenance` (default 7). This is the
/// on-demand maintenance pass behind a cadence; the full hermes worker
/// (process-wide dedup queues, ownership tracking) remains deferred YAGNI.
pub fn spawn_rollup_maintenance<F, Fut>(
    engine: std::sync::Arc<LcmContextEngine>,
    interval: std::time::Duration,
    lookback_days: u32,
    summarizer: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn(String) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<String>> + Send + 'static,
{
    tokio::spawn(async move {
        // Immediate first pass: a fresh DAG is cheap (empty windows skip the
        // summarizer) and stale sessions from earlier runs get caught up
        // right away instead of after the first full interval.
        run_maintenance_pass(&engine, lookback_days, summarizer.clone()).await;
        loop {
            tokio::time::sleep(interval).await;
            run_maintenance_pass(&engine, lookback_days, summarizer.clone()).await;
        }
    })
}

/// One scheduler tick: run the maintenance pass and log the outcome.
async fn run_maintenance_pass<F, Fut>(
    engine: &std::sync::Arc<LcmContextEngine>,
    lookback_days: u32,
    summarizer: F,
) where
    F: Fn(String) -> Fut + Clone,
    Fut: Future<Output = Result<String>>,
{
    match run_rollup_maintenance(engine, None, lookback_days, summarizer).await {
        Ok(report) => {
            tracing::info!(
                sessions = report.sessions_scanned,
                days = report.days_built,
                weeks = report.weeks_built,
                months = report.months_built,
                skipped_existing = report.skipped_existing,
                skipped_empty = report.skipped_empty,
                errors = report.errors,
                "lcm rollup maintenance pass complete"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "lcm rollup maintenance pass failed");
        }
    }
}

/// Spawn a background assertion-extraction task (hermes `_assertion_extraction`
/// maintenance parity, bounded): one immediate run, then every `interval`.
/// Each run LLM-mines durable facts from the recent DAG message nodes and
/// persists them — the same shared backend as `lcm_assert action="extract"`
/// (`assertion_extract::run_assertion_extraction`), so manual and automatic
/// mining can never drift. Empty scopes / no-fact results are clean no-ops;
/// a failed pass is logged and swallowed — it never aborts the loop.
/// Abort the returned `JoinHandle` to stop.
pub fn spawn_assertion_extraction_scheduler(
    engine: std::sync::Arc<LcmContextEngine>,
    extractor: std::sync::Arc<dyn crate::context::AssertionExtractor>,
    interval: std::time::Duration,
    limit: usize,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Immediate first run: mine whatever is already in the DAG instead
        // of waiting for the first full interval.
        run_assertion_extraction_pass(&engine, extractor.clone(), limit).await;
        loop {
            tokio::time::sleep(interval).await;
            run_assertion_extraction_pass(&engine, extractor.clone(), limit).await;
        }
    })
}

/// One assertion-extraction tick: mine durable facts and log the outcome.
async fn run_assertion_extraction_pass(
    engine: &std::sync::Arc<LcmContextEngine>,
    extractor: std::sync::Arc<dyn crate::context::AssertionExtractor>,
    limit: usize,
) {
    match crate::context::assertion_extract::run_assertion_extraction(
        engine,
        extractor.as_ref(),
        None,
        limit,
    )
    .await
    {
        Ok(report) => {
            tracing::info!(
                scanned = report.scanned_nodes,
                saved = report.saved,
                "lcm assertion extraction maintenance pass complete"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "lcm assertion extraction maintenance pass failed");
        }
    }
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
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
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
        // Rebuild refreshes created_at to now (ON CONFLICT DO UPDATE) — the
        // idempotency contract is the row count, not the timestamp.
        assert!(second.created_at >= first.created_at);
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

    #[tokio::test]
    async fn maintenance_builds_missing_and_skips_existing() {
        let (engine, _) = test_engine();
        engine
            .ingest_turn(
                "sess_maint",
                &[
                    Message::user("deploy policy: biweekly on wednesdays"),
                    Message::assistant("deploys every two weeks"),
                ],
            )
            .await
            .unwrap();
        let echo = |t: String| async move { Ok(format!("ECHO[{t}]")) };

        // First pass: nothing exists → day built, week/month have content too.
        let r1 = run_rollup_maintenance(&engine, None, 1, echo)
            .await
            .unwrap();
        assert_eq!(r1.sessions_scanned, 1);
        assert_eq!(r1.days_built, 1, "day rollup for today");
        assert!(r1.total_built() >= 1);

        // Second pass: everything already exists → no new builds, all skipped.
        let r2 = run_rollup_maintenance(&engine, None, 1, echo)
            .await
            .unwrap();
        assert_eq!(r2.total_built(), 0, "idempotent second pass");
        assert!(r2.skipped_existing >= 1);
    }

    #[tokio::test]
    async fn maintenance_scopes_to_one_session() {
        let (engine, _) = test_engine();
        engine
            .ingest_turn("sess_a", &[Message::user("alpha content")])
            .await
            .unwrap();
        engine
            .ingest_turn("sess_b", &[Message::user("beta content")])
            .await
            .unwrap();
        let echo = |t: String| async move { Ok(format!("ECHO[{t}]")) };

        // only_session="sess_a" must build for a alone, never touching b.
        let r = run_rollup_maintenance(&engine, Some("sess_a"), 1, echo)
            .await
            .unwrap();
        assert_eq!(r.sessions_scanned, 1);
        // lookback_days=1 covers day + week + month anchors for sess_a.
        assert_eq!(
            engine.list_rollups("sess_a").unwrap().len(),
            3,
            "sess_a built"
        );
        assert_eq!(
            engine.list_rollups("sess_b").unwrap().len(),
            0,
            "sess_b untouched"
        );
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

    #[tokio::test]
    async fn spawned_maintenance_builds_rollups_on_interval() {
        let (engine, _) = test_engine();
        let engine = std::sync::Arc::new(engine);
        let turn = vec![Message::assistant("alpha fact for the scheduler")];
        engine.ingest_turn("sess_sched", &turn).await.unwrap();

        let summarizer = |t: String| fake_summarizer(t);
        let handle = spawn_rollup_maintenance(
            engine.clone(),
            std::time::Duration::from_millis(30),
            1,
            summarizer,
        );
        // Immediate first pass + a tick or two.
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let rollups = engine.list_rollups("sess_sched").unwrap();
        assert!(
            !rollups.is_empty(),
            "scheduler must build day rollups automatically, got: {rollups:?}"
        );
        assert!(rollups.iter().any(|r| r.summary.starts_with("SUMMARY[")));
        handle.abort();
    }

    #[tokio::test]
    async fn spawned_maintenance_stops_on_abort() {
        let (engine, _) = test_engine();
        let engine = std::sync::Arc::new(engine);
        engine
            .ingest_turn("sess_a", &[Message::assistant("fact for session a")])
            .await
            .unwrap();

        let handle = spawn_rollup_maintenance(
            engine.clone(),
            std::time::Duration::from_millis(20),
            1,
            |t: String| fake_summarizer(t),
        );
        // Wait until the immediate pass builds sess_a's rollup.
        let mut built = false;
        for _ in 0..50 {
            if !engine.list_rollups("sess_a").unwrap().is_empty() {
                built = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(built, "first pass must build sess_a rollup");
        handle.abort();

        // After abort, a fresh session must never be maintained.
        engine
            .ingest_turn("sess_b", &[Message::assistant("fact for session b")])
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            engine.list_rollups("sess_b").unwrap().is_empty(),
            "aborted scheduler must not keep building"
        );
    }

    #[tokio::test]
    async fn spawned_assertion_extraction_persists_mined_facts() {
        let (engine, _) = test_engine();
        let engine = std::sync::Arc::new(engine);
        // Seed message nodes so the extractor has a transcript to mine.
        engine
            .ingest_turn(
                "sess_assert",
                &[
                    Message::user("my preferred stack is Rust and SQLite"),
                    Message::assistant("deploys run biweekly on wednesdays"),
                ],
            )
            .await
            .unwrap();

        // Deterministic fake extractor (hermes `_assertion_extraction` seam):
        // mines the same triples the real LLM would.
        let fake: std::sync::Arc<dyn crate::context::AssertionExtractor> =
            std::sync::Arc::new(crate::context::assertion_extract::tests::FakeExtractor {
                assertions: vec![
                    crate::context::assertion_extract::ExtractedAssertion {
                        subject: "project".to_string(),
                        predicate: "stack".to_string(),
                        object: "Rust and SQLite".to_string(),
                        speaker: "user".to_string(),
                    },
                    crate::context::assertion_extract::ExtractedAssertion {
                        subject: "project".to_string(),
                        predicate: "deploy_cadence".to_string(),
                        object: "biweekly on wednesdays".to_string(),
                        speaker: "assistant".to_string(),
                    },
                ],
            });
        let handle = spawn_assertion_extraction_scheduler(
            engine.clone(),
            fake,
            std::time::Duration::from_millis(30),
            40,
        );
        // Immediate first pass + a tick or two (mirrors the rollup scheduler
        // test's timing budget).
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;

        let rows = engine
            .query_assertion_state("global", "project", None)
            .unwrap();
        assert!(
            !rows.is_empty(),
            "scheduler must persist mined facts automatically, got: {rows:?}"
        );
        assert!(
            rows.iter()
                .any(|r| r.predicate == "stack" && r.object_value == "Rust and SQLite"),
            "mined stack fact must be queryable, got: {rows:?}"
        );
        assert!(
            rows.iter()
                .any(|r| r.predicate == "deploy_cadence"
                    && r.object_value == "biweekly on wednesdays"),
            "mined deploy_cadence fact must be queryable, got: {rows:?}"
        );
        handle.abort();
    }

    #[tokio::test]
    async fn spawned_assertion_extraction_stops_on_abort() {
        let (engine, _) = test_engine();
        let engine = std::sync::Arc::new(engine);
        engine
            .ingest_turn("sess_abort", &[Message::user("one fact: stack is Rust")])
            .await
            .unwrap();

        let fake: std::sync::Arc<dyn crate::context::AssertionExtractor> =
            std::sync::Arc::new(crate::context::assertion_extract::tests::FakeExtractor {
                assertions: vec![crate::context::assertion_extract::ExtractedAssertion {
                    subject: "project".to_string(),
                    predicate: "stack".to_string(),
                    object: "Rust".to_string(),
                    speaker: "user".to_string(),
                }],
            });
        let handle = spawn_assertion_extraction_scheduler(
            engine.clone(),
            fake,
            std::time::Duration::from_millis(20),
            40,
        );
        // Wait for the immediate pass to persist the fact.
        let mut persisted = false;
        for _ in 0..50 {
            if !engine
                .query_assertion_state("global", "project", None)
                .unwrap()
                .is_empty()
            {
                persisted = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(persisted, "immediate pass must mine the seeded fact");
        handle.abort();

        // After abort, new nodes must never be mined.
        engine
            .ingest_turn(
                "sess_late",
                &[Message::assistant("late fact: editor is neovim")],
            )
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let rows = engine
            .query_assertion_state("global", "project", Some("preferred_editor"))
            .unwrap();
        assert!(
            rows.is_empty(),
            "aborted scheduler must not keep mining, got: {rows:?}"
        );
    }
}
