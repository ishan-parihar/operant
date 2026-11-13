//! End-to-end gate on `operant continue` — the command the TUI's exit leftover
//! names, and the command that has to work when the operator runs it.
//!
//! WHY THIS IS AN INTEGRATION TEST AND NOT MORE UNIT TESTS
//!
//! `cmd_continue.rs`'s unit tests cover the resolution rules in isolation. The
//! thing they cannot catch is the COMMAND NOT EXISTING: a handler that is
//! never reachable from clap passes every unit test. The half of the contract
//! lives in the derive surface (main.rs `Commands::Continue`) and the other
//! half in the dispatch arm — neither is visible to a unit test in the module.
//! This drives the real binary, so the only way to pass is for the whole path
//! to agree.
//!
//! The ZERO-ARG path is covered too, because it is the one that stakes out the
//! intent table's conflict row: `operant continue` with no id reads the
//! interrupted-session marker WITHOUT consuming it, so the TUI boot notice and
//! the gateway boot notice still fire afterwards. A regression to
//! `consume_marker` would make every second run of this test see a different
//! session.
//!
//! Every fixture gets a private `HERMES_HOME` (set on the CHILD process only —
//! mutating it in the test process races every other test that resolves a path
//! under the operant home, cf. the dev-dependency note in Cargo.toml) plus a
//! private `OPERANT_CONFIG_DIR`, for the same reason `doctor_exit_code.rs` sets
//! both: the two subsystems resolve their paths differently. `env_clear` plus
//! explicit removals is deliberate — an ambient `OPENAI_API_KEY` must not
//! change any outcome here.
//!
//! No fixture ever reaches an agent or a model: with no TTY on stdout the
//! command cannot start the TUI, so it prints the leftover and stops. That is
//! what makes this safe to run in CI.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A throwaway home for one test. Removed on drop so a failing assertion does
/// not leave a directory behind for the next run to trip over.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "operant-continue-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    /// Write the interrupted-session marker the way the TUI's teardown writes
    /// it: `path_in(<HERMES_HOME>/sessions)`. Written directly rather than
    /// through `session_interrupted::write_marker` so the test does not depend
    /// on the writer at all — this is the READ path's contract.
    fn write_marker(&self, session_id: &str, title: &str) -> &Self {
        let dir = self.dir.join("sessions");
        std::fs::create_dir_all(&dir).expect("create sessions dir");
        std::fs::write(
            dir.join("interrupted-session.json"),
            format!(
                "{{\"version\":1,\"session_id\":\"{session_id}\",\"written_at\":\
                 \"2026-11-02T12:00:00Z\",\"title\":\"{title}\"}}"
            ),
        )
        .expect("write marker");
        self
    }

    fn marker_exists(&self) -> bool {
        self.dir
            .join("sessions")
            .join("interrupted-session.json")
            .exists()
    }

    /// Seed the fixture's database the way the live agent does — through the
    /// real `Database`, not raw SQL, so a schema change in the product moves
    /// this test with it instead of breaking it in a new way.
    fn seed_sessions(&self, older: (&str, &str), newer: (&str, &str)) -> &Self {
        let db = operant_core::database::Database::init(self.dir.join("database.db"))
            .expect("init fixture database");
        db.save_session(older.0, Some(older.1), "test", "2026-11-01", "2026-11-01")
            .expect("seed older session");
        db.save_session(newer.0, Some(newer.1), "test", "2026-11-02", "2026-11-02")
            .expect("seed newer session");
        // ONE message each, with the newer one later: the no-id fallback ranks by
        // the newest MESSAGE, because `list_sessions` ranks a session that never
        // ENDED by when it STARTED — which is what made `continue` with no id
        // answer a session the operator had already finished. A session with no
        // turns has nothing to rehydrate and is not a candidate.
        db.save_message(older.0, "user", "the first prompt", "2026-11-01T09:00:00Z")
            .expect("seed older message");
        db.save_message(newer.0, "user", "the newer prompt", "2026-11-02T09:00:00Z")
            .expect("seed newer message");
        self
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run the real binary with nothing in the environment but what the test names.
fn continue_cmd(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_operant"));
    cmd.arg("continue")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("HERMES_HOME", home)
        // The config file resolves by OPERANT_CONFIG_DIR, not HERMES_HOME.
        .env("OPERANT_CONFIG_DIR", home);
    for key in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_BASE_URL",
        "GITHUB_TOKEN",
        "SOURCEHOUND_HOST",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

fn run(home: &Path) -> Output {
    continue_cmd(home).output().expect("run operant continue")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The subcommand must exist and advertise the session id, or the leftover it
/// prints cannot be acted on.
#[test]
fn the_continue_subcommand_takes_a_session_id() {
    let fixt = Fixture::new("help");
    let out = Command::new(env!("CARGO_BIN_EXE_operant"))
        .arg("continue")
        .arg("--help")
        .output()
        .expect("run operant continue --help");
    let text = stdout(&out);
    assert!(out.status.success(), "`--help` must exit 0: {text}");
    assert!(
        text.contains("Session ID to continue"),
        "the id argument must be documented: {text}"
    );
    assert!(
        text.contains("Usage: operant continue [OPTIONS] [ID]"),
        "the usage line must show the id: {text}"
    );
    drop(fixt);
}

/// An id the operator typed is authoritative: a typo must be an error, not a
/// silent resume of some OTHER session.
#[test]
fn an_unknown_session_id_is_an_error_not_a_guess() {
    let fixt = Fixture::new("bogus-id");
    let out = continue_cmd(fixt.path())
        .arg("sess_typo")
        .output()
        .expect("run operant continue sess_typo");
    let text = format!("{}{}", stdout(&out), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success(), "a typo must fail: {text}");
    assert!(
        text.contains("Session not found: sess_typo"),
        "the error must name the id: {text}"
    );
}

/// With no sessions at all the command must say how to start, not panic or
/// resume nothing.
#[test]
fn an_empty_store_says_how_to_start_one() {
    let fixt = Fixture::new("empty-store");
    let out = run(fixt.path());
    let text = format!("{}{}", stdout(&out), String::from_utf8_lossy(&out.stderr));
    // Nothing to continue IS a failure: the command could not do what it was
    // asked, and exiting 0 would read as "the session is ready".
    assert!(
        !out.status.success(),
        "an empty store is not a success: {text}"
    );
    assert!(
        text.contains("operant chat"),
        "the message must point at the way out: {text}"
    );
}

/// With neither an id nor a marker the most recently updated session is the
/// answer, and the leftover names it verbatim — copy-paste ready.
#[test]
fn the_most_recent_session_is_the_default() {
    let fixt = Fixture::new("most-recent");
    fixt.seed_sessions(
        ("sess_older", "Investigate the importer"),
        ("sess_newer", "Fix the export dialog"),
    );
    let out = run(fixt.path());
    let text = stdout(&out);
    assert!(out.status.success(), "the fallback path must work: {text}");
    assert!(
        text.contains("operant continue sess_newer"),
        "the leftover must name the newest session: {text}"
    );
    assert!(
        text.contains("Fix the export dialog"),
        "the title is what the operator recognises: {text}"
    );
}

/// A CRASHED session is the one `operant continue` answers with, even though a
/// session the operator finished later STARTED after it.
///
/// `list_sessions` ranks by `COALESCE(ended_at, started_at)`, so a session that
/// never ENDED (a crash, a SIGKILL, a closed terminal) is buried under one that
/// started after it and finished normally. The operator's question is "what was
/// I working on", which is the newest TURN, not the newest start.
#[test]
fn a_crashed_session_beats_a_later_finished_one() {
    let fixt = Fixture::new("crash-default");
    let db = operant_core::database::Database::init(fixt.path().join("database.db"))
        .expect("init fixture database");
    // Started first and never ended (ended_at NULL) — the crash shape.
    db.save_session(
        "sess_crashed",
        Some("The session I was working in"),
        "test",
        "2026-11-10",
        "2026-11-10",
    )
    .expect("seed the crashed session");
    db.save_message(
        "sess_crashed",
        "user",
        "the last thing I asked",
        "2026-11-13T18:00:00Z",
    )
    .expect("seed its newest turn");
    // Started LATER and finished normally, but its last turn is OLDER.
    db.save_session(
        "sess_finished",
        Some("An older session I already finished"),
        "test",
        "2026-11-12",
        "2026-11-12",
    )
    .expect("seed the finished session");
    db.save_message(
        "sess_finished",
        "user",
        "an older turn",
        "2026-11-12T10:00:00Z",
    )
    .expect("seed its older turn");
    drop(db);

    let out = run(fixt.path());
    let text = stdout(&out);
    assert!(out.status.success(), "the fallback path must work: {text}");
    assert!(
        text.contains("operant continue sess_crashed"),
        "the crashed session is the one the operator was working in: {text}"
    );
    assert!(
        text.contains("The session I was working in"),
        "the label is the title, not the id: {text}"
    );
}

/// The interrupted-session marker is the better default than recency: it
/// answers "what was I doing", which recency only approximates.
#[test]
fn the_interrupted_marker_beats_recency() {
    let fixt = Fixture::new("marker-default");
    fixt.seed_sessions(
        ("sess_marked", "The marked one"),
        ("sess_newer", "Newer but not what I was doing"),
    )
    .write_marker("sess_marked", "Interrupted work");
    let out = run(fixt.path());
    let text = stdout(&out);
    assert!(out.status.success(), "the marker path must work: {text}");
    assert!(
        text.contains("operant continue sess_marked"),
        "the marker's session must win: {text}"
    );
    assert!(
        text.contains("Interrupted work"),
        "the marker's title rides along: {text}"
    );
}

/// The marker is READ, not consumed: the TUI boot notice and the gateway boot
/// notice both consume it, and a `consume_marker` here would steal their one
/// offer. Twice-run proves one generation survived.
#[test]
fn the_marker_is_read_not_consumed() {
    let fixt = Fixture::new("marker-read-only");
    fixt.seed_sessions(("sess_marked", "The marked one"), ("sess_newer", "Other"))
        .write_marker("sess_marked", "Interrupted work");
    let first = stdout(&run(fixt.path()));
    let second = stdout(&run(fixt.path()));
    // The two runs must ANSWER the same question. The whole stdout is not
    // compared: the first run on a fresh home also seeds the bundled skills,
    // and that line is not part of the answer.
    for text in [&first, &second] {
        assert!(
            text.contains("operant continue sess_marked"),
            "every run must answer with the marked session: {text}"
        );
    }
    assert!(
        fixt.marker_exists(),
        "the marker must outlive the runs that read it"
    );
}

/// An explicit id beats both the marker and recency.
#[test]
fn an_explicit_id_beats_the_marker() {
    let fixt = Fixture::new("explicit-wins");
    fixt.seed_sessions(
        ("sess_older", "Investigate the importer"),
        ("sess_newer", "Fix the export dialog"),
    )
    .write_marker("sess_older", "Interrupted work");
    let out = continue_cmd(fixt.path())
        .arg("sess_newer")
        .output()
        .expect("run operant continue sess_newer");
    let text = stdout(&out);
    assert!(out.status.success(), "the explicit path must work: {text}");
    assert!(
        text.contains("operant continue sess_newer"),
        "the operator's own id must win: {text}"
    );
}
