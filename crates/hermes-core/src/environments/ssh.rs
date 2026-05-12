use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;
use tracing::debug;

use crate::error::{Error, Result};

use super::{run_command, Environment, EnvironmentResult};

/// Execute commands on a remote machine via SSH.
///
/// Spawns a fresh `ssh user@host bash -c <command>` per call.  Uses
/// StrictHostKeyChecking=accept-new and BatchMode=yes for non-interactive
/// operation.  ControlMaster persistence is not implemented in this port;
/// each call opens a new SSH connection.
pub struct SshEnvironment {
    host: String,
    user: String,
    port: u16,
    key_path: Option<String>,
    cwd: Option<String>,
}

impl SshEnvironment {
    pub fn new(host: String, user: String) -> Self {
        Self {
            host,
            user,
            port: 22,
            key_path: None,
            cwd: None,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn with_key_path(mut self, key_path: String) -> Self {
        self.key_path = Some(key_path);
        self
    }

    pub fn with_cwd(mut self, cwd: String) -> Self {
        self.cwd = Some(cwd);
        self
    }

    fn build_ssh_args(&self, remote_cmd: &str) -> Vec<String> {
        let mut args = Vec::new();

        args.push("-o".to_string());
        args.push("StrictHostKeyChecking=accept-new".to_string());
        args.push("-o".to_string());
        args.push("BatchMode=yes".to_string());
        args.push("-o".to_string());
        args.push("ConnectTimeout=10".to_string());

        if self.port != 22 {
            args.push("-p".to_string());
            args.push(self.port.to_string());
        }

        if let Some(ref key) = self.key_path {
            args.push("-i".to_string());
            args.push(key.clone());
        }

        args.push(format!("{}@{}", self.user, self.host));

        // Build remote command: optional cd + the actual command
        let mut remote = String::new();
        if let Some(ref cwd) = self.cwd {
            remote.push_str(&format!("cd {} && ", shell_quote(cwd)));
        }
        remote.push_str(remote_cmd);
        args.push(remote);

        args
    }
}

#[async_trait]
impl Environment for SshEnvironment {
    async fn execute(&self, command: &str, cmd_timeout: Option<Duration>) -> EnvironmentResult {
        let start = std::time::Instant::now();
        let args = self.build_ssh_args(command);

        let mut cmd = Command::new("ssh");
        cmd.args(&args);

        let (stdout, stderr, exit_code) = run_command(&mut cmd, cmd_timeout).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        debug!(exit_code, duration_ms, host = %self.host, "SshEnvironment::execute completed");

        EnvironmentResult {
            stdout,
            stderr,
            exit_code,
            duration_ms,
        }
    }

    async fn upload(&self, src: &str, dest: &str) -> Result<()> {
        let mut cmd = Command::new("scp");
        cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
        cmd.arg("-o").arg("BatchMode=yes");

        if self.port != 22 {
            cmd.arg("-P").arg(self.port.to_string());
        }
        if let Some(ref key) = self.key_path {
            cmd.arg("-i").arg(key);
        }

        cmd.arg(src);
        cmd.arg(format!("{}@{}:{}", self.user, self.host, dest));

        let output = cmd.output().await.map_err(Error::from)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!("scp upload failed: {}", stderr)));
        }
        Ok(())
    }

    async fn download(&self, src: &str, dest: &str) -> Result<()> {
        let mut cmd = Command::new("scp");
        cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
        cmd.arg("-o").arg("BatchMode=yes");

        if self.port != 22 {
            cmd.arg("-P").arg(self.port.to_string());
        }
        if let Some(ref key) = self.key_path {
            cmd.arg("-i").arg(key);
        }

        cmd.arg(format!("{}@{}:{}", self.user, self.host, src));
        cmd.arg(dest);

        let output = cmd.output().await.map_err(Error::from)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Agent(format!("scp download failed: {}", stderr)));
        }
        Ok(())
    }

    async fn check_health(&self) -> Result<bool> {
        let args = self.build_ssh_args("echo ok");
        let mut cmd = Command::new("ssh");
        cmd.args(&args);
        let output = cmd.output().await.map_err(Error::from)?;
        Ok(output.status.success())
    }

    async fn close(&self) -> Result<()> {
        // SSH connection is stateless (per-call); nothing to close.
        Ok(())
    }
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}
