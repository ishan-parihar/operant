//! Integration tests for the `org` CLI surface added in iter-540.
//!
//! These assert the *operator contract* of the four new subcommand groups,
//! not core's store logic (core's inline tests cover that). The properties
//! worth locking are the ones a future refactor would silently break:
//!
//! 1. A blank `--reason` is refused at the boundary, in every mutating path.
//! 2. `org department findings` exits non-zero when a finding exists, so CI
//!    can gate on it. A gate that always exits 0 is worse than no gate.
//! 3. The DM budget is shared and visible: three turns succeed, the fourth
//!    is refused with a reason, and the thread stops being open.
//! 4. Decisions default to `binding = true` and carry dissent as data.
//! 5. An unrecognised authority scope is rejected rather than defaulted.
//!
//! These drive the real binary through `Command`, so they cover argument
//! parsing and the exit code as well as the handler body.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// Locate the `operant` binary the way cargo does for an integration test:
/// `target/<profile>/deps/<test>` -> `target/<profile>/operant`.
fn binary() -> PathBuf {
    let mut dir = std::env::current_exe().expect("current exe");
    dir.pop(); // deps/
    if dir.ends_with("deps") {
        dir.pop();
    }
    dir.join("operant")
}

/// A throwaway config pointed at a scratch database, so a test never reads
/// or writes the operator's real `~/.operant` state.
struct Fixture {
    _dir: tempfile::TempDir,
    config: PathBuf,
}

impl Fixture {
    /// `tag` only labels the temp dir for a human reading a failure, so it is
    /// folded into the dir name rather than stored.
    fn new(tag: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("org-cli-{tag}-"))
            .tempdir()
            .expect("tempdir");
        let db = dir.path().join("db.sqlite");
        let config = dir.path().join("operant.toml");
        std::fs::write(&config, format!("database_path = \"{}\"\n", db.display()))
            .expect("write config");
        Self { _dir: dir, config }
    }

    fn run(&self, args: &[&str]) -> Output {
        let bin = binary();
        assert!(
            bin.exists(),
            "operant binary not found at {} — run `cargo build -p operant-cli` first",
            bin.display()
        );
        Command::new(bin)
            .arg("-c")
            .arg(&self.config)
            .args(args)
            .output()
            .expect("spawn operant")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "`operant {}` failed: {}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr),
            text
        );
        text
    }
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn department_set_creates_a_readable_row() {
    let f = Fixture::new("dept");
    f.ok(&[
        "org",
        "department",
        "set",
        "infra",
        "--display-name",
        "Platform Infrastructure",
        "--mandate",
        "Keep the platform up",
        "--rules",
        "no writes outside your tree",
        "--protocols",
        "write a worklog before finishing",
        "--reason",
        "establish the infra department",
    ]);

    let listing = f.ok(&["org", "department", "list"]);
    assert!(listing.contains("infra"), "{listing}");
    assert!(listing.contains("Platform Infrastructure"), "{listing}");
    // An unstaffed head seat must render as visibly vacant, not be omitted.
    assert!(listing.contains("VACANT"), "{listing}");
}

#[test]
fn a_blank_reason_is_refused_by_every_mutating_path() {
    let f = Fixture::new("blank");
    for args in [
        vec!["org", "department", "set", "infra", "--reason", "   "],
        vec![
            "org",
            "decision",
            "propose",
            "Adopt CI",
            "--scope",
            "org",
            "--rationale",
            "better",
            "--reason",
            "",
        ],
        vec![
            "org",
            "grant",
            "give",
            "content.tooling",
            "--grantee",
            "emp-hod",
            "--scope",
            "department",
            "--reason",
            "\t",
        ],
    ] {
        let out = f.run(&args);
        assert!(
            !out.status.success(),
            "`operant {}` was ACCEPTED but a blank reason must be refused",
            args.join(" ")
        );
        let err = stderr_of(&out);
        assert!(
            err.contains("reason"),
            "error should name the reason rule, got: {err}"
        );
    }
}

#[test]
fn department_findings_exits_non_zero_so_ci_can_gate() {
    let f = Fixture::new("findings");
    f.ok(&[
        "org",
        "department",
        "set",
        "infra",
        "--display-name",
        "Infra",
        "--required-capabilities",
        "rust",
        "--reason",
        "establish the department",
    ]);

    // The head seat is vacant and "rust" is uncovered, so findings exist.
    let out = f.run(&["org", "department", "findings"]);
    assert!(
        !out.status.success(),
        "a gate that exits 0 with a vacant seat is worse than no gate; \
         stdout was:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("vacant_head"), "{text}");
    assert!(text.contains("unfilled_capability"), "{text}");
}

#[test]
fn the_dm_budget_is_shared_and_refuses_the_fourth_turn() {
    let f = Fixture::new("dm");
    f.ok(&["org", "dm", "open", "emp-a", "emp-b"]);
    let listing = f.ok(&["org", "dm", "list", "--employee-id", "emp-a"]);
    let thread_id = listing
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or_default()
        .to_string();
    assert!(thread_id.starts_with("dm_"), "got thread id {thread_id:?}");

    // Three turns are allowed and the counter is visible each time.
    for expected in 1..=3 {
        let out = f.ok(&["org", "dm", "spend", &thread_id]);
        assert!(
            out.contains(&format!("{expected}/3")),
            "turn {expected} should report {expected}/3, got: {out}"
        );
    }

    // The fourth is refused with a visible reason, not silently dropped.
    let out = f.run(&["org", "dm", "spend", &thread_id]);
    assert!(
        !out.status.success(),
        "a turn past the budget must be refused; stdout was:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let err = stderr_of(&out);
    assert!(
        err.contains("exhausted"),
        "error should say exhausted: {err}"
    );

    // And the thread is no longer open, so the conversation ended visibly.
    let after = f.ok(&["org", "dm", "list", "--employee-id", "emp-a"]);
    assert!(
        after.contains("no open threads"),
        "an exhausted thread must stop being open, got: {after}"
    );
}

#[test]
fn decisions_default_to_binding_and_carry_dissent_as_data() {
    let f = Fixture::new("decision");
    f.ok(&[
        "org",
        "decision",
        "propose",
        "Adopt dual-crate CI",
        "--scope",
        "org",
        "--decided-by",
        "emp-ceo",
        "--rationale",
        "Halves the failure blast radius",
        "--reason",
        "quality initiative",
    ]);

    let json = f.ok(&["org", "decision", "list", "--json"]);
    assert!(json.contains("\"binding\": true"), "{json}");

    // Dissent is a first-class column on the decision, not a veto.
    let id = json
        .lines()
        .find_map(|l| l.trim().strip_prefix("\"decision_id\": \""))
        .map(|v| v.trim_end_matches("\",").to_string())
        .expect("decision_id in json");
    f.ok(&[
        "org",
        "decision",
        "dissent",
        &id,
        "--employee-id",
        "emp-hod",
        "--position",
        "oppose",
        "--reason",
        "doubles CI minutes",
    ]);

    let after = f.ok(&["org", "decision", "list", "--json"]);
    assert!(after.contains("\"position\": \"oppose\""), "{after}");
    assert!(after.contains("doubles CI minutes"), "{after}");
}

#[test]
fn an_unknown_authority_scope_is_rejected_rather_than_defaulted() {
    let f = Fixture::new("scope");
    let out = f.run(&[
        "org",
        "grant",
        "give",
        "content.tooling",
        "--grantee",
        "emp-hod",
        "--scope",
        "bogus",
        "--reason",
        "testing scope validation",
    ]);
    assert!(!out.status.success(), "an unknown scope must be refused");
    let err = stderr_of(&out);
    assert!(err.contains("authority scope"), "{err}");
}

#[test]
fn decisions_live_in_their_own_file_not_the_kanban_family() {
    // This is the iter-539 invariant seen from the CLI: the decisions store
    // must not be able to claim a PRAGMA user_version on the kanban file, so
    // it is a sibling rather than a table in the shared one.
    let f = Fixture::new("files");
    f.ok(&[
        "org",
        "department",
        "set",
        "infra",
        "--reason",
        "establish the department",
    ]);
    f.ok(&[
        "org",
        "decision",
        "propose",
        "A decision",
        "--scope",
        "org",
        "--rationale",
        "because",
        "--reason",
        "because",
    ]);

    let dir = f.config.parent().expect("config dir");
    assert!(
        dir.join("operant_kanban.db").exists(),
        "departments should live in the kanban sibling"
    );
    assert!(
        dir.join("operant_decisions.db").exists(),
        "decisions must live in their own file"
    );
}
