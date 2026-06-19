use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;
use tracing::debug;

use crate::error::{Error, Result};

use super::{bash_command, run_command, Environment, EnvironmentResult};

/// Execute commands inside a Docker container.
///
/// Spawns `docker exec <container> bash -c <command>` per call.  The
/// container must already be running; this backend does not manage the
/// container lifecycle.
pub struct DockerEnvironment {
    container_id: String,
    cwd: Option<String>,
    env_vars: Vec<(String, String)>,
}

impl DockerEnvironment {
    pub fn new(container_id: String) -> Self {
        Self {
            container_id,
            cwd: None,
            env_vars: Vec::new(),
        }
    }

    pub fn with_cwd(mut self, cwd: String) -> Self {
        self.cwd = Some(cwd);
        self
    }

    pub fn with_env_vars(mut self, vars: Vec<(String, String)>) -> Self {
        self.env_vars = vars;
        self
    }
}

#[async_trait]
impl Environment for DockerEnvironment {
    async fn execute(&self, command: &str, cmd_timeout: Option<Duration>) -> EnvironmentResult {
        let start = std::time::Instant::now();
        let full_cmd = format!(
            "docker exec {} {} bash -c {}",
            if self.env_vars.is_empty() {
                String::new()
            } else {
                self.env_vars
                    .iter()
                    .map(|(k, v)| format!("-e {}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(" ")
            },
            shell_quote(&self.container_id),
            shell_quote(command),
        );

        #[expect(clippy::disallowed_methods)]
        let mut cmd = bash_command(&full_cmd);

        if let Some(ref cwd) = self.cwd {
            cmd.current_dir(cwd);
        }

        let (stdout, stderr, exit_code) = run_command(&mut cmd, cmd_timeout).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        debug!(exit_code, duration_ms, container = %self.container_id, "DockerEnvironment::execute completed");

        EnvironmentResult {
            stdout,
            stderr,
            exit_code,
            duration_ms,
        }
    }

    async fn upload(&self, src: &str, dest: &str) -> Result<()> {
        let mut cmd = Command::new("docker");
        cmd.arg("cp")
            .arg(src)
            .arg(format!("{}:{}", self.container_id, dest));
        let output = cmd.output().await.map_err(Error::from)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!("docker cp failed: {}", stderr)));
        }
        Ok(())
    }

    async fn download(&self, src: &str, dest: &str) -> Result<()> {
        let mut cmd = Command::new("docker");
        cmd.arg("cp")
            .arg(format!("{}:{}", self.container_id, src))
            .arg(dest);
        let output = cmd.output().await.map_err(Error::from)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!("docker cp failed: {}", stderr)));
        }
        Ok(())
    }

    async fn check_health(&self) -> Result<bool> {
        let mut cmd = Command::new("docker");
        cmd.arg("inspect")
            .arg("-f")
            .arg("{{.State.Running}}")
            .arg(&self.container_id);
        let output = cmd.output().await.map_err(Error::from)?;
        let status = String::from_utf8_lossy(&output.stdout).trim() == "true";
        Ok(status)
    }

    async fn close(&self) -> Result<()> {
        // Container lifecycle is managed externally; nothing to close here.
        Ok(())
    }
}

fn shell_quote(s: &str) -> String {
    // Minimal shell quoting: wrap in single quotes and escape embedded single quotes.
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}
