//! The ONE list of vendor skill roots (iter-2135).
//!
//! The vendor roots are the read-only skill dirs that live OUTSIDE the
//! operator's own pool: the installed content integrations (the ponytail
//! package) and plugin-contributed skill dirs. Before this module, five
//! sites decided that list separately — and three of them already
//! disagreed: the gateway's `/skills` listing had none, the CLI listing and
//! the TUI overlay had only the integration, and the in-loop `skills_list`
//! / `skill_view` tools had only the integration while the frozen prefix
//! advertised plugin skills too.

use std::path::PathBuf;

use operant_core::config::AppConfig;

/// The vendor skill roots — the installed content integrations plus
/// plugin-contributed skill dirs.
///
/// ONE list, computed in ONE place, so every surface that can see or run a
/// skill agrees about which skills exist: the SkillManager the agent
/// advertises from, the in-loop tools, the TUI overlay, the CLI listing and
/// the gateway's `/skills`.
pub fn vendor_skill_roots(config: &AppConfig) -> Vec<PathBuf> {
    let mut host_roots: Vec<PathBuf> = Vec::new();
    #[cfg(feature = "plugins-wasm")]
    host_roots.extend(crate::plugin_tools::plugin_skill_roots(config));
    operant_core::skills::vendor_skill_roots(&host_roots)
}

/// The vendor skills as `(name, description)` rows — the slash-command
/// surfaces show them (the TUI command palette, the gateway's `/help`, the
/// Telegram command menu), because a skill name IS a command:
/// `/ponytail-audit` runs the skill.
///
/// Only the VENDOR skills are listed. The operator's own pool stays behind
/// `/skill <name>`: it is large and personal, and the Telegram command menu
/// caps at 100 entries.
pub fn vendor_skill_command_rows(config: &AppConfig) -> Vec<(String, String)> {
    let roots = vendor_skill_roots(config);
    crate::gateway_commands::skill_command_rows_for_roots(&config.skills.root_dir, &roots)
}
