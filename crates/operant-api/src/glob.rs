//! The one `*`-only glob matcher.
//!
//! Consolidates the two former runtime copies (`loop_support::glob_match`
//! and `webhook_audit::glob_matches`) behind a single implementation.
//!
//! ## Dialect — frozen
//!
//! `*` matches any run of characters, including empty. There is no `?`
//! and no character class, deliberately: both call sites gate TOOL
//! EXPOSURE (`filter_tool_specs_for_turn` — MCP tool-group patterns —
//! and the webhook tool allowlist), so adding fnmatch features would be
//! a permission widening, not a refactor. If `?`/`[...]` are ever
//! wanted, that is its own iteration with its own decision; the broader
//! `context::lcm::glob_match` already exists for surfaces that have
//! accepted that dialect.
//!
//! ## Semantics — ordered segments
//!
//! Star-free segments must appear in order: the first anchored at the
//! text start (unless the pattern starts with `*`), the last anchored at
//! the end (unless the pattern ends with `*`), the middles anywhere
//! between, with no overlap. For 0- and 1-star patterns this is
//! byte-identical to BOTH former implementations (verified by the
//! transplanted pins below). For multi-star patterns it is the ordered
//! semantics the webhook allowlist always had; `loop_support`'s former
//! first-star split treated everything after the second star as literal
//! text, so `a*b*c` could only match a name literally ending in `b*c` —
//! a latent bug, not a designed feature. No live config carries a
//! multi-star pattern at either gate (checked 2026-10-07).

/// `*`-only glob match. See the module docs for the frozen dialect.
pub fn star_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }

    let parts: Vec<&str> = pattern.split('*').collect();

    // First segment anchors the start unless the pattern starts with `*`.
    let mut pos = 0usize;
    if !pattern.starts_with('*') {
        let first = parts[0];
        if !text.starts_with(first) {
            return false;
        }
        pos = first.len();
    }

    // Last segment anchors the end unless the pattern ends with `*`.
    if !pattern.ends_with('*') {
        let last = parts[parts.len() - 1];
        if !text.ends_with(last) {
            return false;
        }
        // Reject overlap with the prefix already consumed: "ab*b" must
        // not match "ab".
        if text.len() < pos + last.len() {
            return false;
        }
    }

    let end_boundary = if pattern.ends_with('*') {
        text.len()
    } else {
        text.len() - parts[parts.len() - 1].len()
    };

    let start_idx = if pattern.starts_with('*') { 0 } else { 1 };
    let end_idx = if pattern.ends_with('*') {
        parts.len()
    } else {
        parts.len() - 1
    };

    for part in &parts[start_idx..end_idx] {
        if part.is_empty() {
            continue;
        }
        if let Some(found) = text[pos..end_boundary].find(part) {
            pos += found + part.len();
        } else {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::star_match;

    // Pins carried over from BOTH former implementations, so the
    // consolidation is provably byte-identical on their common dialect.
    #[test]
    fn exact_and_no_star() {
        assert!(star_match("file_write", "file_write"));
        assert!(!star_match("file_write", "file_read"));
    }

    #[test]
    fn star_matches_everything_including_empty() {
        assert!(star_match("*", ""));
        assert!(star_match("*", "anything at all"));
    }

    #[test]
    fn prefix_and_suffix_wildcards() {
        assert!(star_match("mcp__*", "mcp__github"));
        assert!(star_match("*_read", "file_read"));
        assert!(!star_match("mcp__*", "builtin_read"));
    }

    #[test]
    fn single_middle_star() {
        assert!(star_match("a*b", "aXXb"));
        assert!(star_match("a*b", "ab"));
        assert!(!star_match("a*b", "aX"));
        assert!(!star_match("a*b", "bXa"));
    }

    #[test]
    fn multi_star_segments_are_ordered_not_literal() {
        // The divergence the consolidation fixes: the former
        // loop_support first-star split could only see these as literal
        // suffixes; the shared matcher applies glob ordering.
        assert!(star_match("a*b*c", "aXbYc"));
        assert!(!star_match("a*b*c", "aXcYb"));
        assert!(star_match("*x*y*", "0x1y2"));
    }

    #[test]
    fn overlap_is_rejected() {
        assert!(!star_match("ab*b", "ab"));
        assert!(star_match("ab*b", "abxb"));
    }

    #[test]
    fn empty_pattern_matches_only_empty_text() {
        assert!(star_match("", ""));
        assert!(!star_match("", "x"));
    }
}
