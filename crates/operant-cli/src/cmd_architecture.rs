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
        /// G8 — re-dump every N seconds (for live-loop monitoring). 0 = once.
        #[arg(long, default_value_t = 0)]
        watch_secs: u64,
        /// S6 — when set, build a live Harness from the file and dump the
        /// kernel's `DumpTree` (provider states, claims, generation) instead
        /// of the raw file rows.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        live: bool,
    },
    /// Compile a hermes `_pool.yaml` into architecture rows (Phase 6).
    PoolImport {
        /// Path to the `_pool.yaml` file.
        #[arg(long)]
        path: PathBuf,
        /// JSON output for scripting/CI.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
        /// When set, append the compiled rows into the architecture file
        /// (default `architecture.toml`). Creates the file if missing.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        apply: bool,
        /// Architecture file to apply into (only with --apply).
        #[arg(long, default_value = "architecture.toml")]
        file: PathBuf,
    },
}

pub async fn handle_architecture_command(cmd: ArchitectureSubcommand) -> Result<()> {
    match cmd {
        ArchitectureSubcommand::Validate { file, patch } => {
            handle_validate_command(file, patch).await
        }
        ArchitectureSubcommand::Dump { file, patch, json, watch_secs, live } => {
            handle_dump_command(file, patch, json, watch_secs, live).await
        }
        ArchitectureSubcommand::PoolImport { path, json, apply, file } => {
            handle_pool_import_command(path, json, apply, file).await
        }
    }
}

/// Compile a hermes `_pool.yaml` into architecture rows. Prints the
/// rows as JSON (default) or text. Phase 6 — the rows are NOT yet
/// wired to live adapters; the host's boot pass will add that.
///
/// S5 — with `--apply`, the compiled rows are appended into the
/// architecture file and the resulting file is validated via `Builder`.
pub async fn handle_pool_import_command(
    path: PathBuf,
    json: bool,
    apply: bool,
    file: PathBuf,
) -> Result<()> {
    let compiled =
        operant_harness::load_and_compile_pool(&path).map_err(|e| anyhow!(e.to_string()))?;
    if apply {
        // Load existing architecture (or empty if missing) and append rows.
        let mut arch = if file.exists() {
            let raw = std::fs::read_to_string(&file)
                .with_context(|| format!("reading architecture file {}", file.display()))?;
            operant_harness::Architecture::from_toml(&raw)
                .map_err(|e| anyhow!(e.to_string()))?
        } else {
            operant_harness::Architecture::default()
        };
        // Avoid duplicating existing ids — skip rows whose id already exists.
        let existing: std::collections::HashSet<String> =
            arch.rows.iter().map(|r| r.id.clone()).collect();
        let mut appended = 0usize;
        if !existing.contains(&compiled.family_row.id) {
            arch.rows.push(compiled.family_row.clone());
            appended += 1;
        }
        for row in &compiled.bundle_rows {
            if !existing.contains(&row.id) {
                arch.rows.push(row.clone());
                appended += 1;
            }
        }
        arch.validate().map_err(|e| anyhow!(e.to_string()))?;
        // Validate via BuilderWithFactories so pool rows are accepted.
        {
            let mut builder = operant_harness::BuilderWithFactories::new();
            builder.register_factory(
                "pool",
                std::sync::Arc::new(|row: operant_harness::ArchitectureRow| {
                    match row.kind.as_deref() {
                        Some("pool.bundle") => Ok(std::sync::Arc::new(
                            operant_harness::PoolBundleProvider::new(row),
                        )
                            as std::sync::Arc<dyn operant_harness::Provider>),
                        Some("pool.family") | None => Ok(std::sync::Arc::new(
                            operant_harness::PoolFamilyProvider::new(row),
                        )
                            as std::sync::Arc<dyn operant_harness::Provider>),
                        Some(other) => Err(operant_harness::BuildError::NoConfigRowHandler(
                            row.id.clone(),
                            other.to_string(),
                        )),
                    }
                }),
            );
            builder
                .build_with(&arch)
                .map_err(|e| anyhow!(format!("validate after apply: {e}")))?;
        }
        let toml_str = arch
            .to_toml()
            .map_err(|e| anyhow!(e.to_string()))?;
        std::fs::write(&file, toml_str)
            .with_context(|| format!("writing architecture file {}", file.display()))?;
        println!(
            "pool '{}' applied to {} ({} new rows, {} total, {} active)",
            compiled.name,
            file.display(),
            appended,
            arch.rows.len(),
            arch.active().count()
        );
    }
    if json {
        let payload = json!({
            "name": compiled.name,
            "family_row": compiled.family_row,
            "bundle_rows": compiled.bundle_rows,
            "claims": compiled.claims,
            "applied": apply,
            "file": file.display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if !apply {
        println!("Pool: {}", compiled.name);
        println!("  Family row: {}", compiled.family_row.id);
        println!("  Claims: {:?}", compiled.claims);
        println!("  Bundles:");
        for r in &compiled.bundle_rows {
            println!("    - {} ({})", r.id, r.kind.as_deref().unwrap_or("-"));
        }
    }
    Ok(())
}

/// Read an architecture file, optionally apply patches, and dump the
/// resolved rows as JSON. Used by `operant architecture dump`. When
/// `watch_secs > 0`, re-dumps every N seconds until interrupted.
pub async fn handle_dump_command(
    file: PathBuf,
    patches: Vec<PathBuf>,
    json: bool,
    watch_secs: u64,
    live: bool,
) -> Result<()> {
    if watch_secs == 0 {
        return if live {
            dump_live(&file, &patches, json).await
        } else {
            dump_once(&file, &patches, json)
        };
    }
    let mut stop = tokio::sync::watch::channel(false).0;
    let interval = std::time::Duration::from_secs(watch_secs);
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        let res = if live {
            dump_live(&file, &patches, json).await
        } else {
            dump_once(&file, &patches, json)
        };
        if let Err(e) = res {
            eprintln!("dump error: {e:#}");
        }
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("watch: Ctrl-C received, exiting");
                return Ok(());
            }
        }
    }
}

async fn dump_live(file: &Path, patches: &[PathBuf], json: bool) -> Result<()> {
    use operant_harness::{
        BuilderWithFactories, Harness, HarnessHost, KernelOptions, PoolBundleProvider,
        PoolFamilyProvider,
    };
    let arch = load_and_resolve(file, patches)
        .with_context(|| format!("loading architecture from {}", file.display()))?;
    // Build a live harness with the same factories as production boot.
    let mut builder = BuilderWithFactories::new();
    builder.register_factory(
        "pool",
        std::sync::Arc::new(|row: operant_harness::ArchitectureRow| {
            match row.kind.as_deref() {
                Some("pool.bundle") => Ok(std::sync::Arc::new(PoolBundleProvider::new(row))
                    as std::sync::Arc<dyn operant_harness::Provider>),
                Some("pool.family") | None => Ok(std::sync::Arc::new(PoolFamilyProvider::new(row))
                    as std::sync::Arc<dyn operant_harness::Provider>),
                Some(other) => Err(operant_harness::BuildError::NoConfigRowHandler(
                    row.id.clone(),
                    other.to_string(),
                )),
            }
        }),
    );
    // Live dump builds a real Harness so provider states/claims are visible.
    let mut host = HarnessHost::with_harness(std::sync::Arc::new(Harness::new(KernelOptions {
        audit: false,
    })));
    {
        use operant_core::harness_adapters::ToolSeam;
        use operant_core::tools::ToolRegistry;
        let registry = ToolRegistry::new(std::time::Duration::from_secs(30));
        host.add_seam(std::sync::Arc::new(ToolSeam::new(registry)));
        // prompt.section needs runtime seam — for live dump we still want the
        // DumpTree even if those rows end up Failed/Pending, so we boot
        // best-effort and dump regardless of boot error.
        let _ = host.boot_with_factories(&builder, &arch).await;
        let tree = host.dump().await;
        if json {
            println!("{}", tree.to_json());
        } else {
            println!("Live Harness Dump ({}):", file.display());
            for p in &tree.providers {
                println!(
                    "  [{:?}] {:<24} source={:<10} gen={} effects={}",
                    p.state, p.id, p.source, p.generation, p.effects
                );
            }
            println!("{} providers, {} claims", tree.providers.len(), tree.claims.len());
        }
    }
    Ok(())
}

fn dump_once(file: &Path, patches: &[PathBuf], json: bool) -> Result<()> {
    let arch = load_and_resolve(file, patches)
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
            "ts": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
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
