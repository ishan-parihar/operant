//! Notification taxonomy for the P4-2 post-purge surface.
//!
//! The overlay-banner queue, the countdown progress bar, the error modal
//! and the `OPERANT_FROZEN_NOTIFICATION_CLOCK` test seam are all gone
//! (2026-10-12 purge). What remains is the *classification* —
//! [`NotificationKind`] — consumed by [`App::push_notice`](crate::tui::app::App::push_notice) to route:
//! transient kinds to the 3s status-line notice, `Error` to a persistent
//! inline transcript card.

/// Classification carried by every notice-producing site. The kind decides
/// the destination surface, not the styling — see
/// `App::push_notice` (tui/app/messaging.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Info,
    Warning,
    Error,
}
