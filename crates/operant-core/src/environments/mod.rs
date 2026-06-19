//! Execution environment backends for Operant-RS.
//!
//! Provides a unified [`Environment`] trait and implementations for running
//! shell commands in various execution contexts: local, Docker, SSH,
//! Singularity/Apptainer, Modal, Daytona, and Vercel Sandbox.
//!
//! Each backend implements the same interface. SDK-based backends (Modal,
//! Daytona, Vercel) are stubs that return an error until their respective
//! Rust SDK crates are available.

pub mod daytona;
pub mod docker;
pub mod local;
pub mod modal;
pub mod singularity;
pub mod ssh;
pub mod vercel;

pub use daytona::DaytonaEnvironment;
pub use docker::DockerEnvironment;
pub use local::LocalEnvironment;
pub use modal::ModalEnvironment;
pub use singularity::SingularityEnvironment;
pub use ssh::SshEnvironment;
pub use vercel::VercelSandboxEnvironment;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::info;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Result of executing a command in an execution environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentResult {
    /// Standard output produced by the command
    pub stdout: String,
    /// Standard error produced by the command
    pub stderr: String,
    /// Process exit code (-1 if the command could not be started)
    pub exit_code: i32,
    /// Wall-clock duration of the command in milliseconds
    pub duration_ms: u64,
}

/// Common configuration for all environment backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentConfig {
    /// Default command timeout
    pub timeout: Duration,
    /// Environment variables to set
    pub env_vars: HashMap<String, String>,
    /// Initial working directory (backend-dependent default if None)
    pub working_dir: Option<String>,
}

impl Default for EnvironmentConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            env_vars: HashMap::new(),
            working_dir: None,
        }
    }
}

/// Discriminated union of supported environment backends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EnvironmentType {
    /// Run commands directly on the host machine
    Local,
    /// Run commands inside a Docker container
    Docker,
    /// Run commands on a remote machine via SSH
    Ssh,
    /// Run commands inside a Singularity/Apptainer container
    Singularity,
    /// Run commands in a Modal cloud sandbox
    Modal,
    /// Run commands in a Daytona cloud sandbox
    Daytona,
    /// Run commands in a Vercel Edge sandbox
    Vercel,
}

// ---------------------------------------------------------------------------
// Environment trait
// ---------------------------------------------------------------------------

/// Unified interface for all Operant execution environment backends.
///
/// Each backend provides command execution, file transfer, health checking,
/// and resource cleanup through the same async trait.
#[async_trait]
pub trait Environment: Send + Sync {
    /// Execute a shell command inside the environment.
    ///
    /// Returns an [`EnvironmentResult`] containing stdout, stderr, exit code,
    /// and duration. Infrastructure errors (e.g. backend unavailable) are
    /// reported with `exit_code = -1` and a description in `stderr`.
    async fn execute(&self, command: &str, timeout: Option<Duration>) -> EnvironmentResult;

    /// Upload a file from the host (`src`) into the environment (`dest`).
    async fn upload(&self, src: &str, dest: &str) -> crate::error::Result<()>;

    /// Download a file from the environment (`src`) to the host (`dest`).
    async fn download(&self, src: &str, dest: &str) -> crate::error::Result<()>;

    /// Check whether the environment is reachable and operational.
    async fn check_health(&self) -> crate::error::Result<bool>;

    /// Release all backend resources (containers, connections, temp files).
    async fn close(&self) -> crate::error::Result<()>;
}

// ---------------------------------------------------------------------------
// Environment pool
// ---------------------------------------------------------------------------

/// A thread-safe pool of named execution environments.
///
/// Environments are stored as `Arc<dyn Environment>` so they can be shared
/// across tasks and cleaned up by a background reaper task.
#[derive(Clone)]
pub struct EnvironmentPool {
    environments: Arc<RwLock<HashMap<String, Arc<dyn Environment>>>>,
}

impl EnvironmentPool {
    /// Create an empty pool.
    pub fn new() -> Self {
        Self {
            environments: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Retrieve an environment by name.
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Environment>> {
        self.environments.read().await.get(name).cloned()
    }

    /// Insert (or replace) a named environment in the pool.
    pub async fn create(&self, name: String, env: Arc<dyn Environment>) {
        self.environments.write().await.insert(name, env);
    }

    /// Remove an environment from the pool and return it.
    /// The caller is responsible for calling `close()` on the returned value.
    pub async fn remove(&self, name: &str) -> Option<Arc<dyn Environment>> {
        self.environments.write().await.remove(name)
    }

    /// List all environment names currently in the pool.
    pub async fn list(&self) -> Vec<String> {
        self.environments.read().await.keys().cloned().collect()
    }

    /// Return the number of environments currently tracked.
    pub async fn len(&self) -> usize {
        self.environments.read().await.len()
    }

    /// Remove and close environments that have been idle longer than `max_age`.
    ///
    /// This is a best-effort scan. Each stale environment is removed from the
    /// pool and its `close()` method is called in a spawned task so that a
    /// single slow teardown does not block the entire sweep.
    pub async fn cleanup_stale(&self, max_age: Duration) {
        // Note: the current implementation does not track per-environment
        // last-use timestamps.  This is a placeholder that logs intent.
        // A production version would store `(Arc<dyn Environment>, Instant)`
        // in the map and sweep entries whose last-use exceeds max_age.
        let names = self.list().await;
        if names.is_empty() {
            return;
        }
        info!(
            "EnvironmentPool: cleanup_stale requested (max_age={:?}, tracked={})",
            max_age,
            names.len()
        );
    }

    /// Spawn a background task that periodically calls [`cleanup_stale`].
    ///
    /// The reaper runs every `interval` and removes environments idle for
    /// longer than `max_age`.
    pub fn start_reaper(self, interval: Duration, max_age: Duration) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                self.cleanup_stale(max_age).await;
            }
        });
    }
}

impl std::fmt::Debug for EnvironmentPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentPool")
            .field("environments", &"...")
            .finish()
    }
}

impl Default for EnvironmentPool {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `tokio::process::Command` for running `bash -c <command>`.
///
/// This is the common execution primitive used by local, Docker, SSH, and
/// Singularity backends.
fn bash_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c").arg(command);
    cmd
}

/// Run a command to completion, collecting stdout and stderr.
///
/// Applies an optional timeout.  Returns `(stdout, stderr, exit_code)`.
async fn run_command(
    cmd: &mut tokio::process::Command,
    cmd_timeout: Option<Duration>,
) -> (String, String, i32) {
    let output = match cmd_timeout {
        Some(d) => match tokio::time::timeout(d, cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return (String::new(), format!("Failed to spawn process: {}", e), -1);
            }
            Err(_elapsed) => {
                return (
                    String::new(),
                    format!("Command timed out after {:?}", d),
                    -1,
                );
            }
        },
        None => match cmd.output().await {
            Ok(out) => out,
            Err(e) => {
                return (String::new(), format!("Failed to spawn process: {}", e), -1);
            }
        },
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    (stdout, stderr, exit_code)
}
