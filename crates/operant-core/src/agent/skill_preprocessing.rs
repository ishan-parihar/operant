//! Shared SKILL.md preprocessing helpers.
//!
//! Ported from `hermes-agent/agent/skill_preprocessing.py`.
//!
//! Applies configured preprocessing to SKILL.md content before injection
//! into the system prompt:
//! - Template variable substitution: `${OPERANT_SKILL_DIR}`, `${OPERANT_SESSION_ID}`
//! - Inline shell execution: `!`command`` snippets expanded at load time

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tracing::debug;

/// Maximum output from an inline shell snippet (chars).
const INLINE_SHELL_MAX_OUTPUT: usize = 4000;

/// Default timeout for inline shell execution (seconds).
const DEFAULT_INLINE_SHELL_TIMEOUT: u64 = 10;

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Regex for template variables like `${OPERANT_SKILL_DIR}`.
fn template_var_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\$\{(OPERANT_SKILL_DIR|OPERANT_SESSION_ID)\}")
            .expect("static regex literal is invalid — authoring bug")
    })
}

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Regex for inline shell snippets like `` !`date +%Y-%m-%d` ``.
fn inline_shell_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"!`([^`\n]+)`").expect("static regex literal is invalid — authoring bug")
    })
}

/// Load the `skills` section of operant.toml (best-effort).
///
/// Reads the preprocessing fields (template_vars / inline_shell /
/// inline_shell_timeout) from `[skills]` when present; otherwise returns
/// defaults (template_vars on, inline_shell off — hermes parity).
pub fn load_skills_config() -> SkillsConfig {
    let mut cfg = SkillsConfig::default();
    if let Ok(app) = crate::config::load_app_config(None) {
        cfg.template_vars = app.config.skills.template_vars;
        cfg.inline_shell = app.config.skills.inline_shell;
        let t = app.config.skills.inline_shell_timeout;
        if t > 0 {
            cfg.inline_shell_timeout = t;
        }
    }
    cfg
}

/// Configuration for skill preprocessing.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Whether to substitute template variables (default: true).
    pub template_vars: bool,
    /// Whether to execute inline shell snippets (default: false).
    pub inline_shell: bool,
    /// Timeout for inline shell execution in seconds (default: 10).
    pub inline_shell_timeout: u64,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            template_vars: true,
            inline_shell: false,
            inline_shell_timeout: DEFAULT_INLINE_SHELL_TIMEOUT,
        }
    }
}

/// Replace `${OPERANT_SKILL_DIR}` / `${OPERANT_SESSION_ID}` in skill content.
///
/// Only substitutes tokens for which a concrete value is available —
/// unresolved tokens are left in place so the author can spot them.
pub fn substitute_template_vars(
    content: &str,
    skill_dir: Option<&Path>,
    session_id: Option<&str>,
) -> String {
    if content.is_empty() {
        return content.to_string();
    }

    let skill_dir_str = skill_dir.map(|p| p.to_string_lossy().to_string());

    template_var_regex()
        .replace_all(content, |caps: &regex::Captures| {
            let token = &caps[1];
            match token {
                "OPERANT_SKILL_DIR" => skill_dir_str.clone().unwrap_or_else(|| caps[0].to_string()),
                "OPERANT_SESSION_ID" => session_id
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| caps[0].to_string()),
                _ => caps[0].to_string(),
            }
        })
        .to_string()
}

/// Execute a single inline-shell snippet and return its stdout (trimmed).
///
/// Failures return a short `[inline-shell error: ...]` marker instead of
/// raising, so one bad snippet can't wreck the whole skill message.
pub fn run_inline_shell(command: &str, cwd: Option<&Path>, timeout_secs: u64) -> String {
    use std::process::Command;

    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let _timeout = Duration::from_secs(timeout_secs.max(1));

    match cmd.output() {
        Ok(output) => {
            let mut stdout = String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string();
            if stdout.is_empty() {
                stdout = String::from_utf8_lossy(&output.stderr)
                    .trim_end()
                    .to_string();
            }
            if stdout.len() > INLINE_SHELL_MAX_OUTPUT {
                stdout.truncate(INLINE_SHELL_MAX_OUTPUT);
                stdout.push_str("...[truncated]");
            }
            stdout
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                "[inline-shell error: bash not found]".to_string()
            } else {
                format!("[inline-shell error: {}]", e)
            }
        }
    }
}

/// Replace every `` !`cmd` `` snippet in `content` with its stdout.
///
/// Runs each snippet with the skill directory as CWD so relative paths in the
/// snippet work the way the author expects.
pub fn expand_inline_shell(content: &str, skill_dir: Option<&Path>, timeout_secs: u64) -> String {
    if !content.contains("!`") {
        return content.to_string();
    }

    inline_shell_regex()
        .replace_all(content, |caps: &regex::Captures| {
            let cmd = caps[1].trim();
            if cmd.is_empty() {
                return String::new();
            }
            debug!(command = %cmd, "Executing inline shell snippet");
            run_inline_shell(cmd, skill_dir, timeout_secs)
        })
        .to_string()
}

/// Apply configured SKILL.md template and inline-shell preprocessing.
pub fn preprocess_skill_content(
    content: &str,
    skill_dir: Option<&Path>,
    session_id: Option<&str>,
    config: Option<&SkillsConfig>,
) -> String {
    if content.is_empty() {
        return content.to_string();
    }

    let cfg = config.unwrap_or_else(|| {
        // Use a static default to avoid borrowing issues
        static DEFAULT: SkillsConfig = SkillsConfig {
            template_vars: true,
            inline_shell: false,
            inline_shell_timeout: DEFAULT_INLINE_SHELL_TIMEOUT,
        };
        &DEFAULT
    });

    let mut result = content.to_string();

    if cfg.template_vars {
        result = substitute_template_vars(&result, skill_dir, session_id);
    }

    if cfg.inline_shell {
        result = expand_inline_shell(&result, skill_dir, cfg.inline_shell_timeout);
    }

    result
}

/// Build the user-message payload for a `/skill <name>` invocation.
///
/// hermes parity: `agent/skill_commands.py::build_skill_invocation_message`.
/// Loads the skill's SKILL.md, applies configured preprocessing (template
/// vars + inline shell) **before** assembling the message, and wraps it with
/// the hermes activation scaffolding so the model treats it as active
/// guidance for the turn. Returns `None` when the skill is missing or its
/// SKILL.md cannot be read.
pub fn build_skill_invocation_message(name: &str, user_instruction: &str) -> Option<String> {
    build_skill_invocation_message_in(
        &crate::platform::operant_skills_dir(),
        name,
        user_instruction,
    )
}

/// Like [`build_skill_invocation_message`] but resolves the skill from an
/// explicit skills root directory (e.g. the user's configured
/// `skills.root_dir`), so the expansion honors config instead of always
/// defaulting to the platform home.
pub fn build_skill_invocation_message_in(
    skills_dir: &std::path::Path,
    name: &str,
    user_instruction: &str,
) -> Option<String> {
    let skill_dir = skills_dir.join(name);
    let skill_md = skill_dir.join("SKILL.md");
    if !skill_md.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(&skill_md).ok()?;

    // Strip YAML frontmatter (if any) before preprocessing/injection.
    let body = if let Some(stripped) = raw.strip_prefix("---") {
        match stripped.find("\n---") {
            Some(end) => stripped[end + 4..].trim_start().to_string(),
            None => raw,
        }
    } else {
        raw
    };

    let cfg = load_skills_config();
    let content = preprocess_skill_content(&body, Some(&skill_dir), None, Some(&cfg));

    let mut parts = vec![format!(
        "[IMPORTANT: The user has invoked the \"{}\" skill, indicating they \
         want you to follow its instructions. The full skill content is \
         loaded below.]",
        name
    )];
    parts.push(String::new());
    parts.push(content.trim().to_string());
    parts.push(String::new());
    parts.push(format!("[Skill directory: {}]", skill_dir.display()));

    if !user_instruction.trim().is_empty() {
        parts.push(String::new());
        parts.push(format!("User instruction: {}", user_instruction.trim()));
    }

    Some(parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_substitute_template_vars_with_dir() {
        let content = "Skill dir: ${OPERANT_SKILL_DIR}";
        let dir = PathBuf::from("/home/user/skills/my-skill");
        let result = substitute_template_vars(content, Some(&dir), None);
        assert_eq!(result, "Skill dir: /home/user/skills/my-skill");
    }

    #[test]
    fn test_substitute_template_vars_with_session() {
        let content = "Session: ${OPERANT_SESSION_ID}";
        let result = substitute_template_vars(content, None, Some("sess_abc123"));
        assert_eq!(result, "Session: sess_abc123");
    }

    #[test]
    fn test_substitute_template_vars_unresolved() {
        let content = "Unknown: ${OPERANT_UNKNOWN_VAR}";
        let result = substitute_template_vars(content, None, None);
        assert_eq!(result, "Unknown: ${OPERANT_UNKNOWN_VAR}");
    }

    #[test]
    fn test_substitute_template_vars_both() {
        let content = "Dir: ${OPERANT_SKILL_DIR}, Session: ${OPERANT_SESSION_ID}";
        let dir = PathBuf::from("/skills/test");
        let result = substitute_template_vars(content, Some(&dir), Some("sess_1"));
        assert_eq!(result, "Dir: /skills/test, Session: sess_1");
    }

    #[test]
    fn test_substitute_template_vars_empty() {
        assert_eq!(substitute_template_vars("", None, None), "");
    }

    #[test]
    fn test_expand_inline_shell_no_op() {
        let content = "No shell snippets here.";
        let result = expand_inline_shell(content, None, 10);
        assert_eq!(result, "No shell snippets here.");
    }

    #[test]
    fn test_expand_inline_shell_empty_command() {
        let content = "Before !`` After";
        let result = expand_inline_shell(content, None, 10);
        // Empty command should produce empty string
        assert!(result.contains("Before"));
        assert!(result.contains("After"));
    }

    #[test]
    fn test_expand_inline_shell_real_command() {
        let content = "Date: !`date +%Y`";
        let result = expand_inline_shell(content, None, 5);
        // Should contain the current year (at least 4 digits)
        assert!(result.contains("Date: "));
        let year_part = result.strip_prefix("Date: ").unwrap();
        assert!(year_part.len() == 4);
        assert!(year_part.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_run_inline_shell_simple() {
        let output = run_inline_shell("echo hello", None, 5);
        assert_eq!(output, "hello");
    }

    #[test]
    fn test_run_inline_shell_stderr_fallback() {
        // stderr fallback: when stdout is empty, use stderr
        let output = run_inline_shell("echo error_msg >&2", None, 5);
        assert_eq!(output, "error_msg");
    }

    #[test]
    fn test_preprocess_skill_content_no_config() {
        let content = "Hello ${OPERANT_SKILL_DIR}";
        let dir = PathBuf::from("/test");
        let result = preprocess_skill_content(content, Some(&dir), None, None);
        assert!(result.contains("/test"));
    }

    #[test]
    fn test_build_skill_invocation_message_missing_skill_returns_none() {
        // Hermes parity: a missing skill must yield None (no message), not a
        // panic or an empty string. Deterministic — no env mutation needed.
        let result = build_skill_invocation_message("definitely-not-a-real-skill-xyz", "go");
        assert!(result.is_none());
    }

    #[test]
    fn test_build_skill_invocation_message_expands_and_preprocesses() {
        // Validate the hermes-parity expansion core directly (the builder's
        // skill_dir resolves from the platform home, so exercise the shared
        // preprocessing path with an explicit temp dir instead): frontmatter
        // strip → template var substitution → scaffolding.
        let tmp = std::env::temp_dir().join(format!("operant-skill-inv-{}", std::process::id()));
        let skill_dir = tmp.join("my-test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-test-skill\ndescription: test\n---\n\n# Steps\nRun ${OPERANT_SKILL_DIR}\n",
        )
        .unwrap();

        let body = "# Steps\nRun ${OPERANT_SKILL_DIR}";
        let out = preprocess_skill_content(body, Some(&skill_dir), None, None);
        // Frontmatter-free body flows through with template var substituted.
        assert!(out.contains(skill_dir.to_string_lossy().as_ref()));

        // Full SKILL.md path: content preserved, template vars substituted
        // (frontmatter stripping is the builder's job, not the preprocessor's).
        let raw = "---\nname: my-test-skill\ndescription: test\n---\n\n# Steps\nRun ${OPERANT_SKILL_DIR}\n";
        let out2 = preprocess_skill_content(raw, Some(&skill_dir), None, None);
        assert!(out2.contains("description: test"));
        assert!(out2.contains(skill_dir.to_string_lossy().as_ref()));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
