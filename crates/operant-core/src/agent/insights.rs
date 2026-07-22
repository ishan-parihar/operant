//! Session Insights Engine for Operant.
//!
//! Analyzes historical session data from the SQLite state database to produce
//! comprehensive usage insights — token consumption, cost estimates, tool usage
//! patterns, activity trends, model/platform breakdowns, and session metrics.
//!
//! Ported from `hermes-agent/agent/insights.py`.

use std::collections::BTreeMap;

use chrono::{Datelike, Timelike};
use rusqlite::params;

use crate::database::Database;
use crate::error::Result;

/// Top-level insights report.
#[derive(Debug, Clone)]
pub struct InsightsReport {
    pub days: u32,
    pub source_filter: Option<String>,
    pub empty: bool,
    pub overview: OverviewStats,
    pub models: Vec<ModelStats>,
    pub platforms: Vec<PlatformStats>,
    pub tools: Vec<ToolStats>,
    pub activity: ActivityPatterns,
    pub top_sessions: Vec<SessionHighlight>,
}

#[derive(Debug, Clone, Default)]
pub struct OverviewStats {
    pub total_sessions: usize,
    pub total_messages: usize,
    pub total_tool_calls: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub avg_messages_per_session: f64,
    pub avg_tokens_per_session: f64,
}

#[derive(Debug, Clone)]
pub struct ModelStats {
    pub model: String,
    pub sessions: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub tool_calls: usize,
}

#[derive(Debug, Clone)]
pub struct PlatformStats {
    pub platform: String,
    pub sessions: usize,
    pub messages: usize,
    pub tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ToolStats {
    pub tool: String,
    pub count: usize,
    pub percentage: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ActivityPatterns {
    pub busiest_day: Option<String>,
    pub busiest_hour: Option<u32>,
    pub active_days: usize,
    pub max_streak: usize,
}

#[derive(Debug, Clone)]
pub struct SessionHighlight {
    pub label: String,
    pub value: String,
    pub date: String,
}

/// Internal session summary from the database.
#[derive(Debug, Clone)]
struct SessionRow {
    #[allow(dead_code)]
    id: String,
    model: Option<String>,
    source: Option<String>,
    started_at: Option<String>,
    ended_at: Option<String>,
    message_count: i64,
    tool_call_count: i64,
    input_tokens: i64,
    output_tokens: i64,
}

impl SessionRow {
    fn started_dt(&self) -> Option<chrono::NaiveDateTime> {
        let s = self.started_at.as_deref()?;
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.naive_utc())
    }

    fn ended_dt(&self) -> Option<chrono::NaiveDateTime> {
        let s = self.ended_at.as_deref()?;
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.naive_utc())
    }

    fn duration_secs(&self) -> Option<f64> {
        let start = self.started_dt()?;
        let end = self.ended_dt()?;
        Some((end - start).num_seconds() as f64)
    }
}

/// Insights engine that queries the session database.
pub struct InsightsEngine<'a> {
    db: &'a Database,
}

impl<'a> InsightsEngine<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn generate(&self, days: u32, source: Option<&str>) -> InsightsReport {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();

        let sessions = match self.fetch_sessions(&cutoff, source) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to fetch sessions for insights");
                return empty_report(days, source);
            }
        };

        if sessions.is_empty() {
            return empty_report(days, source);
        }

        InsightsReport {
            days,
            source_filter: source.map(String::from),
            empty: false,
            overview: compute_overview(&sessions),
            models: compute_model_breakdown(&sessions),
            platforms: compute_platform_breakdown(&sessions),
            tools: compute_tool_breakdown(&sessions),
            activity: compute_activity_patterns(&sessions),
            top_sessions: compute_top_sessions(&sessions),
        }
    }

    fn fetch_sessions(&self, cutoff: &str, source: Option<&str>) -> Result<Vec<SessionRow>> {
        let conn = self.db.conn();

        // Always use the same query pattern to avoid type inference issues
        let mut stmt = conn
            .prepare(
                "SELECT id, model, source, started_at, ended_at, message_count, 
                 tool_call_count, input_tokens, output_tokens 
                 FROM sessions WHERE started_at >= ?1 
                 ORDER BY started_at DESC",
            )
            .map_err(|e| crate::error::Error::Agent(format!("Query prep failed: {}", e)))?;

        let rows: Vec<SessionRow> = stmt
            .query_map(params![cutoff], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    model: row.get(1)?,
                    source: row.get(2)?,
                    started_at: row.get(3)?,
                    ended_at: row.get(4)?,
                    message_count: row.get::<_, i64>(5).unwrap_or(0),
                    tool_call_count: row.get::<_, i64>(6).unwrap_or(0),
                    input_tokens: row.get::<_, i64>(7).unwrap_or(0),
                    output_tokens: row.get::<_, i64>(8).unwrap_or(0),
                })
            })
            .map_err(|e| crate::error::Error::Agent(format!("Query failed: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        // Filter by source in Rust instead of SQL to avoid type inference issues
        let filtered = if let Some(s) = source {
            rows.into_iter()
                .filter(|r| r.source.as_deref() == Some(s))
                .collect()
        } else {
            rows
        };

        Ok(filtered)
    }

    pub fn format_terminal(&self, report: &InsightsReport) -> String {
        format_report_terminal(report)
    }

    pub fn format_gateway(&self, report: &InsightsReport) -> String {
        format_report_gateway(report)
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

fn empty_report(days: u32, source: Option<&str>) -> InsightsReport {
    InsightsReport {
        days,
        source_filter: source.map(String::from),
        empty: true,
        overview: OverviewStats::default(),
        models: Vec::new(),
        platforms: Vec::new(),
        tools: Vec::new(),
        activity: ActivityPatterns::default(),
        top_sessions: Vec::new(),
    }
}

fn compute_overview(sessions: &[SessionRow]) -> OverviewStats {
    let n = sessions.len();
    let total_messages: i64 = sessions.iter().map(|s| s.message_count).sum();
    let total_tool_calls: i64 = sessions.iter().map(|s| s.tool_call_count).sum();
    let total_input: i64 = sessions.iter().map(|s| s.input_tokens).sum();
    let total_output: i64 = sessions.iter().map(|s| s.output_tokens).sum();
    let total_tokens = total_input + total_output;

    OverviewStats {
        total_sessions: n,
        total_messages: total_messages as usize,
        total_tool_calls: total_tool_calls as usize,
        total_input_tokens: total_input as u64,
        total_output_tokens: total_output as u64,
        total_tokens: total_tokens as u64,
        avg_messages_per_session: if n > 0 {
            total_messages as f64 / n as f64
        } else {
            0.0
        },
        avg_tokens_per_session: if n > 0 {
            total_tokens as f64 / n as f64
        } else {
            0.0
        },
    }
}

fn compute_model_breakdown(sessions: &[SessionRow]) -> Vec<ModelStats> {
    let mut map: BTreeMap<String, ModelStats> = BTreeMap::new();
    for s in sessions {
        let model = s.model.clone().unwrap_or_else(|| "unknown".to_string());
        let entry = map.entry(model.clone()).or_insert_with(|| ModelStats {
            model,
            sessions: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            tool_calls: 0,
        });
        entry.sessions += 1;
        entry.input_tokens += s.input_tokens as u64;
        entry.output_tokens += s.output_tokens as u64;
        entry.total_tokens += (s.input_tokens + s.output_tokens) as u64;
        entry.tool_calls += s.tool_call_count as usize;
    }
    let mut result: Vec<ModelStats> = map.into_values().collect();
    result.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    result
}

fn compute_platform_breakdown(sessions: &[SessionRow]) -> Vec<PlatformStats> {
    let mut map: BTreeMap<String, PlatformStats> = BTreeMap::new();
    for s in sessions {
        let platform = s.source.clone().unwrap_or_else(|| "unknown".to_string());
        let entry = map
            .entry(platform.clone())
            .or_insert_with(|| PlatformStats {
                platform,
                sessions: 0,
                messages: 0,
                tokens: 0,
            });
        entry.sessions += 1;
        entry.messages += s.message_count as usize;
        entry.tokens += (s.input_tokens + s.output_tokens) as u64;
    }
    let mut result: Vec<PlatformStats> = map.into_values().collect();
    result.sort_by(|a, b| b.sessions.cmp(&a.sessions));
    result
}

fn compute_tool_breakdown(sessions: &[SessionRow]) -> Vec<ToolStats> {
    let total: usize = sessions.iter().map(|s| s.tool_call_count as usize).sum();
    if total == 0 {
        return Vec::new();
    }
    vec![ToolStats {
        tool: "total_tool_calls".to_string(),
        count: total,
        percentage: 100.0,
    }]
}

fn compute_activity_patterns(sessions: &[SessionRow]) -> ActivityPatterns {
    let mut day_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut hour_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut daily_counts: BTreeMap<String, usize> = BTreeMap::new();

    for s in sessions {
        if let Some(dt) = s.started_dt() {
            *day_counts
                .entry(dt.weekday().num_days_from_monday())
                .or_insert(0) += 1;
            *hour_counts.entry(dt.hour()).or_insert(0) += 1;
            *daily_counts
                .entry(dt.format("%Y-%m-%d").to_string())
                .or_insert(0) += 1;
        }
    }

    let day_names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let busiest_day = day_counts
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(day, _)| day_names[*day as usize].to_string());
    let busiest_hour = hour_counts
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(hour, _)| *hour);

    let mut max_streak = 0usize;
    if !daily_counts.is_empty() {
        let dates: Vec<&String> = daily_counts.keys().collect();
        let mut current_streak = 1usize;
        for i in 1..dates.len() {
            if let (Ok(d1), Ok(d2)) = (
                chrono::NaiveDate::parse_from_str(dates[i - 1], "%Y-%m-%d"),
                chrono::NaiveDate::parse_from_str(dates[i], "%Y-%m-%d"),
            ) {
                if (d2 - d1).num_days() == 1 {
                    current_streak += 1;
                    max_streak = max_streak.max(current_streak);
                } else {
                    current_streak = 1;
                }
            }
        }
    }

    ActivityPatterns {
        busiest_day,
        busiest_hour,
        active_days: daily_counts.len(),
        max_streak,
    }
}

fn compute_top_sessions(sessions: &[SessionRow]) -> Vec<SessionHighlight> {
    let mut highlights = Vec::new();
    if let Some((s, dur)) = sessions
        .iter()
        .filter_map(|s| s.duration_secs().map(|d| (s, d)))
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    {
        highlights.push(SessionHighlight {
            label: "Longest session".to_string(),
            value: format_duration(dur),
            date: format_naive(s.started_dt()),
        });
    }
    if let Some(s) = sessions.iter().max_by_key(|s| s.message_count) {
        if s.message_count > 0 {
            highlights.push(SessionHighlight {
                label: "Most messages".to_string(),
                value: format!("{} msgs", s.message_count),
                date: format_naive(s.started_dt()),
            });
        }
    }
    if let Some(s) = sessions
        .iter()
        .max_by_key(|s| s.input_tokens + s.output_tokens)
    {
        let total = s.input_tokens + s.output_tokens;
        if total > 0 {
            highlights.push(SessionHighlight {
                label: "Most tokens".to_string(),
                value: format!("{} tokens", total),
                date: format_naive(s.started_dt()),
            });
        }
    }
    highlights
}

fn format_report_terminal(report: &InsightsReport) -> String {
    if report.empty {
        return format!("  No sessions found in the last {} days.", report.days);
    }
    let mut lines = Vec::new();
    let o = &report.overview;
    lines.push(String::new());
    lines.push("  ╔══════════════════════════════════════════════════════════╗".to_string());
    lines.push("  ║                  📊 Operant Insights                    ║".to_string());
    let period = format!("Last {} days", report.days);
    let padding = 58usize.saturating_sub(period.len()).saturating_sub(2);
    let left = padding / 2;
    let right = padding - left;
    lines.push(format!(
        "  ║{} {} {}║",
        " ".repeat(left),
        period,
        " ".repeat(right)
    ));
    lines.push("  ╚══════════════════════════════════════════════════════════╝".to_string());
    lines.push(String::new());
    lines.push("  📋 Overview".to_string());
    lines.push("  ────────────────────────────────────".to_string());
    lines.push(format!(
        "  Sessions:          {:<12}  Messages:        {}",
        o.total_sessions,
        fmt_num(o.total_messages)
    ));
    lines.push(format!(
        "  Total tokens:      {}",
        fmt_num(o.total_tokens as usize)
    ));
    lines.push(format!(
        "  Avg msgs/session:  {:.1}",
        o.avg_messages_per_session
    ));

    if !report.models.is_empty() {
        lines.push(String::new());
        lines.push("  🤖 Models Used".to_string());
        lines.push("  ────────────────────────────────────".to_string());
        for m in &report.models {
            let name = if m.model.len() > 28 {
                &m.model[..28]
            } else {
                &m.model
            };
            lines.push(format!(
                "  {:<30} {:>8} {:>12}",
                name,
                m.sessions,
                fmt_num(m.total_tokens as usize)
            ));
        }
    }

    let act = &report.activity;
    if let Some(day) = &act.busiest_day {
        lines.push(String::new());
        lines.push("  📅 Activity".to_string());
        lines.push(format!("  Busiest day: {}", day));
        if let Some(hour) = act.busiest_hour {
            let (h, ampm) = if hour < 12 {
                (hour, "AM")
            } else {
                (hour - 12, "PM")
            };
            lines.push(format!("  Busiest hour: {}{}", h, ampm));
        }
        if act.max_streak > 1 {
            lines.push(format!("  Best streak: {} days", act.max_streak));
        }
    }

    if !report.top_sessions.is_empty() {
        lines.push(String::new());
        lines.push("  🏆 Notable Sessions".to_string());
        for ts in &report.top_sessions {
            lines.push(format!("  {:<20} {:<18} ({})", ts.label, ts.value, ts.date));
        }
    }
    lines.join("\n")
}

fn format_report_gateway(report: &InsightsReport) -> String {
    if report.empty {
        return format!("No sessions found in the last {} days.", report.days);
    }
    let o = &report.overview;
    let mut lines = Vec::new();
    lines.push(format!(
        "📊 **Operant Insights** — Last {} days\n",
        report.days
    ));
    lines.push(format!(
        "**Sessions:** {} | **Tokens:** {}",
        o.total_sessions,
        fmt_num(o.total_tokens as usize)
    ));
    if let (Some(day), Some(hour)) = (&report.activity.busiest_day, report.activity.busiest_hour) {
        let (h, ampm) = if hour < 12 {
            (hour, "AM")
        } else {
            (hour - 12, "PM")
        };
        lines.push(format!("**Busiest:** {}s, {}{}", day, h, ampm));
    }
    lines.join("\n")
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn format_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{}s", secs as u64)
    } else if secs < 3600.0 {
        format!("{}m", (secs / 60.0) as u64)
    } else {
        format!("{:.1}h", secs / 3600.0)
    }
}

fn format_naive(dt: Option<chrono::NaiveDateTime>) -> String {
    dt.map(|d| d.format("%b %d").to_string())
        .unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fmt_num() {
        assert_eq!(fmt_num(0), "0");
        assert_eq!(fmt_num(1000), "1,000");
        assert_eq!(fmt_num(1234567), "1,234,567");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30.0), "30s");
        assert_eq!(format_duration(90.0), "1m");
        assert_eq!(format_duration(3600.0), "1.0h");
    }

    #[test]
    fn test_overview_default() {
        let s = OverviewStats::default();
        assert_eq!(s.total_sessions, 0);
    }
}
