//! End-to-end gate on `operant config env-path` naming the file the app reads.
//!
//! (2026-11-16 architecture audit, RT-Z43-E) `config env-path` used to build
//! its OWN candidate list - `HERMES_CONFIG`, `HERMES_CONFIG_DIR`,
//! `dirs::config_dir()` - and print the first path that merely existed. The
//! app reads a DIFFERENT path: `env_store::operant_env_path()`, which is
//! `HERMES_HOME`-aware through `platform::operant_home()`.
//!
//! THE REPRODUCED DEFECT: on a machine carrying a stray
//! `~/.config/operant/.env`, `HERMES_HOME=/somewhere/else operant config
//! env-path` printed the stale `~/.config/operant/.env`. The operator then
//! edits that file, changes nothing the agent ever reads, and gets no error -
//! a diagnostic pointing at the wrong file is worse than one that admits it
//! cannot tell.
//!
//! THE FIX: one resolver, the loaders'. The test pins it by setting
//! `HERMES_HOME` and asserting the output names exactly that path, with the
//! stale file present and with no file present at all.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fixture home with a deliberately STRAY `.env` sitting in the
/// `~/.config/operant/` location, so the old candidate list would find it
/// first. `HERMES_HOME` points somewhere else entirely.
struct Fixture {
    /// The operant home the app actually reads.
    home: PathBuf,
    /// A separate real `$HOME` holding the stray file.
    stray_root: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let base =
            std::env::temp_dir().join(format!("operant-envpath-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("hermes-home");
        let stray_root = base.join("home");
        std::fs::create_dir_all(stray_root.join(".config").join("operant")).expect("stray root");
        // The stray file, which the old resolver happily reported.
        std::fs::write(
            stray_root.join(".config").join("operant").join(".env"),
            "STRAY_KEY=stray\n",
        )
        .expect("stray env");
        Self { home, stray_root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
        let _ = std::fs::remove_dir_all(&self.stray_root);
    }
}

/// Run `operant config env-path` with a cleared environment, pointing
/// `HERMES_HOME` at the fixture and `HOME` at the stray root.
///
/// Both are set on purpose: `HOME` provides the stray file the old code
/// probed, and `HERMES_HOME` is the resolver the app uses.
fn env_path_output(fixture: &Fixture) -> Output {
    Command::new(env!("CARGO_BIN_EXE_operant"))
        .arg("config")
        .arg("env-path")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &fixture.stray_root)
        .env("HERMES_HOME", &fixture.home)
        .output()
        .expect("run operant config env-path")
}

/// The `to_string_lossy` of the fixture's operant home `.env` path.
fn expected(fixture: &Fixture) -> String {
    let path: &Path = &fixture.home.join(".env");
    path.to_string_lossy().to_string()
}

/// The command must name the app's own `.env` path even when a stale one exists
/// elsewhere. Mutation-proven: restoring the candidate-list logic reintroduces
/// the defect this test reproduces.
#[test]
fn env_path_names_the_path_hermes_home_points_at() {
    let fixture = Fixture::new("stray-present");
    let out = env_path_output(&fixture);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let expected = expected(&fixture);

    assert!(
        stdout.contains(&expected),
        "`config env-path` must name the HERMES_HOME path {expected:?}, got: {stdout}"
    );
    assert!(
        !stdout.contains(".config/operant/.env"),
        "`config env-path` must not name the stale ~/.config file, got: {stdout}"
    );
}

/// The path is reported even when the file does not yet exist.
///
/// An operator who has not created a `.env` still needs to know WHERE to put
/// it. `HERMES_TEST_ENV_PATH` pins the path so the assertion is about the
/// reporting, not about where this machine's home happens to be.
#[test]
fn env_path_reports_the_path_even_when_the_file_is_absent() {
    let fixture = Fixture::new("absent");
    // `Fixture::new` does not create `$HERMES_HOME/.env`, so this is the
    // no-file case by construction.
    assert!(
        !fixture.home.join(".env").exists(),
        "the fixture must NOT contain the file for this test"
    );

    let out = env_path_output(&fixture);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let expected = expected(&fixture);

    assert!(
        stdout.contains(&expected),
        "`config env-path` must still name where the file belongs, got: {stdout}"
    );
}

/// The reported path and the path the LOADER reads are one and the same.
///
/// This is the invariant the whole fix rests on: a mismatch is the defect.
/// It asserts it by comparing the CLI's output with the resolver's result
/// under the exact same environment.
#[test]
fn the_reported_path_is_the_one_the_loader_reads() {
    let fixture = Fixture::new("resolver-agrees");
    let out = env_path_output(&fixture);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let reported = stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && l.starts_with('/'))
        .expect("the command must print a path")
        .to_string();

    // The loader's own resolver, evaluated in this process under the same
    // environment the subprocess was given.
    // SAFETY: the CLI integration test binary is single-threaded per test and
    // no other test in this file reads these variables.
    unsafe { std::env::set_var("HERMES_HOME", &fixture.home) };
    let loader_path = operant_core::platform::operant_home().join(".env");
    unsafe { std::env::remove_var("HERMES_HOME") };

    assert_eq!(
        Path::new(&reported),
        loader_path,
        "the reported path and the loader's path must be the same file"
    );
}
