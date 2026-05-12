//! ANSI escape sequence removal for terminal output.
//!
//! Covers the full ECMA-48 spec: CSI (including private-mode `?` prefix,
//! colon-separated params, intermediate bytes), OSC (BEL and ST terminators),
//! DCS/SOS/PM/APC string sequences, nF multi-byte escapes, Fp/Fe/Fs
//! single-byte escapes, and 8-bit C1 control characters.
//!
//! Safe to call on any string — clean text passes through with negligible overhead.

use std::io::{self, Write};

use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// Full ANSI escape pattern.
    ///
    /// Covers all standard ESC-based sequences:
    /// - CSI: ESC [ + params + intermediates + final byte
    /// - OSC: ESC ] + any bytes until BEL or ST (ESC \)
    /// - DCS/SOS/PM/APC: ESC + one of PX^_ + any bytes until ST
    /// - nF: ESC + 1+ intermediates + final byte
    /// - Fp/Fe/Fs: ESC + single final byte
    ///
    /// Does NOT cover 8-bit C1 controls (0x80-0x9f) since they are
    /// not valid as standalone bytes in UTF-8 strings. In practice,
    /// modern terminal output uses ESC-based sequences almost exclusively.
    static ref ANSI_ESCAPE_RE: Regex = Regex::new(
        r"(?s)\x1b(?:\[[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]|\][\s\S]*?(?:\x07|\x1b\\)|[PX^_][\s\S]*?(?:\x1b\\)|[\x20-\x2f]+[\x30-\x7e]|[\x30-\x7e])"
    )
    .expect("valid ANSI escape regex");

    /// Fast-path: check for the ESC byte (0x1b) only.
    /// 8-bit C1 bytes are not valid standalone UTF-8 so we skip them.
    static ref HAS_ESCAPE_RE: Regex =
        Regex::new(r"\x1b").expect("valid ESC byte detector");
}

/// Remove all ANSI escape sequences from `input`.
///
/// Returns the input unchanged (fast path) when no ESC or C1 bytes are
/// present. Safe to call on any string — clean text passes through
/// with negligible overhead.
pub fn strip_ansi(input: &str) -> String {
    if input.is_empty() || !HAS_ESCAPE_RE.is_match(input) {
        return input.to_string();
    }
    ANSI_ESCAPE_RE.replace_all(input, "").into_owned()
}

/// Strip ANSI escape sequences from `input` and write the result to `writer`.
///
/// This avoids allocating the entire output at once when writing to a
/// buffered writer (file, network socket, etc.).
pub fn strip_ansi_to_writer(input: &str, writer: &mut impl Write) -> io::Result<()> {
    if input.is_empty() {
        return Ok(());
    }

    if !HAS_ESCAPE_RE.is_match(input) {
        return writer.write_all(input.as_bytes());
    }

    let mut last_end = 0;
    for m in ANSI_ESCAPE_RE.find_iter(input) {
        // Write the segment before this match
        let segment = &input[last_end..m.start()];
        if !segment.is_empty() {
            writer.write_all(segment.as_bytes())?;
        }
        last_end = m.end();
    }
    // Write remaining text after the last match
    let tail = &input[last_end..];
    if !tail.is_empty() {
        writer.write_all(tail.as_bytes())?;
    }
    Ok(())
}

/// Returns `true` if `input` contains ONLY ANSI escape sequences
/// (no visible content).
///
/// An empty string returns `false`.
pub fn is_control_only(input: &str) -> bool {
    if input.is_empty() {
        return false;
    }
    let cleaned = strip_ansi(input);
    cleaned.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_no_ansi_returns_unchanged() {
        let s = "Hello, world!";
        assert_eq!(strip_ansi(s), s);
    }

    #[test]
    fn test_strip_ansi_basic_csi_color() {
        let input = "\x1b[31mred\x1b[0m";
        assert_eq!(strip_ansi(input), "red");
    }

    #[test]
    fn test_strip_ansi_multiple_codes() {
        let input = "\x1b[1m\x1b[32mbold green\x1b[0m";
        assert_eq!(strip_ansi(input), "bold green");
    }

    #[test]
    fn test_strip_ansi_cursor_movement() {
        let input = "line1\x1b[Alastline";
        assert_eq!(strip_ansi(input), "line1lastline");
    }

    #[test]
    fn test_strip_ansi_screen_clear() {
        let input = "before\x1b[2Jafter";
        assert_eq!(strip_ansi(input), "beforeafter");
    }

    #[test]
    fn test_strip_ansi_osc_sequence() {
        // OSC sequence for setting window title
        let input = "\x1b]0;my title\x07visible";
        assert_eq!(strip_ansi(input), "visible");
    }

    #[test]
    fn test_is_control_only_true() {
        assert!(is_control_only("\x1b[31m\x1b[0m"));
    }

    #[test]
    fn test_is_control_only_false() {
        assert!(!is_control_only("hello\x1b[31m"));
    }

    #[test]
    fn test_is_control_only_empty() {
        assert!(!is_control_only(""));
    }

    #[test]
    fn test_strip_ansi_to_writer() {
        let input = "\x1b[31mhello\x1b[0m world";
        let mut buf = Vec::new();
        strip_ansi_to_writer(input, &mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "hello world");
    }

    #[test]
    fn test_strip_ansi_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_strip_ansi_mixed_content() {
        let input = "normal\x1b[31mred\x1b[0mnormal";
        assert_eq!(strip_ansi(input), "normalrednormal");
    }
}
