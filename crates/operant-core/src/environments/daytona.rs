use std::time::Duration;

use async_trait::async_trait;
use tracing::warn;

use crate::error::{Error, Result};

use super::{Environment, EnvironmentResult};

/// Daytona cloud sandbox execution backend.
///
/// **Stub**: The Daytona Rust SDK is not yet available.  All execution methods
/// return an error indicating that the backend is not yet implemented.
pub struct DaytonaEnvironment {
    _image: String,
    _cwd: String,
}

impl DaytonaEnvironment {
    pub fn new(image: String) -> Self {
        Self {
            _image: image,
            _cwd: "/home/daytona".to_string(),
        }
    }

    pub fn with_cwd(mut self, cwd: String) -> Self {
        self._cwd = cwd;
        self
    }
}

#[async_trait]
impl Environment for DaytonaEnvironment {
    async fn execute(&self, _command: &str, _timeout: Option<Duration>) -> EnvironmentResult {
        warn!("DaytonaEnvironment::execute called but backend is not yet implemented");
        EnvironmentResult {
            stdout: String::new(),
            stderr: "Daytona backend is not yet implemented in Rust".to_string(),
            exit_code: -1,
            duration_ms: 0,
        }
    }

    async fn upload(&self, _src: &str, _dest: &str) -> Result<()> {
        Err(Error::Agent(
            "Daytona upload is not yet implemented".to_string(),
        ))
    }

    async fn download(&self, _src: &str, _dest: &str) -> Result<()> {
        Err(Error::Agent(
            "Daytona download is not yet implemented".to_string(),
        ))
    }

    async fn check_health(&self) -> Result<bool> {
        Err(Error::Agent(
            "Daytona backend is not yet implemented".to_string(),
        ))
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }
}
