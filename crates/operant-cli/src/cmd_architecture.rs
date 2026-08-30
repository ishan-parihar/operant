//! `operant architecture` — inspect the resolved provider tree (plan 016
//! Phase 3 / Phase 7 CLI). Reads `architecture.toml` (or a path passed
//! via `--file`), optionally applies `--patch FILE` overlays, and either
//! validates (exits non-zero on errors) or dumps a deterministic JSON tree
//! of rows + claims for the operator to inspect.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::Subcommand;
use serde_json::json;

use operant_harness::{Architecture, Builder, Composition, HarnessError, Patch};

#[derive(Subcommand, Debug, Clone)]
pub enum ArchitectureSubcommand {
    /// Validate the architecture: parse, apply patches, run Builder. Exits
    /// non-zero on any error.
    Validate {
        /// Path to the architecture TOML file.
        #[arg(long, default_value = "architecture.toml")]
        file: PathBuf,
        /// Patch files to apply in order (each is an `architecture.patch.toml`).
        #[arg(long)]
        patch: Vec<PathBuf>,
    },
    /// Dump the resolved architecture as a deterministic tree (JSON or text).
    Dump {
        /// Path to the architecture TOML file.
        #[arg(long, default_value = "architecture.toml")]
        file: PathBuf,
        /// Patch files to apply in order.
        #[arg(long)]
        patch: Vec<PathBuf>,
        /// JSON output for scripting/CI.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
    },
}

pub async fn handle_architecture_command(cmd: ArchitectureSubcommand) -> Result<()> {
    match cmd {
        ArchitectureSubcommand::Validate { file, patch } => {
            handle_validate_command(file, patch).await
        }
        ArchitectureSubcommand::Dump { file, patch, json } => {
            handle_dump_command(file, patch, json).await
        }
    }
}

/// Read an architecture file, optionally apply patches, and dump the
/// resolved rows as JSON. Used by `operant architecture dump`.
pub async fn handle_dump_command(file: PathBuf, patches: Vec<PathBuf>, json: bool) -> Result<()> {
    let arch = load_and_resolve(&file, &patches)
        .with_context(|| format!("loading architecture from {}", file.display()))?;
    if json {
        let rows_json: Vec<_> = arch
            .rows
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "source": r.source,
                    "disabled": r.disabled,
                    "kind": r.kind,
                    "config": r.config,
                })
            })
            .collect();
        let payload = json!({
            "file": file.display().to_string(),
            "patches": patches.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "row_count": arch.rows.len(),
            "active_count": arch.active().count(),
            "rows": rows_json,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("Architecture ({}):", file.display());
        for row in &arch.rows {
            let marker = if row.disabled { "x" } else { " " };
            let kind = row.kind.as_deref().unwrap_or("-");
            println!(
                "  [{}] {:<24} source={:<10} kind={}",
                marker, row.id, row.source, kind
            );
        }
        println!("{} rows, {} active", arch.rows.len(), arch.active().count());
    }
    Ok(())
}

/// Read an architecture file + patches, run [`Builder::build`], and
/// report validation errors. Exits 0 on a clean architecture, non-zero on
/// any error. Used by `operant architecture validate`.
pub async fn handle_validate_command(file: PathBuf, patches: Vec<PathBuf>) -> Result<()> {
    let arch = load_and_resolve(&file, &patches)
        .with_context(|| format!("loading architecture from {}", file.display()))?;
    arch.validate().map_err(|e| anyhow!(e.to_string()))?;
    let providers = Builder::build(&arch).map_err(|e| anyhow!(e.to_string()))?;
    println!(
        "architecture valid: {} active rows, {} providers",
        arch.active().count(),
        providers.len()
    );
    Ok(())
}

fn load_and_resolve(file: &Path, patches: &[PathBuf]) -> Result<Architecture> {
    let raw =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let arch = Architecture::from_toml(&raw).map_err(|e: HarnessError| anyhow!(e.to_string()))?;
    let mut patch_list = Vec::new();
    for p in patches {
        let raw =
            std::fs::read_to_string(p).with_context(|| format!("reading patch {}", p.display()))?;
        let parsed: Patch =
            toml::from_str(&raw).with_context(|| format!("parsing patch {}", p.display()))?;
        patch_list.push(parsed);
    }
    Composition::resolve(arch, &patch_list).map_err(|e| anyhow!(e.to_string()))
}
