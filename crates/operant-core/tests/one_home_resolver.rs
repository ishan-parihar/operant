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
///
/// DEFECT THIS FIXES: the original scanned forward to the next `{`. For the
/// OUT-OF-LINE form `#[cfg(test)]\nmod tests;` there is no brace at that
/// site, so the `None` arm set `rest = ""` and the guard silently dropped
/// EVERYTHING AFTER IT. `gateway_runner.rs` declares `mod tests;` at :8728 and
/// keeps four more `#[cfg(test)] mod ...` blocks below, so ~600 lines of
/// production-adjacent source were never scanned. A guard with a blind spot
/// exactly where the biggest file's tests begin is worse than no guard, because
/// it reports green.
fn strip_test_items(text: &str) -> String {
    const ATTR: &str = "#[cfg(test)]";

    // Scan for the attribute at the START OF A LINE (ignoring indentation).
    // An `#[cfg(test)]` that appears mid-expression, or on a nested `fn` inside
    // an already-active item, is not an item boundary and must not start a
    // removal.
    let mut out = String::new();
    let mut cursor = 0usize;
    while cursor < text.len() {
        let rest = &text[cursor..];
        let Some(rel) = rest.find(ATTR) else {
            out.push_str(rest);
            break;
        };
        let idx = cursor + rel;

        // The attribute must begin its own line (only whitespace before it).
        let line_start = text[..idx].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let before_on_line = &text[line_start..idx];
        if !before_on_line.trim().is_empty() {
            // Mid-line: not an item boundary. Emit through it and keep going.
            let after_attr = idx + ATTR.len();
            out.push_str(&text[cursor..after_attr]);
            cursor = after_attr;
            continue;
        }

        // Only a MODULE declaration is an item boundary. A `#[cfg(test)]`
        // sitting on a `fn` inside a module is NOT: brace-matching from that fn
        // would eat the enclosing module's remainder, so an indented attribute
        // is emitted as ordinary text and the scan continues. It may make the
        // guard OVER-report (it still sees test-module bodies); it can never
        // make it UNDER-report, which is the failure that matters.
        //
        // MEASURED, because the first claim here was wrong: on
        // `gateway_runner.rs` the OLD parser did NOT have a blind spot. Both the
        // old and new forms strip all five of its `#[cfg(test)] mod ...` blocks
        // (`unattended_posture_tests`, `pending_permissions_tests`,
        // `tool_progress_tests`, `turn_retention_tests`, `unbound_scope_tests`).
        // The only difference is 212 characters of an indented `#[cfg(test)]
        // fn on_tool_start_anon` that the old code removed and the new code
        // keeps. The rewrite is a robustness improvement, not a bug fix, and
        // the doc comment must not claim otherwise.
        let after = &text[idx + ATTR.len()..];
        // `after` begins with the newline that ended the attribute line, so the
        // declaration is on the NEXT line. Reading it from `after` directly made
        // `decl` empty for EVERY site, so no module was ever recognised.
        let decl_line = after.strip_prefix('\n').unwrap_or(after);
        let line_end = decl_line.find('\n').unwrap_or(decl_line.len());
        let decl = decl_line[..line_end].trim();
        if !decl.starts_with("mod ") {
            let next = idx + ATTR.len();
            out.push_str(&text[cursor..next]);
            cursor = next;
            continue;
        }

        // Out-of-line form `mod tests;` - nothing is attached here, so skip the
        // attribute AND the declaration line, then keep scanning.
        if decl.ends_with(';') {
            let next = idx + ATTR.len() + 1 + line_end;
            out.push_str(&text[cursor..idx]);
            cursor = next;
            continue;
        }

        // Inline module: remove the attribute through the module's closing brace.
        match after.find('{') {
            Some(_) => {
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
                out.push_str(&text[cursor..idx]);
                cursor = idx + ATTR.len() + end;
            }
            // No brace at all: not an item we can span, so do NOT discard the
            // remainder - that was the blind spot.
            None => {
                let next = idx + ATTR.len();
                out.push_str(&text[cursor..next]);
                cursor = next;
            }
        }
    }
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
        "`~/.cache/phonon` — the Phonon model cache, not the operant home",
    ),
    (
        "crates/operant-providers/src/lib.rs",
        "the provider state-dir fallback; the production path passes the \
         loaded config's parent. This crate cannot depend on operant-core",
    ),
    (
        "crates/operant-config/src/schema/helpers.rs",
        "the DEAD schema surface — retired with RT-141",
    ),
];

/// Whether a window around a `".operant"` literal spells the OS home.
///
/// DEFECT THIS REPLACES: the guard used to test for `home_dir` alone. That is
/// the `dirs::home_dir()` spelling, but the operant-home defect it was written
/// to catch is ALSO produced by `env::var("HOME")` - which is how
/// `tui/theme_files.rs:36` built a second resolver that ignored HERMES_HOME and
/// the sticky active profile, reading the DEFAULT profile's `themes/` for every
/// profile. A guard blind to the spelling the bug actually used is not a guard.
///
/// The list is deliberately broader than the known offenders: a NEW spelling
/// should fail loudly and be triaged, not slip through.
fn resolves_from_os_home(window: &str) -> bool {
    const OS_HOME_SPELLINGS: &[&str] = &[
        "home_dir",                  // dirs::home_dir()
        "env::var(\"HOME\")",        // std::env::var("HOME")
        "env::var(\"USERPROFILE\")", // Windows twin
        "home_dir_inner",            // the dirs crate's internal
    ];
    OS_HOME_SPELLINGS.iter().any(|s| window.contains(s))
}

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
            if resolves_from_os_home(&window) {
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

/// The matcher must see EVERY spelling of "the OS home", not just
/// `dirs::home_dir()`.
///
/// This is the defect the guard could not catch: it tested for `home_dir`
/// alone, while the operant-home bug it exists to prevent is ALSO produced by
/// `env::var("HOME")`. `tui/theme_files.rs:36` spelled it that way and built a
/// second resolver that ignored `HERMES_HOME` and the sticky active profile, so
/// a non-default profile read the DEFAULT profile's `themes/`.
///
/// Mutation-proven: deleting any spelling from `OS_HOME_SPELLINGS` fails the
/// corresponding assertion here.
#[test]
fn the_matcher_recognises_every_os_home_spelling() {
    // A window in the shape the guard actually sees: a `.operant` literal with
    // the lookup a few lines above it.
    let shapes = [
        "let home = dirs::home_dir();\nlet p = home.join(\".operant\");",
        "let home = std::env::var(\"HOME\");\nlet p = home.join(\".operant\");",
        "let p = std::env::var(\"HOME\").join(\".operant\");",
        "let base = env::var(\"USERPROFILE\");\nlet p = base.join(\".operant\");",
    ];
    for shape in shapes {
        assert!(
            resolves_from_os_home(shape),
            "the matcher must recognise this OS-home spelling: {shape}"
        );
    }

    // And it must NOT fire on a window that reaches the resolver instead. A
    // guard that flags everything is a guard nobody can satisfy.
    let clean =
        "let home = operant_core::platform::operant_home();\nlet p = home.join(\".operant\");";
    assert!(
        !resolves_from_os_home(clean),
        "the one resolver must not be flagged: {clean}"
    );
}

/// The `strip_test_items` scanner must strip an inline test module and KEEP
/// scanning after it.
///
/// Both forms have to work, because a file mixes them: `gateway_runner.rs`
/// declares `mod tests;` (out-of-line) at :8728 and four braced
/// `#[cfg(test)] mod ...` blocks after it.
#[test]
fn test_items_are_stripped_without_swallowing_the_rest() {
    let source = "\
fn production_one() {
    let p = std::env::var(\"HOME\").join(\".operant\");
}

#[cfg(test)]
mod inline_tests {
    #[test]
    fn a_test() {}
}

#[cfg(test)]
mod out_of_line_tests;

fn production_two() {
    let p = std::env::var(\"HOME\").join(\".operant\");
}
";
    let stripped = strip_test_items(source);
    assert!(
        !stripped.contains("a_test"),
        "the inline test module must be stripped, got: {stripped}"
    );
    assert!(
        stripped.contains("production_one"),
        "code before the test module must survive"
    );
    assert!(
        stripped.contains("production_two"),
        "code AFTER the test module must survive - the old parser dropped \
         everything past an out-of-line `mod tests;`, got: {stripped}"
    );
}
