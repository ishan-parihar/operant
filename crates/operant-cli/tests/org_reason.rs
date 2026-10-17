//! The `--reason` mandate on `kanban block` (WAVE1-DECISIONS §3.5).
//!
//! The `org employee/notice/worklog/sync/import` surface this file used to
//! drive was deleted in iter-838 (owner order 2026-10-15); only the kanban
//! half of the mandate survives.
//!
//! These tests drive the real `operant` binary. That is the only way to prove
//! the mandate actually holds, because the mandate has two halves and only
//! one of them lives in a function:
//!
//! 1. **clap rejects a missing `--reason`** — a property of the derive
//!    surface, invisible from a unit test on a handler.
//! 2. **a present-but-blank `--reason` is rejected** — a property of
//!    `require_reason`, which runs after parsing.
//!
//! A unit test can only cover (2). A CLI test that omits (1) would pass
//! against a surface where every reason defaulted to `""`, which is the
//! pre-§3.5 behavior of `kanban block` and the exact failure the mandate
//! exists to remove.
//!
//! Every test gets a private `HERMES_HOME` so it cannot read the
//! developer's real `~/.operant`, and a private `OPERANT_CONFIG_DIR` for
//! the same reason `doctor_exit_code.rs` sets both: the two subsystems
//! resolve their paths differently. `env_clear` plus explicit removals is
//! deliberate — an ambient `OPENAI_API_KEY` must not change any outcome here.
//!
//! Integration test binaries are not covered by the `#![cfg_attr(test, ...)]`
//! exemption in main.rs, so the clippy gate's -D flags reach here. `expect`
//! is used only to surface a failure loudly, which is what a test should do.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A throwaway home for one test. Removed on drop so a failing assertion
/// does not leave a directory behind for the next run to trip over.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("operant-org-reason-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        Self { dir }
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

fn operant(fixture: &Fixture) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_operant"));
    cmd.env_clear();
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    cmd.env("HOME", fixture.path());
    cmd.env("HERMES_HOME", fixture.path());
    cmd.env("OPERANT_CONFIG_DIR", fixture.path());
    cmd.current_dir(fixture.path());
    cmd
}

fn run(fixture: &Fixture, args: &[&str]) -> Output {
    operant(fixture).args(args).output().expect("run operant")
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Exit code, with -1 standing in for "killed by a signal" — a non-zero
/// outcome like any other.
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// `kanban block` is the converging half of the mandate: it used to accept
/// an omitted reason and write the literal `"Blocked via CLI"` into the
/// task event. That is AD-032's failure mode already present in operant.
#[test]
fn org_reason_kanban_block_now_requires_a_reason() {
    let fx = Fixture::new("kanban");

    let out = run(&fx, &["kanban", "block", "task-1"]);
    let text = combined(&out);
    assert_eq!(
        code(&out),
        2,
        "kanban block without --reason must be a clap usage error.\nGot {code} :\n{text}",
        code = code(&out)
    );
    assert!(
        text.contains("--reason"),
        "kanban block must name the missing flag.\nGot:\n{text}"
    );
    assert!(
        !text.contains("Blocked via CLI"),
        "the 'Blocked via CLI' default must be gone from the CLI path.\nGot:\n{text}"
    );
}
