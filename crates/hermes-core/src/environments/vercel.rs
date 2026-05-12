use std::time::Duration;

use async_trait::async_trait;
use tracing::warn;

use crate::error::{Error, Result};

use super::{Environment, EnvironmentResult};

/// Vercel Edge sandbox execution backend.
///
/// **Stub**: The Vercel Rust SDK is not yet available.  All execution methods
/// return an error indicating that the backend is not yet implemented.
pub struct VercelSandboxEnvironment {
    _runtime: Option<String>,
    _cwd: String,
}

impl VercelSandboxEnvironment {
    pub fn new() -> Self {
        Self {
            _runtime: None,
            _cwd: "/vercel/sandbox".to_string(),
        }
    }

    pub fn with_runtime(mut self, runtime: String) -> Self {
        self._runtime = Some(runtime);
        self
    }

    pub fn with_cwd(mut self, cwd: String) -> Self {
        self._cwd = cwd;
        self
    }
}

impl Default for VercelSandboxEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Environment for VercelSandboxEnvironment {
    async fn execute(&self, _command: &str, _timeout: Option<Duration>) -> EnvironmentResult {
        warn!(
            "VercelSandboxEnvironment::execute called but backend is not yet implemented"
        );
        EnvironmentResult {
            stdout: String::new(),
            stderr: "Vercel Sandbox backend is not yet implemented in Rust".to_string(),
            exit_code: -1,
            duration_ms: 0,
        }
    }

    async fn upload(&self, _src: &str, _dest: &str) -> Result<()> {
        Err(Error::Agent(
            "Vercel Sandbox upload is not yet implemented".to_string(),
        ))
    }

    async fn download(&self, _src: &str, _dest: &str) -> Result<()> {
        Err(Error::Agent(
            "Vercel Sandbox download is not yet implemented".to_string(),
        ))
    }

    async fn check_health(&self) -> Result<bool> {
        Err(Error::Agent(
            "Vercel Sandbox backend is not yet implemented".to_string(),
        ))
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }
}
