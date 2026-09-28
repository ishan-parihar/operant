//! Substance measurement for destructive-command justifications.
//!
//! The gate must not be satisfiable by retrying. "A longer meaningless
//! justification must fail" is the whole point, so this module measures
//! **substance, not length**: every signal below is a property of the token
//! *distribution*, and padding cannot move any of them.
//!
//! - `distinct_content_words` — padding adds length, not distinct words, so
//!   the floor is on the count of distinct non-stopword tokens of 3+
//!   characters. `"aaaaaaaa"` scores 1.
//! - `repetition_ratio` — `total_tokens / distinct_tokens`. Hammering one
//!   sentence (or one word) to reach a length leaves the ratio flat-high, so
//!   lorem-ipsum-style filler is rejected however long it is.
//! - `command_overlap` — the share of the justification's content words that
//!   are also tokens of the command. A restatement of the command
//!   ("run rm -rf build") is almost entirely overlap.
//! - `diversity` — normalized Shannon entropy over the token distribution. A
//!   run of one or two tokens has no diversity to measure.
//!
//! There is deliberately **no length floor**. A short, real reason passes; a
//! very long, empty one does not.
//!
//! Everything here is pure: text in, [`Assessment`] out. No I/O, no clock, no
//! randomness — so the gate cannot be satisfied by a lucky string, and the
//! whole thing is testable without a terminal or a filesystem.
//!
//! # Known ceiling
//!
//! These are lexical measures, not semantic ones. A justification that is
//! genuinely 20 distinct words of Greek filler passes. This is deliberate: an
//! LLM asked to justify a destructive command does not emit lorem ipsum under
//! pressure, it emits either a real sentence or a short evasive one, and both
//! are caught. Reading meaning would need a model call inside a safety gate,
//! which is a worse failure mode than a lexical ceiling.

use std::collections::HashSet;

/// Why a justification was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// Fewer distinct content words than the floor — padding, or a bare noun.
    TooFewDistinctWords,
    /// The same few words hammered to reach a length.
    Repetitive,
    /// Mostly a restatement of the command it purports to justify.
    EchoesCommand,
    /// Lexically impoverished: the text has almost no token variety.
    LowDiversity,
}

impl Defect {
    /// Human-readable label used in the model-facing refusal message.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooFewDistinctWords => "too few distinct words to be a real reason",
            Self::Repetitive => "one phrase repeated instead of an explanation",
            Self::EchoesCommand => "restates the command rather than justifying it",
            Self::LowDiversity => "no lexical variety",
        }
    }
}

/// Floor on distinct, non-stopword, 3+ character words.
pub const MIN_DISTINCT_CONTENT_WORDS: usize = 8;
/// Ceiling on `total_tokens / distinct_tokens` — above this, length came
/// from repetition rather than from content.
pub const MAX_REPETITION_RATIO: f64 = 5.0;
/// Ceiling on the share of content words shared with the command itself.
pub const MAX_COMMAND_OVERLAP: f64 = 0.35;
/// Floor on normalized Shannon entropy of the token distribution.
pub const MIN_DIVERSITY: f64 = 0.70;

/// Function words that carry no explanatory content. Deliberately excludes
/// domain words (`build`, `cache`, `stale`, `remove`, …) — those ARE the
/// substance we are trying to measure.
const STOPWORDS: &[&str] = &[
    "the",
    "a",
    "an",
    "and",
    "or",
    "but",
    "nor",
    "of",
    "to",
    "in",
    "on",
    "at",
    "by",
    "for",
    "with",
    "from",
    "as",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "it",
    "its",
    "this",
    "that",
    "these",
    "those",
    "then",
    "than",
    "so",
    "if",
    "not",
    "no",
    "do",
    "does",
    "did",
    "has",
    "have",
    "had",
    "will",
    "would",
    "can",
    "could",
    "should",
    "shall",
    "may",
    "might",
    "must",
    "we",
    "i",
    "you",
    "they",
    "he",
    "she",
    "them",
    "our",
    "my",
    "your",
    "their",
    "please",
    "just",
    "also",
    "more",
    "very",
    "any",
    "all",
    "some",
    "such",
    "into",
    "out",
    "up",
    "down",
    "off",
    "over",
    "under",
    "about",
    "after",
    "before",
    "when",
    "while",
    "because",
    "need",
    "needs",
    "want",
    "wants",
    "use",
    "using",
    "used",
    "run",
    "runs",
    "running",
    "make",
    "makes",
    "get",
    "gets",
    "only",
    "same",
    "other",
    "another",
    "there",
    "here",
    "what",
    "which",
    "who",
    "how",
    "why",
    "where",
    "let",
    "lets",
    "now",
    "new",
    "old",
    "via",
    "per",
    "into",
    "onto",
    "doesn",
    "don",
    "shouldn",
    "wouldn",
    "cannot",
    "every",
    "each",
    "both",
    "either",
    "neither",
    "very",
    "get",
    "goes",
    "went",
    "like",
    "one",
    "two",
    "way",
    "thing",
    "things",
    "really",
    "actually",
    "maybe",
    "probably",
    "basically",
    "simply",
    "just",
    "quite",
    "rather",
    "much",
    "many",
    "own",
];

/// Split text into lowercase tokens, keeping `.` and `_` so `./target` and
/// `Cargo.lock` survive as single tokens (they are meaningful targets).
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A token counts as content when it is 3+ characters, not a stopword, and
/// not a bare number.
fn is_content(token: &str) -> bool {
    token.chars().count() >= 3
        && !STOPWORDS.contains(&token)
        && !token.chars().all(|c| c.is_ascii_digit())
}

/// Normalized Shannon entropy in `[0.0, 1.0]`: the base-2 entropy of the
/// token distribution divided by the entropy of a perfectly uniform one.
fn diversity(tokens: &[String], distinct: usize) -> f64 {
    if distinct < 2 || tokens.is_empty() {
        return 0.0;
    }
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for t in tokens {
        *counts.entry(t.as_str()).or_default() += 1;
    }
    let total = tokens.len() as f64;
    let entropy = counts
        .values()
        .map(|c| {
            let p = *c as f64 / total;
            -p * p.log2()
        })
        .sum::<f64>();
    (entropy / (distinct as f64).log2()).clamp(0.0, 1.0)
}

/// The measured substance of one justification.
#[derive(Debug, Clone, PartialEq)]
pub struct Assessment {
    /// Whether the justification clears every gate.
    pub sufficient: bool,
    /// Which gates it failed.
    pub defects: Vec<Defect>,
    /// Distinct non-stopword words of 3+ characters.
    pub distinct_content_words: usize,
    /// `total_tokens / distinct_tokens`.
    pub repetition_ratio: f64,
    /// Share of content words shared with the command.
    pub command_overlap: f64,
    /// Normalized Shannon entropy of the token distribution.
    pub diversity: f64,
}

impl Assessment {
    /// One-line summary for logs.
    pub fn summary(&self) -> String {
        format!(
            "content_words={} repetition={:.2} overlap={:.2} diversity={:.2} sufficient={}",
            self.distinct_content_words,
            self.repetition_ratio,
            self.command_overlap,
            self.diversity,
            self.sufficient
        )
    }
}

/// Measure the substance of `justification` for `command`. Pure.
pub fn assess(justification: &str, command: &str) -> Assessment {
    let tokens = tokenize(justification);
    let distinct: HashSet<&str> = tokens.iter().map(String::as_str).collect();
    let content: HashSet<&str> = distinct.iter().copied().filter(|t| is_content(t)).collect();

    let distinct_content_words = content.len();
    let repetition_ratio = if distinct.is_empty() {
        f64::INFINITY
    } else {
        tokens.len() as f64 / distinct.len() as f64
    };
    let command_tokens: HashSet<String> = tokenize(command).into_iter().collect();
    let command_overlap = if content.is_empty() {
        0.0
    } else {
        content
            .iter()
            .filter(|t| command_tokens.contains(**t))
            .count() as f64
            / content.len() as f64
    };
    let diversity = diversity(&tokens, distinct.len());

    let mut defects = Vec::new();
    if distinct_content_words < MIN_DISTINCT_CONTENT_WORDS {
        defects.push(Defect::TooFewDistinctWords);
    }
    if repetition_ratio > MAX_REPETITION_RATIO {
        defects.push(Defect::Repetitive);
    }
    if command_overlap > MAX_COMMAND_OVERLAP {
        defects.push(Defect::EchoesCommand);
    }
    if distinct.len() >= 4 && diversity < MIN_DIVERSITY {
        defects.push(Defect::LowDiversity);
    }

    Assessment {
        sufficient: defects.is_empty(),
        defects,
        distinct_content_words,
        repetition_ratio,
        command_overlap,
        diversity,
    }
}

/// Convenience predicate over [`assess`].
pub fn is_substantive(justification: &str, command: &str) -> bool {
    assess(justification, command).sufficient
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn justification_should_reject_padded_meaningless_text() {
        let cmd = "rm -rf /";

        // Short padding.
        let short = "a".repeat(30);
        assert!(!is_substantive(&short, cmd), "repeated 'a' must fail");

        // Long padding: a single word, repeated to absurd length.
        let long = "aaaaaaaaaa ".repeat(3000);
        assert!(
            !is_substantive(&long, cmd),
            "3000 repeats of one word must fail"
        );

        // Lorem-ipsum filler repeated to absurd length: plenty of distinct
        // words, but the same words over and over.
        let lorem = "lorem ipsum dolor sit amet consectetur adipiscing elit sed \
                     do eiusmod tempor incididunt ut labore et dolore magna aliqua "
            .repeat(200);
        let report = assess(&lorem, cmd);
        assert!(!report.sufficient, "repeated filler must fail");
        assert!(
            report.defects.contains(&Defect::Repetitive),
            "expected Repetitive, got {:?}",
            report.defects
        );
        assert!(report.repetition_ratio > MAX_REPETITION_RATIO);
    }

    #[test]
    fn justification_should_reject_restating_the_command() {
        let cmd = "rm -rf ./build";
        // A restatement adds no information the command text did not already carry.
        assert!(!is_substantive("run rm -rf on the build directory", cmd));
        assert!(!is_substantive(
            "this removes the build folder recursively",
            cmd
        ));
        assert!(!is_substantive("rm -rf ./build, as requested", cmd));

        let report = assess("run rm -rf on the build directory", cmd);
        assert!(report.command_overlap > MAX_COMMAND_OVERLAP);
    }

    #[test]
    fn justification_should_accept_a_substantive_reason() {
        let cmd = "rm -rf ./target";
        let good = "The cargo build cache under ./target is stale and has grown to 4GB \
                    of disk, which is filling the container's writable layer. Every \
                    artifact in there is reproducible from the checked-in Cargo.lock \
                    with a single cargo build, so nothing authored by a human is lost.";
        let report = assess(good, cmd);
        assert!(
            report.sufficient,
            "expected a real reason to pass, got {:?} — {}",
            report.defects,
            report.summary()
        );
        assert!(report.distinct_content_words >= MIN_DISTINCT_CONTENT_WORDS);
        assert!(report.repetition_ratio <= MAX_REPETITION_RATIO);
        assert!(report.command_overlap <= MAX_COMMAND_OVERLAP);

        // A second, differently-shaped real reason also passes.
        let other = "Flaky integration fixtures leave orphaned docker volumes behind, \
                     and this sweep reclaims roughly 12GB on the shared build host. \
                     Nothing here is read by the release pipeline.";
        assert!(is_substantive(
            other,
            "docker volume prune -a --filter until=24h"
        ));
    }

    #[test]
    fn a_short_real_reason_passes_because_length_is_not_measured() {
        // Deliberately short: the gates are on distinct words and overlap,
        // not on character count.
        let report = assess(
            "Flaky fixtures left orphaned docker volumes eating 12GB of shared disk",
            "docker volume prune -a",
        );
        assert!(report.sufficient, "{}", report.summary());
    }

    #[test]
    fn empty_and_whitespace_are_rejected_without_panicking() {
        assert!(!is_substantive("", "rm -rf /"));
        assert!(!is_substantive("   \t\n ", "rm -rf /"));
        assert!(!is_substantive("!!! ??? ...", "rm -rf /"));
    }
}
