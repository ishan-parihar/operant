// app/notice_ttl_tests.rs — the transient notices must LEAVE.
//
// The operator's report: "certain notifications do not have the TTL and they
// persist like 'copied to clipboard via wl-copy'".
//
// The mechanism was not the writer but the READERS: a notice set through the
// TTL seam (`set_status_notice`) stamped `status_message_set_at`, but the
// notification line read `status_message` RAW and the row-reservation check
// asked only "is the field set". The field was never cleared, so a notice
// that had served its three seconds stayed on screen for the rest of the
// session — and kept its row.

use crate::tui::operant_app::tui_state::TuiState as _;

/// A notice that has served its TTL: the clock is moved back, so the test
/// does not have to sleep.
fn age_out(app: &mut crate::tui::app::App) {
    app.status_message_set_at = Some(
        std::time::Instant::now()
            - std::time::Duration::from_secs(crate::tui::app::App::STATUS_NOTICE_TTL.as_secs() + 1),
    );
}

/// THE REPORTED CASE: the clipboard notice must not outlive its TTL.
///
/// MUTATION-PROVEN: reading `status_message` raw in `status_notice` (or
/// asking only "is the field set" in `has_notification`) fails this test by
/// name.
#[test]
fn the_clipboard_notice_expires() {
    let mut app = super::tests::make_app();
    app.push_notice(
        crate::tui::app::NotificationKind::Info,
        "Copied to clipboard via wl-copy.".to_string(),
    );
    assert_eq!(
        app.status_notice().as_deref(),
        Some("Copied to clipboard via wl-copy."),
        "the notice is up while it is fresh"
    );
    assert!(
        app.has_notification(),
        "and it reserves the notification row"
    );

    age_out(&mut app);

    assert!(
        app.status_notice().is_none(),
        "a notice past its TTL must leave the notification line: {:?}",
        app.status_notice()
    );
    assert!(
        !app.has_notification(),
        "and it must release the notification row"
    );
}

/// The same contract for a notice the operator raised with a command.
#[test]
fn a_command_notice_expires() {
    let mut app = super::tests::make_app();
    app.set_status_notice("Theme set to: dark");
    assert_eq!(app.status_notice().as_deref(), Some("Theme set to: dark"));
    age_out(&mut app);
    assert!(app.status_notice().is_none(), "the command notice expired");
}

/// A notice that is STATE, not an event, stays until it is replaced: the
/// explicit persistent seam.
#[test]
fn a_persistent_notice_stays() {
    let mut app = super::tests::make_app();
    // A timed notice first, so the clock is already past a TTL when the
    // persistent notice is set: the persistent setter clears the timestamp,
    // and the notice must not inherit the dead one.
    app.set_status_notice("Theme set to: dark");
    age_out(&mut app);
    app.set_status_message_persistent("Copy mode: drag to select (Esc or Alt+Y to exit).");
    assert_eq!(
        app.status_notice().as_deref(),
        Some("Copy mode: drag to select (Esc or Alt+Y to exit)."),
        "a persistent notice has no TTL"
    );
    assert!(app.has_notification());
}

/// The expired field is CLEARED, not merely hidden: otherwise the state
/// lingers and the redraw reason keeps firing for a dead notice.
#[test]
fn an_expired_notice_is_cleared() {
    let mut app = super::tests::make_app();
    app.set_status_notice("Steer queued: do the thing");
    age_out(&mut app);
    app.expire_status_notice();
    assert!(
        app.status_message.is_none(),
        "the expired notice is dropped from the state"
    );
    assert!(app.status_message_set_at.is_none());
    assert!(app.status_notice().is_none());
    assert!(!app.has_notification());
}
