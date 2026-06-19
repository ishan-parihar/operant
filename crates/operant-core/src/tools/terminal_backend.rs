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
use tracing::{debug, warn};

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
        if let Ok(override_path) = std::env::var("HERMES_DOCKER_BINARY") {
            if !override_path.is_empty() {
                return override_path;
            }
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

    /// Start a container if not already running.
    async fn ensure_container(&mut self) -> anyhow::Result<String> {
        if let Some(ref id) = self.container_id {
            return Ok(id.clone());
        }

        let container_name = format!("operant-{}", &uuid::Uuid::new_v4().to_string()[..8]);

        let mut run_args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            container_name.clone(),
        ];

        if !self.config.cwd.is_empty() {
            run_args.extend(["-w".to_string(), self.config.cwd.clone()]);
        }

        for vol in &self.config.volumes {
            run_args.extend(["-v".to_string(), vol.clone()]);
        }

        for (k, v) in &self.config.env {
            run_args.extend(["-e".to_string(), format!("{}={}", k, v)]);
        }

        if self.config.cpu > 0.0 {
            run_args.extend(["--cpus".to_string(), self.config.cpu.to_string()]);
        }
        if self.config.memory_mb > 0 {
            run_args.extend([
                "--memory".to_string(),
                format!("{}m", self.config.memory_mb),
            ]);
        }

        // Security: drop all capabilities, add back minimal set
        run_args.extend([
            "--cap-drop".to_string(),
            "ALL".to_string(),
            "--cap-add".to_string(),
            "DAC_OVERRIDE".to_string(),
            "--cap-add".to_string(),
            "CHOWN".to_string(),
            "--cap-add".to_string(),
            "FOWNER".to_string(),
            "--security-opt".to_string(),
            "no-new-privileges".to_string(),
            "--pids-limit".to_string(),
            "256".to_string(),
        ]);

        run_args.push(self.config.image.clone());
        run_args.extend(["sleep".to_string(), "infinity".to_string()]);

        debug!(
            "Starting Docker container: {} {}",
            self.docker_exe,
            run_args.join(" ")
        );

        let output = Command::new(&self.docker_exe)
            .args(&run_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start Docker container: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Docker run failed: {}", stderr));
        }

        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        self.container_id = Some(id.clone());
        debug!(
            "Started container {} ({})",
            container_name,
            &id[..12.min(id.len())]
        );
        Ok(id)
    }

    /// Find or create a container, handling "no such container" errors.
    async fn recover_container(&mut self) -> anyhow::Result<()> {
        warn!("Container gone — attempting recovery");
        self.container_id = None;
        self.ensure_container().await?;
        Ok(())
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

    /// Test SSH connectivity.
    fn test_connection(&self) -> bool {
        let cmd = self.build_ssh_command(&[
            "echo".to_string(),
            "'SSH connection established'".to_string(),
        ]);
        std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
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
        let mut remote_cmd = String::new();

        // Set working directory
        if let Some(dir) = cwd {
            remote_cmd.push_str(&format!("cd {} && ", dir.to_string_lossy()));
        }

        // Export environment variables
        for (k, v) in env_vars {
            remote_cmd.push_str(&format!("export {}=\"{}\" && ", k, v));
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
pub fn create_backend(config: &AppConfig) -> Box<dyn TerminalBackend> {
    match &config.terminal_backend {
        BackendKind::Local => Box::new(LocalBackend),
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
            Box::new(DockerBackend::new(docker_config))
        }
        BackendKind::Ssh => {
            let ssh_config = SshConfig {
                host: config.tools.terminal.ssh.host.clone().unwrap_or_default(),
                user: config.tools.terminal.ssh.user.clone().unwrap_or_default(),
                port: config.tools.terminal.ssh.port,
                key_path: config.tools.terminal.ssh.key_path.clone(),
            };
            Box::new(SshBackend::new(ssh_config))
        }
        other => {
            warn!(
                "Terminal backend '{}' not yet implemented in Rust, falling back to local",
                other
            );
            Box::new(LocalBackend)
        }
    }
}
