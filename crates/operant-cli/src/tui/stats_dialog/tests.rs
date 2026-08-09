// stats_dialog/tests.rs — Unit tests for stats aggregation and rendering.
//
// Extracted from the stats_dialog.rs monolith.

use super::*;
use ratatui::style::Color;

// ---- helpers -----------------------------------------------------------

fn make_state_with_models(entries: &[(&str, u64, u64, f64)]) -> StatsDialogState {
    let mut state = StatsDialogState::new();
    for (model, input, output, cost) in entries {
        state.add_model_usage(model, *input, *output, *cost);
    }
    state
}

fn make_agg_with_dates(dates: &[&str]) -> AggregatedStats {
    let mut agg = AggregatedStats::default();
    for date in dates {
        agg.daily_tokens.push((date.to_string(), 100));
    }
    agg
}

// ---- model breakdown: add_model_usage ----------------------------------

#[test]
fn test_add_model_usage_new_model() {
    let mut state = StatsDialogState::new();
    state.add_model_usage("claude-3-opus", 1000, 500, 0.05);

    assert_eq!(state.model_breakdown.len(), 1);
    let e = &state.model_breakdown[0];
    assert_eq!(e.model_id, "claude-3-opus");
    assert_eq!(e.input_tokens, 1000);
    assert_eq!(e.output_tokens, 500);
    assert!((e.cost_usd - 0.05).abs() < 1e-9);
}

#[test]
fn test_add_model_usage_accumulates_same_model() {
    let mut state = StatsDialogState::new();
    state.add_model_usage("claude-3-opus", 1000, 500, 0.05);
    state.add_model_usage("claude-3-opus", 2000, 800, 0.10);

    assert_eq!(state.model_breakdown.len(), 1);
    let e = &state.model_breakdown[0];
    assert_eq!(e.input_tokens, 3000);
    assert_eq!(e.output_tokens, 1300);
    assert!((e.cost_usd - 0.15).abs() < 1e-9);
}

#[test]
fn test_add_model_usage_multiple_models() {
    let state = make_state_with_models(&[
        ("claude-3-opus", 1000, 500, 0.05),
        ("claude-3-haiku", 500, 200, 0.01),
        ("claude-3-sonnet", 800, 400, 0.03),
    ]);

    assert_eq!(state.model_breakdown.len(), 3);
    let ids: Vec<&str> = state
        .model_breakdown
        .iter()
        .map(|e| e.model_id.as_str())
        .collect();
    assert!(ids.contains(&"claude-3-opus"));
    assert!(ids.contains(&"claude-3-haiku"));
    assert!(ids.contains(&"claude-3-sonnet"));
}

#[test]
fn test_model_breakdown_totals() {
    let state = make_state_with_models(&[
        ("model-a", 1_000_000, 200_000, 1.00),
        ("model-b", 500_000, 100_000, 0.50),
    ]);
    let total_input: u64 = state.model_breakdown.iter().map(|e| e.input_tokens).sum();
    let total_output: u64 = state.model_breakdown.iter().map(|e| e.output_tokens).sum();
    let total_cost: f64 = state.model_breakdown.iter().map(|e| e.cost_usd).sum();
    assert_eq!(total_input, 1_500_000);
    assert_eq!(total_output, 300_000);
    assert!((total_cost - 1.50).abs() < 1e-9);
}

// ---- streak tracking ---------------------------------------------------

#[test]
fn test_streak_consecutive_days() {
    let agg = make_agg_with_dates(&["2025-01-01", "2025-01-02", "2025-01-03"]);
    let (current, longest) = compute_streaks(&agg);
    assert_eq!(current, 3);
    assert_eq!(longest, 3);
}

#[test]
fn test_streak_gap_resets_current() {
    // Two separate runs: 3 days then a gap, then 2 days.
    let agg = make_agg_with_dates(&[
        "2025-01-01",
        "2025-01-02",
        "2025-01-03",
        "2025-01-10",
        "2025-01-11",
    ]);
    let (current, longest) = compute_streaks(&agg);
    assert_eq!(current, 2);
    assert_eq!(longest, 3);
}

#[test]
fn test_streak_single_day() {
    let agg = make_agg_with_dates(&["2025-03-15"]);
    let (current, longest) = compute_streaks(&agg);
    assert_eq!(current, 1);
    assert_eq!(longest, 1);
}

#[test]
fn test_streak_empty() {
    let agg = AggregatedStats::default();
    let (current, longest) = compute_streaks(&agg);
    assert_eq!(current, 0);
    assert_eq!(longest, 0);
}

#[test]
fn test_streak_longer_tail_wins_longest() {
    // Five days, then a gap, then one day.
    let agg = make_agg_with_dates(&[
        "2025-02-01",
        "2025-02-02",
        "2025-02-03",
        "2025-02-04",
        "2025-02-05",
        "2025-02-20",
    ]);
    let (current, longest) = compute_streaks(&agg);
    assert_eq!(current, 1);
    assert_eq!(longest, 5);
}

#[test]
fn test_consecutive_dates_helper() {
    assert!(consecutive_dates("2025-01-31", "2025-02-01"));
    assert!(consecutive_dates("2024-02-28", "2024-02-29")); // 2024 is a leap year
    assert!(!consecutive_dates("2025-01-01", "2025-01-03"));
    assert!(!consecutive_dates("2025-01-05", "2025-01-04")); // reversed
}

// ---- heatmap color -----------------------------------------------------

#[test]
fn test_heatmap_color_zero() {
    assert_eq!(heatmap_color(0.0), Color::Rgb(30, 30, 30));
}

#[test]
fn test_heatmap_color_max() {
    assert_eq!(heatmap_color(1.0), Color::Rgb(0, 255, 0));
}

#[test]
fn test_heatmap_color_mid() {
    // 0.60 -> high bracket
    assert_eq!(heatmap_color(0.60), Color::Rgb(0, 200, 0));
}

// ---- build_model_breakdown sorting -------------------------------------

#[test]
fn test_build_model_breakdown_sorted_by_cost_desc() {
    let mut agg = AggregatedStats::default();
    agg.by_model.insert(
        "cheap".to_string(),
        ModelStats {
            input_tokens: 100,
            output_tokens: 50,
            cost_cents: 10.0,
            turns: 1,
        },
    );
    agg.by_model.insert(
        "expensive".to_string(),
        ModelStats {
            input_tokens: 200,
            output_tokens: 100,
            cost_cents: 500.0,
            turns: 2,
        },
    );
    agg.by_model.insert(
        "mid".to_string(),
        ModelStats {
            input_tokens: 150,
            output_tokens: 75,
            cost_cents: 100.0,
            turns: 1,
        },
    );

    let breakdown = build_model_breakdown(&agg);
    assert_eq!(breakdown[0].model_id, "expensive");
    assert_eq!(breakdown[1].model_id, "mid");
    assert_eq!(breakdown[2].model_id, "cheap");
}
