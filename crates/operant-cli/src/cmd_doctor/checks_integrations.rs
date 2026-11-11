//! Doctor checks for the editor/pane integrations (Herdr, Omarchy).
//!
//! These are the checks that answer the two complaints this integration
//! exists for:
//!
//! * "my active herdr instance is not able to identify the active operant
//!   sessions"
//! * "omarchy does not show the option to use operant as the default"
//!
//! ## Issues vs advisories
//!
//! A machine without Herdr, or without Omarchy, is a perfectly good machine.
//! So the ABSENCE of the other program is an advisory, never an issue: it
//! would fail the exit code of a healthy install that simply has no use for
//! the integration. What IS an issue is the other program being present and
//! the integration being half-wired — that is the broken state the operator
//! is actually reporting, and it is repairable with `operant doctor --fix`.
//!
//! The split follows the same rule the rest of doctor uses: `issues` gate the
//! exit code, `manual_issues` are advice.

use crate::cmd_doctor::check_result::{
    check_fail, check_info, check_ok, check_warn, section_header,
};
use crate::integrations::{herdr_plugin, omarchy_agent};

/// Run every integration check.
pub fn run_integration_checks(issues: &mut Vec<String>, manual_issues: &mut Vec<String>) {
    check_herdr(issues, manual_issues);
    check_omarchy(issues, manual_issues);
}

/// The Herdr integration.
///
/// The runtime half (Operant reporting its own state) needs nothing
/// installed, so the checks are about whether the reporter is LIVE in this
/// process and whether the plugin around it is in place.
fn check_herdr(issues: &mut Vec<String>, manual_issues: &mut Vec<String>) {
    section_header("Herdr integration");

    if !herdr_plugin::herdr_available() {
        // No Herdr on this machine: nothing to integrate with.
        manual_issues
            .push("herdr is not installed — the Herdr integration stays inactive".to_string());
        check_info("herdr is not installed (the Herdr integration stays inactive)");
        return;
    }
    check_ok(
        "herdr",
        &format!("{} is installed", herdr_plugin::herdr_bin()),
    );

    // Is this process inside a Herdr pane? That is what decides whether the
    // reporter is live.
    match operant_core::herdr::HerdrContext::from_env() {
        Some(ctx) => {
            let pane = ctx.pane_id.clone();
            if ctx.server_socket_present() {
                check_ok(
                    "herdr",
                    &format!("reporting lifecycle state to pane {pane}"),
                );
            } else {
                // The pane context is there but the server is gone: the
                // reports would fail silently, which is exactly the "herdr
                // cannot see my sessions" complaint in another shape.
                check_warn(
                    "herdr",
                    &format!(
                        "pane {pane} has a Herdr context but its server socket is gone — \
                         the reports will not arrive"
                    ),
                );
                manual_issues.push(format!(
                    "herdr: pane {pane} has a Herdr context but its API socket is missing"
                ));
            }
        }
        None => {
            // Not a fault: most Operant runs are not in a Herdr pane.
            check_info(
                "not running inside a Herdr pane (the integration activates when Operant runs in one)",
            );
        }
    }

    // The plugin: the human-facing surface around the reporter.
    let status = herdr_plugin::status();
    if !status.files_current {
        check_fail(
            "herdr",
            &format!(
                "the Operant plugin is out of date ({}) — run 'operant doctor --fix'",
                status.stale.join(", ")
            ),
        );
        issues.push(format!(
            "herdr: the Operant plugin is out of date ({})",
            status.stale.join(", ")
        ));
    } else if !status.linked {
        check_fail(
            "herdr",
            "the Operant plugin is not registered — run 'operant doctor --fix'",
        );
        issues.push("herdr: the Operant plugin is not registered".to_string());
    } else {
        check_ok("herdr", "the Operant plugin is installed and registered");
    }

    // WHAT HERDR SEES RIGHT NOW.
    //
    // This is the question the operator asked ("can herdr see my operant
    // sessions?"), and the plugin being installed does not answer it: the
    // reporter only runs inside a pane, so the live view is the only evidence
    // that matters.
    let live = herdr_plugin::live_agents();
    let operant_panes: Vec<&str> = live
        .iter()
        .filter(|agent| agent.agent == "operant")
        .map(|agent| agent.pane_id.as_str())
        .collect();
    if operant_panes.is_empty() {
        check_info(
            "herdr sees no operant agent right now (expected until an Operant session runs in a Herdr pane)",
        );
    } else {
        check_ok(
            "herdr",
            &format!(
                "herdr sees {} operant agent(s): {}",
                operant_panes.len(),
                operant_panes.join(", ")
            ),
        );
    }

    // THE MISLABEL.
    //
    // Without a reporter, Herdr's screen-based detection matches the Pi
    // manifest against Operant's TUI, so the pane shows up as a Pi agent. The
    // pane is running Operant and Herdr is calling it something else — that is
    // a fault, not a preference, and it is the one the operator reported.
    for pane in &herdr_plugin::mislabelled_operant_panes() {
        check_fail(
            "herdr",
            &format!(
                "herdr calls pane {} '{}' but operant is running in it — the binary in that pane predates the integration; restart it",
                pane.pane_id, pane.label
            ),
        );
        issues.push(format!(
            "herdr: pane {} runs operant but herdr labels it '{}'",
            pane.pane_id, pane.label
        ));
    }
}

/// The Omarchy integration.
fn check_omarchy(issues: &mut Vec<String>, manual_issues: &mut Vec<String>) {
    section_header("Omarchy integration");

    if !omarchy_agent::omarchy_available() {
        manual_issues
            .push("omarchy is not installed — the Omarchy integration stays inactive".to_string());
        check_info("omarchy is not installed (the Omarchy integration stays inactive)");
        return;
    }
    check_ok("omarchy", "the packaged agent launcher is present");

    // The menu row launches `operant` from PATH. A row that cannot launch is
    // worse than no row: the operator picks the agent and nothing happens.
    match which("operant") {
        Some(path) => check_ok(
            "omarchy",
            &format!("operant is on PATH ({})", path.display()),
        ),
        None => {
            check_fail(
                "omarchy",
                "operant is not on PATH — the menu row would fail to launch it",
            );
            issues.push("omarchy: operant is not on PATH".to_string());
        }
    }

    let status = omarchy_agent::status();

    if !status.wrappers_current {
        check_fail(
            "omarchy",
            &format!(
                "the user-level agent wrapper is out of date ({}) — run 'operant doctor --fix'",
                status.stale.join(", ")
            ),
        );
        issues.push(format!(
            "omarchy: the user-level agent wrapper is out of date ({})",
            status.stale.join(", ")
        ));
    } else {
        check_ok("omarchy", "the user-level agent wrapper is current");
    }

    if !status.registered {
        check_fail(
            "omarchy",
            "operant is not registered as an extra agent — run 'operant doctor --fix'",
        );
        issues.push("omarchy: operant is not registered as an extra agent".to_string());
    } else {
        check_ok("omarchy", "operant is registered as a selectable agent");
    }

    if !status.menu_row {
        check_fail(
            "omarchy",
            "the agent menu has no operant row — run 'operant doctor --fix'",
        );
        issues.push("omarchy: the agent menu has no operant row".to_string());
    } else {
        check_ok("omarchy", "the agent menu offers operant");
    }

    // The default agent is information, not a fault: the point of the
    // integration is that Operant is SELECTABLE, not that it is selected.
    match status.default_agent.as_deref() {
        Some("operant") => check_ok("omarchy", "operant is the current default agent"),
        Some(other) => check_info(&format!(
            "the current default agent is {other} (operant is selectable)"
        )),
        None => check_info("no default agent is chosen (operant is selectable)"),
    }

    // The generalization must not have cost the operator the agent they were
    // already using.
    if status.jcode_preserved {
        check_ok("omarchy", "jcode is still registered as an extra agent");
    } else if std::path::Path::new("/usr/local/bin/omarchy-jcode").exists()
        || which("omarchy-jcode").is_some()
    {
        // The launcher is on PATH but the registration is missing: the
        // hand-written wrapper was replaced without carrying jcode over.
        check_fail(
            "omarchy",
            "jcode is installed but no longer registered — run 'operant doctor --fix'",
        );
        issues.push("omarchy: jcode is installed but no longer registered".to_string());
    }
}

/// Resolve a command on PATH.
fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine without Herdr or Omarchy must NOT produce issues.
    ///
    /// This is the contract that keeps `operant doctor` meaningful: an
    /// advisory is advice, an issue is a fault. Counting "the other program
    /// is not installed" as an issue would make the exit code permanently 1
    /// on a perfectly healthy machine that simply has no use for either
    /// integration.
    #[test]
    fn absent_integrations_are_advisories_not_issues() {
        let mut issues = Vec::new();
        let mut manual = Vec::new();
        run_integration_checks(&mut issues, &mut manual);
        if !herdr_plugin::herdr_available() {
            assert!(
                !issues.iter().any(|issue| issue.starts_with("herdr:")),
                "a missing herdr must not be an issue: {issues:?}"
            );
        }
        if !omarchy_agent::omarchy_available() {
            assert!(
                !issues.iter().any(|issue| issue.starts_with("omarchy:")),
                "a missing omarchy must not be an issue: {issues:?}"
            );
        }
    }
}
