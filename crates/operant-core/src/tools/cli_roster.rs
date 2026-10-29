//! The runtime CLI roster — the catalogue of binaries the agent MAY be
//! granted, and the subcommand capabilities each one yields.
//!
//! Plan: `docs/plans/plan-2026-10-28-native-capability-extension-mcp-cli-runtime-admission.md`
//! Phase A.
//!
//! ## Why this replaced a `const`
//!
//! The catalogue used to be `const KNOWN_CLIS: &[KnownCli]` — 16 entries
//! compiled into the binary. That is the exact thing the operator's
//! requirement forbids: "nothing of CLI/MCP may be compile time". A const
//! also cannot describe a binary installed AFTER the build, which is why
//! `operant context cli add ssh` refused a name the operator could see on
//! their own PATH.
//!
//! ## The MCP parity
//!
//! MCP servers are admitted at CONNECT time: `mcp.rs` calls `list_tools()`
//! → JSON-RPC `tools/list`, and one server yields N tools. This module is
//! the same shape for the CLI family: one binary yields N subcommand
//! capabilities, parsed from its `--help`. The mechanism is identical
//! (discover at runtime, persist the result, advertise names cheaply,
//! resolve schemas on demand), so a reviewer can count capabilities on
//! either family the same way.
//!
//! ## Storage
//!
//! One TOML file under the operant home (`<home>/cli_roster.toml`), resolved
//! through [`crate::platform::operant_home`] — the ONE resolver every store
//! under the operant home uses, so `HERMES_HOME` and the sticky active
//! profile are honoured instead of being bypassed.
//!
//! The file is the CANDIDATE catalogue plus the ADMITTED capabilities. It is
//! deliberately NOT the grant: the grant lives in the operator's config
//! (`[cli.tools]`, in `operant.toml`), which is what the agent's runtime
//! reads. Two files, two jobs:
//!
//! - roster file: what EXISTS on this machine and what it yields
//! - config: what the operator CHOSE to grant
//!
//! A binary in the roster but absent from the config is invisible to the
//! agent, exactly as an unlisted MCP server is never connected.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::cli_discovery::CliCategory;

/// A binary in the roster: the recipe for probing it plus what its
/// `--help` yielded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RosterEntry {
    /// The binary name as it appears on PATH (e.g. "git").
    pub name: String,
    /// argv used to probe the version. Empty means `--version`.
    #[serde(default)]
    pub version_args: Vec<String>,
    /// Which family the binary belongs to, for `operant context cli list`.
    #[serde(default)]
    pub category: CliCategory,
    /// The subcommand capabilities parsed from `--help`. Empty means the
    /// binary was never admitted (or its `--help` yielded nothing
    /// parseable), so it contributes exactly ONE capability: the binary
    /// itself.
    #[serde(default)]
    pub capabilities: Vec<CliCapability>,
    /// Epoch seconds of the last successful `--help` parse. `None` means
    /// never parsed.
    #[serde(default)]
    pub admitted_at: Option<u64>,
}

/// One capability yielded by a binary.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CliCapability {
    /// The capability name as the agent sees it: `<binary>:<subcommand>`.
    /// The binary alone (no colon) is the whole-binary capability.
    pub name: String,
    /// One-line description, parsed from `--help` when available.
    #[serde(default)]
    pub description: Option<String>,
}

impl Default for RosterEntry {
    fn default() -> Self {
        Self {
            name: String::new(),
            version_args: Vec::new(),
            category: CliCategory::Build,
            capabilities: Vec::new(),
            admitted_at: None,
        }
    }
}

/// The whole roster file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CliRoster {
    /// The binaries this machine knows about, keyed by name.
    #[serde(default)]
    pub entries: BTreeMap<String, RosterEntry>,
}

/// The path of the roster file next to a config: the config's own
/// directory, which is the operant home's directory in every real layout.
///
/// Derived from the CONFIG, not from `platform::operant_home()`: the
/// config already knows where it lives (`config.database_path`'s
/// parent is the same directory `tool_state_path` uses), and deriving it
/// from a process-global env var made the roster's location depend on
/// `HERMES_HOME` at the moment of the call — which in a parallel test
/// binary meant one test's roster landed in another test's home, and a
/// `HERMES_HOME`-less run wrote to a directory that might not exist.
///
/// See [`roster_path_from_config`].
pub fn roster_path_from_config(config: &crate::config::AppConfig) -> PathBuf {
    let dir = config
        .database_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    dir.join("cli_roster.toml")
}

/// The path of the roster file under an explicit directory. The test seam:
/// production resolves through [`roster_path_from_config`], and a test
/// that needs an isolated roster passes its own directory rather than
/// mutating the operator's real one.
pub fn roster_path_in(dir: &Path) -> PathBuf {
    dir.join("cli_roster.toml")
}

/// Load the roster. A MISSING file is an empty roster (not an error): the
/// roster is a cache of what was discovered, and an absent one means
/// "nothing discovered yet". A CORRUPT file is ALSO an empty roster rather
/// than a hard failure — the agent must not lose its CLI surface because a
/// cache file went bad. The caller is warned in both cases.
pub fn load_roster(path: &Path) -> CliRoster {
    match std::fs::read_to_string(path) {
        Ok(content) => match toml::from_str::<CliRoster>(&content) {
            Ok(roster) => roster,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "CLI roster is corrupt — treating it as empty"
                );
                CliRoster::default()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => CliRoster::default(),
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "CLI roster unreadable — treating it as empty"
            );
            CliRoster::default()
        }
    }
}

/// Write the roster, atomically enough for a cache file: write to a
/// sibling temp path and rename over the target, so a crash mid-write
/// leaves the previous roster intact rather than a truncated one.
pub fn save_roster(path: &Path, roster: &CliRoster) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(roster)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// The seed catalogue — the 16 entries the compile-time const carried,
/// migrated verbatim so nothing regresses when the const goes away.
///
/// This is NOT the roster: it is what the roster is SEEDED with the first
/// time, and what `operant context cli list` falls back to before any
/// discovery has run. After seeding, the roster file owns the truth.
pub fn seed_entries() -> Vec<RosterEntry> {
    let raw: &[(&str, &[&str], CliCategory)] = &[
        ("git", &["--version"], CliCategory::VersionControl),
        ("python", &["--version"], CliCategory::Language),
        ("python3", &["--version"], CliCategory::Language),
        ("node", &["--version"], CliCategory::Language),
        ("npm", &["--version"], CliCategory::PackageManager),
        ("pip", &["--version"], CliCategory::PackageManager),
        ("pip3", &["--version"], CliCategory::PackageManager),
        ("docker", &["--version"], CliCategory::Container),
        ("cargo", &["--version"], CliCategory::Build),
        ("make", &["--version"], CliCategory::Build),
        (
            "kubectl",
            &["version", "--client", "--short"],
            CliCategory::Cloud,
        ),
        ("rustc", &["--version"], CliCategory::Language),
        ("claude", &["--version"], CliCategory::AiAgent),
        ("gemini", &["--version"], CliCategory::AiAgent),
        ("kilo", &["--version"], CliCategory::AiAgent),
        ("gws", &["--version"], CliCategory::Productivity),
    ];
    raw.iter()
        .map(|(name, version_args, category)| RosterEntry {
            name: (*name).to_string(),
            version_args: version_args.iter().map(|a| (*a).to_string()).collect(),
            category: category.clone(),
            capabilities: Vec::new(),
            admitted_at: None,
        })
        .collect()
}

/// The names the operator may choose from: the roster's entries when the
/// roster file exists, and the seed catalogue before then.
///
/// The plan requires that a roster entry whose binary is NOT installed
/// still be REPORTED (as not installed) rather than silently dropped — the
/// catalogue stops overstating what exists, and the operator sees the
/// difference between "I never heard of it" and "it is not on this
/// machine".
pub fn catalogue_names(path: &Path) -> Vec<String> {
    let roster = load_roster(path);
    if roster.entries.is_empty() {
        return seed_entries().into_iter().map(|e| e.name).collect();
    }
    roster.entries.into_keys().collect()
}

/// The roster entries, seeded when the file is absent.
pub fn roster_or_seed(path: &Path) -> CliRoster {
    let roster = load_roster(path);
    if roster.entries.is_empty() {
        let mut entries = BTreeMap::new();
        for entry in seed_entries() {
            entries.insert(entry.name.clone(), entry);
        }
        return CliRoster { entries };
    }
    roster
}

/// Admit a binary into the roster at runtime, and parse its `--help` for
/// capabilities.
///
/// This is the CLI half of the MCP parity: an MCP server is admitted at
/// CONNECT (its tools arrive over `tools/list`), and a CLI binary is
/// admitted here — at the moment the operator names it — with the same
/// consequence: the catalogue stops being a compile-time list and becomes
/// whatever this machine actually has.
///
/// The roster is seeded on first admission, so admitting one binary does
/// not drop the other 16 from the catalogue. The admission is idempotent:
/// re-admitting an existing name refreshes it rather than duplicating it.
///
/// `capabilities` is what `admit_by_help` parsed; pass `None` to admit
/// the binary WITHOUT re-executing `--help` (the roster's capabilities
/// are then left as they were).
///
/// Returns whether the roster file was written.
pub fn admit_with_capabilities(
    path: &Path,
    name: &str,
    category: CliCategory,
    capabilities: Option<Vec<CliCapability>>,
) -> std::io::Result<bool> {
    let mut roster = roster_or_seed(path);
    let already_present = roster.entries.contains_key(name);
    let previous = roster
        .entries
        .get(name)
        .map(|e| e.capabilities.clone())
        .unwrap_or_default();
    roster.entries.insert(
        name.to_string(),
        RosterEntry {
            name: name.to_string(),
            version_args: vec!["--version".to_string()],
            category,
            capabilities: capabilities.unwrap_or(previous),
            admitted_at: Some(now_epoch_secs()),
        },
    );
    save_roster(path, &roster)?;
    Ok(!already_present)
}

/// Admit a binary into the roster at runtime, WITHOUT re-running `--help`.
///
/// The name is a candidate only; its capabilities stay empty until an
/// admission parses them. Kept separate from
/// [`admit_with_capabilities`] so the "record it" and "discover it"
/// steps are not conflated.
pub fn admit(path: &Path, name: &str, category: CliCategory) -> std::io::Result<bool> {
    admit_with_capabilities(path, name, category, None)
}

/// Remove a binary from the roster.
pub fn remove(path: &Path, name: &str) -> std::io::Result<bool> {
    let mut roster = load_roster(path);
    if roster.entries.remove(name).is_none() {
        return Ok(false);
    }
    save_roster(path, &roster)?;
    Ok(true)
}

/// Admit a binary at runtime by parsing its `--help`.
///
/// This is the CLI half of the MCP parity. An MCP server is admitted at
/// CONNECT: the client calls `tools/list` and the server's tools arrive.
/// A CLI binary is admitted here: the operator names it, its `--help` is
/// executed, and its subcommands become capabilities. One binary yields N
/// capabilities, exactly as one server yields N tools.
///
/// ## The bounds (plan 2026-10-28, A0)
///
/// `--help` introspection means EXECUTING a binary the operator named.
/// That is the mechanism the operator asked for and it is not avoidable
/// here, so it is bounded:
///
/// - the binary is run with `--help` and NOTHING ELSE — no argv from the
///   roster or the config reaches the spawn;
/// - the spawn is [`super::cli_discovery::run_with_timeout`], the ONE
///   spawn primitive for this surface: stdin closed, stdout/stderr piped,
///   a hard deadline. No second spawn primitive exists;
/// - the output is DATA. Only lines matching the command grammar become
///   capabilities, and a capability name is restricted to an identifier,
///   so a hostile binary cannot inject a capability NAME. Descriptions
///   are length-capped.
///
/// ## What a failure yields
///
/// A binary whose `--help` cannot be executed (missing, hangs, exits
/// weirdly) still contributes ONE capability — the binary itself. That is
/// the plan's degradation requirement: admission never fails because a
/// binary is uncooperative.
pub fn admit_by_help(binary: &str, timeout: Duration) -> Vec<CliCapability> {
    let Some(output) = super::cli_discovery::run_with_timeout(binary, &["--help"], timeout) else {
        return vec![super::cli_help_parser::whole_binary_capability(binary)];
    };

    // Some tools print help to stderr (git does when stdout is not a
    // tty in some builds). Take whichever stream carries the text.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let help_text = if stdout.trim().is_empty() {
        stderr.as_ref()
    } else {
        stdout.as_ref()
    };

    let capabilities = super::cli_help_parser::parse_capabilities(binary, help_text);
    if capabilities.is_empty() {
        vec![super::cli_help_parser::whole_binary_capability(binary)]
    } else {
        capabilities
    }
}

/// Epoch seconds, saturating at 0 if the clock is before the epoch.
fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("operant-roster-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn the_seed_carries_the_sixteen_entries_the_const_carried() {
        let seed = seed_entries();
        assert_eq!(seed.len(), 16, "the const had 16 entries");
        let names: Vec<&str> = seed.iter().map(|e| e.name.as_str()).collect();
        for expected in [
            "git", "python", "python3", "node", "npm", "pip", "pip3", "docker", "cargo", "make",
            "kubectl", "rustc", "claude", "gemini", "kilo", "gws",
        ] {
            assert!(names.contains(&expected), "seed is missing {expected}");
        }
    }

    #[test]
    fn the_seed_keeps_the_kubectl_version_recipe_verbatim() {
        // kubectl's recipe is `version --client --short`, NOT `--version` —
        // `kubectl --version` is not a valid flag on every kubectl. The
        // const got this right; the migration must not quietly "normalize"
        // it to `--version`.
        let seed = seed_entries();
        let kubectl = seed.iter().find(|e| e.name == "kubectl").expect("kubectl");
        assert_eq!(kubectl.version_args, vec!["version", "--client", "--short"]);
    }

    #[test]
    fn a_missing_roster_file_is_an_empty_roster_not_an_error() {
        let dir = scratch_dir("missing");
        let roster = load_roster(&roster_path_in(&dir));
        assert!(roster.entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_roster_file_is_an_empty_roster_not_a_panic() {
        let dir = scratch_dir("corrupt");
        let path = roster_path_in(&dir);
        std::fs::write(&path, "this is not = = toml").expect("write");
        let roster = load_roster(&path);
        assert!(roster.entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_roster_round_trips_through_the_file() {
        let dir = scratch_dir("roundtrip");
        let path = roster_path_in(&dir);
        let mut roster = CliRoster::default();
        let mut entry = RosterEntry {
            name: "git".to_string(),
            version_args: vec!["--version".to_string()],
            category: CliCategory::VersionControl,
            capabilities: vec![CliCapability {
                name: "git:commit".to_string(),
                description: Some("Record changes to the repository".to_string()),
            }],
            admitted_at: Some(1_700_000_000),
        };
        entry.name = "git".to_string();
        roster.entries.insert("git".to_string(), entry);

        save_roster(&path, &roster).expect("save");
        let loaded = load_roster(&path);
        let entry = loaded
            .entries
            .get("git")
            .expect("git survived the round trip");
        assert_eq!(entry.category, CliCategory::VersionControl);
        assert_eq!(entry.capabilities.len(), 1);
        assert_eq!(entry.capabilities[0].name, "git:commit");
        assert_eq!(entry.admitted_at, Some(1_700_000_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_catalogue_falls_back_to_the_seed_before_discovery_runs() {
        let dir = scratch_dir("seed-fallback");
        let names = catalogue_names(&roster_path_in(&dir));
        assert_eq!(names.len(), 16);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_roster_with_entries_supersedes_the_seed() {
        // The point of a runtime roster: a binary the const never heard of
        // becomes choosable the moment it is in the roster.
        let dir = scratch_dir("supersede");
        let path = roster_path_in(&dir);
        let mut roster = CliRoster::default();
        roster.entries.insert(
            "ssh".to_string(),
            RosterEntry {
                name: "ssh".to_string(),
                ..Default::default()
            },
        );
        save_roster(&path, &roster).expect("save");

        let names = catalogue_names(&path);
        assert!(names.contains(&"ssh".to_string()), "ssh must be choosable");
        assert!(
            !names.contains(&"gws".to_string()),
            "the seed no longer applies"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admitting_a_binary_adds_it_to_the_catalogue() {
        // The negative the plan names: today `context cli add ssh` fails
        // with "not in the CLI catalogue" because the const never heard of
        // ssh. After admission, ssh is choosable.
        let dir = scratch_dir("admit");
        let path = roster_path_in(&dir);
        let was_new = admit(&path, "ssh", CliCategory::Productivity).expect("admit");
        assert!(was_new, "ssh was not in the roster before");
        let names = catalogue_names(&path);
        assert!(names.contains(&"ssh".to_string()), "ssh must be choosable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admitting_a_binary_does_not_drop_the_seed_catalogue() {
        // The seed exists so nothing regresses when the const goes away.
        // An admission ADDS to it; it must not replace it.
        let dir = scratch_dir("admit-keeps-seed");
        let path = roster_path_in(&dir);
        admit(&path, "ssh", CliCategory::Productivity).expect("admit");
        let names = catalogue_names(&path);
        assert_eq!(names.len(), 17, "16 seeded + ssh");
        assert!(names.contains(&"git".to_string()));
        assert!(names.contains(&"ssh".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admitting_the_same_binary_twice_is_idempotent() {
        let dir = scratch_dir("admit-idempotent");
        let path = roster_path_in(&dir);
        assert!(admit(&path, "ssh", CliCategory::Productivity).expect("first admit"));
        assert!(!admit(&path, "ssh", CliCategory::Productivity).expect("second admit"));
        let names = catalogue_names(&path);
        assert_eq!(names.iter().filter(|n| *n == "ssh").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_a_binary_takes_it_out_of_the_catalogue() {
        let dir = scratch_dir("remove");
        let path = roster_path_in(&dir);
        admit(&path, "ssh", CliCategory::Productivity).expect("admit");
        assert!(remove(&path, "ssh").expect("remove"));
        assert!(!remove(&path, "ssh").expect("second remove"));
        assert!(!catalogue_names(&path).contains(&"ssh".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_admitted_binary_survives_a_reload() {
        // Admission must PERSIST — the plan requires it so a restart does
        // not re-execute `--help` or lose the catalogue.
        let dir = scratch_dir("persist");
        let path = roster_path_in(&dir);
        admit(&path, "ssh", CliCategory::Productivity).expect("admit");
        let reloaded = load_roster(&path);
        let entry = reloaded.entries.get("ssh").expect("ssh survived");
        assert_eq!(entry.category, CliCategory::Productivity);
        assert!(entry.admitted_at.is_some(), "admission is timestamped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admitting_with_capabilities_records_them() {
        let dir = scratch_dir("with-caps");
        let path = roster_path_in(&dir);
        let caps = vec![
            CliCapability {
                name: "git:commit".to_string(),
                description: Some("Record changes".to_string()),
            },
            CliCapability {
                name: "git:push".to_string(),
                description: None,
            },
        ];
        admit_with_capabilities(
            &path,
            "git",
            CliCategory::VersionControl,
            Some(caps.clone()),
        )
        .expect("admit");
        let entry = load_roster(&path)
            .entries
            .get("git")
            .cloned()
            .expect("git is in the roster");
        assert_eq!(entry.capabilities.len(), 2);
        assert_eq!(entry.capabilities[0].name, "git:commit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admitting_without_capabilities_keeps_the_previous_ones() {
        // A plain `admit` must not WIPE a capability set a prior
        // `--help` admission recorded — otherwise a refresh would silently
        // reduce a binary to one capability.
        let dir = scratch_dir("keep-caps");
        let path = roster_path_in(&dir);
        let caps = vec![CliCapability {
            name: "git:commit".to_string(),
            description: None,
        }];
        admit_with_capabilities(&path, "git", CliCategory::VersionControl, Some(caps))
            .expect("first admit");
        admit(&path, "git", CliCategory::VersionControl).expect("second admit");
        let entry = load_roster(&path)
            .entries
            .get("git")
            .cloned()
            .expect("git is in the roster");
        assert_eq!(entry.capabilities.len(), 1, "the capabilities survived");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_binary_with_no_help_yields_one_capability_and_does_not_fail() {
        // The plan's verification item 2: "a binary with a useless --help
        // degrades to a single capability and does not crash admission".
        // `/bin/true` prints a usage line with nothing parseable in it.
        if !std::path::Path::new("/bin/true").exists() {
            return;
        }
        let caps = admit_by_help("/bin/true", Duration::from_secs(5));
        assert_eq!(caps.len(), 1, "{caps:?}");
        assert_eq!(caps[0].name, "/bin/true");
    }

    #[test]
    fn a_binary_that_does_not_exist_yields_one_capability_and_does_not_fail() {
        let caps = admit_by_help("definitely-not-a-real-binary-xyzzy", Duration::from_secs(2));
        assert_eq!(caps.len(), 1, "{caps:?}");
        assert_eq!(caps[0].name, "definitely-not-a-real-binary-xyzzy");
    }

    #[test]
    fn a_binary_that_hangs_yields_one_capability_after_the_deadline() {
        // The A0 bound "a hostile binary hanging the agent": the deadline
        // is enforced, and the outcome is a degraded single capability
        // rather than a hang or a panic.
        //
        // `sleep` with `--help` exits immediately, so the hang is
        // simulated by a binary that ignores its argument: a shell
        // script that sleeps regardless.
        let dir = scratch_dir("hang");
        let script = dir.join("hangy");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        let started = std::time::Instant::now();
        let caps = admit_by_help(script.to_str().unwrap_or_default(), Duration::from_secs(1));
        let elapsed = started.elapsed();
        assert_eq!(caps.len(), 1, "{caps:?}");
        assert!(
            elapsed < Duration::from_secs(10),
            "the deadline must bound the wait, took {elapsed:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_help_spawn_passes_no_arguments_but_help() {
        // The A0 bound "arbitrary code execution at admission": the binary
        // is executed ONLY with `--help`. A config field can name a
        // binary, nothing else — no argv from the roster or the config
        // reaches the spawn.
        //
        // Proven by a script that records its arguments: the only one
        // that may appear is `--help`.
        let dir = scratch_dir("argv");
        let record = dir.join("argv.txt");
        let script = dir.join("argvy");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", record.display()),
        )
        .expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        let caps = admit_by_help(script.to_str().unwrap_or_default(), Duration::from_secs(5));
        assert_eq!(caps.len(), 1, "{caps:?}");
        let recorded = std::fs::read_to_string(&record).unwrap_or_default();
        assert_eq!(recorded.trim(), "--help", "only --help reaches the spawn");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn there_is_exactly_one_spawn_primitive_in_the_cli_surface() {
        // The plan's A0 requirement: "No new spawn primitive may be
        // written for this." The check is structural — the CLI surface
        // modules must not contain a second `Command::new`.
        //
        // `cli_discovery.rs` is the one place allowed to spawn (it owns
        // `run_with_timeout`); `cli_roster.rs` and `cli_help_parser.rs`
        // must not spawn at all.
        //
        // The scan skips the `#[cfg(test)]` module: this test's own
        // source contains the literal it searches for, so scanning the
        // whole file would report itself.
        let production_source = |file: &str| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/tools")
                .join(file);
            let text = std::fs::read_to_string(path).expect("read the module source");
            match text.find("#[cfg(test)]") {
                Some(idx) => text[..idx].to_string(),
                None => text,
            }
        };
        for file in ["cli_roster.rs", "cli_help_parser.rs"] {
            let text = production_source(file);
            assert!(
                !text.contains("Command::new"),
                "{file} must not spawn — the one spawn primitive is run_with_timeout"
            );
        }
        // And the primitive itself is in exactly one place.
        let discovery = production_source("cli_discovery.rs");
        assert_eq!(
            discovery.matches("Command::new").count(),
            1,
            "cli_discovery owns the single spawn"
        );
    }
}
