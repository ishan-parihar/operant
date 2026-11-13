//! The seed dictionary — the predictor's cold-start vocabulary.
//!
//! The learned n-gram (plan 2026-10-24 M5) only knows the words the operator
//! has actually submitted, and the hygiene gate (`MIN_PROMPTS`) needs a word
//! in TWO distinct prompts before it may be shown. A fresh install therefore
//! predicts nothing at all: the ghost is dead until the operator has typed
//! the same word twice.
//!
//! OMP closes that gap with a 2.6 MB Norvig web prior blended into the
//! trigram. This is the minimum that buys the same cold-start capability:
//! the most frequent English word under the typed prefix, offered ONLY when
//! the learned vocabulary knows no word under that prefix at all. The
//! dictionary can never outrank what the operator's own history says and it
//! never revives a word the hygiene gate rejected — it fills a vocabulary
//! GAP and nothing else, so the learned model stays the self-learning
//! system (2026-11-12).
//!
//! The asset is `word<TAB>frequency-rank` (0 = most frequent), one word per
//! line, sorted by word so a prefix range is one binary search. It is the
//! 5,000 most frequent lowercase English prose forms, extracted from the
//! Norvig web-corpus count list that oh-my-pi's `pi-predict` ships
//! (`crates/pi-predict/data/web-prior.bin.zst`, Norvig `count_1w`).

use std::sync::LazyLock;

/// The embedded dictionary.
const ASSET: &str = include_str!("dictionary.tsv");

/// Parsed once per process: `(word, frequency rank)`, sorted by word.
static WORDS: LazyLock<Vec<(&'static str, u32)>> = LazyLock::new(|| {
    ASSET
        .lines()
        .filter_map(|line| {
            let (word, rank) = line.split_once('\t')?;
            Some((word, rank.parse().ok()?))
        })
        .collect()
});

/// The most frequent dictionary word strictly under `prefix`: the suffix it
/// appends, and its frequency confidence in `0.0..=1.0` (the most frequent
/// word is 1.0, the last one in the asset is just above 0.0).
///
/// `None` when no dictionary word extends the prefix, and when the prefix is
/// itself a dictionary word — a word the operator has FINISHED is not
/// extended ("the" must not become "their").
pub fn completion(prefix: &str) -> Option<(String, f64)> {
    let words = &*WORDS;
    if prefix.is_empty() || words.is_empty() {
        return None;
    }
    if words.binary_search_by_key(&prefix, |entry| entry.0).is_ok() {
        return None;
    }
    let start = words.partition_point(|entry| entry.0 < prefix);
    let mut best: Option<(&str, u32)> = None;
    for (word, rank) in &words[start..] {
        if !word.starts_with(prefix) {
            break;
        }
        if word.len() <= prefix.len() {
            continue;
        }
        if best.is_none_or(|(_, best_rank)| *rank < best_rank) {
            best = Some((*word, *rank));
        }
    }
    let (word, rank) = best?;
    let confidence = 1.0 - (rank as f64 / words.len() as f64);
    Some((word[prefix.len()..].to_string(), confidence))
}

#[cfg(test)]
mod dictionary_tests {
    use super::*;

    #[test]
    fn the_asset_is_sorted_and_parsed() {
        let words = &*WORDS;
        assert!(words.len() > 1_000, "the seed dictionary is empty");
        assert!(
            words.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "the asset must be sorted by word for the prefix binary search"
        );
        assert!(
            words
                .iter()
                .all(|(word, _)| !word.is_empty() && !word.contains(char::is_whitespace)),
            "a dictionary entry must be one bare word"
        );
    }

    #[test]
    fn a_prefix_completes_to_the_most_frequent_word_under_it() {
        // "help" is the most frequent word under "hel" ("held" 1036, "hello"
        // 2393, "help" 73) — the rank, not the spelling, decides.
        let (suffix, confidence) = completion("hel").expect("a common word completes");
        assert_eq!(suffix, "p");
        assert!(
            confidence > 0.0 && confidence <= 1.0,
            "confidence {confidence}"
        );
    }

    #[test]
    fn a_finished_word_is_not_extended() {
        // "the" is in the dictionary: the operator finished it, so no
        // dictionary word may be offered under it.
        assert!(
            completion("the").is_none(),
            "a finished word must not extend"
        );
    }

    #[test]
    fn an_unknown_prefix_completes_to_nothing() {
        assert!(completion("zzq").is_none());
    }
}
