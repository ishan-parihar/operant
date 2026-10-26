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
}
