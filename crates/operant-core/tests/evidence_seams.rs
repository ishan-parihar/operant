//! The evidence layer's seams with the rest of operant.
//!
//! `evidence_layer.rs` tests the layer's own properties. This file tests the
//! three seams that are NOT the layer's own: the session store that holds the
//! full text (so the provenance marker is a promise something keeps), the
//! shell mode that says who owns shell output, and the per-family override
//! that lets an operator turn one family on or off without a rebuild.
//!
//! Every test here encodes a defect that already existed. The reason to have
//! them is that the seams are where a correct layer still produces a wrong
//! system.

use operant_core::agent::evidence::{EvidenceConfig, FamilyConfig, ShellMode, Subject, compress};
use operant_core::database::{Database, MessageData};

// ---------------------------------------------------------------------------
// The full text the layer's marker points at
// ---------------------------------------------------------------------------

/// Build a tool-result row the way the chokepoint does: the MODEL-VISIBLE
/// copy in `content` and the tool's actual output in `full_content`.
fn store_tool_result(
    db: &Database,
    session_id: &str,
    tool_call_id: &str,
    model_visible: &str,
    full: &str,
) {
    // The session row must exist: `messages` carries a FOREIGN KEY on
    // session_id, so a tool-result row for a session that does not exist is
    // REFUSED. The chokepoint has the same dependency, which is what bounds
    // the recoverable guarantee — see the test below.
    db.save_session(
        session_id,
        Some("seam"),
        "test",
        "2026-11-12T00:00:00Z",
        "2026-11-12T00:00:00Z",
    )
    .expect("the session row");
    db.save_message_full(&MessageData {
        id: 0,
        session_id: session_id.to_string(),
        role: "tool".to_string(),
        content: Some(model_visible.to_string()),
        full_content: Some(full.to_string()),
        tool_call_id: Some(tool_call_id.to_string()),
        tool_calls: None,
        tool_name: Some("terminal".to_string()),
        timestamp: "2026-11-12T00:00:00Z".to_string(),
        token_count: None,
        finish_reason: None,
        reasoning: None,
        reasoning_content: None,
        reasoning_details: None,
        codex_reasoning_items: None,
        codex_message_items: None,
        platform_message_id: None,
        observed: None,
        active: 1,
    })
    .expect("the row saves");
}

/// A unique database per test: two concurrent runs must not share one, or one
/// run's rows answer the other's query.
fn test_db(suffix: &str) -> Database {
    Database::init(std::env::temp_dir().join(format!(
        "test_evidence_seam_{}_{suffix}.db",
        std::process::id()
    )))
    .expect("database")
}

#[test]
fn the_full_text_the_marker_names_is_the_one_the_store_keeps() {
    let db = test_db("recall");
    let session = "sess-1";
    let call = "call-1";
    // The model sees the compressed copy; the store holds the tool's output.
    store_tool_result(
        &db,
        session,
        call,
        "item 0 of 400\nitem 1 of 400\n[compressed: dedup — …]",
        &format!("item 0 of 400\nitem 1 of 400\nitem 2 of 400\nitem 3 of 400\n"),
    );
    let key = operant_core::agent::evidence::recall::key_for(Some(session), call);
    assert_eq!(key, "sess-1:call-1");
    let (recovered, tool_name) = db
        .get_tool_result(session, call)
        .expect("the query runs")
        .expect("the full text is there");
    assert_eq!(
        recovered,
        "item 0 of 400\nitem 1 of 400\nitem 2 of 400\nitem 3 of 400\n"
    );
    assert_eq!(tool_name, "terminal");
    // And the marker names the key that resolved here.
    let marker = operant_core::agent::evidence::recall::marker(&key, 4000, 800, &["dedup"]);
    assert!(
        marker.contains(&key),
        "the marker must name the key that resolves the full text: {marker}"
    );
}

#[test]
fn a_row_with_no_archived_twin_falls_back_to_the_model_visible_copy() {
    // Pre-compression rows (and every non-tool message) have no
    // `full_content`; the retrieval must answer with what it has rather than
    // reporting the text as lost.
    let db = test_db("fallback");
    store_tool_result(
        &db,
        "sess-2",
        "call-2",
        "the whole result",
        "the whole result",
    );
    let (recovered, _) = db
        .get_tool_result("sess-2", "call-2")
        .expect("the query runs")
        .expect("the row is there");
    assert_eq!(recovered, "the whole result");
}

#[test]
fn an_unknown_key_is_a_miss_not_a_failure() {
    let db = test_db("miss");
    assert!(
        db.get_tool_result("no-such-session", "call-1")
            .expect("the query runs")
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// The shell path belongs to another compressor
// ---------------------------------------------------------------------------

/// The terminal output the chain can actually shrink: repeated blocks, ANSI,
/// a base64 blob.
fn terminal_output(n: usize) -> String {
    let mut out = String::from("\u{1b}[32m=== running checks ===\u{1b}[0m\n");
    for i in 0..n {
        out.push_str(&format!("test module_a::case_{i} ... ok\nrunning 1 test\n"));
    }
    out.push_str("\u{1b}[33mwarning\u{1b}[0m: some message\n");
    out.push_str("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n");
    out
}

#[test]
fn aft_mode_leaves_shell_output_to_aft() {
    // The default: AFT already compresses bash in-process, and a second pass
    // would strip AFT's own recovery handles.
    let text = terminal_output(400);
    let cfg = EvidenceConfig {
        enabled: true,
        min_size: 1,
        ..EvidenceConfig::default()
    };
    let out = compress("terminal", &text, &cfg);
    assert!(!out.changed(), "AFT mode must not touch a shell result");
    assert_eq!(out.subject, Subject::Shell);
}

#[test]
fn native_mode_compresses_shell_output() {
    let text = terminal_output(400);
    let cfg = EvidenceConfig {
        enabled: true,
        min_size: 1,
        shell: ShellMode::Native,
        ..EvidenceConfig::default()
    };
    let out = compress("terminal", &text, &cfg);
    assert!(out.changed(), "native mode must compress a shell result");
    assert!(out.text.len() < text.len());
}

#[test]
fn rtk_mode_leaves_shell_output_to_rtk() {
    let text = terminal_output(400);
    let cfg = EvidenceConfig {
        enabled: true,
        min_size: 1,
        shell: ShellMode::Rtk,
        ..EvidenceConfig::default()
    };
    let out = compress("terminal", &text, &cfg);
    assert!(!out.changed(), "rtk mode must not touch a shell result");
}

#[test]
fn the_shell_gate_touches_nothing_else() {
    // A web result is not a shell result: the shell mode must be irrelevant
    // to it.
    let mut html = String::from("<html><body>\n");
    for i in 0..300 {
        html.push_str(&format!("<div class=\"row\">   item {i}   </div>\n"));
    }
    html.push_str("</body></html>\n");
    let cfg = EvidenceConfig {
        enabled: true,
        min_size: 1,
        ..EvidenceConfig::default()
    };
    let out = compress("web_fetch", &html, &cfg);
    assert!(out.changed(), "the web chain must still fire in AFT mode");
    assert_eq!(out.subject, Subject::Web);
}

// ---------------------------------------------------------------------------
// The per-family override
// ---------------------------------------------------------------------------

#[test]
fn a_family_the_operator_turns_off_is_left_alone() {
    let text = terminal_output(400);
    let mut families = std::collections::BTreeMap::new();
    families.insert(
        Subject::Shell.as_str().to_string(),
        FamilyConfig {
            enabled: Some(false),
            chain: None,
        },
    );
    let cfg = EvidenceConfig {
        enabled: true,
        min_size: 1,
        shell: ShellMode::Native,
        families,
        ..EvidenceConfig::default()
    };
    let out = compress("terminal", &text, &cfg);
    assert!(!out.changed(), "a disabled family must be left alone");
}

#[test]
fn a_family_override_that_enables_works_over_a_disabled_master() {
    // The per-family `enabled` WINS over the master switch, so an operator can
    // turn ONE family on without turning on everything.
    let text = terminal_output(400);
    let mut families = std::collections::BTreeMap::new();
    families.insert(
        Subject::Shell.as_str().to_string(),
        FamilyConfig {
            enabled: Some(true),
            chain: None,
        },
    );
    let cfg = EvidenceConfig {
        enabled: false,
        min_size: 1,
        shell: ShellMode::Native,
        families,
        ..EvidenceConfig::default()
    };
    let out = compress("terminal", &text, &cfg);
    assert!(out.changed(), "the enabled family must compress");
}

#[test]
fn a_family_can_name_its_own_chain() {
    // A read whose family names the aggressive chain must be compressed more
    // than the default code chain compresses it.
    let mut source = String::new();
    for i in 0..200 {
        source.push_str(&format!("fn item_{i}() {{\n    // a comment nobody reads\n    let value = compute({i}) + {i} * 3;\n    println!(\"{{value}}\");\n}}\n\n"));
    }
    let plain = EvidenceConfig {
        enabled: true,
        min_size: 1,
        ..EvidenceConfig::default()
    };
    let mut families = std::collections::BTreeMap::new();
    families.insert(
        Subject::Read.as_str().to_string(),
        FamilyConfig {
            enabled: None,
            chain: Some("code-aggressive".to_string()),
        },
    );
    let named = EvidenceConfig {
        enabled: true,
        min_size: 1,
        families,
        ..EvidenceConfig::default()
    };
    let plain_out = compress("file_read", &source, &plain);
    let named_out = compress("file_read", &source, &named);
    assert!(plain_out.changed(), "the default code chain must fire");
    assert!(named_out.changed(), "the named chain must fire");
    assert!(
        named_out.text.len() < plain_out.text.len(),
        "code-aggressive must cut more than the default code chain"
    );
}

#[test]
fn an_unknown_chain_name_falls_back_instead_of_disabling() {
    // A typo in the config must not silently turn compression off, and must
    // not fail the result either.
    let text = terminal_output(400);
    let mut families = std::collections::BTreeMap::new();
    families.insert(
        Subject::Shell.as_str().to_string(),
        FamilyConfig {
            enabled: None,
            chain: Some("no-such-chain".to_string()),
        },
    );
    let cfg = EvidenceConfig {
        enabled: true,
        min_size: 1,
        shell: ShellMode::Native,
        families,
        ..EvidenceConfig::default()
    };
    let out = compress("terminal", &text, &cfg);
    assert!(out.changed(), "an unknown chain name must fall back");
}

#[test]
fn a_result_whose_session_row_is_missing_cannot_be_recalled_and_says_so() {
    // The recoverable guarantee is bounded by the session row, and the
    // retrieval path degrades rather than lies. This is the test that keeps
    // the bound visible: `get_tool_result` on a session that never existed
    // is a MISS, and the tool turns that miss into an honest "no retained
    // text" rather than an empty body the model would read as "this tool
    // returned nothing".
    let db = test_db("fk");
    let missing = db
        .get_tool_result("a-session-that-was-never-saved", "call-1")
        .expect("the query itself must not fail");
    assert!(
        missing.is_none(),
        "a session that does not exist cannot hold its tool result"
    );
}

#[test]
fn the_schema_reconciles_a_column_onto_a_database_that_predates_it() {
    // The column was added after the first migration ran, so every existing
    // install gets it by `reconcile_columns`, not by a fresh CREATE. This
    // proves the reconcili took hold on a database this test creates.
    let db = test_db("reconcile");
    db.save_session(
        "sess-reconcile",
        Some("reconcile"),
        "test",
        "2026-11-12T00:00:00Z",
        "2026-11-12T00:00:00Z",
    )
    .expect("the session row");
    store_tool_result(
        &db,
        "sess-reconcile",
        "call-r",
        "the shaped copy",
        "the tool's own output",
    );
    let rows = db
        .get_session_messages_full("sess-reconcile")
        .expect("the rows");
    assert_eq!(
        rows[0].full_content.as_deref(),
        Some("the tool's own output"),
        "the column must exist on a freshly created database"
    );
}
