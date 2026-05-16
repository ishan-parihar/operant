//! RL Training CLI — reinforcement learning commands for Hermes.
//!
//! Ported from hermes-agent/rl_cli.py concepts.
//! Provides commands to train, evaluate, and manage RL-based decision
//! making for the agent.

use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::rl_training::{
    check_rl_env_vars, check_tinker_atropos, list_available_environments, RlTrainer,
};
use std::path::PathBuf;

/// RL training subcommands
#[derive(Debug, Clone, Subcommand)]
pub enum RlSubcommand {
    /// Run an RL training session
    Train {
        /// Number of training episodes
        #[arg(long, default_value_t = 50)]
        episodes: u64,

        /// Steps per episode
        #[arg(long, default_value_t = 10)]
        steps: usize,

        /// Learning rate (alpha)
        #[arg(long)]
        learning_rate: Option<f64>,

        /// Discount factor (gamma)
        #[arg(long)]
        discount_factor: Option<f64>,
    },
    /// Evaluate the trained Q-table
    Evaluate {
        /// State to evaluate actions for
        #[arg(long)]
        state: Option<String>,
    },
    /// Show RL training status and statistics
    Status,
    /// Reset all RL training state
    Reset {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// List available RL environments
    ListEnvironments,
    /// Check RL environment setup and configuration
    Doctor,
    /// Run interactive RL training session
    Run {
        /// Training prompt/task description
        prompt: String,

        /// Model override
        #[arg(long)]
        model: Option<String>,

        /// Maximum agent iterations
        #[arg(long, default_value_t = 200)]
        max_iterations: u32,
    },
}

/// Get the RL data directory for a config.
fn rl_dir(config: &AppConfig) -> PathBuf {
    config
        .database_path
        .parent()
        .map(|p| p.join("rl"))
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".hermes")
                .join("rl")
        })
}

/// Create an RL trainer from config.
fn create_trainer(config: &AppConfig) -> RlTrainer {
    let dir = rl_dir(config);
    RlTrainer::new(dir)
}

pub async fn handle_rl_command(config: &AppConfig, cmd: RlSubcommand) -> Result<()> {
    match cmd {
        RlSubcommand::Train {
            episodes,
            steps,
            learning_rate,
            discount_factor,
        } => cmd_train(config, episodes, steps, learning_rate, discount_factor).await,
        RlSubcommand::Evaluate { state } => cmd_evaluate(config, state).await,
        RlSubcommand::Status => cmd_status(config).await,
        RlSubcommand::Reset { yes } => cmd_reset(config, yes).await,
        RlSubcommand::ListEnvironments => cmd_list_environments().await,
        RlSubcommand::Doctor => cmd_doctor(config).await,
        RlSubcommand::Run {
            prompt,
            model,
            max_iterations,
        } => cmd_run(config, &prompt, model, max_iterations).await,
    }
}

/// Train the Q-table with simulated episodes.
async fn cmd_train(
    config: &AppConfig,
    episodes: u64,
    steps_per_episode: usize,
    learning_rate: Option<f64>,
    discount_factor: Option<f64>,
) -> Result<()> {
    let trainer = create_trainer(config);
    trainer.load()?;

    // Override hyperparameters if provided
    if let Some(lr) = learning_rate {
        let mut state = trainer.state_write();
        state.learning_rate = lr;
    }
    if let Some(df) = discount_factor {
        let mut state = trainer.state_write();
        state.discount_factor = df;
    }

    let train_state = trainer.get_state();
    println!("Hermes RL Training");
    println!("{}", "-".repeat(50));
    println!("Episodes:        {}", episodes);
    println!("Steps/episode:   {}", steps_per_episode);
    println!("Learning rate:   {:.4}", train_state.learning_rate);
    println!("Discount factor: {:.4}", train_state.discount_factor);
    println!("Initial epsilon: {:.4}", train_state.epsilon);
    println!("Current states:  {}", train_state.episodes);
    println!();

    // Run simulated training
    println!("Training...");
    let summary = trainer.run_training_session(episodes, steps_per_episode, |ep, step| {
        // Simulate: state = s{step % 4}, action = a{step % 3}
        // Reward: positive for even steps, negative for odd
        let state = format!("s{}", (ep as usize + step) % 4);
        let action = format!("a{}", step % 3);
        let reward = if step % 2 == 0 { 1.0 } else { -0.2 };
        let next_state = format!("s{}", (ep as usize + step + 1) % 4);
        (state, action, reward, next_state)
    });

    println!("{}", "-".repeat(50));
    println!("✅ Training complete!");
    println!("  Episodes:        {}", summary.episodes_completed);
    println!("  Duration:        {:.2}s", summary.duration_seconds);
    println!("  Total steps:     {}", summary.total_steps);
    println!("  Cumulative reward: {:.4}", summary.cumulative_reward);
    println!("  Best episode:    {:.4}", summary.best_episode_reward);
    println!("  Final epsilon:   {:.4}", summary.final_epsilon);
    println!("  Unique states:   {}", summary.unique_states);
    println!("  Q-table entries: {}", summary.total_q_entries);

    Ok(())
}

/// Evaluate the Q-table for a specific state or overall performance.
async fn cmd_evaluate(config: &AppConfig, state: Option<String>) -> Result<()> {
    let trainer = create_trainer(config);
    trainer.load()?;

    let (state_count, entry_count) = trainer.get_q_table_stats();
    if state_count == 0 {
        println!("No Q-table data available. Run `hermes rl train` first.");
        return Ok(());
    }

    let rl_state = trainer.get_state();

    println!("Hermes RL Evaluation");
    println!("{}", "-".repeat(50));
    println!("Unique states:   {}", state_count);
    println!("Q-table entries: {}", entry_count);
    println!("Episodes:        {}", rl_state.episodes);

    // If a specific state is requested, show its action values
    if let Some(ref target_state) = state {
        let entries = trainer.get_q_table();
        if let Some(entry) = entries.iter().find(|e| e.state == *target_state) {
            println!();
            println!("State: '{}'", target_state);
            println!("Actions (sorted by Q-value):");
            let mut actions: Vec<(&String, &f64)> = entry.actions.iter().collect();
            actions.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            for (action, value) in actions {
                let visits = trainer.get_visit_count(target_state, action);
                println!("  {:<12}  Q={:>8.4}  visits={}", action, value, visits);
            }
        } else {
            println!("\nState '{}' not found in Q-table.", target_state);
        }
    } else {
        // Show top states by number of actions
        let entries = trainer.get_q_table();
        let top: Vec<_> = entries.iter().take(10).collect();
        if !top.is_empty() {
            println!("\nTop {} states (by action count):", top.len());
            for entry in &top {
                let best = entry
                    .actions
                    .iter()
                    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                if let Some((best_action, best_value)) = best {
                    println!(
                        "  {:<20}  {} actions, best: {} ({:.4})",
                        entry.state,
                        entry.actions.len(),
                        best_action,
                        best_value
                    );
                }
            }
        }
    }

    Ok(())
}

/// Show training status.
async fn cmd_status(config: &AppConfig) -> Result<()> {
    let trainer = create_trainer(config);
    trainer.load()?;

    print!("{}", trainer.status());
    Ok(())
}

/// Reset all RL training state.
async fn cmd_reset(config: &AppConfig, yes: bool) -> Result<()> {
    if !yes {
        println!("This will reset all RL training data (Q-table, episodes, rewards).");
        println!("Use --yes to confirm.");
        return Ok(());
    }

    let trainer = create_trainer(config);
    trainer.load()?;
    trainer.reset()?;
    println!("✅ RL training state has been reset.");
    Ok(())
}

/// List available RL environments.
async fn cmd_list_environments() -> Result<()> {
    println!("Available RL Environments:");
    println!("{}", "-".repeat(50));

    let envs = list_available_environments();
    for env in &envs {
        let name = env["name"].as_str().unwrap_or("unknown");
        let description = env["description"].as_str().unwrap_or("");
        let status = env["status"].as_str().unwrap_or("");

        let status_icon = match status {
            "available" => "✅",
            "requires-setup" => "⚠️",
            _ => "❓",
        };

        println!("\n  {} {}", status_icon, name);
        println!("     {}", description);
    }

    println!();
    println!("Use `hermes rl doctor` to check your setup.");
    Ok(())
}

/// Check RL environment setup.
async fn cmd_doctor(config: &AppConfig) -> Result<()> {
    println!("Hermes RL Doctor Check");
    println!("{}", "-".repeat(50));

    // Check environment variables
    let missing = check_rl_env_vars();
    if missing.is_empty() {
        println!("✅ All required environment variables are set.");
    } else {
        for key in &missing {
            println!("  ❌ Missing: {}", key);
        }
        println!("  Set these in your .env file or shell.");
    }

    // Check configuration
    println!("✅ Model: {}", config.agent.model);

    // Check RL data directory
    let rl_data_dir = rl_dir(config);
    let data_status = if rl_data_dir.exists() {
        "✅"
    } else {
        "ℹ️"
    };
    println!(
        "{} RL data directory: {}",
        data_status,
        rl_data_dir.display()
    );

    // Check tinker-atropos if available
    if let Some(home) = dirs::home_dir() {
        let hermes_home = home.join(".hermes");
        if hermes_home.exists() {
            let (found, msg) = check_tinker_atropos(&hermes_home);
            if found {
                println!("✅ tinker-atropos: {}", msg);
            } else {
                println!("  ⚠️  tinker-atropos: {}", msg);
            }
        }
    }

    // Check saved training data
    let trainer = create_trainer(config);
    match trainer.load() {
        Ok(()) => {
            let (states, entries) = trainer.get_q_table_stats();
            if states > 0 {
                println!(
                    "✅ Saved training data: {} states, {} Q-entries",
                    states, entries
                );
                let rl_state = trainer.get_state();
                println!(
                    "   Episodes: {}, Total steps: {}",
                    rl_state.episodes, rl_state.total_steps
                );
            } else {
                println!("ℹ️  No saved training data. Run `hermes rl train` to start.");
            }
        }
        Err(e) => {
            println!("  ⚠️  Could not load training data: {}", e);
        }
    }

    Ok(())
}

/// Run interactive RL agent session (port of Python rl_cli.py).
async fn cmd_run(
    config: &AppConfig,
    prompt: &str,
    model: Option<String>,
    max_iterations: u32,
) -> Result<()> {
    let model_name = model.unwrap_or_else(|| config.agent.model.clone());

    println!("🎯 Hermes RL Training Agent");
    println!("{}", "=".repeat(60));
    println!("Model:      {}", model_name);
    println!("Prompt:     {}", prompt);
    println!("Iterations: {}", max_iterations);

    // Check environment variables
    let missing = check_rl_env_vars();
    if !missing.is_empty() {
        println!();
        println!("⚠️  Missing required RL environment variables:");
        for key in &missing {
            println!("  - {}", key);
        }
        println!("RL training agent cannot proceed without these variables.");
        println!("Set them in ~/.hermes/.env or your shell.");
        return Ok(());
    }

    // Load RL trainer state
    let trainer = create_trainer(config);
    trainer.load()?;
    let rl_state = trainer.get_state();
    println!();
    println!(
        "RL State: {} episodes, {} Q-table entries",
        rl_state.episodes,
        {
            let (_, entries) = trainer.get_q_table_stats();
            entries
        }
    );

    println!();
    println!("Starting RL agent session...");
    println!("{}", "=".repeat(60));

    // Note: Full agent integration requires wiring into HermesAgent.
    // Currently this provides the RL context and state management.
    // The agent is expected to use RL tools (rl_list_environments, etc.)
    // during the conversation.
    println!(
        "Task: {}\n\
         \n\
         The agent will use RL tools to:\n\
         1. Discover available environments\n\
         2. Configure training parameters\n\
         3. Run training and evaluate results\n\
         \n\
         (Full agent loop integration requires python hermes-agent for now.)",
        prompt
    );

    Ok(())
}
