// notifications.rs — Notification / banner system for the TUI.

use std::collections::VecDeque;
use std::time::Instant;

use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;
use unicode_width::UnicodeWidthStr;

/// Severity / visual style of a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationKind {
    Info,
    Warning,
    Error,
    Success,
}

/// A single notification entry.
#[derive(Debug, Clone)]
pub struct Notification {
    /// Unique identifier (used for dismissal — currently only used in tests).
    #[allow(dead_code)] // Unique notification ID
    pub id: String,
    pub kind: NotificationKind,
    pub message: String,
    /// When this notification was created — used to calculate progress bar fill.
    pub pushed_at: Instant,
    /// When `Some`, the notification auto-expires at this instant.
    pub expires_at: Option<Instant>,
    // (iter-140: dismissible field deleted — was always true, never read
    // outside dismiss_current() which always checked it. Now all
    // notifications are dismissible.)
}

/// A FIFO queue of active notifications.
#[derive(Debug, Default)]
pub struct NotificationQueue {
    pub notifications: VecDeque<Notification>,
    next_id: u64,
}

/// The notification timeline's clock.
///
/// Corpus determinism seam (2026-10-09 live-audit wave): the banner's
/// shrinking progress bar is driven by `(exp - now)` against a real clock,
/// so a golden captured at a random moment shows the bar at a random width
/// — the documented wall-clock drift that made `first-message` and
/// `tool-block` flaky and any new banner-bearing scenario nondeterministic.
/// With `OPERANT_FROZEN_NOTIFICATION_CLOCK` set (the corpus PINNED_ENV
/// does), the timeline latches to the first read: banners render
/// full-width and never expire mid-run, byte-stable across runs. Real
/// sessions never set the var and keep live countdowns.
fn now() -> Instant {
    static FROZEN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    if std::env::var_os("OPERANT_FROZEN_NOTIFICATION_CLOCK").is_some() {
        return *FROZEN.get_or_init(Instant::now);
    }
    Instant::now()
}

impl NotificationQueue {
    pub fn new() -> Self {
        Self {
            notifications: VecDeque::new(),
            next_id: 0,
        }
    }

    /// Push a new notification.
    ///
    /// * `duration_secs` — `None` for persistent, `Some(n)` for auto-expire after *n* seconds.
    pub fn push(&mut self, kind: NotificationKind, msg: String, duration_secs: Option<u64>) {
        let pushed_at = now();
        let expires_at = duration_secs.map(|secs| pushed_at + std::time::Duration::from_secs(secs));
        self.notifications
            .retain(|n| !(n.kind == kind && n.message == msg));
        let id = format!("notif-{}", self.next_id);
        self.next_id += 1;
        self.notifications.push_back(Notification {
            id,
            kind,
            message: msg,
            pushed_at,
            expires_at,
        });
    }

    /// Dismiss the notification with the given `id`.
    #[allow(dead_code)] // Notification dismissal
    pub fn dismiss(&mut self, id: &str) {
        self.notifications.retain(|n| n.id != id);
    }

    /// Remove all expired notifications.  Call this once per render frame.
    pub fn tick(&mut self) {
        let now = now();
        self.notifications
            .retain(|n| n.expires_at.is_none_or(|exp| exp > now));
    }

    /// Return the currently visible (most recent) notification, if any.
    pub fn current(&self) -> Option<&Notification> {
        self.notifications.back()
    }

    /// Dismiss the currently visible notification.
    pub fn dismiss_current(&mut self) {
        // (iter-140: removed dismissible check — all notifications are
        // now dismissible since the field was always true.)
        self.notifications.pop_back();
    }

    pub fn current_is_error(&self) -> bool {
        self.current()
            .is_some_and(|n| n.kind == NotificationKind::Error)
    }

    /// Return `true` if there are no active notifications.
    pub fn is_empty(&self) -> bool {
        self.notifications.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

impl NotificationKind {
    pub fn color(&self) -> Color {
        match self {
            NotificationKind::Info => theme_colors::accent(),
            NotificationKind::Warning => theme_colors::warning(),
            NotificationKind::Error => theme_colors::error(),
            NotificationKind::Success => theme_colors::success(),
        }
    }

    /// Ink for the notification *body*, which is a different job from
    /// [`Self::color`]: the frame colour says "which channel is this", the body
    /// colour says "how bad is it". A failure therefore has to read as a
    /// failure from its body alone — at a glance, in peripheral vision, and in
    /// both themes, since every value here is a role rather than a literal and
    /// so resolves through the palette in light and dark alike.
    pub fn body_color(&self) -> Color {
        match self {
            NotificationKind::Error => theme::error_color(),
            NotificationKind::Warning => theme::warning_color(),
            NotificationKind::Success => theme::success_color(),
            NotificationKind::Info => theme_colors::text(),
        }
    }

    /// Failures are the only kind that shouts. A warning is already a second
    /// notch down (icon + frame are both amber), so bolding it too would cost
    /// the error its one remaining signal.
    pub fn body_is_emphasis(&self) -> bool {
        matches!(self, NotificationKind::Error)
    }

    pub fn icon(&self) -> &'static str {
        match self {
            NotificationKind::Info => "ℹ",
            NotificationKind::Warning => "⚠",
            NotificationKind::Error => "✗",
            NotificationKind::Success => "✓",
        }
    }
}

// ---------------------------------------------------------------------------
// Actionable wording
// ---------------------------------------------------------------------------

/// Name the keystroke or command that gets the user out of this failure.
///
/// A notification that says only *what* went wrong leaves the user to guess the
/// remedy, which for a provider failure is not guessable: the fix is `/login`
/// for one class, `/model` for another, `/compact` for a third, and the
/// message text itself never says which. This is the jcode rule — a failure
/// states its recovery action — applied at the one place every failure passes
/// through.
///
/// `None` means "there is nothing useful to add": either the notice is not a
/// failure (`Info` / `Success` need no remedy), or it is a failure with no
/// user-reachable lever, in which case inventing one would send the user
/// pressing keys that cannot work.
pub fn recovery_action(kind: &NotificationKind, message: &str) -> Option<&'static str> {
    // Only the two severities that imply an unmet need. An informational
    // notice is not a problem, and a success has nothing to recover from.
    if !matches!(kind, NotificationKind::Error | NotificationKind::Warning) {
        return None;
    }

    // Lowercased once; every probe below is a substring test on it.
    let m = message.to_ascii_lowercase();

    // Order is by *specificity of the fix*, not by HTTP status. A credential
    // failure and a permission failure both arrive as 4xx, but their remedies
    // are different levers (`/login` vs `/model`), so the narrower match has to
    // win before the broader one can swallow it. Quota is checked before rate
    // limit because a billing wall is reported as a 429 and re-authenticating
    // it accomplishes nothing.
    const CLASSES: &[(&[&str], &str)] = &[
        // ── credentials: the key is wrong, not the request ──
        (
            &[
                "401",
                "unauthorized",
                "invalid api key",
                "invalid_api_key",
                "api key",
                "credential",
                "no api key",
            ],
            "Run /login to re-authenticate, or /model to switch to a working route, then send again.",
        ),
        // ── billing: the account is empty, retrying cannot help ──
        (
            &["quota", "billing", "insufficient funds", "payment required"],
            "This account is out of credit — top it up, or /model to switch to a working route.",
        ),
        // ── authorization: the route works, this account may not use it ──
        (
            &["403", "forbidden", "access denied", "permission denied"],
            "The provider denied this request — /model switches to a route this account can reach.",
        ),
        // ── the named model is gone ──
        (
            &[
                "404",
                "model not found",
                "no such model",
                "unsupported model",
            ],
            "Run /model to pick a route this account can reach, then send again.",
        ),
        // ── the conversation outgrew the window ──
        (
            &[
                "context length",
                "maximum context",
                "context window exceeded",
                "context window full",
                "prompt is too long",
            ],
            "Run /compact to shrink the conversation, then send again.",
        ),
        // ── throttle ──
        (
            &["429", "rate limit", "too many requests"],
            "Rate limited — this retries with backoff; /model switches to a working route now.",
        ),
        // ── the provider itself is sick ──
        (
            &[
                "503",
                "502",
                "529",
                "overloaded",
                "bad gateway",
                "service unavailable",
            ],
            "The provider is degraded — retry in a moment, or /model to switch route.",
        ),
        // ── the request outlived its budget ──
        (
            &["408", "timeout", "timed out"],
            "The request timed out — send again to retry, or /model to switch route.",
        ),
        // ── we never reached the provider ──
        (
            &[
                "connection refused",
                "connection reset",
                "connection closed",
                "connection error",
                "connection",
                "broken pipe",
                "network",
                "dns",
                "unexpected eof",
            ],
            "Network unreachable — check connectivity, then send again.",
        ),
    ];

    CLASSES
        .iter()
        .find(|(needles, _)| needles.iter().any(|n| m.contains(n)))
        .map(|(_, action)| *action)
}

/// Render the topmost notification as a floating toast at the top-right of `area`.
///
/// Layout (3 rows):
///   row 0: ▐ `icon` `message truncated`          `Esc` ▌
///   row 1: ▐ [progress bar for timed notifs]            ▌
///   row 2: ▐ [`→ recovery action` | blank]                ▌
///
/// Row 2 was padding. It is the only row the toast had to spare, and a remedy
/// is exactly the thing that does not fit on the message line, so it went there
/// rather than growing the toast.
pub fn render_notification_banner(frame: &mut Frame, queue: &NotificationQueue, area: Rect) {
    let notif = match queue.current() {
        Some(n) => n,
        None => return,
    };

    // Toast width: 48 cols max, right-aligned with a 2-col right margin.
    let toast_width = 52u16.min(area.width.saturating_sub(4));
    if toast_width < 20 {
        return;
    }
    // Adapt toast height based on available space, minimum 1 row for the message
    let toast_height = 3u16.min(area.height.saturating_sub(1).max(1));
    let toast_area = Rect {
        x: area.x + area.width.saturating_sub(toast_width + 2),
        // Position at bottom if not enough space at top
        y: if area.height >= 4 {
            area.y + 1
        } else {
            area.y + area.height.saturating_sub(toast_height)
        },
        width: toast_width,
        height: toast_height,
    };

    let color = notif.kind.color();
    // A toast raised above the app's `user_bg` base fill. `SelectionBg` is the
    // palette's only raised-surface role; `vendor/**` is frozen, so a dedicated
    // toast role is not available here.
    let bg = theme::selection_bg_color();

    // Clear the area so the toast has a distinct background.
    frame.render_widget(Clear, toast_area);

    // ── Row 0: icon + message + optional "Esc" hint ──
    let inner_w = toast_width.saturating_sub(4) as usize; // 2 side bars + 1 pad each side
    let esc_hint = "  esc";
    let icon_with_spaces = format!(" {} ", notif.kind.icon());
    let icon_width = icon_with_spaces.width();
    let esc_width = esc_hint.width();

    // Available width for message: use inner_w as the base
    let msg_width_budget = inner_w.saturating_sub(icon_width + esc_width);

    // Truncate message based on display width, not character count
    let message = {
        let msg_width = notif.message.width();
        if msg_width > msg_width_budget {
            // Truncate character by character, checking width until we fit
            let mut truncated = String::new();
            for ch in notif.message.chars() {
                let test = format!("{}{}", truncated, ch);
                if test.width() + 1 > msg_width_budget {
                    // +1 for ellipsis
                    break;
                }
                truncated.push(ch);
            }
            format!("{}…", truncated)
        } else {
            notif.message.clone()
        }
    };

    let mut row0_spans = vec![
        Span::styled(
            icon_with_spaces.clone(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            message,
            body_style(&notif.kind).add_modifier(if notif.kind.body_is_emphasis() {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ),
    ];
    if true {
        row0_spans.push(Span::styled(
            esc_hint.to_string(),
            Style::default().fg(theme_colors::muted()),
        ));
    }

    // ── Row 1: thin progress bar for timed notifications ──
    let progress_line = if let Some(exp) = notif.expires_at {
        let now = now();
        let remaining = if exp > now {
            (exp - now).as_millis()
        } else {
            0
        };
        let total_ms = (exp - notif.pushed_at).as_millis().max(1);
        let frac = (remaining as f64 / total_ms as f64).min(1.0);
        let bar_w = (inner_w as f64 * frac) as usize;
        let bar_w = bar_w.min(inner_w);
        let filled: String = "─".repeat(bar_w);
        let empty: String = " ".repeat(inner_w.saturating_sub(bar_w));
        Line::from(vec![
            Span::styled(format!(" {}", filled), Style::default().fg(color)),
            Span::styled(empty, Style::default().fg(theme_colors::muted())),
            Span::raw(" "),
        ])
    } else {
        Line::from(Span::styled(
            format!(" {}", "─".repeat(inner_w)),
            Style::default().fg(theme_colors::border()),
        ))
    };

    // Render background and borders to the buffer
    {
        let buf = frame.buffer_mut();

        // Helper: paint a full row with bg color, with bounds checking
        let paint_row = |buf: &mut ratatui::buffer::Buffer, row: u16| {
            if toast_area.y + row >= buf.area().bottom() {
                return;
            }
            for col in 0..toast_width {
                let x = toast_area.x + col;
                if x >= buf.area().right() {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, toast_area.y + row)) {
                    cell.set_bg(bg);
                }
            }
        };
        for row in 0..toast_height {
            paint_row(buf, row);
        }

        // Left accent bar (all rows)
        if toast_area.x < buf.area().right() {
            for row in 0..toast_height {
                if toast_area.y + row < buf.area().bottom()
                    && let Some(cell) = buf.cell_mut((toast_area.x, toast_area.y + row))
                {
                    cell.set_bg(bg);
                    cell.set_fg(color);
                    cell.set_char('▌');
                }
            }
        }
        // Right border bar (all rows)
        let right_x = toast_area.x + toast_width.saturating_sub(1);
        if right_x < buf.area().right() && toast_area.x < buf.area().right() {
            for row in 0..toast_height {
                if toast_area.y + row < buf.area().bottom()
                    && let Some(cell) = buf.cell_mut((right_x, toast_area.y + row))
                {
                    cell.set_bg(bg);
                    cell.set_fg(theme_colors::border());
                    cell.set_char('▐');
                }
            }
        }
    }

    // Render widgets (message, progress, padding)
    // Row 0: message (always show if space allows)
    if toast_area.y < frame.area().height {
        let msg_rect = Rect {
            x: toast_area.x + 1,
            y: toast_area.y,
            width: toast_width.saturating_sub(2),
            height: 1,
        };
        let para0 = Paragraph::new(Line::from(row0_spans)).style(Style::default().bg(bg));
        frame.render_widget(para0, msg_rect);
    }

    // Row 1: progress / divider (if space allows)
    if toast_height > 1 && toast_area.y + 1 < frame.area().height {
        let prog_rect = Rect {
            x: toast_area.x + 1,
            y: toast_area.y + 1,
            width: toast_width.saturating_sub(2),
            height: 1,
        };
        let para1 = Paragraph::new(progress_line).style(Style::default().bg(bg));
        frame.render_widget(para1, prog_rect);
    }

    // ── Row 2: the recovery action, or blank ──
    let recovery = recovery_action(&notif.kind, &notif.message).map(|action| {
        // `inner_w` already excludes both border columns and one pad each side.
        crate::tui::render::truncate_text(&format!("\u{2192} {action}"), inner_w)
    });
    let row2_line = match &recovery {
        Some(text) if !text.is_empty() => {
            Line::from(Span::styled(format!(" {text}"), body_style(&notif.kind)))
        }
        _ => Line::from(""),
    };
    if toast_height > 2 && toast_area.y + 2 < frame.area().height {
        let pad_rect = Rect {
            x: toast_area.x + 1,
            y: toast_area.y + 2,
            width: toast_width.saturating_sub(2),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(row2_line).style(Style::default().bg(bg)),
            pad_rect,
        );
    }
}

/// Body style for a notification, resolved through the semantic roles so a
/// failure reads as a failure in both themes without this file picking a
/// colour.
fn body_style(kind: &NotificationKind) -> Style {
    Style::default().fg(kind.body_color())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_current() {
        let mut q = NotificationQueue::new();
        assert!(q.current().is_none());
        q.push(NotificationKind::Info, "hello".to_string(), None);
        assert_eq!(q.current().unwrap().message, "hello");
    }

    #[test]
    fn dismiss_by_id() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Warning, "warn".to_string(), None);
        let id = q.current().unwrap().id.clone();
        q.dismiss(&id);
        assert!(q.is_empty());
    }

    #[test]
    fn current_prefers_latest_notification() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Warning, "older".to_string(), None);
        q.push(NotificationKind::Info, "newer".to_string(), Some(3));
        assert_eq!(q.current().unwrap().message, "newer");
        q.dismiss_current();
        assert_eq!(q.current().unwrap().message, "older");
    }

    #[test]
    fn duplicate_notification_is_refreshed_not_duplicated() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Info, "same".to_string(), Some(3));
        q.push(NotificationKind::Info, "same".to_string(), Some(5));
        assert_eq!(q.notifications.len(), 1);
    }

    #[test]
    fn tick_removes_expired() {
        let mut q = NotificationQueue::new();
        // Push a notification that expired in the past
        q.notifications.push_back(super::Notification {
            id: "x".to_string(),
            kind: NotificationKind::Info,
            message: "gone".to_string(),
            pushed_at: Instant::now(),
            expires_at: Some(Instant::now() - std::time::Duration::from_secs(1)),
        });
        assert!(!q.is_empty());
        q.tick();
        assert!(q.is_empty());
    }

    #[test]
    fn persistent_notification_survives_tick() {
        let mut q = NotificationQueue::new();
        q.push(NotificationKind::Success, "persistent".to_string(), None);
        q.tick();
        assert!(!q.is_empty());
    }

    // ---- recovery actions ------------------------------------------------
    //
    // The classifier is pure string matching over a message the caller did not
    // write here, so the pins below are the only thing holding "a 401 says
    // /login" and "a 429 says /model" apart. Each case is a real provider error
    // string from `operant-providers`, not a paraphrase.

    /// Every error a provider can produce has to land on *some* lever, or the
    /// user is left with the raw status code. `None` is only correct when the
    /// message carries no failure semantics at all.
    #[test]
    fn actionable_errors_cover_the_real_provider_vocabulary() {
        let cases: &[(&str, &str)] = &[
            ("API error (401 Unauthorized): invalid api key", "/login"),
            ("401 unauthorized", "/login"),
            ("Error: exceeded your current quota", "out of credit"),
            (
                "API error (403 Forbidden): access denied",
                "denied this request",
            ),
            ("API error (404 Not Found): model not found", "/model"),
            (
                "Error: context length exceeded, reduce the length",
                "/compact",
            ),
            ("429 Too Many Requests: rate limit exceeded", "/model"),
            ("Error: overloaded_error (529)", "/model"),
            ("502 Bad Gateway", "/model"),
            ("408 Request Timeout", "timed out"),
            ("Claude Code request timed out after 30s", "timed out"),
            ("connection refused", "Network unreachable"),
            ("Error: connection reset by peer", "Network unreachable"),
        ];
        for (message, expected) in cases {
            let action = recovery_action(&NotificationKind::Error, message);
            assert!(
                action.is_some_and(|a| a.contains(expected)),
                "error {message:?} named no {expected:?} remedy (got {action:?})"
            );
        }
    }

    /// The narrower match has to win: a credential failure and a permission
    /// failure are both 4xx but are fixed by different keys, and a billing wall
    /// arrives as a 429. Mis-attributing any of these sends the user to a lever
    /// that cannot move.
    #[test]
    fn the_specific_class_beats_the_general_one() {
        // A 401 says "invalid api key" — quota/rate-limit/permission probes must
        // not swallow it.
        let auth = recovery_action(
            &NotificationKind::Error,
            "API error (401 Unauthorized): invalid api key",
        );
        assert!(auth.is_some_and(|a| a.contains("/login")));

        // A billing wall is reported as a rate limit; re-authenticating is
        // useless, so the quota remedy must win.
        let quota = recovery_action(&NotificationKind::Error, "429: exceeded your current quota");
        assert!(quota.is_some_and(|a| a.contains("out of credit")));

        // A 404 must not be read as a bare "not found" belonging to something
        // else (a missing file has no /model remedy).
        let model = recovery_action(&NotificationKind::Error, "aft read: file not found");
        assert!(model.is_none(), "a file-not-found is not a routing failure");
    }

    /// A notice with nothing to recover from must return `None` rather than an
    /// invented lever — a key that cannot work is worse than no key.
    #[test]
    fn non_failures_and_unclassifiable_failures_name_no_action() {
        // Non-failures never carry a remedy.
        for kind in [NotificationKind::Info, NotificationKind::Success] {
            assert_eq!(
                recovery_action(&kind, "401 Unauthorized: invalid api key"),
                None,
                "{kind:?} is not a failure and must not claim a remedy"
            );
        }
        // A failure with no user-reachable lever stays bare.
        assert_eq!(
            recovery_action(&NotificationKind::Error, "the shader failed to link"),
            None
        );
        // The soft context warning already names /compact, so it must not also
        // grow a remedy line saying the same thing twice.
        assert_eq!(
            recovery_action(
                &NotificationKind::Warning,
                "Context window 80% full. Consider /compact."
            ),
            None
        );
    }

    /// A persistent rate-limit warning is the one notice the user must be able
    /// to act on without waiting, so it has to name the switch.
    #[test]
    fn rate_limit_warning_names_the_route_switch() {
        let action = recovery_action(
            &NotificationKind::Warning,
            "Rate limit reached — retry in ~30s",
        );
        assert!(action.is_some_and(|a| a.contains("/model")));
    }
}
