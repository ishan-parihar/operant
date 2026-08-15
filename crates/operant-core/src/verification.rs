//! Verification harness (hermes `agent/verify/` parity — recipes.py +
//! runner.py + environment.py).
//!
//! Detects a project's verification recipe from its manifests (Rust, Node,
//! Python, Go, Java, Makefile, docker-compose) and runs it phase by phase:
//! bootstrap → build → test, plus an optional start phase that boots the
//! app and polls an HTTP readiness URL. Results are recorded in the
//! `verification_events` evidence ledger (hermes `verification_evidence.py`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const DEFAULT_PHASE_TIMEOUT: Duration = Duration::from_secs(300);
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(120);
const START_READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(1000);
const OUTPUT_TAIL_CHARS: usize = 4000;

/// A detected (or user-supplied) verification recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRecipe {
    pub name: String,
    pub language: String,
    #[serde(default)]
    pub bootstrap: Vec<String>,
    #[serde(default)]
    pub build: Vec<String>,
    #[serde(default)]
    pub test: Vec<String>,
}

/// Start-phase specification: command to boot, port + path to probe.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StartSpec {
    pub command: Vec<String>,
    pub port: u16,
    #[serde(default = "default_readiness_path")]
    pub path: String,
}

fn default_readiness_path() -> String {
    "/".to_string()
}

/// Result of one verification phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseResult {
    pub name: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub output_tail: String,
    pub duration_ms: u64,
}

/// Overall verification result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub recipe: Option<VerificationRecipe>,
    pub phases: Vec<PhaseResult>,
    pub ok: bool,
}

/// An ad-hoc check specification for `verify_task action="ad_hoc"`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CheckSpec {
    pub cmd: String,
    /// Expected exit code; `None` expects 0.
    #[serde(default)]
    pub expected_exit: Option<i32>,
    /// Substring that must appear in output for the check to pass.
    #[serde(default)]
    pub expect_contains: Option<String>,
}

/// Detect the verification recipe for a project rooted at `root`.
pub fn detect_recipe(root: &Path) -> Option<VerificationRecipe> {
    if root.join("Cargo.toml").is_file() {
        return Some(VerificationRecipe {
            name: "cargo".to_string(),
            language: "rust".to_string(),
            bootstrap: vec![],
            build: vec!["cargo build".to_string()],
            test: vec!["cargo test".to_string()],
        });
    }
    if root.join("go.mod").is_file() {
        return Some(VerificationRecipe {
            name: "go".to_string(),
            language: "go".to_string(),
            bootstrap: vec![],
            build: vec!["go build ./...".to_string()],
            test: vec!["go test ./...".to_string()],
        });
    }
    if root.join("package.json").is_file() {
        return Some(detect_node_recipe(root));
    }
    if root.join("pyproject.toml").is_file()
        || root.join("requirements.txt").is_file()
        || root.join("setup.py").is_file()
    {
        return Some(detect_python_recipe(root));
    }
    if root.join("pom.xml").is_file() {
        return Some(VerificationRecipe {
            name: "maven".to_string(),
            language: "java".to_string(),
            bootstrap: vec![],
            build: vec!["mvn -q compile".to_string()],
            test: vec!["mvn -q test".to_string()],
        });
    }
    if root.join("build.gradle").is_file() || root.join("build.gradle.kts").is_file() {
        return Some(VerificationRecipe {
            name: "gradle".to_string(),
            language: "java".to_string(),
            bootstrap: vec![],
            build: vec!["gradle build -x test".to_string()],
            test: vec!["gradle test".to_string()],
        });
    }
    if root.join("Makefile").is_file() || root.join("makefile").is_file() {
        return Some(VerificationRecipe {
            name: "make".to_string(),
            language: "make".to_string(),
            bootstrap: vec![],
            build: vec!["make build".to_string()],
            test: vec!["make test".to_string()],
        });
    }
    if root.join("docker-compose.yml").is_file() || root.join("docker-compose.yaml").is_file() {
        return Some(VerificationRecipe {
            name: "docker-compose".to_string(),
            language: "docker".to_string(),
            bootstrap: vec![],
            build: vec![],
            test: vec![],
        });
    }
    None
}

fn detect_node_recipe(root: &Path) -> VerificationRecipe {
    // Package manager from lockfiles.
    let pm = if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else if root.join("bun.lockb").is_file() || root.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    };
    let (build, test) = read_node_scripts(root, pm);
    VerificationRecipe {
        name: "node".to_string(),
        language: "node".to_string(),
        bootstrap: vec![format!("{} install", pm)],
        build,
        test,
    }
}

fn read_node_scripts(root: &Path, pm: &str) -> (Vec<String>, Vec<String>) {
    let raw = match std::fs::read_to_string(root.join("package.json")) {
        Ok(r) => r,
        Err(_) => return (vec![], vec![]),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return (vec![], vec![]),
    };
    let script = |name: &str| -> Option<String> {
        parsed["scripts"][name]
            .as_str()
            .map(|_| format!("{} run {}", pm, name))
    };
    let mut build = Vec::new();
    if let Some(b) = script("build") {
        build.push(b);
    } else if pm != "npm" {
        build.push(format!("{} run build", pm));
    }
    let mut test = Vec::new();
    if let Some(t) = script("test") {
        test.push(t);
    }
    (build, test)
}

fn detect_python_recipe(root: &Path) -> VerificationRecipe {
    let (pm, bootstrap) = if root.join("uv.lock").is_file() {
        ("uv", vec!["uv sync".to_string()])
    } else if root.join("poetry.lock").is_file() {
        ("poetry", vec!["poetry install".to_string()])
    } else if root.join("Pipfile").is_file() {
        ("pipenv", vec!["pipenv install --dev".to_string()])
    } else {
        ("pip", vec!["pip install -e .".to_string()])
    };
    let _ = pm;
    VerificationRecipe {
        name: "python".to_string(),
        language: "python".to_string(),
        bootstrap,
        build: vec!["python -m compileall -q .".to_string()],
        test: vec!["python -m pytest -q".to_string()],
    }
}

/// Run a single shell command in `cwd`, capturing a tail of stdout+stderr.
pub async fn run_phase(name: &str, command: &str, cwd: &Path, timeout: Duration) -> PhaseResult {
    let started = std::time::Instant::now();
    let mut child = match tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return PhaseResult {
                name: name.to_string(),
                ok: false,
                exit_code: None,
                output_tail: format!("Failed to spawn: {e}"),
                duration_ms: started.elapsed().as_millis() as u64,
            };
        }
    };

    // Capture stdout+stderr concurrently with a bounded tail.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (out_task, err_task) = (
        tokio::spawn(capture_tail(stdout)),
        tokio::spawn(capture_tail(stderr)),
    );

    let status = tokio::time::timeout(timeout, child.wait()).await;
    let (exit_code, timed_out) = match status {
        Ok(Ok(st)) => (st.code(), false),
        Ok(Err(e)) => {
            return PhaseResult {
                name: name.to_string(),
                ok: false,
                exit_code: None,
                output_tail: format!("Wait error: {e}"),
                duration_ms: started.elapsed().as_millis() as u64,
            };
        }
        Err(_) => (None, true),
    };

    let out = out_task.await.unwrap_or_default();
    let err = err_task.await.unwrap_or_default();
    let mut output = out;
    if !err.trim().is_empty() {
        if !output.trim().is_empty() {
            output.push('\n');
        }
        output.push_str(&err);
    }
    let mut output_tail = tail(&output, OUTPUT_TAIL_CHARS);
    let ok = if timed_out {
        false
    } else {
        exit_code == Some(0)
    };
    if timed_out {
        output_tail = format!("[timed out after {:?}]\n{}", timeout, output_tail);
    }

    PhaseResult {
        name: name.to_string(),
        ok,
        exit_code,
        output_tail,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

/// Run an ad-hoc check against output expectations.
pub async fn run_check(spec: &CheckSpec, cwd: &Path) -> PhaseResult {
    let mut result = run_phase("check", &spec.cmd, cwd, DEFAULT_PHASE_TIMEOUT).await;
    let expected = spec.expected_exit.unwrap_or(0);
    let mut ok = result.exit_code == Some(expected);
    if let Some(needle) = spec.expect_contains.as_deref()
        && ok
        && !result.output_tail.contains(needle)
    {
        ok = false;
        result.output_tail = format!(
            "[missing expected output: {:?}]\n{}",
            needle,
            tail(&result.output_tail, OUTPUT_TAIL_CHARS)
        );
    }
    result.ok = ok;
    result
}

/// Run all phases of a recipe, in order. `include_start` boots the app and
/// probes its readiness URL when the recipe (or start spec) provides one.
pub async fn run_verify(
    recipe: &VerificationRecipe,
    root: &Path,
    include_start: bool,
    start: Option<&StartSpec>,
) -> VerifyResult {
    let mut phases = Vec::new();

    for cmd in &recipe.bootstrap {
        phases.push(run_phase("bootstrap", cmd, root, BOOTSTRAP_TIMEOUT).await);
    }
    for cmd in &recipe.build {
        phases.push(run_phase("build", cmd, root, DEFAULT_PHASE_TIMEOUT).await);
    }
    for cmd in &recipe.test {
        phases.push(run_phase("test", cmd, root, DEFAULT_PHASE_TIMEOUT).await);
    }

    if include_start && let Some(spec) = start {
        phases.push(run_start(spec, root).await);
    }

    let ok = phases.iter().all(|p| p.ok);
    VerifyResult {
        recipe: Some(recipe.clone()),
        phases,
        ok,
    }
}

/// Boot a command, poll the readiness URL until it answers HTTP 200, then
/// kill the process group.
async fn run_start(spec: &StartSpec, cwd: &Path) -> PhaseResult {
    let started = std::time::Instant::now();
    let command = spec.command.join(" ");
    let mut child = match tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return PhaseResult {
                name: "start".to_string(),
                ok: false,
                exit_code: None,
                output_tail: format!("Failed to spawn start command: {e}"),
                duration_ms: started.elapsed().as_millis() as u64,
            };
        }
    };

    let url = format!("http://localhost:{}{}", spec.port, spec.path);
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + START_READY_TIMEOUT;
    let mut last_err = String::from("not yet polled");
    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let _ = child.kill().await;
                let status = child.wait().await.ok();
                return PhaseResult {
                    name: "start".to_string(),
                    ok: true,
                    exit_code: status.and_then(|s| s.code()),
                    output_tail: format!(
                        "app booted and answered {url} (HTTP 200) in {:?}",
                        started.elapsed()
                    ),
                    duration_ms: started.elapsed().as_millis() as u64,
                };
            }
            Ok(resp) => {
                last_err = format!("HTTP {}", resp.status().as_u16());
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
        tokio::time::sleep(READY_POLL_INTERVAL).await;
    }

    let _ = child.kill().await;
    let _ = child.wait().await;
    PhaseResult {
        name: "start".to_string(),
        ok: false,
        exit_code: None,
        output_tail: format!(
            "app did not become ready at {url} within {:?} (last: {last_err})",
            START_READY_TIMEOUT
        ),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

async fn capture_tail<S>(stream: Option<S>) -> String
where
    S: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let Some(mut stream) = stream else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).into_owned()
}

fn tail(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let skip = text.chars().count() - max_chars;
    let mut s = String::with_capacity(max_chars);
    for ch in text.chars().skip(skip) {
        s.push(ch);
    }
    format!(
        "[...truncated {} chars]\n{}",
        text.chars().count() - max_chars,
        s
    )
}

/// Locate the project root: nearest ancestor containing a manifest.
pub fn find_project_root(start: &Path) -> PathBuf {
    let mut current = Some(start.to_path_buf());
    while let Some(dir) = current {
        if [
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "go.mod",
            "pom.xml",
            "Makefile",
        ]
        .iter()
        .any(|m| dir.join(m).is_file())
        {
            return dir;
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    start.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("operant_verify_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_rust_recipe() {
        let dir = temp_project("rust");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let r = detect_recipe(&dir).unwrap();
        assert_eq!(r.language, "rust");
        assert_eq!(r.test, vec!["cargo test"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_node_recipe_with_pnpm() {
        let dir = temp_project("node");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"build":"tsc","test":"vitest run"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        let r = detect_recipe(&dir).unwrap();
        assert_eq!(r.language, "node");
        assert_eq!(r.bootstrap, vec!["pnpm install"]);
        assert_eq!(r.build, vec!["pnpm run build"]);
        assert_eq!(r.test, vec!["pnpm run test"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_python_recipe_with_uv() {
        let dir = temp_project("py");
        std::fs::write(dir.join("pyproject.toml"), "[project]\n").unwrap();
        std::fs::write(dir.join("uv.lock"), "").unwrap();
        let r = detect_recipe(&dir).unwrap();
        assert_eq!(r.language, "python");
        assert_eq!(r.bootstrap, vec!["uv sync"]);
        assert_eq!(r.test, vec!["python -m pytest -q"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_makefile_and_go() {
        let dir = temp_project("make");
        std::fs::write(dir.join("Makefile"), "build:\n\ttrue\n").unwrap();
        let r = detect_recipe(&dir).unwrap();
        assert_eq!(r.language, "make");

        let gdir = temp_project("go");
        std::fs::write(gdir.join("go.mod"), "module x\n").unwrap();
        let r = detect_recipe(&gdir).unwrap();
        assert_eq!(r.language, "go");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&gdir);
    }

    #[test]
    fn finds_project_root_upward() {
        let dir = temp_project("root_find");
        let nested = dir.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        assert_eq!(find_project_root(&nested), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_phase_captures_output() {
        let dir = temp_project("phase");
        let result = run_phase("echo", "echo hello-verify", &dir, Duration::from_secs(30)).await;
        assert!(result.ok);
        assert!(result.output_tail.contains("hello-verify"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_phase_fails_on_nonzero_exit() {
        let dir = temp_project("phase_fail");
        let result = run_phase("fail", "exit 3", &dir, Duration::from_secs(30)).await;
        assert!(!result.ok);
        assert_eq!(result.exit_code, Some(3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_phase_respects_timeout() {
        let dir = temp_project("phase_timeout");
        let result = run_phase("sleep", "sleep 30", &dir, Duration::from_millis(200)).await;
        assert!(!result.ok);
        assert!(result.output_tail.contains("timed out"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_check_expect_contains() {
        let dir = temp_project("check");
        let spec = CheckSpec {
            cmd: "echo sparkle-unicorn".to_string(),
            expected_exit: None,
            expect_contains: Some("sparkle".to_string()),
        };
        let r = run_check(&spec, &dir).await;
        assert!(r.ok);

        let bad = CheckSpec {
            cmd: "echo sparkle-unicorn".to_string(),
            expected_exit: None,
            expect_contains: Some("dragon".to_string()),
        };
        let r = run_check(&bad, &dir).await;
        assert!(!r.ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_verify_full_recipe() {
        let dir = temp_project("full");
        let recipe = VerificationRecipe {
            name: "demo".to_string(),
            language: "shell".to_string(),
            bootstrap: vec![],
            build: vec!["true".to_string()],
            test: vec!["false".to_string()],
        };
        let result = run_verify(&recipe, &dir, false, None).await;
        assert!(!result.ok, "test phase false must fail overall");
        assert_eq!(result.phases.len(), 2);
        assert!(result.phases[0].ok);
        assert!(!result.phases[1].ok);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
