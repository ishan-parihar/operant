//! Word-navigation classes for the composer's word-wise operations.
//!
//! Port of oh-my-pi's `getWordNavKind` word model (plan 2026-10-24, M1):
//! characters split into whitespace / word / delimiter / CJK classes, and a
//! word stop consumes a RUN of one class — so a punctuation run (`->`, `::`)
//! is one stop and a CJK run is one stop. Joiners (apostrophes and hyphens)
//! stay INSIDE a word when they sit between word characters, so `don't` and
//! `well-known` move as one stop. This mirrors the terminal-editor
//! expectation the OMP plan measured against; the previous three-class scan
//! (`is_word_char` in `vim.rs`, kept for vim's own `w`/`b`/`e` motions,
//! which use vim's different word rules) split those words at every
// joiner.
//!
//! Char-based, not grapheme-based: the composer's `String` is iterated as
//! `char`s everywhere else in `prompt_input`, and re-basing word motion on
//! grapheme clusters would fork the cursor model. Emoji ZWJ sequences will
//! stop mid-cluster; that is the accepted v1 trade from the plan.

/// Word-navigation class of one character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WordClass {
    /// Whitespace: skipped before a stop.
    Whitespace,
    /// Letters, digits, `_` — plus joiners between word characters.
    Word,
    /// Punctuation and symbol runs: one stop per run.
    Delimiter,
    /// Han/Hiragana/Katakana/Hangul runs: one stop per run.
    Cjk,
}

/// Characters that bind a word together when BETWEEN word characters
/// (OMP `WORD_NAV_JOINERS`): ASCII apostrophe, right single quote, and the
/// three hyphen glyphs that commonly appear in prose.
const JOINERS: [char; 5] = ['\'', '\u{2019}', '-', '\u{2010}', '\u{2011}'];

/// Han ideograph ranges (Unified, Ext A, compatibility ideographs).
fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// Hiragana + Katakana (incl. half-width and phonetic extensions).
fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D)
}

/// Hangul syllables + compatibility jamo.
fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7A3)
}

/// Classify one character. NOTE: CJK scripts are `char::is_alphanumeric()`
/// in Rust, so the CJK test must run before the word fallback — the same
/// ordering OMP's regex chain relies on.
pub(super) fn classify(c: char) -> WordClass {
    if c.is_whitespace() {
        WordClass::Whitespace
    } else if is_han(c) || is_kana(c) || is_hangul(c) {
        WordClass::Cjk
    } else if c == '_' || c.is_alphanumeric() {
        WordClass::Word
    } else {
        WordClass::Delimiter
    }
}

fn is_joiner(c: char) -> bool {
    JOINERS.contains(&c)
}

/// Char index where a backward word stop lands, starting from `start` in
/// `chars` (a slice scanned right-to-left). Whitespace before the cursor is
/// skipped, then the run of the class found there is consumed. A joiner is
/// consumed only when a word character sits on BOTH sides of it.
pub(super) fn stop_backward(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    match classify(chars[i - 1]) {
        WordClass::Whitespace => i,
        WordClass::Delimiter | WordClass::Cjk => {
            let kind = classify(chars[i - 1]);
            while i > 0 && classify(chars[i - 1]) == kind {
                i -= 1;
            }
            i
        }
        WordClass::Word => {
            let mut has_word_left = false;
            while i > 0 {
                let c = chars[i - 1];
                match classify(c) {
                    WordClass::Word => {
                        has_word_left = true;
                        i -= 1;
                    }
                    _ if has_word_left
                        && is_joiner(c)
                        && i >= 2
                        && classify(chars[i - 2]) == WordClass::Word =>
                    {
                        i -= 1;
                    }
                    _ => break,
                }
            }
            i
        }
    }
}

/// Char count consumed by a forward word stop starting at `start` in
/// `chars`. Whitespace at the cursor is skipped, then the run of the class
/// found there is consumed (joiners bind as in [`stop_backward`]). The
/// caller decides whether to keep walking over trailing whitespace —
/// cursor motion does (readline-style "next word start"), word deletes do
/// not.
pub(super) fn stop_forward(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i >= chars.len() {
        return chars.len();
    }
    match classify(chars[i]) {
        WordClass::Whitespace => i,
        WordClass::Delimiter | WordClass::Cjk => {
            let kind = classify(chars[i]);
            while i < chars.len() && classify(chars[i]) == kind {
                i += 1;
            }
            i
        }
        WordClass::Word => {
            let mut has_word_right = false;
            while i < chars.len() {
                let c = chars[i];
                match classify(c) {
                    WordClass::Word => {
                        has_word_right = true;
                        i += 1;
                    }
                    _ if has_word_right
                        && is_joiner(c)
                        && i + 1 < chars.len()
                        && classify(chars[i + 1]) == WordClass::Word =>
                    {
                        i += 1;
                    }
                    _ => break,
                }
            }
            i
        }
    }
}
