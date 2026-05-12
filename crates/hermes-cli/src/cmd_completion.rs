//! Shell completion generation for Hermes-RS.
//!
//! Supports bash, zsh, and fish completions via `clap_complete`.

use std::io::{self, Write};

use anyhow::Result;
use clap::{CommandFactory, Subcommand, ValueEnum};

use crate::Cli;

/// Available shell kinds for auto-completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
}

/// Completion subcommand.
#[derive(Debug, Clone, Subcommand)]
pub enum CompletionSubcommand {
    /// Generate shell completion script
    Shell {
        /// The shell to generate completions for
        #[arg(value_enum)]
        shell: ShellKind,
    },
}

/// Handle the `completion` subcommand.
pub fn handle_completion_command(cmd: CompletionSubcommand) -> Result<()> {
    let shell = match cmd {
        CompletionSubcommand::Shell { shell } => shell,
    };

    let mut cmd = Cli::command();
    let mut stdout = io::stdout();
    let bin_name = "hermes";
    match shell {
        ShellKind::Bash => clap_complete::generate(clap_complete::shells::Bash, &mut cmd, bin_name, &mut stdout),
        ShellKind::Zsh => clap_complete::generate(clap_complete::shells::Zsh, &mut cmd, bin_name, &mut stdout),
        ShellKind::Fish => clap_complete::generate(clap_complete::shells::Fish, &mut cmd, bin_name, &mut stdout),
    }
    stdout.flush()?;

    Ok(())
}
