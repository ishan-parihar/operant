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

/// Every skill root for this install, in precedence order — the ONE list the
/// TUI, the CLI, the gateway and the in-loop tools read.
///
/// This is what makes `[skills] extra_roots` and `project_root` reach a
/// surface at all. Before it, each caller passed `config.skills.root_dir`
/// plus the vendor roots, so a root the operator wrote in the config was
/// visible to the resolver (and therefore to `operant doctor`) and to nothing
/// that actually loads a skill. The resolver is the only place the config is
/// read, so a surface cannot pick up a subset by accident.
pub fn skill_roots(config: &AppConfig) -> operant_core::skills::SkillRoots {
    operant_core::skills::SkillRoots::resolve(
        &config.skills,
        &vendor_skill_roots(config),
        None,
        None,
    )
}

/// A [`SkillManager`] over the config's resolved roots, with the extra roots
/// already applied.
///
/// `SkillManager::new(pool)` + `add_extra_root` per vendor root is what every
/// call site used to do; it silently ignored `extra_roots` and the project
/// root, which is the whole point of the config keys.
#[must_use]
pub fn skill_manager(config: &AppConfig) -> operant_core::skills::SkillManager {
    operant_core::skills::SkillManager::from_roots(&skill_roots(config))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn skill_dir(root: &std::path::Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("skill dir");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: skill {name}\n---\n\nbody\n"),
        )
        .expect("SKILL.md");
    }

    /// THE contract of the whole resolver: a skill in `[skills] extra_roots`
    /// must reach the manager the agent gets.
    ///
    /// iter-2295's defect was that 13 call sites passed only
    /// `config.skills.root_dir` plus the vendor roots, so `extra_roots`
    /// reached `operant doctor` and nothing that loads a skill. This test is
    /// the one that fails if a surface is rewired to the old form.
    #[test]
    fn a_skill_in_extra_roots_reaches_the_manager() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pool = tmp.path().join("pool");
        let extra = tmp.path().join("extra");
        std::fs::create_dir_all(&pool).expect("pool dir");
        std::fs::create_dir_all(&extra).expect("extra dir");
        skill_dir(&pool, "from-pool");
        skill_dir(&extra, "from-extra");

        let mut config = AppConfig::default();
        config.skills.root_dir = pool.clone();
        config.skills.extra_roots = vec![extra.clone()];

        let mut manager = skill_manager(&config);
        manager.load_all().expect("load");

        assert!(
            manager.get("from-pool").is_some(),
            "the pool's skill is missing"
        );
        assert!(
            manager.get("from-extra").is_some(),
            "a skill in extra_roots never reached the manager -- the resolver's \
             whole point is that this cannot happen"
        );
    }

    /// The project root rides the same list, and is only consulted when
    /// `project_root` is not "off".
    #[test]
    fn the_project_root_rides_the_resolved_list() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pool = tmp.path().join("pool");
        let proj = tmp.path().join("proj");
        std::fs::create_dir_all(&pool).expect("pool");
        std::fs::create_dir_all(&proj).expect("proj");
        skill_dir(&pool, "from-pool");
        skill_dir(&proj, "from-project");

        let mut config = AppConfig::default();
        config.skills.root_dir = pool;
        config.skills.project_root = proj.display().to_string();

        let roots = skill_roots(&config);
        assert!(
            roots.as_paths().iter().any(|p| p == &proj),
            "the explicit project root did not resolve: {:?}",
            roots.as_paths()
        );

        let mut manager = skill_manager(&config);
        manager.load_all().expect("load");
        assert!(
            manager.get("from-project").is_some(),
            "the project root's skill is missing"
        );
    }
}
