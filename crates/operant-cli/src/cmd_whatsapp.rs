use anyhow::Result;
use clap::Subcommand;
use operant_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum WhatsappSubcommand {
    /// Check WhatsApp connection status
    Status,
    /// Show WhatsApp setup instructions
    Setup,
}

pub async fn handle_whatsapp_command(config: &AppConfig, cmd: WhatsappSubcommand) -> Result<()> {
    match cmd {
        WhatsappSubcommand::Status => whatsapp_status(config).await,
        WhatsappSubcommand::Setup => whatsapp_setup(config).await,
    }
}

async fn whatsapp_status(_config: &AppConfig) -> Result<()> {
    let mode = std::env::var("WHATSAPP_MODE").ok();
    let qr = std::env::var("WHATSAPP_QR").ok();

    println!("WhatsApp status:");
    if let Some(m) = &mode {
        println!("  Mode: {m}");
    } else {
        println!("  Mode: not configured");
    }
    if qr.is_some() {
        println!("  QR pairing: configured");
    } else {
        println!("  QR pairing: not configured");
    }

    let configured = mode.is_some() || qr.is_some();
    println!();
    if configured {
        println!("WhatsApp is partially configured via environment variables.");
        println!("The interactive QR pairing wizard is available in the Python operant-agent only.");
        println!("  Install with: pip install -e '.[whatsapp]' in operant-agent/");
    } else {
        println!("WhatsApp is not configured. Run `operant whatsapp setup` for instructions.");
    }

    Ok(())
}

async fn whatsapp_setup(_config: &AppConfig) -> Result<()> {
    println!("WhatsApp Setup Instructions");
    println!("===========================");
    println!();
    println!("The Rust CLI does not include the interactive QR pairing wizard.");
    println!("Full WhatsApp integration requires the Python operant-agent.");
    println!();
    println!("Step-by-step:");
    println!();
    println!("  1. Install the Python operant-agent with WhatsApp support:");
    println!();
    println!("     cd operant-agent/");
    println!("     pip install -e '.[whatsapp]'");
    println!();
    println!("  2. Set the required environment variables:");
    println!();
    println!("     export WHATSAPP_MODE=qr");
    println!("     export WHATSAPP_QR=true");
    println!();
    println!("  3. Run the Python operant-agent with WhatsApp gateway:");
    println!();
    println!("     operant gateway whatsapp");
    println!();
    println!("  4. Scan the QR code displayed in the terminal with WhatsApp on your phone.");
    println!();
    println!("  5. Once paired, you can interact with the agent via WhatsApp messages.");
    println!();
    println!("Notes:");
    println!("  - The WhatsApp integration uses a whatsapp-web.js bridge under the hood.");
    println!("  - A headless Chromium instance is required for QR pairing (first time).");
    println!("  - Sessions persist across restarts in the .operant/ directory.");
    println!();
    println!("For more details, see the operant-agent documentation.");

    Ok(())
}
