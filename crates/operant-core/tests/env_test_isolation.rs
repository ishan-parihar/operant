//! RT-420: the crate's env-mutating tests must hold the shared lock.
//!
//! `HERMES_HOME` and `OPERANT_ENV_FILE` are PROCESS-WIDE: a test that sets
//! one without the shared `TEST_ENV_LOCK` changes it for every other test
//! running at the same time. The failure is not hypothetical — a full-suite
//! run failed with "Source profile 'source' does not exist at
//! /var/tmp/.../profiles/source", because a profile test created the
//! profile under one `HERMES_HOME` and an unlocked env test moved it before
//! the clone.
//!
//! This contract pins the fix at the SOURCE level: every test that mutates a
//! shared variable must take the lock in the same function. It is a source
//! contract because the race is a timing shape — no deterministic fixture
//! can force it without hanging the suite — and the evidence for it is the
//! observed suite failure.

use std::path::Path;

/// The process-wide variables whose mutation can break another test.
const SHARED_VARS: &[&str] = &["HERMES_HOME", "OPERANT_ENV_FILE"];

/// Every file in the crate that mutates one of the shared variables.
fn files_touching_shared_vars() -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if SHARED_VARS.iter().any(|v| {
                    text.contains(&format!("set_var(\"{v}\""))
                        || text.contains(&format!("remove_var(\"{v}\""))
                }) {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    out
}

/// The test region of `text` (everything from the first `#[cfg(test)]`).
fn test_region(text: &str) -> &str {
    match text.find("#[cfg(test)]") {
        Some(i) => &text[i..],
        None => "",
    }
}

#[test]
fn every_shared_env_mutation_is_taken_under_the_lock() {
    let mut offenders: Vec<String> = Vec::new();
    for path in files_touching_shared_vars() {
        let text = std::fs::read_to_string(&path).expect("read");
        let region = test_region(&text);
        if region.is_empty() {
            continue;
        }
        // Walk each test function that mutates a shared variable and check
        // that its body takes the lock.
        let lines: Vec<&str> = region.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let mutates = SHARED_VARS.iter().any(|v| {
                line.contains(&format!("set_var(\"{v}\""))
                    || line.contains(&format!("remove_var(\"{v}\""))
            });
            if !mutates {
                continue;
            }
            // Find the enclosing fn (walk back).
            let mut start = None;
            for j in (0..=i).rev() {
                if lines[j].trim_start().starts_with("fn ")
                    || lines[j].trim_start().starts_with("async fn ")
                {
                    start = Some(j);
                    break;
                }
            }
            let Some(start) = start else { continue };
            // Only TEST functions are checked. A HELPER that mutates the
            // variable (e.g. profile.rs's `use_temp_home`) cannot take the
            // lock itself: its callers already hold it, and the tokio mutex
            // is not reentrant — taking it twice would deadlock. The
            // helper's contract is "the caller holds the lock", which is
            // what the test functions below check.
            let is_test = lines[..start]
                .iter()
                .rev()
                .take(3)
                .any(|l| l.trim() == "#[test]");
            if !is_test {
                continue;
            }
            let body = lines[start..=i].join("\n");
            if !body.contains("test_env_lock") {
                offenders.push(format!(
                    "{}:{} — {}",
                    path.display(),
                    start + 1,
                    lines[start].trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these tests mutate a process-wide variable without the shared lock, so \
         they race every other env-mutating test:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn no_local_lock_guards_a_variable_another_file_also_mutates() {
    // RT-420: the crate had THREE locks for one process-wide environment:
    // the shared `platform::TEST_ENV_LOCK` and two LOCAL `ENV_LOCK`s. A
    // local lock serializes its own file's tests and nothing else — so a
    // test holding it could still race another file's test for the SAME
    // variable. `sourcehound_cdp.rs` and `tools/browser_cdp_tool.rs` both
    // mutate `BROWSER_CDP_URL`, under different locks.
    //
    // The hazard is precise: a local lock is a problem only when it guards a
    // variable ANOTHER file also mutates. A local lock over one file's own
    // fixture (a fake `gh` script, a cache) is fine.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();

    // Which variables does more than one file mutate?
    // A variable is SHARED when MORE THAN ONE FILE mutates it (distinct
    // files, not distinct mutations — a file that sets and clears the same
    // var ten times still touches only itself).
    let mut per_var: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let region = test_region(&text);
        for var in vars_mutated(region) {
            per_var
                .entry(var)
                .or_default()
                .insert(path.display().to_string());
        }
    }
    let shared: Vec<String> = per_var
        .iter()
        .filter(|(_, holders)| holders.len() > 1)
        .map(|(var, _)| var.clone())
        .collect();

    // Now: does any file hold a LOCAL lock while mutating a shared variable?
    let mut offenders: Vec<String> = Vec::new();
    for path in &files {
        if path.file_name().is_some_and(|n| n == "platform.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let region = test_region(&text);
        // Local lock names defined in this file.
        let local_locks: Vec<String> = region
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                if !t.starts_with("static ") || !t.contains("Mutex") {
                    return None;
                }
                let name = t
                    .trim_start_matches("static ")
                    .split(':')
                    .next()?
                    .trim()
                    .to_string();
                Some(name)
            })
            .collect();
        if local_locks.is_empty() {
            continue;
        }
        // Per test function: does it hold a local lock AND mutate a shared var?
        let lines: Vec<&str> = region.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let mutates_shared = vars_mutated(line).iter().any(|v| shared.contains(v));
            if !mutates_shared {
                continue;
            }
            let mut start = None;
            for j in (0..=i).rev() {
                if lines[j].trim_start().starts_with("fn ")
                    || lines[j].trim_start().starts_with("async fn ")
                {
                    start = Some(j);
                    break;
                }
            }
            let Some(start) = start else { continue };
            let body = lines[start..=i].join("\n");
            let holds_local = local_locks.iter().any(|l| body.contains(l));
            let holds_shared = body.contains("test_env_lock");
            if holds_local && !holds_shared {
                offenders.push(format!(
                    "{}:{} — holds a local lock while mutating a shared variable",
                    path.display(),
                    start + 1
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a local lock serializes only its own file, so these tests can still race \
         another file's test for the same process-wide variable:\n{}",
        offenders.join("\n")
    );
}

/// The env-var names a snippet mutates.
fn vars_mutated(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for marker in ["set_var(\"", "remove_var(\""] {
        let mut rest = text;
        while let Some(i) = rest.find(marker) {
            let tail = &rest[i + marker.len()..];
            let Some(end) = tail.find('"') else { break };
            out.push(tail[..end].to_string());
            rest = &tail[end..];
        }
    }
    out
}
