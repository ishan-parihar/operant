//! End-to-end gate on `operant doctor`'s exit code.
//!
//! Unit tests on the credential scanners cannot catch a caller that stops
//! calling them: deleting the ambient cross-check from `run_config_checks` left
//! all 15 of them green, because they only exercise the helpers. These tests
//! drive the real binary, so the only way to pass is for the whole path — .env
//! read, ambient scan, glyph, vector, `doctor_exit_code` — to agree.
//!
//! The environment variable that redirects the config directory is `HERMES_HOME`
//! (`operant_home()` in operant-core/src/platform.rs reads only that), not
//! `OPERANT_CONFIG_DIR`. Setting the wrong one silently exercises the developer's
//! real ~/.operant instead of the fixture, which is how this whole class of
//! defect stayed invisible for so long.

// Integration test binaries are not covered by the `#![cfg_attr(test, ...)]`
// exemption in main.rs, so the gate's -D flags reach here. `expect` is used only
// to surface a failure loudly, which is what a test should do.
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fixture home directory. Each test gets its own so they cannot interfere.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("operant-doctor-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        Self { dir }
    }

    fn write_env(&self, contents: &str) -> &Self {
        std::fs::write(self.dir.join(".env"), contents).expect("write .env");
        self
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run the real binary against `home` with a deliberately empty environment
/// apart from what the test names, and return its exit code.
fn doctor_exit(home: &Path) -> i32 {
    let out = doctor_cmd(home).output().expect("run operant doctor");
    // `code()` is `Option<i32>`, and -1 stands in for "killed by a signal", which
    // for this purpose is a non-zero outcome like any other.
    out.status.code().unwrap_or(-1)
}

/// Same invocation, returning stdout+stderr for assertions on what was said.
fn doctor_output(home: &Path) -> String {
    let out = doctor_cmd(home).output().expect("run operant doctor");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn doctor_cmd(home: &Path) -> Command {
    let bin = env!("CARGO_BIN_EXE_operant");
    // Start from a clean env: any real credential on the developer's machine
    // must not be able to satisfy the check under test.
    let mut cmd = Command::new(bin);
    cmd.arg("doctor")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("HERMES_HOME", home);
    // Belt and braces: strip the variables the ambient scan looks at, in case
    // `env_clear` is ever not enough.
    for key in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_BASE_URL",
        "GITHUB_TOKEN",
        "IGS_HOST",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

/// A machine with no `.env` and no credential anywhere cannot call a model.
/// `doctor` must say so with a non-zero exit code.
///
/// This is the case that regressed: the build that shipped as `fb3573eb`
/// reported exit 0 here, so a fresh install with nothing configured looked
/// healthy to any monitoring that trusts the exit code.
#[test]
fn no_config_at_all_fails() {
    let f = Fixture::new("noconfig");
    assert_ne!(
        doctor_exit(f.path()),
        0,
        "a machine with no .env and no credential must not report healthy"
    );
}

/// A `.env` that exists but holds no credential is the same broken machine in
/// a subtler costume — the most common shape of a half-finished install, since
/// `operant setup` creates the file before it writes a key into it.
#[test]
fn env_file_without_a_credential_fails() {
    let f = Fixture::new("nokey");
    f.write_env("# comment\nFOO=bar\n");
    assert_ne!(
        doctor_exit(f.path()),
        0,
        "a .env with no credential must not report healthy"
    );
}

/// A `.env` carrying a credential must not be failed by the *credential
/// presence* checks. Without this the regression above would also be satisfied
/// by simply always returning non-zero.
///
/// The endpoint is an unroutable local port and the key is a placeholder, so the
/// live provider probe in `checks_api.rs` fails fast and offline — a connection
/// refusal rather than a real 401 round-trip. The assertion is deliberately about
/// the presence check only: that check must not be the thing failing the run.
#[test]
fn credential_presence_check_no_longer_fails_a_configured_machine() {
    let f = Fixture::new("configured");
    f.write_env(
        "OPENAI_API_KEY=sk-test-not-a-real-key\n\
         OPENAI_BASE_URL=http://127.0.0.1:9/v1\n",
    );
    let out = doctor_output(f.path());
    assert!(
        !out.contains("No provider API keys configured"),
        "a .env holding a credential must not be reported as having none.\n{out}"
    );
}
