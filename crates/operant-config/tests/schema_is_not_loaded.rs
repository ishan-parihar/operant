#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The parallel config schema must not start being loaded by accident.
//!
//! `operant_config::schema::Config` is a SPEC, not the live config: the
//! binary reads `operant_core::config::AppConfig`. The schema's loader,
//! `Config::load_or_init`, has no production caller — only this crate's
//! tests call it.
//!
//! This guard scans every non-test Rust file in the workspace and fails if
//! one of them loads the aggregate. The point is not to forbid the change —
//! adopting the schema surface as the config spine is a legitimate plan
//! (`docs/plans/plan-2026-10-21-config-schema-retirement.md`) — but to make
//! the change DELIBERATE: whoever wires it up has to update this test in
//! the same commit, so the two never drift.

use std::path::{Path, PathBuf};

/// The workspace root (this crate lives at `<root>/crates/operant-config`).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/operant-config is two levels under the workspace root")
        .to_path_buf()
}

/// Remove `#[cfg(test)]`-attributed items (braces included) so a call in a
/// test module is not counted as a production call.
fn strip_test_items(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(idx) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx + "#[cfg(test)]".len()..];
        match after.find('{') {
            Some(_brace) => {
                let mut depth: i64 = 0;
                let mut end = after.len();
                for (i, ch) in after.char_indices() {
                    if ch == '{' {
                        depth += 1;
                    } else if ch == '}' {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                }
                rest = &after[end..];
            }
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_parallel_schema_is_not_loaded_in_production() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut offenders: Vec<String> = Vec::new();

    for path in files {
        // The crate that OWNS the schema is where the loader is defined and
        // where its own tests live — both are expected. This guard is about
        // the rest of the workspace.
        if path.starts_with(root.join("crates").join("operant-config")) {
            continue;
        }
        // Integration tests are test code by definition.
        if path.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let stripped = strip_test_items(&text);
        if stripped.contains("load_or_init") {
            offenders.push(format!(
                "{}",
                path.strip_prefix(&root).unwrap_or(&path).display()
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "the parallel config schema is being loaded in production by: {offenders:?}. \
         The live config is operant_core::config::AppConfig. If this change is \
         deliberate (adopting the schema surface as the config spine — see \
         docs/plans/plan-2026-10-21-config-schema-retirement.md), update this \
         test in the same commit and close RT-141."
    );
}
