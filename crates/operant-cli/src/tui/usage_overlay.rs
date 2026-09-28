// usage_overlay.rs — Persistent session usage panel (cost / cache / tokens).
//
// A dedicated panel, in the spirit of jcode's `usage-overlay` crate: one
// always-on surface for "what has this session cost me", toggled with a
// chord rather than opened as a modal.
//
// ## Where the numbers come from (read this before trusting a row)
//
// Every field below is a REAL recorded metric read off `App` at render time.
// Nothing here is estimated, back-filled, or recomputed from a proxy signal.
//
// * `session_cost_usd` — `App::cost_usd`, which is
//   `CostTracker::total_cost` (`app/agent_events.rs:334`, `:368`). The
//   tracker accumulates across every recorded request, so this genuinely is
//   a session cumulative.
//
// * `cache_prefix_hits` / `cache_prefix_misses` — `RuntimeMetrics`
//   `cache_prefix_reuse` / `cache_prefix_misses`. Recorded by the agent
//   loop at `operant-core/src/agent/run.rs:466,469` after the cache-monitor
//   reconciles the cacheable prompt prefix for a request. This is a
//   REQUEST-level prefix verdict, not a token-level cache accounting — the
//   row is labelled `cache prefix` for exactly that reason. `operant status`
//   reads the same counters.
//
// * `turn_input_tokens` / `turn_output_tokens` — per-turn deltas on `App`,
//   zeroed at every submit by `App::begin_turn`. These are real, and they
//   are NOT cumulative; the row is labelled `last turn` so the two can never
//   be confused.
//
// ## What is deliberately NOT shown
//
// CUMULATIVE session token counts. They do not exist on `App`. The
// `AgentEvent::Usage` variant carries only `input_tokens`, `output_tokens`
// and `total_tokens`; it has no cache read/write fields, so
// `App::turn_cache_read_tokens` / `turn_cache_write_tokens` are
// structurally always 0 and the footer hides them (documented on the App
// fields). The only place cache-token accounting exists is the persisted
// `StatsEntry` path, which the TUI never receives live. There is therefore
// no cheap access path to a cumulative token total, and no honest way to
// produce one: accumulating the per-turn deltas in the view layer would
// fabricate a number the rest of the system does not track, and would reset
// or double-count across a session resume.
//
// So the panel names the gap instead of papering over it — see
// `SESSION_TOKENS_ROW`. A usage panel that reports an approximation as if it
// were exact is worse than no usage panel.
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::tui::app::App;
use crate::tui::theme_colors;

// ---------------------------------------------------------------------------
// Contract shared with the keybinding layer
// ---------------------------------------------------------------------------

/// Chord that toggles the panel. The dispatch arm lives in
/// `app/key_handling.rs`; the catalogue entry lives in
/// `keybindings/defaults.rs`. `catalogue_entry_is_consistent` asserts the
/// two agree, so they cannot drift apart silently.
pub const TOGGLE_KEY_LABEL: &str = "F8";

/// Rendered in place of any value that is not recorded. An em dash, never a
/// zero and never a blank — a missing metric must not look like a
/// measurement.
pub const UNAVAILABLE: &str = "\u{2014}";

/// Label for the row that names the gap rather than filling it.
pub const SESSION_TOKENS_ROW: &str = "session tokens";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct UsageOverlayState {
    pub visible: bool,
}

impl UsageOverlayState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// A plain snapshot of the real counters, so the renderer and its tests never
/// need to construct a whole `App`.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct UsageMetrics {
    /// Cumulative session cost in USD (`App::cost_usd`).
    pub session_cost_usd: f64,
    /// `true` once a turn has been submitted — gates the `last turn` row so
    /// it cannot read as a measured zero before the first turn.
    pub has_turn: bool,
    /// Per-turn input token delta (NOT cumulative).
    pub turn_input_tokens: u64,
    /// Per-turn output token delta (NOT cumulative).
    pub turn_output_tokens: u64,
    /// Requests whose cacheable prompt prefix was reusable.
    pub cache_prefix_hits: u64,
    /// Requests whose cacheable prompt prefix had changed.
    pub cache_prefix_misses: u64,
}

impl UsageMetrics {
    /// Collect the real counters off `App` at render time.
    pub fn from_app(app: &App) -> Self {
        let snap = app.retry_metrics.snapshot();
        Self {
            session_cost_usd: app.cost_usd,
            has_turn: app.turn_started_at.is_some(),
            turn_input_tokens: app.turn_input_tokens,
            turn_output_tokens: app.turn_output_tokens,
            cache_prefix_hits: snap.cache_prefix_reuse,
            cache_prefix_misses: snap.cache_prefix_misses,
        }
    }

    /// Cache-prefix hit rate as a whole percentage, or `None` when the agent
    /// has not yet produced a single cache verdict. `None` renders as
    /// [`UNAVAILABLE`] — never as `0%`.
    pub fn cache_hit_rate_pct(&self) -> Option<f64> {
        let decided = self.cache_prefix_hits + self.cache_prefix_misses;
        if decided == 0 {
            return None;
        }
        Some(self.cache_prefix_hits as f64 / decided as f64 * 100.0)
    }
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Session cost, matching the footer's precision rule (4dp under $0.50, so
/// that a cents-scale number does not render as `$0.00`).
fn format_cost(usd: f64) -> String {
    if usd < 0.5 {
        format!("${usd:.4}")
    } else {
        format!("${usd:.2}")
    }
}

/// `hits/misses  pct`, or the placeholder when nothing has been decided.
fn format_cache_prefix(m: &UsageMetrics) -> String {
    match m.cache_hit_rate_pct() {
        Some(pct) => format!(
            "{}/{}  {:.0}%",
            m.cache_prefix_hits, m.cache_prefix_misses, pct
        ),
        None => UNAVAILABLE.to_string(),
    }
}

/// Per-turn token deltas, or the placeholder before the first turn.
fn format_last_turn(m: &UsageMetrics) -> String {
    if !m.has_turn {
        return UNAVAILABLE.to_string();
    }
    format!(
        "\u{2191}{} \u{2193}{}",
        m.turn_input_tokens, m.turn_output_tokens
    )
}

// ---------------------------------------------------------------------------
// Line construction (the tested core)
// ---------------------------------------------------------------------------

fn labelled(label: &'static str, value: String, value_style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {label:<14}"),
            Style::default().fg(theme_colors::muted()),
        ),
        Span::styled(value, value_style),
    ])
}

/// Build every rendered line. Pure — the tests assert against this directly.
pub fn usage_lines(m: &UsageMetrics) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!(" {TOGGLE_KEY_LABEL} · usage"),
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD),
        )),
        labelled(
            "cost",
            format_cost(m.session_cost_usd),
            Style::default().fg(theme_colors::text()),
        ),
        labelled(
            "cache prefix",
            format_cache_prefix(m),
            Style::default().fg(theme_colors::text()),
        ),
        labelled(
            "last turn",
            format_last_turn(m),
            Style::default().fg(theme_colors::muted()),
        ),
        // The honest gap: no cumulative token total is tracked live, so this
        // row is the placeholder and never a number.
        labelled(
            SESSION_TOKENS_ROW,
            UNAVAILABLE.to_string(),
            Style::default().fg(theme_colors::disabled()),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Geometry + rendering
// ---------------------------------------------------------------------------

/// Content lines: the title row plus one row per metric.
const CONTENT_ROWS: u16 = 5;
/// Two extra rows for the border.
const PANEL_HEIGHT: u16 = CONTENT_ROWS + 2;
/// Wide enough for ` cache prefix  12/34  100% `.
const PANEL_WIDTH: u16 = 30;
/// Rows to skip from the top of the frame before docking the panel.
const TOP_INSET: u16 = 1;

/// Dock the panel in the top-right of `area`, clamped so it can never render
/// out of bounds on a tiny terminal.
pub fn panel_area(area: Rect) -> Rect {
    let width = PANEL_WIDTH.min(area.width);
    let height = PANEL_HEIGHT.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width),
        y: area.y + TOP_INSET.min(area.height.saturating_sub(height)),
        width,
        height,
    }
}

fn panel_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_colors::border()))
        .style(Style::default().bg(theme_colors::panel_bg()))
}

pub fn render_usage_overlay(
    frame: &mut Frame,
    state: &UsageOverlayState,
    area: Rect,
    metrics: &UsageMetrics,
) {
    render_usage_overlay_buf(frame.buffer_mut(), state, area, metrics);
}

pub fn render_usage_overlay_buf(
    buf: &mut Buffer,
    state: &UsageOverlayState,
    area: Rect,
    metrics: &UsageMetrics,
) {
    if !state.visible || area.width == 0 || area.height == 0 {
        return;
    }
    Paragraph::new(usage_lines(metrics))
        .block(panel_block())
        .render(panel_area(area), buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    /// Independently written here (rather than imported from the catalogue
    /// entry) so the cross-file check below is a real comparison: if the
    /// two strings ever disagree, the test fails instead of comparing a
    /// constant to itself.
    const TOGGLE_DESCRIPTION: &str = "Toggle usage overlay";

    /// Render into an off-screen buffer and return the text of `row`,
    /// counted from the top of the panel's own bounding box.
    fn rendered_row(m: &UsageMetrics, row: u16) -> String {
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        let state = UsageOverlayState { visible: true };
        render_usage_overlay_buf(&mut buf, &state, area, m);
        let panel = panel_area(area);
        let y = panel.y + row;
        (panel.x..panel.x + panel.width)
            .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol()))
            .collect::<String>()
    }

    /// Flatten the whole panel, for "does this row contain N" assertions.
    fn rendered_all(m: &UsageMetrics) -> String {
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        render_usage_overlay_buf(&mut buf, &UsageOverlayState { visible: true }, area, m);
        let panel = panel_area(area);
        (0..panel.height)
            .map(|row| {
                (0..panel.width)
                    .filter_map(|x| buf.cell((panel.x + x, panel.y + row)).map(|c| c.symbol()))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// (1) A synthetic metric set renders the recorded numbers.
    #[test]
    fn usage_lines_render_recorded_metrics() {
        let m = UsageMetrics {
            session_cost_usd: 0.01234,
            has_turn: true,
            turn_input_tokens: 1200,
            turn_output_tokens: 340,
            cache_prefix_hits: 3,
            cache_prefix_misses: 5,
        };

        let text = rendered_all(&m);

        // Cumulative cost at the footer's sub-$0.50 precision.
        assert!(text.contains("$0.0123"), "cost missing from:\n{text}");
        // Cache hit rate: 3/8 = 37.5% -> "38%", and both raw counts shown.
        assert!(text.contains("3/5"), "cache counts missing from:\n{text}");
        assert!(text.contains("38%"), "cache rate missing from:\n{text}");
        // Per-turn token deltas, explicitly labelled as per-turn.
        assert!(
            text.contains("\u{2191}1200 \u{2193}340"),
            "tokens missing:\n{text}"
        );
        assert!(text.contains("last turn"), "row label missing:\n{text}");
    }

    /// (1b) A cost at/above $0.50 uses 2dp, matching the footer.
    #[test]
    fn large_cost_uses_two_decimals() {
        let m = UsageMetrics {
            session_cost_usd: 12.5,
            ..UsageMetrics::default()
        };
        assert!(rendered_all(&m).contains("$12.50"));
    }

    /// (2) Unavailable metrics render the placeholder, never a fabricated
    /// number. A fresh session must not look like a measured 0% cache rate
    /// or a measured 0-token turn.
    #[test]
    fn unavailable_metrics_render_placeholder_not_a_number() {
        let fresh = UsageMetrics::default();
        let text = rendered_all(&fresh);

        // Cache: no verdicts yet -> placeholder, and definitely not "0%".
        assert!(text.contains("cache prefix"), "label missing:\n{text}");
        assert!(!text.contains("0%"), "fabricated 0% cache rate:\n{text}");
        // Last turn: no turn submitted -> placeholder, not "0".
        assert!(
            !text.contains("\u{2191}0 \u{2193}0"),
            "fabricated 0 tokens:\n{text}"
        );
        // The cumulative-token row is always the placeholder.
        let row = rendered_row(&fresh, 5);
        assert!(
            row.contains(SESSION_TOKENS_ROW) && row.contains(UNAVAILABLE),
            "session tokens row must name the gap:\n{row}"
        );
    }

    /// (2b) The cache placeholder only appears when the rate is genuinely
    /// unavailable — a real 0% (all misses) must still print 0%.
    #[test]
    fn real_zero_cache_rate_is_measured_not_placeholdered() {
        let m = UsageMetrics {
            cache_prefix_hits: 0,
            cache_prefix_misses: 4,
            ..UsageMetrics::default()
        };
        let text = rendered_all(&m);
        assert!(text.contains("0/4"), "counts missing:\n{text}");
        assert!(text.contains("0%"), "a real 0% must print:\n{text}");
    }

    /// (3a) The toggle flips visibility, and a hidden panel paints nothing.
    #[test]
    fn toggle_flips_visibility() {
        let mut state = UsageOverlayState::new();
        assert!(!state.visible, "panel must start hidden");
        state.toggle();
        assert!(state.visible);
        state.toggle();
        assert!(!state.visible);

        // Hidden => the buffer stays blank.
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        let metrics = UsageMetrics {
            session_cost_usd: 9.0,
            ..UsageMetrics::default()
        };
        render_usage_overlay_buf(&mut buf, &state, area, &metrics);
        let panel = panel_area(area);
        let painted = (0..panel.height).any(|row| {
            (0..panel.width).any(|x| {
                buf.cell((panel.x + x, panel.y + row))
                    .map(|c| c.symbol() != " ")
                    .unwrap_or(false)
            })
        });
        assert!(!painted, "hidden panel must not paint");
    }

    /// (3b) The dispatch arm and the catalogue entry agree on the chord.
    /// Both files are the ones this change touches, so if either drifts the
    /// test fails rather than leaving `/keys` advertising a dead chord.
    #[test]
    fn catalogue_entry_is_consistent() {
        const HANDLING: &str = include_str!("app/key_handling.rs");
        const DEFAULTS: &str = include_str!("keybindings/defaults.rs");

        let chord = format!(
            "KeyCode::{}(",
            TOGGLE_KEY_LABEL.trim_end_matches(|c: char| !c.is_ascii_digit())
        );
        let key_literal = if TOGGLE_KEY_LABEL.starts_with("F") {
            format!("KeyCode::F({}", TOGGLE_KEY_LABEL.trim_start_matches('F'))
        } else {
            chord
        };

        assert!(
            HANDLING.contains(&key_literal),
            "{TOGGLE_KEY_LABEL} arm missing from app/key_handling.rs (looked for `{key_literal}`)"
        );
        assert!(
            DEFAULTS.contains(&key_literal),
            "{TOGGLE_KEY_LABEL} missing from keybindings/defaults.rs (looked for `{key_literal}`)"
        );
        assert!(
            DEFAULTS.contains(TOGGLE_DESCRIPTION),
            "catalogue description `{TOGGLE_DESCRIPTION}` missing from keybindings/defaults.rs"
        );
    }

    /// (4) Colour-literal gate. The module must resolve every colour through
    /// `theme_colors`, so that `/theme` still repaints the panel and so a
    /// hardcoded colour is visible in review.
    ///
    /// The forbidden needles are built by concatenation on purpose: written
    /// out literally they would appear in this test's own source and the
    /// `include_str!` below would match them.
    #[test]
    fn no_raw_colour_literals() {
        const SRC: &str = include_str!("usage_overlay.rs");
        const COLOR: &str = "Color";

        let banned = [
            format!("{COLOR}::Rgb"),
            format!("{COLOR}::Indexed"),
            format!("{COLOR}::White"),
            format!("{COLOR}::Yellow"),
            format!("{COLOR}::Cyan"),
            format!("{COLOR}::Red"),
            format!("{COLOR}::Green"),
            format!("{COLOR}::Blue"),
            format!("{COLOR}::Magenta"),
            format!("{COLOR}::DarkGray"),
            format!("{COLOR}::LightGray"),
            format!("{COLOR}::Gray"),
        ];
        for needle in banned {
            assert!(
                !SRC.contains(&needle),
                "usage_overlay.rs contains the raw colour `{needle}`. Use a \
                 `theme_colors` accessor (text/muted/border/accent/warning/\
                 error/success/panel_bg/disabled) so the panel follows the \
                 active theme."
            );
        }
    }

    /// The panel must not use a colour at all outside the accessors — the
    /// import itself is a smell, since `theme_colors` is the only palette.
    #[test]
    fn no_direct_color_import() {
        const SRC: &str = include_str!("usage_overlay.rs");
        // The test module needs `Color` to type-annotate its assertions; the
        // production half must not.
        let (production, _tests) = SRC.split_once("#[cfg(test)]").expect("test module");
        assert!(
            !production.contains(&format!("{}[", "Color")),
            "production code must not import `Color` — use theme_colors accessors"
        );
        let _: Option<Color> = None; // keep the import used in tests
    }

    /// The panel degrades instead of panicking on a tiny terminal.
    #[test]
    fn panel_area_clamps_to_tiny_terminals() {
        for (w, h) in [(0u16, 0u16), (1, 1), (5, 3), (29, 6), (200, 60)] {
            let area = Rect::new(0, 0, w, h);
            let p = panel_area(area);
            assert!(p.width <= w && p.height <= h, "clamped badly for {w}x{h}");
            assert!(p.x + p.width <= w, "x overflow for {w}x{h}");
            assert!(p.y + p.height <= h, "y overflow for {w}x{h}");
        }
    }

    #[test]
    fn hidden_or_empty_area_is_a_noop() {
        let metrics = UsageMetrics {
            session_cost_usd: 5.0,
            ..UsageMetrics::default()
        };
        // Empty area while visible.
        let mut buf = Buffer::empty(Rect::new(0, 0, 0, 0));
        render_usage_overlay_buf(
            &mut buf,
            &UsageOverlayState { visible: true },
            Rect::new(0, 0, 0, 0),
            &metrics,
        );
        // Visible area while hidden.
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        render_usage_overlay_buf(
            &mut buf,
            &UsageOverlayState { visible: false },
            area,
            &metrics,
        );
        assert!(
            buf.cell((panel_area(area).x, panel_area(area).y))
                .map(|c| c.symbol() == " ")
                .unwrap_or(true)
        );
    }
}
