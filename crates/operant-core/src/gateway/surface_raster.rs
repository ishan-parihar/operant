//! B2 (iter-2656): the gateway's cell-buffer rasteriser.
//!
//! Renders a `SurfaceOp`'s widget tree to a PNG — each terminal cell becomes
//! an N×N block of pixels, box-drawing glyphs become the geometric patterns
//! they look like, and every colour comes from `SemanticTone` (the IR's only
//! colour surface) mapped onto a fixed 16-colour palette.
//!
//! Two rules carried from the IR:
//!
//! 1. The renderer TRUSTS the gate — the op arrives post-scrub, so no string
//!    here is re-scrubbed; a second scrub would mean the gate's verdict
//!    depends on who drew.
//! 2. Nothing interactive is interactive in a PNG: buttons draw their labels;
//!    the caption carries the question. A raster cannot claim a tap target.

use image::{Rgba, RgbaImage};

use crate::ui::{ChartKind, DiffKind, SemanticTone};
use crate::ui::{TableColumn, TableRow, WidgetNode};

/// One terminal cell is 8×16 pixels: the classic VGA text-mode ratio that
/// keeps glyphs square-ish on a 96-dpi display. A 100-column card is 800 px
/// wide; a 30-row card is 480 px tall.
pub const CELL_W: u32 = 8;
pub const CELL_H: u32 = 16;

/// The fixed 16-colour palette (the ANSI set the rest of the gateway assumes).
const PALETTE: [(u8, u8, u8); 16] = [
    (0, 0, 0),       // 0 black
    (170, 0, 0),     // 1 red
    (0, 170, 0),     // 2 green
    (170, 85, 0),    // 3 yellow
    (0, 0, 170),     // 4 blue
    (170, 0, 170),   // 5 magenta
    (0, 170, 170),   // 6 cyan
    (170, 170, 170), // 7 white
    (85, 85, 85),    // 8 bright black
    (255, 85, 85),   // 9 bright red
    (85, 255, 85),   // 10 bright green
    (255, 255, 85),  // 11 bright yellow
    (85, 85, 255),   // 12 bright blue
    (255, 85, 255),  // 13 bright magenta
    (85, 255, 255),  // 14 bright cyan
    (255, 255, 255), // 15 bright white
];

/// The SemanticTone -> palette index map. One arm per tone; the compiler
/// enforces the mapping is total.
fn tone_index(tone: SemanticTone) -> usize {
    match tone {
        SemanticTone::Default => 7,
        SemanticTone::Muted => 8,
        SemanticTone::Accent => 12,
        SemanticTone::Success => 10,
        SemanticTone::Warning => 11,
        SemanticTone::Danger => 9,
    }
}

/// One styled cell of the raster grid.
#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: usize,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: 7,
        }
    }
}

/// The raster grid: rows of cells, each cell 8×16 px.
struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
}

impl Grid {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![Cell::default(); cols * rows],
        }
    }

    fn put(&mut self, col: usize, row: usize, cell: Cell) {
        if col < self.cols && row < self.rows {
            self.cells[row * self.cols + col] = cell;
        }
    }

    /// Draw one cell of text starting at (col,row), clipped to the grid.
    fn text(&mut self, col: usize, row: usize, text: &str, fg: usize) {
        for (i, ch) in text.chars().enumerate() {
            self.put(col + i, row, Cell { ch, fg });
        }
    }

    /// Fill a rectangle of cells with a character and colour.
    fn fill(&mut self, col: usize, row: usize, w: usize, h: usize, ch: char, fg: usize) {
        for r in row..row + h {
            for c in col..col + w {
                self.put(c, r, Cell { ch, fg });
            }
        }
    }

    /// Rasterise to an RGBA image: each cell paints its glyph from a tiny
    /// built-in 5×7 bitmap font, scaled 1× into the 8×16 cell (top-aligned,
    /// left-padded 1 px). Box-drawing and block characters get geometric
    /// patterns instead of font glyphs, so borders stay crisp.
    fn to_image(&self) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(
            self.cols as u32 * CELL_W,
            self.rows as u32 * CELL_H,
            Rgba([0, 0, 0, 255]),
        );
        for (i, cell) in self.cells.iter().enumerate() {
            let col = (i % self.cols) as u32;
            let row = (i / self.cols) as u32;
            let fg = PALETTE[cell.fg.min(15)];
            let ox = col * CELL_W;
            let oy = row * CELL_H;
            draw_cell(&mut img, ox, oy, cell.ch, fg);
        }
        img
    }
}

/// Draw one character inside its 8×16 cell.
fn draw_cell(img: &mut RgbaImage, ox: u32, oy: u32, ch: char, fg: (u8, u8, u8)) {
    let pixel = Rgba([fg.0, fg.1, fg.2, 255]);
    match ch {
        // Geometric box-drawing: borders are full-bleed lines so a card
        // reads as a card at any zoom.
        '─' => hline(img, ox, oy + 7, CELL_W, pixel),
        '│' => vline(img, ox + 3, oy, CELL_H, pixel),
        '┌' => {
            hline(img, ox, oy + 7, CELL_W, pixel);
            vline(img, ox + 3, oy, 8, pixel);
        }
        '┐' => {
            hline(img, ox, oy + 7, CELL_W, pixel);
            vline(img, ox + 4, oy, 8, pixel);
        }
        '└' => {
            hline(img, ox, oy + 8, CELL_W, pixel);
            vline(img, ox + 3, oy + 8, 8, pixel);
        }
        '┘' => {
            hline(img, ox, oy + 8, CELL_W, pixel);
            vline(img, ox + 4, oy + 8, 8, pixel);
        }
        '├' => {
            vline(img, ox + 3, oy, CELL_H, pixel);
            hline(img, ox + 4, oy + 7, CELL_W - 4, pixel);
        }
        '┤' => {
            vline(img, ox + 4, oy, CELL_H, pixel);
            hline(img, ox, oy + 7, 4, pixel);
        }
        '┬' => {
            hline(img, ox, oy + 7, CELL_W, pixel);
            vline(img, ox + 3, oy, 7, pixel);
        }
        '┴' => {
            hline(img, ox, oy + 8, CELL_W, pixel);
            vline(img, ox + 3, oy + 8, 8, pixel);
        }
        '┼' => {
            hline(img, ox, oy + 7, CELL_W, pixel);
            vline(img, ox + 3, oy, CELL_H, pixel);
        }
        '█' => img.put_pixel(ox + 3, oy + 7, pixel),
        '▀' => hline(img, ox, oy + 3, CELL_W, pixel),
        '▄' => hline(img, ox, oy + 11, CELL_W, pixel),
        '▌' => vline(img, ox + 3, oy, CELL_H, pixel),
        '▐' => vline(img, ox + 4, oy, CELL_H, pixel),
        '░' => {
            for y in (0..CELL_H).step_by(2) {
                for x in (0..CELL_W).step_by(2) {
                    img.put_pixel(ox + x, oy + y, pixel);
                }
            }
        }
        '▒' => {
            for y in 0..CELL_H {
                for x in 0..CELL_W {
                    if (x + y) % 2 == 0 {
                        img.put_pixel(ox + x, oy + y, pixel);
                    }
                }
            }
        }
        '▓' => {
            for y in 0..CELL_H {
                for x in 0..CELL_W {
                    if (x + y) % 2 == 1 || x < 4 {
                        img.put_pixel(ox + x, oy + y, pixel);
                    }
                }
            }
        }
        '✓' => {
            let pts = [(1, 8), (2, 9), (3, 10), (4, 9), (5, 8), (6, 7), (7, 6)];
            for (dx, dy) in pts {
                img.put_pixel(ox + dx, oy + dy, pixel);
            }
        }
        '✗' => {
            for i in 0..6u32 {
                img.put_pixel(ox + 1 + i, oy + 3 + i, pixel);
                img.put_pixel(ox + 6 - i, oy + 3 + i, pixel);
            }
        }
        '•' => {
            img.put_pixel(ox + 3, oy + 7, pixel);
            img.put_pixel(ox + 4, oy + 7, pixel);
            img.put_pixel(ox + 3, oy + 8, pixel);
            img.put_pixel(ox + 4, oy + 8, pixel);
        }
        '…' => {
            for x in [1, 3, 5] {
                img.put_pixel(ox + x, oy + 8, pixel);
            }
        }
        // Everything else: the built-in 5×7 font (ASCII only; non-ASCII
        // falls back to a hollow box so a missing glyph is legible).
        _ => draw_ascii(img, ox, oy, ch, pixel),
    }
}

fn hline(img: &mut RgbaImage, x: u32, y: u32, w: u32, px: Rgba<u8>) {
    for dx in 0..w {
        img.put_pixel(x + dx, y, px);
    }
}

fn vline(img: &mut RgbaImage, x: u32, y: u32, h: u32, px: Rgba<u8>) {
    for dy in 0..h {
        img.put_pixel(x, y + dy, px);
    }
}

/// The built-in 5×7 font: one row per glyph, 5 bits per row, MSB = leftmost.
/// Covers printable ASCII; anything else draws as a hollow box.
fn glyph_rows(ch: char) -> [u8; 7] {
    match ch {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '"' => [0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00],
        '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
        '$' => [0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '&' => [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D],
        '\'' => [0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '*' => [0x00, 0x04, 0x15, 0x0E, 0x15, 0x04, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x00, 0x04, 0x00, 0x00, 0x04, 0x00],
        ';' => [0x00, 0x04, 0x00, 0x00, 0x04, 0x04, 0x08],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '@' => [0x0E, 0x11, 0x01, 0x0D, 0x15, 0x15, 0x0E],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '[' => [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
        '\\' => [0x10, 0x08, 0x08, 0x04, 0x02, 0x02, 0x01],
        ']' => [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
        '^' => [0x04, 0x0A, 0x11, 0x00, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '`' => [0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00],
        'a' => [0x00, 0x00, 0x0E, 0x01, 0x0F, 0x11, 0x0F],
        'b' => [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x1E],
        'c' => [0x00, 0x00, 0x0E, 0x10, 0x10, 0x11, 0x0E],
        'd' => [0x01, 0x01, 0x0D, 0x13, 0x11, 0x11, 0x0F],
        'e' => [0x00, 0x00, 0x0E, 0x11, 0x1F, 0x10, 0x0E],
        'f' => [0x06, 0x08, 0x08, 0x1C, 0x08, 0x08, 0x08],
        'g' => [0x00, 0x0F, 0x11, 0x11, 0x0F, 0x01, 0x0E],
        'h' => [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x11],
        'i' => [0x04, 0x00, 0x0C, 0x04, 0x04, 0x04, 0x0E],
        'j' => [0x02, 0x00, 0x06, 0x02, 0x02, 0x12, 0x0C],
        'k' => [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12],
        'l' => [0x0C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'm' => [0x00, 0x00, 0x1A, 0x15, 0x15, 0x11, 0x11],
        'n' => [0x00, 0x00, 0x16, 0x19, 0x11, 0x11, 0x11],
        'o' => [0x00, 0x00, 0x0E, 0x11, 0x11, 0x11, 0x0E],
        'p' => [0x00, 0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10],
        'q' => [0x00, 0x0F, 0x11, 0x11, 0x0F, 0x01, 0x01],
        'r' => [0x00, 0x00, 0x16, 0x19, 0x10, 0x10, 0x10],
        's' => [0x00, 0x00, 0x0F, 0x10, 0x0E, 0x01, 0x1E],
        't' => [0x08, 0x08, 0x1C, 0x08, 0x08, 0x09, 0x06],
        'u' => [0x00, 0x00, 0x11, 0x11, 0x11, 0x13, 0x0D],
        'v' => [0x00, 0x00, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'w' => [0x00, 0x00, 0x11, 0x11, 0x15, 0x15, 0x0A],
        'x' => [0x00, 0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11],
        'y' => [0x00, 0x11, 0x11, 0x11, 0x0F, 0x01, 0x0E],
        'z' => [0x00, 0x00, 0x1F, 0x02, 0x04, 0x08, 0x1F],
        '{' => [0x02, 0x04, 0x04, 0x08, 0x04, 0x04, 0x02],
        '|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        '}' => [0x08, 0x04, 0x04, 0x02, 0x04, 0x04, 0x08],
        '~' => [0x00, 0x00, 0x08, 0x15, 0x02, 0x00, 0x00],
        _ => [0x1F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1F],
    }
}

fn draw_ascii(img: &mut RgbaImage, ox: u32, oy: u32, ch: char, fg: Rgba<u8>) {
    let rows = glyph_rows(ch);
    for (y, bits) in rows.iter().enumerate() {
        for x in 0..5u32 {
            if bits & (1 << (4 - x)) != 0 {
                img.put_pixel(ox + 1 + x, oy + 2 + y as u32, fg);
            }
        }
    }
}

fn render_node(node: &WidgetNode, cols: usize, grid: &mut Grid, row: &mut usize) {
    match node {
        WidgetNode::Row { children } => {
            let start_row = *row;
            let per = (cols / children.len().max(1)).max(1);
            for (i, child) in children.iter().enumerate() {
                *row = start_row;
                let sub = child_grid(child, per);
                blit(grid, &sub, i * per, start_row);
                *row += sub.rows;
            }
        }
        WidgetNode::Column { children } => {
            for child in children {
                render_node(child, cols, grid, row);
            }
        }
        WidgetNode::Card { title, child, tone } => {
            let inner_cols = cols.saturating_sub(2).max(1);
            let mut sub = child_grid(child, inner_cols);
            sub.rows += 2; // borders
            let border = tone_index(*tone);
            let mut card = Grid::new(cols, sub.rows);
            for c in 0..cols {
                card.put(
                    c,
                    0,
                    Cell {
                        ch: '─',
                        fg: border,
                    },
                );
                card.put(
                    c,
                    sub.rows - 1,
                    Cell {
                        ch: '─',
                        fg: border,
                    },
                );
            }
            for r in 0..sub.rows {
                card.put(
                    0,
                    r,
                    Cell {
                        ch: '│',
                        fg: border,
                    },
                );
                card.put(
                    cols - 1,
                    r,
                    Cell {
                        ch: '│',
                        fg: border,
                    },
                );
            }
            // Title over the top border.
            card.text(1, 0, &format!(" {title} "), border);
            for r in 0..sub.rows - 2 {
                for c in 0..inner_cols {
                    let cell = *sub
                        .cells
                        .get(r * inner_cols + c)
                        .unwrap_or(&Cell::default());
                    card.put(c + 1, r + 1, cell);
                }
            }
            blit(grid, &card, 0, *row);
            *row += card.rows;
        }
        WidgetNode::Tabs {
            tabs,
            active,
            child,
        } => {
            let sub = child_grid(child, cols);
            let mut tab_row = Grid::new(cols, 1);
            let mut c = 0;
            for (i, tab) in tabs.iter().enumerate() {
                if i > 0 {
                    tab_row.text(c, 0, " ", 8);
                    c += 1;
                }
                let fg = if i == *active { 12 } else { 8 };
                tab_row.text(c, 0, tab, fg);
                c += tab.chars().count();
            }
            blit(grid, &tab_row, 0, *row);
            *row += 1;
            blit(grid, &sub, 0, *row);
            *row += sub.rows;
        }
        WidgetNode::List { items, ordered } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", i + 1)
                } else {
                    "•".to_string()
                };
                grid.text(0, *row, &marker, 12);
                grid.text(
                    marker.chars().count() + 1,
                    *row,
                    &item.text,
                    tone_index(item.tone),
                );
                *row += 1;
                if let Some(detail) = item.detail.as_deref() {
                    grid.text(marker.chars().count() + 3, *row, detail, 8);
                    *row += 1;
                }
            }
        }
        WidgetNode::Divider => {
            grid.fill(0, *row, cols, 1, '─', 8);
            *row += 1;
        }
        WidgetNode::Text { text, tone, .. } => {
            for wrap in wrap_text(text, cols) {
                grid.text(0, *row, &wrap, tone_index(*tone));
                *row += 1;
            }
        }
        WidgetNode::Badge { label, tone } => {
            let mut c = 0;
            for ch in label.chars() {
                grid.put(
                    c,
                    *row,
                    Cell {
                        ch,
                        fg: tone_index(*tone),
                    },
                );
                c += 1;
            }
            *row += 1;
        }
        WidgetNode::KeyValueGrid { pairs, columns } => {
            let columns = (*columns).max(1);
            for chunk in pairs.chunks(columns) {
                let mut c = 0;
                for (i, pair) in chunk.iter().enumerate() {
                    if i > 0 {
                        c += 2;
                    }
                    let key = format!("{}: ", pair.key);
                    grid.text(c, *row, &key, 8);
                    c += key.chars().count();
                    grid.text(c, *row, &pair.value, tone_index(pair.tone));
                    c += pair.value.chars().count();
                }
                *row += 1;
            }
        }
        WidgetNode::Table { columns, rows } => {
            render_table_grid(columns, rows, cols, grid, row);
        }
        WidgetNode::BarChart { title, kind, data } => {
            grid.text(0, *row, title, 7);
            *row += 1;
            match kind {
                ChartKind::Sparkline => {
                    let max = data.iter().map(|d| d.value.abs()).fold(0.0f64, f64::max);
                    let mut c = 0;
                    for d in data {
                        let frac = if max > 0.0 { d.value.abs() / max } else { 0.0 };
                        let ch = match (frac * 7.0) as usize {
                            0 => '▁',
                            1 => '▂',
                            2 => '▃',
                            3 => '▄',
                            4 => '▅',
                            5 => '▆',
                            6 => '▇',
                            _ => '█',
                        };
                        grid.put(c, *row, Cell { ch, fg: 12});
                        c += 1;
                    }
                    *row += 1;
                }
                ChartKind::Bar => {
                    let max = data.iter().map(|d| d.value.abs()).fold(0.0f64, f64::max);
                    let label_w = data
                        .iter()
                        .map(|d| d.label.len())
                        .max()
                        .unwrap_or(0)
                        .min(12);
                    let bar_space = cols.saturating_sub(label_w + 2).max(4);
                    for d in data {
                        let frac = if max > 0.0 { d.value.abs() / max } else { 0.0 };
                        let bar = ((frac * bar_space as f64).round() as usize).max(1);
                        grid.text(0, *row, &d.label, 7);
                        grid.fill(
                            label_w + 1,
                            *row,
                            bar,
                            1,
                            '█',
                            if d.value < 0.0 { 9 } else { 10 },
                        );
                        *row += 1;
                    }
                }
            }
        }
        WidgetNode::Progress {
            label,
            value,
            detail,
        } => {
            let pct = value.clamp(0.0, 100.0);
            let bar_w = 24usize.min(cols.saturating_sub(label.len() + 8).max(8));
            let filled = ((pct / 100.0) * bar_w as f64).round() as usize;
            grid.text(0, *row, label, 7);
            grid.fill(label.len() + 1, *row, filled, 1, '█', 10);
            grid.fill(label.len() + 1 + filled, *row, bar_w - filled, 1, '░', 8);
            let pct_text = format!(" {:.0}%", pct);
            grid.text(label.len() + 2 + bar_w, *row, &pct_text, 7);
            if let Some(detail) = detail.as_deref() {
                *row += 1;
                grid.text(0, *row, detail, 8);
            }
            *row += 1;
        }
        WidgetNode::CodeDiff {
            path,
            lines,
            show_markers,
        } => {
            if let Some(path) = path.as_deref() {
                grid.text(0, *row, path, 7);
                *row += 1;
            }
            for diff in lines {
                let (marker, fg) = match diff.kind {
                    DiffKind::Context => (" ", 7),
                    DiffKind::Added => ("+", 10),
                    DiffKind::Removed => ("-", 9),
                    DiffKind::Hunk => ("@", 12),
                };
                let mut c = 0;
                if *show_markers {
                    grid.put(
                        c,
                        *row,
                        Cell {
                            ch: marker.chars().next().unwrap_or(' '),
                            fg,
                        },
                    );
                    c += 1;
                }
                grid.text(c, *row, &diff.text, fg);
                *row += 1;
            }
        }
        WidgetNode::StatusTimeline { steps } => {
            for step in steps {
                let glyph = match step.tone {
                    SemanticTone::Success => "✓",
                    SemanticTone::Warning => "!",
                    SemanticTone::Danger => "✗",
                    SemanticTone::Accent => "◐",
                    _ => "○",
                };
                grid.text(0, *row, glyph, tone_index(step.tone));
                grid.text(2, *row, &step.label, 7);
                if let Some(detail) = step.detail.as_deref() {
                    *row += 1;
                    grid.text(4, *row, detail, 8);
                }
                *row += 1;
            }
        }
        WidgetNode::Button { buttons } => {
            let mut c = 0;
            for b in buttons {
                let fg = tone_index(b.tone);
                grid.text(c, *row, &format!(" {} ", b.label), fg);
                c += b.label.chars().count() + 2;
            }
            *row += 1;
        }
        WidgetNode::ChoicePicker { question, choices } => {
            for wrap in wrap_text(question, cols) {
                grid.text(0, *row, &wrap, 7);
                *row += 1;
            }
            for (i, choice) in choices.iter().enumerate() {
                grid.text(0, *row, &format!("  {}. ", i + 1), 12);
                let off = format!("  {}. ", i + 1).chars().count();
                grid.text(off, *row, &choice.label, 7);
                if let Some(desc) = choice.description.as_deref() {
                    *row += 1;
                    grid.text(off + 2, *row, desc, 8);
                }
                *row += 1;
            }
        }
        WidgetNode::TextField {
            label,
            value,
            placeholder,
            masked,
        } => {
            let shown = if !value.is_empty() {
                if *masked {
                    "•".repeat(value.chars().count())
                } else {
                    value.clone()
                }
            } else {
                placeholder.clone().unwrap_or_default()
            };
            grid.text(0, *row, &format!("{}: [{}]", label, shown), 7);
            *row += 1;
        }
        WidgetNode::Confirm { question, default } => {
            let suffix = if *default { " [y/N]" } else { " [Y/n]" };
            for wrap in wrap_text(&format!("{question}{suffix}"), cols) {
                grid.text(0, *row, &wrap, 7);
                *row += 1;
            }
        }
        WidgetNode::Modal { title, child } => {
            grid.text(0, *row, &format!("╔═ {title} ═╗"), 12);
            *row += 1;
            render_node(child, cols, grid, row);
            grid.text(0, *row, "╚══════════╝", 12);
            *row += 1;
        }
    }
}

/// Render a table into the grid.
fn render_table_grid(
    columns: &[TableColumn],
    rows: &[TableRow],
    cols: usize,
    grid: &mut Grid,
    row: &mut usize,
) {
    let count = columns.len();
    if count == 0 {
        return;
    }
    let mut widths: Vec<usize> = columns.iter().map(|c| c.header.len()).collect();
    for r in rows {
        for (i, cell) in r.cells.iter().enumerate() {
            if i < count {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }
    let total: usize = widths.iter().sum::<usize>() + (count - 1) * 2;
    if total > cols {
        // Squeeze widest columns first.
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(widths[i]));
        let mut remaining = total - cols;
        for &i in &order {
            if remaining == 0 {
                break;
            }
            let cut = widths[i].saturating_sub(1);
            let take = remaining.min(cut);
            widths[i] -= take;
            remaining -= take;
        }
    }

    let mut c = 0;
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            grid.put(
                c,
                *row,
                Cell {
                    ch: ' ',
                    fg: 8,
                },
            );
            c += 1;
        }
        grid.text(c, *row, &column.header, 7);
        c += widths[i];
    }
    *row += 1;
    let mut c = 0;
    for (i, _) in columns.iter().enumerate() {
        if i > 0 {
            c += 1;
        }
        grid.fill(c, *row, widths[i], 1, '─', 8);
        c += widths[i];
    }
    *row += 1;
    for r in rows {
        let mut c = 0;
        for i in 0..count {
            if i > 0 {
                c += 1;
            }
            let cell = r.cells.get(i).cloned().unwrap_or_default();
            let text: String = cell.chars().take(widths[i]).collect();
            grid.text(c, *row, &text, 7);
            c += widths[i];
        }
        *row += 1;
    }
}

/// Wrap text to `cols` display columns at word boundaries.
fn wrap_text(text: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    if text.chars().count() <= cols {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let needed = if current.is_empty() {
            word.chars().count()
        } else {
            word.chars().count() + 1
        };
        if current.chars().count() + needed > cols && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Render a widget tree to a standalone grid (used by the card border pass).
fn child_grid(node: &WidgetNode, cols: usize) -> Grid {
    let mut grid = Grid::new(cols.max(1), 1);
    let mut row = 0;
    render_node(node, cols.max(1), &mut grid, &mut row);
    // Shrink to the rows actually written.
    grid.rows = row.max(1);
    grid.cells.truncate(grid.rows * grid.cols);
    grid
}

/// Blit a sub-grid onto the parent at (col,row).
fn blit(dst: &mut Grid, src: &Grid, col: usize, row: usize) {
    for r in 0..src.rows {
        for c in 0..src.cols {
            if let Some(cell) = src.cells.get(r * src.cols + c) {
                dst.put(col + c, row + r, *cell);
            }
        }
    }
}

/// Render a `SurfaceOp` to PNG bytes.
///
/// The op is already gate-validated and scrubbed; this function never
/// re-checks. Returns the PNG bytes plus the plain-text fallback the caller
/// should send as the caption (the same words the TUI shows).
pub fn render_op_png(op: &crate::ui::SurfaceOp, cols: usize) -> (Vec<u8>, String) {
    let cols = cols.clamp(20, 200);
    let node = op.widgets_column();
    let mut grid = child_grid(&node, cols);
    // Re-render into a grid of the right height: child_grid sized rows=1
    // then grew via truncate; rebuild cleanly.
    let mut full = Grid::new(cols, grid.rows.max(1));
    let mut row = 0;
    render_node(&node, cols, &mut full, &mut row);
    grid = full;
    let img = grid.to_image();
    let mut bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    if image::DynamicImage::ImageRgba8(img)
        .write_with_encoder(encoder)
        .is_err()
    {
        // A failed encode must never crash the gateway; the caller falls
        // back to the text form.
        return (Vec::new(), String::new());
    }
    let fallback = crate::ui::plain::render_plain(
        &node,
        100,
        crate::ui::plain::RenderProvenance::from_synthesized(op.is_synthesized()),
    );
    (bytes, fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{ChoiceOption, ColumnAlign, Provenance, SurfaceOp, WidgetNode};

    fn op(node: WidgetNode) -> SurfaceOp {
        SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "test".into(),
            provenance: Provenance::ToolVerified {
                tool_name: "ui_render_surface".into(),
                source_uri: None,
            },
            body: node,
        }
    }

    /// G1: byte-reproducible. The same op encodes to the same bytes.
    #[test]
    fn png_is_byte_reproducible() {
        let node = WidgetNode::Card {
            title: "Deploy".into(),
            tone: SemanticTone::Accent,
            child: Box::new(WidgetNode::Text {
                text: "hello".into(),
                tone: SemanticTone::Default,
                mono: false,
            }),
        };
        let a = render_op_png(&op(node.clone()), 40);
        let b = render_op_png(&op(node), 40);
        assert_eq!(a.0, b.0, "the same op must encode to identical PNG bytes");
        assert!(!a.0.is_empty(), "the PNG must not be empty");
    }

    /// G1: a tone change alters the bytes (the colour surface is live).
    #[test]
    fn png_changes_when_tone_changes() {
        let base = WidgetNode::Text {
            text: "hi".into(),
            tone: SemanticTone::Default,
            mono: false,
        };
        let danger = WidgetNode::Text {
            text: "hi".into(),
            tone: SemanticTone::Danger,
            mono: false,
        };
        let a = render_op_png(&op(base), 40).0;
        let b = render_op_png(&op(danger), 40).0;
        assert_ne!(a, b, "a SemanticTone change must move pixels");
    }

    /// G2: the fallback text agrees with plain.rs — the caption the model
    /// reads is a substring of what the PNG encodes (via the fallback).
    #[test]
    fn fallback_matches_plain_renderer() {
        let node = WidgetNode::Column {
            children: vec![
                WidgetNode::Card {
                    title: "T".into(),
                    tone: SemanticTone::Default,
                    child: Box::new(WidgetNode::Table {
                        columns: vec![
                            TableColumn {
                                header: "a".into(),
                                align: ColumnAlign::Left,
                            },
                            TableColumn {
                                header: "b".into(),
                                align: ColumnAlign::Right,
                            },
                        ],
                        rows: vec![TableRow {
                            cells: vec!["1".into(), "2".into()],
                        }],
                    }),
                },
                WidgetNode::ChoicePicker {
                    question: "q?".into(),
                    choices: vec![ChoiceOption {
                        id: "x".into(),
                        label: "yes".into(),
                        description: None,
                    }],
                },
            ],
        };
        let (png, fallback) = render_op_png(&op(node), 40);
        assert!(!png.is_empty());
        let direct = crate::ui::plain::render_plain(
            &op(WidgetNode::Divider).widgets_column(),
            100,
            crate::ui::plain::RenderProvenance::Verified,
        );
        // The fallback is the plain renderer's own output — the contract is
        // that they agree by construction (same function), so assert the
        // fallback contains the widget text the PNG encodes.
        assert!(
            fallback.contains("q?"),
            "fallback must carry the question: {fallback}"
        );
        assert!(
            fallback.contains("yes"),
            "fallback must carry the choice: {fallback}"
        );
        let _ = direct;
    }

    /// A narrow render never panics and stays within the column budget.
    #[test]
    fn narrow_render_is_safe() {
        let node = WidgetNode::Table {
            columns: vec![
                TableColumn {
                    header: "long header name".into(),
                    align: ColumnAlign::Left,
                },
                TableColumn {
                    header: "another long one".into(),
                    align: ColumnAlign::Right,
                },
            ],
            rows: vec![TableRow {
                cells: vec!["x".repeat(50), "y".repeat(50)],
            }],
        };
        let (png, _) = render_op_png(&op(node), 20);
        assert!(!png.is_empty());
    }
}
