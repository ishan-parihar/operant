//! CLI tool auto-discovery — scans PATH for known CLI tools.
//! Zero external dependencies (uses `std::process::Command` + `std::env`).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Max time to wait for a `which` lookup.
const WHICH_TIMEOUT: Duration = Duration::from_secs(2);
/// Max time to wait for a `<tool> --version` probe.
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// Category of a discovered CLI tool.
///
/// The default variant is `Build` — the same default the hand-written
/// `impl Default` had, so a roster entry that omits its category parses
/// to the same value it did before.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CliCategory {
    VersionControl,
    Language,
    PackageManager,
    Container,
    #[default]
    Build,
    Cloud,
    AiAgent,
    Productivity,
}

impl std::fmt::Display for CliCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VersionControl => write!(f, "Version Control"),
            Self::Language => write!(f, "Language"),
            Self::PackageManager => write!(f, "Package Manager"),
            Self::Container => write!(f, "Container"),
            Self::Build => write!(f, "Build"),
            Self::Cloud => write!(f, "Cloud"),
            Self::AiAgent => write!(f, "AI Agent"),
            Self::Productivity => write!(f, "Productivity"),
        }
    }
}

/// A discovered CLI tool with metadata.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredCli {
    pub name: String,
    pub path: PathBuf,
    pub version: Option<String>,
    pub category: CliCategory,
}

/// The runtime catalogue of CLI tools to scan for.
///
/// Plan 2026-10-28, Phase A1: this used to be `const KNOWN_CLIS` — 16
/// entries compiled into the binary. The operator's requirement forbids
/// that ("nothing of CLI/MCP may be compile time"), and a const cannot
/// describe a binary installed after the build. The catalogue now lives in
/// a roster file next to the config, seeded with the same 16 entries so
/// nothing regresses.
///
/// See [`super::cli_roster`] for the storage and the seed.
fn roster_entries(roster_path: &Path) -> Vec<super::cli_roster::RosterEntry> {
    super::cli_roster::roster_or_seed(roster_path)
        .entries
        .into_values()
        .collect()
}

/// Whether a binary exists on PATH.
///
/// This is the existence check SEPARATE from the version probe: the two
/// answer different questions, and coupling them produced a lie. `ssh
/// --version` prints "unknown option -- -" and exits non-zero, so a
/// version-driven check would report ssh as not installed — while ssh is
/// exactly the binary the plan's verification item 1 names ("`context cli
/// add ssh` must stop being reachable for a binary that exists on PATH").
pub fn is_installed(name: &str) -> bool {
    find_executable(name).is_some()
}

/// Discover available CLI tools on the system.
/// Scans PATH for known tools and returns metadata for each found.
///
/// `roster_path` is where the RUNTIME catalogue lives; the roster
/// supplies the recipes, and `additional` names binaries to probe that
/// the catalogue does not carry yet.
pub fn discover_cli_tools(
    roster_path: &Path,
    additional: &[String],
    excluded: &[String],
) -> Vec<DiscoveredCli> {
    let mut results = Vec::new();

    for known in roster_entries(roster_path) {
        if excluded.iter().any(|e| e == &known.name) {
            continue;
        }
        let version_args: Vec<&str> = if known.version_args.is_empty() {
            vec!["--version"]
        } else {
            known.version_args.iter().map(|a| a.as_str()).collect()
        };
        if let Some(cli) = probe_cli(&known.name, &version_args, known.category.clone()) {
            results.push(cli);
        }
    }

    // Probe additional user-specified tools
    for tool_name in additional {
        if excluded.iter().any(|e| e == tool_name) {
            continue;
        }
        // Skip if already discovered
        if results.iter().any(|r| r.name == *tool_name) {
            continue;
        }
        if let Some(cli) = probe_cli(tool_name, &["--version"], CliCategory::Build) {
            results.push(cli);
        }
    }

    results
}

/// The catalogue of CLIs the agent could be granted.
///
/// This exists so the surface can be CONFIGURED, not just discovered: an
/// operator choosing `[[cli.tools]]` entries needs to know what the choices
/// are, and `operant context cli --list` renders this list. Discovery then
/// filters it to what is actually on PATH — a name in this list that is not
/// installed reports unusable rather than silently vanishing.
///
/// Deliberately does NOT grant anything: listing a CLI here means it is a
/// candidate, and the `[[cli.tools]]` entries decide what is active.
///
/// Plan 2026-10-28 Phase A1: the source is the runtime roster, not a const.
pub fn known_cli_names(roster_path: &Path) -> Vec<String> {
    super::cli_roster::catalogue_names(roster_path)
}

/// Probe a single CLI tool: check if it exists and get its version.
fn probe_cli(name: &str, version_args: &[&str], category: CliCategory) -> Option<DiscoveredCli> {
    // Try to find the tool using `which` (Unix) or `where` (Windows)
    let path = find_executable(name)?;

    // Try to get version
    let version = get_version(name, version_args);

    Some(DiscoveredCli {
        name: name.to_string(),
        path,
        version,
        category,
    })
}

/// Run a command with stdin closed and a hard timeout.
/// Returns `None` if it fails to spawn or does not exit in time.
///
/// This is THE spawn primitive for the CLI surface, and the plan
/// (2026-10-28, A0) requires that no second one be written: every
/// execution of an operator-named binary goes through here, so the
/// bounds — stdin closed, stdout/stderr piped, a hard deadline — are
/// one implementation, not one per call site.
///
/// It is `pub` so the `--help` admission path can reuse it rather than
/// growing its own spawn with its own (possibly weaker) bounds.
pub fn run_with_timeout(name: &str, args: &[&str], timeout: Duration) -> Option<Output> {
    let mut child = Command::new(name)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }

    // Version/banner output is small, so reading after exit cannot deadlock
    // on pipe-buffer backpressure in practice.
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut io) = child.stdout.take() {
        let _ = io.read_to_end(&mut stdout);
    }
    if let Some(mut io) = child.stderr.take() {
        let _ = io.read_to_end(&mut stderr);
    }
    let status = child.wait().ok()?;

    Some(Output {
        status,
        stdout,
        stderr,
    })
}

/// Find an executable on PATH.
fn find_executable(name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let which_cmd = "where";
    #[cfg(not(target_os = "windows"))]
    let which_cmd = "which";

    let output = run_with_timeout(which_cmd, &[name], WHICH_TIMEOUT)?;

    if !output.status.success() {
        return None;
    }

    let path_str = String::from_utf8_lossy(&output.stdout);
    let first_line = path_str.lines().next()?.trim();
    if first_line.is_empty() {
        return None;
    }
    Some(PathBuf::from(first_line))
}

/// Get the version string of a CLI tool.
fn get_version(name: &str, args: &[&str]) -> Option<String> {
    let output = run_with_timeout(name, args, VERSION_TIMEOUT)?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Some tools print version to stderr (e.g., pip)
    let version_text = if stdout.trim().is_empty() {
        stderr.trim().to_string()
    } else {
        stdout.trim().to_string()
    };

    // Extract first line only
    let first_line = version_text.lines().next()?.trim().to_string();
    if first_line.is_empty() {
        return None;
    }
    // A version line carries a number. "unknown option -- -" (what `ssh
    // --version` prints) does not, and storing it as the version would be
    // a lie the operator sees in `operant context cli list`. A tool whose
    // version probe is not `--version` simply reports no version.
    if !first_line.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(first_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory for a test's roster file, so a test never
    /// reads or writes the OPERATOR's live roster.
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant-cli-discovery-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn discover_returns_vec() {
        // Just verify it runs without panic
        let results = discover_cli_tools(
            &crate::tools::cli_roster::roster_path_in(&scratch_dir("discover")),
            &[],
            &[],
        );
        // We can't assert specific tools exist in CI, but structure is valid
        for cli in &results {
            assert!(!cli.name.is_empty());
        }
    }

    #[test]
    fn excluded_tools_are_skipped() {
        let results = discover_cli_tools(
            &crate::tools::cli_roster::roster_path_in(&scratch_dir("excluded")),
            &[],
            &["git".to_string()],
        );
        assert!(!results.iter().any(|r| r.name == "git"));
    }

    #[test]
    fn a_version_probe_that_is_not_a_version_reports_no_version() {
        // `ssh --version` prints "unknown option -- -" and exits non-zero.
        // Storing that as the version would put a lie in `operant context
        // cli list`, so a version line must carry a digit.
        assert!(get_version("ssh", &["--version"]).is_none());
    }

    #[test]
    fn existence_does_not_depend_on_the_version_probe() {
        // The plan's verification item 1 names ssh: `context cli add ssh`
        // must succeed for a binary that IS on PATH. If existence were
        // derived from the version probe, ssh (whose flag is `-V`) would
        // report as not installed.
        if !is_installed("ssh") {
            return; // ssh absent on this machine — nothing to assert
        }
        assert!(is_installed("ssh"));
        // The probe and the existence check are independent: ssh exists
        // even though its `--version` probe yields nothing usable.
        assert!(get_version("ssh", &["--version"]).is_none());
    }

    #[test]
    fn a_binary_that_is_not_installed_is_not_installed() {
        assert!(!is_installed("definitely-not-a-real-binary-xyzzy"));
    }

    #[test]
    fn category_display() {
        assert_eq!(CliCategory::VersionControl.to_string(), "Version Control");
        assert_eq!(CliCategory::Language.to_string(), "Language");
        assert_eq!(CliCategory::PackageManager.to_string(), "Package Manager");
        assert_eq!(CliCategory::Container.to_string(), "Container");
        assert_eq!(CliCategory::Build.to_string(), "Build");
        assert_eq!(CliCategory::Cloud.to_string(), "Cloud");
        assert_eq!(CliCategory::AiAgent.to_string(), "AI Agent");
        assert_eq!(CliCategory::Productivity.to_string(), "Productivity");
    }
}
