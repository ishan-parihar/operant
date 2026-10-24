//! Fuzzy path scoring for the `@` mention popup (plan 2026-10-24, M2).
//!
//! Port of oh-my-pi's two scoring layers:
//! - `subsequenceScore` (autocomplete.ts): exact 100 / prefix 80 / includes
//!   60 / subsequence `40 − 5·gaps`, plus the ranked-tier subsequence test.
//! - `score_fuzzy_path` (pi-natives/fd.rs): try the FILENAME first, fall
//!   back to the whole path, `+10` for directories, shallower paths win
//!   ties, and the top-K is kept in a worst-first heap so a walk can stop
//!   once the cap is full and no remaining candidate could out-score the
//!   current worst.
//!
//! Scoring is pure and allocation-light: matching runs over `&str` slices
//! of the entry's already-`String` path.

/// Bonus added to directory matches (OMP `if is_directory && score > 0`).
pub(super) const DIR_BONUS: i32 = 10;

/// One scored candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoredEntry {
    /// Match quality; higher is better. `0` = no match.
    pub score: i32,
    /// Depth from the walk root (tiebreak: shallower wins).
    pub depth: usize,
    /// Path relative to the walk root.
    pub rel_path: String,
    pub is_dir: bool,
}

/// Subsequence score against `target` (OMP `subsequenceScore`):
/// exact 100, prefix 80, substring 60, subsequence `max(1, 40 − 5·gaps)`.
fn subsequence_score(query: &str, target: &str) -> i32 {
    if query.is_empty() {
        return 1;
    }
    if target == query {
        return 100;
    }
    if target.starts_with(query) {
        return 80;
    }
    if target.contains(query) {
        return 60;
    }
    // Subsequence match — tighter (fewer target chars skipped between
    // matches) wins. A "gap" is a run of skipped target chars between two
    // consecutive query-char matches (OMP counts `ti - lastMatchIdx > 1`).
    let mut qi = query.chars();
    let Some(mut expected) = qi.next() else {
        return 1;
    };
    let mut gaps = 0;
    let mut matched_any = false;
    let mut last_match_idx: Option<usize> = None;
    for (ti, tc) in target.char_indices() {
        if tc == expected {
            if let Some(last) = last_match_idx
                && ti > last + tc.len_utf8()
            {
                gaps += 1;
            }
            matched_any = true;
            last_match_idx = Some(ti);
            if let Some(next) = qi.next() {
                expected = next;
            } else {
                break;
            }
        }
    }
    if !matched_any || qi.next().is_some() {
        return 0; // query not fully consumed: not a subsequence
    }
    (40 - gaps * 5).max(1)
}

/// Whether `query` is a subsequence of `target` (OMP `subsequenceMatch`).
pub fn subsequence_match(query: &str, target: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut qi = query.chars().peekable();
    for ti in target.chars() {
        if qi.peek() == Some(&ti) {
            qi.next();
            if qi.peek().is_none() {
                return true;
            }
        }
    }
    false
}

/// Score one path against `query` (both lowercase, `/`-separated, relative).
/// Filename matches dominate; a whole-path subsequence is the weak tier.
fn score_path(query: &str, rel_path: &str, is_dir: bool, depth: usize) -> i32 {
    let filename = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let mut score = subsequence_score(query, filename);
    if score == 0 {
        // Weak tier: the query matches only spread across the path
        // (`scrma` ~ `src/main.rs`). Capped under the filename-subsequence
        // floor so a filename hit always beats a path hit.
        let path_score = if subsequence_match(query, rel_path) {
            30
        } else {
            0
        };
        score = path_score;
    }
    if score > 0 && is_dir {
        score += DIR_BONUS;
    }
    let _ = depth; // depth is a tiebreak, not a score input
    score
}

/// Rank `entries` against `query`, keeping the best `limit`. Ties prefer
/// shallower paths, then lexicographic order (OMP `cmp_candidate`).
pub fn rank_entries(
    entries: &[super::walker::DiscoveredEntry],
    query: &str,
    limit: usize,
) -> Vec<ScoredEntry> {
    let query_lower = query.to_lowercase();
    let mut scored: Vec<ScoredEntry> = entries
        .iter()
        .map(|e| ScoredEntry {
            score: score_path(&query_lower, &e.rel_path.to_lowercase(), e.is_dir, e.depth),
            depth: e.depth,
            rel_path: e.rel_path.clone(),
            is_dir: e.is_dir,
        })
        .filter(|s| s.score > 0)
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.depth.cmp(&b.depth))
            .then(a.rel_path.cmp(&b.rel_path))
    });
    scored.truncate(limit);
    scored
}

#[cfg(test)]
mod score_tests {
    use super::*;
    use crate::tui::file_discovery::walker::DiscoveredEntry;

    fn entry(rel: &str, is_dir: bool, depth: usize) -> DiscoveredEntry {
        DiscoveredEntry {
            rel_path: rel.to_string(),
            is_dir,
            depth,
        }
    }

    #[test]
    fn tiers_exact_prefix_includes_subsequence() {
        assert_eq!(subsequence_score("main", "main"), 100);
        assert_eq!(subsequence_score("ma", "main.rs"), 80);
        assert_eq!(subsequence_score("in", "main.rs"), 60);
        assert!(subsequence_score("mr", "main.rs") >= 1, "subsequence tier");
        assert_eq!(subsequence_score("zzz", "main.rs"), 0);
    }

    #[test]
    fn filename_beats_path_subsequence() {
        let ranked = rank_entries(
            &[
                entry("src/main.rs", false, 2),
                entry("some/mr/dir.txt", false, 3),
            ],
            "mr",
            10,
        );
        assert_eq!(
            ranked.first().map(|s| s.rel_path.as_str()),
            Some("src/main.rs")
        );
    }

    #[test]
    fn directory_bonus_and_depth_tiebreak() {
        // Walk entries never carry a trailing slash for dirs — the popup
        // appends it for display/insert only.
        let ranked = rank_entries(
            &[
                entry("docs/a/readme.md", false, 3),
                entry("readme", true, 1),
                entry("readme.md", false, 1),
            ],
            "readme",
            10,
        );
        // `readme` dir (100 + 10) beats `readme.md` (100) beats the deep md.
        assert_eq!(ranked[0].rel_path, "readme");
        assert!(ranked[0].is_dir);
        assert_eq!(ranked[1].rel_path, "readme.md");
    }

    #[test]
    fn limit_truncates_best_first() {
        let entries: Vec<_> = (0..20)
            .map(|i| entry(&format!("f{i}.txt"), false, 1))
            .collect();
        let ranked = rank_entries(&entries, "f1", 3);
        assert_eq!(ranked.len(), 3);
    }

    #[test]
    fn subsequence_match_basics() {
        // s(0) m(4) a(5) r(8) — spread across the path's components.
        assert!(subsequence_match("smar", "src/main.rs"));
        assert!(!subsequence_match("zz", "src/main.rs"));
        assert!(subsequence_match("", "anything"));
    }
}
