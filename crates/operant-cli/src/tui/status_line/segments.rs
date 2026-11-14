//! The status-line segment model (plan 2026-11-14 U8).
//!
//! `draw_status` builds its bar as one imperative `Line`: 51 hand-written
//! ` · ` joins across a dozen `ProcessingStatus` branches. That cannot be
//! reflowed, cannot drop a segment when the terminal is narrow, and cannot be
//! configured. This module is what it is being moved onto.
//!
//! The port is the smallest shape that carries OMP's three ideas:
//! - **segments are data** ([`Segment`]), not spans built in place;
//! - **a preset names the segments and the separator** ([`Preset`]), so the bar
//!   is described rather than assembled;
//! - **overflow is an ordered degrade** ([`fit`]), not a truncation: shrink the
//!   elastic segments, drop the least important ones, and never drop the path
//!   first — that is OMP's rule (`status-line/component.ts:3245-3320`) and it
//!   exists because a naive right-to-left pop once collapsed a whole bar to the
//!   model segment.

// The whole module is U8's raw material and is not yet drawn: `draw_status`
// still assembles its bar by hand, and converting it is a separate rewiring.
// Everything here is complete and tested so that rewiring is only a rewiring.
#![allow(dead_code)]

use crate::tui::status_line::separators::SeparatorStyle;

/// One piece of the bar.
///
/// `elastic` marks the segments that may be shrunk before anything is dropped:
/// OMP has exactly two, the session name on the right and the path on the left,
/// because they are the only ones whose content is a truncation of something
/// longer rather than a fixed token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Stable identifier, matching OMP's `STATUS_LINE_SEGMENT_IDS` where an
    /// equivalent exists.
    pub id: &'static str,
    /// The rendered text.
    pub text: String,
    /// May be shortened to `min_width` before any segment is dropped.
    pub elastic: bool,
    /// The floor an elastic segment shrinks to.
    pub min_width: usize,
}

impl Segment {
    /// A fixed-width segment: rendered as-is, dropped rather than shrunk.
    pub fn fixed(id: &'static str, text: impl Into<String>) -> Self {
        Self {
            id,
            text: text.into(),
            elastic: false,
            min_width: 0,
        }
    }

    /// An elastic segment with a shrink floor.
    pub fn elastic(id: &'static str, text: impl Into<String>, min_width: usize) -> Self {
        Self {
            id,
            text: text.into(),
            elastic: true,
            min_width,
        }
    }

    /// Display width of the current text.
    ///
    /// Character count, not grapheme clusters: the bar's contents are ASCII
    /// plus a handful of known-wide glyphs, and pulling in a width table for a
    /// status row is not worth the dependency.
    pub fn width(&self) -> usize {
        self.text.chars().count()
    }

    /// Shorten to `width` characters, marking the cut with an ellipsis when it
    /// actually cut.
    pub fn truncate_to(&mut self, width: usize) {
        if self.width() <= width {
            return;
        }
        // Reserve one cell for the ellipsis, and never go below the floor.
        let keep = width.saturating_sub(1).max(self.min_width.min(width));
        let mut shortened: String = self.text.chars().take(keep).collect();
        shortened.push('…');
        self.text = shortened;
    }
}

/// A named bar layout, transcribed from `status-line/presets.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// Config key.
    pub key: &'static str,
    /// Segment ids, in render order.
    pub segments: &'static [&'static str],
    /// How segments are joined.
    pub separator: SeparatorStyle,
}

impl Preset {
    /// Every preset, in picker order.
    ///
    /// `custom` is absent: it is the operator's own list, which arrives from
    /// config rather than from this table, and inventing a default for it here
    /// would silently override what they wrote.
    pub const ALL: &'static [Preset] = &[
        Preset {
            key: "default",
            segments: &["vim", "model", "mode", "path", "git", "context_pct"],
            separator: SeparatorStyle::PowerlineThin,
        },
        Preset {
            key: "minimal",
            segments: &["vim", "path", "git"],
            separator: SeparatorStyle::Slash,
        },
        Preset {
            key: "compact",
            segments: &["vim", "model", "mode", "git"],
            separator: SeparatorStyle::PowerlineThin,
        },
        Preset {
            key: "full",
            segments: &["vim", "model", "mode", "path", "git", "context_pct", "time"],
            separator: SeparatorStyle::Powerline,
        },
        Preset {
            key: "ascii",
            segments: &["vim", "model", "mode", "path", "git"],
            separator: SeparatorStyle::Ascii,
        },
    ];

    /// Look up a preset by key, falling back to `default` — the same degrade
    /// OMP's `getPreset` uses.
    pub fn by_key(key: &str) -> &'static Preset {
        let normalized = key.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        Preset::ALL
            .iter()
            .find(|preset| preset.key == normalized)
            .unwrap_or(&Preset::ALL[0])
    }
}

/// Fit `segments` into `width` columns, degrading in OMP's order.
///
/// The ladder, and why each rung is where it is:
/// 1. **Shrink the elastic segments** (session name, then path) toward their
///    floors. Cheapest: no information is lost, only length.
/// 2. **Drop the last segment.** The bar's tail is its least important content
///    by construction — callers append in priority order.
/// 3. **Never drop `path` first.** OMP calls this out explicitly
///    (`component.ts:3300-3310`): a naive pop could hide the working directory
///    while keeping the model name, which is the one segment the operator is
///    least likely to need in order to know where they are.
///
/// Returns the segments that fit, in order. An empty result means not even one
/// segment fits, and the caller should render nothing rather than a bar that
/// claims to be complete.
pub fn fit(segments: &[Segment], width: usize) -> Vec<Segment> {
    let mut kept: Vec<Segment> = segments.to_vec();
    if kept.is_empty() {
        return kept;
    }

    // Rung 1: shrink elastic segments, path before session name (OMP shrinks
    // the right-hand title first, then the path).
    for elastic_id in ["session_name", "path"] {
        let Some(index) = kept.iter().position(|segment| segment.id == elastic_id) else {
            continue;
        };
        let Some(overflow) = kept
            .iter()
            .map(Segment::width)
            .sum::<usize>()
            .checked_sub(width)
            .filter(|overflow| *overflow > 0)
        else {
            continue;
        };
        let current = kept[index].width();
        let floor = kept[index].min_width.min(current);
        let shrinkable = current.saturating_sub(floor);
        if shrinkable > 0 {
            kept[index].truncate_to(current - shrinkable.min(overflow));
        }
    }

    // Rung 2 and 3: drop from the end, but step over `path`.
    while kept.iter().map(Segment::width).sum::<usize>() > width && kept.len() > 1 {
        let drop = kept
            .iter()
            .rposition(|segment| segment.id != "path")
            .unwrap_or(kept.len() - 1);
        kept.remove(drop);
    }

    // A single segment that is still too wide gets truncated rather than
    // overflowing the row.
    if kept.len() == 1 && kept[0].width() > width {
        kept[0].truncate_to(width);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments() -> Vec<Segment> {
        vec![
            Segment::fixed("vim", "NORMAL"),
            Segment::fixed("model", "claude-opus-4"),
            Segment::elastic("path", "/home/ishanp/a/very/deep/project", 8),
            Segment::fixed("git", "main*"),
        ]
    }

    /// A bar that already fits must come back untouched — the degrade is for
    /// overflow only.
    #[test]
    fn a_bar_that_fits_is_returned_untouched() {
        let input = segments();
        assert_eq!(fit(&input, 200), input);
    }

    /// Rung 1: an elastic segment shrinks before anything is dropped.
    #[test]
    fn an_elastic_segment_shrinks_before_anything_is_dropped() {
        let fitted = fit(&segments(), 46);
        assert_eq!(
            fitted.len(),
            4,
            "nothing may be dropped while shrinking is still possible"
        );
        let path = fitted.iter().find(|s| s.id == "path").expect("path kept");
        assert!(path.width() < "/home/ishanp/a/very/deep/project".len());
        assert!(path.text.ends_with('…'), "a cut path says so");
    }

    /// Rung 2: once the elastic segment is at its floor, segments drop from the
    /// end.
    #[test]
    fn segments_drop_from_the_end_once_shrinking_is_exhausted() {
        let fitted = fit(&segments(), 30);
        assert!(
            fitted.len() < 4,
            "a bar that cannot shrink enough must drop segments"
        );
        assert_eq!(fitted.last().map(|s| s.id), Some("path"));
    }

    /// Rung 3, the one OMP had to learn the hard way: the path survives longer
    /// than the segments after it.
    #[test]
    fn the_path_is_the_last_segment_to_go() {
        let fitted = fit(&segments(), 20);
        assert!(
            fitted.iter().any(|segment| segment.id == "path"),
            "the working directory must outlast the model and git segments: {fitted:?}"
        );
    }

    /// A width so small that nothing meaningful fits still yields the path
    /// rather than an empty bar claiming to be complete.
    #[test]
    fn the_path_survives_a_bar_narrower_than_every_segment() {
        let fitted = fit(&segments(), 12);
        assert_eq!(fitted.len(), 1);
        assert_eq!(fitted[0].id, "path");
    }

    /// An empty input must not panic or invent a segment.
    #[test]
    fn no_segments_yields_no_segments() {
        assert!(fit(&[], 80).is_empty());
    }

    /// Every preset's separator must be a real style, and `default` must be
    /// reachable through the same key path config uses.
    #[test]
    fn every_preset_resolves_by_its_key() {
        for preset in Preset::ALL {
            assert_eq!(Preset::by_key(preset.key).key, preset.key);
            assert!(
                !preset.segments.is_empty(),
                "{} has no segments",
                preset.key
            );
        }
        assert_eq!(Preset::by_key("no-such-preset").key, "default");
    }
}
