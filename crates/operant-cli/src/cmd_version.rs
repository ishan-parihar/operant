use anyhow::Result;
use operant_core::config::AppConfig;

pub async fn handle_version_command(_config: &AppConfig, detailed: bool) -> Result<()> {
    // memory-wire provenance (plan 11A): the compiled rev from the lock.
    let rev = operant_core::MEMORY_WIRE_REV;
    let short: String = rev.chars().take(7).collect();
    if detailed {
        let info = operant_core::platform::platform_info();
        println!("operant {}", env!("CARGO_PKG_VERSION"));
        println!("memory-wire: {rev}");
        println!("OS: {}", info.os);
        println!("Arch: {}", info.arch);
        let shell_name = info
            .shell
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        println!("Shell: {}", shell_name);
        if cfg!(debug_assertions) {
            println!("Build: debug");
        } else {
            println!("Build: release");
        }
    } else {
        println!("{} (memory-wire {short})", env!("CARGO_PKG_VERSION"));
    }
    Ok(())
}
