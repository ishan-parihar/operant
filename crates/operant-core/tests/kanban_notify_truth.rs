//! RT-430: the kanban subscriber log claimed a notification that never
//! happened.
//!
//! `NotifyManager::notify_subscribers` logged "Notify {task} on {platform}
//! ({chat}) about '{event}': {payload}" — the operator reading that line
//! would believe the subscriber was told. Nothing was sent: the manager
//! holds the subscription database and no platform handle, and the
//! function has no caller in the workspace.
//!
//! SOURCE contract: the failure is not forceable (there is no delivery to
//! fail), so what is pinned is the log's honesty.

use std::path::Path;

fn production_region() -> String {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("kanban")
            .join("notify.rs"),
    )
    .expect("notify.rs exists");
    match src.find("#[cfg(test)]") {
        Some(i) => src[..i].to_string(),
        None => src,
    }
}

#[test]
fn the_subscriber_log_does_not_claim_a_delivery() {
    let prod = production_region();
    // The old claim.
    assert!(
        !prod.contains("\"Notify {} on {} ({}) about '{}': {}\""),
        "the log must not claim the subscriber was notified"
    );
    // The truth.
    let not_delivered = concat!("subscription matched, NOT delivered", "");
    assert_eq!(
        prod.matches(not_delivered).count(),
        1,
        "the log must say the subscription was matched but NOT delivered"
    );
    let unwired = concat!("delivery path is not wired", "");
    assert_eq!(
        prod.matches(unwired).count(),
        1,
        "the log must name the missing delivery path"
    );
}
