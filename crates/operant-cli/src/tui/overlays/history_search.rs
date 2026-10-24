// overlays/history_search.rs — Ctrl+R history search floating panel.
//
// Extracted from the overlays.rs monolith. Includes HistoryEntry,
// pinned-entry persistence (~/.operant/history_pins.json), fuzzy
// subsequence scoring, the HistorySearchOverlay state machine, and
// the floating-panel renderer was retired: the prompt's inline history bar
// (`operant_ui::ui_input::draw_prompt_history_search_overlay`) renders it.

// ============================================================================
// HistorySearchOverlay
// ============================================================================

// ---------------------------------------------------------------------------
// HistoryEntry
// ---------------------------------------------------------------------------

/// A single history entry and its pinned state.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub text: String,
    /// Whether this entry has been pinned by the user.  Pinned entries always
    /// appear at the top of the history overlay list and are persisted to
    /// `~/.operant/history_pins.json`.
    pub pinned: bool,
}

impl HistoryEntry {
    /// An unpinned entry.
    pub fn legacy(text: String) -> Self {
        Self {
            text,
            pinned: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Pinned-entry persistence  (~/.operant/history_pins.json)
// ---------------------------------------------------------------------------

fn pins_path() -> std::path::PathBuf {
    operant_core::platform::operant_home().join("history_pins.json")
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
    // Positions come from `folded_find` in ORIGINAL-target coordinates —
    // offsets found in the lowercased copy are not byte-aligned in the
    // original when a char's fold changes its length (R48-3).
    if let Some((m_start, m_end)) = super::fold::folded_find(target, query) {
        let position_bonus = 1.0 / (1.0 + m_start as f32);
        let score = 1.0 + position_bonus;
        // Matched positions are the contiguous byte range [m_start, m_end)
        let positions: Vec<usize> = (m_start..m_end).collect();
        return Some((score, positions));
    }

    // --- Subsequence path ---------------------------------------------------
    let q_chars: Vec<char> = q_lc.chars().collect();
    let t_chars: Vec<char> = t_lc.chars().collect();

    let mut q_pos = 0usize;
    // Map: folded-char index -> byte offset in the ORIGINAL target. The
    // folded text can hold MORE chars than the target (İ folds to two), so
    // each folded char records the byte offset of the original char that
    // produced it — every entry is a real original char boundary (R48-3).
    let t_byte_offsets: Vec<usize> = {
        let mut out = Vec::with_capacity(t_chars.len());
        for (b, ch) in target.char_indices() {
            for _ in ch.to_lowercase() {
                out.push(b);
            }
        }
        out
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

impl HistorySearchOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open with a `&[String]` slice (legacy callers).  All entries are
    /// treated as legacy (no timestamp).
    pub fn open(history: &[String]) -> Self {
        Self::open_with_entries(newest_first_unique(history))
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
                    })
                } else {
                    subsequence_score(&q, &entry.text).map(|(score, _)| MatchEntry {
                        snapshot_idx: i,
                        score,
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
        let pinned_texts = load_pinned_texts();
        self.snapshot = newest_first_unique(history)
            .into_iter()
            .map(|mut e| {
                e.pinned = pinned_texts.contains(&e.text);
                e
            })
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

/// History (oldest → newest) as search entries newest-first, each prompt
/// once at its most recent position — Ctrl+R preselects the latest match,
/// and score ties keep recency order (the sort is stable).
fn newest_first_unique(history: &[String]) -> Vec<HistoryEntry> {
    let mut seen = std::collections::HashSet::new();
    history
        .iter()
        .rev()
        .filter(|text| seen.insert(text.as_str()))
        .map(|text| HistoryEntry::legacy(text.clone()))
        .collect()
}
