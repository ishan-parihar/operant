// pinned_images.rs — a persistent strip of terminal graphics that survives a redraw.
//
// # The defect this exists to fix
//
// Kitty/Sixel/iTerm2 paint OUTSIDE ratatui's cell grid. Nothing in the cell
// buffer represents a painted image, so the only record of one is the escape
// sequence — and the old post-paint pass wrote it exactly once
// (`pending_inline_images` was drained, a mermaid raster was consumed) and then
// forgot it. `render_app` runs inside the draw closure, so ratatui's own flush
// happens *after* that write: the flush overwrote the image as soon as it wrote
// any cell underneath it, and there was no second copy to restore from. A pasted
// screenshot survived at most one frame, and a mermaid diagram the same.
//
// # The fix
//
// A registry of placed graphics on `App`, behind a `RefCell` because the render
// pass holds `&App` and still has to write every frame. `paint_strip` is the
// per-frame pass: it lays the graphics out at a fixed rect, blanks the cells
// they occupy, and re-emits each sequence. Re-emitting every frame is the whole
// property — the queue is still drained once, because the *attachment* is
// consumed, but the *graphic* it produced is not.
//
// # Pinning is a cursor move, not a re-encode
//
// `image_render` builds the Kitty sequence as
// `a=T,f=100,q=1,c=…,r=…,m=1;<base64>` with no absolute coordinates, and `a=T`
// means "transmit and display": the terminal draws the image at the CURRENT
// cursor. So `placement_prefix` emits a CUP (`ESC[<row>;<col>H`, 1-based) and
// the payload goes out byte-for-byte unchanged. Nothing here re-encodes an image
// or needs a second renderer knob.
//
// # Where the strip goes
//
// Top of the messages area, spanning its width, immediately BELOW the band the
// usage overlay reserves (it docks top-right, see `usage_overlay::panel_area`)
// and ABOVE the async-delegation task rows (they dock bottom-left and grow
// upward, see `background_tasks::rows_area`). The band is reserved whether or
// not the panel is currently visible: the re-emit paints over whatever is on
// the physical screen, and F8 can be toggled at any moment. Both of those
// rects are derived from the real functions rather than hardcoded widths, so
// they cannot drift apart silently.
//
// Two ceilings, stated rather than hidden:
//
//   * `background_tasks` rows can in principle grow to the full height of the
//     messages area, in which case no in-area strip is safe. `strip_area` gives
//     a *precise* guarantee instead — the strip never overlaps the rows while
//     `row_count <= area.bottom - strip.y - strip.height - 1` — which
//     `strip_area_never_overlaps_the_usage_overlay_or_task_rows` asserts for
//     every terminal size. The upgrade path, if a session ever runs enough
//     concurrent delegations to eat that band, is to pass the live row count in
//     and subtract it here.
//   * A graphic wider than the strip still paints at its natural width (the
//     payload fixes the size, not the rect), so on a narrow terminal it can
//     reach past the strip's right edge. It is still inside the messages area,
//     and the reserved usage band is above it, so nothing outside the strip is
//     corrupted — but it can cover transcript text it was not sized for.
//
// The strip *claims* its cells: `paint_strip` blanks them, because a cell the
// ratatui diff keeps rewriting is a cell the image can never survive in. The
// transcript is still scrollable, so the covered lines are not lost — a pinned
// pane covers what is behind it, same as jcode.

use std::cell::RefCell;
use std::io::{self, Write};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::tui::image_paste::{PastedImage, render_attachment};
use crate::tui::image_render::{self, ImageRenderConfig, RenderedImage};
use crate::tui::mermaid::LandedRasters;
use crate::tui::usage_overlay;

/// DECSC — save the cursor position.
const CURSOR_SAVE: &str = "\u{1b}7";
/// DECRP — restore the cursor position saved by [`CURSOR_SAVE`].
const CURSOR_RESTORE: &str = "\u{1b}8";

/// Graphics retained at once.
///
/// A handful, because every frame re-emits every retained escape sequence: a
/// Kitty payload is the base64 of the whole file, so the cap is what keeps the
/// per-frame write bounded. Eviction is OLDEST-OUT on insert — the newest
/// attachment is the one the user just made and the one they are looking at,
/// and a transcript that pins twenty screenshots helps nobody. A terminal with
/// no graphics protocol never gets here, so the cap costs a real user nothing.
pub const MAX_GRAPHICS: usize = 4;

/// Rows a single graphic may claim.
///
/// The renderer already caps a graphic at `ImageRenderConfig::max_height_cells`
/// (40 by default), which is more than a short terminal can give. Clamping per
/// entry keeps one tall diagram from consuming the whole strip.
pub const MAX_ENTRY_ROWS: u16 = 12;

/// One placed graphic: the sequence to re-emit, and the cells it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct PinnedGraphic {
    /// The terminal graphics escape sequence, byte-for-byte as the renderer
    /// produced it.
    pub escape_sequence: String,
    /// Cell width the terminal will paint.
    pub width_cells: u16,
    /// Cell height the terminal will paint, clamped to [`MAX_ENTRY_ROWS`].
    pub height_cells: u16,
    /// The cell rect this graphic occupied at the last [`PinnedImageRegistry::layout`].
    /// A zero width means "did not fit this frame": the entry stays registered
    /// and paints as soon as the terminal is tall enough again, and is skipped
    /// in the meantime.
    pub rect: Rect,
}

/// The session's pinned graphics, oldest first.
///
/// Behind a [`RefCell`] for the same reason `BackgroundTaskRegistry` is: the
/// render pass holds `&App` and still has to add to and re-read the registry
/// every frame.
#[derive(Debug, Clone, Default)]
pub struct PinnedImageRegistry {
    graphics: RefCell<Vec<PinnedGraphic>>,
}

impl PinnedImageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin a rendered graphic.
    ///
    /// Returns `false` and retains nothing when the render produced no graphic:
    /// a terminal with no graphics protocol yields a *textual placeholder*, and
    /// pinning a string as though it were a picture would blank transcript
    /// cells and re-write the same text every frame for no reason.
    pub fn add(&self, rendered: &RenderedImage) -> bool {
        if !rendered.success || rendered.escape_sequence.is_empty() {
            return false;
        }
        let mut graphics = self.graphics.borrow_mut();
        if graphics.len() == MAX_GRAPHICS {
            // Oldest out. See MAX_GRAPHICS for why.
            graphics.remove(0);
        }
        graphics.push(PinnedGraphic {
            escape_sequence: rendered.escape_sequence.clone(),
            width_cells: rendered.width_cells.max(1),
            height_cells: rendered.height_cells.clamp(1, MAX_ENTRY_ROWS),
            rect: Rect::default(),
        });
        true
    }

    /// Retained graphics, oldest first.
    pub fn len(&self) -> usize {
        self.graphics.borrow().len()
    }

    /// True when nothing is pinned. The overwhelmingly common case: a session
    /// that has never pasted an image skips the whole pass.
    pub fn is_empty(&self) -> bool {
        self.graphics.borrow().is_empty()
    }

    /// Rows the strip has to reserve: the sum of the retained cell heights,
    /// already clamped per entry by [`PinnedImageRegistry::add`].
    pub fn rows_needed(&self) -> usize {
        self.graphics
            .borrow()
            .iter()
            .map(|g| usize::from(g.height_cells))
            .sum()
    }

    /// Recompute every entry's rect for `area`, newest entries included.
    ///
    /// The returned vector is index-aligned with the registry; a zero-width
    /// rect means the entry did not fit this frame and is not painted. Entries
    /// are placed top-down from the strip's top edge, each clamped to the strip
    /// width, so a graphic that no longer fits is skipped and the ones above it
    /// stay put.
    pub fn layout(&self, area: Rect) -> Vec<Rect> {
        let strip = strip_area(area, self.rows_needed());
        let bottom = strip.y.saturating_add(strip.height);
        let mut offset = 0u16;
        let mut rects = Vec::new();
        for graphic in self.graphics.borrow_mut().iter_mut() {
            let y = strip.y.saturating_add(offset);
            let fits = strip.width > 0 && y.saturating_add(graphic.height_cells) <= bottom;
            let rect = if fits {
                let rect = Rect {
                    x: strip.x,
                    y,
                    width: graphic.width_cells.min(strip.width),
                    height: graphic.height_cells,
                };
                offset = offset.saturating_add(graphic.height_cells);
                rect
            } else {
                Rect {
                    x: strip.x,
                    y,
                    width: 0,
                    height: 0,
                }
            };
            // Remember the cell the graphic claimed this frame so the post-flush
            // emit half can place it without redoing the layout.
            graphic.rect = rect;
            rects.push(rect);
        }
        rects
    }
}

// ---------------------------------------------------------------------------
// Routing the two producers into the registry
// ---------------------------------------------------------------------------

/// Route freshly pasted attachments into the registry; returns how many were
/// pinned.
///
/// `queue` is drained exactly once — the attachment itself is consumed, because
/// the temp PNG it points at is not ours to keep — but the graphic it produced
/// is retained and re-emitted every frame. That split is the whole fix: the old
/// pass drained the queue, wrote the sequence, and had nothing left.
pub fn pin_pasted(reg: &PinnedImageRegistry, queue: &RefCell<Vec<PastedImage>>) -> usize {
    let queued: Vec<PastedImage> = queue.borrow_mut().drain(..).collect();
    let mut pinned = 0;
    for image in queued {
        if reg.add(&render_attachment(&image)) {
            pinned += 1;
        }
    }
    pinned
}

/// Route freshly landed mermaid rasters into the registry; returns how many
/// were pinned.
///
/// The PNG is deleted here rather than kept: the escape sequence is the only
/// thing anyone reads from here on, and the registry now holds a copy of it. A
/// diagram the ladder resolved to source text has no PNG and pins nothing — the
/// source is in the transcript either way.
pub fn pin_rasters(reg: &PinnedImageRegistry, landed: &LandedRasters) -> usize {
    let mut pinned = 0;
    for png in &landed.pngs {
        let rendered = image_render::render_image(png, &ImageRenderConfig::default());
        if reg.add(&rendered) {
            pinned += 1;
        }
        let _ = std::fs::remove_file(png);
    }
    pinned
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// The strip region for `rows_needed` rows of pinned graphics.
///
/// A zero-height rect is the "nothing to draw" answer, for an empty registry
/// and for a terminal with no room below the usage overlay's reserved band. Pure
/// so the collision guarantees can be asserted directly against the two
/// functions the real chrome uses.
pub fn strip_area(area: Rect, rows_needed: usize) -> Rect {
    let none = Rect {
        x: area.x,
        y: area.y,
        width: 0,
        height: 0,
    };
    if rows_needed == 0 || area.width == 0 || area.height == 0 {
        return none;
    }

    // Start below the usage overlay's band. `panel_area` clamps to the area, so
    // on an area shorter than the panel the band covers everything and the
    // strip gets nothing — a graphic painted over the prompt would be worse
    // than no graphic.
    let panel = usage_overlay::panel_area(area);
    let top = panel.y.saturating_add(panel.height);
    let bottom = area.y.saturating_add(area.height);
    if top >= bottom {
        return none;
    }

    Rect {
        x: area.x,
        y: top,
        width: area.width,
        height: u16::try_from(rows_needed)
            .unwrap_or(u16::MAX)
            .min(bottom - top),
    }
}

// ---------------------------------------------------------------------------
// The per-frame pass, split around ratatui's flush
// ---------------------------------------------------------------------------
//
// `Terminal::draw` runs its closure and only THEN writes the cell diff to the
// terminal. Anything written to stdout from inside `render_app` therefore lands
// *before* the diff that would cover it — which is exactly why the image died on
// the first redraw, and why the OSC 8 overlay already lives out in the run loop.
// So the pass is two halves:
//
//   1. `prepare_strip` — inside the draw. Lays out and claims (blanks) the cells.
//   2. `emit_strip`    — after the draw. Writes the escape sequences.
//
// Both halves are load-bearing. Blanking pre-flush is what keeps the ratatui
// diff from rewriting those cells on steady-state frames; emitting post-flush is
// what stops a scrolled or resized frame from overwriting the graphic.

/// Half one: lay the strip out and blank the cells it owns. Must be called from
/// inside the `terminal.draw` closure, before the flush.
pub fn prepare_strip(buf: &mut Buffer, reg: &PinnedImageRegistry, area: Rect) {
    if reg.is_empty() {
        return;
    }
    for rect in reg.layout(area) {
        if rect.width != 0 && rect.height != 0 {
            blank(buf, rect);
        }
    }
}

/// Half two: re-emit every pinned graphic at the cells `prepare_strip` claimed.
/// Must be called AFTER `terminal.draw` has returned, so the sequences are not
/// overwritten by the cell diff.
///
/// A write failure is not an error worth propagating: the TUI has already
/// painted a correct frame, and an unwritable stdout is a lost picture, not a
/// lost session.
pub fn emit_strip(reg: &PinnedImageRegistry) {
    tracing::trace!(target: "pinned_images", count = reg.len(), "re-emitting pinned graphics");
    if let Err(err) = emit_strip_to(reg, &mut std::io::stdout()) {
        tracing::debug!(target: "pinned_images", "pinned graphics write failed: {err}");
    }
}

/// [`emit_strip`] with an explicit sink, so the emitted bytes are assertable
/// without a terminal.
pub fn emit_strip_to<W: Write>(reg: &PinnedImageRegistry, out: &mut W) -> io::Result<()> {
    let graphics = reg.graphics.borrow();
    let mut placed: Vec<(Rect, &str)> = Vec::new();
    for graphic in graphics.iter() {
        let rect = graphic.rect;
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        placed.push((rect, graphic.escape_sequence.as_str()));
    }
    if placed.is_empty() {
        return Ok(());
    }
    write_placements(out, &placed)
}

/// `CUP` for `rect`: move the cursor to its top-left cell so the payload paints
/// there. CUP is 1-based on both axes, ratatui is 0-based, hence the `+ 1`.
///
/// Public because it is the conversion the whole design rests on, and a wrong
/// off-by-one here is a picture painted one cell off — invisible in review.
pub fn placement_prefix(rect: Rect) -> String {
    format!(
        "\u{1b}[{};{}H",
        rect.y.saturating_add(1),
        rect.x.saturating_add(1)
    )
}

/// Write each placement as `CUP` + payload, bracketed by one cursor save and
/// restore for the whole strip.
///
/// Saved once around the loop rather than per graphic: the images are painted at
/// the cursor, so the sequence has to be bracketed, and N save/restore pairs
/// would leave the cursor wherever the last image happened to end.
fn write_placements<W: Write>(out: &mut W, placements: &[(Rect, &str)]) -> io::Result<()> {
    out.write_all(CURSOR_SAVE.as_bytes())?;
    for (rect, sequence) in placements {
        out.write_all(placement_prefix(*rect).as_bytes())?;
        out.write_all(sequence.as_bytes())?;
    }
    out.write_all(CURSOR_RESTORE.as_bytes())?;
    out.flush()
}

/// Blank the cells a pinned graphic owns.
///
/// Symbol only: the style is left as the frame fill and the transcript left it,
/// because rewriting the style to anything else makes the cell differ from the
/// previous frame's and hands the ratatui diff something to rewrite every frame
/// — which is precisely what destroys the image.
fn blank(buf: &mut Buffer, rect: Rect) {
    for y in rect.y..rect.y.saturating_add(rect.height) {
        for x in rect.x..rect.x.saturating_add(rect.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::background_tasks;
    use crate::tui::image_render::{PROTOCOL_FREE_ENV, with_env};
    use std::path::{Path, PathBuf};

    /// A terminal large enough that every guarantee is non-vacuous.
    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 40,
    };

    /// Present a Kitty terminal so `render_attachment` produces a real escape
    /// sequence whatever terminal happens to run the test suite.
    const KITTY_ENV: [(&str, Option<&str>); 4] = [
        ("TERM", Some("xterm-kitty")),
        ("KITTY_WINDOW_ID", Some("1")),
        ("TERM_PROGRAM", None),
        ("ITERM_SESSION_ID", None),
    ];

    /// A 24-byte PNG carrying a real IHDR, so the renderer has dimensions to
    /// scale and the Kitty payload is a genuine one.
    fn test_png(name: &str, w: u32, h: u32) -> PathBuf {
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        data[8..12].copy_from_slice(&13u32.to_be_bytes());
        data[12..16].copy_from_slice(b"IHDR");
        data[16..20].copy_from_slice(&w.to_be_bytes());
        data[20..24].copy_from_slice(&h.to_be_bytes());
        let path = std::env::temp_dir().join(format!("operant-pinned-{name}.png"));
        std::fs::write(&path, &data).expect("write test png");
        path
    }

    fn pasted(path: &Path, w: u32, h: u32) -> PastedImage {
        PastedImage {
            path: path.to_path_buf(),
            label: "clipboard.png".to_string(),
            dimensions: Some((w, h)),
        }
    }

    /// One frame of the pinned pass, returning the bytes it wrote.
    ///
    /// Deliberately mirrors production: `prepare_strip` runs inside the draw
    /// (layout + blank), `emit_strip_to` runs after it (the write). A test that
    /// collapsed the two would not catch a regression in either half.
    fn frame(reg: &PinnedImageRegistry, area: Rect, buf: &mut Buffer) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        prepare_strip(buf, reg, area);
        emit_strip_to(reg, &mut out).expect("write into a Vec cannot fail");
        out
    }

    // Test 1: THE REGRESSION. A graphic pinned in frame 1 is still pinned, and
    // still written, in frame 2 — which is the redraw the user performs. Against
    // the old drain-once pass the registry is empty here and frame 2 writes
    // nothing, because the queue was drained and the sequence forgotten.
    #[test]
    fn registry_survives_a_redraw() {
        let reg = PinnedImageRegistry::new();
        let path = test_png("redraw", 64, 32);
        let queue = RefCell::new(vec![pasted(&path, 64, 32)]);

        // Frame 1: the paste lands. The queue is drained…
        with_env(&KITTY_ENV, || pin_pasted(&reg, &queue));
        assert!(
            queue.borrow().is_empty(),
            "the attachment itself is consumed once"
        );
        assert_eq!(reg.len(), 1, "frame 1 must pin the attachment");

        // …and every later frame starts from an empty queue, which is exactly
        // the state the drain-once pass was stuck in forever.
        let mut buf = Buffer::empty(AREA);
        let painted_first = frame(&reg, AREA, &mut buf);
        let painted_redraw = frame(&reg, AREA, &mut buf);

        assert_eq!(
            reg.len(),
            1,
            "the pinned graphic must survive the redraw, not be consumed by it"
        );
        assert!(!painted_first.is_empty(), "frame 1 must write the graphic");
        assert_eq!(
            painted_first, painted_redraw,
            "every frame re-emits the same graphic — a redraw that writes nothing \
             leaves the image gone"
        );
        let text = String::from_utf8(painted_redraw).expect("the payload is ASCII");
        assert!(
            text.contains("_G"),
            "the Kitty payload must be in the frame's output: {text:?}"
        );

        let _ = std::fs::remove_file(&path);
    }

    // Test 2: the strip must never land on the other two things that paint into
    // the messages area — the usage panel (top-right) and the async-delegation
    // task rows (bottom-left, growing upward).
    #[test]
    fn strip_area_never_overlaps_the_usage_overlay_or_task_rows() {
        // Degenerate, narrow, exactly-panel-tall, and comfortably large — with
        // and without a non-zero origin, so an off-by-one in the clamping shows.
        let sizes = [
            (0, 0, 0, 0),
            (0, 0, 1, 1),
            (0, 0, 5, 3),
            (0, 0, 31, 8),
            (0, 0, 7, 7),
            (0, 0, 20, 9),
            (3, 2, 20, 10),
            (0, 0, 40, 12),
            (0, 0, 80, 24),
            (0, 0, 120, 50),
            (0, 0, 200, 60),
        ];
        for (x, y, w, h) in sizes {
            let area = Rect::new(x, y, w, h);
            for rows_needed in [1usize, 4, 12, 48, 1000] {
                let strip = strip_area(area, rows_needed);
                let what = format!("{x}x{y} {w}x{h}, {rows_needed} rows -> {strip:?}");

                // Never escapes the area it was handed.
                assert!(strip.x >= area.x && strip.right() <= area.right(), "{what}");
                assert!(
                    strip.y >= area.y && strip.bottom() <= area.bottom(),
                    "{what}"
                );

                // The usage panel, using the panel's own geometry function.
                assert!(
                    !strip.intersects(usage_overlay::panel_area(area)),
                    "strip overlaps the usage panel: {what}"
                );

                // The task rows, for every row count the strip leaves room for.
                // The band it leaves free is exactly the guarantee: the rows are
                // bottom-anchored, so the strip is safe while
                // `row_count <= area.bottom - strip.y - strip.height - 1`.
                let max_rows = area
                    .bottom()
                    .saturating_sub(strip.y)
                    .saturating_sub(strip.height)
                    .saturating_sub(1);
                for row_count in 0..=max_rows {
                    let rows = background_tasks::rows_area(area, usize::from(row_count));
                    assert!(
                        !strip.intersects(rows),
                        "strip overlaps {row_count} task rows: {what} (rows {rows:?})"
                    );
                }
            }
        }
    }

    // Test 3: empty is a zero-height no-op, and a degenerate terminal neither
    // panics nor writes.
    #[test]
    fn strip_area_is_zero_height_when_empty() {
        for (w, h) in [(0u16, 0u16), (1, 1), (8, 8), (80, 24), (200, 60)] {
            let area = Rect::new(0, 0, w, h);
            let strip = strip_area(area, 0);
            assert_eq!(strip.height, 0, "empty registry must reserve nothing");
            assert_eq!(strip.width, 0, "empty registry must reserve nothing");

            let reg = PinnedImageRegistry::new();
            assert!(reg.is_empty());
            let mut buf = Buffer::empty(area);
            assert!(frame(&reg, area, &mut buf).is_empty());
        }
    }

    // Test 3b: a *populated* registry on a degenerate terminal writes nothing
    // rather than indexing out of the buffer.
    #[test]
    fn a_populated_registry_degrades_on_a_degenerate_terminal() {
        let reg = PinnedImageRegistry::new();
        let path = test_png("degenerate", 64, 32);
        with_env(&KITTY_ENV, || {
            reg.add(&render_attachment(&pasted(&path, 64, 32)))
        });
        assert_eq!(reg.len(), 1, "the graphic is still registered");

        for (w, h) in [(0u16, 0u16), (1, 1), (3, 2), (10, 8)] {
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let painted = frame(&reg, area, &mut buf);
            let strip = strip_area(area, reg.rows_needed());
            if strip.height == 0 || strip.width == 0 {
                assert!(
                    painted.is_empty(),
                    "no room for the strip must mean no write: {painted:?}"
                );
            }
        }
        let _ = std::fs::remove_file(&path);
    }

    // Test 4: the payload is written with an explicit cursor move, and the CUP
    // conversion is 1-based on both axes. The top-left cell of the screen is
    // row 1, column 1 — writing `0;0` would pin every image off-screen or
    // wrapped to the bottom of the terminal.
    #[test]
    fn placement_prefix_is_a_one_based_cursor_move() {
        assert_eq!(placement_prefix(Rect::new(0, 0, 10, 4)), "\u{1b}[1;1H");
        assert_eq!(placement_prefix(Rect::new(4, 7, 10, 4)), "\u{1b}[8;5H");
        // The far corner must saturate, never wrap round to 0.
        assert_eq!(
            placement_prefix(Rect::new(u16::MAX, u16::MAX, 1, 1)),
            "\u{1b}[65535;65535H"
        );
    }

    // Test 4b: the bytes on the wire are save, then CUP+payload per graphic,
    // then restore — with the payload untouched. Pinning is a cursor move; if
    // anything ever rewrote the payload it would have to re-encode the image.
    #[test]
    fn write_placements_brackets_the_payload_with_a_cursor_move_and_restore() {
        let placements = [
            (Rect::new(0, 0, 4, 2), "\u{1b}_Gpayload\u{1b}\\"),
            (Rect::new(2, 6, 4, 2), "SECOND"),
        ];
        let mut out: Vec<u8> = Vec::new();
        write_placements(&mut out, &placements).expect("write into a Vec cannot fail");
        let text = String::from_utf8(out).expect("the fixture payloads are ASCII");
        assert_eq!(
            text,
            "\u{1b}7\u{1b}[1;1H\u{1b}_Gpayload\u{1b}\\\u{1b}[7;3HSECOND\u{1b}8"
        );
        assert!(text.contains("_Gpayload"), "payload must be passed through");
    }

    // Test 5: the module must stay WIRED. Nothing else here can fail if the
    // render pass stops calling the registry — the registry would compile, pass
    // every test above, and quietly show the user nothing, which is the failure
    // this whole module exists to fix. So pin the call sites by name, against
    // the source with its COMMENTS STRIPPED.
    //
    // The stripping is not decoration. A plain `contains` gate is satisfied by a
    // commented-out call, which is exactly how it was mutation-proven useless:
    // commenting the re-emit out left the gate green. See
    // `comment_stripping_cannot_be_fooled_by_a_commented_out_call`.
    #[test]
    fn render_pass_still_wires_the_registry() {
        let render = code_only(include_str!("render/mod.rs"));

        for needle in [
            "pinned_images::pin_pasted",
            "pinned_images::pin_rasters",
            // Pre-flush half: layout + blank, inside the draw closure.
            "pinned_images::prepare_strip",
        ] {
            assert!(
                render.contains(needle),
                "render/mod.rs no longer calls `{needle}` — the pinned registry would \
                 compile and pass every test in this file while the user sees no \
                 image at all"
            );
        }

        // The write half lives in the run loop, NOT in render_app: `Terminal::draw`
        // flushes after its closure returns, so a write from inside the closure
        // lands before the cell diff that covers it. Losing this call does not
        // blank anything, so none of the other tests here would notice.
        let loop_ = code_only(include_str!("app/mod.rs"));
        assert!(
            loop_.contains("pinned_images::emit_strip"),
            "app/mod.rs no longer calls `pinned_images::emit_strip` after the draw — \
             the strip would reserve and blank its cells every frame while the \
             graphic itself was never written, so the user sees a blank gap"
        );
        assert!(
            !loop_.contains("pinned_images::prepare_strip"),
            "app/mod.rs calls `prepare_strip` — blanking must happen INSIDE the draw \
             closure, before the flush, or the cell diff repaints the strip"
        );

        // And the one-shot writers must stay gone: both of them write a graphic
        // exactly once and forget it, which is the defect.
        assert!(
            !render.contains("emit_inline_image"),
            "render/mod.rs calls image_paste::emit_inline_image again — that writes \
             each pasted graphic once and forgets it, so the next redraw destroys it"
        );
        assert!(
            !render.contains("mermaid::drain_ready_images"),
            "render/mod.rs calls mermaid::drain_ready_images again — the one-shot \
             mermaid consumer; it must hand rasters to pinned_images instead"
        );
    }

    // The gate's own machinery. A wiring gate that a commented-out call can
    // satisfy is not a gate, and this is the exact shape that fooled it: the
    // needle is present, in the file, on a line the compiler ignores.
    #[test]
    fn comment_stripping_cannot_be_fooled_by_a_commented_out_call() {
        let live = "fn f() {\n    crate::tui::pinned_images::paint_strip(a, b, c);\n}\n";
        assert!(code_only(live).contains("pinned_images::paint_strip"));

        for dead in [
            // Line-commented call.
            "fn f() {\n    // crate::tui::pinned_images::paint_strip(a, b, c);\n}\n",
            // Block-commented call, the shape rustfmt leaves behind when a
            // whole call is wrapped.
            "fn f() {\n    /* crate::tui::pinned_images::paint_strip(a, b, c); */\n}\n",
        ] {
            assert!(
                !code_only(dead).contains("pinned_images::paint_strip"),
                "a commented-out call must not satisfy a wiring gate: {dead}"
            );
        }

        // A doc comment that merely NAMES the old writer is prose, so it is
        // stripped — which is what lets the negative gate stay honest when the
        // prose is updated. A real call is code, and survives.
        let mentioned = "/// see image_paste::emit_inline_image\nfn f() {}\n";
        assert!(!code_only(mentioned).contains("emit_inline_image"));
        let called = "fn f() { image_paste::emit_inline_image(&img); }\n";
        assert!(code_only(called).contains("emit_inline_image"));
    }

    /// `src` with comments removed, so a commented-out call cannot satisfy a
    /// wiring gate.
    ///
    /// Tracks string literals so a `//` inside one (a URL, a regex) is not
    /// mistaken for a comment. Not a Rust parser: char literals need no special
    /// case, because a bare `/` in code is pushed through and only `//` and
    /// `/*` open a comment. Raw strings (`r#"…"#`) are not tracked — no source
    /// searched by this module contains one.
    fn code_only(src: &str) -> String {
        let chars: Vec<char> = src.chars().collect();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '/' if chars.get(i + 1) == Some(&'/') => {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                }
                '/' if chars.get(i + 1) == Some(&'*') => {
                    i += 2;
                    while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                        i += 1;
                    }
                    i = (i + 2).min(chars.len());
                }
                '"' => {
                    out.push('"');
                    i += 1;
                    while i < chars.len() {
                        if chars[i] == '\\' {
                            out.push(chars[i]);
                            if let Some(next) = chars.get(i + 1) {
                                out.push(*next);
                            }
                            i += 2;
                            continue;
                        }
                        let c = chars[i];
                        out.push(c);
                        i += 1;
                        if c == '"' {
                            break;
                        }
                    }
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        out
    }

    // The cap is part of the contract: every frame re-emits every retained
    // payload, so the write stays bounded at MAX_GRAPHICS, oldest first out.
    #[test]
    fn the_registry_is_capped_and_evicts_oldest_out() {
        let reg = PinnedImageRegistry::new();
        // Distinct pixel heights so each graphic lands on a distinct number of
        // cell rows: the survivors are then identifiable by their row demand.
        for i in 1..=(MAX_GRAPHICS + 2) {
            let px: u32 = 16 * u32::try_from(i).expect("i is a small literal-driven count");
            let path = test_png(&format!("cap{i}"), 32, px);
            with_env(&KITTY_ENV, || {
                assert!(reg.add(&render_attachment(&pasted(&path, 32, px))))
            });
            let _ = std::fs::remove_file(&path);
        }
        assert_eq!(reg.len(), MAX_GRAPHICS, "the cap is a hard ceiling");

        // Graphics 1 and 2 were evicted; the survivors are rows 3..=MAX+2, i.e.
        // cell heights 3..=MAX_GRAPHICS+2.
        let expected: usize = (3..=MAX_GRAPHICS + 2).sum();
        assert_eq!(reg.rows_needed(), expected, "oldest out, not newest out");
    }

    // A per-entry height clamp, so one tall diagram cannot eat the strip.
    #[test]
    fn an_oversized_graphic_is_clamped_to_the_row_budget() {
        let reg = PinnedImageRegistry::new();
        let path = test_png("tall", 64, 3200);
        with_env(&KITTY_ENV, || {
            reg.add(&render_attachment(&pasted(&path, 64, 3200)))
        });
        assert_eq!(reg.rows_needed(), usize::from(MAX_ENTRY_ROWS));

        let rects = reg.layout(AREA);
        assert_eq!(rects.len(), 1);
        assert!(rects[0].height <= MAX_ENTRY_ROWS, "{:?}", rects[0]);
        let _ = std::fs::remove_file(&path);
    }

    // A protocol-free terminal yields a textual placeholder, and pinning a
    // string as though it were a picture would blank transcript cells forever.
    #[test]
    fn a_placeholder_is_never_pinned() {
        let reg = PinnedImageRegistry::new();
        let path = test_png("placeholder", 640, 480);
        let rendered = with_env(&PROTOCOL_FREE_ENV, || {
            render_attachment(&pasted(&path, 640, 480))
        });
        assert!(!rendered.success);
        assert!(!reg.add(&rendered), "a placeholder must not be pinned");
        assert!(reg.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    // The strip's cells must be blanked, or the ratatui diff keeps rewriting
    // them and the image has nowhere to live.
    #[test]
    fn the_strip_cells_are_blanked() {
        let reg = PinnedImageRegistry::new();
        let path = test_png("blank", 64, 32);
        with_env(&KITTY_ENV, || {
            reg.add(&render_attachment(&pasted(&path, 64, 32)))
        });

        let mut buf = Buffer::empty(AREA);
        for y in AREA.y..AREA.bottom() {
            for x in AREA.x..AREA.right() {
                buf.cell_mut((x, y)).expect("in bounds").set_symbol("#");
            }
        }
        frame(&reg, AREA, &mut buf);

        let rect = reg.layout(AREA)[0];
        assert!(rect.width > 0 && rect.height > 0, "{rect:?}");
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let cell = buf.cell((x, y)).expect("in bounds");
                assert_eq!(cell.symbol(), " ", "cell ({x},{y}) not blanked");
            }
        }
        // A cell outside the strip keeps its content.
        let outside = buf.cell((rect.x + rect.width, rect.y)).expect("in bounds");
        assert_eq!(outside.symbol(), "#", "the strip must not blank the row");

        let _ = std::fs::remove_file(&path);
    }
}
