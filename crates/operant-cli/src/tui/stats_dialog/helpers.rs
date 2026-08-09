// stats_dialog/helpers.rs — Stats aggregation helpers.
//
// Extracted from the stats_dialog.rs monolith. Model breakdown building,
// streak computation, and date helpers.

use super::*;

pub(crate) fn build_model_breakdown(stats: &AggregatedStats) -> Vec<ModelBreakdown> {
    let mut breakdown: Vec<ModelBreakdown> = stats
        .by_model
        .iter()
        .map(|(model_id, ms)| ModelBreakdown {
            model_id: model_id.clone(),
            input_tokens: ms.input_tokens,
            output_tokens: ms.output_tokens,
            cost_usd: ms.cost_cents / 100.0,
        })
        .collect();
    breakdown.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    breakdown
}

/// Compute (current_streak, longest_streak) in days from the aggregated stats.
/// A streak is a consecutive run of calendar days with any activity, ending on
/// the most-recent active day.
pub(crate) fn compute_streaks(stats: &AggregatedStats) -> (u32, u32) {
    if stats.daily_tokens.is_empty() {
        return (0, 0);
    }

    // Collect sorted unique active dates
    let mut dates: Vec<&str> = stats.daily_tokens.iter().map(|(d, _)| d.as_str()).collect();
    dates.dedup();

    let mut longest: u32 = 1;
    let mut current_run: u32 = 1;

    for window in dates.windows(2) {
        if consecutive_dates(window[0], window[1]) {
            current_run += 1;
            if current_run > longest {
                longest = current_run;
            }
        } else {
            current_run = 1;
        }
    }

    // The "current" streak is the run ending on the last active date.
    // Recompute from the end.
    let mut current_streak: u32 = 1;
    for window in dates.windows(2).rev() {
        if consecutive_dates(window[0], window[1]) {
            current_streak += 1;
        } else {
            break;
        }
    }

    (current_streak, longest)
}

/// Returns true when `next` is exactly one calendar day after `prev`.
/// Both strings must be "YYYY-MM-DD".
pub(crate) fn consecutive_dates(prev: &str, next: &str) -> bool {
    let prev_days = date_to_days_since_epoch(prev);
    let next_days = date_to_days_since_epoch(next);
    match (prev_days, next_days) {
        (Some(p), Some(n)) => n == p + 1,
        _ => false,
    }
}

fn date_to_days_since_epoch(date: &str) -> Option<u64> {
    // Expect "YYYY-MM-DD"
    if date.len() != 10 {
        return None;
    }
    let year: u64 = date[0..4].parse().ok()?;
    let month: u64 = date[5..7].parse().ok()?;
    let day: u64 = date[8..10].parse().ok()?;
    // Days from 1970-01-01 (approximate, good enough for streak detection)
    let y = year - 1970;
    let leap_days = if y > 0 {
        (y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400 + 1
    } else {
        0
    };
    let days_in_years = y * 365 + leap_days;
    let leap = is_leap_year(year as u32);
    let months = if leap {
        [0u64, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335]
    } else {
        [0u64, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
    };
    let month_days = months.get((month as usize).saturating_sub(1))?;
    Some(days_in_years + month_days + day - 1)
}
