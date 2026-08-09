// stats_dialog/state.rs — StatsDialogState methods + Default.
//
// Extracted from the stats_dialog.rs monolith.

use super::*;

impl StatsDialogState {
    pub fn new() -> Self {
        Self {
            visible: false,
            tab: StatsTab::Overview,
            range_days: 30,
            data: None,
            scroll: 0,
            model_breakdown: Vec::new(),
            current_streak_days: 0,
            longest_streak_days: 0,
        }
    }

    pub fn open(&mut self) {
        let stats = load_stats();
        self.model_breakdown = build_model_breakdown(&stats);
        let (current, longest) = compute_streaks(&stats);
        self.current_streak_days = current;
        self.longest_streak_days = longest;
        self.data = Some(stats);
        self.visible = true;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn next_tab(&mut self) {
        self.tab = match self.tab {
            StatsTab::Overview => StatsTab::DailyTokens,
            StatsTab::DailyTokens => StatsTab::CostHeatmap,
            StatsTab::CostHeatmap => StatsTab::Models,
            StatsTab::Models => StatsTab::Overview,
        };
        self.scroll = 0;
    }

    pub fn prev_tab(&mut self) {
        self.tab = match self.tab {
            StatsTab::Overview => StatsTab::Models,
            StatsTab::DailyTokens => StatsTab::Overview,
            StatsTab::CostHeatmap => StatsTab::DailyTokens,
            StatsTab::Models => StatsTab::CostHeatmap,
        };
        self.scroll = 0;
    }

    pub fn cycle_range(&mut self) {
        self.range_days = match self.range_days {
            7 => 30,
            30 => 0,
            _ => 7,
        };
    }

    /// Record usage for a model, accumulating into `model_breakdown`.
    /// `cost` is in USD (not cents).
    #[allow(dead_code)] // Model usage tracking
    pub fn add_model_usage(&mut self, model_id: &str, input: u64, output: u64, cost: f64) {
        #[allow(dead_code)]
        if let Some(entry) = self
            .model_breakdown
            .iter_mut()
            .find(|e| e.model_id == model_id)
        {
            entry.input_tokens += input;
            entry.output_tokens += output;
            entry.cost_usd += cost;
        } else {
            self.model_breakdown.push(ModelBreakdown {
                model_id: model_id.to_string(),
                input_tokens: input,
                output_tokens: output,
                cost_usd: cost,
            });
        }
    }
}

impl Default for StatsDialogState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers: build model breakdown and compute streaks
// ---------------------------------------------------------------------------
