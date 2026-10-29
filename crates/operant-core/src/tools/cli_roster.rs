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

/// Admit a binary into the roster at runtime.
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
/// Returns whether the roster file was written.
pub fn admit(path: &Path, name: &str, category: CliCategory) -> std::io::Result<bool> {
    let mut roster = roster_or_seed(path);
    let already_present = roster.entries.contains_key(name);
    roster.entries.insert(
        name.to_string(),
        RosterEntry {
            name: name.to_string(),
            version_args: vec!["--version".to_string()],
            category,
            capabilities: Vec::new(),
            admitted_at: Some(now_epoch_secs()),
        },
    );
    save_roster(path, &roster)?;
    Ok(!already_present)
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
}
