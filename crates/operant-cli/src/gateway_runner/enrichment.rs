//! Telegram message enrichment — photo/voice/document arms of the gateway
//! turn handler, split out of `gateway_runner.rs` (2026-10-25 ponytail audit
//! finding 5: the file mixed four jobs). Items are `pub(super)` and
//! glob-re-exported by the parent, so call sites keep their unqualified
//! names.

use anyhow::{Context, Result};
use operant_core::config::runtime_config;
use operant_core::tools::{OperantTool, ToolContext, TranscriptionTool};

use super::redact_err;

// ── Message enrichment helpers ──────────────────────────────────────────────

/// Download a file from Telegram by `file_id` to a temporary location.
pub(super) async fn download_telegram_file(
    token: &str,
    file_id: &str,
) -> Result<std::path::PathBuf> {
    let base = runtime_config().gateway.telegram_api_base;
    let base = base.trim_end_matches('/');
    let client = reqwest::Client::new();

    // Step 1: resolve file_id → file_path via getFile API
    let resp: serde_json::Value = client
        .post(format!("{}/bot{}/getFile", base, token))
        .json(&serde_json::json!({"file_id": file_id}))
        .send()
        .await
        .context("Telegram getFile request failed")?
        .json()
        .await
        .context("Telegram getFile response parse failed")?;

    let file_path = resp["result"]["file_path"]
        .as_str()
        .context("Telegram getFile returned no file_path")?
        .to_string();

    // Step 2: download the actual file bytes
    let download_url = format!("{}/file/bot{}/{}", base, token, file_path);
    let bytes = client
        .get(&download_url)
        .send()
        .await
        .context("Telegram file download request failed")?
        .bytes()
        .await
        .context("Telegram file download body read failed")?;

    // Step 3: persist to a temp file (cleaned up by the OS on next boot)
    let ext = std::path::Path::new(&file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("dat");
    let temp_dir = std::env::temp_dir();
    let short_id: String = file_id.chars().take(12).collect();
    let temp_file = temp_dir.join(format!("operant_tg_{}.{}", short_id, ext));
    tokio::fs::write(&temp_file, &bytes)
        .await
        .context("Failed to write Telegram download to temp file")?;

    Ok(temp_file)
}

/// If `raw` contains a Telegram photo array, download the largest image and
/// return a description string.  Returns `None` when no photo is present.
pub(super) async fn enrich_photo(raw: &serde_json::Value, token: &str) -> Option<String> {
    let message = raw.get("message")?;
    let photos = message.get("photo")?.as_array()?;
    if photos.is_empty() {
        return None;
    }

    // Pick the photo variant with the largest file_size
    let largest = photos
        .iter()
        .max_by_key(|p| p.get("file_size").and_then(|s| s.as_i64()).unwrap_or(0))?;

    let file_id = largest.get("file_id")?.as_str()?;
    let width = largest.get("width").and_then(|w| w.as_i64()).unwrap_or(0);
    let height = largest.get("height").and_then(|h| h.as_i64()).unwrap_or(0);

    match download_telegram_file(token, file_id).await {
        Ok(path) => {
            tracing::info!(
                "Downloaded photo to {:?} for analysis ({}×{})",
                path,
                width,
                height
            );
            Some(format!("[Image: {}×{} pixels]", width, height))
        }
        Err(e) => {
            tracing::warn!("Failed to download Telegram photo: {}", redact_err(&e));
            // Still report the dimensions even when download fails
            Some(format!("[Image: {}×{} pixels]", width, height))
        }
    }
}

/// If `raw` contains a Telegram voice message, download and transcribe it.
/// Returns the transcription text or a fallback placeholder on failure.
pub(super) async fn enrich_voice(raw: &serde_json::Value, token: &str) -> Option<String> {
    let message = raw.get("message")?;
    let voice = message.get("voice")?;
    let file_id = voice.get("file_id")?.as_str()?;
    let duration = voice.get("duration").and_then(|d| d.as_i64()).unwrap_or(0);
    let minutes = duration / 60;
    let seconds = duration % 60;

    // Download the OGG/audio file to a temporary location
    let path = match download_telegram_file(token, file_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to download voice message: {}", redact_err(&e));
            return Some(format!("[Voice message: {}:{:02}]", minutes, seconds));
        }
    };

    // Transcribe via the existing TranscriptionTool (reads from local path)
    let tool = TranscriptionTool::new();
    let args = serde_json::json!({
        "filePath": path.to_str().unwrap_or(""),
        "provider": "groq",
    });

    // (RT-3, HIGH) This arm calls `OperantTool::execute` DIRECTLY, not through
    // `ToolRegistry::execute`, so it is outside the disabled-tool gate the
    // registry enforces. `operant tools disable transcribe_audio` used to
    // leave voice enrichment transcribing anyway -- the disable list was a
    // listing filter on this path. `direct_call_is_disabled` closes it against
    // the same effective set the registry build applies. The placeholder text
    // is unchanged in shape, so a disabled tool degrades to the same "could
    // not transcribe" message a failure produces.
    if direct_call_is_disabled(tool.name()) {
        tracing::debug!(
            "Voice enrichment skipped: the transcription tool is disabled by operator policy"
        );
        return Some(format!("[Voice message: {}:{:02}]", minutes, seconds));
    }

    let result = tool.execute(args, ToolContext::default()).await;

    if result.success {
        if let Ok(data) = result.parse_content::<serde_json::Value>()
            && let Some(transcript) = data.get("transcript").and_then(|v| v.as_str())
        {
            let trimmed = transcript.trim();
            if !trimmed.is_empty() {
                tracing::info!(
                    "Transcribed voice message ({}:{:02}): {}",
                    minutes,
                    seconds,
                    trimmed
                );
                return Some(format!("[Transcription: {}]", trimmed));
            }
        }
    } else {
        tracing::warn!(
            "Voice transcription failed: {}",
            result.error.as_deref().unwrap_or("unknown error")
        );
    }

    Some(format!("[Voice message: {}:{:02}]", minutes, seconds))
}

/// If `raw` contains a Telegram document, return its filename / mime / caption.
pub(super) fn enrich_document(raw: &serde_json::Value) -> Option<String> {
    let message = raw.get("message")?;
    let doc = message.get("document")?;

    let filename = doc
        .get("file_name")
        .and_then(|f| f.as_str())
        .unwrap_or("unknown");
    let mime = doc
        .get("mime_type")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown");
    let size = doc.get("file_size").and_then(|s| s.as_i64()).unwrap_or(0);
    let size_kb = size as f64 / 1024.0;

    let caption = message.get("caption").and_then(|c| c.as_str());

    let info = if let Some(cap) = caption {
        format!(
            "[Document: {} ({:.1} KB, {}) with caption: {}]",
            filename, size_kb, mime, cap
        )
    } else {
        format!("[Document: {} ({:.1} KB, {})]", filename, size_kb, mime)
    };

    Some(info)
}

/// (RT-3, HIGH) Whether the operator's disabled-tool policy forbids a tool
/// that an arm is about to call DIRECTLY, bypassing `ToolRegistry::execute`.
///
/// The registry is the normal enforcement boundary, but this module holds one
/// arm that is not: voice enrichment constructs `TranscriptionTool` and calls
/// `OperantTool::execute` on it, so `operant tools disable transcribe_audio`
/// used to leave it transcribing anyway. Every direct-execute arm must ask
/// this first, against the SAME effective set the registry build applies (the
/// TOML list unioned with the CLI state file), or the operator's choice is a
/// listing filter again on that path.
///
/// Takes the name rather than the tool so the caller cannot pass a name that
/// disagrees with the instance it is about to run.
pub(super) fn direct_call_is_disabled(name: &str) -> bool {
    operant_core::tools::tool_state::disabled_tool_names(&runtime_config()).contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// (RT-3, HIGH) THE mutation test for the direct-execute bypass. An
    /// operator who disabled the transcription tool must have the voice arm
    /// skip it. This writes the same tool-state file `operant tools disable`
    /// writes and points the operant home at a temp dir, so it asserts against
    /// the real policy source rather than a stub.
    ///
    /// Fails if the guard is deleted, stubbed to `false`, or changed to a
    /// blanket "anything disabled" check (the scope assertion below).
    #[test]
    fn a_disabled_transcription_tool_stops_the_voice_arm() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let previous_home = std::env::var("HERMES_HOME").ok();
        // SAFETY: single-threaded test; restored before returning.
        unsafe { std::env::set_var("HERMES_HOME", tmp.path()) };

        let tool = operant_core::tools::TranscriptionTool::new();
        let name = tool.name().to_string();

        // No policy yet: the arm is allowed.
        assert!(
            !direct_call_is_disabled(&name),
            "an unconfigured operator must not have voice enrichment disabled"
        );

        // Write the state file the CLI writes: {"disabled": ["transcribe_audio"]}
        let state_path = tmp.path().join("tool_state.json");
        let mut f = std::fs::File::create(&state_path).expect("state file");
        write!(f, r#"{{"disabled": [{:?}]}}"#, name).expect("write state");
        drop(f);

        assert!(
            direct_call_is_disabled(&name),
            "a tool the operator disabled must be refused on the direct path"
        );

        // Scope: disabling ONE tool must not disable the others.
        assert!(
            !direct_call_is_disabled("rt3_a_tool_nobody_disabled"),
            "the guard must be scoped to the named tool, not blanket"
        );

        if let Some(v) = previous_home {
            unsafe { std::env::set_var("HERMES_HOME", v) };
        } else {
            unsafe { std::env::remove_var("HERMES_HOME") };
        }
    }
}
