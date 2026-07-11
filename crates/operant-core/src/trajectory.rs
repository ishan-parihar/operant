//! Trajectory saving for RL training data generation
//!
//! Exports conversation trajectories in formats suitable for
//! reinforcement learning training (e.g., RLHF, RLAIF).

use crate::client::Message;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single step in a trajectory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    /// Step index in the trajectory
    pub step: usize,
    /// The agent's reasoning (thought process)
    pub thought: Option<String>,
    /// Action taken (tool call name)
    pub action: Option<String>,
    /// Action arguments
    pub action_args: Option<String>,
    /// Observation/result from the action
    pub observation: Option<String>,
    /// Final response if this was the last step
    pub response: Option<String>,
    /// Whether this step was successful
    pub success: bool,
}

/// A complete trajectory (conversation) for training
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    /// Unique trajectory identifier
    pub id: String,
    /// Session ID this trajectory came from
    pub session_id: String,
    /// Model used
    pub model: String,
    /// Timestamp when trajectory was created
    pub timestamp: i64,
    /// Total tokens used
    pub total_tokens: usize,
    /// Number of tool calls made
    pub tool_calls: usize,
    /// Number of iterations
    pub iterations: usize,
    /// Whether the trajectory was successful
    pub success: bool,
    /// The individual steps
    pub steps: Vec<TrajectoryStep>,
    /// Messages in the conversation
    pub messages: Vec<Message>,
    /// Metadata about the trajectory
    pub metadata: HashMap<String, String>,
}

impl Trajectory {
    /// Create a new trajectory
    pub fn new(
        id: impl Into<String>,
        session_id: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        Self {
            id: id.into(),
            session_id: session_id.into(),
            model: model.into(),
            timestamp,
            total_tokens: 0,
            tool_calls: 0,
            iterations: 0,
            success: false,
            steps: Vec::new(),
            messages: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Add a step to the trajectory
    pub fn add_step(&mut self, step: TrajectoryStep) {
        self.steps.push(step);
    }

    /// Add a message to the trajectory
    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Set success status
    pub fn set_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    /// Set metadata
    pub fn set_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Calculate total tokens from all messages
    pub fn calculate_tokens(&mut self) {
        self.total_tokens = self
            .messages
            .iter()
            .map(crate::context_management::estimate_message_tokens)
            .sum();
    }

    /// Convert to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Convert to compact JSON string
    pub fn to_json_compact(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}
