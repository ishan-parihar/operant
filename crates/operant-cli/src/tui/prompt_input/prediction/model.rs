//! The learned counts of the personal-history word predictor (plan
//! 2026-10-24 M5).
//!
//! Port of OMP `pi-predict`'s n-gram model, scoped for operant: an
//! interpolated absolute-discounting word trigram over the user's own
//! submitted prompts. Divergences from OMP, on purpose:
//! - no embedded Norvig web prior (the prior is uniform; OMP ships a 2.6 MB
//!   data file — the show threshold compensates on a thin corpus);
//! - no SmolLM / NSSpellChecker engines and no `Predictor` trait for them
//!   (one engine, no speculation);
//! - misspelling folding needs a dictionary; without the web file there is
//!   none, so hygiene is the `MIN_PROMPTS` rule instead.

use std::collections::HashMap;

/// Minimum number of DISTINCT typed prompts a word must appear in before
/// it may be suggested (OMP `min_count` hygiene: one-off typos and junk
/// never surface; a word the user actually uses recurs).
const MIN_PROMPTS: u32 = 2;

/// A prompt that looks pasted or logged learns at this weight (OMP
/// `paste_weight`): multi-line or very long prompts are not typing data.
const PASTE_WEIGHT: f64 = 0.1;

/// Words this long or longer are skipped — completion targets prose words,
/// not tokens/hashes.
const MAX_WORD_LEN: usize = 24;

/// The learned model. All word ids index [`Model::vocab`], which stays
/// sorted so a typed prefix is one contiguous id range (binary search).
#[derive(Debug, Default, Clone)]
pub struct Model {
    /// Lowercase vocabulary; `[0..base_len)` is sorted, the rest is the
    /// append-only overflow.
    vocab: Vec<String>,
    /// Length of the sorted base segment.
    base_len: usize,
    /// vocab word → id.
    index: HashMap<String, u32>,
    /// Unigram counts (fractional: pastes count partially).
    count: Vec<f64>,
    /// Distinct-prompt occurrences per word (hygiene gate).
    prompt_counts: HashMap<u32, u32>,
    /// prev id → follower id → bigram count.
    followers: HashMap<u32, HashMap<u32, f64>>,
    /// (pair, third) → trigram continuation count.
    tri3: HashMap<(u64, u32), f64>,
    /// pair → total mass following the pair (for the discount denominator).
    pair_totals: HashMap<u64, f64>,
    /// pair → distinct third words after the pair.
    pair_types: HashMap<u64, u32>,
    /// Total unigram mass.
    total: f64,
}

/// A prose word: letters (incl. non-ASCII), marks, and apostrophes, 2..
/// [`MAX_WORD_LEN`] chars — the class the ghost completes.
fn is_prose_word(tok: &str) -> bool {
    tok.len() >= 2
        && tok.len() <= MAX_WORD_LEN
        && tok
            .chars()
            .all(|c| c.is_alphabetic() || c == '\'' || c == '\u{2019}')
        && tok.chars().any(|c| c.is_alphabetic())
}

/// Strip learn-side noise from a prompt before tokenizing: backtick code
/// spans and `@file:`-style reference tokens are not typing data (the
/// query side does not need this — it reads the composer, whose code and
/// references the M6 prose gate already excludes).
fn strip_nonprose(prompt: &str) -> String {
    let mut out = String::with_capacity(prompt.len());
    let mut in_code = false;
    for c in prompt.chars() {
        match c {
            '`' => {
                in_code = !in_code;
                out.push(' ');
            }
            _ if in_code => {}
            _ => out.push(c),
        }
    }
    out.split_whitespace()
        .filter(|tok| {
            !(tok.contains("@file:")
                || tok.contains("@folder:")
                || tok.contains("@git:")
                || tok.contains("@url:"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split `text` into lowercased prose words. Non-prose tokens (paths,
/// code, numbers) are dropped — they are not typing data.
pub fn prose_words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphabetic() || c == '\'' || c == '\u{2019}'))
        .filter(|tok| is_prose_word(tok))
        .map(|tok| tok.to_lowercase())
        .collect()
}

fn pack_pair(u: u32, v: u32) -> u64 {
    (u64::from(u) << 32) | u64::from(v)
}

impl Model {
    pub fn word(&self, id: u32) -> &str {
        self.vocab
            .get(id as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn id(&self, word: &str) -> Option<u32> {
        self.index.get(word).copied()
    }

    pub fn vocab_len(&self) -> usize {
        self.vocab.len()
    }

    pub fn unigram(&self, id: u32) -> f64 {
        self.count.get(id as usize).copied().unwrap_or(0.0)
    }

    pub fn total_mass(&self) -> f64 {
        self.total
    }

    /// Every vocab id whose word starts with `prefix`. The vocab is a
    /// sorted BASE segment plus an appended OVERFLOW segment (OMP's
    /// structure): interns append to the overflow so ids never shift
    /// mid-prompt, and [`Model::compact`] folds the overflow back into the
    /// sorted base once it grows past [`COMPACT_THRESHOLD`].
    pub fn prefix_ids(&self, prefix: &str) -> Vec<u32> {
        let mut out = Vec::new();
        // The base segment is sorted (binary search); the overflow is
        // append-ordered (linear scan). Both, never a scan of one by the
        // other's rule — an unsorted overflow fed to partition_point
        // returned garbage ranges that double-counted ids.
        let (base, overflow) = self.vocab.split_at(self.base_len);
        let lo = base.partition_point(|w| w.as_str() < prefix);
        for id in lo..lo + base[lo..].partition_point(|w| w.starts_with(prefix)) {
            out.push(id as u32);
        }
        for (id, w) in overflow.iter().enumerate() {
            if w.starts_with(prefix) {
                out.push((self.base_len + id) as u32);
            }
        }
        out
    }

    /// Whether `id` may be suggested (hygiene gate).
    pub fn eligible(&self, id: u32) -> bool {
        self.prompt_counts
            .get(&id)
            .is_some_and(|c| *c >= MIN_PROMPTS)
    }

    /// Overflow words beyond which [`Model::learn`] folds the overflow
    /// back into the sorted base (OMP compacts every 4096 new words; the
    /// remap is a rare, bounded stall instead of a per-intern cost).
    const COMPACT_THRESHOLD: usize = 4096;

    /// Intern `word`: found ids return unchanged; new words APPEND to the
    /// overflow segment so no id ever shifts while a prompt is being
    /// learned (shifting corrupted counts recorded earlier in the same
    /// walk — the reason this is append-only).
    fn intern(&mut self, word: &str) -> u32 {
        if let Some(id) = self.index.get(word) {
            return *id;
        }
        let id = self.vocab.len() as u32;
        self.vocab.push(word.to_string());
        self.count.push(0.0);
        self.index.insert(word.to_string(), id);
        if self.vocab.len() - self.base_len > Self::COMPACT_THRESHOLD {
            self.compact();
        }
        id
    }

    /// Sort the whole vocab and remap every id-keyed map through the
    /// old→new permutation.
    fn compact(&mut self) {
        let old_vocab = self.vocab.clone();
        let mut order: Vec<u32> = (0..old_vocab.len() as u32).collect();
        order.sort_by(|a, b| old_vocab[*a as usize].cmp(&old_vocab[*b as usize]));
        // new_id → old_id; invert for old→new.
        let mut old_to_new = vec![0u32; order.len()];
        for (new_id, old_id) in order.iter().enumerate() {
            old_to_new[*old_id as usize] = new_id as u32;
        }
        let map = |id: u32| old_to_new[id as usize];
        self.vocab = order
            .iter()
            .map(|old| old_vocab[*old as usize].clone())
            .collect();
        self.count = order.iter().map(|old| self.count[*old as usize]).collect();
        let followers = std::mem::take(&mut self.followers);
        let mut out = HashMap::with_capacity(followers.len());
        for (prev, inner) in followers {
            let mut inner_out = HashMap::with_capacity(inner.len());
            for (w, c) in inner {
                inner_out.insert(map(w), c);
            }
            out.insert(map(prev), inner_out);
        }
        self.followers = out;
        let tri3 = std::mem::take(&mut self.tri3);
        let mut tri_out = HashMap::with_capacity(tri3.len());
        for ((pair, w), c) in tri3 {
            let u = map((pair >> 32) as u32);
            let v = map(pair as u32);
            tri_out.insert((pack_pair(u, v), map(w)), c);
        }
        self.tri3 = tri_out;
        let pc = std::mem::take(&mut self.prompt_counts);
        self.prompt_counts = pc.into_iter().map(|(id, c)| (map(id), c)).collect();
        self.base_len = self.vocab.len();
        self.reindex();
    }

    /// Learn from one submitted prompt. Multi-line or very long prompts
    /// are pastes and learn at [`PASTE_WEIGHT`].
    pub fn learn(&mut self, prompt: &str) {
        let words = prose_words(&strip_nonprose(prompt));
        if words.is_empty() {
            return;
        }
        let looks_pasted = prompt.lines().count() > 1 || words.len() > 80;
        let weight = if looks_pasted { PASTE_WEIGHT } else { 1.0 };
        let mut seen: Vec<u32> = Vec::new();
        let mut prev: Option<u32> = None;
        let mut prev2: Option<u32> = None;
        for word in &words {
            let id = self.intern(word);
            if !seen.contains(&id) {
                seen.push(id);
            }
            self.count[id as usize] += weight;
            self.total += weight;
            if let Some(p) = prev {
                *self
                    .followers
                    .entry(p)
                    .or_default()
                    .entry(id)
                    .or_insert(0.0) += weight;
                if let Some(pp) = prev2 {
                    let pair = pack_pair(pp, p);
                    *self.tri3.entry((pair, id)).or_insert(0.0) += weight;
                    *self.pair_totals.entry(pair).or_insert(0.0) += weight;
                    *self.pair_types.entry(pair).or_insert(0) += 1;
                }
            }
            prev2 = prev;
            prev = Some(id);
        }
        if weight >= 1.0 {
            for id in seen {
                *self.prompt_counts.entry(id).or_insert(0) += 1;
            }
        }
    }

    /// Bigram count c(v, w).
    pub fn bigram(&self, v: u32, w: u32) -> f64 {
        self.followers
            .get(&v)
            .and_then(|m| m.get(&w))
            .copied()
            .unwrap_or(0.0)
    }

    /// Trigram continuation count c(u, v, w).
    pub fn trigram(&self, u: u32, v: u32, w: u32) -> f64 {
        self.tri3.get(&(pack_pair(u, v), w)).copied().unwrap_or(0.0)
    }

    /// Bigram context stats: (total mass after `v`, distinct followers).
    pub fn follower_stats(&self, v: u32) -> (f64, usize) {
        match self.followers.get(&v) {
            Some(m) => (m.values().sum(), m.len()),
            None => (0.0, 0),
        }
    }

    /// Trigram pair stats: (total mass after `(u,v)`, distinct thirds).
    pub fn pair_stats(&self, u: u32, v: u32) -> (f64, u32) {
        let pair = pack_pair(u, v);
        (
            self.pair_totals.get(&pair).copied().unwrap_or(0.0),
            self.pair_types.get(&pair).copied().unwrap_or(0),
        )
    }

    // -- snapshot plumbing (snapshot.rs serializes these) ----------------

    pub(super) fn vocab_dump(&self) -> &[String] {
        &self.vocab
    }

    pub(super) fn count_dump(&self) -> &[f64] {
        &self.count
    }

    pub(super) fn prompt_counts_dump(&self) -> Vec<(u32, u32)> {
        self.prompt_counts.iter().map(|(k, v)| (*k, *v)).collect()
    }

    pub(super) fn bigrams_dump(&self) -> Vec<(u32, u32, f64)> {
        self.followers
            .iter()
            .flat_map(|(prev, inner)| inner.iter().map(move |(w, c)| (*prev, *w, *c)))
            .collect()
    }

    pub(super) fn trigrams_dump(&self) -> Vec<(u32, u32, u32, f64)> {
        self.tri3
            .iter()
            .map(|((pair, w), c)| ((pair >> 32) as u32, *pair as u32, *w, *c))
            .collect()
    }

    /// Rebuild a model from snapshot parts (ids are positions in `vocab`).
    pub(super) fn from_parts(
        vocab: Vec<String>,
        count: Vec<f64>,
        prompt_counts: HashMap<u32, u32>,
        bigrams: Vec<(u32, u32, f64)>,
        trigrams: Vec<(u32, u32, u32, f64)>,
        total: f64,
    ) -> Self {
        let mut followers: HashMap<u32, HashMap<u32, f64>> = HashMap::new();
        for (prev, w, c) in bigrams {
            *followers.entry(prev).or_default().entry(w).or_insert(0.0) = c;
        }
        let mut tri3 = HashMap::new();
        for (u, v, w, c) in trigrams {
            tri3.insert((pack_pair(u, v), w), c);
        }
        // pair_totals/pair_types stay empty here: rebuild_indexes()
        // recomputes them authoritatively from tri3, and decode() always
        // calls it right after from_parts.
        Self {
            base_len: 0, // rebuild_indexes()/compact() set the true base
            vocab,
            index: HashMap::new(),
            count,
            prompt_counts,
            followers,
            tri3,
            pair_totals: HashMap::new(),
            pair_types: HashMap::new(),
            total,
        }
    }

    /// Rebuild the word→id index and (defensively) the pair type counts.
    pub(super) fn rebuild_indexes(&mut self) {
        self.reindex();
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (i, w) in self.vocab.iter().enumerate() {
            self.index.insert(w.clone(), i as u32);
        }
        self.base_len = self.vocab.len();
        // Recompute pair totals/types from tri3 — the authoritative store.
        let mut pair_totals = HashMap::new();
        let mut pair_types: HashMap<u64, std::collections::HashSet<u32>> = HashMap::new();
        for ((pair, w), c) in &self.tri3 {
            *pair_totals.entry(*pair).or_insert(0.0) += c;
            pair_types.entry(*pair).or_default().insert(*w);
        }
        self.pair_totals = pair_totals;
        self.pair_types = pair_types
            .into_iter()
            .map(|(k, set)| (k, set.len() as u32))
            .collect();
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn prose_words_filters_non_prose() {
        let ws = prose_words("fix the parser and don't stop — 42 tokens");
        assert_eq!(
            ws,
            vec!["fix", "the", "parser", "and", "don't", "stop", "tokens"]
        );
        // learn() strips references and code spans first.
        let stripped = prose_words(&strip_nonprose(
            "check @file:src/main.rs and `let x = 1` ok",
        ));
        assert_eq!(stripped, vec!["check", "and", "ok"]);
    }

    #[test]
    fn counts_and_ranges() {
        let mut m = Model::default();
        for p in ["can you refactor the parser", "can you review the diff"] {
            m.learn(p);
        }
        let ids = m.prefix_ids("the");
        assert_eq!(ids.len(), 1);
        assert_eq!(m.word(ids[0]), "the");
        // 'the parser' and 'the diff' each occurred once.
        let the = m.id("the").expect("the");
        let parser = m.id("parser").expect("parser");
        let diff = m.id("diff").expect("diff");
        assert!((m.bigram(the, parser) - 1.0).abs() < 1e-9);
        assert!((m.bigram(the, diff) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn append_only_ids_never_shift_mid_learn() {
        let mut m = Model::default();
        m.learn("alpha zulu");
        m.learn("alpha bravo zulu"); // bravo appends AFTER zulu
        let ids = m.prefix_ids("z");
        assert_eq!(ids.len(), 1);
        assert_eq!(m.word(ids[0]), "zulu");
        let alpha = m.id("alpha").expect("alpha");
        let zulu = m.id("zulu").expect("zulu");
        // alpha→zulu occurred once; the word 'zulu' itself twice.
        assert!((m.bigram(alpha, zulu) - 1.0).abs() < 1e-9);
        assert!((m.unigram(zulu) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn hygiene_requires_two_prompts() {
        let mut m = Model::default();
        m.learn("once upon a uniquetypo");
        let id = m.id("uniquetypo").expect("learned");
        assert!(!m.eligible(id), "one-off junk never suggests");
        m.learn("again about uniquetypo");
        assert!(m.eligible(id));
    }

    #[test]
    fn pastes_learn_at_low_weight() {
        let mut m = Model::default();
        m.learn("line one\npastedword line two");
        let id = m.id("pastedword").expect("learned");
        assert!((m.unigram(id) - PASTE_WEIGHT).abs() < 1e-9);
        assert!(!m.eligible(id), "pastes never make a word suggestible");
    }
}
