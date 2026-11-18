#![allow(clippy::expect_used, clippy::unwrap_used)]

//! `ui_request_input` reaches a real host in the TUI AND in the gateway, and
//! every host that registers a sender must also DRAIN it.
//!
//! This guard started as the mirror of a wrong comment. `SURFACE_REPLY_TIMEOUT`
//! (300 s) claimed to be an outer backstop "because the host's own dialog has
//! its own timeout (the gateway's receiver runs on the 120s question timeout,
//! `clarify`'s rule)". There was no such receiver: exactly one production caller
//! registered one, `tui/adapter_types/tui_app.rs`, so on a gateway deployment
//! `try_send_surface_request` returned `None` and `ui_request_input` failed fast
//! with "no interactive surface is attached" on EVERY turn -- the tool looked
//! shipped while being unreachable from the transport most operators use. That
//! is the iter-98 `clarify` bug (gateway never called set_user_question_sender,
//! so clarify hung forever) one layer up.
//!
//! iter-2640 registered the gateway's receiver, which makes the 120 s claim
//! true and this a general invariant rather than a TUI-only asymmetry: a host
//! that registers without draining is a turn that waits on an answer nobody
//! will ever send. The test now checks each registration has a `recv()` beside
//! it, so the next host cannot repeat the pattern.

use operant_core::OperantTool;
use std::path::{Path, PathBuf};

/// The workspace root (this crate lives at `<root>/crates/operant-core`).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root is two levels above operant-core")
        .to_path_buf()
}

/// Every `.rs` file under `root`, relative to `root`.
fn rust_files_under(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Build output and vendored trees carry the same call.
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(name, "target" | ".git" | "node_modules" | "vendor") {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

/// The source with every `#[cfg(test)]` module's body blanked out.
///
/// Line numbers are preserved (each stripped line becomes an empty line) so a
/// reported site can still be found in the real file.
fn without_test_modules(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut out: Vec<&str> = lines.clone();
    let mut idx = 0usize;

    while idx < lines.len() {
        if lines[idx].trim() != "#[cfg(test)]" {
            idx += 1;
            continue;
        }
        // The module body starts at the next `{`; brace-count from there so a
        // nested brace inside a test cannot end the strip early.
        out[idx] = "";
        let mut depth = 0usize;
        let mut opened = false;
        let mut cursor = idx + 1;
        while cursor < lines.len() {
            for ch in lines[cursor].chars() {
                if ch == '{' {
                    depth += 1;
                    opened = true;
                } else if ch == '}' {
                    depth = depth.saturating_sub(1);
                }
            }
            out[cursor] = "";
            cursor += 1;
            if opened && depth == 0 {
                break;
            }
        }
        idx = cursor.max(idx + 1);
    }

    out.join("\n")
}

#[test]
fn every_surface_request_sender_is_also_drained_by_its_host() {
    let root = workspace_root();
    let crates = root.join("crates");
    let mut registrants = Vec::new();

    for path in rust_files_under(&crates) {
        let rel = path.strip_prefix(&crates).expect("walked under crates/");
        // This file is the guard: its own registration call is part of the
        // check, not a production site.
        if rel.ends_with("tests/surface_host_receivers.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("a crate source file is readable");
        // Strip every `#[cfg(test)]` module. A test that registers the sender
        // proves nothing about which HOST does: `ui_tool.rs`'s and
        // `request.rs`'s own unit tests both call the setter and would
        // otherwise read as production sites.
        let source = without_test_modules(&source);

        for (lineno, line) in source.lines().enumerate() {
            let line = line.trim();
            if !line.contains("set_surface_request_sender") {
                continue;
            }
            // A definition is not a registration.
            if line.starts_with("pub fn ") || line.starts_with("fn ") {
                continue;
            }
            registrants.push(format!("{}:{lineno}", rel.display()));
        }
    }

    let tui_only: Vec<&String> = registrants
        .iter()
        .filter(|site| site.starts_with("operant-cli/src/tui/"))
        .collect();

    assert!(
        !tui_only.is_empty(),
        "the TUI must register a surface receiver, or `ui_request_input` degrades \
         to an error even in the interactive session it was built for. Found: {registrants:?}"
    );

    // iter-2640: the gateway NOW registers one. `ui_request_input` used to fail
    // on every gateway turn with "no interactive surface is attached" while
    // looking shipped, because `set_surface_request_sender` had a single caller
    // and it was the TUI's. That was the iter-98 `clarify` bug one layer up.
    //
    // So the invariant is no longer "the TUI only". It is: every registered host
    // must ALSO drain what it registers, or the tool blocks on a channel nobody
    // reads. This guard therefore pairs each registration site with a receiver
    // in the same file, and fails when a host registers without one.
    let gateway = registrants
        .iter()
        .find(|site| site.starts_with("operant-cli/src/gateway_runner.rs"));
    assert!(
        gateway.is_some(),
        "the gateway must register a surface receiver, or `ui_request_input` degrades \
         to an error on every gateway turn. Found: {registrants:?}"
    );

    // The companion half: a registration with no `recv()` beside it is a turn
    // that waits on an answer nobody will ever send.
    let gateway_source =
        std::fs::read_to_string(workspace_root().join("crates/operant-cli/src/gateway_runner.rs"))
            .expect("the gateway runner is readable");
    assert!(
        gateway_source.contains("surface_rx.recv()"),
        "the gateway registers a surface sender, so it must also drain the \
         receiver (`surface_rx.recv()`); otherwise a card blocks its turn until \
         the timeout."
    );
}

/// The behavioural half of the hostless guard, asserted where it is
/// determinable.
///
/// With no host registered the tool must FAIL FAST, not sit on the 300 s
/// `SURFACE_REPLY_TIMEOUT` await: an integration test that waited even two
/// seconds would be a false alarm none of the hosts' code controls. It runs
/// only when no sibling test in this binary already claimed the process-global
/// `OnceLock`, and it returns `Ok(())` in that case rather than passing
/// silently -- it is a guard, not a coverage claim.
#[test]
fn the_tool_returns_a_hostless_error_when_no_host_is_registered() {
    if operant_core::ui::request::has_surface_request_sender() {
        // A sibling test registered the global sender; this binary cannot
        // observe the hostless path any more.
        return;
    }

    let tool = operant_core::tools::UiRequestInputTool;
    let runtime = tokio::runtime::Runtime::new().expect("a test runtime");
    let result = runtime.block_on(tool.execute(
        serde_json::json!({
            "surface_id": "hostless-guard",
            "kind": "choice",
            "question": "Pick one",
            "choices": [{"id": "a", "label": "A"}],
        }),
        // A TUI transport on purpose. Since iter-2640 the capability gate runs
        // BEFORE the host check, so a context with no transport is `text_only`
        // and refuses the `choice_picker` for being unrenderable -- a correct
        // refusal, but a different one. Declaring the transport isolates the
        // hostless path this test is about.
        operant_core::tools::ToolContext::default().with_metadata("transport", "tui"),
    ));

    assert!(
        !result.success,
        "the hostless path must be a FAILED call, not a silent success: the model has to
         know it has to ask in plain text. Got: {result:?}"
    );
    let error = result
        .error
        .as_deref()
        .expect("a failed tool result names why");
    assert!(
        error.contains("no interactive surface"),
        "the failure must tell the model to use plain text or `clarify`, not just say
         something broke. Got: {error}"
    );
}
