//! LLM-driven assertion extraction (hermes-lcm `assertion_extraction.py`
//! `ModelAssertionExtractor` parity, bounded).
//!
//! Hermes's opt-in extractor runs a structured LLM call over conversation
//! text and decodes durable (subject, predicate, object) assertion triples,
//! canonicalizing keys so the store's active-state resolution is stable.
//! This port mirrors that seam:
//!
//!   - [`AssertionExtractor`] — injectable async trait (testable with a fake,
//!     exactly like `rollup.rs`'s `Summarizer`).
//!   - [`LlmAssertionExtractor`] — the real LLM-backed implementation over the
//!     shared OpenAI-compatible client (`_call_structured_assertion_llm` +
//!     `build_structured_assertion_prompt` parity).
//!   - [`parse_assertion_payload`] — tolerant decode (markdown fences, bare
//!     array or `{"assertions":[...]}` envelope) + canonicalization
//!     (subject/predicate trimmed + lowercased, empties dropped, hard cap).
//!
//! Surfaced to the agent as `lcm_assert action="extract"` (opt-in via
//! `agent.context_lcm_assertion_extraction`), so the agent can mine durable
//! facts out of the lossless DAG without manually typing every fact.

use std::sync::Arc;

use async_trait::async_trait;

use crate::client::{Message, OpenAIClient};
use crate::error::{Error, Result};

/// One durable assertion extracted from conversation text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedAssertion {
    /// Canonical subject key, e.g. `project:hermes`, `user`, `assistant:self`.
    pub subject: String,
    /// Canonical predicate key, e.g. `prefers`, `uses`, `deadline`.
    pub predicate: String,
    /// The object value — the fact itself.
    pub object: String,
    /// Speaker attribution (`user` | `assistant` | `tool`).
    pub speaker: String,
}

/// Hard cap on the number of assertions decoded from one LLM payload —
/// a rogue or verbose model can never flood the assertion store.
pub const MAX_ASSERTIONS_PER_CALL: usize = 20;

/// Hard cap on transcript characters fed to the extractor per call (mirrors
/// the rollup summarizer's deterministic bound; char-boundary safe).
pub const MAX_TRANSCRIPT_CHARS: usize = 24_000;

/// Inject the extraction prompt into a chat call and decode the result.
/// `client`/`model` are injected so the same client config that drives the
/// agent also drives extraction (no separate credential seam).
#[derive(Clone)]
pub struct LlmAssertionExtractor {
    client: Arc<OpenAIClient>,
    model: String,
}

impl LlmAssertionExtractor {
    pub fn new(client: Arc<OpenAIClient>, model: String) -> Self {
        Self { client, model }
    }

    /// One raw LLM completion for the extraction prompt, returning the
    /// visible content with a `reasoning_content` fallback (hermes
    /// reasoning-model parity — some providers put the answer in the
    /// reasoning channel and emit no visible content).
    async fn extract_once(&self, system: &str, transcript: &str) -> Result<String> {
        let msgs = vec![
            Message::system(system),
            Message::user(transcript.to_string()),
        ];
        // 4096 max_tokens (hermes `max_tokens=4000` parity): reasoning
        // models burn the first chunk on `reasoning_content` and only then
        // emit the visible JSON — 1024 truncated before the answer
        // (completion_tokens == max, raw_len == 0) and 2048 still capped
        // rambling runs mid-prose on live testing. 4096 fits reasoning +
        // JSON; the longer window is safe because `lcm_assert` carries a
        // 180s per-tool timeout override.
        let resp = self
            .client
            .chat(&self.model, &msgs, None, Some(4096), Some(0.0))
            .await
            .map_err(|e| Error::Agent(format!("lcm assertion extract LLM call failed: {e}")))?;
        Ok(resp
            .choices
            .first()
            .map(|c| {
                c.message
                    .content
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| c.message.reasoning_content.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default())
    }

    /// One LLM completion + decode, returning the raw response, the parsed
    /// assertions, and whether the response was valid JSON at all (so the
    /// caller can tell "no facts" from "rambled in prose").
    async fn extract_and_parse(
        &self,
        system: &str,
        transcript: &str,
    ) -> Result<(String, Vec<ExtractedAssertion>, bool)> {
        let resp = self.extract_once(system, transcript).await?;
        let trimmed = strip_code_fences(&resp).trim();
        let was_valid = serde_json::from_str::<serde_json::Value>(trimmed).is_ok();
        let parsed = parse_assertion_payload(&resp);
        Ok((resp, parsed, was_valid))
    }
}

#[async_trait]
impl AssertionExtractor for LlmAssertionExtractor {
    fn name(&self) -> &str {
        "llm"
    }

    async fn extract(&self, transcript: &str) -> Result<Vec<ExtractedAssertion>> {
        let prompt = build_assertion_prompt("");
        let (mut resp, mut parsed, was_valid) = self.extract_and_parse(&prompt, transcript).await?;
        // Bounded single retry (hermes payload-strictness parity, softened):
        // free-tier reasoning models sometimes ramble past the token budget
        // into prose with no JSON at all. A second call with a hard
        // "JSON ONLY" reinforcement recovers most of those — never loop.
        //
        // The retry is gated on `!was_valid`: a legitimate `[]` /
        // `{"assertions":[]}` ("no durable facts") must NOT trigger a second
        // LLM call — that would amplify cost on the common no-facts case.
        if parsed.is_empty() && !was_valid {
            tracing::debug!(
                raw_len = resp.len(),
                "lcm assertion extractor: non-parseable response, retrying with JSON-only reinforcement"
            );
            let reinforced = format!(
                "{prompt}\n\nYour previous response was REJECTED because it contained \
                 prose instead of JSON. Output ONLY the raw JSON array now — no \
                 explanation, no commentary, no markdown."
            );
            if let Ok((retry, retried, _)) = self.extract_and_parse(&reinforced, transcript).await
                && !retried.is_empty()
            {
                resp = retry;
                parsed = retried;
            }
        }
        tracing::debug!(
            raw_len = resp.len(),
            parsed = parsed.len(),
            model = %self.model,
            "lcm assertion extractor LLM response"
        );
        Ok(parsed)
    }
}

/// The extractor seam (hermes `ModelAssertionExtractor` parity, bounded).
#[async_trait]
pub trait AssertionExtractor: Send + Sync {
    /// Human-readable extractor identity (diagnostics).
    fn name(&self) -> &str;
    /// Extract durable assertion triples from a conversation transcript.
    async fn extract(&self, transcript: &str) -> Result<Vec<ExtractedAssertion>>;
}

/// Build the structured extraction prompt (hermes
/// `build_structured_assertion_prompt` parity, bounded). `system_hint` is
/// reserved for a caller-supplied prefix; the transcript arrives as the
/// user message.
pub fn build_assertion_prompt(system_hint: &str) -> String {
    let hint = if system_hint.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n\n", system_hint.trim())
    };
    format!(
        "{hint}You are a strict assertion extractor. From the conversation \
         transcript below, extract ONLY durable assertions: stable facts, \
         explicit preferences, decisions, project constraints, and agreed \
         values. Ignore transient chat, greetings, hedges, and procedural \
         noise.\n\nRespond with JSON only, exactly one of:\n\
         {{ \"assertions\": [ {{\"subject\": \"...\", \"predicate\": \"...\", \"object\": \"...\", \"speaker\": \"user|assistant\"}} ] }}\n\
         or a bare JSON array of the same objects.\n\nRules:\n\
         - subject: the entity the fact is about; use a short stable key \
         (e.g. project:name, user, assistant:self).\n\
         - predicate: a short verb/noun key (e.g. prefers, uses, deadline, \
         decided, constraint).\n\
         - object: the fact itself, concise but complete.\n\
         - speaker: the role that stated the fact.\n\
         - Extract at most 20 assertions. Return an empty array when there \
         are no durable assertions.\n\
         - Output raw JSON only — no markdown fences, no commentary."
    )
}

/// Tolerant decode + canonicalization of an LLM assertion payload
/// (hermes `decode_assertion_payload` + canonicalization parity).
///
/// Accepts a markdown-fenced block, a bare `[...]` array, or an
/// `{"assertions": [...]}` envelope. Non-object entries, empty
/// subject/predicate/object, and the overflow beyond
/// [`MAX_ASSERTIONS_PER_CALL`] are dropped. Malformed input yields an empty
/// vector (never an error — extraction is best-effort).
///
/// Robustness beyond hermes's strict `decode_assertion_payload` (which
/// raises): free-tier models wrap the JSON in prose, so the body is also
/// scanned for an embedded `[...]`/`{"assertions":...}` payload when the
/// whole-body parse fails.
pub fn parse_assertion_payload(raw: &str) -> Vec<ExtractedAssertion> {
    let body = strip_code_fences(raw);
    match decode_payload(body) {
        Some(out) => out,
        None => {
            // Salvage: locate the first array or envelope marker and cut the
            // balanced JSON span out of the surrounding prose ("Here is the
            // JSON: [...] That is all."), then decode that span.
            let mut markers = Vec::new();
            if let Some(start) = body.find('[') {
                markers.push(start);
            }
            if let Some(start) = body.find("{\"assertions\"") {
                markers.push(start);
            }
            markers.sort_unstable();
            for start in markers {
                if let Some(end) = balanced_json_end(body, start)
                    && let Some(out) = decode_payload(&body[start..end])
                    && !out.is_empty()
                {
                    return out;
                }
            }
            Vec::new()
        }
    }
}

/// Find the end (exclusive byte index) of the JSON value starting at
/// `start`, balancing `{}`/`[]` while skipping string contents. Returns
/// `None` when the value is unterminated.
fn balanced_json_end(body: &str, start: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Decode one strict body (array or envelope) into assertions.
fn decode_payload(body: &str) -> Option<Vec<ExtractedAssertion>> {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let array = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(map) => match map.get("assertions") {
            Some(serde_json::Value::Array(arr)) => arr.clone(),
            // Valid JSON but not an assertion payload — a clean empty
            // result, not an error (no salvage: the body parsed fine).
            _ => return Some(Vec::new()),
        },
        _ => return Some(Vec::new()),
    };

    let mut out = Vec::with_capacity(array.len().min(MAX_ASSERTIONS_PER_CALL));
    for entry in array.into_iter().take(MAX_ASSERTIONS_PER_CALL) {
        let obj = match entry {
            serde_json::Value::Object(m) => m,
            _ => continue,
        };
        let subject = canonical_key(obj.get("subject").and_then(|v| v.as_str()).unwrap_or(""));
        let predicate = canonical_key(obj.get("predicate").and_then(|v| v.as_str()).unwrap_or(""));
        let object = obj
            .get("object")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if subject.is_empty() || predicate.is_empty() || object.is_empty() {
            continue;
        }
        let speaker = obj
            .get("speaker")
            .and_then(|v| v.as_str())
            .unwrap_or("assistant")
            .trim()
            .to_lowercase();
        let speaker = match speaker.as_str() {
            "user" | "assistant" | "tool" => speaker,
            _ => "assistant".to_string(),
        };
        out.push(ExtractedAssertion {
            subject,
            predicate,
            object,
            speaker,
        });
    }
    Some(out)
}

/// Strip ```json ... ``` fences (and a trailing comma before the closing
/// fence, a common LLM slip).
fn strip_code_fences(raw: &str) -> &str {
    let mut body = raw.trim();
    if let Some(rest) = body.strip_prefix("```") {
        // Drop the language tag line (e.g. ```json) up to the first newline.
        if let Some(nl) = rest.find('\n') {
            body = rest[nl + 1..].trim();
        } else {
            body = rest.trim();
        }
        if let Some(stripped) = body.strip_suffix("```") {
            body = stripped.trim();
        }
    }
    body.trim_end_matches(',')
}

/// Canonical subject/predicate key: trim, lowercase, collapse internal
/// whitespace runs. The colon is deliberately NOT rearranged — the explicit
/// `lcm_assert` save path stores keys verbatim, so an extractor-produced
/// `project:hermes` must match a user-saved `project:hermes` (adding a
/// space after the colon would silently break that match).
fn canonical_key(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Bound a raw transcript deterministically for the extractor: cap the
/// character count on a UTF-8 char boundary (mirror rollup truncation).
pub fn truncate_transcript(transcript: &str) -> String {
    let mut bound = MAX_TRANSCRIPT_CHARS.min(transcript.len());
    while !transcript.is_char_boundary(bound) {
        bound -= 1;
    }
    transcript[..bound].to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn parses_bare_array() {
        let payload = r#"[{"subject":"project:hermes","predicate":"prefers","object":"Rust over Python","speaker":"user"}]"#;
        let out = parse_assertion_payload(payload);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].subject, "project:hermes");
        assert_eq!(out[0].predicate, "prefers");
        assert_eq!(out[0].object, "Rust over Python");
        assert_eq!(out[0].speaker, "user");
    }

    #[test]
    fn canonical_keys_match_explicit_save_keys() {
        // The extractor's canonical form must match the verbatim keys the
        // `lcm_assert` save action stores, or query-by-subject would miss
        // mined facts.
        let payload = r#"[{"subject":"Project:hermes","predicate":"Prefers","object":"Rust","speaker":"assistant"}]"#;
        let out = parse_assertion_payload(payload);
        assert_eq!(out[0].subject, "project:hermes");
        assert_eq!(out[0].predicate, "prefers");
    }

    #[test]
    fn parses_envelope_and_fences() {
        let payload = "```json\n{\"assertions\":[{\"subject\":\"User\",\"predicate\":\"deadline\",\"object\":\"Aug 30\",\"speaker\":\"user\"}]}\n```";
        let out = parse_assertion_payload(payload);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].subject, "user", "subject canonicalized lower");
        assert_eq!(out[0].predicate, "deadline");
        assert_eq!(out[0].object, "Aug 30");
    }

    #[test]
    fn canonicalizes_keys_and_drops_empties() {
        let payload = r#"[
            {"subject":"  Project: Hermes ","predicate":" Uses ","object":"Rust","speaker":"assistant"},
            {"subject":"x","predicate":"","object":"empty predicate"},
            {"subject":"","predicate":"p","object":"empty subject"},
            {"subject":"k","predicate":"p","object":""},
            "not an object"
        ]"#;
        let out = parse_assertion_payload(payload);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].subject, "project: hermes");
        assert_eq!(out[0].predicate, "uses");
        assert_eq!(out[0].speaker, "assistant");
    }

    #[test]
    fn caps_at_max_assertions() {
        let entries: Vec<String> = (0..30)
            .map(|i| {
                format!(
                    r#"{{"subject":"s{i}","predicate":"p","object":"o{i}","speaker":"assistant"}}"#
                )
            })
            .collect();
        let payload = format!("[{}]", entries.join(","));
        let out = parse_assertion_payload(&payload);
        assert_eq!(out.len(), MAX_ASSERTIONS_PER_CALL);
    }

    #[test]
    fn malformed_payload_returns_empty() {
        assert!(parse_assertion_payload("not json at all").is_empty());
        assert!(parse_assertion_payload("{\"assertions\": 42}").is_empty());
        assert!(parse_assertion_payload("").is_empty());
    }

    #[test]
    fn salvages_json_embedded_in_prose() {
        // Free-tier models wrap the JSON in prose ("Here is the JSON:").
        // The parser must salvage the embedded array rather than drop it.
        let payload = "Here are the durable facts I found: \
            [{\"subject\":\"project:hermes\",\"predicate\":\"prefers\",\"object\":\"Rust\",\"speaker\":\"assistant\"}] \
            That is all.";
        let out = parse_assertion_payload(payload);
        assert_eq!(out.len(), 1, "embedded array must be salvaged");
        assert_eq!(out[0].subject, "project:hermes");

        // Envelope form embedded in prose as well.
        let envelope = "Sure — {\"assertions\":[{\"subject\":\"user\",\"predicate\":\"deadline\",\"object\":\"Aug 30\",\"speaker\":\"user\"}]} done.";
        let out = parse_assertion_payload(envelope);
        assert_eq!(out.len(), 1, "embedded envelope must be salvaged");
        assert_eq!(out[0].predicate, "deadline");
    }

    #[test]
    fn prose_without_json_still_returns_empty() {
        // The retry path depends on this: pure rambling prose must yield
        // zero assertions so the extractor knows to re-ask.
        assert!(parse_assertion_payload("We need to extract ONLY durable assertions from the conversation transcript. Let me think...").is_empty());
    }

    #[test]
    fn empty_array_is_valid_json_not_a_retry_trigger() {
        // A legitimate "no durable facts" answer (empty array / empty
        // envelope) is VALID JSON — it must not look like a prose ramble
        // that needs the expensive JSON-only retry.
        let raw = "[]";
        let trimmed = strip_code_fences(raw).trim();
        assert!(serde_json::from_str::<serde_json::Value>(trimmed).is_ok());
        assert!(parse_assertion_payload(raw).is_empty());

        let raw = r#"{"assertions":[]}"#;
        let trimmed = strip_code_fences(raw).trim();
        assert!(serde_json::from_str::<serde_json::Value>(trimmed).is_ok());
        assert!(parse_assertion_payload(raw).is_empty());
    }

    #[test]
    fn prompt_is_structured_and_strict() {
        let prompt = build_assertion_prompt("");
        assert!(prompt.contains("\"assertions\""));
        assert!(prompt.contains("subject"));
        assert!(prompt.contains("predicate"));
        assert!(prompt.contains("no markdown fences"));
    }

    #[test]
    fn truncate_keeps_char_boundaries() {
        let transcript = "héllo 🚀 ".repeat(10_000); // multi-byte content
        let out = truncate_transcript(&transcript);
        assert!(out.len() <= MAX_TRANSCRIPT_CHARS);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn speaker_defaults_and_validates() {
        let payload = r#"[{"subject":"a","predicate":"p","object":"o","speaker":"garbage"}]"#;
        let out = parse_assertion_payload(payload);
        assert_eq!(out[0].speaker, "assistant", "unknown speaker defaults");
        let payload = r#"[{"subject":"a","predicate":"p","object":"o"}]"#;
        let out = parse_assertion_payload(payload);
        assert_eq!(out[0].speaker, "assistant", "missing speaker defaults");
    }

    /// Deterministic fake extractor for tool-level tests.
    pub struct FakeExtractor {
        pub assertions: Vec<ExtractedAssertion>,
    }

    #[async_trait]
    impl AssertionExtractor for FakeExtractor {
        fn name(&self) -> &str {
            "fake"
        }

        async fn extract(&self, _transcript: &str) -> Result<Vec<ExtractedAssertion>> {
            Ok(self.assertions.clone())
        }
    }
}
