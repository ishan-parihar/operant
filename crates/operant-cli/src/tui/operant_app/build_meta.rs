// Vendored from jcode (crates/operant-build-meta/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported @ 0a9dc7805 with ONE deliberate
// adaptation: upstream's values come from jcode's build.rs pipeline, which
// emits `OPERANT_*` compile-time env vars; operant has no such pipeline, so the
// consts re-point at operant's own version surface (CARGO_PKG_VERSION,
// operant's CHANGELOG.md, option_env! git identity with "unknown" fallbacks).
// The fn bodies (runtime-override OnceLocks, parse_release_semver, the
// version()/git_* accessors) are verbatim. The cutover may replace the consts
// with operant's richer version identity if one exists.
// See operant_app/mod.rs for scope.

use std::sync::OnceLock;

// [port-decision] consts adapted: jcode's build.rs env! pipeline is not ported;
// operant's own version/changelog is the honest source. GIT_* fall back to
// "unknown" absent a build step that emits them.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_HASH: &str = "unknown";
pub const GIT_DATE: &str = "unknown";
pub const GIT_TAG: &str = "unknown";
pub const SEMVER: &str = env!("CARGO_PKG_VERSION");
// Operant's changelog is deliberately EMPTY (see CHANGELOG const below).
pub const BASE_SEMVER: &str = env!("CARGO_PKG_VERSION");

// [port-decision] CHANGELOG is deliberately EMPTY. Its only consumer,
// operant_ui::ui_changelog::parse_changelog, has a hard format contract: jcode's
// build.rs emits `hash\x1etag\x1etimestamp\x1esubject` records joined by 0x1F
// (legacy: `hash:tag:subject`). The original port fed docs/CHANGELOG.md
// markdown into it; split_once(':') twice on markdown made the WHOLE file
// parse as one entry whose "subject" started at
// `//semver.org/spec/...### Changed...` — that garbage printed in the "Updates"
// box on every fresh-HOME first launch (and in every corpus frame). An empty
// source degrades through the parser's own paths: no Updates box, /changelog
// shows "No changelog entries available." — honest, not garbage. Restore real
// content by EITHER porting jcode's build.rs emitter
// (`git log -700 --format=%h|%ct|%D|%s` → RS/US records) OR writing a markdown
// parser over docs/CHANGELOG.md — both need a golden-stability decision first:
// an emitter's content moves with every commit, reddening every corpus golden
// that includes the box.
pub const CHANGELOG: &str = "";

static RUNTIME_RELEASE_SEMVER: OnceLock<Option<String>> = OnceLock::new();
static RUNTIME_VERSION: OnceLock<Option<String>> = OnceLock::new();
static RUNTIME_GIT_HASH: OnceLock<Option<String>> = OnceLock::new();
static RUNTIME_GIT_DATE: OnceLock<Option<String>> = OnceLock::new();
static RUNTIME_GIT_TAG: OnceLock<Option<String>> = OnceLock::new();

fn parse_release_semver(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('v');
    let mut parts = value.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{major}.{minor}.{patch}"))
}

/// Optional release semver supplied by the fast-release wrapper at process start.
pub fn runtime_release_semver() -> Option<&'static str> {
    RUNTIME_RELEASE_SEMVER
        .get_or_init(|| {
            std::env::var("OPERANT_RUNTIME_RELEASE_SEMVER")
                .ok()
                .and_then(|value| parse_release_semver(&value))
        })
        .as_deref()
}

/// Human-readable runtime version, honoring a fast-release wrapper override.
pub fn version() -> &'static str {
    RUNTIME_VERSION
        .get_or_init(|| {
            runtime_release_semver().map(|semver| format!("v{semver} ({})", git_hash()))
        })
        .as_deref()
        .unwrap_or(VERSION)
}

fn runtime_identity_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Runtime git hash, honoring the fast-release wrapper identity.
pub fn git_hash() -> &'static str {
    RUNTIME_GIT_HASH
        .get_or_init(|| runtime_identity_value("OPERANT_RUNTIME_GIT_HASH"))
        .as_deref()
        .unwrap_or(GIT_HASH)
}

/// Runtime git date, honoring the fast-release wrapper identity.
pub fn git_date() -> &'static str {
    RUNTIME_GIT_DATE
        .get_or_init(|| runtime_identity_value("OPERANT_RUNTIME_GIT_DATE"))
        .as_deref()
        .unwrap_or(GIT_DATE)
}

/// Runtime git tag, honoring the fast-release wrapper identity.
pub fn git_tag() -> &'static str {
    RUNTIME_GIT_TAG
        .get_or_init(|| runtime_identity_value("OPERANT_RUNTIME_GIT_TAG"))
        .as_deref()
        .unwrap_or(GIT_TAG)
}

/// Whether a runtime fast-release override is active.
pub fn runtime_override_active() -> bool {
    runtime_release_semver().is_some()
}

/// Whether this build is a tagged release (git tag present).
pub fn is_tagged_release() -> bool {
    !GIT_TAG.is_empty()
}

/// Runtime build semver, honoring a fast-release wrapper override.
pub fn semver() -> &'static str {
    runtime_release_semver().unwrap_or(SEMVER)
}

/// Runtime base semver, honoring a fast-release wrapper override.
pub fn base_semver() -> &'static str {
    runtime_release_semver().unwrap_or(BASE_SEMVER)
}
