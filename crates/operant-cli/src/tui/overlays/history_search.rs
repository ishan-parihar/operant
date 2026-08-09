// overlays/history_search.rs — Ctrl+R history search floating panel.
//
// Extracted from the overlays.rs monolith. Includes HistoryEntry,
// pinned-entry persistence (~/.operant/history_pins.json), fuzzy
// subsequence scoring, the HistorySearchOverlay state machine, and
// the floating-panel renderer.

use super::*;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

// ============================================================================
// HistorySearchOverlay
// ============================================================================

// ---------------------------------------------------------------------------
// HistoryEntry — wrapper with optional timestamp
// ---------------------------------------------------------------------------

pub(crate) fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A single history entry with an optional Unix timestamp and pinned state.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub text: String,
    /// Unix timestamp (seconds since epoch) when this entry was recorded.
    /// `None` for legacy entries without timestamps.
    pub timestamp: Option<u64>,
    /// Whether this entry has been pinned by the user.  Pinned entries always
    /// appear at the top of the history overlay list and are persisted to
    /// `~/.operant/history_pins.json`.
    pub pinned: bool,
}

impl HistoryEntry {
    /// Create a legacy entry without a timestamp.
    pub fn legacy(text: String) -> Self {
        Self {
            text,
            timestamp: None,
            pinned: false,
        }
    }

    /// Human-readable relative time: "just now", "2m ago", "3h ago", "2d ago", etc.
    pub fn relative_time(&self) -> String {
        let ts = match self.timestamp {
            None => return String::new(),
            Some(t) => t,
        };
        let now = current_unix_secs();
        let delta = now.saturating_sub(ts);
        if delta < 60 {
            "just now".to_string()
        } else if delta < 3600 {
            format!("{}m ago", delta / 60)
        } else if delta < 86400 {
            format!("{}h ago", delta / 3600)
        } else {
            format!("{}d ago", delta / 86400)
        }
    }
}

// ---------------------------------------------------------------------------
// Pinned-entry persistence  (~/.operant/history_pins.json)
// ---------------------------------------------------------------------------

fn pins_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".operant")
        .join("history_pins.json")
}

/// Load the set of pinned entry texts from `~/.operant/history_pins.json`.
/// Returns an empty set if the file does not exist or cannot be parsed.
pub fn load_pinned_texts() -> std::collections::HashSet<String> {
    let path = pins_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return std::collections::HashSet::new(),
    };
    serde_json::from_str::<std::collections::HashSet<String>>(&content).unwrap_or_default()
}

/// Persist `pinned_texts` to `~/.operant/history_pins.json`.
/// Failures are silently ignored (best-effort).
pub fn save_pinned_texts(pinned_texts: &std::collections::HashSet<String>) {
    let path = pins_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(pinned_texts) {
        let _ = std::fs::write(&path, json);
    }
}

// ---------------------------------------------------------------------------
// Fuzzy / subsequence matching
// ---------------------------------------------------------------------------

/// Compute a match score for `query` against `target`.
///
/// Fast path: if `target` contains `query` as a substring the score is
/// `1.0 + position_bonus` so it always beats a pure subsequence match.
///
/// Subsequence path: each character of `query` must appear in `target` in
/// order. The score is `consecutive_run_bonus + position_bonus` where
///   - `consecutive_run_bonus = longest_consecutive_run as f32 / query.len() as f32`
///   - `position_bonus       = 1.0 / (1.0 + first_match_position as f32)`
///
/// Returns `None` when `query` is neither a substring nor a subsequence of
/// `target`.
///
/// The returned `Vec<usize>` contains the byte indices in `target` that were
/// matched (useful for highlight rendering).
pub fn subsequence_score(query: &str, target: &str) -> Option<(f32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0.0, Vec::new()));
    }

    let q_lc = query.to_lowercase();
    let t_lc = target.to_lowercase();

    // --- Fast path: substring match (always wins over subsequence) ----------
    if let Some(pos) = t_lc.find(q_lc.as_str()) {
        let position_bonus = 1.0 / (1.0 + pos as f32);
        let score = 1.0 + position_bonus;
        // Matched positions are the contiguous byte range [pos, pos+q_lc.len())
        let positions: Vec<usize> = (pos..pos + q_lc.len()).collect();
        return Some((score, positions));
    }

    // --- Subsequence path ---------------------------------------------------
    let q_chars: Vec<char> = q_lc.chars().collect();
    let t_chars: Vec<char> = t_lc.chars().collect();

    let mut q_pos = 0usize;
    // Map: char index in t_chars -> byte offset in original target
    let t_byte_offsets: Vec<usize> = {
        let mut off = 0usize;
        t_chars
            .iter()
            .map(|c| {
                let o = off;
                off += c.len_utf8();
                o
            })
            .collect()
    };

    let mut matched_char_indices: Vec<usize> = Vec::with_capacity(q_chars.len());

    for (t_i, &tc) in t_chars.iter().enumerate() {
        if q_pos < q_chars.len() && tc == q_chars[q_pos] {
            matched_char_indices.push(t_i);
            q_pos += 1;
        }
    }

    if q_pos < q_chars.len() {
        // Not all query chars found in order
        return None;
    }

    // Compute longest consecutive run among matched char indices
    let mut max_run = 1usize;
    let mut cur_run = 1usize;
    for w in matched_char_indices.windows(2) {
        if w[1] == w[0] + 1 {
            cur_run += 1;
            if cur_run > max_run {
                max_run = cur_run;
            }
        } else {
            cur_run = 1;
        }
    }

    let q_len = q_chars.len() as f32;
    let consecutive_run_bonus = max_run as f32 / q_len;
    let first_match_pos = matched_char_indices[0];
    let position_bonus = 1.0 / (1.0 + first_match_pos as f32);
    let score = consecutive_run_bonus + position_bonus;

    let byte_positions: Vec<usize> = matched_char_indices
        .iter()
        .map(|&ci| t_byte_offsets[ci])
        .collect();

    Some((score, byte_positions))
}

// ---------------------------------------------------------------------------
// MatchEntry — scored match with highlight positions
// ---------------------------------------------------------------------------

/// One scored match result produced by `update_matches`.
#[derive(Debug, Clone)]
pub struct MatchEntry {
    /// Index of this entry in the `snapshot` held by `HistorySearchOverlay`.
    pub snapshot_idx: usize,
    pub score: f32,
    /// Byte positions in `entry.text` that were matched (for highlighting).
    pub highlight_positions: Vec<usize>,
}

// ---------------------------------------------------------------------------
// HistorySearchOverlay
// ---------------------------------------------------------------------------

/// State for the Ctrl+R history search floating panel.
#[derive(Debug, Default)]
pub struct HistorySearchOverlay {
    pub visible: bool,
    pub query: String,
    /// Scored, sorted matches.  `matches[i].snapshot_idx` is the index into
    /// `snapshot`.  `matches` is sorted best-score-first.
    pub matches: Vec<MatchEntry>,
    pub selected_idx: usize,
    /// Snapshot of the history taken at `open()` time, stored as
    /// `HistoryEntry` so timestamps are available.
    pub snapshot: Vec<HistoryEntry>,
}

/// Convenience accessor: the plain list of `snapshot_idx` values from
/// `matches`, in order.  Kept for callers that only need indices.
impl HistorySearchOverlay {
    #[allow(dead_code)] // History search match indices
    pub fn match_indices(&self) -> Vec<usize> {
        #[allow(dead_code)]
        self.matches.iter().map(|m| m.snapshot_idx).collect()
    }
}

impl HistorySearchOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open with a `&[String]` slice (legacy callers).  All entries are
    /// treated as legacy (no timestamp).
    pub fn open(history: &[String]) -> Self {
        let entries: Vec<HistoryEntry> = history
            .iter()
            .map(|s| HistoryEntry::legacy(s.clone()))
            .collect();
        Self::open_with_entries(entries)
    }

    /// Open with a pre-built `Vec<HistoryEntry>` (timestamp-aware callers).
    ///
    /// Pinned state is loaded from `~/.operant/history_pins.json` and applied
    /// to any matching entries.
    pub fn open_with_entries(entries: Vec<HistoryEntry>) -> Self {
        let pinned_texts = load_pinned_texts();
        let entries = entries
            .into_iter()
            .map(|mut e| {
                if pinned_texts.contains(&e.text) {
                    e.pinned = true;
                }
                e
            })
            .collect();
        let mut s = Self {
            visible: true,
            query: String::new(),
            matches: Vec::new(),
            selected_idx: 0,
            snapshot: entries,
        };
        s.recompute_matches();
        s
    }

    /// Toggle the pinned state of the currently selected entry.
    ///
    /// Persists the updated pin set to `~/.operant/history_pins.json` and
    /// recomputes the match list so the entry moves to/from the pinned section.
    pub fn toggle_pin(&mut self) {
        let Some(m) = self.matches.get(self.selected_idx) else {
            return;
        };
        let snap_idx = m.snapshot_idx;
        let Some(entry) = self.snapshot.get_mut(snap_idx) else {
            return;
        };
        entry.pinned = !entry.pinned;

        // Rebuild the persisted pin set from the full snapshot.
        let pinned_texts: std::collections::HashSet<String> = self
            .snapshot
            .iter()
            .filter(|e| e.pinned)
            .map(|e| e.text.clone())
            .collect();
        save_pinned_texts(&pinned_texts);

        // Recompute without moving selected_idx so the cursor stays stable.
        self.recompute_matches();
    }

    // ------------------------------------------------------------------
    // Internal scoring
    // ------------------------------------------------------------------

    fn recompute_matches(&mut self) {
        let q = self.query.to_lowercase();
        let mut scored: Vec<MatchEntry> = self
            .snapshot
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                if q.is_empty() {
                    Some(MatchEntry {
                        snapshot_idx: i,
                        score: 0.0,
                        highlight_positions: Vec::new(),
                    })
                } else {
                    subsequence_score(&q, &entry.text).map(|(score, positions)| MatchEntry {
                        snapshot_idx: i,
                        score,
                        highlight_positions: positions,
                    })
                }
            })
            .collect();

        // Sort: pinned entries always first, then by score descending.
        // Stable sort preserves insertion order for ties within each group.
        scored.sort_by(|a, b| {
            let a_pinned = self.snapshot.get(a.snapshot_idx).is_some_and(|e| e.pinned);
            let b_pinned = self.snapshot.get(b.snapshot_idx).is_some_and(|e| e.pinned);
            match (b_pinned, a_pinned) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => b
                    .score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            }
        });

        self.matches = scored;
        // Clamp selection
        if !self.matches.is_empty() && self.selected_idx >= self.matches.len() {
            self.selected_idx = self.matches.len() - 1;
        }
    }

    // ------------------------------------------------------------------
    // Public API — backward-compatible with &[String] callers
    // ------------------------------------------------------------------

    /// Recompute matches from the given `history` slice.
    ///
    /// This updates the internal snapshot and recomputes.  Callers that pass
    /// `&app.prompt_input.history` every time will continue to work unchanged.
    pub fn update_matches(&mut self, history: &[String]) {
        // Rebuild snapshot preserving existing timestamps where possible.
        // Simple strategy: replace snapshot with legacy entries from `history`.
        // (A more sophisticated approach would merge by text, but keeping it
        // simple avoids complexity and matches the current call-site pattern.)
        self.snapshot = history
            .iter()
            .map(|s| HistoryEntry::legacy(s.clone()))
            .collect();
        self.recompute_matches();
    }

    pub fn push_char(&mut self, c: char, history: &[String]) {
        self.query.push(c);
        self.selected_idx = 0;
        self.update_matches(history);
    }

    pub fn pop_char(&mut self, history: &[String]) {
        self.query.pop();
        self.selected_idx = 0;
        self.update_matches(history);
    }

    pub fn select_prev(&mut self) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        if self.selected_idx == 0 {
            self.selected_idx = count - 1;
        } else {
            self.selected_idx -= 1;
        }
    }

    pub fn select_next(&mut self) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.selected_idx = (self.selected_idx + 1) % count;
    }

    /// Return the currently selected history entry text, if any.
    ///
    /// The `history` parameter is accepted for backward compatibility but the
    /// overlay uses its internal snapshot.  If `history` is non-empty it is
    /// used as a fallback when the snapshot is empty.
    pub fn current_entry<'a>(&self, history: &'a [String]) -> Option<&'a str> {
        let snap_idx = self.matches.get(self.selected_idx)?.snapshot_idx;
        // Try the history slice first (keeps existing call-sites working).
        history.get(snap_idx).map(String::as_str)
    }

    pub fn close(&mut self) {
        self.visible = false;
    }
}

/// Render the history search floating panel.
pub fn render_history_search_overlay(
    frame: &mut Frame,
    overlay: &HistorySearchOverlay,
    history: &[String],
    area: Rect,
) {
    if !overlay.visible {
        return;
    }

    const VISIBLE_MATCHES: usize = 8;
    let dialog_width = 72u16.min(area.width.saturating_sub(4));
    let match_count = overlay.matches.len().max(1);
    let rows = VISIBLE_MATCHES.min(match_count) as u16;
    // +2 for blank separator + hint footer line, +2 for block borders
    let dialog_height = (6 + rows).min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    frame.render_widget(Clear, dialog_area);

    let mut lines: Vec<Line> = Vec::new();

    // --- Search query line ---------------------------------------------------
    let result_count_str = format!("{} results", overlay.matches.len());
    lines.push(Line::from(vec![
        Span::raw("  Search: "),
        Span::styled(
            overlay.query.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("\u{2588}", Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled(
            result_count_str,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ),
    ]));
    lines.push(Line::from(""));

    if overlay.matches.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  (no matches)",
            Style::default().fg(Color::DarkGray),
        )]));
    } else {
        let start = overlay
            .selected_idx
            .saturating_sub(VISIBLE_MATCHES / 2)
            .min(overlay.matches.len().saturating_sub(VISIBLE_MATCHES));
        let end = (start + VISIBLE_MATCHES).min(overlay.matches.len());

        for (display_i, match_entry) in overlay.matches[start..end].iter().enumerate() {
            let real_i = start + display_i;
            let is_selected = real_i == overlay.selected_idx;

            // Resolve snapshot entry (for text, timestamp, pinned state).
            let snap_entry: Option<&HistoryEntry> = overlay.snapshot.get(match_entry.snapshot_idx);

            // Resolve entry text: prefer snapshot, fall back to passed-in history.
            let entry_text: &str = snap_entry
                .map(|e| e.text.as_str())
                .or_else(|| history.get(match_entry.snapshot_idx).map(String::as_str))
                .unwrap_or("");

            let is_pinned = snap_entry.is_some_and(|e| e.pinned);

            // Relative timestamp (right-aligned suffix)
            let time_suffix: String = snap_entry
                .map(|e| {
                    let t = e.relative_time();
                    if t.is_empty() {
                        t
                    } else {
                        format!(" · {}", t)
                    }
                })
                .unwrap_or_default();

            // Pin star shown to the left of pinned entries: "★ " (2 chars wide)
            // Available width for the entry text
            let pin_prefix_width: usize = if is_pinned { 2 } else { 0 };
            let prefix_width: usize = 4 + pin_prefix_width; // "    " or "  ► " + optional "★ "
            let time_width = UnicodeWidthStr::width(time_suffix.as_str());
            let max_text_chars =
                (dialog_width as usize).saturating_sub(prefix_width + time_width + 2);

            let (prefix, base_style) = if is_selected {
                (
                    "  \u{25BA} ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("    ", Style::default().fg(Color::White))
            };

            // Build highlighted spans for the entry text
            let text_spans = build_highlighted_spans(
                entry_text,
                &match_entry.highlight_positions,
                max_text_chars,
                base_style,
                is_selected,
            );

            let mut row_spans: Vec<Span> = vec![Span::raw(prefix)];

            // Pin star badge (shown for all pinned entries)
            if is_pinned {
                row_spans.push(Span::styled(
                    "\u{2605} ", // ★
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            row_spans.extend(text_spans);
            if !time_suffix.is_empty() {
                row_spans.push(Span::styled(
                    time_suffix,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ));
            }

            lines.push(Line::from(row_spans));
        }
    }

    // Footer hint bar (below the match list)
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "  \u{2191}\u{2193} navigate  \u{00b7}  Enter select  \u{00b7}  p pin/unpin  \u{00b7}  Esc cancel",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" History Search ")
        .border_style(Style::default().fg(Color::Cyan));

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, dialog_area);
}

/// Build a list of `Span`s for `text`, highlighting the bytes at
/// `highlight_positions` in yellow. Text is truncated to `max_chars`.
fn build_highlighted_spans<'a>(
    text: &str,
    highlight_positions: &[usize],
    max_chars: usize,
    base_style: Style,
    _is_selected: bool,
) -> Vec<Span<'a>> {
    // Collect char-level info (byte offset, char)
    let chars: Vec<(usize, char)> = text.char_indices().collect();

    // Convert highlight byte-positions to a set of byte offsets for O(1) lookup
    let hl_set: std::collections::HashSet<usize> = highlight_positions.iter().copied().collect();

    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut current_text = String::new();
    let mut current_highlighted = false;
    let mut truncated = false;

    for (char_count, (byte_off, ch)) in chars.iter().enumerate() {
        if char_count >= max_chars {
            truncated = true;
            break;
        }
        let is_hl = hl_set.contains(byte_off);
        if is_hl != current_highlighted && !current_text.is_empty() {
            let style = if current_highlighted {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                base_style
            };
            spans.push(Span::styled(current_text.clone(), style));
            current_text.clear();
        }
        current_highlighted = is_hl;
        current_text.push(*ch);
    }
    if !current_text.is_empty() {
        let style = if current_highlighted {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            base_style
        };
        spans.push(Span::styled(current_text, style));
    }
    if truncated {
        spans.push(Span::styled(
            "…".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    spans
}
