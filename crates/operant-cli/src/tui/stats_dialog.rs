//! Stats dialog — mirrors src/components/Stats.tsx
//!
//! Four-tab overlay: Overview | Daily Tokens | Cost Heatmap | Models
//! Data source: ~/.operant/stats.jsonl (append-only per-turn usage log)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::tui::overlays::{
    OPERANT_ACCENT, OPERANT_MUTED, OPERANT_PANEL_BG, begin_modal_buf, modal_header_line_area,
    render_modal_title_buf,
};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single entry in ~/.operant/stats.jsonl
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatsEntry {
    pub timestamp_ms: u64,
    pub session_id: Option<String>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Cost in USD cents (f64)
    pub cost_cents: f64,
    pub project: Option<String>,
}

/// Aggregated stats for display.
#[derive(Debug, Clone, Default)]
pub struct AggregatedStats {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_cents: f64,
    pub by_model: HashMap<String, ModelStats>,
    /// (date_str "YYYY-MM-DD", tokens) pairs sorted by date
    pub daily_tokens: Vec<(String, u64)>,
    /// (date_str "YYYY-MM-DD", cost_cents) for heatmap
    pub daily_costs: HashMap<String, f64>,
    pub peak_day: Option<String>,
    pub peak_day_tokens: u64,
}

/// Per-model usage stats (used in AggregatedStats.by_model).
#[derive(Debug, Clone, Default)]
pub struct ModelStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_cents: f64,
    pub turns: u64,
}

/// Per-model breakdown entry for the Models tab (cost in USD, not cents).
#[derive(Debug, Clone, Default)]
pub struct ModelBreakdown {
    pub model_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

/// Load and aggregate stats from ~/.operant/stats.jsonl
pub fn load_stats() -> AggregatedStats {
    let path = dirs::home_dir()
        .map(|h| h.join(".operant").join("stats.jsonl"))
        .unwrap_or_default();

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return AggregatedStats::default(),
    };

    let mut agg = AggregatedStats::default();
    let mut daily: HashMap<String, u64> = HashMap::new();

    for line in content.lines() {
        let Ok(entry) = serde_json::from_str::<StatsEntry>(line) else {
            continue;
        };

        let total_tokens = entry.input_tokens + entry.output_tokens;
        agg.total_input_tokens += entry.input_tokens;
        agg.total_output_tokens += entry.output_tokens;
        agg.total_cost_cents += entry.cost_cents;

        let model_entry = agg.by_model.entry(entry.model.clone()).or_default();
        model_entry.input_tokens += entry.input_tokens;
        model_entry.output_tokens += entry.output_tokens;
        model_entry.cost_cents += entry.cost_cents;
        model_entry.turns += 1;

        // Date from timestamp
        let date = timestamp_to_date(entry.timestamp_ms);
        *daily.entry(date.clone()).or_insert(0) += total_tokens;
        *agg.daily_costs.entry(date).or_insert(0.0) += entry.cost_cents;
    }

    // Build sorted daily_tokens
    let mut daily_sorted: Vec<(String, u64)> = daily.into_iter().collect();
    daily_sorted.sort_by(|a, b| a.0.cmp(&b.0));
    agg.peak_day = daily_sorted.iter().max_by_key(|d| d.1).map(|d| d.0.clone());
    agg.peak_day_tokens = daily_sorted.iter().map(|d| d.1).max().unwrap_or(0);
    agg.daily_tokens = daily_sorted;

    agg
}

fn timestamp_to_date(ts_ms: u64) -> String {
    // Simple ISO date from Unix timestamp in ms
    let secs = ts_ms / 1000;
    let days_since_epoch = secs / 86400;
    // Rough Gregorian calendar calculation
    let year = 1970 + (days_since_epoch * 4 + 2) / 1461;
    let day_of_year = days_since_epoch - (year - 1970) * 365 - (year - 1970 - 1) / 4;
    let (month, day) = day_of_year_to_month_day(day_of_year as u32, is_leap_year(year as u32));
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn is_leap_year(year: u32) -> bool {
    // Modulo instead of `is_multiple_of` (stable since 1.87; MSRV is 1.85).
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn day_of_year_to_month_day(doy: u32, leap: bool) -> (u32, u32) {
    let months = if leap {
        [31u32, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut remaining = doy;
    for (i, &m) in months.iter().enumerate() {
        if remaining < m {
            return (i as u32 + 1, remaining + 1);
        }
        remaining -= m;
    }
    (12, 31)
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsTab {
    Overview,
    DailyTokens,
    CostHeatmap,
    Models,
}

#[derive(Debug, Clone)]
pub struct StatsDialogState {
    pub visible: bool,
    pub tab: StatsTab,
    pub range_days: u32, // 7, 30, or 0 = all
    pub data: Option<AggregatedStats>,
    pub scroll: u16,
    /// Per-model breakdown for the Models tab (cost in USD).
    pub model_breakdown: Vec<ModelBreakdown>,
    /// How many consecutive days the user has had activity (ending today).
    pub current_streak_days: u32,
    /// The longest streak ever recorded.
    pub longest_streak_days: u32,
}

mod helpers;
mod render;
mod state;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use helpers::consecutive_dates;
pub(crate) use helpers::{build_model_breakdown, compute_streaks};
#[cfg(test)]
pub(crate) use render::heatmap_color;
pub use render::render_stats_dialog;
