//! Redraw cadence and performance tier system.
//!
//! Instead of rendering at a fixed 60fps regardless of terminal state, we
//! adapt the redraw interval based on:
//! - **Performance tier**: Minimal (SSH/WSL), Normal (default), High (local)
//! - **Activity state**: Streaming, idle, deep idle
//! - **Focus state**: Backgrounded tabs should not burn CPU
//!
//! This reduces CPU usage by 5-10x on idle terminals and dramatically
//! improves battery life on laptops.
//!
//! The accessibility preference `reduce_motion` (persisted in
//! `~/.operant/settings.json`, toggled from the settings screen) overrides the
//! detected tier down to [`PerformanceTier::Minimal`] and makes
//! [`crate::tui::render::shimmer_spans`] emit static text.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Process-wide `reduce_motion` flag, set once from the persisted setting at
/// startup and again whenever the settings screen toggles it.
///
/// Read by the shimmer helper, which has no `App` handle, so the value has to
/// live somewhere process-wide.
static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

/// Record the user's `reduce_motion` preference for the running process.
pub fn set_reduce_motion(enabled: bool) {
    REDUCE_MOTION.store(enabled, Ordering::Relaxed);
}

/// Whether the user asked for reduced motion.
pub fn reduce_motion_enabled() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed)
}

/// Performance tier — controls animation FPS and redraw cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PerformanceTier {
    /// SSH, WSL, or resource-constrained environments.
    /// Minimal animations, slowest redraw.
    Minimal = 0,
    /// Default tier — balanced performance and visuals.
    Normal = 1,
    /// Local terminal with full capability.
    /// Fastest animations, smoothest experience.
    High = 2,
}

impl PerformanceTier {
    /// Auto-detect the best tier based on environment variables and terminal capabilities.
    pub fn detect() -> Self {
        // Explicit override
        if let Ok(val) = std::env::var("OPERANT_PERF_TIER") {
            match val.to_lowercase().as_str() {
                "minimal" | "0" => return Self::Minimal,
                "normal" | "1" => return Self::Normal,
                "high" | "2" => return Self::High,
                _ => {}
            }
        }

        // SSH detection
        if std::env::var("SSH_CLIENT").is_ok() || std::env::var("SSH_TTY").is_ok() {
            return Self::Minimal;
        }

        // tmux/screen often indicates a remote or multiplexed session
        if std::env::var("TMUX").is_ok() {
            return Self::Normal;
        }

        // WSL detection
        if let Ok(osrelease) = std::fs::read_to_string("/proc/version")
            && osrelease.to_lowercase().contains("microsoft")
        {
            return Self::Normal;
        }

        // TERM_PROGRAM hints
        if let Ok(term) = std::env::var("TERM_PROGRAM") {
            match term.as_str() {
                "iTerm.app" | "WezTerm" | "ghostty" | "kitty" => return Self::High,
                "Apple_Terminal" | "Terminal.app" => return Self::Normal,
                _ => {}
            }
        }

        Self::Normal
    }

    /// Animation FPS for this tier.
    pub fn animation_fps(&self) -> u32 {
        match self {
            Self::Minimal => 15,
            Self::Normal => 30,
            Self::High => 60,
        }
    }

    /// Fast redraw FPS (for streaming, active input).
    pub fn fast_fps(&self) -> u32 {
        match self {
            Self::Minimal => 10,
            Self::Normal => 20,
            Self::High => 30,
        }
    }

    /// Whether decorative animations (idle animation, etc.) are enabled.
    pub fn animations_enabled(&self) -> bool {
        *self != Self::Minimal
    }

    /// Clamp the detected tier to the static level when the user asked for
    /// reduced motion.
    ///
    /// `Minimal` is the non-animated tier: [`PerformanceTier::animations_enabled`]
    /// is false for it, so the redraw interval falls back to `fast_fps`.
    pub fn with_reduce_motion(self, reduce_motion: bool) -> Self {
        if reduce_motion { Self::Minimal } else { self }
    }
}

/// Calculate the optimal redraw interval based on current state.
///
/// This is the core of the performance optimization: instead of a fixed
/// 16ms (60fps) interval, we choose the slowest interval that still feels
/// responsive for the current state.
///
/// The `idle_timeout` parameter defines how long (in seconds) of inactivity
/// before entering idle mode. If `None`, defaults to 5 seconds.
/// Deep idle (slowest cadence) kicks in at `idle_timeout * 6` seconds.
///
/// `is_focused` reflects whether the terminal window has keyboard focus. When
/// unfocused (backgrounded tab), we drop straight to the slowest cadence for
/// the tier so the process doesn't burn CPU/battery while invisible — the
/// event loop still wakes to drain agent events, but never re-renders
/// animations the user can't see.
pub fn redraw_interval(
    tier: PerformanceTier,
    is_streaming: bool,
    time_since_activity: Option<Duration>,
    idle_timeout: Option<Duration>,
    is_focused: bool,
) -> Duration {
    // Backgrounded tab: use the tier's slowest cadence regardless of activity
    // OR streaming state. The user can't see the output while the window is
    // unfocused, so rendering at animation speed would burn CPU for nothing;
    // the event loop still wakes to drain agent events and repaints whenever
    // the window regains focus (FocusGained interrupts the poll immediately).
    if !is_focused {
        return match tier {
            PerformanceTier::Minimal => Duration::from_secs(5),
            PerformanceTier::Normal => Duration::from_secs(2),
            PerformanceTier::High => Duration::from_secs(1),
        };
    }

    let idle_threshold = idle_timeout.unwrap_or(Duration::from_secs(5));
    let deep_idle_threshold = idle_threshold * 6;

    let since = time_since_activity.unwrap_or(Duration::ZERO);
    let is_idle = since >= idle_threshold;
    let is_deep_idle = since >= deep_idle_threshold;

    if is_streaming {
        return Duration::from_millis((1000 / tier.fast_fps()) as u64);
    }

    if is_deep_idle {
        return match tier {
            PerformanceTier::Minimal => Duration::from_secs(5),
            PerformanceTier::Normal => Duration::from_secs(2),
            PerformanceTier::High => Duration::from_secs(1),
        };
    }

    if is_idle {
        return match tier {
            PerformanceTier::Minimal => Duration::from_millis(500),
            PerformanceTier::Normal => Duration::from_millis(250),
            PerformanceTier::High => Duration::from_millis(200),
        };
    }

    if tier.animations_enabled() {
        Duration::from_millis((1000 / tier.animation_fps()) as u64)
    } else {
        Duration::from_millis((1000 / tier.fast_fps()) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::shimmer_spans;

    #[test]
    fn tier_ordinals_are_consistent() {
        assert!(PerformanceTier::Minimal < PerformanceTier::Normal);
        assert!(PerformanceTier::Normal < PerformanceTier::High);
    }

    #[test]
    fn minimal_tier_disables_animations() {
        assert!(!PerformanceTier::Minimal.animations_enabled());
        assert!(PerformanceTier::Normal.animations_enabled());
        assert!(PerformanceTier::High.animations_enabled());
    }

    #[test]
    fn streaming_always_uses_fast_interval() {
        let interval = redraw_interval(PerformanceTier::Minimal, true, None, None, true);
        assert!(interval <= Duration::from_millis(200));
    }

    #[test]
    fn deep_idle_uses_slowest_interval() {
        let interval = redraw_interval(
            PerformanceTier::High,
            false,
            Some(Duration::from_secs(60)),
            None,
            true,
        );
        assert!(interval >= Duration::from_secs(1));
    }

    #[test]
    fn unfocused_uses_slowest_cadence() {
        // A backgrounded tab must not burn CPU: even while actively streaming,
        // an unfocused terminal falls to the tier's slowest cadence.
        let since_activity = Some(Duration::ZERO);
        assert_eq!(
            redraw_interval(PerformanceTier::Minimal, true, since_activity, None, false),
            Duration::from_secs(5)
        );
        assert_eq!(
            redraw_interval(PerformanceTier::Normal, true, since_activity, None, false),
            Duration::from_secs(2)
        );
        assert_eq!(
            redraw_interval(PerformanceTier::High, true, since_activity, None, false),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn detection_returns_valid_tier() {
        let tier = PerformanceTier::detect();
        assert!(matches!(
            tier,
            PerformanceTier::Minimal | PerformanceTier::Normal | PerformanceTier::High
        ));
    }

    /// `reduce_motion` used to be persisted through four layers of config and
    /// read by nothing. It must now (a) kill the frame-count-driven shimmer
    /// and (b) drop the tier to its non-animated level.
    #[test]
    fn reduce_motion_should_disable_shimmer_and_force_static_tier() {
        // (a) The tier is clamped to the static level.
        for detected in [
            PerformanceTier::Minimal,
            PerformanceTier::Normal,
            PerformanceTier::High,
        ] {
            let clamped = detected.with_reduce_motion(true);
            assert_eq!(clamped, PerformanceTier::Minimal);
            assert!(!clamped.animations_enabled());
        }
        // …and left alone when the preference is off.
        assert_eq!(
            PerformanceTier::High.with_reduce_motion(false),
            PerformanceTier::High
        );

        // (b) The process-wide flag actually reaches the shimmer helper.
        assert!(!reduce_motion_enabled());
        let shimmering = [shimmer_spans("Thinking", 0), shimmer_spans("Thinking", 400)];
        // With motion enabled the sweep moves between frames.
        assert_ne!(
            shimmer_styles(&shimmering[0]),
            shimmer_styles(&shimmering[1])
        );

        set_reduce_motion(true);
        assert!(reduce_motion_enabled());
        let still = [shimmer_spans("Thinking", 0), shimmer_spans("Thinking", 400)];
        set_reduce_motion(false);
        // With motion disabled every frame is identical.
        assert_eq!(shimmer_styles(&still[0]), shimmer_styles(&still[1]));
        // The text is still there — reduced motion removes the sweep, not the label.
        let flat: String = still[0].iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(flat, "Thinking");

        // (c) The redraw cadence follows the clamped tier: with motion
        // reduced, the tick uses the static `fast_fps`, not `animation_fps`.
        let clamped = PerformanceTier::High.with_reduce_motion(true);
        assert!(!clamped.animations_enabled());
        let static_tick = Duration::from_millis(1000 / PerformanceTier::Minimal.fast_fps() as u64);
        assert_eq!(
            redraw_interval(clamped, false, None, None, true),
            static_tick
        );
        // The animated tier would have ticked faster.
        assert!(
            redraw_interval(PerformanceTier::High, false, None, None, true) < static_tick,
            "reduced motion must be at least as slow as the static cadence"
        );
    }

    /// The style of every span, for asserting shimmer output without a buffer.
    fn shimmer_styles(spans: &[ratatui::text::Span<'_>]) -> Vec<(String, ratatui::style::Color)> {
        spans
            .iter()
            .map(|s| {
                (
                    s.content.to_string(),
                    s.style.fg.unwrap_or(ratatui::style::Color::Reset),
                )
            })
            .collect()
    }
}
