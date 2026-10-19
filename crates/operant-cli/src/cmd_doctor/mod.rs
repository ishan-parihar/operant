//! Module-level re-exports for the `operant doctor` command.
//!
//! This module is a drop-in replacement for the original flat `cmd_doctor.rs`.
//! It delegates individual check groups to sub-modules and exposes the same
//! public API (`handle_doctor_command`) that `main.rs` calls.

pub mod check_result;
pub mod checks_api;
pub mod checks_config;
pub mod checks_fix;
pub mod checks_tools;

use anyhow::Result;
use operant_core::config::AppConfig;
use operant_core::doctor::Severity;

use self::check_result::{
    check_fail, check_info, check_ok, print_banner, print_summary, section_header,
};

/// The exit code `operant doctor` should return.
///
/// Split out because `process::exit` cannot be called from a test without
/// killing the test binary, and this is the only part of the command worth
/// asserting on.
///
/// It used to always be 0. On a fresh machine doctor printed "Found 4 issue(s)
/// to address" and still returned success, so the natural shell incantation
/// `operant doctor && operant chat` walked straight into the failure doctor had
/// just described. A diagnostic that cannot fail is a diagnostic nobody can
/// gate on.
///
/// Both lists count. `issues` are auto-fixable and `manual_issues` need a human,
/// but neither means the system is healthy, and a caller gating on the exit code
/// has no way to tell the two apart without parsing the output.
/// Machine-readable exit status.
///
/// Only `issues` — genuine problems — produce a non-zero code.
/// `_manual_issues` are ADVISORIES: "git not found (recommended)", "Nous Portal
/// auth (not logged in)", "~/.operant/cron/ will be created on first use".
/// Counting them made `operant doctor` exit 1 on a perfectly working install
/// that simply had not configured optional integrations — 13 advisories were
/// present on a working machine, so the exit code was permanently 1 and
/// carried no information. That is the mirror image of the bug this function
/// was written to fix (a doctor that could never fail), and just as useless to
/// anything scripting it. A check that always fails carries no more
/// information than one that never does.
///
/// Exit 1 = something is wrong. Exit 0 = healthy, advisories displayed.
fn doctor_exit_code(issues: &[String], _manual_issues: &[String]) -> i32 {
    if issues.is_empty() { 0 } else { 1 }
}

/// Dispatch handle — called from `main.rs` for `operant doctor [--fix] [--json]`.
pub async fn handle_doctor_command(config: &AppConfig, fix: bool, json: bool) -> Result<()> {
    // H10: enter json mode BEFORE any check runs — every display helper
    // suppresses its prints, so `doctor --json`'s stdout is exactly one
    // JSON document (a smoke check parses it whole).
    check_result::set_json_mode(json);
    if fix {
        return checks_fix::cmd_fix(config).await;
    }

    let mut issues: Vec<String> = Vec::new();
    let mut manual_issues: Vec<String> = Vec::new();

    // Each section runs its checks and returns (issues, manual_issues).
    checks_config::run_config_checks(config, &mut issues, &mut manual_issues);
    checks_tools::run_tool_checks(config, &mut issues, &mut manual_issues);
    checks_tools::run_integration_checks(&mut issues, &mut manual_issues).await;
    checks_api::run_api_checks(config, &mut issues).await;
    checks_tools::run_platform_checks(config, &mut issues, &mut manual_issues);
    checks_tools::check_operant_binaries(&mut manual_issues);

    // Unified engine (docs/ORGANISM-ARCHITECTURE.md §6 F2): the SAME check
    // list the gateway serializes at GET /api/doctor. Engine errors are
    // genuine issues (they gate the exit code); warnings are advisories.
    // W4 (audit O-25): the org-mode section — only when [holon] enabled.
    // Personal mode keeps its exact output (invariant 5).
    if config.holon.enabled {
        section_header("Holon org mode");
        print_holon_org_checks(config);
    }

    let engine_results = operant_core::doctor::diagnose(config);
    for result in &engine_results {
        match result.severity {
            Severity::Error => issues.push(result.message.clone()),
            Severity::Warn => manual_issues.push(result.message.clone()),
            Severity::Ok => {}
        }
    }
    // H10 (docs/agentic-os/): `doctor --json` is a smoke check — its stdout
    // must be EXACTLY one JSON document. Every human-readable print gates on
    // `!json` (the section headers and the text render used to leak into
    // the stream and break parsers).
    if !json {
        section_header("Diagnostics");
        print!("{}", operant_core::doctor::render_text(&engine_results));
    }

    if json {
        let result = serde_json::json!({
            "issues": issues,
            "manual_issues": manual_issues,
            "total_issues": issues.len() + manual_issues.len(),
            "auto_fixable": issues.len(),
            "manual_required": manual_issues.len(),
            // The unified engine's structured results — the same list the
            // gateway serves at GET /api/doctor (§6 F2).
            "diagnostics": engine_results,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_banner();
        print_summary(&issues, &manual_issues, 0, false);
    }

    // Flush before exiting. stdout is block-buffered when it is not a terminal
    // and `process::exit` runs no destructors, so a non-zero exit here would
    // otherwise throw away the report the user just asked for — and the case
    // where that bites hardest is the piped one (`operant doctor | tee log`),
    // which is exactly where a machine-readable exit code is most wanted.
    use std::io::Write;
    let _ = std::io::stdout().flush();

    let code = doctor_exit_code(&issues, &manual_issues);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// G3 (REMAINING-GAPS): the clock organ's liveness verdict, read from the
/// stamp `organs.rs::stamp_last_tick` writes after every successful
/// `holon tick`. Before this, doctor unconditionally printed OK for the
/// clock organ — a clock that died at boot read as healthy forever.
#[derive(Debug, PartialEq, Eq)]
enum ClockVerdict {
    /// No stamp (or an unreadable one): no tick ever completed.
    NeverTicked,
    /// Last tick's age in seconds — beyond the staleness bound.
    Stale(u64),
    /// Last tick's age in seconds — within the staleness bound.
    Fresh(u64),
}

/// Generous vs the 120s cadence: one missed tick on a loaded box must not
/// flunk the doctor. Two-and-a-half cadences without a stamp is dead or
/// wedged.
const CLOCK_STALE_AFTER_SECS: u64 = 300;

fn clock_organ_verdict(home: &std::path::Path) -> ClockVerdict {
    let stamp = home.join(".clock").join("last_tick");
    let text = match std::fs::read_to_string(&stamp) {
        Ok(t) => t,
        Err(_) => return ClockVerdict::NeverTicked,
    };
    let ticked_at = match text.trim().parse::<u64>() {
        Ok(s) => s,
        Err(_) => return ClockVerdict::NeverTicked,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let age = now.saturating_sub(ticked_at);
    if age > CLOCK_STALE_AFTER_SECS {
        ClockVerdict::Stale(age)
    } else {
        ClockVerdict::Fresh(age)
    }
}

/// O-25: org-mode lines. The read-API call goes through the same
/// `operant_os::api` every org view uses (mode=ro, the version gate), so
/// what doctor says is what `operant org holon` will hit.
fn print_holon_org_checks(config: &AppConfig) {
    let home = config.holon.home.clone();
    if home.as_os_str().is_empty() {
        check_fail(
            "[holon]",
            "enabled but no home is set — org mode cannot reach an instance",
        );
        return;
    }
    // `HolonApi::open` is itself the gate: it opens mode=ro AND refuses an
    // api_version this build does not know — a success line means every
    // org view will work.
    match operant_os::api::HolonApi::open(&home) {
        Ok(_) => check_ok(
            "[holon]",
            &format!("read API reachable at {}", home.display()),
        ),
        Err(e) => check_fail("[holon]", &format!("holon instance: {e}")),
    }
    match &config.holon.bin {
        Some(b) if std::path::Path::new(b).exists() => {
            check_ok("[holon]", &format!("kernel launcher pinned at {b}"));
        }
        Some(b) => {
            check_fail(
                "[holon]",
                &format!("kernel launcher [holon] bin={b} does not exist"),
            );
        }
        None => {
            let line = "[holon] bin unset — the PATH `holon` (or $HOLON_BIN) is used".to_string();
            check_info(&line);
        }
    }
    if config.holon.supervise {
        if config.holon.supervise_employees.is_empty() {
            check_fail("[holon]", "supervise is on with no supervise_employees");
        } else {
            check_ok(
                "[holon]",
                &format!(
                    "supervise organ: {} employee(s), max {} concurrent run(s)",
                    config.holon.supervise_employees.len(),
                    config.holon.max_concurrent_runs
                ),
            );
        }
    }
    if config.holon.clock {
        match clock_organ_verdict(&home) {
            ClockVerdict::NeverTicked => check_fail(
                "[holon]",
                "clock organ: no liveness stamp — `holon tick` has never completed (a daemon younger than 2 minutes has not ticked yet)",
            ),
            ClockVerdict::Stale(age_secs) => check_fail(
                "[holon]",
                &format!(
                    "clock organ stale: last tick {} minute(s) ago (cadence is 2 minutes)",
                    age_secs / 60
                ),
            ),
            ClockVerdict::Fresh(age_secs) => check_ok(
                "[holon]",
                &format!("clock organ: last tick {age_secs}s ago (cadence 2 minutes)"),
            ),
        }
    }
    // O-25: the Notion arm's script — the same three-way resolution the
    // gateway arm uses ([holon] notion_script, else <bin>/../adapters/notion,
    // else the instance home). A missing script means the arm stays off
    // silently; doctor says so.
    let notion_bin_fallback = config
        .holon
        .bin
        .as_deref()
        .and_then(|b| std::path::Path::new(b).parent())
        .map(|p| p.join("adapters").join("notion").join("holon_notion.py"));
    let notion_home_fallback = home.join("adapters").join("notion").join("holon_notion.py");
    let notion = match &config.holon.notion_script {
        Some(p) => Some(p.clone()),
        None => notion_bin_fallback.filter(|p| p.exists()).or_else(|| {
            notion_home_fallback
                .exists()
                .then_some(notion_home_fallback.clone())
        }),
    };
    match notion {
        Some(p) if p.exists() => check_ok(
            "[holon]",
            &format!("notion adapter script at {}", p.display()),
        ),
        _ => check_fail(
            "[holon]",
            "no notion adapter script (set [holon] notion_script, or install the adapter) — the notion webhook arm stays off",
        ),
    }
    // O-25: the archive's evidence — the last snapshot directory under
    // the operant home. Doctor names it and its age; an org-mode operator
    // wants to know the archived state is recent before retiring readers.
    let archive_root = operant_core::platform::operant_home().join("archive");
    let latest = std::fs::read_dir(&archive_root)
        .ok()
        .and_then(|rd| {
            let mut entries: Vec<std::path::PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("org-"))
                })
                .collect();
            entries.sort();
            entries.pop()
        })
        .filter(|p| p.is_dir());
    match latest {
        Some(p) => {
            let age = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| {
                    let secs = std::time::SystemTime::now()
                        .duration_since(t)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    if secs < 86400 {
                        format!("{}h ago", secs / 3600)
                    } else {
                        format!("{}d ago", secs / 86400)
                    }
                })
                .unwrap_or_else(|| "unknown age".to_string());
            check_ok(
                "[holon]",
                &format!("org archive: {} ({})", p.display(), age),
            );
        }
        None => check_info(
            "[holon] no org archive taken yet — run `operant org holon archive` before retiring live readers",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{ClockVerdict, clock_organ_verdict, doctor_exit_code};

    /// G3 regression: doctor must distinguish a ticking clock organ from
    /// a dead one. Before the liveness stamp it printed OK unconditionally.
    #[test]
    fn clock_verdict_reads_the_liveness_stamp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path();
        let stamp_dir = home.join(".clock");
        std::fs::create_dir_all(&stamp_dir).expect("stamp dir");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // No stamp yet — the organ never completed a tick.
        assert_eq!(clock_organ_verdict(home), ClockVerdict::NeverTicked);

        // Fresh stamp (the organs.rs writer's exact format: unix seconds).
        std::fs::write(stamp_dir.join("last_tick"), now.to_string()).expect("stamp");
        match clock_organ_verdict(home) {
            ClockVerdict::Fresh(age) => assert!(age <= 2, "fresh stamp read as {age}s old"),
            other => panic!("a fresh stamp must be Fresh, got {other:?}"),
        }

        // Stale stamp — one past the 300s bound.
        std::fs::write(stamp_dir.join("last_tick"), (now - 301).to_string()).expect("stale");
        match clock_organ_verdict(home) {
            ClockVerdict::Stale(age) => {
                assert!(age >= 301, "stale stamp read as {age}s old")
            }
            other => panic!("a stale stamp must be Stale, got {other:?}"),
        }

        // Garbage stamp — treated as never ticked, never a panic.
        std::fs::write(stamp_dir.join("last_tick"), "not-a-number").expect("garbage");
        assert_eq!(clock_organ_verdict(home), ClockVerdict::NeverTicked);
    }

    fn issues(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("issue {i}")).collect()
    }

    #[test]
    fn a_clean_doctor_run_exits_zero() {
        assert_eq!(doctor_exit_code(&[], &[]), 0);
    }

    /// The regression: doctor found things and reported success anyway.
    #[test]
    fn finding_issues_exits_nonzero() {
        assert_eq!(doctor_exit_code(&issues(4), &[]), 1);
    }

    /// Manual issues count too — a caller gating on the exit code cannot tell
    /// Manual issues are advisories ("git not found (recommended)"), not
    /// failures. Counting them meant a healthy install with unconfigured
    /// optional integrations could never exit 0 — 13 such advisories were
    /// present on a working machine — so the exit code carried no information
    /// at all. This pins the corrected contract: advisories are displayed, and
    /// they do not gate a caller.
    #[test]
    fn manual_issues_do_not_fail_the_run() {
        assert_eq!(doctor_exit_code(&[], &issues(1)), 0);
        assert_eq!(doctor_exit_code(&[], &issues(13)), 0);
    }

    /// …but a real issue still fails even alongside advisories, which is the
    /// half of the contract that matters for scripting.
    #[test]
    fn real_issue_fails_even_with_advisories_present() {
        assert_eq!(doctor_exit_code(&issues(1), &issues(5)), 1);
    }

    #[test]
    fn a_single_issue_is_enough_to_fail() {
        assert_eq!(doctor_exit_code(&["one".to_string()], &[]), 1);
    }
}
