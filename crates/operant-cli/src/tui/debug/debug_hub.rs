//! TuiDebugHub — centralized debug state.
//!
//! Holds the event bus plus aggregate debug counters. Published to from the
//! run loop and event handlers; read from the F12 debug overlay.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use parking_lot::Mutex;

use super::event_bus::{TuiEvent, TuiEventBus, now_secs};

/// Centralized debug state for the TUI. Cheap to clone (inner is Arc).
/// All fields are thread-safe (AtomicBool/AtomicU64/Mutex).
#[derive(Clone)]
pub struct TuiDebugHub {
    inner: Arc<Inner>,
}

struct Inner {
    event_bus: TuiEventBus,
    started_at: Instant,
    frame_count: AtomicU64,
    last_render_ms: AtomicU64,
    last_error: Mutex<Option<String>>,
    overlay_visible: AtomicBool,
    /// Path to dump the event log on exit (if set via env var).
    event_log_path: Mutex<Option<std::path::PathBuf>>,
    /// Per-frame text capture, armed by the headless simulator only.
    frame_capture: Mutex<Option<FrameCapture>>,
}

/// Requested per-frame captures for one headless run. Disarmed (and free)
/// unless `arm_frame_capture` is called.
#[derive(Debug, Clone)]
pub struct FrameCapture {
    requested: std::collections::BTreeSet<u64>,
    dir: std::path::PathBuf,
    captured: std::collections::BTreeSet<u64>,
}

impl TuiDebugHub {
    /// Create a new hub. `enabled` controls whether the event bus records.
    pub fn new(enabled: bool) -> Self {
        let event_log_path = std::env::var("OPERANT_TUI_EVENT_LOG")
            .ok()
            .map(std::path::PathBuf::from);

        Self {
            inner: Arc::new(Inner {
                event_bus: TuiEventBus::new(enabled),
                started_at: Instant::now(),
                frame_count: AtomicU64::new(0),
                last_render_ms: AtomicU64::new(0),
                last_error: Mutex::new(None),
                overlay_visible: AtomicBool::new(false),
                event_log_path: Mutex::new(event_log_path),
                frame_capture: Mutex::new(None),
            }),
        }
    }

    /// Create from env var: enabled if `OPERANT_TUI_DEBUG=1`.
    pub fn new_from_env() -> Self {
        let enabled = std::env::var("OPERANT_TUI_DEBUG")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);
        Self::new(enabled)
    }

    // ── Event bus access ─────────────────────────────────────────────

    pub fn event_bus(&self) -> &TuiEventBus {
        &self.inner.event_bus
    }

    pub fn publish(&self, event: TuiEvent) {
        self.inner.event_bus.publish(event);
    }

    // ── Frame tracking ───────────────────────────────────────────────

    /// Called from the run loop after each `terminal.draw`. Records frame
    /// count, render time, and publishes a FrameRendered event.
    pub fn record_frame(&self, render_ms: f64) {
        let frame = self.inner.frame_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.inner
            .last_render_ms
            .store(render_ms as u64, Ordering::Relaxed);
        self.inner.event_bus.publish(TuiEvent::FrameRendered {
            frame,
            render_ms,
            at: now_secs(),
        });
    }

    pub fn frame_count(&self) -> u64 {
        self.inner.frame_count.load(Ordering::Relaxed)
    }

    pub fn last_render_ms(&self) -> u64 {
        self.inner.last_render_ms.load(Ordering::Relaxed)
    }

    // ── Per-frame capture (headless simulator) ─────────────────────────

    /// Arm per-frame text capture for the given frame indices. Indices are
    /// 0-based over *painted* frames (`0` = the first frame the run loop
    /// draws), matching `TuiEvent::FrameRendered::frame - 1`. `dir` must
    /// already exist; each capture is written to `<dir>/frame-<NNNN>.txt`
    /// using the same index, 4-digit zero padded so a lexical sort equals a
    /// numeric sort. A second arm on the same hub is a no-op.
    pub fn arm_frame_capture(&self, frames: Vec<u64>, dir: std::path::PathBuf) {
        let mut slot = self.inner.frame_capture.lock();
        if slot.is_some() {
            return;
        }
        *slot = Some(FrameCapture {
            requested: frames.into_iter().collect(),
            dir,
            captured: std::collections::BTreeSet::new(),
        });
    }

    /// Called from the run loop with the *just-rendered* buffer, immediately
    /// after `record_frame`. Writes the buffer as trimmed text rows when this
    /// frame was requested, and does nothing otherwise.
    ///
    /// This is the only place mid-run frames can be captured: `App::run`
    /// returns before its final state is ever painted, so a finished buffer
    /// cannot be rewound. Capturing here (like the OSC 8 scan beside it) keeps
    /// the production loop untouched — the call is inert unless armed.
    pub fn capture_frame(&self, buffer: &ratatui::buffer::Buffer) {
        let mut slot = self.inner.frame_capture.lock();
        let Some(capture) = slot.as_mut() else {
            return;
        };
        // `record_frame` already bumped the counter, so this is the 1-based
        // number of the frame just painted.
        let painted = self.inner.frame_count.load(Ordering::Relaxed);
        let index = painted.saturating_sub(1);
        if !capture.requested.contains(&index) {
            return;
        }
        let path = capture.dir.join(format!("frame-{index:04}.txt"));
        let rows = buffer_rows(buffer);
        if let Err(e) = std::fs::write(&path, rows.join("\n") + "\n") {
            eprintln!("[tui-debug] frame capture {index} -> {path:?} failed: {e}");
            return;
        }
        capture.captured.insert(index);
    }

    /// `(requested, captured)` 0-based frame indices for the armed capture, or
    /// `None` when capture was never armed. A requested index missing from
    /// `captured` means the run ended before that frame was painted — the
    /// caller must report it rather than silently shipping fewer files than
    /// asked for.
    pub fn frame_capture_status(&self) -> Option<(Vec<u64>, Vec<u64>)> {
        let slot = self.inner.frame_capture.lock();
        slot.as_ref().map(|c| {
            (
                c.requested.iter().copied().collect(),
                c.captured.iter().copied().collect(),
            )
        })
    }

    pub fn uptime_secs(&self) -> f64 {
        self.inner.started_at.elapsed().as_secs_f64()
    }

    // ── Error tracking ───────────────────────────────────────────────

    pub fn record_error(&self, source: &str, message: &str) {
        let formatted = format!("[{source}] {message}");
        *self.inner.last_error.lock() = Some(formatted.clone());
        self.inner.event_bus.publish(TuiEvent::Error {
            source: source.to_string(),
            message: message.to_string(),
            at: now_secs(),
        });
    }

    pub fn last_error(&self) -> Option<String> {
        self.inner.last_error.lock().clone()
    }

    // ── Overlay toggle ───────────────────────────────────────────────

    pub fn toggle_overlay(&self) {
        let was = self.inner.overlay_visible.load(Ordering::Relaxed);
        self.inner.overlay_visible.store(!was, Ordering::Relaxed);
    }

    pub fn overlay_visible(&self) -> bool {
        self.inner.overlay_visible.load(Ordering::Relaxed)
    }

    // ── Exit dump ────────────────────────────────────────────────────

    /// Dump the event log to the path set by OPERANT_TUI_EVENT_LOG, if any.
    /// Call this on clean TUI exit.
    pub fn dump_on_exit(&self) {
        let path = self.inner.event_log_path.lock().clone();
        if let Some(path) = path {
            if let Err(e) = self.inner.event_bus.dump_to_file(&path) {
                eprintln!("[tui-debug] failed to dump event log to {path:?}: {e}");
            } else {
                eprintln!("[tui-debug] event log dumped to {path:?}");
            }
        }
    }
}

/// Project a rendered buffer into the `operant-style-v1` grid: a header line,
/// then exactly `height` lines of exactly `width` space-separated
/// `<fg>/<bg>/<mods>@<symbol>` tokens, newline-terminated.
///
/// This is a *verbatim* projection. Nothing is normalised, reordered, trimmed
/// or collapsed — the text dump cannot see a palette, and post-processing the
/// style dump would hide exactly the regressions it exists to catch. The only
/// transformation is the per-cell token encoding below, which exists to keep
/// one cell on one token.
///
/// Encoding rules (see `crates/operant-cli/tests/tui_scenarios/schema.json` →
/// `baseline_formats.style`, which is authoritative):
///   * colour: `reset|black|red|green|yellow|blue|magenta|cyan|gray|darkgray|
///     lightred|lightgreen|lightyellow|lightblue|lightmagenta|lightcyan|white|
///     rgb:RRGGBB|indexed:N`. `Color::Reset` → `reset`.
///   * mods: alphabetically sorted, `+`-joined subset of
///     `b d i r u x`; `-` when none.
///   * symbol: space → `_`; empty or NUL → `~`; backslash → `\\`; any other
///     control char → `?`; every other printable char literal.
///   * a symbol wider than one column (wide CJK/emoji) is written once, in the
///     cell where it STARTS; its continuation cells become
///     `~/<same fg>/<same bg>/-`.
///
/// Note: ratatui's `Cell` has no colour value distinct from `Color::Reset`
/// ("no colour set" *is* `Color::Reset`), so the schema's unset-colour `-`
/// placeholder is unreachable here and `reset` is emitted instead. The
/// width-carry, not the cell's own contents, is what identifies a
/// continuation cell — `Buffer::set_stringn` resets those cells to
/// `Cell::EMPTY`, which reads back as an ordinary space.
pub fn buffer_style_dump(buffer: &ratatui::buffer::Buffer) -> String {
    let mut out = format!(
        "operant-style-v1 {}x{}\n",
        buffer.area.width, buffer.area.height
    );
    for row in buffer_style_rows(buffer) {
        out.push_str(&row);
        out.push('\n');
    }
    out
}

/// The grid lines of [`buffer_style_dump`] without the header. One `String`
/// per terminal row, each holding exactly `width` space-separated tokens.
pub fn buffer_style_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;

    let width = (buffer.area.width as usize).max(1);
    let height = buffer.area.height as usize;
    let content = buffer.content();
    let mut rows = Vec::with_capacity(height);
    for y in 0..height {
        let mut tokens: Vec<String> = Vec::with_capacity(width);
        // Columns still occupied by a wide symbol's tail, and the colours of
        // the cell that symbol started in.
        let mut carry: Option<(ratatui::style::Color, ratatui::style::Color)> = None;
        let mut carry_left = 0usize;
        for x in 0..width {
            let Some(cell) = content.get(y * width + x) else {
                break;
            };
            if carry_left > 0 {
                if let Some((fg, bg)) = carry {
                    // `~` in the fg slot, the START cell's colours, and `-`
                    // in the mods slot, with an empty symbol field: the
                    // schema writes this cell as `~/<fg>/<bg>/-`, eliding the
                    // `@<symbol>` tail the grammar otherwise requires.
                    tokens.push(format!(
                        "~/{}/{}/-@",
                        style_color_field(fg),
                        style_color_field(bg)
                    ));
                }
                carry_left -= 1;
                continue;
            }
            let symbol = cell.symbol();
            let columns = UnicodeWidthStr::width(symbol);
            if columns > 1 {
                carry = Some((cell.fg, cell.bg));
                carry_left = columns - 1;
            }
            tokens.push(style_token(cell));
        }
        rows.push(tokens.join(" "));
    }
    rows
}

/// Encode one cell as `<fg>/<bg>/<mods>@<symbol>`.
pub fn style_token(cell: &ratatui::buffer::Cell) -> String {
    format!(
        "{}/{}/{}@{}",
        style_color_field(cell.fg),
        style_color_field(cell.bg),
        style_mods_field(cell.modifier),
        style_symbol_field(cell.symbol()),
    )
}

/// The `fg`/`bg` field. Same vocabulary in both positions; `Color::Reset` is
/// the buffer's default and encodes as `reset`.
fn style_color_field(color: ratatui::style::Color) -> String {
    use ratatui::style::Color;
    match color {
        Color::Reset => "reset".to_string(),
        Color::Black => "black".to_string(),
        Color::Red => "red".to_string(),
        Color::Green => "green".to_string(),
        Color::Yellow => "yellow".to_string(),
        Color::Blue => "blue".to_string(),
        Color::Magenta => "magenta".to_string(),
        Color::Cyan => "cyan".to_string(),
        Color::Gray => "gray".to_string(),
        Color::DarkGray => "darkgray".to_string(),
        Color::LightRed => "lightred".to_string(),
        Color::LightGreen => "lightgreen".to_string(),
        Color::LightYellow => "lightyellow".to_string(),
        Color::LightBlue => "lightblue".to_string(),
        Color::LightMagenta => "lightmagenta".to_string(),
        Color::LightCyan => "lightcyan".to_string(),
        Color::White => "white".to_string(),
        Color::Rgb(r, g, b) => format!("rgb:{r:02X}{g:02X}{b:02X}"),
        Color::Indexed(n) => format!("indexed:{n}"),
    }
}

/// The `mods` field: alphabetically sorted `+`-joined subset of
/// `b d i r u x`, or `-` when no modifier is set. `SLOW_BLINK`, `RAPID_BLINK`
/// and `HIDDEN` have no letter in the schema's vocabulary and are not emitted;
/// no surface in the tree sets them today, so no rendered style is lost.
fn style_mods_field(modifier: ratatui::style::Modifier) -> String {
    use ratatui::style::Modifier;
    let mut out = String::new();
    // Declaration order below is alphabetical by letter: b, d, i, r, u, x.
    for (letter, flag) in [
        ('b', Modifier::BOLD),
        ('d', Modifier::DIM),
        ('i', Modifier::ITALIC),
        ('r', Modifier::REVERSED),
        ('u', Modifier::UNDERLINED),
        ('x', Modifier::CROSSED_OUT),
    ] {
        if !modifier.contains(flag) {
            continue;
        }
        if !out.is_empty() {
            out.push('+');
        }
        out.push(letter);
    }
    if out.is_empty() {
        out.push('-');
    }
    out
}

/// The `symbol` field, kept to one token per cell by substituting the
/// characters that would otherwise break token splitting or be invisible.
fn style_symbol_field(symbol: &str) -> String {
    if symbol.is_empty() {
        return "~".to_string();
    }
    let mut out = String::with_capacity(symbol.len());
    for ch in symbol.chars() {
        match ch {
            '\0' => out.push('~'),
            ' ' => out.push('_'),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push('?'),
            c => out.push(c),
        }
    }
    out
}

/// Split a ratatui buffer into trimmed text rows, one per terminal line.
/// Shared by the final-frame capture in `run_headless` and the mid-run
/// per-frame capture so both produce byte-identical text.
pub fn buffer_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    let width = (buffer.area.width as usize).max(1);
    buffer
        .content()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_records_frames() {
        let hub = TuiDebugHub::new(true);
        assert_eq!(hub.frame_count(), 0);
        hub.record_frame(5.0);
        hub.record_frame(3.0);
        assert_eq!(hub.frame_count(), 2);
        assert_eq!(hub.last_render_ms(), 3);
    }

    #[test]
    fn hub_records_errors() {
        let hub = TuiDebugHub::new(true);
        assert!(hub.last_error().is_none());
        hub.record_error("test", "something broke");
        assert_eq!(hub.last_error().unwrap(), "[test] something broke");
    }

    #[test]
    fn overlay_toggle() {
        let hub = TuiDebugHub::new(false);
        assert!(!hub.overlay_visible());
        hub.toggle_overlay();
        assert!(hub.overlay_visible());
        hub.toggle_overlay();
        assert!(!hub.overlay_visible());
    }

    #[test]
    fn new_from_env_respects_flag() {
        // Default: not set → disabled.
        let hub = TuiDebugHub::new_from_env();
        hub.record_frame(1.0);
        // Bus is disabled, so no events recorded.
        assert_eq!(hub.event_bus().len(), 0);
    }

    // ── Style dump encoder ────────────────────────────────────────────

    /// One cell with the given symbol, for encoder tests.
    fn cell(symbol: &str) -> ratatui::buffer::Cell {
        let mut c = ratatui::buffer::Cell::default();
        c.set_symbol(symbol);
        c
    }

    #[test]
    fn style_color_vocabulary_is_complete_in_both_fields() {
        use ratatui::style::Color;
        // Every member of the schema's fg/bg vocabulary, in both positions.
        let cases: Vec<(Color, &str)> = vec![
            (Color::Reset, "reset"),
            (Color::Black, "black"),
            (Color::Red, "red"),
            (Color::Green, "green"),
            (Color::Yellow, "yellow"),
            (Color::Blue, "blue"),
            (Color::Magenta, "magenta"),
            (Color::Cyan, "cyan"),
            (Color::Gray, "gray"),
            (Color::DarkGray, "darkgray"),
            (Color::LightRed, "lightred"),
            (Color::LightGreen, "lightgreen"),
            (Color::LightYellow, "lightyellow"),
            (Color::LightBlue, "lightblue"),
            (Color::LightMagenta, "lightmagenta"),
            (Color::LightCyan, "lightcyan"),
            (Color::White, "white"),
            (Color::Rgb(0x0A, 0x1B, 0xFF), "rgb:0A1BFF"),
            (Color::Rgb(0, 0, 0), "rgb:000000"),
            (Color::Indexed(42), "indexed:42"),
            (Color::Indexed(255), "indexed:255"),
        ];
        for (color, name) in cases {
            let mut fg = cell("a");
            fg.fg = color;
            assert_eq!(style_token(&fg), format!("{name}/reset/-@a"), "fg {name}");
            let mut bg = cell("a");
            bg.bg = color;
            assert_eq!(style_token(&bg), format!("reset/{name}/-@a"), "bg {name}");
        }
    }

    #[test]
    fn style_rgb_is_uppercase_hex_and_zero_padded() {
        assert_eq!(
            style_color_field_pub(ratatui::style::Color::Rgb(1, 2, 3)),
            "rgb:010203"
        );
        assert_eq!(
            style_color_field_pub(ratatui::style::Color::Rgb(255, 254, 253)),
            "rgb:FFFEFD"
        );
    }

    /// `style_color_field` is private; exercise it through the public token
    /// encoder with a colour nobody else sets.
    fn style_color_field_pub(color: ratatui::style::Color) -> String {
        let mut c = cell("x");
        c.fg = color;
        style_token(&c)
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string()
    }

    #[test]
    fn style_mods_are_alphabetical_plus_joined_or_dash() {
        use ratatui::style::Modifier;
        let cases: Vec<(Modifier, &str)> = vec![
            (Modifier::empty(), "-"),
            (Modifier::BOLD, "b"),
            (Modifier::DIM, "d"),
            (Modifier::ITALIC, "i"),
            (Modifier::REVERSED, "r"),
            (Modifier::UNDERLINED, "u"),
            (Modifier::CROSSED_OUT, "x"),
            // Declaration order in the encoder is not the sorted order; the
            // output must be sorted regardless of how it was assembled.
            (Modifier::CROSSED_OUT | Modifier::BOLD, "b+x"),
            (Modifier::UNDERLINED | Modifier::REVERSED, "r+u"),
            (Modifier::ITALIC | Modifier::DIM, "d+i"),
            (
                Modifier::BOLD
                    | Modifier::DIM
                    | Modifier::ITALIC
                    | Modifier::REVERSED
                    | Modifier::UNDERLINED
                    | Modifier::CROSSED_OUT,
                "b+d+i+r+u+x",
            ),
        ];
        for (modifier, expected) in cases {
            let mut c = cell("a");
            c.modifier = modifier;
            assert_eq!(
                style_token(&c),
                format!("reset/reset/{expected}@a"),
                "mods {expected}"
            );
        }
    }

    #[test]
    fn style_symbol_substitutions_keep_one_token_per_cell() {
        // The five substitution classes the schema names, one per case.
        assert_eq!(style_token(&cell(" ")), "reset/reset/-@_");
        assert_eq!(style_token(&cell("\\")), "reset/reset/-@\\\\");
        assert_eq!(style_token(&cell("\u{7}")), "reset/reset/-@?");
        assert_eq!(style_token(&cell("\u{1b}[0m")), "reset/reset/-@?[0m");
        assert_eq!(style_token(&cell("")), "reset/reset/-@~");
        assert_eq!(style_token(&cell("\0")), "reset/reset/-@~");
        // Printable characters, including non-ASCII, are literal.
        assert_eq!(style_token(&cell("A")), "reset/reset/-@A");
        assert_eq!(style_token(&cell("%s")), "reset/reset/-@%s");
        assert_eq!(style_token(&cell("漢")), "reset/reset/-@漢");
        assert_eq!(style_token(&cell("█")), "reset/reset/-@█");
        // A backslash inside a multi-char symbol is doubled in place, so the
        // symbol field stays a single token.
        assert_eq!(style_token(&cell("a\\b")), "reset/reset/-@a\\\\b");
    }

    #[test]
    fn style_row_writes_wide_symbol_once_then_continuation_cells() {
        // 漢 and 😀 are each two columns wide: the symbol is written in the
        // cell where it starts, and the cells it covers become `~` carrying
        // the START cell's colours with mods forced to `-`.
        let area = ratatui::layout::Rect::new(0, 0, 6, 1);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        let mut wide = ratatui::buffer::Cell::default();
        wide.set_symbol("漢");
        wide.fg = ratatui::style::Color::LightCyan;
        wide.bg = ratatui::style::Color::Blue;
        wide.modifier = ratatui::style::Modifier::BOLD;
        buf[(0, 0)] = wide;
        buf[(2, 0)].set_symbol("😀");
        buf[(4, 0)].set_symbol("z");
        assert_eq!(
            buffer_style_rows(&buf),
            vec![
                "lightcyan/blue/b@漢 ~/lightcyan/blue/-@ \
                 reset/reset/-@😀 ~/reset/reset/-@ \
                 reset/reset/-@z reset/reset/-@_"
            ]
        );
    }

    #[test]
    fn style_wide_carry_follows_the_real_set_stringn_path() {
        // The rows above are hand-built. This one goes through the same
        // `Buffer::set_stringn` the widgets use, where the continuation cell
        // is `Cell::EMPTY` (a space) — proving the width-carry, not the
        // continuation cell's own contents, is what marks a tail.
        let area = ratatui::layout::Rect::new(0, 0, 4, 1);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        buf.set_stringn(0, 0, "漢a", 4, ratatui::style::Style::default());
        assert_eq!(
            buffer_style_rows(&buf),
            vec!["reset/reset/-@漢 ~/reset/reset/-@ reset/reset/-@a reset/reset/-@_"]
        );
    }

    #[test]
    fn style_dump_has_header_and_exact_grid_shape() {
        let area = ratatui::layout::Rect::new(0, 0, 3, 2);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        buf[(0, 0)]
            .set_symbol("a")
            .set_style(ratatui::style::Style::default());
        buf[(1, 0)].set_symbol("b");
        let dump = buffer_style_dump(&buf);
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(
            lines,
            vec![
                "operant-style-v1 3x2",
                "reset/reset/-@a reset/reset/-@b reset/reset/-@_",
                "reset/reset/-@_ reset/reset/-@_ reset/reset/-@_",
            ]
        );
        // One trailing newline, and header + height lines exactly.
        assert!(dump.ends_with('\n'));
        assert!(!dump.ends_with("\n\n"));
        assert_eq!(lines.len(), 3);
        for row in &lines[1..] {
            assert_eq!(row.split(' ').count(), 3, "row width: {row}");
        }
    }

    #[test]
    fn style_rows_are_verbatim_under_repeated_styles() {
        // No collapsing of runs: four identical cells stay four tokens.
        let area = ratatui::layout::Rect::new(0, 0, 4, 1);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        for x in 0..4 {
            buf[(x, 0)].set_symbol("#");
        }
        assert_eq!(
            buffer_style_rows(&buf),
            vec!["reset/reset/-@# reset/reset/-@# reset/reset/-@# reset/reset/-@#"]
        );
    }
}
