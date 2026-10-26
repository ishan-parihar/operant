//! Ponytail content integration (plan
//! `2026-10-26-ponytail-as-a-self-updating-content-integration`).
//!
//! Upstream: <https://github.com/DietrichGebert/ponytail> — a CONTENT
//! package (markdown SKILL.md prompts), whose GitHub releases carry NO
//! assets: the tag tarball is the payload. This module builds the
//! `IntegrationSpec` and exposes the installed-skills path the CLI wires
//! into `SkillManager` extra roots (phase 3).
//!
//! Storage: `~/.operant/integrations/ponytail-vX.Y.Z/skills/<skill>/` —
//! versioned like AFT's cache dirs, pruned to `keep` newest. NOT in
//! `~/.operant/skills/`: that is the operator's skill pool, and an
//! auto-updated vendor tree must never overwrite it.

use crate::binary_updater::{InstallKind, IntegrationSpec};
use crate::platform::operant_home;

/// Upstream repo (public — the `gh api` release poll works tokenlessly
/// there; `gh auth` is still used when present for rate limits).
pub const PONYTAIL_REPO: &str = "DietrichGebert/ponytail";

/// Where versioned ponytail installs live.
pub fn integrations_dir() -> std::path::PathBuf {
    operant_home().join("integrations")
}

/// The ponytail integration spec: content payload, tag-tarball install,
/// skills-only extraction, community-trust guard scan.
pub fn ponytail_spec() -> IntegrationSpec {
    IntegrationSpec {
        name: "ponytail",
        repo: PONYTAIL_REPO,
        // No release assets exist; the marker is unused on the content path.
        asset_marker: "",
        content_tarball: true,
        install: InstallKind::Content {
            dir: integrations_dir(),
            keep: 2,
            content_subdir: "skills",
            guard_source: "community:DietrichGebert/ponytail",
        },
    }
}

/// The installed ponytail skills dir (`<integrations>/ponytail-vX.Y.Z/skills/`),
/// newest version first — the path `SkillManager` gains as an extra load
/// root (phase 3). `None` when ponytail is not installed.
pub fn installed_skills_dir() -> Option<std::path::PathBuf> {
    let (_, dir) = crate::binary_updater::latest_version_dir(&integrations_dir());
    dir.map(|d| d.join("skills"))
}

/// Kick the once-per-session background update check. Same gate as the
/// binary integrations: `OPERANT_NO_AUTO_UPDATE` off, not a test build.
/// Phase 2 wires this at CLI/TUI startup when `[agent.ponytail] enabled`
/// is true.
pub fn kick_update_check() {
    crate::binary_updater::kick_session_update(ponytail_spec());
}

/// Version check for doctor / `update --integrations` — no install.
pub async fn check() -> Option<crate::binary_updater::Versions> {
    crate::binary_updater::check(&ponytail_spec()).await.ok()
}

// ---------------------------------------------------------------------------
// Phase 4 — always-on posture injection (frozen-prefix `<ponytail>` block)
// ---------------------------------------------------------------------------

/// Process-global runtime override for the injected posture level, set by
/// the TUI `/ponytail [lite|full|ultra|off]` command. `None` = the config
/// value applies; `Some(Level)` = that level; injection OFF is modelled as
/// the explicit `Override::Off` variant (not `None`, which means "no
/// override") — the same presence-vs-value distinction the reasoning
/// override cell in the TUI config shim makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostureOverride {
    /// Inject this level's ruleset.
    Level(crate::config::PonytailLevel),
    /// `/ponytail off`: clear injection for this process (config stays
    /// untouched on disk).
    Off,
}

static POSTURE_OVERRIDE: std::sync::RwLock<Option<PostureOverride>> = std::sync::RwLock::new(None);

/// Set the process-global posture override (the `/ponytail` command).
pub fn set_posture_override(override_value: Option<PostureOverride>) {
    if let Ok(mut cell) = POSTURE_OVERRIDE.write() {
        *cell = override_value;
    }
}

/// The current process-global posture override (`None` = config applies).
pub fn posture_override() -> Option<PostureOverride> {
    POSTURE_OVERRIDE.read().ok().and_then(|cell| *cell)
}

/// Test-only: the override cell guard pattern (`reasoning_override_test_guard`).
#[cfg(test)]
pub(crate) struct PostureOverrideGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for PostureOverrideGuard {
    fn drop(&mut self) {
        set_posture_override(None);
    }
}

#[cfg(test)]
static POSTURE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn posture_override_test_guard() -> PostureOverrideGuard {
    PostureOverrideGuard {
        _lock: POSTURE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    }
}

/// The upstream level suffix lines (plan: lite/ultra are two-sentence
/// deltas carried as a suffix; `full` is upstream's own default and carries
/// no suffix).
fn level_suffix_line(level: crate::config::PonytailLevel) -> Option<&'static str> {
    match level {
        crate::config::PonytailLevel::Full => None,
        crate::config::PonytailLevel::Lite => Some(
            "Level: lite — build what was asked; name the smaller option when \
             one exists, but ship the requested shape.",
        ),
        crate::config::PonytailLevel::Ultra => Some(
            "Level: ultra — also question the request before building: \
             challenge the premise, propose the alternative, then build what \
             survives the challenge.",
        ),
    }
}

/// Build the `<ponytail>` block for the frozen prefix, or `None` when the
/// posture must NOT inject (disabled / on_invoke / Holon run / not
/// installed / no override forcing it on).
///
/// The ruleset body is the installed `ponytail` skill's own SKILL.md —
/// upstream's core "lazy senior dev" prompt — with frontmatter stripped.
/// Byte-stability: the block only changes when the install updates or the
/// level/override changes, so the prompt-cache discipline holds.
pub fn frozen_prefix_block(settings: &crate::config::PonytailSettings) -> Option<String> {
    // Holon runs NEVER carry personal postures — the org brief is the only
    // guidance for a bound body (plan's design preference, called out in
    // the example toml).
    if crate::platform::is_holon_run() {
        return None;
    }
    // Resolve the effective injection decision: config first, override
    // can force a level or force off.
    let inject = match posture_override() {
        Some(PostureOverride::Off) => false,
        Some(PostureOverride::Level(_)) => true,
        None => settings.enabled && settings.inject == crate::config::PonytailInject::AlwaysOn,
    };
    if !inject {
        return None;
    }
    let level = match posture_override() {
        Some(PostureOverride::Level(l)) => l,
        _ => settings.default_level,
    };
    // The ruleset body comes from the installed core skill. Missing body
    // (not installed) injects NOTHING rather than an empty shell — a
    // `<ponytail>` block with no content would be worse than absent.
    let body = installed_ruleset()?;
    let mut block = String::from("\n\n<ponytail>\n");
    block.push_str(body.trim());
    if let Some(suffix) = level_suffix_line(level) {
        block.push_str("\n\n");
        block.push_str(suffix);
    }
    block.push_str("\n</ponytail>");
    Some(block)
}

/// The installed core ponytail ruleset: the `ponytail` skill's SKILL.md
/// body (frontmatter stripped). `None` when not installed.
pub fn installed_ruleset() -> Option<String> {
    let skills = installed_skills_dir()?;
    let raw = std::fs::read_to_string(skills.join("ponytail").join("SKILL.md")).ok()?;
    if let Some(stripped) = raw.strip_prefix("---")
        && let Some(end) = stripped.find("\n---")
    {
        return Some(stripped[end + 4..].trim_start().to_string());
    }
    Some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec must be content-shaped: tag-tarball install, skills
    /// extraction, keep=2, community guard source.
    #[test]
    fn ponytail_spec_is_content_shaped() {
        let spec = ponytail_spec();
        assert!(spec.content_tarball);
        assert_eq!(spec.repo, "DietrichGebert/ponytail");
        match &spec.install {
            InstallKind::Content {
                keep,
                content_subdir,
                guard_source,
                ..
            } => {
                assert_eq!(*keep, 2, "prune parity with AFT");
                assert_eq!(*content_subdir, "skills");
                assert_eq!(*guard_source, "community:DietrichGebert/ponytail");
            }
            other => panic!("ponytail must install as Content, got {other:?}"),
        }
    }

    /// `installed_skills_dir` resolves the NEWEST version dir's skills/ and
    /// is None when nothing is installed.
    #[test]
    fn installed_skills_dir_finds_newest_and_none_when_absent() {
        // HERMES_HOME is process-global: serialise against every other
        // env-mutating test through the crate's lock (same pattern as
        // binary_updater's OPERANT_NO_AUTO_UPDATE tests).
        let _guard = crate::platform::test_env_lock_blocking();
        let base =
            std::env::temp_dir().join(format!("operant-ponytail-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // No install at all.
        // SAFETY: serialised by the crate's environment lock.
        unsafe {
            std::env::set_var("HERMES_HOME", base.join("home-a"));
        }
        assert!(
            installed_skills_dir().is_none(),
            "nothing installed -> None"
        );
        // Two version dirs: the newer wins.
        for (v, home) in [("5.3.1", "home-b"), ("5.3.2", "home-b")] {
            let skills = base
                .join(home)
                .join("integrations")
                .join(format!("ponytail-v{v}"))
                .join("skills");
            std::fs::create_dir_all(&skills).unwrap();
            std::fs::write(skills.join("marker.txt"), v).unwrap();
        }
        // SAFETY: serialised by the crate's environment lock.
        unsafe {
            std::env::set_var("HERMES_HOME", base.join("home-b"));
        }
        let got = installed_skills_dir().expect("newest version dir");
        assert_eq!(
            got,
            base.join("home-b")
                .join("integrations")
                .join("ponytail-v5.3.2")
                .join("skills")
        );
        assert_eq!(
            std::fs::read_to_string(got.join("marker.txt")).unwrap(),
            "5.3.2"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Phase 4 acceptance: `frozen_prefix_block` injects `<ponytail>` only
    /// when enabled+always_on (or the runtime override forces it), carries
    /// the installed ruleset + level suffix, and never injects on the
    /// default (off / on_invoke) config.
    #[test]
    fn frozen_prefix_block_gates_on_config_and_override() {
        let _posture = posture_override_test_guard();
        let _env = crate::platform::test_env_lock_blocking();
        let key = "HERMES_HOME";
        let original = std::env::var(key).ok();
        let base =
            std::env::temp_dir().join(format!("operant-ponytail-prefix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // A fake install with the core ponytail skill present.
        let skills = base
            .join("integrations")
            .join("ponytail-v9.9.9")
            .join("skills");
        std::fs::create_dir_all(skills.join("ponytail")).unwrap();
        std::fs::write(
            skills.join("ponytail").join("SKILL.md"),
            "---\nname: ponytail\ndescription: core posture\n---\nLAZY SENIOR DEV BODY\n",
        )
        .unwrap();
        // SAFETY: serialised by the crate's environment lock.
        unsafe { std::env::set_var(key, &base) };

        let off = crate::config::PonytailSettings::default();
        let on = crate::config::PonytailSettings {
            enabled: true,
            inject: crate::config::PonytailInject::AlwaysOn,
            ..off
        };

        // Default config: no block (skills only, zero prompt-byte change).
        assert!(frozen_prefix_block(&off).is_none());
        // on_invoke: still no block even when enabled.
        let invoke_only = crate::config::PonytailSettings {
            inject: crate::config::PonytailInject::OnInvoke,
            ..on.clone()
        };
        assert!(frozen_prefix_block(&invoke_only).is_none());
        // enabled + always_on: the block carries body + tags.
        let block = frozen_prefix_block(&on).expect("enabled+always_on must inject");
        assert!(block.contains("<ponytail>"));
        assert!(block.contains("LAZY SENIOR DEV BODY"));
        assert!(
            !block.contains("description: core posture"),
            "frontmatter stripped"
        );
        // full (default level) carries NO suffix line.
        assert!(!block.contains("Level:"));
        // lite / ultra carry their suffix.
        let lite_settings = crate::config::PonytailSettings {
            default_level: crate::config::PonytailLevel::Lite,
            ..on.clone()
        };
        let lite = frozen_prefix_block(&lite_settings).unwrap();
        assert!(lite.contains("Level: lite"));
        let ultra_settings = crate::config::PonytailSettings {
            default_level: crate::config::PonytailLevel::Ultra,
            ..on.clone()
        };
        let ultra = frozen_prefix_block(&ultra_settings).unwrap();
        assert!(ultra.contains("Level: ultra"));

        // Runtime override: forces injection even from the off config...
        set_posture_override(Some(PostureOverride::Level(
            crate::config::PonytailLevel::Ultra,
        )));
        let forced = frozen_prefix_block(&off).expect("override forces injection");
        assert!(
            forced.contains("Level: ultra"),
            "override level wins over config"
        );
        // ...and Off suppresses even the enabled config.
        set_posture_override(Some(PostureOverride::Off));
        assert!(frozen_prefix_block(&on).is_none());

        // Not installed: enabled config still injects NOTHING (no empty
        // shell) — point the home at an empty dir.
        set_posture_override(None);
        let empty = base.join("empty-home");
        std::fs::create_dir_all(&empty).unwrap();
        // SAFETY: serialised by the crate's environment lock.
        unsafe { std::env::set_var(key, &empty) };
        assert!(frozen_prefix_block(&on).is_none());

        match original {
            Some(val) => unsafe { std::env::set_var(key, val) },
            None => unsafe { std::env::remove_var(key) },
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
