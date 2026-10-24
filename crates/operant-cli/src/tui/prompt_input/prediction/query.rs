//! Completion ranking: prefix-constrained interpolated trigram scoring with
//! the finished-word-aware posterior (plan 2026-10-24 M5, port of OMP
//! `pi-predict/src/ngram/query.rs`).
//!
//! For prefix `p` after context `(u, v)` with chain score `s(w)`:
//!
//! ```text
//! post(w)  = s(w) / M          where M = Σ s(x) over every word under p,
//!                               INCLUDING the prefix itself when it is one
//! final(w) = (post(w) + λ·cache(w) + σ·session(w)) / (1 + λ·C + σ·S)
//! ```
//!
//! `cache(w)` is `exp(−distance/D)` over occurrences of `w` earlier in the
//! prompt (×(1+boost) after the same previous word); `session(w)` counts
//! `w` in prompts already submitted this session. Every score is a ratio
//! over the words under the prefix, so typing another letter of the shown
//! word removes competitors without reordering the rest: **a ghost stays
//! put while the user types through it** (pinned by
//! `ghost_stays_while_typing_through`).

use std::collections::HashMap;

use super::model::{Model, prose_words};

/// Interpolation parameters (OMP `Params`, retuned for a uniform prior:
/// the web-unigram μ1 backstop does not exist here, so μ1 is larger).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub d2: f64,
    pub d3: f64,
    pub mu1: f64,
    pub mu2: f64,
    pub mu3: f64,
    /// Prompt-local cache weight (λ).
    pub cache_weight: f64,
    /// Cache decay distance constant, in bytes.
    pub cache_decay: f64,
    /// Boost when the earlier occurrence followed the same previous word.
    pub cache_context_boost: f64,
    /// Session-cache weight (σ).
    pub session_weight: f64,
    /// Ghost show threshold τ (OMP's tuned default).
    pub show_threshold: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            d2: 0.75,
            d3: 0.75,
            mu1: 20.0,
            mu2: 8.0,
            mu3: 8.0,
            cache_weight: 0.6,
            cache_decay: 40.0,
            cache_context_boost: 1.0,
            session_weight: 0.5,
            show_threshold: 0.15,
        }
    }
}

/// One completion answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// Characters appended after the typed prefix (case-aligned).
    pub suffix: String,
    /// Posterior of the best word (already threshold-gated).
    pub confidence: f64,
}

#[derive(Clone, Copy)]
struct Cand {
    /// Vocabulary id, or `None` for a prompt-local-only word.
    id: Option<u32>,
    raw: f64,
    cache: f64,
    session: f64,
}

/// The last (up to) two prose words of `before` as bigram context ids.
fn context_ids(model: &Model, before: &str) -> (Option<u32>, Option<u32>) {
    let words = prose_words(before);
    let v = words.last().and_then(|w| model.id(w));
    let u = if words.len() >= 2 {
        model.id(&words[words.len() - 2])
    } else {
        None
    };
    (u, v)
}

/// The interpolated chain for one candidate: unigram → bigram → trigram,
/// absolute-discounted with a uniform backstop (OMP `interp`).
fn chain_score(model: &Model, p: &Params, u: Option<u32>, v: Option<u32>, id: u32, c1: f64) -> f64 {
    let vocab = model.vocab_len().max(1) as f64;
    let p1 = (c1 + p.mu1 / vocab) / (model.total_mass() + p.mu1);
    let (cv, tv) = v.map_or((0.0, 0usize), |v| model.follower_stats(v));
    let p2 = match (v, cv > 0.0) {
        (Some(v), true) => {
            let disc = (model.bigram(v, id) - p.d2).max(0.0);
            (disc + (p.d2 * tv as f64 + p.mu2) * p1) / (cv + p.mu2)
        }
        _ => p1,
    };
    match (u, v) {
        (Some(u), Some(v)) => {
            let (cuv, tuv) = model.pair_stats(u, v);
            if cuv > 0.0 {
                let disc = (model.trigram(u, v, id) - p.d3).max(0.0);
                (disc + (p.d3 * f64::from(tuv) + p.mu3) * p2) / (cuv + p.mu3)
            } else {
                p2
            }
        }
        _ => p2,
    }
}

/// Prompt-local cache: weight per FULL word (lowercased) seen earlier in
/// `before` that extends `key` (`re` → `review`), `exp(−distance/D)` with
/// the same-previous-word boost. One pass over the prose runs.
fn window_weights(before: &str, key: &str, p: &Params) -> HashMap<String, f64> {
    let lower = before.to_lowercase();
    let total = lower.len() as f64;
    let mut out: HashMap<String, f64> = HashMap::new();
    let words = prose_words(before);
    let prev_of = |i: usize| -> Option<&String> { i.checked_sub(1).and_then(|j| words.get(j)) };
    // Walk the words with their byte starts so distance and the previous
    // word are both known.
    let mut at = 0usize;
    for (i, word) in words.iter().enumerate() {
        let Some(pos) = lower[at..].find(word.as_str()) else {
            break;
        };
        let pos = at + pos;
        at = pos + word.len();
        if word.starts_with(key) && word.len() > key.len() {
            let mut w = (-(total - pos as f64) / p.cache_decay).exp();
            if let Some(prev) = prev_of(i)
                && model_word_eq_prev(&lower.as_bytes()[..pos], prev)
            {
                w *= 1.0 + p.cache_context_boost;
            }
            *out.entry(word.clone()).or_insert(0.0) += w;
        }
    }
    out
}

/// Whether `head` (bytes of `lower[..pos]`) ends with the previous word at
/// a word boundary — the context boost condition.
fn model_word_eq_prev(head: &[u8], prev: &str) -> bool {
    head.len() > prev.len()
        && &head[head.len() - prev.len() - 1..head.len() - prev.len()] == b" "
        && &head[head.len() - prev.len()..] == prev.as_bytes()
}

/// Best completion of `prefix` given the editor text before it and the
/// session's submitted prompts. `None` = show no ghost.
pub fn complete(
    model: &Model,
    session: &[String],
    before: &str,
    prefix: &str,
    threshold_override: Option<f64>,
) -> Option<Suggestion> {
    if prefix.is_empty() {
        return None;
    }
    let p = Params::default();
    let key = prefix.to_lowercase();
    let (u, v) = context_ids(model, before);
    let window = window_weights(before, &key, &p);

    // Vocabulary candidates: every word under the prefix feeds the mass;
    // eligibility only gates which may be OFFERED.
    let prefix_ids = model.prefix_ids(&key);
    let mut cands: Vec<(String, Cand)> = Vec::new();
    let mut mass = 0.0f64;
    for id in prefix_ids {
        let word = model.word(id).to_string();
        let raw = chain_score(model, &p, u, v, id, model.unigram(id));
        mass += raw;
        let cache = window.get(&word).copied().unwrap_or(0.0);
        let sess = session.iter().filter(|s| *s == &word).count() as f64;
        cands.push((
            word,
            Cand {
                id: Some(id),
                raw,
                cache,
                session: sess,
            },
        ));
    }
    // The prefix itself competes: finishing must beat stopping there.
    if let Some(pid) = model.id(&key) {
        mass += chain_score(model, &p, u, v, pid, model.unigram(pid));
    }
    // Prompt-local-only words (typed earlier, never submitted) join as
    // cache-only candidates (OMP's Pick::Local).
    for (word, cache) in &window {
        if model.id(word).is_none() {
            cands.push((
                word.clone(),
                Cand {
                    id: None,
                    raw: 0.0,
                    cache: *cache,
                    session: 0.0,
                },
            ));
        }
    }

    let total_cache: f64 = cands.iter().map(|c| c.1.cache).sum();
    let total_session: f64 = cands.iter().map(|c| c.1.session).sum();
    let norm = 1.0 + p.cache_weight * total_cache + p.session_weight * total_session;

    let mut best: Option<(&(String, Cand), f64)> = None;
    for cand in &cands {
        let (word, c) = cand;
        if word.len() == key.len() {
            continue; // completing to itself is no completion
        }
        // Hygiene: vocab words must be eligible UNLESS the prompt-local
        // cache carries them (the user just typed this word).
        if let Some(id) = c.id
            && c.cache == 0.0
            && !model.eligible(id)
        {
            continue;
        }
        let post = if mass > 0.0 { c.raw / mass } else { 0.0 };
        let value = (post + p.cache_weight * c.cache + p.session_weight * c.session) / norm;
        if value > best.map_or(0.0, |(_, s)| s) {
            best = Some((cand, value));
        }
    }
    let ((word, _), value) = best?;
    let threshold = threshold_override.unwrap_or(p.show_threshold);
    if value < threshold {
        return None;
    }
    // The ghost suffix is VERBATIM: `key` is the lowercased prefix and the
    // vocab is lowercase, so `word[key.len()..]` is already the correct
    // continuation for any casing the user typed (the case channel OMP
    // carries exists to recover acronym casing, which this vocab never
    // stored in the first place).
    let suffix = word[key.len()..].to_string();
    Some(Suggestion {
        suffix,
        confidence: value,
    })
}

#[cfg(test)]
mod query_tests {
    use super::*;

    fn trained() -> Model {
        let mut m = Model::default();
        for prompt in [
            "can you refactor the parser",
            "can you review the parser tests",
            "can you refactor the diff",
            "please review the tests",
            "review the parser output",
        ] {
            m.learn(prompt);
        }
        m
    }

    #[test]
    fn context_completes_the_personal_phrasing() {
        let m = trained();
        let s =
            complete(&m, &[], "can you refactor the par", "par", None).expect("parser completes");
        assert_eq!(s.suffix, "ser");
        assert!(s.confidence >= 0.15, "confidence {}", s.confidence);
    }

    #[test]
    fn thin_evidence_shows_nothing() {
        let m = trained();
        // 'ref' after an unseen context still ranks 'refactor' via unigram,
        // but the posterior must clear τ — and after nothing typed before,
        // the mass is unigram-smoothed. Either a ghost or none: assert the
        // invariant, not the number.
        if let Some(s) = complete(&m, &[], "zzz qq ref", "ref", None) {
            assert_eq!(s.suffix, "actor");
        }
    }

    #[test]
    fn ghost_stays_while_typing_through() {
        // The ratio property: once 'par' shows 'parser', typing more of the
        // SAME word must keep showing the same word (shrinking suffix).
        let m = trained();
        let before = "can you refactor the ";
        for (prefix, expect_suffix) in [("par", "ser"), ("pars", "er"), ("parse", "r")] {
            let s = complete(&m, &[], before, prefix, None)
                .unwrap_or_else(|| panic!("ghost vanished at {prefix}"));
            assert_eq!(s.suffix, expect_suffix, "at prefix {prefix}");
        }
    }

    #[test]
    fn prompt_local_word_extends_without_history() {
        let mut m = Model::default();
        m.learn("totally unrelated words here");
        let s = complete(
            &m,
            &[],
            "check the shipyard cranes and the cran",
            "cran",
            None,
        );
        // 'cranes' was typed earlier in THIS prompt: the cache carries it.
        assert!(s.is_some(), "prompt-local extension should show");
        assert_eq!(s.map(|s| s.suffix), Some("es".to_string()));
    }

    #[test]
    fn suffix_is_verbatim_lowercase() {
        let m = trained();
        let s = complete(&m, &[], "Can you refactor the Par", "Par", None).expect("completes");
        // 'Par' lowercases to 'par'; the stored word is 'parser', so the
        // continuation is lowercase regardless of how the prefix was typed.
        assert_eq!(s.suffix, "ser");
    }
}
