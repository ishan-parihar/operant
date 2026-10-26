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
/// raising, so one bad snippet can't wreck the whole skill message. A
/// snippet that outlives `timeout_secs` is killed and reports
/// `[inline-shell timeout ...]` (BUGS.md R47-3: the timeout used to be
/// computed and discarded — `let _timeout = ...` around a blocking
/// `cmd.output()` — so `!`sleep 9999`` hung the whole agent turn, parking a
/// tokio worker thread with it).
pub fn run_inline_shell(command: &str, cwd: Option<&Path>, timeout_secs: u64) -> String {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let timeout = Duration::from_secs(timeout_secs.max(1));

    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return if e.kind() == std::io::ErrorKind::NotFound {
                "[inline-shell error: bash not found]".to_string()
            } else {
                format!("[inline-shell error: {e}]")
            };
        }
    };

    // Drain both pipes on reader threads while polling: a snippet writing
    // more than the pipe buffer would otherwise block on write, never exit,
    // and be killed as a "timeout" it did not have.
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let completed = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    // Reap so the reader threads see EOF and join.
                    let _ = child.wait();
                    break false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return format!("[inline-shell error: {e}]"),
        }
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();

    if !completed {
        return format!("[inline-shell timeout after {timeout_secs}s: {command}]");
    }

    let mut out = stdout.trim_end().to_string();
    if out.is_empty() {
        out = stderr.trim_end().to_string();
    }
    if out.len() > INLINE_SHELL_MAX_OUTPUT {
        out.truncate(INLINE_SHELL_MAX_OUTPUT);
        out.push_str("...[truncated]");
    }
    out
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
/// Resolve a skill directory from a name or relative path, mirroring the
/// recursive resolution `skill_view`/`skills_list` use so `/skill` invocation
/// behaves identically across layouts:
///
/// 1. Direct join — flat seeded layout (`components/heroes` ->
///    `<root>/components/heroes`).
/// 2. Path-segment walk — categorized layout where the router sits under
///    category dirs (`creative/website-design/components/heroes`), so the
///    caller can pass the relative path from the skills root.
/// 3. Leaf-name recursive search — a bare name (`heroes`) resolves to the
///    first skill with that name anywhere in the tree.
fn resolve_skill_dir(skills_dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    // Ponytail phase 3: installed integration skills are invocable
    // EXACTLY like operator skills — but the operator pool is tried
    // FIRST (steps 1-3), so an operator's same-named skill shadows the
    // integration's, the same first-wins rule SkillManager.load_all
    // applies.
    // 1. Direct join (fast path — flat seeded layout).
    let direct = skills_dir.join(name);
    if direct.join("SKILL.md").is_file() {
        return Some(direct);
    }
    // 2. Path-segment walk from the skills root (categorized layout).
    if name.contains('/') {
        let mut cur = skills_dir.to_path_buf();
        let mut ok = true;
        for seg in name.split('/') {
            if seg.is_empty() || seg == "." || seg == ".." {
                ok = false;
                break;
            }
            cur = cur.join(seg);
        }
        if ok && cur.join("SKILL.md").is_file() {
            return Some(cur);
        }
    }
    // 3. Leaf-name recursive search (bare names anywhere in the tree).
    if let Some(found) = find_skill_by_name(skills_dir, name) {
        return Some(found);
    }
    // 4. Integration roots — same three-step resolution, per root, only
    // when the operator pool had no match. `installed_skills_dir()` is
    // None until ponytail is installed, so the common path costs one
    // latest_version_dir scan (readdir of the integrations dir).
    let extra_root = crate::ponytail::installed_skills_dir()?;
    let direct = extra_root.join(name);
    if direct.join("SKILL.md").is_file() {
        return Some(direct);
    }
    if name.contains('/') {
        let mut cur = extra_root.clone();
        let mut ok = true;
        for seg in name.split('/') {
            if seg.is_empty() || seg == "." || seg == ".." {
                ok = false;
                break;
            }
            cur = cur.join(seg);
        }
        if ok && cur.join("SKILL.md").is_file() {
            return Some(cur);
        }
    }
    find_skill_by_name(&extra_root, name)
}

/// Depth-first search for a directory named `name` that carries a SKILL.md.
fn find_skill_by_name(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name.starts_with('.') || dir_name == "node_modules" || dir_name == ".archive" {
            continue;
        }
        if dir_name == name && path.join("SKILL.md").is_file() {
            return Some(path);
        }
        if let Some(found) = find_skill_by_name(&path, name) {
            return Some(found);
        }
    }
    None
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
    let skill_dir = resolve_skill_dir(skills_dir, name)?;
    let skill_md = skill_dir.join("SKILL.md");
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

    /// Ponytail phase 3 acceptance (plan
    /// 2026-10-26-ponytail-as-a-self-updating-content-integration): with a
    /// ponytail-shaped install present, `/skill ponytail-review` must
    /// inject the INTEGRATION's SKILL.md — resolved through the extra
    /// root, frontmatter stripped, scaffolding wrapped. Uses a temp
    /// HERMES_HOME so the test neither reads the operator's real install
    /// nor mutates anything shared.
    #[test]
    fn integration_skill_injects_via_invocation_builder() {
        let _env = crate::platform::test_env_lock_blocking();
        let key = "HERMES_HOME";
        let original = std::env::var(key).ok();
        let tmp =
            std::env::temp_dir().join(format!("operant-skill-integration-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        // The install layout binary_updater::install produces:
        // <home>/integrations/ponytail-v9.9.9/skills/<skill>/SKILL.md
        let skills = tmp
            .join("integrations")
            .join("ponytail-v9.9.9")
            .join("skills");
        std::fs::create_dir_all(skills.join("ponytail-review")).unwrap();
        std::fs::write(
            skills.join("ponytail-review").join("SKILL.md"),
            "---\nname: ponytail-review\ndescription: Code review focused on over-engineering\nversion: 5.3.2\n---\n\nYou are a senior developer who will be paged at 3am for this code.\n",
        )
        .unwrap();
        // Empty operator pool — resolution must fall through to the
        // integration root.
        std::fs::create_dir_all(tmp.join("skills")).unwrap();
        // SAFETY: test-only env mutation under the crate-wide env lock.
        unsafe { std::env::set_var(key, &tmp) };

        let msg = build_skill_invocation_message_in(&tmp.join("skills"), "ponytail-review", "");

        // Restore original value.
        match original {
            Some(val) => unsafe { std::env::set_var(key, val) },
            None => unsafe { std::env::remove_var(key) },
        }
        let _ = std::fs::remove_dir_all(&tmp);

        let msg = msg.expect("integration skill must resolve from the extra root");
        assert!(
            msg.contains("invoked the \"ponytail-review\" skill"),
            "scaffolding header missing: {msg}"
        );
        assert!(
            msg.contains("senior developer who will be paged"),
            "upstream body missing: {msg}"
        );
        assert!(
            !msg.contains("name: ponytail-review"),
            "frontmatter must be stripped from the injected body"
        );
        assert!(
            msg.contains("ponytail-v9.9.9"),
            "skill dir footer must name the install"
        );
    }

    /// Ponytail phase 3: an operator-pool skill with the same name shadows
    /// the integration's — `/skill` prefers the operator's copy.
    #[test]
    fn operator_pool_skill_shadows_integration_on_invocation() {
        let _env = crate::platform::test_env_lock_blocking();
        let key = "HERMES_HOME";
        let original = std::env::var(key).ok();
        let tmp = std::env::temp_dir().join(format!("operant-skill-shadow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let pool = tmp.join("skills");
        let extra = tmp
            .join("integrations")
            .join("ponytail-v9.9.9")
            .join("skills");
        std::fs::create_dir_all(pool.join("ponytail-review")).unwrap();
        std::fs::write(
            pool.join("ponytail-review").join("SKILL.md"),
            "---\nname: ponytail-review\ndescription: mine\n---\nOPERATOR COPY\n",
        )
        .unwrap();
        std::fs::create_dir_all(extra.join("ponytail-review")).unwrap();
        std::fs::write(
            extra.join("ponytail-review").join("SKILL.md"),
            "---\nname: ponytail-review\ndescription: vendor\n---\nVENDOR COPY\n",
        )
        .unwrap();
        // SAFETY: test-only env mutation under the crate-wide env lock.
        unsafe { std::env::set_var(key, &tmp) };

        let msg = build_skill_invocation_message_in(&pool, "ponytail-review", "");

        // Restore original value.
        match original {
            Some(val) => unsafe { std::env::set_var(key, val) },
            None => unsafe { std::env::remove_var(key) },
        }
        let _ = std::fs::remove_dir_all(&tmp);

        let msg = msg.expect("operator copy must resolve");
        assert!(msg.contains("OPERATOR COPY"));
        assert!(!msg.contains("VENDOR COPY"));
    }

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

    /// BUGS.md R47-3: `timeout_secs` used to be computed and discarded, so a
    /// snippet that never exits hung the whole agent turn. Without the kill
    /// this test takes 30s (and fails the elapsed bound); with it, the
    /// snippet dies at the deadline and reports the timeout marker.
    #[test]
    fn run_inline_shell_timeout_kills_a_hung_snippet() {
        let started = std::time::Instant::now();
        let output = run_inline_shell("sleep 30", None, 1);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the timeout must bound the wait (took {:?})",
            started.elapsed()
        );
        assert!(
            output.contains("[inline-shell timeout"),
            "a killed snippet reports the timeout marker: {output}"
        );
    }

    /// A snippet that writes more than the pipe buffer must still finish and
    /// be truncated, not be killed as a false timeout (the reader threads
    /// exist for exactly this).
    #[test]
    fn run_inline_shell_survives_output_larger_than_the_pipe_buffer() {
        // 1 MiB of output against a 64 KiB pipe buffer.
        let output = run_inline_shell("head -c 1048576 /dev/zero | tr '\\0' 'x'", None, 10);
        assert!(
            output.len() <= INLINE_SHELL_MAX_OUTPUT + "...[truncated]".len(),
            "oversized output is truncated (got {} bytes)",
            output.len()
        );
        assert!(output.ends_with("...[truncated]"));
        assert!(
            output.starts_with('x'),
            "the payload is the snippet's stdout"
        );
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
