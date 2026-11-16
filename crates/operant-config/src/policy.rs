//! The one security-policy primitive that is still live: the shell-command
//! splitter.
//!
//! (p2c, 2026-11-16) This file was 3,791 lines holding a second, parallel
//! security-policy surface: `CommandPolicy`, an autonomy ladder, a deny-list
//! engine, risk scoring, per-path justification rules and their tests. NONE of
//! it had a caller outside this crate -- the live gate is
//! `operant_core::blast_radius`, and the live config is
//! `operant_core::config::AppConfig`. Only ONE function here is imported
//! anywhere else in the workspace:
//! [`split_unquoted_segments`], called from
//! `operant_core::tools::file_tools` so the Holon command fence applies the
//! same rule a shell would (RT-454: one splitter, one rule).
//!
//! Everything else was deleted. If a new caller needs a policy primitive,
//! check `blast_radius` FIRST -- adding a second one here is how the 3,791
//! lines accumulated.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    None,
    Single,
    Double,
}

///
/// Separators:
/// - `;` and newline
/// - `|`
/// - `&&`, `||`
///
/// Characters inside single or double quotes are treated as literals, so
/// `sqlite3 db "SELECT 1; SELECT 2;"` remains a single segment.
///
/// Public because the Holon command fence applies the SAME rule to the
/// lines that run through a shell (RT-454): one splitter, one rule.
pub fn split_unquoted_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = QuoteState::None;
    let mut escaped = false;
    let mut chars = command.chars().peekable();

    let push_segment = |segments: &mut Vec<String>, current: &mut String| {
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            segments.push(trimmed.to_string());
        }
        current.clear();
    };

    while let Some(ch) = chars.next() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
                current.push(ch);
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    current.push(ch);
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    current.push(ch);
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                }
                current.push(ch);
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    current.push(ch);
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    current.push(ch);
                    continue;
                }

                match ch {
                    '\'' => {
                        quote = QuoteState::Single;
                        current.push(ch);
                    }
                    '"' => {
                        quote = QuoteState::Double;
                        current.push(ch);
                    }
                    ';' | '\n' => push_segment(&mut segments, &mut current),
                    '|' => {
                        if chars.next_if_eq(&'|').is_some() {
                            // Consume full `||`; both characters are separators.
                        }
                        push_segment(&mut segments, &mut current);
                    }
                    '&' => {
                        if chars.next_if_eq(&'&').is_some() {
                            // `&&` is a separator; single `&` is handled separately.
                            push_segment(&mut segments, &mut current);
                        } else {
                            current.push(ch);
                        }
                    }
                    _ => current.push(ch),
                }
            }
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (p2c) The splitter is the ONLY reason this crate still exists as a
    /// dependency of `operant-core`, so its contract is pinned here rather
    /// than only at the call site in `file_tools`.
    #[test]
    fn separators_split_outside_quotes_and_not_inside() {
        let segs = split_unquoted_segments("a; b | c && d || e\nf");
        assert_eq!(segs, vec!["a", "b", "c", "d", "e", "f"]);
    }

    /// The quote rule is the whole reason this function exists rather than a
    /// naive `split` -- a `;` inside a SQL string is data, not a separator.
    #[test]
    fn a_separator_inside_quotes_is_data_not_a_split() {
        let segs = split_unquoted_segments("sqlite3 db \"SELECT 1; SELECT 2;\"");
        assert_eq!(
            segs,
            vec![r#"sqlite3 db "SELECT 1; SELECT 2;""#],
            "the quoted semicolons must not split the statement"
        );
        let segs = split_unquoted_segments("echo 'a | b'");
        assert_eq!(segs, vec!["echo 'a | b'"]);
    }

    /// Escapes inside double quotes do not close the quote; a backslash outside
    /// quotes escapes the next character either way.
    #[test]
    fn an_escaped_quote_does_not_end_the_string() {
        let segs = split_unquoted_segments(r#"echo "a \" ; b""#);
        assert_eq!(segs, vec![r#"echo "a \" ; b""#]);
    }

    /// Empty and whitespace-only segments are dropped, so a trailing `;`
    /// produces no empty tail.
    #[test]
    fn empty_segments_are_dropped() {
        assert!(split_unquoted_segments("").is_empty());
        assert!(split_unquoted_segments("   ").is_empty());
        assert_eq!(split_unquoted_segments("a; ; b"), vec!["a", "b"]);
        assert_eq!(split_unquoted_segments("a;"), vec!["a"]);
    }
}
