#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The operant home has exactly ONE resolver.
//!
//! `operant_core::platform::operant_home()` is it: `HERMES_HOME`, then the
//! sticky active profile, then `~/.operant`. Every store under the operant
//! home — the database, the config, `settings.json`, `auth.json`, the
//! `.env` pins, the logs, the caches — must resolve through it.
//!
//! The defect this guard exists for: a store that built its own path from
//! the OS home (`dirs::home_dir().join(".operant")`) landed in the DEFAULT
//! profile's tree even when `HERMES_HOME` or an active profile pointed
//! somewhere else. `operant profile use work` wrote a sticky file that
//! nothing read, and every store that ignored `HERMES_HOME` made the
//! profile a lie.
//!
//! The guard scans every non-test Rust file in the workspace and fails when
//! one resolves an operant-home path from the OS home. The point is not to
//! forbid the pattern — `~/.operant` IS the default home — but to make
//! every SECOND spelling of it deliberate: whoever adds one has to update
//! this test in the same commit, so the two never drift.

use std::path::{Path, PathBuf};

/// The workspace root (this crate lives at `<root>/crates/operant-core`).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/operant-core is two levels under the workspace root")
        .to_path_buf()
}

/// Remove `#[cfg(test)]`-attributed items (braces included) so a path built
/// in a test module is not counted as a production one.
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

/// Files that may build an operant-home-shaped path from the OS home, and
/// why each one is NOT a second resolver.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/operant-core/src/platform.rs",
        "the resolver itself: `~/.operant` is the default home",
    ),
    (
        "crates/operant-core/src/credential_files.rs",
        "the OS home is used for `~` expansion and the credential-pattern \
         list; the operant data dir comes from the resolver",
    ),
    (
        "crates/operant-core/src/context_references.rs",
        "the OS home is used for `~` expansion and the sensitive-path \
         check — not an operant store",
    ),
    (
        "crates/operant-core/src/cookies.rs",
        "browser cookie databases live under the OS home, not the operant home",
    ),
    (
        "crates/operant-cli/src/tui/prompt_input/typeahead.rs",
        "`~` expansion in the path typeahead",
    ),
    (
        "crates/operant-cli/src/tui/memory_file_selector.rs",
        "the PROJECT-level `.operant/AGENTS.md`, not the operant home",
    ),
    (
        "crates/operant-cli/src/tui/agents_view.rs",
        "the PROJECT-level `.operant/agents`, not the operant home",
    ),
    (
        "crates/operant-cli/src/cmd_setup.rs",
        "`~/.cache/kokoros` — the model cache, not the operant home",
    ),
    (
        "crates/operant-providers/src/lib.rs",
        "the provider state-dir fallback; the production path passes the \
         loaded config's parent. This crate cannot depend on operant-core",
    ),
    (
        "crates/operant-providers/src/openai_codex.rs",
        "same provider state-dir fallback as operant-providers/src/lib.rs",
    ),
    (
        "crates/operant-config/src/platform/native.rs",
        "the DEAD parallel runtime adapter — retired with the dead schema \
         surface (RT-141)",
    ),
    (
        "crates/operant-config/src/schema/config_impl.rs",
        "the DEAD schema surface — retired with RT-141",
    ),
    (
        "crates/operant-config/src/schema/helpers.rs",
        "the DEAD schema surface — retired with RT-141",
    ),
];

/// The window (in lines) around a `".operant"` literal that may hold the
/// OS-home lookup that produced it.
const WINDOW: usize = 10;

#[test]
fn the_operant_home_has_one_resolver() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut offenders: Vec<String> = Vec::new();

    for path in files {
        // Integration tests are test code by definition.
        if path.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if ALLOWED.iter().any(|(allowed, _)| *allowed == relative) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let stripped = strip_test_items(&text);
        let lines: Vec<&str> = stripped.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("\".operant\"") {
                continue;
            }
            let start = index.saturating_sub(WINDOW);
            let end = (index + WINDOW + 1).min(lines.len());
            let window = lines[start..end].join("\n");
            if window.contains("home_dir") {
                offenders.push(format!("{relative}:{}", index + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these files build an operant-home path from the OS home instead of \
         the one resolver (operant_core::platform::operant_home), so they \
         ignore HERMES_HOME and the active profile: {offenders:?}. Use the \
         resolver; if a site genuinely needs the OS home, add it to the \
         ALLOWED list in this test with the reason."
    );
}
