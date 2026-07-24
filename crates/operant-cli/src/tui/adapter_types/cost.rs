#[derive(Debug, Clone, Default)]
pub struct CostTracker {
    pub total_cost: f64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub model: String,
}

impl CostTracker {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn record_usage(&mut self, input: u32, output: u32) {
        self.input_tokens += input;
        self.output_tokens += output;
    }
    /// Accumulate a real per-request cost (from `AgentEvent::Cost`'s
    /// models_dev-sourced estimate, or a flat-rate fallback when the
    /// model isn't in the models_dev catalog).
    pub fn record_cost(&mut self, cost_usd: f64) {
        self.total_cost += cost_usd;
    }
    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
    }
}
