//! The `--reason` mandate, end to end (Wave 1, WAVE1-DECISIONS §3.5).
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

/// Every mutating subcommand, as the argv prefix that reaches it minus the
/// `--reason` flag under test.
///
/// The point of the table is coverage: a subcommand added to `cmd_org.rs`
/// without a reason field has to be added here too, because the test that
/// names it is the only thing that fails when someone drops the field.
const MUTATING_SUBCOMMANDS: &[&[&str]] = &[
    &["org", "employee", "create", "emp-test01", "Test Employee"],
    &[
        "org",
        "employee",
        "update",
        "emp-test01",
        "--name",
        "Renamed",
    ],
    &["org", "employee", "retire", "emp-test01"],
    &["org", "sync"],
    &["org", "notice", "post", "a notice body"],
    &["org", "notice", "ack", "n_1"],
    &["org", "notice", "pin", "n_1"],
    &["org", "notice", "unpin", "n_1"],
    &["org", "worklog", "append", "an entry"],
    &["org", "import", "/nonexistent/organism-tree"],
];

/// Commands that validate a `--reason` and then deliberately write nothing,
/// failing loudly instead. Each one must still honour the reason contract —
/// a missing or blank reason is rejected before the handler runs — because a
/// command that will never write still must not be a hole in the mandate.
///
/// `org sync` is deliberately NOT here: it is wired to the real
/// `EmployeeDb::backfill_from_cron_jobs` (iter-517) and does write rows, so
/// its contract is pinned by `org_reason_wired_sync_still_demands_a_reason`.
const UNWIRED_SUBCOMMANDS: &[&[&str]] = &[&["org", "import", "/nonexistent/organism-tree"]];

/// The read-only subcommands. These must **not** gain a `--reason` — a
/// read that demands a write justification is a §3.5 violation too, and a
/// growing one.
const READ_ONLY_SUBCOMMANDS: &[&[&str]] = &[
    &["org", "check"],
    &["org", "list"],
    &["org", "notice", "list"],
    &["org", "worklog", "list"],
];

/// §3.5, table row 1: a missing `--reason` is rejected by clap.
///
/// clap's exit code for a usage error is 2. Asserting on 2 rather than
/// merely "non-zero" matters: a non-zero code would also be satisfied by a
/// handler that panicked, which would mean clap did *not* catch it and the
/// mandate is being enforced somewhere it can be bypassed.
#[test]
fn org_reason_missing_flag_is_rejected_by_clap() {
    for args in MUTATING_SUBCOMMANDS {
        let fx = Fixture::new("missing");
        let out = run(&fx, args);
        let text = combined(&out);
        assert_eq!(
            code(&out),
            2,
            "`operant {}` must exit 2 (clap usage error) without --reason.\n\
             Got {} with output:\n{text}",
            args.join(" "),
            code(&out)
        );
        assert!(
            text.contains("--reason"),
            "`operant {}` must name the missing flag.\nGot:\n{text}",
            args.join(" ")
        );
    }
}

/// §3.5's second half: present-but-blank is not a reason. clap accepts an
/// empty string happily, so this is the gate that has to exist.
#[test]
fn org_reason_blank_is_rejected() {
    for args in MUTATING_SUBCOMMANDS {
        for blank in ["", " ", "\t", "   \t\n  "] {
            let fx = Fixture::new("blank");
            let mut argv: Vec<&str> = args.to_vec();
            argv.push("--reason");
            argv.push(blank);
            let out = run(&fx, &argv);
            let text = combined(&out);
            assert_ne!(
                code(&out),
                0,
                "`operant {} --reason {blank:?}` must not succeed — a blank \
                 reason satisfies NOT NULL while recording nothing.\nGot:\n{text}",
                args.join(" ")
            );
            assert!(
                text.contains("--reason"),
                "`operant {} --reason {blank:?}` must explain the rejection.\nGot:\n{text}",
                args.join(" ")
            );
        }
    }
}

/// The happy path: a real reason is accepted and the write lands with the
/// reason on the row, so the audit record is not a parsed-and-discarded flag.
///
/// `org sync` and `org import` are excluded from the write assertion: both
/// are declared but unwired (see `cmd_sync`/`cmd_import`), so they accept a
/// reason and then fail loudly without writing. They are covered separately
/// by `org_reason_unwired_commands_still_validate`.
#[test]
fn org_reason_happy_path_persists_the_reason() {
    let fx = Fixture::new("happy");

    // employee create
    let out = run(
        &fx,
        &[
            "org",
            "employee",
            "create",
            "emp-a02e3f692fb0",
            "Nightly Curator",
            "--skills",
            "research,curation",
            "--reason",
            "wave 1 registry seed",
        ],
    );
    assert_eq!(code(&out), 0, "create failed:\n{}", combined(&out));

    // notice post
    let out = run(
        &fx,
        &[
            "org",
            "notice",
            "post",
            "regret cache is full",
            "--correlation-id",
            "corr-1",
            "--reason",
            "operator must prune before 03:00 run",
        ],
    );
    assert_eq!(code(&out), 0, "post failed:\n{}", combined(&out));

    // worklog append
    let out = run(
        &fx,
        &[
            "org",
            "worklog",
            "append",
            "pruned 12 regrets",
            "--reason",
            "reclaim disk before nightly",
        ],
    );
    assert_eq!(code(&out), 0, "append failed:\n{}", combined(&out));

    // The reasons must be readable back through the read-only surface. If
    // any of these print the reason as empty, the flag was parsed and
    // discarded — the exact failure this packet exists to prevent.
    let out = run(&fx, &["org", "list"]);
    assert_eq!(code(&out), 0);
    assert!(
        combined(&out).contains("wave 1 registry seed"),
        "employees.reason must carry the reason.\nGot:\n{}",
        combined(&out)
    );

    let out = run(&fx, &["org", "notice", "list"]);
    assert_eq!(code(&out), 0);
    assert!(
        combined(&out).contains("operator must prune before 03:00 run"),
        "notices.reason must carry the reason.\nGot:\n{}",
        combined(&out)
    );

    let out = run(&fx, &["org", "worklog", "list"]);
    assert_eq!(code(&out), 0);
    // The worklog has NO `reason` column: §3.4's 20 fields (the organism's
    // `WorklogEntry`) contain no such field, and inventing a 21st would have
    // broken the "mirrors the organism exactly" contract. The reason
    // therefore rides in the row's `artifacts` JSON, which is where
    // `operator_entry` puts it. Asserting the *text* is present anywhere in
    // the listing is the honest form of "the reason was persisted with the
    // effect" — the point of the test.
    assert!(
        combined(&out).contains("pruned 12 regrets"),
        "worklog must carry what_done.\nGot:\n{}",
        combined(&out)
    );
    assert!(
        combined(&out).contains("reclaim disk before nightly"),
        "the --reason must be persisted in the worklog row's artifacts.\nGot:\n{}",
        combined(&out)
    );
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

/// The two unwired commands must still hold the clap half of the contract
/// and must fail loudly rather than pretending to have written rows.
#[test]
fn org_reason_unwired_commands_still_validate() {
    for args in UNWIRED_SUBCOMMANDS {
        let fx = Fixture::new("unwired");
        // No reason: clap rejects.
        assert_eq!(
            code(&run(&fx, args)),
            2,
            "`{}` must demand a reason",
            args.join(" ")
        );
        // Blank reason: require_reason rejects.
        let mut argv: Vec<&str> = args.to_vec();
        argv.extend_from_slice(&["--reason", "   "]);
        assert_ne!(
            code(&run(&fx, &argv)),
            0,
            "`{}` must reject a blank reason",
            args.join(" ")
        );
        // Real reason: reaches the handler, which refuses loudly and writes
        // nothing. A silent success here would be the worst outcome.
        let mut argv: Vec<&str> = args.to_vec();
        argv.extend_from_slice(&["--reason", "wave 1 bring-up"]);
        let out = run(&fx, &argv);
        let text = combined(&out);
        assert_ne!(
            code(&out),
            0,
            "`{}` is unwired and must not claim success:\n{text}",
            args.join(" ")
        );
        assert!(
            text.contains("not wired yet"),
            "`{}` must say plainly that it is unwired.\nGot:\n{text}",
            args.join(" ")
        );
    }
}

/// `org sync` is wired, so the reason contract for it is the real one: a
/// missing or blank reason is still rejected *before* any row is written.
///
/// This is the assertion that matters most for the whole `--reason` mandate:
/// `org sync` is the single most consequential mutation in the org surface
/// (it writes a hundred-plus rows at once), and it previously validated the
/// reason and then wrote nothing at all.
#[test]
fn org_reason_wired_sync_still_demands_a_reason() {
    let fx = Fixture::new("sync");
    assert_eq!(
        code(&run(&fx, &["org", "sync"])),
        2,
        "`org sync` must demand a reason"
    );
    let mut argv: Vec<&str> = vec!["org", "sync"];
    argv.extend_from_slice(&["--reason", "   "]);
    assert_ne!(
        code(&run(&fx, &argv)),
        0,
        "`org sync` must reject a blank reason"
    );
}

/// Read-only subcommands must not acquire a `--reason`. §3.5's table marks
/// these "none", and a read that demands a justification is how a mandate
/// stops being a mandate and starts being ceremony.
#[test]
fn org_reason_read_only_subcommands_need_none() {
    for args in READ_ONLY_SUBCOMMANDS {
        let fx = Fixture::new("readonly");
        let out = run(&fx, args);
        let text = combined(&out);
        // They must run. Exit 0 or a clean domain error (an empty board
        // still reports zero counts) — what they must never do is exit 2,
        // which is clap refusing a missing required argument.
        assert_ne!(
            code(&out),
            2,
            "`operant {}` is read-only and must not require --reason.\nGot:\n{text}",
            args.join(" ")
        );
    }
}

/// The CLI's `org_db_path` duplicates core's derivation while
/// `operant_core::org` is not exported. This pins the two against each
/// other so the copy cannot drift: a different sibling DB file would
/// silently split the registry in two, which is the "no second store" rule
/// (§1.2 Q2) this derivation exists to uphold.
#[test]
fn org_reason_db_path_matches_core_derivation() {
    let cli = std::fs::read_to_string(concat!(std::env!("CARGO_MANIFEST_DIR"), "/src/cmd_org.rs"))
        .expect("read cmd_org.rs");
    let core_path = concat!(
        std::env!("CARGO_MANIFEST_DIR"),
        "/../operant-core/src/org/employee_db.rs"
    );
    let Ok(core) = std::fs::read_to_string(core_path) else {
        // Packet A's module is in flight (this tree is being written
        // concurrently). While it is absent there is nothing to pin the
        // CLI copy against, so the test has no claim to make. It must not
        // pass silently, though — that would let the two derivations drift
        // unnoticed once the module lands.
        eprintln!(
            "SKIPPED: {core_path} not readable — packet A's org module is not \
             on disk yet, so the CLI's org_db_path copy has nothing to be \
             pinned against. Re-run this test once it lands."
        );
        return;
    };
    assert!(
        !core.is_empty(),
        "{core_path} is readable but empty — a truncated read would make this \
         pin vacuous"
    );

    // The pin is now stronger than "the two agree": there must be only ONE
    // derivation. `cmd_org.rs` previously carried a local copy of
    // `org_db_path` pinned against core's, which is exactly the shape that
    // let its `ORG_SCHEMA` drift (the `worklog` table diverged to a
    // 14-column shape while core's had 20). Asserting the copy is *absent*
    // removes the drift opportunity instead of detecting it after the fact.
    assert!(
        !cli.contains("fn org_db_path"),
        "cmd_org.rs must NOT define its own org_db_path — it imports \
         operant_core::org::employee_db::org_db_path, so there is exactly one \
         derivation in the tree"
    );
    assert!(
        cli.contains("use operant_core::org::employee_db::org_db_path;"),
        "cmd_org.rs must import org_db_path from operant-core"
    );

    for needle in [".join(\"operant_kanban.db\")", ".parent()"] {
        assert!(
            core.contains(needle),
            "operant-core::org::org_db_path must contain {needle}"
        );
    }
}

/// `org sync` must persist the operator's reason, not a constant.
///
/// Regression: `org sync` required `--reason`, validated it, and echoed it in
/// its summary line, while `EmployeeDb::employee_from_cron_job` stamped a
/// hardcoded `BACKFILL_REASON` into every row. The write was audited in the
/// terminal but not in the registry, which is the only place an auditor reads
/// months later. Proving it end-to-end needs a cron job to exist, so this
/// drives the cron path first and then checks the employee rows it produced.
#[test]
fn org_reason_sync_persists_the_operator_reason_in_the_registry() {
    let fx = Fixture::new("sync-reason");
    let reason = "reconcile registry after the q3 headcount freeze (ORG-419)";

    // Seed one cron job, so `org sync` has something to backfill.
    let seeded = run(
        &fx,
        &["cron", "create", "backfill-probe", "every 15m", "probe"],
    );
    assert_eq!(
        code(&seeded),
        0,
        "seeding a cron job must work, got: {}",
        combined(&seeded)
    );

    let out = run(&fx, &["org", "sync", "--reason", reason]);
    assert_eq!(
        code(&out),
        0,
        "org sync should succeed, got: {}",
        combined(&out)
    );
    assert!(
        combined(&out).contains(reason),
        "the summary must echo the reason, got: {}",
        combined(&out)
    );

    // The registry is the audit surface. Read it back as the CLI would.
    let listed = run(&fx, &["org", "list", "--json"]);
    assert_eq!(
        code(&listed),
        0,
        "org list must succeed, got: {}",
        combined(&listed)
    );
    let body = combined(&listed);
    // Not guarded by an `if`: if the backfill silently wrote nothing, this
    // test must fail loudly rather than skip its own assertion. `org sync`
    // printed "N employee row(s) written" above, so N >= 1 is already
    // established — a row must therefore be visible here.
    assert!(
        body.contains("backfill-probe"),
        "the seeded cron job must appear as an employee row after org sync; \
         the reason check below would otherwise be vacuous. Got: {body}"
    );
    assert!(
        body.contains(reason),
        "the employee row must carry the operator's reason, not a constant; \
         got: {body}"
    );
    assert!(
        !body.contains("backfill from cron job"),
        "no row may fall back to the library constant while an operator reason \
         was supplied; got: {body}"
    );
}

/// A repeated ack by the same sender must not write a second ack notice.
///
/// Regression: the CLI hand-rolled its ack and, unlike `NoticeBoard::ack`,
/// appended the sender to `acked_by` and inserted an ack notice on every
/// call. Three acks from one employee produced three protocol rows and a
/// duplicated entry, so the board's own record disagreed with itself. The
/// handler now goes through core's idempotent `ack`.
#[test]
fn org_reason_repeated_ack_is_idempotent() {
    let fx = Fixture::new("ack-idem");

    let posted = run(
        &fx,
        &[
            "org",
            "notice",
            "post",
            "please review the deploy",
            "--sender",
            "emp-ceo01",
            "--recipients",
            "agent:emp-eng01",
            "--correlation-id",
            "corr-ack-1",
            "--ack-required",
            "--reason",
            "asking eng to review",
        ],
    );
    assert_eq!(
        code(&posted),
        0,
        "posting a notice must succeed, got: {}",
        combined(&posted)
    );

    let id = combined(&posted)
        .split("Notice '")
        .nth(1)
        .and_then(|s| s.split('\'').next())
        .map(str::to_string)
        .unwrap_or_default();
    assert!(!id.is_empty(), "the posted notice id must be echoed");

    for _ in 0..3 {
        let ack = run(
            &fx,
            &[
                "org",
                "notice",
                "ack",
                &id,
                "--sender",
                "emp-eng01",
                "--reason",
                "reviewed and looks fine",
            ],
        );
        assert_eq!(
            code(&ack),
            0,
            "each ack must succeed, got: {}",
            combined(&ack)
        );
    }

    // Exactly one ack row must exist for this notice. Core's `ack` stamps
    // `ack of notice <id>` into the ack row's `reason`, which the board
    // listing exposes, so counting that marker counts the ack rows.
    let listed = run(&fx, &["org", "notice", "list", "--json"]);
    let body = combined(&listed);
    let marker = format!("ack of notice {id}");
    let ack_rows = body.matches(&marker).count();
    assert_eq!(
        ack_rows, 1,
        "three acks by one sender must leave exactly one ack row, got {ack_rows}: {body}"
    );
}
