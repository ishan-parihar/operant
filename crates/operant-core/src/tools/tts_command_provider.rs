use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;

use async_trait::async_trait;
use regex::Regex;
use tokio::process::Command;
use tracing::debug;

use super::tts_provider::{AudioFormat, SynthesisResult, TtsError, TtsProvider};

const DEFAULT_TIMEOUT_SECS: u64 = 120;

pub struct CommandProvider {
    name: String,
    command_template: String,
    output_format: AudioFormat,
    timeout_secs: u64,
    voice: Option<String>,
    model: Option<String>,
    voice_compatible: bool,
}

impl CommandProvider {
    pub fn new(name: String, command_template: String) -> Self {
        Self {
            name,
            command_template,
            output_format: AudioFormat::Mp3,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            voice: None,
            model: None,
            voice_compatible: false,
        }
    }

    pub fn with_format(mut self, format: AudioFormat) -> Self {
        self.output_format = format;
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn with_voice(mut self, voice: String) -> Self {
        self.voice = Some(voice);
        self
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_voice_compatible(mut self, compatible: bool) -> Self {
        self.voice_compatible = compatible;
        self
    }

    pub fn output_format(&self) -> &AudioFormat {
        &self.output_format
    }

    fn render_template(&self, placeholders: &HashMap<String, String>) -> Result<String, TtsError> {
        // Match {name} or {{name}} but not escaped {{}}
        let re = Regex::new(r"\{\{(\w+)\}\}|\{(\w+)\}")
            .map_err(|e| TtsError::ConfigError(format!("Invalid template regex: {}", e)))?;

        let mut result = self.command_template.clone();
        for cap in re.captures_iter(&self.command_template) {
            let name = cap
                .get(1)
                .or_else(|| cap.get(2))
                .expect("placeholder regex guarantees group 1 or 2")
                .as_str();
            let value = placeholders.get(name).cloned().unwrap_or_default();
            // Escape single quotes for shell safety
            let escaped = value.replace('\'', "'\\''");
            result = result.replace(&cap[0], &format!("'{}'", escaped));
        }
        Ok(result)
    }
}

#[async_trait]
impl TtsProvider for CommandProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn voice_compatible(&self) -> bool {
        self.voice_compatible
    }

    async fn synthesize(
        &self,
        text: &str,
        output_path: &str,
        voice: Option<&str>,
        model: Option<&str>,
        format: AudioFormat,
    ) -> Result<SynthesisResult, TtsError> {
        let tmp_dir = tempfile::tempdir().map_err(TtsError::Io)?;
        let input_path = tmp_dir.path().join("input.txt");
        tokio::fs::write(&input_path, text)
            .await
            .map_err(TtsError::Io)?;

        let mut placeholders = HashMap::new();
        placeholders.insert(
            "input_path".into(),
            input_path.to_string_lossy().into_owned(),
        );
        placeholders.insert(
            "text_path".into(),
            input_path.to_string_lossy().into_owned(),
        );
        placeholders.insert("output_path".into(), output_path.to_string());
        placeholders.insert("format".into(), format.to_string());
        placeholders.insert(
            "voice".into(),
            voice.or(self.voice.as_deref()).unwrap_or("").to_string(),
        );
        placeholders.insert(
            "model".into(),
            model.or(self.model.as_deref()).unwrap_or("").to_string(),
        );

        let command = self.render_template(&placeholders)?;
        debug!(provider = %self.name, command = %command, "Running command TTS");

        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| TtsError::SynthesisFailed(format!("Failed to run command: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TtsError::SynthesisFailed(format!(
                "Command exited with {}: {}",
                output.status,
                stderr.chars().take(200).collect::<String>()
            )));
        }

        let out_path = Path::new(output_path);
        if !out_path.exists() || out_path.metadata().map(|m| m.len() == 0).unwrap_or(true) {
            return Err(TtsError::SynthesisFailed(format!(
                "Provider '{}' produced no output at {}",
                self.name, output_path
            )));
        }

        Ok(SynthesisResult {
            output_path: output_path.to_string(),
            format,
            voice_compatible: self.voice_compatible,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_substitution() {
        let provider = CommandProvider::new("test".into(), "echo {text} > {output_path}".into());
        let mut placeholders = HashMap::new();
        placeholders.insert("text".into(), "hello world".into());
        placeholders.insert("output_path".into(), "/tmp/out.mp3".into());

        let result = provider.render_template(&placeholders).unwrap();
        assert!(result.contains("'hello world'"));
        assert!(result.contains("'/tmp/out.mp3'"));
    }

    #[test]
    fn test_double_brace_escaping() {
        let provider = CommandProvider::new("test".into(), "echo {{literal}} {value}".into());
        let mut placeholders = HashMap::new();
        placeholders.insert("value".into(), "replaced".into());

        let result = provider.render_template(&placeholders).unwrap();
        assert!(result.contains("'replaced'"));
    }
}
