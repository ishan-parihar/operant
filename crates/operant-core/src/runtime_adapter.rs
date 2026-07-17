//! Runtime adapter trait for platform abstraction.
//!
//! Modeled after zeroclaw's `runtime_traits.rs`. Abstracts platform differences
//! (shell access, filesystem, long-running processes) so the agent can adapt
//! its behavior to different execution environments (native, Docker, serverless,
//! embedded, etc.).

use std::path::{Path, PathBuf};

/// Runtime adapter that abstracts platform differences for the agent.
///
/// Implement this trait to port the agent to a new execution environment.
/// The adapter declares platform capabilities (shell access, filesystem,
/// long-running processes) and provides platform-specific implementations
/// for operations like spawning shell commands. The orchestration loop
/// queries these capabilities to adapt its behavior — for example, disabling
/// tool execution on runtimes without shell access.
///
/// Implementations must be `Send + Sync` because the adapter is shared
/// across async tasks on the Tokio runtime.
pub trait RuntimeAdapter: Send + Sync {
    /// Return the human-readable name of this runtime environment.
    ///
    /// Used in logs and diagnostics (e.g., `"native"`, `"docker"`,
    /// `"cloudflare-workers"`).
    fn name(&self) -> &str;

    /// Report whether this runtime supports shell command execution.
    ///
    /// When `false`, the agent disables shell-based tools. Serverless and
    /// edge runtimes typically return `false`.
    fn has_shell_access(&self) -> bool;

    /// Report whether this runtime supports filesystem read/write.
    ///
    /// When `false`, the agent disables file-based tools and falls back to
    /// in-memory storage.
    fn has_filesystem_access(&self) -> bool;

    /// Return the base directory for persistent storage on this runtime.
    ///
    /// Memory backends, logs, and other artifacts are stored under this path.
    /// Implementations should return a platform-appropriate writable directory.
    fn storage_path(&self) -> PathBuf;

    /// Report whether this runtime supports long-running background processes.
    ///
    /// When `true`, the agent may start the gateway server, heartbeat loop,
    /// and other persistent tasks. Serverless runtimes with short execution
    /// limits should return `false`.
    fn supports_long_running(&self) -> bool;

    /// Return the maximum memory budget in bytes for this runtime.
    ///
    /// A value of `0` (the default) indicates no limit. Constrained
    /// environments (embedded, serverless) should return their actual
    /// memory ceiling so the agent can adapt buffer sizes and caching.
    fn memory_budget(&self) -> u64 {
        0
    }

    /// Build a shell command process configured for this runtime.
    ///
    /// Constructs a [`tokio::process::Command`] that will execute `command`
    /// with `workspace_dir` as the working directory. Implementations may
    /// prepend sandbox wrappers, set environment variables, or redirect
    /// I/O as appropriate for the platform.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime does not support shell access or if
    /// the command cannot be constructed (e.g., missing shell binary).
    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command>;
}

/// A native runtime that supports full shell access, filesystem, and
/// long-running processes. This is the default for desktop/server deployments.
pub struct NativeRuntime {
    storage_dir: PathBuf,
}

impl Default for NativeRuntime {
    fn default() -> Self {
        Self::default_home()
    }
}

impl NativeRuntime {
    /// Create a new native runtime with the given storage directory.
    pub fn new(storage_dir: PathBuf) -> Self {
        Self { storage_dir }
    }

    /// Create a native runtime using the default operant home directory.
    ///
    /// Uses `~/.operant` as the storage path, falling back to `/tmp/.operant`
    /// if the home directory cannot be determined.
    pub fn default_home() -> Self {
        let home = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".operant");
        Self::new(home)
    }
}

impl RuntimeAdapter for NativeRuntime {
    fn name(&self) -> &str {
        "native"
    }

    fn has_shell_access(&self) -> bool {
        true
    }

    fn has_filesystem_access(&self) -> bool {
        true
    }

    fn storage_path(&self) -> PathBuf {
        self.storage_dir.clone()
    }

    fn supports_long_running(&self) -> bool {
        true
    }

    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg("-c").arg(command).current_dir(workspace_dir);
        Ok(cmd)
    }
}

/// A sandboxed runtime with restricted shell access and filesystem.
/// Useful for untrusted code execution or CI environments.
pub struct SandboxedRuntime {
    storage_dir: PathBuf,
    allow_shell: bool,
}

impl SandboxedRuntime {
    /// Create a new sandboxed runtime.
    pub fn new(storage_dir: PathBuf, allow_shell: bool) -> Self {
        Self {
            storage_dir,
            allow_shell,
        }
    }
}

impl RuntimeAdapter for SandboxedRuntime {
    fn name(&self) -> &str {
        "sandboxed"
    }

    fn has_shell_access(&self) -> bool {
        self.allow_shell
    }

    fn has_filesystem_access(&self) -> bool {
        true
    }

    fn storage_path(&self) -> PathBuf {
        self.storage_dir.clone()
    }

    fn supports_long_running(&self) -> bool {
        false
    }

    fn memory_budget(&self) -> u64 {
        256 * 1024 * 1024 // 256 MB
    }

    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        if !self.allow_shell {
            anyhow::bail!("shell access is disabled in sandboxed runtime");
        }
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg("-c").arg(command).current_dir(workspace_dir);
        Ok(cmd)
    }
}

/// A serverless runtime with no shell access, no filesystem, and no
/// long-running processes. Tools that require these capabilities are
/// automatically disabled.
pub struct ServerlessRuntime;

impl RuntimeAdapter for ServerlessRuntime {
    fn name(&self) -> &str {
        "serverless"
    }

    fn has_shell_access(&self) -> bool {
        false
    }

    fn has_filesystem_access(&self) -> bool {
        false
    }

    fn storage_path(&self) -> PathBuf {
        PathBuf::from("/tmp/operant-serverless")
    }

    fn supports_long_running(&self) -> bool {
        false
    }

    fn memory_budget(&self) -> u64 {
        128 * 1024 * 1024 // 128 MB
    }

    fn build_shell_command(
        &self,
        _command: &str,
        _workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        anyhow::bail!("shell access is not available in serverless runtime");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_runtime_has_full_capabilities() {
        let rt = NativeRuntime::new(PathBuf::from("/tmp/test"));
        assert_eq!(rt.name(), "native");
        assert!(rt.has_shell_access());
        assert!(rt.has_filesystem_access());
        assert!(rt.supports_long_running());
        assert_eq!(rt.memory_budget(), 0);
        assert_eq!(rt.storage_path(), PathBuf::from("/tmp/test"));
    }

    #[test]
    fn native_runtime_builds_shell_command() {
        let rt = NativeRuntime::new(PathBuf::from("/tmp/test"));
        let cmd = rt.build_shell_command("echo hello", Path::new(".")).unwrap();
        // Just verify it constructs without error
        drop(cmd);
    }

    #[test]
    fn sandboxed_runtime_respects_shell_flag() {
        let rt_no_shell = SandboxedRuntime::new(PathBuf::from("/tmp/test"), false);
        assert!(!rt_no_shell.has_shell_access());
        assert!(rt_no_shell.has_filesystem_access());
        assert!(!rt_no_shell.supports_long_running());
        assert_eq!(rt_no_shell.memory_budget(), 256 * 1024 * 1024);

        let rt_with_shell = SandboxedRuntime::new(PathBuf::from("/tmp/test"), true);
        assert!(rt_with_shell.has_shell_access());
    }

    #[test]
    fn sandboxed_runtime_rejects_shell_when_disabled() {
        let rt = SandboxedRuntime::new(PathBuf::from("/tmp/test"), false);
        let result = rt.build_shell_command("echo hello", Path::new("."));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("disabled"));
    }

    #[test]
    fn serverless_runtime_has_no_capabilities() {
        let rt = ServerlessRuntime;
        assert_eq!(rt.name(), "serverless");
        assert!(!rt.has_shell_access());
        assert!(!rt.has_filesystem_access());
        assert!(!rt.supports_long_running());
        assert_eq!(rt.memory_budget(), 128 * 1024 * 1024);
    }

    #[test]
    fn serverless_runtime_rejects_shell() {
        let rt = ServerlessRuntime;
        let result = rt.build_shell_command("echo hello", Path::new("."));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not available"));
    }
}
