// One-off characterisation of the per-frame colour-substitution pass.
//
// `render_app` calls `style::theme_mode::adapt_buffer_for_display(frame.buffer_mut())`
// as the LAST colour transform of every painted frame, over the whole cell buffer.
// Deliverable 1.10 of `docs/PLAN-TUI-OVERHAUL.md` asks what that costs, measured
// in `--release`, with an uncertainty figure rather than an estimate.
//
// This module is `#[cfg(test)]` and every test in it is `#[ignore]`, so it costs
// the normal build and the normal test run nothing. Run it explicitly:
//
//     ./scripts/check.sh test --release -p operant-cli --bin operant -- \
//         --ignored --nocapture --test-threads=1
//
// No `criterion`: this is a one-off characterisation, not a tracked benchmark
// suite, and adding a dependency to a workspace other agents are editing is not
// worth it. `std::time::Instant` plus `std::hint::black_box` is the whole tool.
//
// The header is `//` rather than `//!` so the whole file can be `include!`d
// verbatim by an out-of-tree measurement crate when this crate's test target is
// mid-edit by another agent (see the report for that run).

use super::vendor::style::STYLE_TEST_LOCK;
use super::vendor::style::palette::{ALL_ROLES, Palette, Role, set_palette};
use super::vendor::style::theme_mode::{ThemeMode, adapt_buffer_for_display, set_theme_mode};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use std::time::Instant;

/// Untimed iterations before the first timed repeat: first-touch page faults on
/// the buffer, the HashMap allocations, and the branch predictor warming up are
/// not what we are trying to measure.
const WARMUP: usize = 200;
/// Timed calls per repeat. A single 120x40 call is only a few tens of
/// microseconds, so one `Instant::now()` pair would be a large fraction of the
/// reading. Batching amortises the clock read; the per-call figure is the total
/// divided by the count.
const INNER: usize = 200;
/// Timed repeats per case. The plan asks for >=3; 7 gives a median that survives
/// one contended sample.
const REPEATS: usize = 7;

/// Frame budgets the report is stated against.
const FPS_30_NS: u128 = 33_333_333;
const FPS_60_NS: u128 = 16_666_667;

// ---------------------------------------------------------------------------
// Buffer fixture
// ---------------------------------------------------------------------------

/// A deterministic xorshift. Realistic colour mix without a `rand` dependency
/// (removed in iter-43) and without a run-to-run varying fixture, which would
/// make the numbers incomparable between repeats.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The colours a real frame actually carries, in the proportions it carries
/// them. A uniform buffer is an unrealistically easy cache-warm case: it would
/// collapse the substitution memo to one entry and understate the cost.
///
/// Composition, chosen from the real widget set the TUI paints:
///  - ~35% body text on `Reset`, mostly `UserText`/`AiText`/ad-hoc literals
///  - ~15% `Dim` separators, hints and gutters
///  - ~10% box-drawing borders (`─│╭╯┐└┘`) on a `Border` role line
///  - ~10% role accents (user / ai / tool / file link / warning / error / ...)
///  - ~10% named colours from the status bar, which take the
///    `remap_named_with` branch rather than literal role matching
///  - ~8% a `UserBg` user-message panel, and a `SelectionBg` reversed row
///  - ~12% `Indexed` colours, a few underlined, some bold/italic
fn fill_realistic(buf: &mut Buffer, indexed_fanout: usize) {
    let width = buf.area.width as usize;
    let height = buf.area.height as usize;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    let role = |r: Role| super::vendor::style::palette::role_color(r);
    let named: [Color; 8] = [
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::DarkGray,
        Color::Cyan,
        Color::Magenta,
        Color::LightBlue,
        Color::Gray,
    ];
    let ad_hoc: [(u8, u8, u8); 8] = [
        (57, 59, 66),
        (88, 91, 100),
        (140, 143, 152),
        (196, 199, 206),
        (34, 39, 47),
        (92, 96, 110),
        (70, 74, 88),
        (24, 27, 33),
    ];
    // 256-colour output in a real frame comes from a handful of terminals'
    // quantisation, not from 256 distinct codes. `indexed_fanout` is therefore
    // the memo size knob: 6 for a realistic frame, 256 for the pathological
    // ceiling. It matters because the substitution memo (and, in light mode,
    // the contrast-repair memo) is keyed on distinct COLOURS, not cells — so
    // this is the single variable the light-mode cost actually scales with.
    let indexed: Vec<Color> = (0..indexed_fanout)
        .map(|i| Color::Indexed(((i * 7) % 256) as u8))
        .collect();

    // Row band layout: header, body, footer. Mirrors `render_app`.
    let header_rows = 2usize.min(height);
    let footer_rows = 2usize.min(height.saturating_sub(header_rows));

    for y in 0..height {
        for x in 0..width {
            let cell = &mut buf.content[y * width + x];
            cell.bg = Color::Reset;
            cell.underline_color = Color::Reset;
            cell.modifier = Modifier::empty();

            let in_header = y < header_rows;
            let in_footer = y >= height - footer_rows;
            let in_body = !in_header && !in_footer;

            // Box-drawing columns / rows inside the body, so the border glyph
            // run is contiguous the way a real Block paints it.
            let border_col = in_body && (x == 0 || x == width - 1 || x == width / 2);
            let border_row = in_body && y == height / 3;

            if in_header {
                cell.set_symbol(if x == 0 { "◆" } else { " " });
                cell.fg = match x {
                    0 => role(Role::HeaderIcon),
                    1..=12 => role(Role::HeaderName),
                    13 => role(Role::Dim),
                    14 => role(Role::HeaderSession),
                    _ => role(Role::Dim),
                };
                if y == 1 {
                    cell.fg = role(Role::Dim);
                    cell.set_symbol("─");
                }
            } else if in_footer {
                cell.set_symbol(if x % 3 == 0 { "─" } else { " " });
                cell.fg = match x {
                    0..=2 => role(Role::Success),
                    3..=5 => role(Role::Warning),
                    6..=8 => role(Role::Error),
                    9 => role(Role::Info),
                    _ => role(Role::Dim),
                };
                if x % 7 == 0 {
                    cell.fg = named[rng.below(named.len())];
                }
            } else if border_col || border_row {
                cell.set_symbol(match (x, y) {
                    (0, _) | (_, 0) => "│",
                    _ if y == 0 => "─",
                    _ => "│",
                });
                cell.fg = role(Role::Border);
                cell.bg = if x < width / 2 {
                    Color::Reset
                } else {
                    role(Role::SelectionBg)
                };
            } else if in_body && x < width / 2 && (y % 7 == 3) {
                // A user-message panel: a run of tinted cells behind body text.
                cell.bg = role(Role::UserBg);
                cell.fg = role(Role::UserText);
                cell.set_symbol("a");
            } else {
                // Ordinary transcript text.
                let roll = rng.below(100);
                cell.fg = if roll < 12 {
                    role(Role::Dim)
                } else if roll < 26 {
                    role(Role::User)
                } else if roll < 40 {
                    role(Role::Ai)
                } else if roll < 50 {
                    role(Role::Tool)
                } else if roll < 58 {
                    role(Role::FileLink)
                } else if roll < 66 {
                    named[rng.below(named.len())]
                } else if roll < 74 {
                    role(Role::Warning)
                } else if roll < 80 {
                    role(Role::Error)
                } else if roll < 86 {
                    // 256-colour terminal output, e.g. an AFT status line.
                    indexed[rng.below(indexed.len())]
                } else if roll < 96 {
                    let (r, g, b) = ad_hoc[rng.below(ad_hoc.len())];
                    Color::Rgb(r, g, b)
                } else {
                    role(Role::AiText)
                };
                cell.set_symbol(match roll % 5 {
                    0 => " ",
                    1 => "x",
                    2 => "│",
                    3 => "╭",
                    _ => "·",
                });
                if roll % 17 == 0 {
                    cell.modifier.insert(Modifier::BOLD);
                }
                if roll % 23 == 0 {
                    cell.modifier.insert(Modifier::ITALIC);
                }
                if roll % 29 == 0 {
                    cell.underline_color = role(Role::Accent);
                }
            }

            // One reversed row, the selected-row idiom, which also forces the
            // `Modifier::REVERSED` branch of the light-theme path.
            if in_body && y == height / 2 {
                cell.modifier.insert(Modifier::REVERSED);
                cell.bg = role(Role::SelectionBg);
                cell.fg = role(Role::Accent);
                cell.set_symbol("▌");
            }
        }
    }
}

/// A frame whose colour cardinality matches a real one: ~30 distinct
/// foregrounds, 3 backgrounds, a few dozen (fg, bg) pairs.
fn realistic(w: u16, h: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
    fill_realistic(&mut buf, 6);
    buf
}

/// The pathological ceiling: a frame that uses a quarter of the 256-colour
/// cube. Included to show the cost driver is memo size rather than cell count,
/// so the realistic figure is not mistaken for the worst case.
fn wide_colour_fanout(w: u16, h: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
    fill_realistic(&mut buf, 256);
    buf
}

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

/// Per-call nanoseconds for each repeat, plus the fixture's colour cardinality
/// (the substitution memo's size, which is what the role-matching cost scales
/// with rather than the cell count).
struct Measurement {
    per_repeat: Vec<u64>,
    cells: usize,
    distinct_fg: usize,
    distinct_bg: usize,
    distinct_fg_bg: usize,
}

impl Measurement {
    fn min(&self) -> u64 {
        *self.per_repeat.iter().min().unwrap()
    }

    /// Lower median, so an even repeat count does not average the two middle
    /// samples.
    fn median(&self) -> u64 {
        let mut sorted = self.per_repeat.clone();
        sorted.sort_unstable();
        sorted[sorted.len() / 2]
    }

    /// Spread between the fastest and typical repeat, as a fraction of the
    /// median. This is the noise floor: a figure above ~20% means the machine
    /// was contended and the number is a 3-significant-figure claim at best.
    fn noise_floor_pct(&self) -> f64 {
        let (min, median) = (self.min() as f64, self.median() as f64);
        if median == 0.0 {
            return 0.0;
        }
        (median - min) / median * 100.0
    }

    fn ns_per_cell(&self, ns_per_call: u64) -> f64 {
        ns_per_call as f64 / self.cells as f64
    }
}

/// Time one case. `buf` is cloned outside the timed region — only the
/// substitution pass is inside, which is the number deliverable 1.10 asks for.
fn measure(buf: &Buffer) -> Measurement {
    let cells = buf.content.len();
    let mut distinct_fg: Vec<Color> = buf.content.iter().map(|c| c.fg).collect();
    distinct_fg.sort_by_key(|c| format!("{c:?}"));
    distinct_fg.dedup();
    let mut distinct_bg: Vec<Color> = buf.content.iter().map(|c| c.bg).collect();
    distinct_bg.sort_by_key(|c| format!("{c:?}"));
    distinct_bg.dedup();
    let mut distinct_pairs: Vec<(Color, Color)> =
        buf.content.iter().map(|c| (c.fg, c.bg)).collect();
    distinct_pairs.sort_by_key(|c| format!("{c:?}"));
    distinct_pairs.dedup();

    let mut scratch = buf.clone();
    for _ in 0..WARMUP {
        adapt_buffer_for_display(std::hint::black_box(&mut scratch));
    }

    let mut per_repeat = Vec::with_capacity(REPEATS);
    for _ in 0..REPEATS {
        // Re-seed from the pristine fixture so every repeat starts from the same
        // colour state. Cloning is deliberately outside the clock.
        let mut work = buf.clone();
        let start = Instant::now();
        for _ in 0..INNER {
            adapt_buffer_for_display(std::hint::black_box(&mut work));
        }
        let elapsed = start.elapsed().as_nanos();
        per_repeat.push((elapsed / INNER as u128) as u64);
    }

    Measurement {
        per_repeat,
        cells,
        distinct_fg: distinct_fg.len(),
        distinct_bg: distinct_bg.len(),
        distinct_fg_bg: distinct_pairs.len(),
    }
}

/// A palette as a user who has touched `/theme` would have: a handful of
/// roles retuned, not all 22.
fn configured_palette() -> Palette {
    let mut palette = Palette::default();
    palette.set(Role::User, (96, 165, 250));
    palette.set(Role::Ai, (74, 222, 128));
    palette.set(Role::Accent, (196, 181, 253));
    palette.set(Role::Border, (63, 63, 70));
    palette.set(Role::Error, (248, 113, 113));
    palette.set(Role::UserBg, (31, 41, 55));
    palette
}

fn report(label: &str, size: (u16, u16), m: &Measurement) {
    let min = m.min();
    let median = m.median();
    println!(
        "{label:<34} {:>3}x{:<3} {:>6} cells | \
         min {:>9.0} ns/call  {:>6.2} ns/cell | \
         median {:>9.0} ns/call  {:>6.2} ns/cell | \
         spread {:>5.1}% | \
         30fps {:>6.3}%  60fps {:>6.3}% | \
         distinct fg {} / bg {} / (fg,bg) {}",
        size.0,
        size.1,
        m.cells,
        min,
        m.ns_per_cell(min),
        median,
        m.ns_per_cell(median),
        m.noise_floor_pct(),
        min as f64 / FPS_30_NS as f64 * 100.0,
        min as f64 / FPS_60_NS as f64 * 100.0,
        m.distinct_fg,
        m.distinct_bg,
        m.distinct_fg_bg,
    );
}

// ---------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------

/// Deliverable 1.10. Run with `--release -- --ignored --nocapture`.
///
/// Three cases over two buffer sizes:
///
/// 1. **no-op** — dark mode, unconfigured palette. `configured_palette()`
///    returns `None` off the `HAS_OVERRIDES` atomic, so
///    `adapt_buffer_impl` early-returns and never walks the buffer. This is
///    the default state of every session that has not opened `/theme`.
/// 2. **configured / dark** — dark mode, palette with overrides. Full walk of
///    every cell, three memoised substitutions per cell (fg, bg,
///    underline_color), no luminance flip. The cost a user who *has*
///    configured colours pays.
/// 3. **configured / light** — light mode, palette with overrides. The walk
///    plus a hue flip plus `readable_light_foreground`'s 12-round bisection.
///    Note the memo that makes the bisection affordable is a LOCAL
///    `HashMap` rebuilt on every call, so the bisection re-runs once per
///    distinct (fg, bg) pair on every frame, not once per session.
#[test]
#[ignore = "one-off release-mode characterisation; prints, asserts nothing"]
fn measure_adapt_buffer_for_display_cost() {
    // The palette and theme mode are process-global. Serialise against the
    // other style tests, and always restore, so a failure cannot leak.
    let _lock = STYLE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            set_theme_mode(ThemeMode::Dark);
            set_palette(Palette::default());
        }
    }
    let _restore = Restore;

    println!();
    println!("adapt_buffer_for_display — per-frame colour substitution");
    println!(
        "warmup {WARMUP} · inner {INNER} · repeats {REPEATS} · \
         min and median of the repeats; 'spread' = (median-min)/median"
    );
    println!(
        "NOTE: min is the honest 'how fast can this go' figure and is what the \
         frame-budget\n      columns use; median is the typical observation."
    );
    println!();

    for size in [(120u16, 40u16), (200, 60)] {
        let buf = realistic(size.0, size.1);

        // 1. no-op: dark + unconfigured palette -> early return.
        set_theme_mode(ThemeMode::Dark);
        set_palette(Palette::default());
        report("1. no-op (dark, unconfigured)", size, &measure(&buf));

        // 2. configured palette, dark: full walk, substitution only.
        set_theme_mode(ThemeMode::Dark);
        set_palette(configured_palette());
        report("2. configured palette (dark)", size, &measure(&buf));

        // 3. configured palette, light: walk + flip + contrast repair.
        set_theme_mode(ThemeMode::Light);
        set_palette(configured_palette());
        report("3. configured palette (light)", size, &measure(&buf));

        // 4. light mode is the one that walks even with no palette configured:
        //    the default state of a light-terminal user who never opened
        //    /theme. Included because it is the case that decides whether the
        //    per-frame design is safe.
        set_theme_mode(ThemeMode::Light);
        set_palette(Palette::default());
        report("4. no palette, light (walk only)", size, &measure(&buf));

        println!();
    }

    // Ceiling case: same cell count, 256 distinct indexed colours. Isolates the
    // cost driver (memo size) from the one the plan was worried about (cells).
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let buf = wide_colour_fanout(200, 60);
        set_theme_mode(mode);
        set_palette(configured_palette());
        report(
            &format!("5. 256-colour fan-out ({mode:?})"),
            (200, 60),
            &measure(&buf),
        );
    }
    println!();

    // Sanity on the fixture and the setup, so a silently-degraded measurement
    // cannot be mistaken for a fast one.
    let buf = realistic(120, 40);
    let distinct_fg = {
        let mut v: Vec<Color> = buf.content.iter().map(|c| c.fg).collect();
        v.sort_by_key(|c| format!("{c:?}"));
        v.dedup();
        v.len()
    };
    assert!(
        distinct_fg > 12 && distinct_fg < 64 && ALL_ROLES.len() == 22,
        "fixture colour cardinality is not realistic: {distinct_fg} distinct foregrounds"
    );
    // The no-op path must be byte-identical, or case 1 is measuring a mutation
    // that production never performs.
    let mut untouched = buf.clone();
    set_theme_mode(ThemeMode::Dark);
    set_palette(Palette::default());
    adapt_buffer_for_display(&mut untouched);
    assert_eq!(untouched, buf, "no-op case mutated the frame");
    // ...and the configured path must actually substitute, or case 2 is
    // measuring the early return with extra steps.
    let mut recolored = buf.clone();
    set_palette(configured_palette());
    adapt_buffer_for_display(&mut recolored);
    assert_ne!(recolored, buf, "configured case performed no substitution");

    println!("fixture + setup assertions passed");
}
