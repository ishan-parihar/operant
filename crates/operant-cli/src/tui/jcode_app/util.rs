// Vendored from jcode (crates/jcode-core/src/util.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// the token-estimation helpers the renderers use: APPROX_CHARS_PER_TOKEN (:17),
// ApproxTokenSeverity (:20), estimate_tokens (:27), format_number (:32),
// format_approx_token_count (:45), approx_tool_output_token_severity (:62).
//! (Upstream reaches these via jcode-base/src/util.rs:1 `pub use jcode_core::util::*`.)

pub const APPROX_CHARS_PER_TOKEN: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproxTokenSeverity {
    Normal,
    Warning,
    Danger,
}

// [port-decision] dedup: ApproxTokenSeverity defined twice across concatenated
// sources; kept the first (identical) copy.

/// Estimate token count using jcode's existing chars-per-token heuristic.
pub fn estimate_tokens(s: &str) -> usize {
    s.len() / APPROX_CHARS_PER_TOKEN
}

/// Format a number with ASCII thousands separators.
pub fn format_number(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0 && (digits.len() - idx).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Format a token count in the compact style used by the TUI.
pub fn format_approx_token_count(tokens: usize) -> String {
    match tokens {
        0..=999 => format!("{} tok", tokens),
        1_000..=9_999 => {
            let whole = tokens / 1_000;
            let tenth = (tokens % 1_000) / 100;
            if tenth == 0 {
                format!("{}k tok", whole)
            } else {
                format!("{}.{}k tok", whole, tenth)
            }
        }
        _ => format!("{}k tok", tokens / 1_000),
    }
}

/// Light severity levels for tool outputs that are unusually large for context.
pub fn approx_tool_output_token_severity(tokens: usize) -> ApproxTokenSeverity {
    if tokens >= 12_000 {
        ApproxTokenSeverity::Danger
    } else if tokens >= 4_000 {
        ApproxTokenSeverity::Warning
    } else {
        ApproxTokenSeverity::Normal
    }
}

/// Truncate a string at a valid UTF-8 character boundary.
///
/// Returns a slice of at most `max_bytes` bytes, ending at a valid char boundary.
/// This prevents panics when truncating strings that contain multi-byte characters.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Find the largest valid char boundary at or before max_bytes
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
