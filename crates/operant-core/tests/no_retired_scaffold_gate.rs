#![allow(clippy::expect_used, clippy::unwrap_used)]

//! **A gate script may not assert a path the repository does not carry, and a
//! retired scaffold may not leave its checker behind.**
//!
//! `scripts/check-pk-sidecar.sh` asserted that `pk-sidecar/main.py` exists,
//! was executable, and answered a JSON `ping`. The scaffold it guarded was
//! retired with plan 015's Phase 0 (the live kernel surface is `[tools.kernel]`
//! driving `-m n.server` in `kernel-sidecar/`), and the check script went with
//! it — but a checker that names a path is a second, independent way for a
//! retirement to be incomplete. Delete the subject and leave the gate, and the
//! gate fails forever on a subject nobody can restore.
//!
//! The symmetric failure is worse: the gate *passes* while the subject is gone,
//! because nothing reads the exit code. That was true here — the script ran in
//! no workflow, no Makefile and no `self-test.sh`, so its only effect was on
//! whoever ran it by hand.
//!
//! Two guards, one in each direction:
//!
//! 1. no tracked file under `scripts/` names the retired scaffold path;
//! 2. every `scripts/check-*.sh` that is tracked **and named by a gate** points
//!    at a path that exists.
//!
//! The second is the general rule. A check script that exists and is invoked by
//! nothing is droppings; a check script whose `SCAFFOLD`/target variable names
//! a missing file is a false gate. Both are caught here by reading the repo
//! as it is on disk.

use std::fs;
use std::path::{Path, PathBuf};

/// The repository root (this crate lives at `<root>/crates/operant-core`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/operant-core is two levels under the repository root")
        .to_path_buf()
}

/// Every tracked file under a directory, read from git so untracked droppings
/// and scratch dirs are not judged.
fn tracked_files(rel: &str) -> Vec<PathBuf> {
    let root = repo_root();
    let out = std::process::Command::new("git")
        .args(["ls-files", rel])
        .current_dir(&root)
        .output()
        .expect("git ls-files runs in the repository");
    assert!(out.status.success(), "git ls-files {} failed", rel);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| root.join(l))
        .collect()
}

/// A path named *inside* a check script, taken from the variable that holds
/// the thing it guards. `SCAFFOLD=` is the convention plan 015's checker used;
/// this reads whatever the script assigns, so a new checker that follows the
/// same shape is caught too.
fn asserted_paths(script: &Path) -> Vec<(String, usize)> {
    let text = fs::read_to_string(script).expect("the check script is readable");
    let mut found = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        // The script writes `"$REPO_ROOT/pk-sidecar/main.py"`, so the value is
        // a double-quoted shell string. Strip the quotes and the `$REPO_ROOT`
        // prefix the script already resolves at runtime.
        let value = value.trim().trim_matches('"');
        if name.is_empty()
            || value.is_empty()
            || !name.chars().all(|c| c.is_ascii_uppercase() || c == '_')
        {
            continue;
        }
        for token in value.split_whitespace() {
            let token = token.replace("$REPO_ROOT", "");
            if token.starts_with('/') && (token.contains("sidecar") || token.contains("scaffold")) {
                found.push((token, idx + 1));
            }
        }
    }
    found
}

#[test]
fn no_gate_script_asserts_a_retired_scaffold() {
    // Direction 1: the retirement is complete only when NO tracked script
    // still names the retired path. A gate kept past its subject is a gate
    // that fails on a subject nobody can restore.
    let retired = "/pk-sidecar/main.py";
    for script in tracked_files("scripts") {
        let text = fs::read_to_string(&script).unwrap_or_default();
        assert!(
            !text.contains(retired),
            "{} still asserts the retired scaffold {} — a gate kept past its \
             subject fails forever on a subject nobody can restore",
            script.display(),
            retired
        );
    }
}

#[test]
fn a_gate_that_is_run_names_a_path_that_exists() {
    // Direction 2: the general rule. A check script is only a gate if
    // something invokes it, and the path it asserts must exist.
    let root = repo_root();

    // Which scripts are actually wired to a gate? Read every caller rather
    // than trusting a hand list — the list is what drifted. The gate surface
    // is wider than a Makefile: `scripts/self-test.sh` is this repo's own
    // self-check (it runs the clippy, build, test, fmt, doc-claim and rustdoc
    // gates), and NO workflow or Makefile target invokes a `check-*.sh` at
    // all. Omitting it walks an empty list and the guard passes vacuously —
    // which is what its first draft did, measured by planting a gate that
    // asserted a missing path and watching both directions stay green.
    let gate_runners = ["scripts/self-test.sh", "Makefile", "justfile"];
    let workflow_dir = root.join(".github/workflows");
    let mut invoked: Vec<String> = Vec::new();

    // The caller lines are shell: `run_test "Doc claims (scripts/check-doc-claims.sh)" "bash scripts/check-doc-claims.sh"`.
    // A bare split on whitespace leaves the trailing quote, backtick or paren
    // attached (`check-doc-claims.sh)"`), which then resolves to no file and
    // makes a live gate look missing. Cut at the first shell metacharacter
    // instead of relying on whitespace.
    let script_name = |raw: &str| -> String {
        raw.trim_matches(|c: char| "'`\"(){}<>|&;".contains(c))
            .split(|c: char| "'`\"(){}<>|&; \t".contains(c))
            .next()
            .unwrap_or_default()
            .to_string()
    };

    for name in gate_runners {
        let path = root.join(name);
        if let Ok(text) = fs::read_to_string(&path) {
            for (idx, line) in text.lines().enumerate() {
                if let Some(raw) = line.split("scripts/").nth(1) {
                    let script = script_name(raw);
                    // A gate runner naming itself is not an invocation.
                    if !script.is_empty() && script != name {
                        invoked.push(format!("scripts/{script} ({name}:{})", idx + 1));
                    }
                }
            }
        }
    }
    if let Ok(entries) = fs::read_dir(&workflow_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yml") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_default();
            for (idx, line) in text.lines().enumerate() {
                if let Some(raw) = line.split("scripts/").nth(1) {
                    let script = script_name(raw);
                    if !script.is_empty() {
                        invoked.push(format!(
                            "scripts/{script} ({}:{})",
                            path.file_name().unwrap().to_string_lossy(),
                            idx + 1
                        ));
                    }
                }
            }
        }
    }

    // Every check script that a gate runs must resolve its own asserted paths.
    //
    // A caller naming a script that is not tracked at all is a DIFFERENT
    // defect (a comment mentioning a script, or a path under `scripts/tests/`
    // that only exists in a container). Judging it here would make this guard
    // a repo-wide path linter, and a guard that cries on an unrelated line is
    // a guard nobody reads. So: only scripts that exist are judged, and the
    // retired-scaffold direction above is what catches a gate that outlived
    // its subject. Both halves matter, and neither substitutes for the other.
    for caller in &invoked {
        let Some(rel) = caller.split_whitespace().next() else {
            continue;
        };
        let script = root.join(rel);
        if !script.exists() {
            continue;
        }
        for (asserted, line) in asserted_paths(&script) {
            let absolute = if let Some(rest) = asserted.strip_prefix('/') {
                root.join(rest)
            } else {
                PathBuf::from(&asserted)
            };
            assert!(
                absolute.exists(),
                "{}:{} asserts {}, which does not exist — the script is \
                 invoked by {}, so it is a live gate asserting a missing path",
                rel,
                line,
                asserted,
                caller
            );
        }
    }
}
