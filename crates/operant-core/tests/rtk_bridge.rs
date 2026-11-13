//! Integration tests for the RTK bridge.
//!
//! The pure exit-code mapping, the colliding-subcommand guard and the
//! degradation paths run on any host. The tests that need the real `rtk`
//! binary SKIP (print a note and return) when [`find_rtk`] resolves nothing.

use std::path::{Path, PathBuf};
use std::time::Instant;

use operant_core::rtk_bridge::{
    RTK_COLLIDING_SUBCOMMANDS, RewriteVerdict, RtkLocation, classify, find_rtk, is_rtk_command,
    rewrite_command, rtk_version,
};

/// The real rtk binary, or `None` (with a printed note) when this host has
/// none — a test that cannot run says so instead of failing.
fn rtk_binary() -> Option<RtkLocation> {
    match find_rtk() {
        Some(location) => Some(location),
        None => {
            println!("SKIP: no rtk binary on this host (install rtk or set RTK_BINARY)");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Pure exit-code → verdict mapping (no rtk binary needed)
// ---------------------------------------------------------------------------

/// rtk's DEFAULT verdict on a host with no rules: a rewrite WAS printed.
#[test]
fn exit_three_with_a_printed_command_is_rewritten() {
    let verdict = classify(3, "rtk git status\n", "");
    assert_eq!(
        verdict,
        RewriteVerdict::Rewritten {
            command: "rtk git status".to_string(),
        }
    );
}

/// A host that grants permission still yields the rewrite — exit 0 is a
/// permission verdict, not a success signal.
#[test]
fn exit_zero_with_a_printed_command_is_rewritten() {
    let verdict = classify(0, "rtk git status", "");
    assert_eq!(
        verdict,
        RewriteVerdict::Rewritten {
            command: "rtk git status".to_string(),
        }
    );
}

/// The exit code says "rewritten" but nothing was printed: there is nothing
/// to run.
#[test]
fn exit_zero_without_output_is_no_rule() {
    assert_eq!(classify(0, "", ""), RewriteVerdict::NoRule);
}

/// Exit 1 is rtk's "no rewrite" (no RTK equivalent, excluded, or unattestable
/// input): the caller runs the ORIGINAL command unchanged.
#[test]
fn exit_one_without_output_is_no_rule() {
    assert_eq!(classify(1, "", ""), RewriteVerdict::NoRule);
}

/// The exit code is the primary signal; stdout is its confirmation. A command
/// that did come out of stdout is trusted over an exit-1 "no rewrite".
#[test]
fn exit_one_with_output_is_rewritten() {
    let verdict = classify(1, "rtk git status\n", "");
    assert_eq!(
        verdict,
        RewriteVerdict::Rewritten {
            command: "rtk git status".to_string(),
        }
    );
}

/// A deny rule is the ONLY case where the caller must not run the command.
#[test]
fn exit_two_is_refused_with_the_stderr_diagnostic() {
    let verdict = classify(2, "", "rtk: denied by policy\n");
    assert_eq!(
        verdict,
        RewriteVerdict::Refused {
            reason: "rtk: denied by policy".to_string(),
        }
    );
}

/// A pipeline rule that denies a command prints its message on stdout.
#[test]
fn exit_two_uses_the_stdout_message_when_stderr_is_empty() {
    let verdict = classify(2, "blocked by pipeline rule", "");
    assert_eq!(
        verdict,
        RewriteVerdict::Refused {
            reason: "blocked by pipeline rule".to_string(),
        }
    );
}

/// MEASURED: rtk's stderr is empty in every deny on this host. A refusal
/// without any output still carries a reason — it must not look like a
/// success or an invented cause.
#[test]
fn exit_two_without_any_output_still_names_the_missing_reason() {
    let verdict = classify(2, "", "");
    assert_eq!(
        verdict,
        RewriteVerdict::Refused {
            reason: "rtk denied the command without giving a reason".to_string(),
        }
    );
}

/// An exit code rtk does not document means rtk itself misbehaved; the caller
/// runs the original command unchanged.
#[test]
fn an_undocumented_exit_code_is_failed() {
    let verdict = classify(101, "", "rtk crashed");
    match verdict {
        RewriteVerdict::Failed { reason } => {
            assert!(reason.contains("101"), "reason: {reason}");
            assert!(reason.contains("rtk crashed"), "reason: {reason}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// rtk's raw output has no trailing newline; a stray one is trimmed.
#[test]
fn a_rewritten_command_loses_its_trailing_newline() {
    let verdict = classify(3, "rtk git status\n", "");
    assert_eq!(
        verdict,
        RewriteVerdict::Rewritten {
            command: "rtk git status".to_string(),
        }
    );
}

/// The square brackets (`[rtk git status]`) are rtk's DISPLAY convention; the
/// raw stdout has none. The bridge must not strip brackets from what it gets.
#[test]
fn square_brackets_are_not_stripped() {
    let verdict = classify(3, "[rtk git status]", "");
    assert_eq!(
        verdict,
        RewriteVerdict::Rewritten {
            command: "[rtk git status]".to_string(),
        }
    );
}

// ---------------------------------------------------------------------------
// Colliding subcommands (no rtk binary needed)
// ---------------------------------------------------------------------------

/// The measured set is eleven names, pinned by the array's type.
#[test]
fn the_colliding_subcommand_set_has_eleven_names() {
    assert_eq!(RTK_COLLIDING_SUBCOMMANDS.len(), 11);
}

/// Each colliding name is rejected on its own and with arguments.
#[test]
fn every_colliding_subcommand_name_is_rejected() {
    for name in RTK_COLLIDING_SUBCOMMANDS {
        assert!(is_rtk_command(name), "{name} must be rejected");
        assert!(
            is_rtk_command(&format!("{name} --flag")),
            "{name} --flag must be rejected"
        );
    }
}

/// A command rtk can rewrite is not a colliding subcommand.
#[test]
fn a_normal_command_is_not_rejected() {
    assert!(!is_rtk_command("git status"));
    assert!(!is_rtk_command(""));
    assert!(!is_rtk_command("   "));
}

/// Only the FIRST token is inspected.
#[test]
fn is_rtk_command_looks_at_the_first_token_only() {
    assert!(!is_rtk_command("git status && ls"));
}

// ---------------------------------------------------------------------------
// Degradation (no rtk binary needed)
// ---------------------------------------------------------------------------

/// A binary that is not there degrades to "run the original command".
#[test]
fn a_missing_binary_is_failed() {
    let missing = RtkLocation::Path(PathBuf::from("/nonexistent/rtk-bridge-test/rtk"));
    let verdict = rewrite_command(&missing, "git status");
    assert!(
        matches!(verdict, RewriteVerdict::Failed { .. }),
        "verdict: {verdict:?}"
    );
    assert!(rtk_version(&missing).is_none());
}

/// An executable stand-in for rtk, recording exactly what argv it was given.
#[cfg(unix)]
/// A scratch directory for a test that EXECUTES a file it just wrote.
///
/// The workspace filesystem refuses to execute a file whose writeback has not
/// landed yet (ETXTBSY, "Text file busy"), which made the fake-binary tests
/// flaky — they passed or failed depending on how fast the filesystem
/// flushed. tmpfs has no such window, so the fake binaries live there.
/// When `/dev/shm` is not usable, the ordinary temp dir is the fallback and
/// the ETXTBSY window is simply accepted.
fn test_dir() -> tempfile::TempDir {
    match tempfile::Builder::new()
        .prefix("rtk-bridge")
        .tempdir_in("/dev/shm")
    {
        Ok(dir) => dir,
        Err(_) => tempfile::tempdir().expect("a temp dir"),
    }
}

fn fake_rtk(dir: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-rtk");
    // The script is written with an explicit handle that is SYNCED before it
    // is closed: a freshly written file whose writeback has not landed yet
    // fails to exec with ETXTBSY ("Text file busy"), which made this test
    // flaky — it passed or failed depending on how fast the filesystem
    // flushed the page.
    {
        use std::io::Write;
        let Ok(mut file) = std::fs::File::create(&path) else {
            panic!("create fake rtk")
        };
        if file.write_all(script.as_bytes()).is_err() {
            panic!("write fake rtk")
        }
        if file.sync_all().is_err() {
            panic!("sync fake rtk")
        }
    }
    let Ok(metadata) = std::fs::metadata(&path) else {
        panic!("stat fake rtk")
    };
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o755);
    let Ok(()) = std::fs::set_permissions(&path, permissions) else {
        panic!("chmod fake rtk")
    };
    path
}

/// The command is ONE argv element, not a shell string — shell metacharacters
/// inside it are data, never syntax.
#[test]
#[cfg(unix)]
fn the_command_is_passed_as_one_argv_element() {
    let dir = test_dir();
    let record = dir.path().join("argv.txt");
    let command = "git status && ls | grep 'foo bar'";
    let script = fake_rtk(
        dir.path(),
        &format!(
            "#!/bin/sh\n{{\n  printf 'argc=%s\\n' \"$#\"\n  for arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\"; done\n}} > '{}'\nprintf 'rtk git status'\nexit 3\n",
            record.display()
        ),
    );

    let verdict = rewrite_command(&RtkLocation::Env(script), command);
    assert!(
        matches!(verdict, RewriteVerdict::Rewritten { .. }),
        "verdict: {verdict:?}"
    );

    let Ok(recorded) = std::fs::read_to_string(&record) else {
        panic!("read argv record")
    };
    assert!(recorded.contains("argc=2"), "recorded: {recorded}");
    assert!(recorded.contains("arg=rewrite"), "recorded: {recorded}");
    assert!(
        recorded.contains(&format!("arg={command}")),
        "recorded: {recorded}"
    );
}

/// A hung rtk is killed; the agent's turn is never held hostage.
#[test]
#[cfg(unix)]
fn a_hung_rtk_is_killed_and_reported_as_failed() {
    let dir = test_dir();
    let script = fake_rtk(dir.path(), "#!/bin/sh\nexec sleep 30\n");
    let started = Instant::now();
    let verdict = rewrite_command(&RtkLocation::Env(script), "git status");
    let elapsed = started.elapsed();

    match verdict {
        RewriteVerdict::Failed { reason } => {
            assert!(reason.contains("timed out"), "reason: {reason}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "elapsed: {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// Against the real rtk binary (skipped when it is absent)
// ---------------------------------------------------------------------------

/// The default verdict on a host with no rules: exit 3 + a printed rewrite.
#[test]
fn the_real_rtk_rewrites_a_plain_command() {
    let Some(rtk) = rtk_binary() else { return };
    let verdict = rewrite_command(&rtk, "git status");
    assert_eq!(
        verdict,
        RewriteVerdict::Rewritten {
            command: "rtk git status".to_string(),
        }
    );
}

/// rtk rewrites a pipeline's final stage.
#[test]
fn the_real_rtk_rewrites_a_pipeline() {
    let Some(rtk) = rtk_binary() else { return };
    let verdict = rewrite_command(&rtk, "cat README.md | grep foo");
    assert_eq!(
        verdict,
        RewriteVerdict::Rewritten {
            command: "cat README.md | rtk grep foo".to_string(),
        }
    );
}

/// `echo $(ls)` is unattestable input: rtk declines, and the caller runs the
/// command unchanged.
#[test]
fn the_real_rtk_declines_an_unattestable_command() {
    let Some(rtk) = rtk_binary() else { return };
    let verdict = rewrite_command(&rtk, "echo $(ls)");
    assert_eq!(verdict, RewriteVerdict::NoRule);
}

/// `ls -la` would become rtk's own `ls` subcommand instead of the real binary.
#[test]
fn the_bridge_never_rewrites_a_colliding_command() {
    let Some(rtk) = rtk_binary() else { return };
    let verdict = rewrite_command(&rtk, "ls -la");
    assert_eq!(verdict, RewriteVerdict::NoRule);
}

/// The binary reports a version.
#[test]
fn the_real_rtk_reports_its_version() {
    let Some(rtk) = rtk_binary() else { return };
    let Some(version) = rtk_version(&rtk) else {
        panic!("rtk --version should answer")
    };
    assert!(version.starts_with("rtk"), "version: {version}");
}

/// Resolution finds the binary this host carries.
#[test]
fn the_real_rtk_is_resolved() {
    let Some(location) = rtk_binary() else { return };
    assert!(!location.path().as_os_str().is_empty());
}
