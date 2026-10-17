//! Background auto-update polling for the sourcehound binary.
//!
//! (plan 11B) The download/verify/replace/prune/throttle machinery now
//! lives in `crate::binary_updater` — shared with the aft bridge. This
//! module only builds the sourcehound `IntegrationSpec` and kicks the
//! once-per-session check from the first sourcehound tool call.

use crate::binary_updater::{InstallKind, IntegrationSpec};

/// The sourcehound integration spec: single binary on PATH, SHA256SUMS
/// releases, linux/x86_64 asset only (verified against real published
/// names).
pub fn sourcehound_spec() -> Option<IntegrationSpec> {
    let bin = super::sourcehound::find_sourcehound_binary()?;
    Some(IntegrationSpec {
        name: "sourcehound",
        repo: "ishan-parihar/sourcehound",
        asset_marker: "linux-x86_64",
        install: InstallKind::ReplaceOnPath(bin),
    })
}

/// Kick the once-per-session background update check. Cheap by
/// construction: an atomic once-guard, a PATH lookup at most once, and the
/// network work happens on a spawned task the caller never awaits. The
/// 6 h on-disk throttle means a fresh process re-polls at most four times
/// a day instead of once per session.
pub fn kick_update_check() {
    if let Some(spec) = sourcehound_spec() {
        crate::binary_updater::kick_session_update(spec);
    }
}

/// Version check for doctor / `/version` — no install, no throttle.
pub async fn check() -> Option<crate::binary_updater::Versions> {
    crate::binary_updater::check(&sourcehound_spec()?)
        .await
        .ok()
}
