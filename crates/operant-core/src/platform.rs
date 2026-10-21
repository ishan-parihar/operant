//! Cross-platform utility module for operant-rs.
//!
//! Provides shell detection, config directory helpers, file permission utilities,
//! interpreter detection, and platform information using only the standard library.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A CRATE-WIDE lock for tests that mutate the process environment.
///
/// The environment is process-global, so any test that changes `HOME`,
/// `HERMES_HOME` or a path derived from them changes what EVERY other
/// test sees. Each module used to have its OWN `env_lock()`, so the
/// modules did not exclude each other — and `get_default_operant_root()`
/// computes the root from the CURRENT `HERMES_HOME` and `HOME`, so an
/// interleaved mutation changes the root mid-test.
///
/// The observed symptom was `Profile 'testprofile' does not exist` in
/// `profile.rs`'s tests, followed by three more failures with
/// `PoisonError` — because the first failure poisoned that module's lock
/// and every later `env_lock().lock().unwrap()` then panicked.
///
/// One lock, shared by every test that touches the environment. It is
/// poison-tolerant: a test that panics while holding it must not take
/// every other environment test down with it.
#[cfg(test)]
static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The environment lock, for an ASYNC test.
///
/// The lock is async-aware: a `std::sync::MutexGuard` held across an
/// `.await` is a `!Send` hazard, and the browser tests bracket the await
/// with the mutation.
#[cfg(test)]
pub(crate) async fn test_env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    TEST_ENV_LOCK.lock().await
}

/// The environment lock, for a SYNCHRONOUS test.
///
/// The SAME lock as the async accessor — one lock, or the two would not
/// exclude each other, which is the whole point.
///
/// `blocking_lock` is safe here: a sync `#[test]` runs outside any tokio
/// runtime, and `blocking_lock` panics if it finds one.
#[cfg(test)]
pub(crate) fn test_env_lock_blocking() -> tokio::sync::MutexGuard<'static, ()> {
    TEST_ENV_LOCK.blocking_lock()
}

// ---------------------------------------------------------------------------
// 1. Shell detection
// ---------------------------------------------------------------------------

/// Information about the detected shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellInfo {
    /// Absolute path (or bare command name) of the shell executable.
    pub path: PathBuf,
    /// The flag used to pass a command string, e.g. `["-c"]` or `["/C"]`.
    pub args_pattern: Vec<String>,
}

/// Find the best available shell on the current platform.
///
/// - **Windows**: tries `pwsh.exe`, then `powershell.exe`, then `cmd.exe`.
/// - **Unix**: tries the `$SHELL` env var, then `bash`, then `sh`.
pub fn detect_shell() -> ShellInfo {
    #[cfg(target_os = "windows")]
    {
        detect_shell_windows()
    }
    #[cfg(not(target_os = "windows"))]
    {
        detect_shell_unix()
    }
}

#[cfg(target_os = "windows")]
fn detect_shell_windows() -> ShellInfo {
    for (cmd, args) in &[
        ("pwsh.exe", vec!["-Command"]),
        ("powershell.exe", vec!["-Command"]),
        ("cmd.exe", vec!["/C"]),
    ] {
        if which::which(cmd).is_ok() {
            return ShellInfo {
                path: PathBuf::from(cmd),
                args_pattern: args.iter().map(|s| s.to_string()).collect(),
            };
        }
    }
    // Fallback – cmd.exe should always be present on Windows.
    ShellInfo {
        path: PathBuf::from("cmd.exe"),
        args_pattern: vec!["/C".to_string()],
    }
}

#[cfg(not(target_os = "windows"))]
fn detect_shell_unix() -> ShellInfo {
    if let Ok(shell) = env::var("SHELL")
        && !shell.is_empty()
        && which::which(&shell).is_ok()
    {
        return ShellInfo {
            path: PathBuf::from(shell),
            args_pattern: vec!["-c".to_string()],
        };
    }
    for candidate in &["bash", "sh"] {
        if which::which(candidate).is_ok() {
            return ShellInfo {
                path: PathBuf::from(candidate),
                args_pattern: vec!["-c".to_string()],
            };
        }
    }
    // Ultimate fallback.
    ShellInfo {
        path: PathBuf::from("sh"),
        args_pattern: vec!["-c".to_string()],
    }
}

// (iter-148: command_exists deleted — use which::which(cmd).is_ok()
// instead, already in deps. Was a hand-rolled fork+exec that runs
// `cmd --version` which doesn't work for all commands.)

// ---------------------------------------------------------------------------
// 2. Config directory helpers
// ---------------------------------------------------------------------------

/// The operant home ROOT — the directory that holds `profiles/` and the
/// sticky `active_profile` file.
///
/// `HERMES_HOME` wins when it is set (an explicit operator choice);
/// `~/.operant` is the fallback. This is the root REGARDLESS of which
/// profile is active, so the profile list and the sticky file stay in one
/// place when a profile is switched on.
///
/// A `HERMES_HOME` that points INTO a profile directory
/// (`…/profiles/<name>`) resolves its root to the grandparent, so
/// `profiles/` and `active_profile` stay BESIDE the profile instead of
/// inside it.
pub fn default_operant_root() -> PathBuf {
    if let Some(explicit) = explicit_home() {
        if explicit.parent().is_some_and(|p| p.ends_with("profiles"))
            && let Some(root) = explicit.parent().and_then(Path::parent)
        {
            return root.to_path_buf();
        }
        return explicit;
    }
    os_default_home()
}

/// The effective operant home for this process.
///
/// Resolution order:
/// 1. `$HERMES_HOME` — an explicit operator choice. It wins over the
///    sticky profile: a deployment that sets it means it.
/// 2. The sticky active profile — `<root>/profiles/<name>`, when
///    `<root>/active_profile` names a profile that exists.
/// 3. The default root (`~/.operant`).
///
/// This is THE resolver: every store under the operant home resolves
/// through it, so a profile is isolated from the default one and the
/// operator's `HERMES_HOME` still overrides everything.
pub fn operant_home() -> PathBuf {
    if let Some(explicit) = explicit_home() {
        return explicit;
    }
    let root = os_default_home();
    match active_profile_name(&root) {
        Some(name) => {
            let dir = root.join("profiles").join(name);
            if dir.is_dir() { dir } else { root }
        }
        None => root,
    }
}

/// `$HERMES_HOME`, when it is set and non-empty.
fn explicit_home() -> Option<PathBuf> {
    env::var("HERMES_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The OS default home — `~/.operant` on Unix, `%APPDATA%\operant` on
/// Windows. The home BEFORE `HERMES_HOME` and before any profile.
///
/// It stays a credential root even when `HERMES_HOME` points elsewhere: the
/// operator's canonical stores live there, and a path that is a credential
/// store in one deployment must stay one in another.
pub fn os_default_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = env::var("APPDATA").unwrap_or_else(|_| {
            let home = env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Default".to_string());
            format!("{home}\\AppData\\Roaming")
        });
        PathBuf::from(base).join("operant")
    }

    #[cfg(not(target_os = "windows"))]
    {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".operant")
    }
}

/// The profile named by the sticky `<root>/active_profile` file.
///
/// The name must be a plain profile token: the file steers a PATH, so a
/// tampered or malformed value must not escape the profiles directory.
/// A name that does not parse is ignored (the default profile stays
/// active) rather than trusted.
fn active_profile_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("active_profile")).ok()?;
    let name = text.trim();
    if name.is_empty() || !crate::profile::is_profile_id(name) {
        return None;
    }
    Some(name.to_string())
}

/// The Holon run-environment contract, read once (audit O-01/O-02).
///
/// Holon exports the run's directory as `HOLON_RUN_DIR` (CONTRACT §1, wave-1
/// H-01) with `HOLON_BRIEF_DIR` as its documented alias — both name the run
/// directory holding `BRIEF.md`, `policy.json` and `result.json`. Older
/// kernels export only the alias; operant accepts either, preferring the
/// canonical name. The employee arrives as `HOLON_EMPLOYEE_ID`, with
/// `HOLON_EMPLOYEE` as a legacy alias.
///
/// These live here (not in operant-os) because the agent loop and the cron
/// tool — core code — read them; operant-os re-exports for its own call sites.
///
/// The run directory for THIS process if it is a Holon body: the dir the
/// kernel prepared, carrying the brief, the policy and the result path.
pub fn holon_run_dir() -> Option<PathBuf> {
    for key in ["HOLON_RUN_DIR", "HOLON_BRIEF_DIR"] {
        if let Some(v) = env::var_os(key)
            && !v.is_empty()
        {
            return Some(PathBuf::from(v));
        }
    }
    None
}

/// Whether THIS process is a Holon body at all (run dir present).
pub fn is_holon_run() -> bool {
    holon_run_dir().is_some()
}

/// The employee id this body runs as, if the kernel named one.
pub fn holon_employee() -> Option<String> {
    for key in ["HOLON_EMPLOYEE_ID", "HOLON_EMPLOYEE"] {
        if let Ok(v) = env::var(key)
            && !v.is_empty()
        {
            return Some(v);
        }
    }
    None
}

/// `<operant_home>/config`
pub fn operant_config_dir() -> PathBuf {
    operant_home().join("config")
}

/// `<operant_home>/data`
pub fn operant_data_dir() -> PathBuf {
    operant_home().join("data")
}

/// `<operant_home>/memories`
pub fn operant_memories_dir() -> PathBuf {
    operant_home().join("memories")
}

/// `<operant_home>/skills`
pub fn operant_skills_dir() -> PathBuf {
    operant_home().join("skills")
}

/// `<operant_home>/sessions`
pub fn operant_sessions_dir() -> PathBuf {
    operant_home().join("sessions")
}

// ---------------------------------------------------------------------------
// 3. File permissions helpers
// ---------------------------------------------------------------------------

/// Set secure permissions on a path (mode `0o700` on Unix, no-op on Windows).
pub fn set_secure_permissions(path: &std::path::Path) -> std::io::Result<()> {
    set_file_permissions(path, 0o700)
}

/// Set file permissions to `mode` on Unix; no-op on Windows.
#[cfg(not(target_os = "windows"))]
pub fn set_file_permissions(path: &std::path::Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, perms)
}

/// Set file permissions – no-op on Windows.
#[cfg(target_os = "windows")]
pub fn set_file_permissions(path: &std::path::Path, _mode: u32) -> std::io::Result<()> {
    let _ = path;
    Ok(())
}

// ---------------------------------------------------------------------------
// 4. Interpreter detection
// ---------------------------------------------------------------------------

/// Try to find a working Python interpreter (`python3` first, then `python`).
pub fn find_python() -> Option<PathBuf> {
    for candidate in &["python3", "python"] {
        if interpreter_works(candidate) {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

/// Try to find a working Node.js interpreter.
pub fn find_node() -> Option<PathBuf> {
    if interpreter_works("node") {
        return Some(PathBuf::from("node"));
    }
    None
}

/// Try to find `rustc`.
pub fn find_rustc() -> Option<PathBuf> {
    if interpreter_works("rustc") {
        return Some(PathBuf::from("rustc"));
    }
    None
}

/// Returns `true` when `cmd --version` exits successfully.
fn interpreter_works(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 5. Platform info
// ---------------------------------------------------------------------------

/// Operating system name as a short lowercase string.
pub fn os_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        env::consts::OS
    }
}

/// `true` when compiled for Windows.
pub const fn is_windows() -> bool {
    cfg!(target_os = "windows")
}

/// `true` when compiled for macOS.
pub const fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

/// `true` when compiled for Linux.
pub const fn is_linux() -> bool {
    cfg!(target_os = "linux")
}

/// Snapshot of the current platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformInfo {
    pub os: &'static str,
    pub arch: &'static str,
    pub shell: ShellInfo,
}

/// Collect platform information in one call.
pub fn platform_info() -> PlatformInfo {
    PlatformInfo {
        os: os_name(),
        arch: env::consts::ARCH,
        shell: detect_shell(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -- Shell detection ----------------------------------------------------

    #[test]
    fn test_detect_shell_returns_non_empty_path() {
        let shell = detect_shell();
        assert!(
            !shell.path.as_os_str().is_empty(),
            "shell path must not be empty"
        );
    }

    #[test]
    fn test_detect_shell_has_args_pattern() {
        let shell = detect_shell();
        assert!(
            !shell.args_pattern.is_empty(),
            "args_pattern must contain at least one element"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_detect_shell_windows_known_shells() {
        let shell = detect_shell();
        let name = shell.path.to_string_lossy().to_lowercase();
        assert!(
            name.contains("pwsh") || name.contains("powershell") || name.contains("cmd"),
            "expected a known Windows shell, got: {name}"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_detect_shell_unix_args() {
        let shell = detect_shell();
        assert_eq!(shell.args_pattern, vec!["-c"]);
    }

    // -- Config directories -------------------------------------------------

    #[test]
    #[serial_test::serial]
    fn test_operant_home_respects_env() {
        // The crate-wide env lock: every test that mutates HOME or
        // HERMES_HOME must hold it, or it races the profile tests (which
        // create and delete real directories under the resolved root).
        let _guard = test_env_lock_blocking();
        let key = "HERMES_HOME";
        let original = env::var(key).ok();

        // SAFETY: test-only env mutation under #[serial_test::serial]
        unsafe { env::set_var(key, "/tmp/operant_test_home") };
        assert_eq!(operant_home(), PathBuf::from("/tmp/operant_test_home"));

        // Restore original value.
        match original {
            Some(val) => unsafe { env::set_var(key, val) },
            None => unsafe { env::remove_var(key) },
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_operant_home_default_not_empty() {
        let _guard = test_env_lock_blocking();
        let key = "HERMES_HOME";
        let original = env::var(key).ok();

        // SAFETY: test-only env mutation under #[serial_test::serial]
        unsafe { env::remove_var(key) };
        let home = operant_home();
        assert!(!home.as_os_str().is_empty());

        if let Some(val) = original {
            unsafe { env::set_var(key, val) };
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_operant_subdirs_are_children_of_home() {
        let home = operant_home();
        assert!(operant_config_dir().starts_with(&home));
        assert!(operant_data_dir().starts_with(&home));
        assert!(operant_memories_dir().starts_with(&home));
        assert!(operant_skills_dir().starts_with(&home));
        assert!(operant_sessions_dir().starts_with(&home));
    }

    #[test]
    fn test_operant_subdir_names() {
        // Verify suffix only — avoids race with tests that modify HERMES_HOME.
        assert!(operant_config_dir().ends_with("config"));
        assert!(operant_data_dir().ends_with("data"));
        assert!(operant_memories_dir().ends_with("memories"));
        assert!(operant_skills_dir().ends_with("skills"));
        assert!(operant_sessions_dir().ends_with("sessions"));
    }

    // -- The active profile -------------------------------------------------

    /// Build a fake operant root with one profile in it, and point `HOME`
    /// at it. Returns the root and the profile directory.
    fn root_with_profile(name: &str) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("the temp home");
        let root = temp.path().join(".operant");
        let profile = root.join("profiles").join(name);
        fs::create_dir_all(&profile).expect("the profile directory");
        (temp, profile)
    }

    fn restore(key: &str, original: Option<String>) {
        match original {
            Some(val) => unsafe { env::set_var(key, val) },
            None => unsafe { env::remove_var(key) },
        }
    }

    /// The sticky `active_profile` file selects the profile's home.
    ///
    /// This is the behaviour `operant profile use <name>` promises: the
    /// switch it writes is READ by the next run. Before this, the file was
    /// written and never read, so the profile never took effect.
    #[test]
    #[serial_test::serial]
    fn the_sticky_active_profile_selects_the_home() {
        let _guard = test_env_lock_blocking();
        let home_key = "HOME";
        let hermes_key = "HERMES_HOME";
        let original_home = env::var(home_key).ok();
        let original_hermes = env::var(hermes_key).ok();

        let (temp, profile) = root_with_profile("work");
        // SAFETY: test-only env mutation under #[serial_test::serial]
        unsafe {
            env::set_var(home_key, temp.path());
            env::remove_var(hermes_key);
        }
        fs::write(
            temp.path().join(".operant").join("active_profile"),
            "work\n",
        )
        .expect("the sticky file");

        assert_eq!(
            operant_home(),
            profile,
            "the active profile's directory is the effective home"
        );
        assert_eq!(
            default_operant_root(),
            temp.path().join(".operant"),
            "the root that holds profiles/ does not follow the active profile"
        );

        restore(home_key, original_home);
        restore(hermes_key, original_hermes);
    }

    /// An explicit `HERMES_HOME` wins over the sticky profile: a deployment
    /// that sets it means it.
    #[test]
    #[serial_test::serial]
    fn an_explicit_hermes_home_wins_over_the_active_profile() {
        let _guard = test_env_lock_blocking();
        let home_key = "HOME";
        let hermes_key = "HERMES_HOME";
        let original_home = env::var(home_key).ok();
        let original_hermes = env::var(hermes_key).ok();

        let (temp, _profile) = root_with_profile("work");
        let explicit = temp.path().join("elsewhere");
        // SAFETY: test-only env mutation under #[serial_test::serial]
        unsafe {
            env::set_var(home_key, temp.path());
            env::set_var(hermes_key, &explicit);
        }
        fs::write(
            temp.path().join(".operant").join("active_profile"),
            "work\n",
        )
        .expect("the sticky file");

        assert_eq!(operant_home(), explicit);

        restore(home_key, original_home);
        restore(hermes_key, original_hermes);
    }

    /// A sticky file that names something that is not a profile is IGNORED,
    /// not obeyed: the file steers a path, so a tampered value must not be
    /// able to point the home anywhere.
    #[test]
    #[serial_test::serial]
    fn a_tampered_active_profile_is_ignored() {
        let _guard = test_env_lock_blocking();
        let home_key = "HOME";
        let hermes_key = "HERMES_HOME";
        let original_home = env::var(home_key).ok();
        let original_hermes = env::var(hermes_key).ok();

        let (temp, _profile) = root_with_profile("work");
        // SAFETY: test-only env mutation under #[serial_test::serial]
        unsafe {
            env::set_var(home_key, temp.path());
            env::remove_var(hermes_key);
        }
        fs::write(
            temp.path().join(".operant").join("active_profile"),
            "../../etc\n",
        )
        .expect("the sticky file");

        assert_eq!(
            operant_home(),
            temp.path().join(".operant"),
            "a name that is not a profile token must not steer the home"
        );

        restore(home_key, original_home);
        restore(hermes_key, original_hermes);
    }

    /// A sticky file that names a profile that does not exist is ignored —
    /// the default profile stays active rather than a missing directory
    /// becoming the home.
    #[test]
    #[serial_test::serial]
    fn an_active_profile_that_does_not_exist_is_ignored() {
        let _guard = test_env_lock_blocking();
        let home_key = "HOME";
        let hermes_key = "HERMES_HOME";
        let original_home = env::var(home_key).ok();
        let original_hermes = env::var(hermes_key).ok();

        let (temp, _profile) = root_with_profile("work");
        // SAFETY: test-only env mutation under #[serial_test::serial]
        unsafe {
            env::set_var(home_key, temp.path());
            env::remove_var(hermes_key);
        }
        fs::write(
            temp.path().join(".operant").join("active_profile"),
            "gone\n",
        )
        .expect("the sticky file");

        assert_eq!(operant_home(), temp.path().join(".operant"));

        restore(home_key, original_home);
        restore(hermes_key, original_hermes);
    }

    // -- File permissions ---------------------------------------------------

    #[test]
    fn test_set_secure_permissions_on_tempdir() {
        let dir = env::temp_dir().join("operant_perm_test");
        let _ = fs::create_dir_all(&dir);

        let result = set_secure_permissions(&dir);
        assert!(result.is_ok(), "set_secure_permissions failed: {result:?}");

        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn test_set_file_permissions_on_tempfile() {
        let path = env::temp_dir().join("operant_perm_test_file.txt");
        fs::write(&path, "test").expect("write test file");

        let result = set_file_permissions(&path, 0o644);
        assert!(result.is_ok(), "set_file_permissions failed: {result:?}");

        let _ = fs::remove_file(&path);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_set_file_permissions_unix_mode() {
        use std::os::unix::fs::PermissionsExt;

        let path = env::temp_dir().join("operant_unix_mode_test.txt");
        fs::write(&path, "test").expect("write test file");

        set_file_permissions(&path, 0o700).expect("set permissions");
        let meta = fs::metadata(&path).expect("metadata");
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);

        let _ = fs::remove_file(&path);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_set_file_permissions_windows_noop() {
        let path = env::temp_dir().join("operant_win_perm_test.txt");
        fs::write(&path, "test").expect("write test file");

        // Should succeed as a no-op.
        assert!(set_file_permissions(&path, 0o700).is_ok());

        let _ = fs::remove_file(&path);
    }

    // -- Interpreter detection ----------------------------------------------

    #[test]
    fn test_find_rustc_available() {
        // In any Rust development environment rustc should be present.
        // We don't hard-fail if it isn't – CI images may differ.
        if let Some(p) = find_rustc() {
            assert!(p.to_string_lossy().contains("rustc"));
        }
    }

    #[test]
    fn test_find_python_returns_valid_path_or_none() {
        if let Some(p) = find_python() {
            let name = p.to_string_lossy().to_lowercase();
            assert!(
                name.contains("python"),
                "expected 'python' in path, got: {name}"
            );
        }
    }

    #[test]
    fn test_find_node_returns_valid_path_or_none() {
        if let Some(p) = find_node() {
            let name = p.to_string_lossy().to_lowercase();
            assert!(
                name.contains("node"),
                "expected 'node' in path, got: {name}"
            );
        }
    }

    // -- Platform info ------------------------------------------------------

    #[test]
    fn test_os_name_not_empty() {
        assert!(!os_name().is_empty());
    }

    #[test]
    fn test_exactly_one_os_flag() {
        let flags = [is_windows(), is_macos(), is_linux()];
        let count = flags.iter().filter(|&&f| f).count();
        // On the three major platforms exactly one should be true.
        // On exotic targets none may be true, which is acceptable.
        assert!(count <= 1, "at most one OS flag should be true");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_is_windows() {
        assert!(is_windows());
        assert!(!is_macos());
        assert!(!is_linux());
        assert_eq!(os_name(), "windows");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_is_macos() {
        assert!(is_macos());
        assert!(!is_windows());
        assert!(!is_linux());
        assert_eq!(os_name(), "macos");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_is_linux() {
        assert!(is_linux());
        assert!(!is_windows());
        assert!(!is_macos());
        assert_eq!(os_name(), "linux");
    }

    #[test]
    fn test_platform_info_consistent() {
        let info = platform_info();
        assert_eq!(info.os, os_name());
        assert!(!info.arch.is_empty());
        assert_eq!(info.shell, detect_shell());
    }

    #[test]
    fn test_platform_info_arch_known() {
        let info = platform_info();
        let known = [
            "x86_64",
            "x86",
            "aarch64",
            "arm",
            "mips",
            "mips64",
            "powerpc",
            "powerpc64",
            "riscv64",
            "s390x",
        ];
        // Don't hard-fail on exotic arches, but log for awareness.
        if !known.contains(&info.arch) {
            eprintln!("note: unexpected arch value: {}", info.arch);
        }
    }
}
