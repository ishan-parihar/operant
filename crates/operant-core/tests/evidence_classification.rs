//! The coverage test that keeps the classifier and the router honest.
//!
//! The classifier's table is the only thing that stops a tool result from
//! being compressed with the wrong chain, and the only thing that keeps an
//! unclassified result from reaching the chain at all. So the table must cover
//! every tool the registry serves — and this file reads the LIVE names, so a
//! tool added tomorrow fails here until someone classifies it.
//!
//! The router half is the same discipline pointed at the payload: the fields
//! that carry the result must be routed, and the metadata must not be.

use operant_core::agent::evidence::classify::classify;
use operant_core::agent::evidence::route::route;
use operant_core::agent::evidence::types::{ContentFormat, Subject};
use operant_core::tools::builtin::builtin_tool_names;
use std::path::Path;

/// The AFT tools, in registration order.
const AFT_TOOLS: &[&str] = &[
    "aft_read",
    "aft_write",
    "aft_edit",
    "aft_apply_patch",
    "aft_bash",
    "aft_search",
    "aft_outline",
    "aft_zoom",
    "aft_inspect",
    "aft_callers",
    "aft_grep",
    "aft_glob",
    "aft_ast_search",
    "aft_ast_replace",
    "aft_checkpoint",
    "aft_list_checkpoints",
    "aft_checkpoint_paths",
    "aft_restore_checkpoint",
    "aft_undo",
    "aft_status",
];

/// Every `aft_*` name the AFT module mentions, read from the source.
///
/// The point of the slice: a tool the classifier does not classify must fail
/// HERE, not silently fall through to a guess. Reading the module rather than
/// a hand-copied list means a newly added AFT tool is caught the day it lands.
fn aft_names_from_source() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/aft_tools.rs");
    let source = std::fs::read_to_string(&path).expect("aft_tools.rs is readable");
    let mut names = Vec::new();
    let mut rest = source.as_str();
    while let Some(start) = rest.find("\"aft_") {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        let name = &after[..end];
        if name.len() > "aft_".len()
            && name
                .chars()
                .all(|char| char.is_ascii_lowercase() || char == '_')
        {
            names.push(name.to_string());
        }
        rest = &after[end..];
    }
    names.sort();
    names.dedup();
    names
}

// ── registry coverage ────────────────────────────────────────────────────

/// Every advertised builtin tool — the sourcehound composites included —
/// classifies to `Some`. This is the assertion that stops the table rotting.
#[test]
fn every_registered_builtin_tool_classifies() {
    let names = builtin_tool_names();
    assert!(!names.is_empty(), "the registry must advertise its tools");
    for name in names {
        assert!(
            classify(name, "").is_some(),
            "builtin tool `{name}` has no classification — the layer would leave its result alone"
        );
    }
}

/// Every AFT tool classifies to `Some`, both from the hand-written list and
/// from the module's own source.
#[test]
fn every_aft_tool_classifies() {
    let scanned = aft_names_from_source();

    for name in AFT_TOOLS {
        assert!(
            scanned.iter().any(|found| found.as_str() == *name),
            "AFT tool `{name}` is no longer in the module — update the list or delete the row"
        );
        assert!(
            classify(name, "").is_some(),
            "AFT tool `{name}` has no classification"
        );
    }

    // The half that catches a NEW tool: the module's own names, one by one.
    for name in &scanned {
        assert!(
            classify(name, "").is_some(),
            "AFT tool `{name}` (found in aft_tools.rs) has no classification — add it to the table"
        );
    }
}

/// The registry's MCP namespace classifies as an MCP result.
#[test]
fn the_mcp_namespace_classifies_as_mcp() {
    for name in [
        // `mcp_<server>_<tool>` — the registry's own spelling.
        "mcp_github_create-5fissue",
        "mcp_linear_search_nodes",
        // `mcp__<server>__<tool>` — the double-underscore spelling.
        "mcp__github__create_issue",
        // A tool from a server whose tool name carries an escaped underscore.
        "mcp_operant_mcp_search",
    ] {
        let (subject, _) = classify(name, "{}").expect("an MCP tool classifies");
        assert_eq!(subject, Subject::Mcp, "`{name}` must be an MCP result");
    }
}

/// The builtin that shares the `mcp_` prefix is not an MCP tool.
#[test]
fn mcp_management_is_a_builtin_not_an_mcp_result() {
    let (subject, _) = classify("mcp_management", "{}").expect("classifies");
    assert_eq!(subject, Subject::ToolResult);
}

/// An unknown tool is left alone: `None`, not a guess.
#[test]
fn an_unknown_tool_is_left_alone() {
    assert!(
        classify("a_tool_nobody_registered", "{}").is_none(),
        "an unclassified tool must be left alone, not guessed at"
    );
    assert!(
        classify("grep_lite", "src/a.rs:1:hit").is_none(),
        "a plausible-looking name is still an unknown tool"
    );
}

// ── the classifications the layer depends on ─────────────────────────────

#[test]
fn the_frozen_classifications_hold() {
    assert_eq!(
        classify("aft_read", "fn main() {}"),
        Some((Subject::Read, ContentFormat::Code))
    );
    assert_eq!(
        classify("terminal", "cargo build --quiet"),
        Some((Subject::Shell, ContentFormat::Term))
    );
    assert_eq!(
        classify("aft_grep", "src/a.rs:1:hit\nsrc/b.rs:2:hit"),
        Some((Subject::Search, ContentFormat::Matches))
    );
    assert_eq!(
        classify("web_fetch", "<html><body>hi</body></html>"),
        Some((Subject::Web, ContentFormat::Web))
    );
    assert_eq!(
        classify("file_read", r#"{"name":"operant","version":"0.2.1"}"#),
        Some((Subject::Read, ContentFormat::Json))
    );
}

/// A read is source code, but the CONTENT wins when it is not source.
#[test]
fn a_read_of_a_non_source_file_is_classified_by_its_content() {
    // The file's own text, through the read envelope.
    assert_eq!(
        classify("file_read", r#"{"path":"x.json","content":"{\"a\":1}"}"#),
        Some((Subject::Read, ContentFormat::Json))
    );
    // A bare JSON body.
    assert_eq!(
        classify("file_read", r#"{"a": 1, "b": [2, 3]}"#),
        Some((Subject::Read, ContentFormat::Json))
    );
    // A lockfile, by name and by shape.
    assert_eq!(
        classify(
            "file_read",
            r#"{"path":"Cargo.lock","content":"[[package]]\nname = \"x\"\nversion = \"0.1.0\"\n"}"#
        ),
        Some((Subject::Read, ContentFormat::Lockfile))
    );
    assert_eq!(
        classify(
            "file_read",
            r#"{"path":"deps.toml","content":"[[package]]\nname = \"x\"\nversion = \"0.1.0\"\n"}"#
        ),
        Some((Subject::Read, ContentFormat::Lockfile))
    );
    // A markdown document.
    assert_eq!(
        classify(
            "file_read",
            "{\"path\":\"README.md\",\"content\":\"# Title\\n\\n- one\\n- two\\n\"}"
        ),
        Some((Subject::Read, ContentFormat::Web))
    );
    // A MANIFEST is not a lockfile, however much it looks like one.
    assert_eq!(
        classify(
            "file_read",
            "{\"path\":\"Cargo.toml\",\"content\":\"[package]\\nname = \\\"operant\\\"\\nversion = \\\"0.2.1\\\"\\n\"}"
        ),
        Some((Subject::Read, ContentFormat::Code))
    );
    // Source code, through the envelope.
    assert_eq!(
        classify(
            "aft_read",
            "{\"file\":\"src/main.rs\",\"content\":\"fn main() {}\",\"length\":13}"
        ),
        Some((Subject::Read, ContentFormat::Code))
    );
}

/// A web tool returns a page; an API response behind it is JSON.
#[test]
fn a_web_result_is_a_page_or_an_api_response() {
    assert_eq!(
        classify(
            "http_request",
            r#"{"status_code":200,"body":"{\"ok\":true}"}"#
        ),
        Some((Subject::Web, ContentFormat::Json))
    );
    assert_eq!(
        classify(
            "web_fetch",
            r#"{"url":"https://example.com","body":"<html>hi</html>"}"#
        ),
        Some((Subject::Web, ContentFormat::Web))
    );
    assert_eq!(
        classify("web_search", r#"{"query":"x","results":[]}"#),
        Some((Subject::Web, ContentFormat::Json))
    );
}

/// An MCP result is shaped by its content, never by the tool name.
#[test]
fn an_mcp_result_is_shaped_by_its_content() {
    assert_eq!(
        classify("mcp_index_query", r#"{"mode":"stats","more":{"a":1}}"#),
        Some((Subject::Mcp, ContentFormat::Json))
    );
    assert_eq!(
        classify("mcp__browser__snapshot", "<html><body>hi</body></html>"),
        Some((Subject::Mcp, ContentFormat::Web))
    );
    assert_eq!(
        classify("mcp_cloakctl_call", "src/a.rs:1:hit\nsrc/b.rs:2:hit"),
        Some((Subject::Mcp, ContentFormat::Matches))
    );
}

// ── the router ──────────────────────────────────────────────────────────

/// A terminal result's payload is `stdout`/`stderr`; the metadata is not.
#[test]
fn the_router_isolates_the_shell_payload_from_its_metadata() {
    let content = r#"{"exit_code":0,"stdout":"hello\n","stderr":"","runtime":"local"}"#;
    let plan = route("terminal", content, ContentFormat::Term);

    assert!(plan.is_json);
    assert_eq!(
        plan.segments.len(),
        1,
        "the shell payload is one segment, not the whole envelope"
    );
    let segment = &plan.segments[0];
    assert_eq!(segment.path, "stdout");
    assert_eq!(segment.format, ContentFormat::Term);
    assert_eq!(segment.text, "hello\n");

    for segment in &plan.segments {
        assert_ne!(segment.path, "exit_code", "status flags are metadata");
        assert_ne!(segment.path, "runtime", "runtime is metadata");
        assert_ne!(segment.path, "stderr", "an empty payload is not a payload");
    }
}

/// A JSON envelope with no recognised payload field is one whole segment.
#[test]
fn a_payload_with_no_recognised_field_is_one_whole_segment() {
    let content = r#"{"exit_code":0,"runtime":"local","truncated":false}"#;
    let plan = route("terminal", content, ContentFormat::Term);

    assert!(plan.is_json);
    assert_eq!(plan.segments.len(), 1);
    assert_eq!(plan.segments[0].path, "");
    assert_eq!(plan.segments[0].text, content);
    assert_eq!(plan.segments[0].format, ContentFormat::Term);
}

/// A JSON ARRAY is one segment, not a set of fields.
#[test]
fn a_json_array_is_one_segment() {
    let content = r#"[{"file":"a.rs"},{"file":"b.rs"}]"#;
    let plan = route("terminal", content, ContentFormat::Term);

    assert!(plan.is_json);
    assert_eq!(plan.segments.len(), 1);
    assert_eq!(plan.segments[0].path, "");
    assert_eq!(plan.segments[0].text, content);
}

/// A non-JSON payload is one segment with the given format.
#[test]
fn a_non_json_payload_is_one_segment() {
    let content = "line one\nline two\n";
    let plan = route("terminal", content, ContentFormat::Term);

    assert!(!plan.is_json);
    assert_eq!(plan.segments.len(), 1);
    assert_eq!(plan.segments[0].path, "");
    assert_eq!(plan.segments[0].text, content);
    assert_eq!(plan.segments[0].format, ContentFormat::Term);
}

/// Each family routes its own payload field.
#[test]
fn the_router_routes_the_payload_field_of_each_family() {
    // A match list.
    let matches = r#"{"pattern":"x","matches":["src/a.rs:1:hit"],"count":1}"#;
    let plan = route("aft_grep", matches, ContentFormat::Matches);
    assert_eq!(plan.segments.len(), 1);
    assert_eq!(plan.segments[0].path, "matches");
    assert_eq!(plan.segments[0].format, ContentFormat::Matches);

    // A web page body.
    let page = r#"{"url":"https://example.com","body":"<html></html>","status_code":200}"#;
    let plan = route("web_fetch", page, ContentFormat::Web);
    assert_eq!(plan.segments.len(), 1);
    assert_eq!(plan.segments[0].path, "body");
    assert_ne!(plan.segments[0].path, "status_code");

    // A file read.
    let read = r#"{"path":"a.rs","content":"fn main() {}","length":13}"#;
    let plan = route("file_read", read, ContentFormat::Code);
    assert_eq!(plan.segments.len(), 1);
    assert_eq!(plan.segments[0].path, "content");
    assert_eq!(plan.segments[0].format, ContentFormat::Code);
}

/// A match list of records is flattened into the shape the chain groups by.
#[test]
fn a_structured_match_list_is_flattened_into_match_lines() {
    let content = r#"{"matches":[{"file":"src/a.rs","line":1,"content":"let x = 1;"},{"file":"src/b.rs","line":9,"content":"let y = 2;"}],"count":2}"#;
    let plan = route("aft_grep", content, ContentFormat::Matches);
    assert_eq!(plan.segments.len(), 1);
    assert_eq!(plan.segments[0].format, ContentFormat::Matches);
    assert!(plan.segments[0].text.contains("src/a.rs:1:let x = 1;"));
    assert!(plan.segments[0].text.contains("src/b.rs:9:let y = 2;"));
}

/// A tool whose family has no payload keys compresses the whole document
/// rather than nothing.
#[test]
fn a_family_without_payload_keys_compresses_the_whole_document() {
    let content = r#"{"id":"abc","structuredContent":{"a":1},"isError":false}"#;
    let plan = route("mcp_index_query", content, ContentFormat::Json);

    assert!(plan.is_json);
    assert_eq!(plan.segments.len(), 1);
    assert_eq!(plan.segments[0].path, "");
    assert_eq!(plan.segments[0].text, content);
}
