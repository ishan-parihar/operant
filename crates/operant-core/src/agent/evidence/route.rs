//! Splitting a tool result into the parts worth compressing.
//!
//! Most tool results are JSON envelopes: `{"exit_code":0,"stdout":"…","stderr":"…"}`.
//! Compressing the envelope as one blob would compress the metadata along
//! with the payload and hand the model a `stdout` it can no longer read.
//! Routing extracts the payload fields and leaves the metadata alone.

use super::classify::subject_for;
use super::types::{ContentFormat, Subject};

/// One compressible part of a tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// The top-level key the text came from. Empty when the whole payload is
    /// the text.
    pub path: String,
    /// The text to compress.
    pub text: String,
    /// The shape of this text.
    pub format: ContentFormat,
}

/// How a tool result decomposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePlan {
    /// Whether the original content was JSON and must be re-assembled as JSON.
    pub is_json: bool,
    /// The parts to compress, in document order.
    pub segments: Vec<Segment>,
}

/// The payload keys of each family: the fields that carry the bulk.
///
/// Everything else — `exit_code`, `runtime`, `hint`, `truncated`, counts,
/// status flags — is metadata, and metadata is left exactly as it is.
///
/// `stderr` is metadata too, and deliberately so. The plan's failure-mode
/// table names it: "compression destroys the failure detail the model needed"
/// is what happens when a chain runs over the whole payload, and the guard
/// it names is that the router never routes `stderr`. A long stderr used to
/// be capped by the Term chain, which is exactly the loss the guard exists
/// to prevent.
fn payload_keys(subject: Subject) -> &'static [&'static str] {
    match subject {
        Subject::Shell => &["stdout"],
        Subject::Web => &["text", "content", "body", "data", "markdown"],
        Subject::Search => &["matches", "results"],
        Subject::Read => &["content", "text"],
        _ => &[],
    }
}

/// Decompose a tool result into compressible segments.
///
/// A JSON result yields one segment per payload key; a non-JSON result yields
/// a single segment holding the whole text.
pub fn route(tool_name: &str, content: &str, format: ContentFormat) -> RoutePlan {
    let trimmed = content.trim_start();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return whole(content, format, false);
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return whole(content, format, false);
    };

    let Some(object) = value.as_object() else {
        // A JSON ARRAY is one payload, not a set of fields.
        return whole(content, format, true);
    };

    let keys = subject_for(tool_name).map(payload_keys).unwrap_or(&[]);
    let mut segments = Vec::new();
    for (key, value) in object {
        if !keys.contains(&key.as_str()) {
            continue;
        }
        let Some(text) = payload_text(value, format) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        segments.push(Segment {
            path: key.clone(),
            text,
            format,
        });
    }

    if segments.is_empty() {
        // No recognised payload field: compress the whole document rather
        // than compress nothing.
        segments.push(Segment {
            path: String::new(),
            text: content.to_string(),
            format,
        });
    }

    RoutePlan {
        is_json: true,
        segments,
    }
}

/// The whole document as one segment.
fn whole(content: &str, format: ContentFormat, is_json: bool) -> RoutePlan {
    RoutePlan {
        is_json,
        segments: vec![Segment {
            path: String::new(),
            text: content.to_string(),
            format,
        }],
    }
}

/// The text of a payload field.
///
/// A string is the payload. A LIST of matches is flattened into the
/// `path:line:text` shape the matches chain understands; any other list is
/// flattened into one JSON document per element, so a JSON-shaped chain can
/// still see it.
fn payload_text(value: &serde_json::Value, format: ContentFormat) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(items) => {
            if format == ContentFormat::Matches {
                match_lines(items)
            } else {
                let mut parts = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        serde_json::Value::String(text) => parts.push(text.clone()),
                        other => parts.push(other.to_string()),
                    }
                }
                Some(parts.join("\n"))
            }
        }
        serde_json::Value::Object(_) if format == ContentFormat::Json => Some(value.to_string()),
        _ => None,
    }
}

/// Flatten a list of matches into `path:line:text` lines.
///
/// An element the matches chain would not recognise is refused outright: the
/// chain groups by the text before the first colon, and a JSON blob would be
/// grouped into nonsense.
fn match_lines(items: &[serde_json::Value]) -> Option<String> {
    let mut lines = Vec::with_capacity(items.len());
    for item in items {
        match item {
            serde_json::Value::String(text) => lines.push(text.clone()),
            serde_json::Value::Object(map) => {
                let file = map
                    .get("file")
                    .or_else(|| map.get("path"))
                    .and_then(|value| value.as_str());
                let number = map.get("line").and_then(|value| value.as_u64());
                let body = map
                    .get("content")
                    .or_else(|| map.get("text"))
                    .and_then(|value| value.as_str());
                match (file, number, body) {
                    (Some(file), Some(number), Some(body)) => {
                        lines.push(format!("{file}:{number}:{body}"));
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    Some(lines.join("\n"))
}
