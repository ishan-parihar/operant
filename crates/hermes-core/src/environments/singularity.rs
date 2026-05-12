use std::time::Duration;

use async_trait::async_trait;
use tracing::debug;

use crate::error::{Error, Result};

use super::{bash_command, run_command, Environment, EnvironmentResult};

/// Execute commands inside a Singularity or Apptainer container instance.
///
/// Uses `singularity exec instance://<name> bash -c <command>` (or `apptainer`
/// if that is the available executable).  The instance must have been started
/// externally before commands can be executed.
pub struct SingularityEnvironment {
    /// Container runtime: "singularity" or "apptainer"
    executable: String,
    /// Instance name (as started via `singularity instance start`)
    instance_name: String,
    cwd: Option<String>,
}

impl SingularityEnvironment {
    pub fn new(instance_name: String) -> Self {
        let executable = Self::detect_executable();
        Self {
            executable,
            instance_name,
            cwd: None,
        }
    }

    pub fn with_cwd(mut self, cwd: String) -> Self {
        self.cwd = Some(cwd);
        self
    }

    fn detect_executable() -> String {
        // Check for apptainer first, fall back to singularity.
        std::process::Command::new("which")
            .arg("apptainer")
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some("apptainer".to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "singularity".to_string())
    }
}

#[async_trait]
impl Environment for SingularityEnvironment {
    async fn execute(&self, command: &str, cmd_timeout: Option<Duration>) -> EnvironmentResult {
        let start = std::time::Instant::now();

        // Build: <executable> exec instance://<name> bash -c <command>
        let full_cmd = format!(
            "{} exec instance://{} bash -c {}",
            self.executable,
            shell_quote(&self.instance_name),
            shell_quote(command),
        );

        let mut cmd = bash_command(&full_cmd);

        if let Some(ref cwd) = self.cwd {
            cmd.current_dir(cwd);
        }

        let (stdout, stderr, exit_code) = run_command(&mut cmd, cmd_timeout).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        debug!(
            exit_code,
            duration_ms,
            instance = %self.instance_name,
            "SingularityEnvironment::execute completed"
        );

        EnvironmentResult {
            stdout,
            stderr,
            exit_code,
            duration_ms,
        }
    }

    async fn upload(&self, src: &str, dest: &str) -> Result<()> {
        let full_cmd = format!(
            "{} exec instance://{} cp {} {}",
            self.executable,
            shell_quote(&self.instance_name),
            shell_quote(src),
            shell_quote(dest),
        );
        let mut cmd = bash_command(&full_cmd);
        let output = cmd.output().await.map_err(Error::from)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!(
                "Singularity upload failed: {}",
                stderr
            )));
        }
        Ok(())
    }

    async fn download(&self, src: &str, dest: &str) -> Result<()> {
        let full_cmd = format!(
            "{} exec instance://{} cp {} {}",
            self.executable,
            shell_quote(&self.instance_name),
            shell_quote(src),
            shell_quote(dest),
        );
        let mut cmd = bash_command(&full_cmd);
        let output = cmd.output().await.map_err(Error::from)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!(
                "Singularity download failed: {}",
                stderr
            )));
        }
        Ok(())
    }

    async fn check_health(&self) -> Result<bool> {
        let full_cmd = format!(
            "{} exec instance://{} echo ok",
            self.executable,
            shell_quote(&self.instance_name),
        );
        let mut cmd = bash_command(&full_cmd);
        let output = cmd.output().await.map_err(Error::from)?;
        Ok(output.status.success())
    }

    async fn close(&self) -> Result<()> {
        // Instance lifecycle is managed externally.
        Ok(())
    }
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}
