//! RL Training module — reinforcement learning for agent decision-making.
//!
//! Implements table-based Q-learning for improving agent action selection
//! over time. Ported from concept in operant-agent/rl_cli.py.
//!
//! ## Architecture
//!
//! - `QTable`: Core value-function table mapping (state, action) → Q-value
//! - `RlState`: Persistent training metadata (episodes, rewards, epsilon)
//! - `RlTrainer`: Orchestrator that manages the training loop and persistence
//!
//! The Q-table is persisted as JSON alongside the training state, loaded from
//! a configurable path (default: `~/.operant/rl/`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::info;

/// Default exploration rate (epsilon) — fraction of actions chosen randomly.
const DEFAULT_EPSILON: f64 = 0.3;
/// Minimum epsilon after decay.
const MIN_EPSILON: f64 = 0.01;
/// Epsilon decay rate per episode.
const EPSILON_DECAY: f64 = 0.995;
/// Default learning rate (alpha).
const DEFAULT_LEARNING_RATE: f64 = 0.1;
/// Default discount factor (gamma).
const DEFAULT_DISCOUNT_FACTOR: f64 = 0.9;

/// A single action with its Q-value for display/tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionValue {
    pub action: String,
    pub value: f64,
}

/// A Q-value entry linking a state to all known actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QStateEntry {
    pub state: String,
    pub actions: HashMap<String, f64>,
}

/// The Q-table: maps state keys to action-value maps.
///
/// Thread-safe via `Arc<Mutex<...>>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QTable {
    /// state_key → { action_key → q_value }
    table: HashMap<String, HashMap<String, f64>>,
    /// Number of times each (state, action) pair has been visited.
    visit_counts: HashMap<String, HashMap<String, u64>>,
}

impl QTable {
    /// Create a new empty Q-table.
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
            visit_counts: HashMap::new(),
        }
    }

    /// Get the Q-value for a state-action pair.
    /// Returns `None` if no value has been learned yet.
    pub fn get_q_value(&self, state: &str, action: &str) -> Option<f64> {
        self.table.get(state)?.get(action).copied()
    }

    /// Set the Q-value for a state-action pair.
    pub fn set_q_value(&mut self, state: &str, action: &str, value: f64) {
        self.table
            .entry(state.to_string())
            .or_default()
            .insert(action.to_string(), value);
    }

    /// Update Q-value using the Bellman equation:
    /// Q(s,a) ← Q(s,a) + α * (reward + γ * max Q(s',a') - Q(s,a))
    ///
    /// Returns the new Q-value.
    pub fn update(
        &mut self,
        state: &str,
        action: &str,
        reward: f64,
        next_state: &str,
        learning_rate: f64,
        discount_factor: f64,
    ) -> f64 {
        let current_q = self.get_q_value(state, action).unwrap_or(0.0);
        let max_next_q = self.get_max_q_value(next_state).unwrap_or(0.0);
        let td_error = reward + discount_factor * max_next_q - current_q;
        let new_q = current_q + learning_rate * td_error;

        self.set_q_value(state, action, new_q);

        // Track visit count
        self.visit_counts
            .entry(state.to_string())
            .or_default()
            .entry(action.to_string())
            .and_modify(|c| *c += 1)
            .or_insert(1);

        new_q
    }

    /// Get the maximum Q-value for a state across all known actions.
    pub fn get_max_q_value(&self, state: &str) -> Option<f64> {
        self.table.get(state).and_then(|actions| {
            actions
                .values()
                .cloned()
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        })
    }

    /// Get the best action for a state (highest Q-value).
    /// Returns `None` if the state has no known actions.
    pub fn get_best_action(&self, state: &str) -> Option<(String, f64)> {
        self.table.get(state).and_then(|actions| {
            actions
                .iter()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(action, value)| (action.clone(), *value))
        })
    }

    /// Select an action using epsilon-greedy policy.
    /// - With probability ε, choose a random action.
    /// - Otherwise, choose the action with the highest Q-value.
    pub fn select_action(
        &self,
        state: &str,
        available_actions: &[String],
        epsilon: f64,
    ) -> Option<String> {
        if available_actions.is_empty() {
            return None;
        }

        let mut rng = rand::thread_rng();
        if rng.gen::<f64>() < epsilon {
            // Explore: random action
            let idx = rng.gen_range(0..available_actions.len());
            Some(available_actions[idx].clone())
        } else {
            // Exploit: best known action among available ones
            let mut best_action = available_actions[0].clone();
            let mut best_value = self.get_q_value(state, &best_action).unwrap_or(0.0);

            for action in available_actions.iter().skip(1) {
                let value = self.get_q_value(state, action).unwrap_or(0.0);
                if value > best_value {
                    best_value = value;
                    best_action = action.clone();
                }
            }
            Some(best_action)
        }
    }

    /// Get all entries in the table for display/serialization.
    pub fn entries(&self) -> Vec<QStateEntry> {
        let mut entries: Vec<QStateEntry> = self
            .table
            .iter()
            .map(|(state, actions)| QStateEntry {
                state: state.clone(),
                actions: actions.clone(),
            })
            .collect();
        entries.sort_by(|a, b| a.state.cmp(&b.state));
        entries
    }

    /// Number of unique states in the table.
    pub fn state_count(&self) -> usize {
        self.table.len()
    }

    /// Total number of (state, action) pairs.
    pub fn entry_count(&self) -> usize {
        self.table.values().map(|m| m.len()).sum()
    }

    /// Visit count for a specific state-action pair.
    pub fn visit_count(&self, state: &str, action: &str) -> u64 {
        self.visit_counts
            .get(state)
            .and_then(|m| m.get(action))
            .copied()
            .unwrap_or(0)
    }
}

impl Default for QTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Training state metadata persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlState {
    /// Total episodes completed.
    pub episodes: u64,
    /// Total training steps completed.
    pub total_steps: u64,
    /// Cumulative reward across all training.
    pub cumulative_reward: f64,
    /// Current exploration rate.
    pub epsilon: f64,
    /// Learning rate (alpha).
    pub learning_rate: f64,
    /// Discount factor (gamma).
    pub discount_factor: f64,
    /// Timestamp of last training run (ISO 8601).
    pub last_trained: Option<String>,
    /// Average reward over the last N episodes.
    pub last_avg_reward: Option<f64>,
    /// Best episode reward so far.
    pub best_episode_reward: f64,
    /// Total training runs started.
    pub training_runs: u64,
}

impl Default for RlState {
    fn default() -> Self {
        Self {
            episodes: 0,
            total_steps: 0,
            cumulative_reward: 0.0,
            epsilon: DEFAULT_EPSILON,
            learning_rate: DEFAULT_LEARNING_RATE,
            discount_factor: DEFAULT_DISCOUNT_FACTOR,
            last_trained: None,
            last_avg_reward: None,
            best_episode_reward: f64::NEG_INFINITY,
            training_runs: 0,
        }
    }
}

/// Result of a single training step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub state: String,
    pub action: String,
    pub reward: f64,
    pub next_state: String,
    pub new_q_value: f64,
}

/// Summary of an episode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeResult {
    pub episode: u64,
    pub steps: usize,
    pub total_reward: f64,
    pub avg_reward: f64,
    pub epsilon: f64,
    pub unique_states: usize,
}

/// Summary of a training run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingSummary {
    pub episodes_completed: u64,
    pub total_steps: u64,
    pub cumulative_reward: f64,
    pub best_episode_reward: f64,
    pub final_epsilon: f64,
    pub unique_states: usize,
    pub total_q_entries: usize,
    pub duration_seconds: f64,
}

/// The main RL trainer — manages Q-table, training state, and persistence.
#[derive(Clone)]
pub struct RlTrainer {
    q_table: Arc<Mutex<QTable>>,
    state: Arc<Mutex<RlState>>,
    q_table_path: PathBuf,
    state_path: PathBuf,
}

impl RlTrainer {
    /// Create a new RL trainer with persistence in the given directory.
    pub fn new(rl_dir: PathBuf) -> Self {
        fs::create_dir_all(&rl_dir).ok();
        Self {
            q_table: Arc::new(Mutex::new(QTable::new())),
            state: Arc::new(Mutex::new(RlState::default())),
            q_table_path: rl_dir.join("q_table.json"),
            state_path: rl_dir.join("rl_state.json"),
        }
    }

    /// Load state from disk (creates defaults if not found).
    pub fn load(&self) -> crate::error::Result<()> {
        // Load Q-table
        if self.q_table_path.exists() {
            let content = fs::read_to_string(&self.q_table_path).map_err(|e| {
                crate::error::Error::Agent(format!("Failed to read Q-table: {}", e))
            })?;
            let table: QTable = serde_json::from_str(&content).map_err(|e| {
                crate::error::Error::Agent(format!("Failed to parse Q-table: {}", e))
            })?;
            *self.q_table.lock().unwrap() = table;
        }

        // Load RL state
        if self.state_path.exists() {
            let content = fs::read_to_string(&self.state_path).map_err(|e| {
                crate::error::Error::Agent(format!("Failed to read RL state: {}", e))
            })?;
            let rl_state: RlState = serde_json::from_str(&content).map_err(|e| {
                crate::error::Error::Agent(format!("Failed to parse RL state: {}", e))
            })?;
            *self.state.lock().unwrap() = rl_state;
        }

        info!(
            "RL trainer loaded: {} states, {} Q-entries, {} episodes",
            self.q_table.lock().unwrap().state_count(),
            self.q_table.lock().unwrap().entry_count(),
            self.state.lock().unwrap().episodes,
        );

        Ok(())
    }

    /// Save current Q-table and state to disk.
    pub fn save(&self) -> crate::error::Result<()> {
        // Save Q-table
        let table_json =
            serde_json::to_string_pretty(&*self.q_table.lock().unwrap()).map_err(|e| {
                crate::error::Error::Agent(format!("Failed to serialize Q-table: {}", e))
            })?;
        fs::write(&self.q_table_path, &table_json)
            .map_err(|e| crate::error::Error::Agent(format!("Failed to write Q-table: {}", e)))?;

        // Save RL state
        let state_json =
            serde_json::to_string_pretty(&*self.state.lock().unwrap()).map_err(|e| {
                crate::error::Error::Agent(format!("Failed to serialize RL state: {}", e))
            })?;
        fs::write(&self.state_path, &state_json)
            .map_err(|e| crate::error::Error::Agent(format!("Failed to write RL state: {}", e)))?;

        Ok(())
    }

    /// Get the current training state.
    pub fn get_state(&self) -> RlState {
        self.state.lock().unwrap().clone()
    }

    /// Get a mutable reference to the training state (locked).
    pub fn state_write(&self) -> std::sync::MutexGuard<'_, RlState> {
        self.state.lock().unwrap()
    }

    /// Get Q-value for a state-action pair.
    pub fn get_q_value(&self, state: &str, action: &str) -> Option<f64> {
        self.q_table.lock().unwrap().get_q_value(state, action)
    }

    /// Get visit count for a state-action pair.
    pub fn get_visit_count(&self, state: &str, action: &str) -> u64 {
        self.q_table.lock().unwrap().visit_count(state, action)
    }

    /// Get the Q-table for inspection.
    pub fn get_q_table(&self) -> Vec<QStateEntry> {
        self.q_table.lock().unwrap().entries()
    }

    /// Get summary statistics for the Q-table.
    pub fn get_q_table_stats(&self) -> (usize, usize) {
        let table = self.q_table.lock().unwrap();
        (table.state_count(), table.entry_count())
    }

    /// Perform a single training step: update Q-value for (state, action, reward, next_state).
    ///
    /// Returns the new Q-value.
    pub fn train_step(&self, state: &str, action: &str, reward: f64, next_state: &str) -> f64 {
        // IMPORTANT: Lock q_table first, then state — consistent ordering
        // to prevent deadlocks with save() which locks q_table then state.
        let mut table_guard = self.q_table.lock().unwrap();
        let mut state_guard = self.state.lock().unwrap();

        let new_q = table_guard.update(
            state,
            action,
            reward,
            next_state,
            state_guard.learning_rate,
            state_guard.discount_factor,
        );

        state_guard.total_steps += 1;
        state_guard.cumulative_reward += reward;

        new_q
    }

    /// Complete an episode: update metadata, decay epsilon.
    /// Only locks `state` (not `q_table`), safe to call from `run_training_session`.
    pub fn finish_episode(&self, total_reward: f64, steps: usize) {
        let mut state = self.state.lock().unwrap();
        state.episodes += 1;
        state.last_avg_reward = if steps > 0 {
            Some(total_reward / steps as f64)
        } else {
            Some(0.0)
        };

        if total_reward > state.best_episode_reward {
            state.best_episode_reward = total_reward;
        }

        // Decay epsilon
        state.epsilon = (state.epsilon * EPSILON_DECAY).max(MIN_EPSILON);
        state.last_trained = Some(Utc::now().to_rfc3339());
    }

    /// Select an action for a given state using epsilon-greedy.
    pub fn select_action(&self, state: &str, available_actions: &[String]) -> Option<String> {
        let epsilon = self.state.lock().unwrap().epsilon;
        self.q_table
            .lock()
            .unwrap()
            .select_action(state, available_actions, epsilon)
    }

    /// Reset all training state (episodes, rewards, Q-table).
    pub fn reset(&self) -> crate::error::Result<()> {
        *self.q_table.lock().unwrap() = QTable::new();
        *self.state.lock().unwrap() = RlState::default();
        self.save()?;
        info!("RL trainer reset: all Q-values and training state cleared.");
        Ok(())
    }

    /// Get a training status summary for display.
    pub fn status(&self) -> String {
        let state = self.state.lock().unwrap();
        let (state_count, entry_count) = {
            let table = self.q_table.lock().unwrap();
            (table.state_count(), table.entry_count())
        };

        format!(
            r#"RL Training Status
{:─^40}

Episodes completed:  {}
Total steps:         {}
Cumulative reward:   {:.4}
Best episode reward: {:.4}
Average reward:      {}
Exploration rate:    {:.4}
Learning rate:       {:.4}
Discount factor:     {:.4}
Training runs:       {}
Unique states:       {}
Q-table entries:     {}
Last trained:        {}
"#,
            "",
            state.episodes,
            state.total_steps,
            state.cumulative_reward,
            state.best_episode_reward,
            state
                .last_avg_reward
                .map(|v| format!("{:.4}", v))
                .unwrap_or_else(|| "N/A".to_string()),
            state.epsilon,
            state.learning_rate,
            state.discount_factor,
            state.training_runs,
            state_count,
            entry_count,
            state.last_trained.as_deref().unwrap_or("Never"),
        )
    }

    /// Run a simulated training session with a callback that generates
    /// (state, action, reward, next_state) tuples for each step.
    ///
    /// This is useful for automated training where the environment is
    /// simulated or driven by an external loop.
    /// The callback receives (episode_number, step_number) and returns
    /// (state, action, reward, next_state).
    pub fn run_training_session<F>(
        &self,
        episodes: u64,
        steps_per_episode: usize,
        env_fn: F,
    ) -> TrainingSummary
    where
        F: Fn(u64, usize) -> (String, String, f64, String),
    {
        let start = std::time::Instant::now();

        {
            let mut state = self.state.lock().unwrap();
            state.training_runs += 1;
        }

        for ep in 0..episodes {
            let mut ep_reward = 0.0;

            for step in 0..steps_per_episode {
                let (s, a, r, ns) = env_fn(ep + 1, step);
                let _new_q = self.train_step(&s, &a, r, &ns);
                ep_reward += r;
            }

            self.finish_episode(ep_reward, steps_per_episode);
        }

        let duration = start.elapsed().as_secs_f64();
        let (
            episodes_completed,
            total_steps,
            cumulative_reward,
            best_episode_reward,
            final_epsilon,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.episodes,
                s.total_steps,
                s.cumulative_reward,
                s.best_episode_reward,
                s.epsilon,
            )
        };
        let (unique_states, total_q_entries) = {
            let table = self.q_table.lock().unwrap();
            (table.state_count(), table.entry_count())
        };

        let summary = TrainingSummary {
            episodes_completed,
            total_steps,
            cumulative_reward,
            best_episode_reward,
            final_epsilon,
            unique_states,
            total_q_entries,
            duration_seconds: duration,
        };

        self.save().ok();

        summary
    }
}

// ─── Common environment display helpers ───

/// List available RL environments (for CLI display).
pub fn list_available_environments() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "builtin-qlearning",
            "description": "Built-in Q-learning environment for decision optimization",
            "class": "QTable",
            "status": "available"
        }),
        serde_json::json!({
            "name": "tinker-atropos",
            "description": "Atropos RL training framework (requires tinker-atropos submodule)",
            "class": "External",
            "status": "requires-setup"
        }),
    ]
}

/// Check environment variables needed for RL training.
pub fn check_rl_env_vars() -> Vec<String> {
    let mut missing = Vec::new();
    for key in &["TINKER_API_KEY", "WANDB_API_KEY", "OPENROUTER_API_KEY"] {
        if std::env::var(key).is_err() {
            missing.push(key.to_string());
        }
    }
    missing
}

/// Check if tinker-atropos directory exists.
pub fn check_tinker_atropos(operant_home: &Path) -> (bool, String) {
    let tinker_path = operant_home.join("tinker-atropos");
    if tinker_path.exists() {
        let envs_path = tinker_path.join("tinker_atropos").join("environments");
        if envs_path.exists() {
            let count = fs::read_dir(&envs_path)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            e.file_name().to_string_lossy().ends_with(".py")
                                && !e.file_name().to_string_lossy().starts_with('_')
                        })
                        .count()
                })
                .unwrap_or(0);
            (
                true,
                format!(
                    "Found at {}. {} environments.",
                    tinker_path.display(),
                    count
                ),
            )
        } else {
            (
                true,
                format!(
                    "Found at {} (no environments directory).",
                    tinker_path.display()
                ),
            )
        }
    } else {
        (
            false,
            "tinker-atropos submodule not found. Run: git submodule update --init".to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_q_table_new_is_empty() {
        let table = QTable::new();
        assert_eq!(table.state_count(), 0);
        assert_eq!(table.entry_count(), 0);
    }

    #[test]
    fn test_set_and_get_q_value() {
        let mut table = QTable::new();
        assert!(table.get_q_value("state_a", "action_1").is_none());

        table.set_q_value("state_a", "action_1", 0.5);
        assert_eq!(table.get_q_value("state_a", "action_1"), Some(0.5));
        assert_eq!(table.state_count(), 1);
        assert_eq!(table.entry_count(), 1);
    }

    #[test]
    fn test_bellman_update() {
        let mut table = QTable::new();

        // Initial Q-value for state_a, action_1 is 0
        // Reward = 1, discount = 0.9, no next-state knowledge
        // new_q = 0 + 0.1 * (1 + 0.9 * 0 - 0) = 0.1
        let new_q = table.update("state_a", "action_1", 1.0, "state_b", 0.1, 0.9);

        assert!((new_q - 0.1).abs() < 1e-10);
        assert_eq!(table.visit_count("state_a", "action_1"), 1);
    }

    #[test]
    fn test_bellman_update_with_next_state_value() {
        let mut table = QTable::new();

        // Set Q(state_b, action_1) = 0.5
        table.set_q_value("state_b", "action_1", 0.5);

        // Q(state_a, action_1) = initial 0
        // TD = 1 + 0.9 * 0.5 - 0 = 1.45
        // new_q = 0 + 0.1 * 1.45 = 0.145
        let new_q = table.update("state_a", "action_1", 1.0, "state_b", 0.1, 0.9);
        assert!((new_q - 0.145).abs() < 1e-10);
    }

    #[test]
    fn test_get_best_action() {
        let mut table = QTable::new();
        table.set_q_value("state_a", "action_1", 0.1);
        table.set_q_value("state_a", "action_2", 0.8);
        table.set_q_value("state_a", "action_3", 0.3);

        let best = table.get_best_action("state_a");
        assert!(best.is_some());
        let (action, value) = best.unwrap();
        assert_eq!(action, "action_2");
        assert!((value - 0.8).abs() < 1e-10);
    }

    #[test]
    fn test_epsilon_greedy_exploit() {
        let table = QTable::new();
        let actions = vec!["action_1".to_string(), "action_2".to_string()];

        // With epsilon = 0, always picks the "best" (which with no entries is first)
        let selected = table.select_action("state_a", &actions, 0.0);
        assert!(selected.is_some());
    }

    #[test]
    fn test_epsilon_greedy_empty_actions() {
        let table = QTable::new();
        let actions: Vec<String> = vec![];
        let selected = table.select_action("state_a", &actions, 0.3);
        assert!(selected.is_none());
    }

    #[test]
    fn test_entries_sorted() {
        let mut table = QTable::new();
        table.set_q_value("state_b", "action_1", 0.5);
        table.set_q_value("state_a", "action_1", 0.1);

        let entries = table.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].state, "state_a");
        assert_eq!(entries[1].state, "state_b");
    }

    #[test]
    fn test_get_max_q_value() {
        let mut table = QTable::new();
        table.set_q_value("state_a", "action_1", 0.1);
        table.set_q_value("state_a", "action_2", 0.8);

        let max_q = table.get_max_q_value("state_a");
        assert!((max_q.unwrap() - 0.8).abs() < 1e-10);

        assert!(table.get_max_q_value("unknown_state").is_none());
    }

    #[test]
    fn test_trainer_persistence() {
        let tmp = TempDir::new().unwrap();
        let rl_dir = tmp.path().join("rl");
        let trainer = RlTrainer::new(rl_dir.clone());

        // Train some steps
        trainer.train_step("state_a", "action_1", 1.0, "state_b");
        trainer.train_step("state_b", "action_1", 0.5, "state_a");
        trainer.finish_episode(1.5, 2);

        // Save
        trainer.save().unwrap();

        // Create a new trainer from the same dir and load
        let trainer2 = RlTrainer::new(rl_dir.clone());
        trainer2.load().unwrap();

        assert!(trainer2.get_q_value("state_a", "action_1").is_some());
        assert_eq!(trainer2.get_state().episodes, 1);
        assert!((trainer2.get_state().cumulative_reward - 1.5).abs() < 1e-10);
    }

    #[test]
    fn test_trainer_reset() {
        let tmp = TempDir::new().unwrap();
        let rl_dir = tmp.path().join("rl");
        let trainer = RlTrainer::new(rl_dir);

        trainer.train_step("state_a", "action_1", 1.0, "state_b");
        trainer.finish_episode(1.0, 1);
        trainer.save().unwrap();
        assert_eq!(trainer.get_state().episodes, 1);

        trainer.reset().unwrap();
        assert_eq!(trainer.get_state().episodes, 0);
        assert!(trainer.get_q_value("state_a", "action_1").is_none());
    }

    #[test]
    fn test_trainer_get_q_value() {
        let tmp = TempDir::new().unwrap();
        let trainer = RlTrainer::new(tmp.path().join("rl"));

        assert!(trainer.get_q_value("state_a", "action_1").is_none());
        trainer.train_step("state_a", "action_1", 1.0, "state_b");
        assert!(trainer.get_q_value("state_a", "action_1").is_some());
    }

    #[test]
    fn test_train_step_returns_new_q_value() {
        let tmp = TempDir::new().unwrap();
        let trainer = RlTrainer::new(tmp.path().join("rl"));

        let new_q = trainer.train_step("s1", "a1", 1.0, "s2");
        // Q(s1, a1) starts at 0, α=0.1, γ=0.9, max Q(s2, *) = 0
        // new = 0 + 0.1 * (1 + 0.9*0 - 0) = 0.1
        assert!((new_q - 0.1).abs() < 1e-10);
    }

    #[test]
    fn test_episode_decays_epsilon() {
        let tmp = TempDir::new().unwrap();
        let trainer = RlTrainer::new(tmp.path().join("rl"));

        let initial_epsilon = trainer.get_state().epsilon;
        trainer.finish_episode(10.0, 5);
        let after_epsilon = trainer.get_state().epsilon;

        assert!(after_epsilon < initial_epsilon);
        assert!(after_epsilon >= MIN_EPSILON);
    }

    #[test]
    fn test_training_session_simulated() {
        let tmp = TempDir::new().unwrap();
        let trainer = RlTrainer::new(tmp.path().join("rl"));

        // Run a training session with a simple simulated environment
        let episodes = 10;
        let steps_per_episode = 5;

        let summary = trainer.run_training_session(episodes, steps_per_episode, |_ep, step| {
            let state = format!("s{}", step % 3);
            let action = format!("a{}", step % 2);
            let reward = if step % 2 == 0 { 1.0 } else { -0.5 };
            let next_state = format!("s{}", (step + 1) % 3);
            (state, action, reward, next_state)
        });

        assert!(summary.episodes_completed >= episodes);
        assert!(summary.total_steps > 0);
        assert!(summary.unique_states > 0);
        assert!(summary.total_q_entries > 0);
        assert!(summary.duration_seconds >= 0.0);
    }

    #[test]
    fn test_status_output() {
        let tmp = TempDir::new().unwrap();
        let trainer = RlTrainer::new(tmp.path().join("rl"));

        let status = trainer.status();
        assert!(status.contains("Episodes completed:  0"));
        assert!(status.contains("Q-table entries:     0"));
    }

    #[test]
    fn test_best_episode_tracking() {
        let tmp = TempDir::new().unwrap();
        let trainer = RlTrainer::new(tmp.path().join("rl"));

        trainer.finish_episode(5.0, 3);
        trainer.finish_episode(10.0, 5);
        trainer.finish_episode(3.0, 2);

        assert!((trainer.get_state().best_episode_reward - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_select_action_returns_available() {
        let tmp = TempDir::new().unwrap();
        let trainer = RlTrainer::new(tmp.path().join("rl"));

        let actions = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let selected = trainer.select_action("test_state", &actions);
        assert!(selected.is_some());
        assert!(actions.contains(&selected.unwrap()));
    }
}
