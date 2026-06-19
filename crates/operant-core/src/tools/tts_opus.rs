use std::path::Path;

use tokio::process::Command;
use tracing::{debug, warn};

use super::tts_provider::TtsError;

pub fn has_ffmpeg() -> bool {
    which::which("ffmpeg").is_ok()
}

pub async fn convert_to_opus(input_path: &str) -> Result<String, TtsError> {
    if !has_ffmpeg() {
        return Err(TtsError::ConfigError(
            "ffmpeg not found; cannot convert to Opus".into(),
        ));
    }

    let input = Path::new(input_path);
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let parent = input.parent().unwrap_or(Path::new("."));
    let ogg_path = parent.join(format!("{}.ogg", stem));

    let output = Command::new("ffmpeg")
        .args([
            "-i",
            input_path,
            "-acodec",
            "libopus",
            "-ac",
            "1",
            "-b:a",
            "64k",
            "-vbr",
            "off",
            "-y",
            ogg_path.to_str().unwrap_or("output.ogg"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| TtsError::SynthesisFailed(format!("ffmpeg execution failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(
            "ffmpeg Opus conversion failed (code {}): {}",
            output.status,
            chars_limit(&stderr, 200)
        );
        return Err(TtsError::SynthesisFailed(format!(
            "ffmpeg Opus conversion failed: {}",
            chars_limit(&stderr, 300)
        )));
    }

    if !ogg_path.exists() || ogg_path.metadata().map(|m| m.len() == 0).unwrap_or(true) {
        return Err(TtsError::SynthesisFailed(
            "ffmpeg produced empty Opus output".into(),
        ));
    }

    debug!(input = input_path, output = %ogg_path.display(), "Converted to Opus");
    Ok(ogg_path.to_string_lossy().into_owned())
}

fn chars_limit(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_ffmpeg() {
        let _ = has_ffmpeg();
    }
}
