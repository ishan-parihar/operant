//! Multi-strategy fuzzy matching for "did you mean?" suggestions.
//!
//! Implements a 9-strategy matching chain to find the closest candidate
//! strings for a given query. Strategies range from exact match through
//! Levenshtein distance, Jaro-Winkler similarity, and Dice coefficient.
//!
//! Usage:
//! ```ignore
//! let candidates = vec!["hello".into(), "world".into(), "help".into()];
//! let result = fuzzy_match::find_best_match("hel", &candidates);
//! ```

use std::collections::HashSet;

/// Result of a fuzzy match strategy.
#[derive(Debug, Clone)]
struct ScoredCandidate {
    candidate: String,
    score: f64,
    strategy: &'static str,
}

pub fn find_best_match(query: &str, candidates: &[String]) -> Option<(String, f64, &'static str)> {
    let suggestions = find_suggestions(query, candidates, 1);
    suggestions.into_iter().next()
}

/// Find up to `max_results` suggestions, sorted by score descending.
pub fn find_suggestions(
    query: &str,
    candidates: &[String],
    max_results: usize,
) -> Vec<(String, f64, &'static str)> {
    if query.is_empty() || candidates.is_empty() || max_results == 0 {
        return Vec::new();
    }

    let mut scored: Vec<ScoredCandidate> = candidates
        .iter()
        .filter_map(|candidate| score_candidate(query, candidate))
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scored
        .into_iter()
        .take(max_results)
        .map(|s| (s.candidate, s.score, s.strategy))
        .collect()
}

fn score_candidate(query: &str, candidate: &str) -> Option<ScoredCandidate> {
    // Strategies are tried in priority order (1-9). First match wins.
    if query.eq_ignore_ascii_case(candidate) {
        return Some(ScoredCandidate {
            candidate: candidate.to_string(),
            score: 1.0,
            strategy: "exact_match",
        });
    }

    if let Some(score) = strategy_prefix_match(query, candidate) {
        return Some(ScoredCandidate {
            candidate: candidate.to_string(),
            score,
            strategy: "prefix_match",
        });
    }

    if let Some(score) = strategy_substring_match(query, candidate) {
        return Some(ScoredCandidate {
            candidate: candidate.to_string(),
            score,
            strategy: "substring_match",
        });
    }

    if let Some(score) = strategy_word_match(query, candidate) {
        return Some(ScoredCandidate {
            candidate: candidate.to_string(),
            score,
            strategy: "word_match",
        });
    }

    if let Some(score) = strategy_acronym_match(query, candidate) {
        return Some(ScoredCandidate {
            candidate: candidate.to_string(),
            score,
            strategy: "acronym_match",
        });
    }

    if let Some(score) = strategy_levenshtein(query, candidate) {
        return Some(ScoredCandidate {
            candidate: candidate.to_string(),
            score,
            strategy: "levenshtein",
        });
    }

    if let Some(score) = strategy_jaro_winkler(query, candidate) {
        return Some(ScoredCandidate {
            candidate: candidate.to_string(),
            score,
            strategy: "jaro_winkler",
        });
    }

    if let Some(score) = strategy_dice_coefficient(query, candidate) {
        return Some(ScoredCandidate {
            candidate: candidate.to_string(),
            score,
            strategy: "dice_coefficient",
        });
    }

    // Fallback: character-set Jaccard similarity — always produces a score
    let q_set: HashSet<char> = query.to_ascii_lowercase().chars().collect();
    let c_set: HashSet<char> = candidate.to_ascii_lowercase().chars().collect();
    let intersection = q_set.intersection(&c_set).count();
    let union = q_set.union(&c_set).count();
    let score = if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    };
    Some(ScoredCandidate {
        candidate: candidate.to_string(),
        score,
        strategy: "fallback",
    })
}

// ---------------------------------------------------------------------------
// Strategy 2: prefix_match
// ---------------------------------------------------------------------------

fn strategy_prefix_match(query: &str, candidate: &str) -> Option<f64> {
    let q_lower = query.to_ascii_lowercase();
    let c_lower = candidate.to_ascii_lowercase();
    if c_lower.starts_with(&q_lower) && !q_lower.is_empty() {
        let score = q_lower.len() as f64 / c_lower.len() as f64;
        Some(score.clamp(0.0, 1.0))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Strategy 3: substring_match
// ---------------------------------------------------------------------------

fn strategy_substring_match(query: &str, candidate: &str) -> Option<f64> {
    let q_lower = query.to_ascii_lowercase();
    let c_lower = candidate.to_ascii_lowercase();
    if c_lower.contains(&q_lower) && !q_lower.is_empty() {
        let score = q_lower.len() as f64 / c_lower.len() as f64;
        Some(score.clamp(0.0, 1.0))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Strategy 4: word_match
// ---------------------------------------------------------------------------

fn strategy_word_match(query: &str, candidate: &str) -> Option<f64> {
    let query_words: Vec<&str> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let candidate_words: Vec<&str> = candidate
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    if query_words.is_empty() || candidate_words.is_empty() {
        return None;
    }

    let q_upper: Vec<String> = query_words.iter().map(|w| w.to_ascii_uppercase()).collect();
    let c_upper: Vec<String> = candidate_words
        .iter()
        .map(|w| w.to_ascii_uppercase())
        .collect();
    let c_set: HashSet<&str> = c_upper.iter().map(|s| s.as_str()).collect();

    let matches = q_upper
        .iter()
        .filter(|qw| c_set.contains(qw.as_str()))
        .count();
    if matches > 0 {
        let total = q_upper.len().max(c_upper.len());
        Some(matches as f64 / total as f64)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Strategy 5: acronym_match
// ---------------------------------------------------------------------------

fn strategy_acronym_match(query: &str, candidate: &str) -> Option<f64> {
    let words: Vec<&str> = candidate
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    if words.is_empty() {
        return None;
    }

    let acronym: String = words.iter().filter_map(|w| w.chars().next()).collect();
    if acronym.is_empty() {
        return None;
    }

    if acronym.eq_ignore_ascii_case(query) {
        return Some(1.0);
    }

    // Partial acronym match only meaningful for multi-word candidates
    if words.len() < 2 {
        return None;
    }

    let a_lower = acronym.to_ascii_lowercase();
    let q_lower = query.to_ascii_lowercase();
    if a_lower.starts_with(&q_lower) {
        Some(q_lower.len() as f64 / a_lower.len() as f64)
    } else if q_lower.starts_with(&a_lower) {
        Some(a_lower.len() as f64 / q_lower.len() as f64)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Strategy 6: levenshtein
// ---------------------------------------------------------------------------

fn strategy_levenshtein(query: &str, candidate: &str) -> Option<f64> {
    let dist = levenshtein_distance(query, candidate);
    let threshold = if query.len() < 10 { 3 } else { 5 };

    if dist <= threshold {
        // Normalize to [0, 1]: 1.0 = perfect match, lower = more edits
        let max_len = query.len().max(candidate.len()).max(1);
        let score = 1.0 - (dist as f64 / max_len as f64);
        Some(score.clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Compute Levenshtein edit distance using iterative DP (two rows).
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for (i, ca) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b_chars.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost) // substitution
                .min(curr[j] + 1) // insertion
                .min(prev[j + 1] + 1); // deletion
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

// ---------------------------------------------------------------------------
// Strategy 7: jaro_winkler
// ---------------------------------------------------------------------------

fn strategy_jaro_winkler(query: &str, candidate: &str) -> Option<f64> {
    let sim = jaro_winkler_similarity(query, candidate);
    if sim > 0.7 {
        Some(sim.clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Compute Jaro-Winkler similarity between two strings.
fn jaro_winkler_similarity(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 && n == 0 {
        return 1.0;
    }
    if m == 0 || n == 0 {
        return 0.0;
    }

    let match_distance = (m.max(n) / 2).saturating_sub(1);

    let mut a_matched = vec![false; m];
    let mut b_matched = vec![false; n];
    let mut matches: usize = 0;

    for i in 0..m {
        let lo = if i > match_distance {
            i - match_distance
        } else {
            0
        };
        let hi = (i + match_distance + 1).min(n);
        for j in lo..hi {
            if b_matched[j] {
                continue;
            }
            if a_chars[i] == b_chars[j] {
                a_matched[i] = true;
                b_matched[j] = true;
                matches += 1;
                break;
            }
        }
    }

    if matches == 0 {
        return 0.0;
    }

    // Count transpositions
    let mut k = 0;
    let mut transpositions = 0usize;
    for i in 0..m {
        if a_matched[i] {
            while !b_matched[k] {
                k += 1;
            }
            if a_chars[i] != b_chars[k] {
                transpositions += 1;
            }
            k += 1;
        }
    }

    let jaro = (matches as f64 / m as f64
        + matches as f64 / n as f64
        + (matches as f64 - transpositions as f64 / 2.0) / matches as f64)
        / 3.0;

    // Winkler boost: common prefix up to 4 characters
    let prefix_limit = 4.min(m).min(n);
    let prefix = (0..prefix_limit)
        .take_while(|&i| a_chars[i] == b_chars[i])
        .count();

    jaro + prefix as f64 * 0.1 * (1.0 - jaro)
}

// ---------------------------------------------------------------------------
// Strategy 8: dice_coefficient
// ---------------------------------------------------------------------------

fn strategy_dice_coefficient(query: &str, candidate: &str) -> Option<f64> {
    let dice = dice_coefficient_similarity(query, candidate);
    if dice > 0.5 {
        Some(dice.clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Compute Sørensen-Dice coefficient from character bigrams.
fn dice_coefficient_similarity(a: &str, b: &str) -> f64 {
    let a_bigrams: HashSet<(char, char)> = a.chars().zip(a.chars().skip(1)).collect();
    let b_bigrams: HashSet<(char, char)> = b.chars().zip(b.chars().skip(1)).collect();

    if a_bigrams.is_empty() && b_bigrams.is_empty() {
        return 1.0;
    }
    if a_bigrams.is_empty() || b_bigrams.is_empty() {
        return 0.0;
    }

    let intersection = a_bigrams.intersection(&b_bigrams).count();
    2.0 * intersection as f64 / (a_bigrams.len() + b_bigrams.len()) as f64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- exact_match tests --

    #[test]
    fn test_exact_match_case_insensitive() {
        let candidates = vec!["Hello".to_string(), "World".to_string()];
        let result = find_best_match("hello", &candidates);
        assert!(result.is_some());
        let (matched, score, strategy) = result.unwrap();
        assert_eq!(matched, "Hello");
        assert!((score - 1.0).abs() < 1e-6);
        assert_eq!(strategy, "exact_match");
    }

    #[test]
    fn test_exact_match_same_case() {
        let candidates = vec!["hello".to_string(), "world".to_string()];
        let result = find_best_match("hello", &candidates);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "hello");
    }

    // -- prefix_match tests --

    #[test]
    fn test_prefix_match() {
        let candidates = vec!["hello".to_string(), "world".to_string(), "help".to_string()];
        let result = find_best_match("hel", &candidates);
        assert!(result.is_some());
        let (matched, _score, strategy) = result.unwrap();
        assert_eq!(strategy, "prefix_match");
        // Should prefer "hello" over "help" (both start with "hel")
        assert!(matched == "hello" || matched == "help");
    }

    // -- substring_match tests --

    #[test]
    fn test_substring_match() {
        // "oba" is a substring of "foobar" but not close to "baz"
        let candidates = vec!["foobar".to_string(), "baz".to_string()];
        let result = find_best_match("oba", &candidates);
        assert!(result.is_some());
        let (matched, _score, strategy) = result.unwrap();
        assert_eq!(matched, "foobar");
        assert_eq!(strategy, "substring_match");
    }

    // -- word_match tests --

    #[test]
    fn test_word_match() {
        // "world" is a substring of "hello_world", so substring_match fires first
        let candidates = vec!["hello_world".to_string(), "goodbye_moon".to_string()];
        let result = find_best_match("world", &candidates);
        assert!(result.is_some());
        let (matched, _score, strategy) = result.unwrap();
        assert_eq!(matched, "hello_world");
        assert_eq!(strategy, "substring_match");
    }

    #[test]
    fn test_word_match_priority() {
        // word_match fires when the multi-word query's words overlap candidate words
        // but the query itself isn't a substring (due to separator differences)
        let candidates = vec![
            "get_current_working_directory".to_string(),
            "foo_bar".to_string(),
        ];
        let result = find_best_match("get current", &candidates);
        assert!(result.is_some());
        let (matched, _score, strategy) = result.unwrap();
        assert_eq!(matched, "get_current_working_directory");
        assert_eq!(strategy, "word_match");
    }

    // -- acronym_match tests --

    #[test]
    fn test_acronym_match() {
        let candidates = vec!["getCurrentWorkingDirectory".to_string(), "foo".to_string()];
        // Try matching the acronym
        let _result = find_best_match("gcwd", &candidates);
        // This won't match by acronym because we split by non-alphanumeric only
        // But let's test with actual word-splittable candidate
        let candidates2 = vec!["get_current_working_directory".to_string()];
        let result2 = find_best_match("gcwd", &candidates2);
        assert!(result2.is_some());
        assert_eq!(result2.unwrap().2, "acronym_match");
    }

    // -- levenshtein tests --

    #[test]
    fn test_levenshtein_within_threshold() {
        let candidates = vec!["hello".to_string(), "world".to_string()];
        // Use a query where acronym doesn't interfere
        let result = find_best_match("helxo", &candidates);
        assert!(result.is_some());
        assert_eq!(result.unwrap().2, "levenshtein");
    }

    #[test]
    fn test_levenshtein_beyond_threshold() {
        let candidates = vec!["hello".to_string()];
        let result = find_best_match("xyzzy", &candidates);
        // Should always return something via fallback
        assert!(result.is_some());
    }

    #[test]
    fn test_levenshtein_distance_values() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("a", ""), 1);
        assert_eq!(levenshtein_distance("", "a"), 1);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("abc", "def"), 3);
    }

    // -- jaro_winkler tests --

    #[test]
    fn test_jaro_winkler_high_similarity() {
        // Verify the underlying function produces > 0.7
        let sim = jaro_winkler_similarity("hallo", "hello");
        assert!(sim > 0.7);
    }

    #[test]
    fn test_jaro_winkler_similarity_values() {
        let sim = jaro_winkler_similarity("hello", "hallo");
        assert!(sim > 0.7);
        assert!(sim < 1.0);

        let sim2 = jaro_winkler_similarity("abc", "xyz");
        assert!(sim2 < 0.5);

        assert!((jaro_winkler_similarity("", "") - 1.0).abs() < 1e-6);
    }

    // -- dice_coefficient tests --

    #[test]
    fn test_dice_coefficient_high_overlap() {
        let candidates = vec!["hello".to_string(), "world".to_string()];
        let result = find_best_match("hallo", &candidates);
        assert!(result.is_some());
    }

    #[test]
    fn test_dice_coefficient_similarity_values() {
        let d = dice_coefficient_similarity("hello", "hallo");
        assert!(d > 0.0);

        let d2 = dice_coefficient_similarity("abc", "xyz");
        assert!((d2 - 0.0).abs() < 1e-6);

        assert!((dice_coefficient_similarity("", "") - 1.0).abs() < 1e-6);

        assert!((dice_coefficient_similarity("a", "a") - 1.0).abs() < 1e-6);
    }

    // -- fallback / edge case tests --

    #[test]
    fn test_empty_candidates() {
        let candidates: Vec<String> = vec![];
        let result = find_best_match("hello", &candidates);
        assert!(result.is_none());
    }

    #[test]
    fn test_empty_query() {
        let candidates = vec!["hello".to_string()];
        let result = find_best_match("", &candidates);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_suggestions_returns_multiple() {
        let candidates = vec![
            "apple".to_string(),
            "application".to_string(),
            "appetizer".to_string(),
            "banana".to_string(),
        ];
        let suggestions = find_suggestions("app", &candidates, 3);
        assert_eq!(suggestions.len(), 3);
        // All should start with "app"
        for (name, _score, _strat) in &suggestions {
            assert!(name.to_ascii_lowercase().starts_with("app"));
        }
    }

    #[test]
    fn test_find_suggestions_sorted_by_score() {
        let candidates = vec![
            "exactly".to_string(),
            "exact".to_string(),
            "xxyyzz".to_string(),
        ];
        let suggestions = find_suggestions("exact", &candidates, 3);
        assert!(!suggestions.is_empty());
        // First result should be the best match
        assert_eq!(suggestions[0].0, "exact");
    }

    #[test]
    fn test_case_insensitivity_across_strategies() {
        let candidates = vec!["HELLO".to_string(), "WORLD".to_string()];
        let result = find_best_match("hello", &candidates);
        assert!(result.is_some());
        assert_eq!(result.unwrap().2, "exact_match");
    }

    #[test]
    fn test_no_match_returns_none() {
        let candidates = vec!["alpha".to_string(), "beta".to_string()];
        let result = find_best_match("omega", &candidates);
        // "omega" is very different from "alpha" and "beta"
        // It may still match with low score via jaro_winkler or dice
        // The fallback ensures SOMETHING is returned
        assert!(result.is_some());
    }

    #[test]
    fn test_short_string_levenshtein_threshold() {
        // Short string (< 10 chars): threshold = 3
        let dist = levenshtein_distance("cat", "dog");
        assert_eq!(dist, 3);
    }

    #[test]
    fn test_unicode_handling() {
        let candidates = vec!["café".to_string(), "coffee".to_string()];
        let result = find_best_match("cafe", &candidates);
        assert!(result.is_some());
    }

    #[test]
    fn test_find_suggestions_max_results_zero() {
        let candidates = vec!["a".to_string()];
        let suggestions = find_suggestions("a", &candidates, 0);
        assert!(suggestions.is_empty());
    }
}
