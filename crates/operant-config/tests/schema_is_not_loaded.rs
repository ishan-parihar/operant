#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The parallel config aggregate must not come back.
//!
//! `operant_config::schema::Config` was a SECOND config surface: an
//! aggregate of ~30 sections with its own loader (`Config::load_or_init`),
//! its own `config.toml` resolution and its own migration. Nothing loaded
//! it in production — only this crate's own tests did — so it was retired
//! (RT-141, iter-1138). The live config is `operant_core::config::AppConfig`.
//!
//! This guard scans every non-test Rust file in the workspace and fails if
//! one of them brings the aggregate back: a `load_or_init` call, or a
//! `schema::Config` type in use. The point is not to forbid the change —
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
fn the_parallel_config_aggregate_is_not_loaded_in_production() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut offenders: Vec<String> = Vec::new();

    for path in files {
        // The crate that OWNED the schema is where the surface lived — the
        // section types the live config composes are still here, and this
        // guard is about the aggregate coming back.
        if path.starts_with(root.join("crates").join("operant-config")) {
            continue;
        }
        // Integration tests are test code by definition.
        if path.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let stripped = strip_test_items(&text);
        if stripped.contains("load_or_init") || stripped.contains("schema::Config") {
            offenders.push(format!(
                "{}",
                path.strip_prefix(&root).unwrap_or(&path).display()
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "the retired config aggregate is being used in production by: {offenders:?}. \
         The live config is operant_core::config::AppConfig. If this change is \
         deliberate (adopting the schema surface as the config spine — see \
         docs/plans/plan-2026-10-21-config-schema-retirement.md), update this \
         test in the same commit and close RT-141."
    );
}

#[test]
fn the_aggregate_type_is_gone() {
    // The guard above catches USE; this one catches the type being
    // RE-INTRODUCED. A doc comment mentioning it is fine — the aggregate's
    // own history is worth recording — but a definition is not.
    let schema = workspace_root()
        .join("crates")
        .join("operant-config")
        .join("src")
        .join("schema");
    let mut files = Vec::new();
    rust_files(&schema, &mut files);
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let stripped = strip_test_items(&text);
        assert!(
            !stripped.contains("pub struct Config {"),
            "{} defines the retired aggregate again — it was never loaded in \
             production (RT-141)",
            path.display()
        );
    }
}
