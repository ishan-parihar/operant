use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;
use tracing::debug;

use crate::error::{Error, Result};

use super::{bash_command, run_command, Environment, EnvironmentResult};

/// Execute commands directly on the host machine.
///
/// Spawns a fresh `bash -c` process per call.  Environment variables and
/// working directory can be configured at construction time.
pub struct LocalEnvironment {
    cwd: Option<String>,
    env_vars: Vec<(String, String)>,
}

impl LocalEnvironment {
    pub fn new(cwd: Option<String>) -> Self {
        Self {
            cwd,
            env_vars: Vec::new(),
        }
    }

    pub fn with_env_vars(mut self, vars: Vec<(String, String)>) -> Self {
        self.env_vars = vars;
        self
    }
}

#[async_trait]
impl Environment for LocalEnvironment {
    async fn execute(&self, command: &str, cmd_timeout: Option<Duration>) -> EnvironmentResult {
        let start = std::time::Instant::now();
        let mut cmd = bash_command(command);

        if let Some(ref cwd) = self.cwd {
            cmd.current_dir(cwd);
        }

        cmd.env_clear();
        cmd.envs(self.env_vars.iter().map(|(k, v)| (k.clone(), v.clone())));

        let (stdout, stderr, exit_code) = run_command(&mut cmd, cmd_timeout).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        debug!(exit_code, duration_ms, "LocalEnvironment::execute completed");

        EnvironmentResult {
            stdout,
            stderr,
            exit_code,
            duration_ms,
        }
    }

    async fn upload(&self, src: &str, dest: &str) -> Result<()> {
        tokio::fs::copy(src, dest).await?;
        Ok(())
    }

    async fn download(&self, src: &str, dest: &str) -> Result<()> {
        tokio::fs::copy(src, dest).await?;
        Ok(())
    }

    async fn check_health(&self) -> Result<bool> {
        // Quick probe: run "echo ok" and check exit code
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo ok");
        let output = cmd.output().await.map_err(Error::from)?;
        Ok(output.status.success())
    }

    async fn close(&self) -> Result<()> {
        // Local environment has no resources to release
        Ok(())
    }
}
