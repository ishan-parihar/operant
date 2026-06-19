use anyhow::Result;
use operant_core::config::AppConfig;

pub async fn handle_version_command(_config: &AppConfig, detailed: bool) -> Result<()> {
    if detailed {
        let info = operant_core::platform::platform_info();
        println!("operant {}", env!("CARGO_PKG_VERSION"));
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
        println!("{}", env!("CARGO_PKG_VERSION"));
    }
    Ok(())
}
