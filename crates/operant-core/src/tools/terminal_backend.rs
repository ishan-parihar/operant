//! Terminal execution backends.
//!
//! Provides a trait abstraction over different command execution environments:
//! - `LocalBackend`: Execute directly on the host machine (default)
//! - `DockerBackend`: Execute in Docker containers
//! - `SshBackend`: Execute on remote machines over SSH

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::debug;

use crate::config::{AppConfig, TerminalBackend as BackendKind};
use crate::platform;

/// Output from a command execution.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Trait for terminal execution backends.
#[async_trait]
pub trait TerminalBackend: Send + Sync {
    /// Execute a command and return output.
    async fn execute_command(
        &self,
        command: &str,
        cwd: Option<&Path>,
        env_vars: &HashMap<String, String>,
        use_shell: bool,
        timeout: Duration,
        max_output: usize,
    ) -> anyhow::Result<CommandOutput>;

    /// Backend name for logging/errors.
    fn name(&self) -> &str;

    /// Check if this backend is available on this machine.
    fn is_available(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Local backend
// ---------------------------------------------------------------------------

/// Execute commands directly on the host machine.
pub struct LocalBackend;

#[async_trait]
impl TerminalBackend for LocalBackend {
    async fn execute_command(
        &self,
        command: &str,
        cwd: Option<&Path>,
        env_vars: &HashMap<String, String>,
        use_shell: bool,
        timeout: Duration,
        max_output: usize,
    ) -> anyhow::Result<CommandOutput> {
        let mut cmd = if use_shell {
            let shell = platform::detect_shell();
            let mut c = Command::new(&shell.path);
            for arg in &shell.args_pattern {
                c.arg(arg);
            }
            c.arg(command);
            c
        } else {
            let parts = shell_words::split(command)
                .map_err(|e| anyhow::anyhow!("Failed to parse command string: {}. Consider using useShell=true if you have special shell characters.", e))?;
            if parts.is_empty() {
                return Err(anyhow::anyhow!("Empty command string"));
            }
            let mut c = Command::new(&parts[0]);
            c.args(&parts[1..]);
            c
        };

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        } else if let Ok(c) = std::env::current_dir() {
            cmd.current_dir(c);
        }

        if !env_vars.is_empty() {
            let mut env: HashMap<String, String> = std::env::vars().collect();
            for (k, v) in env_vars {
                env.insert(k.clone(), v.clone());
            }
            cmd.envs(&env);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn process: {}", e))?;

        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        let mut stdout_output = String::new();
        let mut stderr_output = String::new();

        if let Some(stdout) = stdout_handle {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Ok(Some(line))) = tokio::time::timeout(timeout, reader.next_line()).await {
                if stdout_output.len() + line.len() < max_output {
                    stdout_output.push_str(&line);
                    stdout_output.push('\n');
                } else if stdout_output.len() < max_output {
                    let remaining = max_output - stdout_output.len();
                    stdout_output.push_str(&line[..remaining.min(line.len())]);
                    stdout_output.push_str("\n[output truncated]");
                } else {
                    stdout_output.push_str("\n[output truncated]");
                    break;
                }
            }
        }

        if let Some(stderr) = stderr_handle {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Ok(Some(line))) = tokio::time::timeout(timeout, reader.next_line()).await {
                if stderr_output.len() + line.len() < max_output / 4 {
                    stderr_output.push_str(&line);
                    stderr_output.push('\n');
                }
            }
        }

        let status = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(anyhow::anyhow!("Failed to wait for process: {}", e)),
            Err(_) => {
                let _ = child.kill().await;
                return Err(anyhow::anyhow!("Command timed out after {:?}", timeout));
            }
        };

        Ok(CommandOutput {
            success: status.success(),
            exit_code: status.code(),
            stdout: stdout_output,
            stderr: stderr_output,
        })
    }

    fn name(&self) -> &str {
        "local"
    }

    fn is_available(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Docker backend
// ---------------------------------------------------------------------------

/// Configuration for the Docker backend.
#[derive(Debug, Clone, Default)]
pub struct DockerConfig {
    pub image: String,
    pub container_name: Option<String>,
    pub volumes: Vec<String>,
    pub env: HashMap<String, String>,
    pub cpu: f64,
    pub memory_mb: u64,
    pub cwd: String,
}

/// Execute commands inside a Docker container.
pub struct DockerBackend {
    pub config: DockerConfig,
    container_id: Option<String>,
    docker_exe: String,
}

impl DockerBackend {
    /// Create a new Docker backend. Validates docker availability.
    pub fn new(config: DockerConfig) -> Self {
        let docker_exe = Self::find_docker();
        Self {
            config,
            container_id: None,
            docker_exe,
        }
    }

    /// Locate the docker CLI binary.
    fn find_docker() -> String {
        // Check HERMES_DOCKER_BINARY env var first
        if let Ok(override_path) = std::env::var("HERMES_DOCKER_BINARY")
            && !override_path.is_empty()
        {
            return override_path;
        }

        // Check common locations
        for candidate in &["docker", "podman"] {
            if which::which(candidate).is_ok() {
                return candidate.to_string();
            }
        }

        #[cfg(target_os = "macos")]
        {
            let mac_paths = [
                "/usr/local/bin/docker",
                "/opt/homebrew/bin/docker",
                "/Applications/Docker.app/Contents/Resources/bin/docker",
            ];
            for path in &mac_paths {
                if std::path::Path::new(path).exists() {
                    return path.to_string();
                }
            }
        }

        "docker".to_string()
    }

    /// Check if Docker daemon is reachable.
    fn daemon_reachable(&self) -> bool {
        std::process::Command::new(&self.docker_exe)
            .args(["version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl TerminalBackend for DockerBackend {
    async fn execute_command(
        &self,
        command: &str,
        cwd: Option<&Path>,
        env_vars: &HashMap<String, String>,
        use_shell: bool,
        _timeout: Duration,
        max_output: usize,
    ) -> anyhow::Result<CommandOutput> {
        let container_id = self
            .container_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Docker container not started"))?;

        let mut exec_args = vec!["exec".to_string()];

        // Inject env vars for this command
        for (k, v) in env_vars {
            exec_args.extend(["-e".to_string(), format!("{}={}", k, v)]);
        }

        if let Some(dir) = cwd {
            exec_args.extend(["-w".to_string(), dir.to_string_lossy().to_string()]);
        }

        exec_args.push(container_id.to_string());

        if use_shell {
            let shell = platform::detect_shell();
            exec_args.push(shell.path.to_string_lossy().to_string());
            for arg in &shell.args_pattern {
                exec_args.push(arg.clone());
            }
            exec_args.push(command.to_string());
        } else {
            let parts = shell_words::split(command)
                .map_err(|e| anyhow::anyhow!("Failed to parse command: {}", e))?;
            exec_args.extend(parts);
        }

        debug!("Docker exec: {} {}", self.docker_exe, exec_args.join(" "));

        let output = Command::new(&self.docker_exe)
            .args(&exec_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to execute in Docker: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Truncate output
        let stdout = if stdout.len() > max_output {
            format!("{}...[truncated]", &stdout[..max_output])
        } else {
            stdout
        };
        let stderr = if stderr.len() > max_output / 4 {
            format!("{}...[truncated]", &stderr[..max_output / 4])
        } else {
            stderr
        };

        Ok(CommandOutput {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout,
            stderr,
        })
    }

    fn name(&self) -> &str {
        "docker"
    }

    fn is_available(&self) -> bool {
        which::which(&self.docker_exe).is_ok() || self.daemon_reachable()
    }
}

// ---------------------------------------------------------------------------
// SSH backend
// ---------------------------------------------------------------------------

/// Configuration for the SSH backend.
#[derive(Debug, Clone, Default)]
pub struct SshConfig {
    pub host: String,
    pub user: String,
    pub port: u16,
    pub key_path: Option<String>,
}

/// Execute commands on a remote machine over SSH.
pub struct SshBackend {
    pub config: SshConfig,
    control_socket: Option<String>,
}

impl SshBackend {
    pub fn new(config: SshConfig) -> Self {
        let control_dir = std::env::temp_dir().join("operant-ssh");
        let _ = std::fs::create_dir_all(&control_dir);

        // Generate a short deterministic socket path
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        format!("{}@{}:{}", config.user, config.host, config.port).hash(&mut hasher);
        let socket_id = format!("{:016x}", hasher.finish());

        let control_socket = control_dir
            .join(format!("{}.sock", socket_id))
            .to_string_lossy()
            .to_string();

        Self {
            config,
            control_socket: Some(control_socket),
        }
    }

    /// Build the base SSH command with ControlMaster options.
    fn build_ssh_command(&self, extra_args: &[String]) -> Vec<String> {
        let mut cmd = vec!["ssh".to_string()];

        if let Some(ref socket) = self.control_socket {
            cmd.extend(["-o".to_string(), format!("ControlPath={}", socket)]);
            cmd.extend(["-o".to_string(), "ControlMaster=auto".to_string()]);
            cmd.extend(["-o".to_string(), "ControlPersist=300".to_string()]);
        }

        cmd.extend(["-o".to_string(), "BatchMode=yes".to_string()]);
        cmd.extend([
            "-o".to_string(),
            "StrictHostKeyChecking=accept-new".to_string(),
        ]);
        cmd.extend(["-o".to_string(), "ConnectTimeout=10".to_string()]);

        if self.config.port != 22 {
            cmd.extend(["-p".to_string(), self.config.port.to_string()]);
        }

        if let Some(ref key) = self.config.key_path {
            cmd.extend(["-i".to_string(), key.clone()]);
        }

        cmd.extend(extra_args.iter().cloned());
        cmd.push(format!("{}@{}", self.config.user, self.config.host));
        cmd
    }

    /// Cleanup the SSH ControlMaster connection.
    fn cleanup_connection(&self) {
        if let Some(ref socket) = self.control_socket {
            let _ = std::process::Command::new("ssh")
                .args([
                    "-o",
                    &format!("ControlPath={}", socket),
                    "-O",
                    "exit",
                    &format!("{}@{}", self.config.user, self.config.host),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = std::fs::remove_file(socket);
        }
    }
}

impl Drop for SshBackend {
    fn drop(&mut self) {
        self.cleanup_connection();
    }
}

#[async_trait]
impl TerminalBackend for SshBackend {
    async fn execute_command(
        &self,
        command: &str,
        cwd: Option<&Path>,
        env_vars: &HashMap<String, String>,
        use_shell: bool,
        timeout: Duration,
        max_output: usize,
    ) -> anyhow::Result<CommandOutput> {
        // Build the remote command with every model-controlled component
        // shell-quoted (hermes `shlex.quote` parity) so `$()`, backticks, or
        // `;` inside a working_dir or env value cannot inject into the remote
        // shell.
        let remote_cmd = Self::build_remote_command(command, cwd, env_vars, use_shell)?;
        let remote_cmd_arg = format!("bash -c {}", shell_words::quote(&remote_cmd));

        let extra_args = vec![remote_cmd_arg];
        let ssh_cmd = self.build_ssh_command(&extra_args);

        debug!("SSH exec: {}", ssh_cmd.join(" "));

        let output = tokio::time::timeout(
            timeout,
            Command::new(&ssh_cmd[0])
                .args(&ssh_cmd[1..])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("SSH command timed out after {:?}", timeout))?
        .map_err(|e| anyhow::anyhow!("Failed to execute SSH command: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Truncate output
        let stdout = if stdout.len() > max_output {
            format!("{}...[truncated]", &stdout[..max_output])
        } else {
            stdout
        };
        let stderr = if stderr.len() > max_output / 4 {
            format!("{}...[truncated]", &stderr[..max_output / 4])
        } else {
            stderr
        };

        Ok(CommandOutput {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout,
            stderr,
        })
    }

    fn name(&self) -> &str {
        "ssh"
    }

    fn is_available(&self) -> bool {
        which::which("ssh").is_ok() && which::which("scp").is_ok()
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Create the appropriate backend based on app config.
pub fn create_backend(config: &AppConfig) -> anyhow::Result<Box<dyn TerminalBackend>> {
    match &config.terminal_backend {
        BackendKind::Local => Ok(Box::new(LocalBackend)),
        BackendKind::Docker => {
            let docker_config = DockerConfig {
                image: config
                    .tools
                    .terminal
                    .docker
                    .image
                    .clone()
                    .unwrap_or_else(|| "nikolaik/python-nodejs:python3.11-nodejs20".to_string()),
                volumes: config.tools.terminal.docker.volumes.clone(),
                env: config.tools.terminal.docker.env.clone(),
                cpu: config.tools.terminal.docker.cpu,
                memory_mb: config.tools.terminal.docker.memory_mb,
                cwd: "/root".to_string(),
                ..Default::default()
            };
            Ok(Box::new(DockerBackend::new(docker_config)))
        }
        BackendKind::Ssh => {
            let ssh_config = SshConfig {
                host: config.tools.terminal.ssh.host.clone().unwrap_or_default(),
                user: config.tools.terminal.ssh.user.clone().unwrap_or_default(),
                port: config.tools.terminal.ssh.port,
                key_path: config.tools.terminal.ssh.key_path.clone(),
            };
            Ok(Box::new(SshBackend::new(ssh_config)))
        }
        other => {
            // Fail closed instead of silently downgrading to unsandboxed local
            // execution: a hermes-style `terminal_backend = "modal"` config must
            // not run commands on the host just because the Rust port does not
            // implement that backend yet (hermes implements modal/vercel/daytona).
            anyhow::bail!(
                "Terminal backend '{}' is not implemented in the Rust port — refusing to fall back to unsandboxed local execution. Use \"local\", \"docker\", or \"ssh\".",
                other
            )
        }
    }
}

impl SshBackend {
    /// Build the remote shell command string.
    ///
    /// Every model-controlled component is neutralized before reaching the
    /// remote shell — hermes parity with `shlex.quote` (see
    /// `hermes-agent/tools/terminal_tool.py::_validate_workdir` and
    /// `hermes-agent/tools/environments/ssh.py`):
    ///
    /// - `cwd` is shell-quoted (a `;`/`$()`/backtick payload would otherwise
    ///   be executed by the remote shell)
    /// - env values are shell-quoted (double-quoting alone leaves `$()`,
    ///   backticks, and `\` active)
    /// - env names are validated against `[A-Za-z_][A-Za-z0-9_]*` (an unquoted
    ///   name could inject separators)
    ///
    /// The command itself is shell-quoted when `use_shell` is set.
    fn build_remote_command(
        command: &str,
        cwd: Option<&Path>,
        env_vars: &HashMap<String, String>,
        use_shell: bool,
    ) -> anyhow::Result<String> {
        let mut remote_cmd = String::new();

        // Working directory — shell-quoted (hermes `shlex.quote` parity).
        if let Some(dir) = cwd {
            remote_cmd.push_str(&format!(
                "cd {} && ",
                shell_words::quote(&dir.to_string_lossy())
            ));
        }

        // Environment variables — validate the name, shell-quote the value.
        for (k, v) in env_vars {
            if !is_valid_env_name(k) {
                anyhow::bail!("Invalid environment variable name: {:?}", k);
            }
            remote_cmd.push_str(&format!("export {}={} && ", k, shell_words::quote(v)));
        }

        if use_shell {
            let shell = platform::detect_shell();
            remote_cmd.push_str(&format!(
                "{} {} {}",
                shell.path.to_string_lossy(),
                shell.args_pattern.join(" "),
                shell_words::quote(command)
            ));
        } else {
            remote_cmd.push_str(command);
        }

        Ok(remote_cmd)
    }
}

/// Validate a shell environment variable name (`[A-Za-z_][A-Za-z0-9_]*`).
fn is_valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(k: &str, v: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(k.to_string(), v.to_string());
        m
    }

    #[test]
    fn ssh_env_values_are_shell_quoted() {
        let cmd = SshBackend::build_remote_command(
            "echo hi",
            None,
            &env("API_KEY", "$(rm -rf /tmp/x) && touch /tmp/y"),
            false,
        )
        .unwrap();
        // shell_words::quote single-quotes the value — no expansion remotely.
        assert!(cmd.contains("export API_KEY='$(rm -rf /tmp/x) && touch /tmp/y'"));
        assert!(!cmd.contains("export API_KEY=\"$(rm"));
    }

    #[test]
    fn ssh_cwd_is_shell_quoted() {
        let cmd = SshBackend::build_remote_command(
            "pwd",
            Some(Path::new("/tmp; touch /tmp/pwn")),
            &HashMap::new(),
            false,
        )
        .unwrap();
        assert!(cmd.contains("cd '/tmp; touch /tmp/pwn'"));
    }

    #[test]
    fn ssh_env_name_injection_is_rejected() {
        let bad = env("X; touch /tmp/pwn", "1");
        assert!(SshBackend::build_remote_command("true", None, &bad, false).is_err());
        let bad_digit = env("1BAD", "1");
        assert!(SshBackend::build_remote_command("true", None, &bad_digit, false).is_err());
    }

    #[test]
    fn create_backend_refuses_unimplemented_backends() {
        for kind in [
            crate::config::TerminalBackend::Modal,
            crate::config::TerminalBackend::Daytona,
            crate::config::TerminalBackend::VercelSandbox,
            crate::config::TerminalBackend::Singularity,
        ] {
            let mut config = crate::config::AppConfig::default();
            config.terminal_backend = kind.clone();
            match create_backend(&config) {
                Ok(_) => panic!("backend {} should be refused, not silently local", kind),
                Err(e) => {
                    let msg = e.to_string();
                    assert!(
                        msg.contains("not implemented"),
                        "expected fail-closed message, got: {msg}"
                    );
                }
            }
        }
    }

    #[test]
    fn create_backend_default_is_local() {
        let backend = create_backend(&crate::config::AppConfig::default()).unwrap();
        assert_eq!(backend.name(), "local");
    }
}
