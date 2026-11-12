//! The evidence layer: the point where a tool result becomes model-visible text.
//!
//! Operant already shrinks tool results — a 4 096-character cap and a
//! share-of-window guard. Both decide **how much** to keep; neither decides
//! **what** to keep. This layer adds the missing half: a result is described
//! by `(Subject, ContentFormat)`, that pair selects a chain of methods that
//! know the shape, and every method declares whether it can lose information
//! and how the lost information comes back.
//!
//! Three properties hold for every compression this layer performs:
//!
//! * **never worse** — a result is never larger after compression than before;
//! * **fail-open** — a method that panics leaves the payload untouched;
//! * **recoverable** — a lossy method may only run when the full text was
//!   written to the recall store.
//!
//! The layer runs at the tool-result chokepoint, *after* secret redaction and
//! *before* the spend guard, so no transformation or recall record ever sees
//! an unredacted byte, and the guard measures the residual cost of a result
//! that shaping could not shrink.

pub mod chain;
pub mod classify;
pub mod methods;
pub mod recall;
pub mod route;
pub mod types;

use serde::{Deserialize, Serialize};

pub use chain::{ChainOutcome, run_chain_guarded, run_chain_traced};
pub use types::{
    Block, ContentFormat, CostClass, Method, MethodCtx, MethodInfo, MethodKind, Origin, Outcome,
    Produces, Recovery, Subject,
};

/// How shell results are compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellMode {
    /// AFT already compresses shell output; the layer leaves it alone.
    #[default]
    Aft,
    /// The layer compresses the shell payload itself.
    Native,
    /// The `rtk` binary compresses the shell output.
    Rtk,
}

/// A per-family override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FamilyConfig {
    /// Whether this family is compressed at all.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// The named chain to use, instead of the one the format selects.
    #[serde(default)]
    pub chain: Option<String>,
}

/// The evidence layer's configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceConfig {
    /// Whether the layer compresses anything at all.
    #[serde(default)]
    pub enabled: bool,
    /// Results smaller than this are left alone.
    #[serde(default = "default_min_size")]
    pub min_size: usize,
    /// How shell results are compressed.
    #[serde(default)]
    pub shell: ShellMode,
    /// Per-family overrides, keyed by subject name.
    #[serde(default)]
    pub families: std::collections::BTreeMap<String, FamilyConfig>,
}

fn default_min_size() -> usize {
    512
}

impl Default for EvidenceConfig {
    fn default() -> Self {
        EvidenceConfig {
            enabled: false,
            min_size: default_min_size(),
            shell: ShellMode::default(),
            families: std::collections::BTreeMap::new(),
        }
    }
}

/// What the layer did to one result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compressed {
    /// The text to hand to the model.
    pub text: String,
    /// What produced the result.
    pub subject: Subject,
    /// The shape the result was treated as.
    pub format: ContentFormat,
    /// The methods that changed the text.
    pub applied: Vec<&'static str>,
    /// Whether any applied method declared itself lossy.
    ///
    /// The flag travels with the result rather than being re-derived from the
    /// method registry at the call site: the call site needs it to decide
    /// whether the FULL text must be written to the recall store BEFORE the
    /// compressed text is handed to the model, and a flag that lives with the
    /// result cannot drift from the chain that produced it.
    pub lossy: bool,
    /// The estimated token count before compression.
    pub tokens_before: usize,
    /// The estimated token count after compression.
    pub tokens_after: usize,
}

impl Compressed {
    /// A result the layer left alone.
    pub fn unchanged(subject: Subject, format: ContentFormat, text: &str) -> Self {
        let tokens = estimate_tokens(text);
        Compressed {
            text: text.to_string(),
            subject,
            format,
            applied: Vec::new(),
            lossy: false,
            tokens_before: tokens,
            tokens_after: tokens,
        }
    }

    /// Whether the layer changed anything.
    pub fn changed(&self) -> bool {
        !self.applied.is_empty()
    }
}

/// The layer's token estimate: four characters per token.
///
/// It is operant's CANONICAL estimator, re-exported rather than copied. The
/// copy that used to live here counted BYTES while the canonical one counts
/// CHARACTERS (the CJK fix from iter-18), so on multibyte text it
/// over-counted by up to 4× — and the numbers in a provenance marker must
/// mean the same thing as the numbers in the context footer.
pub use crate::context_management::estimate_tokens;

/// Compress one tool result.
///
/// Returns the result unchanged when the layer is disabled, when the payload
/// is below `min_size`, when the tool is unknown, or when compression would
/// not make the payload smaller.
pub fn compress(tool_name: &str, content: &str, cfg: &EvidenceConfig) -> Compressed {
    let (subject, format) = match classify::classify(tool_name, content) {
        Some(pair) => pair,
        None => {
            return Compressed::unchanged(Subject::ToolResult, ContentFormat::Plain, content);
        }
    };

    // The SHELL path is owned by another compressor: AFT compresses bash
    // in-process, and the `rtk` binary is the terminal tool's executor. The
    // layer must not compress what they already compressed — a second pass
    // over rtk's reduced output would strip the very handles rtk prints for
    // its own recovery. `Native` is the mode where the layer compresses the
    // shell payload itself.
    if subject == Subject::Shell && cfg.shell != ShellMode::Native {
        return Compressed::unchanged(subject, format, content);
    }

    // The per-family override decides whether this family is compressed, and
    // it WINS over the master switch: an operator can turn ONE family on
    // without turning on everything, or one family off, without a rebuild.
    let family = cfg.families.get(subject.as_str());
    let enabled = family.and_then(|f| f.enabled).unwrap_or(cfg.enabled);
    if !enabled || content.len() < cfg.min_size {
        return Compressed::unchanged(subject, format, content);
    }

    // Inline media is a DELIVERY, not a payload: the bytes are the tool's
    // output. The existing spend guard already exempts `data:image/`
    // (agent/compress.rs:145) and the layer must not second-guess it — a
    // base64 blob inside a data URL is not a compressible blob.
    if content.contains("data:image/") || content.contains("data:audio/") {
        return Compressed::unchanged(subject, format, content);
    }

    let plan = route::route(tool_name, content, format);
    let origin = Origin {
        tool: Some(tool_name.to_string()),
        path: envelope_path(content),
        command: None,
    };

    let mut applied: Vec<&'static str> = Vec::new();
    let mut compressed_segments: Vec<(String, String)> = Vec::with_capacity(plan.segments.len());
    let mut lossy = false;

    // The family may name its own chain. An unknown name falls back to the
    // format's chain: a typo in the config must not silently disable
    // compression, and must not fail the result either.
    let named = family
        .and_then(|f| f.chain.as_deref())
        .and_then(methods::named_chain);

    for segment in &plan.segments {
        let chain = named.unwrap_or_else(|| methods::chain_for(segment.format));
        let block = Block::new(&segment.text, subject, segment.format, &origin);
        let ctx = MethodCtx::new(tool_name);
        match chain::run_chain_guarded(chain, &block, &ctx) {
            Some(outcome) => {
                applied.extend(outcome.applied.iter().copied());
                lossy |= outcome.lossy;
                compressed_segments.push((segment.path.clone(), outcome.text));
            }
            None => compressed_segments.push((segment.path.clone(), segment.text.clone())),
        }
    }

    // A single segment with no key holds the WHOLE document: a JSON array, or
    // a JSON object with no recognised payload field. Its compressed text
    // replaces the document outright — it may no longer be JSON (a table
    // becomes TOON), which is the point, and the model reads it as text.
    let text = if plan.segments.len() == 1 && plan.segments[0].path.is_empty() {
        compressed_segments[0].1.clone()
    } else {
        reassemble(plan.is_json, content, &compressed_segments)
    };

    // The never-worse guarantee is about the whole result, not one segment.
    if text.len() >= content.len() {
        return Compressed::unchanged(subject, format, content);
    }

    let tokens_before = estimate_tokens(content);
    let tokens_after = estimate_tokens(&text);

    Compressed {
        text,
        subject,
        format,
        applied,
        lossy,
        tokens_before,
        tokens_after,
    }
}

/// The file path a tool result's envelope names, if it names one.
///
/// The code filter needs it: without the extension it cannot pick a grammar,
/// and a conservative text fallback would strip far less than a
/// string-aware Rust or Python scanner. A read's result carries its path in
/// the envelope, so the path is read from the same document.
fn envelope_path(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let object = value.as_object()?;
    for key in ["path", "file", "file_path", "filename"] {
        if let Some(path) = object.get(key).and_then(|v| v.as_str())
            && !path.is_empty()
        {
            return Some(path.to_string());
        }
    }
    None
}

/// Put the compressed segments back into the shape the tool produced.
///
/// Only the keyed case needs re-assembly: a segment with an empty path holds
/// the whole document and is returned by the caller directly.
fn reassemble(is_json: bool, original: &str, segments: &[(String, String)]) -> String {
    if !is_json {
        return segments
            .first()
            .map(|(_, text)| text.clone())
            .unwrap_or_else(|| original.to_string());
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(original) else {
        return original.to_string();
    };
    let Some(object) = value.as_object_mut() else {
        return original.to_string();
    };
    for (path, text) in segments {
        if path.is_empty() {
            continue;
        }
        if let Some(slot) = object.get_mut(path)
            && slot.is_string()
        {
            *slot = serde_json::Value::String(text.clone());
        }
    }
    serde_json::to_string(&value).unwrap_or_else(|_| original.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer the shell results are allowed through.
    ///
    /// `ShellMode::Native` is the only mode where the layer compresses the
    /// shell payload itself — AFT and rtk both own it in their own modes, and
    /// the default (AFT) leaves a terminal result alone. Every shell test
    /// below therefore names the mode explicitly, because the default is the
    /// NO-OP.
    fn cfg() -> EvidenceConfig {
        EvidenceConfig {
            enabled: true,
            shell: ShellMode::Native,
            ..EvidenceConfig::default()
        }
    }

    #[test]
    fn a_disabled_layer_changes_nothing() {
        let out = compress("terminal", "a\nb\nc\n", &EvidenceConfig::default());
        assert!(!out.changed());
        assert_eq!(out.text, "a\nb\nc\n");
    }

    #[test]
    fn a_small_result_is_left_alone() {
        let out = compress("terminal", "a\nb\nc\n", &cfg());
        assert!(!out.changed());
    }

    #[test]
    fn a_terminal_result_is_compressed() {
        let text: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let out = compress("terminal", &text, &cfg());
        assert!(out.changed(), "expected compression");
        assert!(out.tokens_after < out.tokens_before);
    }

    #[test]
    fn a_terminal_result_keeps_its_envelope() {
        let stdout: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let payload = serde_json::json!({"exit_code": 0, "stdout": stdout, "runtime": "local"});
        let text = serde_json::to_string(&payload).expect("serializes");
        let out = compress("terminal", &text, &cfg());
        let parsed: serde_json::Value = serde_json::from_str(&out.text).expect("still JSON");
        assert_eq!(parsed["exit_code"], 0);
        assert_eq!(parsed["runtime"], "local");
        assert!(parsed["stdout"].as_str().expect("string").len() < stdout.len());
    }

    #[test]
    fn compression_never_grows_the_payload() {
        let text = "short\n";
        let out = compress("terminal", text, &cfg());
        assert!(out.text.len() <= text.len());
    }
}
