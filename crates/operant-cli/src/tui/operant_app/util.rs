// Vendored from jcode (crates/operant-core/src/util.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// the token-estimation helpers the renderers use: ApproxTokenSeverity,
// format_approx_token_count, approx_tool_output_token_severity.
//! (Upstream reaches these via operant-base/src/util.rs:1 `pub use operant_core::util::*`.)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproxTokenSeverity {
    Normal,
    Warning,
    Danger,
}

// [port-decision] dedup: ApproxTokenSeverity defined twice across concatenated
// sources; kept the first (identical) copy.

/// Estimate token count with the canonical estimator
/// (`operant_core::context_management::estimate_tokens`): chars / 4.
pub fn estimate_tokens(s: &str) -> usize {
    operant_core::context_management::estimate_tokens(s)
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
