use anyhow::Result;
use clap::Subcommand;
use operant_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum UpdateSubcommand {
    /// Check for available updates
    Check,
    /// Apply available update
    Apply,
    /// Force an update check for the external integrations (sourcehound,
    /// aft, ponytail) now — bypasses the 6 h throttle
    Integrations {
        /// Refresh only this integration (sourcehound, aft, ponytail).
        /// Without it, all three are refreshed — and the other two are
        /// BINARIES, so refreshing the ponytail content would also swap
        /// the aft binary out from under the operator.
        #[arg(long, value_name = "NAME")]
        only: Option<String>,
    },
}

pub async fn handle_update_command(_config: &AppConfig, cmd: UpdateSubcommand) -> Result<()> {
    match cmd {
        UpdateSubcommand::Check => cmd_check().await,
        UpdateSubcommand::Apply => cmd_apply().await,
        UpdateSubcommand::Integrations { only } => cmd_integrations(only.as_deref()).await,
    }
}

/// (plan 11B) Force-check the external integrations now: print the state
/// of each and apply any pending update (bypassing the once-guard and the
/// 6 h on-disk throttle — the user asked).
async fn cmd_integrations(only: Option<&str>) -> Result<()> {
    if let Some(name) = only
        && !matches!(name, "sourcehound" | "aft" | "ponytail")
    {
        anyhow::bail!("unknown integration '{name}' — choose sourcehound, aft or ponytail");
    }
    println!("operant integrations:\n");
    let report = |name: &str, r: Result<Option<operant_core::binary_updater::UpdateNotice>>| match r
    {
        Ok(Some(operant_core::binary_updater::UpdateNotice::Updated { from, to, .. })) => {
            println!("  {name}: updated {from} → {to}")
        }
        Ok(Some(operant_core::binary_updater::UpdateNotice::Available { to, .. })) => {
            println!("  {name}: {to} available (no asset for this platform)")
        }
        Ok(Some(operant_core::binary_updater::UpdateNotice::Failed { reason, .. })) => {
            println!("  {name}: update failed — {reason}")
        }
        Ok(None) => println!("  {name}: up to date"),
        Err(e) => println!("  {name}: check failed — {e}"),
    };
    let wanted = |name: &str| only.is_none_or(|only| only == name);
    if wanted("sourcehound") {
        if let Some(spec) = operant_core::tools::sourcehound_update::sourcehound_spec() {
            let r = operant_core::binary_updater::update(&spec).await;
            report("sourcehound", r);
        } else {
            println!("  sourcehound: not found on PATH");
        }
    }
    if wanted("aft") {
        let aft = operant_core::aft_bridge::aft_spec();
        let r = operant_core::binary_updater::update(&aft).await;
        report("aft", r);
    }
    // Ponytail is a CONTENT integration (no asset): the tag tarball IS the
    // payload, so `update()` installs directly and never returns Available.
    if wanted("ponytail") {
        let ponytail = operant_core::ponytail::ponytail_spec();
        let r = operant_core::binary_updater::update(&ponytail).await;
        report("ponytail", r);
    }
    Ok(())
}

async fn cmd_check() -> Result<()> {
    println!("operant v{}", env!("CARGO_PKG_VERSION"));
    println!("Visit https://github.com/ishanp/HERMES/releases to check for updates.");
    Ok(())
}

async fn cmd_apply() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!("Current version: {}", version);
    println!();
    println!("To update Operant:");
    println!("  1. Navigate to the operant-rs directory");
    println!("  2. Run: git pull && cargo install --path crates/operant-cli");
    println!("  3. Or visit: https://github.com/ishanp/HERMES/releases");
    Ok(())
}
