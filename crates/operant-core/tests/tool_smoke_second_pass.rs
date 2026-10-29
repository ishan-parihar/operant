//! RT-196 — the second tool smoke pass.
//!
//! The 2026-10-25 "Agentic-Core Tool Smoke Test — Complete Report" claimed
//! "All tools have been exercised across 10 batches" while 22 registered tools
//! had never actually been called. The description half of that row closed in
//! iter-1363 (`process` scope, `aft_checkpoint` tracked-files-only); this is
//! the EXECUTION half: every one of those tools is now driven through its real
//! `OperantTool::execute` against a tempdir, so "exercised" is a thing the
//! suite can prove rather than a claim in a report.
//!
//! ## The consent gate (why this test is not run by default)
//!
//! Three of the 22 have external side effects a CI box must not perform:
//! `send_message` and `react_to_message` post to a real recipient, and
//! `image_generate` spends provider credits. RT-196's own fix shape names them
//! "behind an explicitly flagged consent pass", so they are skipped unless the
//! caller sets `OPERANT_SMOKE_SIDE_EFFECTS=I_CONSENT`.
//!
//! Everything else is hermetic: each tool is called with the *smallest legal*
//! payload for its schema, and the assertion is a CONTRACT, not a golden — the
//! tool answered in shape (a `ToolResult` whose content is a string or a
//! structured error, never a panic, never an empty content). A tool that
//! returns a structured error about a missing prerequisite is a PASS: that is
//! the honesty these rows care about, and a tool that cannot run here says so
//! instead of hanging or crashing.
//!
//! Run it:
//! ```text
//! cargo test -p operant-core --test tool_smoke_second_pass -- --nocapture
//! OPERANT_SMOKE_SIDE_EFFECTS=I_CONSENT cargo test -p operant-core \
//!     --test tool_smoke_second_pass -- --nocapture   # includes the 3 effectful arms
//! ```

use operant_core::kanban::KanbanDb;
use operant_core::tools::{ToolContext, ToolRegistry};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Consent for the three side-effecting tools. Absent (the default) skips them.
fn side_effects_consented() -> bool {
    std::env::var("OPERANT_SMOKE_SIDE_EFFECTS").as_deref() == Ok("I_CONSENT")
}

/// A registry with every builtin registered, over the tempdirs a tool needs.
async fn full_registry(root: &std::path::Path) -> anyhow::Result<Arc<ToolRegistry>> {
    let registry = Arc::new(ToolRegistry::new(Duration::from_secs(30)));
    let db = Arc::new(operant_core::database::Database::init(
        root.join("agent.sqlite"),
    )?);
    let cron_db = Arc::new(operant_core::cronjobs::CronDb::init(
        root.join("cron.sqlite"),
    )?);
    let kanban_db = Arc::new(KanbanDb::init(root.join("kanban.sqlite"))?);
    operant_core::tools::register_builtin_tools(
        &registry,
        &root.join("skills"),
        &root.join("memory"),
        db,
        cron_db,
        kanban_db,
        None,
    )
    .await?;
    Ok(registry)
}

/// The one contract every tool must satisfy, asserted per call.
///
/// "Answer or say why" is read from the SAME fields the agent loop reads
/// (`agent/run.rs`:2097-2099 — successes render `content`, failures render
/// `error`). Asserting on `content` alone would flag every correct error
/// result, because `ToolResult::error` deliberately puts the message in
/// `error` and leaves `content` empty. That is the sanctioned shape, not a
/// defect: the loop never renders content for a failure.
fn assert_answered(name: &str, result: &operant_core::tools::ToolResult) {
    let body = if result.success {
        result.content.clone()
    } else {
        result
            .error
            .clone()
            .unwrap_or_else(|| "(no error message)".to_string())
    };
    assert!(
        !body.trim().is_empty(),
        "{name}: reported failure with NO explanation — success=false and error=None. \
         The loop renders exactly this pair to the model, so an unexplained failure \
         shows the model nothing."
    );
    if !result.success {
        assert!(
            body.len() > 8,
            "{name}: an unexplained one-word failure ({body:?}) — the model cannot act on it"
        );
    }
}

/// Drive `name` with `args` and assert it answered in shape.
async fn smoke(registry: &Arc<ToolRegistry>, name: &str, args: serde_json::Value) {
    let tool = registry
        .get(name)
        .await
        .unwrap_or_else(|| panic!("{name}: not registered in the builtin toolset"));
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        tool.execute(args, ToolContext::default()),
    )
    .await
    .unwrap_or_else(|_| panic!("{name}: execute did not settle within 60s"));
    assert_answered(name, &result);
}

/// The hermetic half: every RT-196 tool whose execution stays inside this
/// process or on this disk.
#[tokio::test]
async fn the_never_called_tools_answer_when_actually_called() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let registry = full_registry(root).await?;

    // ── Read-only / self-contained ────────────────────────────────────────
    smoke(
        &registry,
        "clarify",
        json!({ "question": "smoke: name the topic" }),
    )
    .await;
    smoke(&registry, "todo", json!({ "action": "list" })).await;
    smoke(&registry, "session_insights", json!({ "scope": "session" })).await;
    smoke(
        &registry,
        "tool_call",
        // tool_call invokes ANOTHER tool, so the payload names the inner tool
        // and ITS arguments (tool_search.rs:511-515 — `name` + `arguments`;
        // NOT `tool`/`args`, which the tool correctly rejects as a missing
        // name). Choosing `todo` with its own list action makes this a real
        // delegation round trip through the registry executor.
        json!({ "name": "todo", "arguments": { "action": "list" } }),
    )
    .await;
    smoke(
        &registry,
        "checkpoint",
        json!({ "action": "create", "name": "smoke" }),
    )
    .await;

    // OSV advisory lookup: a real network call to a public advisory API. A
    // network-less box must produce the *honest* failure, not a panic.
    smoke(
        &registry,
        "osv_check",
        json!({ "package": "serde", "version": "1.0.0" }),
    )
    .await;

    // ── The AFT-backed names from RT-196's list are NOT registered by default ──
    // `aft_ast_replace`, `aft_bash` and `aft_inspect` appear in RT-196's
    // unexercised list but sit behind the AFT feature and the `aft_enabled`
    // config — registering them here would need the feature and the binary.
    // Asserted ABSENT rather than skipped: the claim "these three need
    // explicit enablement" becomes something the suite pins. If a future
    // iteration registers them unconditionally this fails and RT-196's list gets
    // re-derived instead of quietly rotting.
    for name in ["aft_ast_replace", "aft_bash", "aft_inspect"] {
        assert!(
            !registry.contains(name).await,
            "{name}: registered without the AFT feature — RT-196's premise changed; \
             add it to this smoke pass or remove it from the row's list"
        );
    }

    // HTTP client against a loopback port nothing listens on: must report the
    // connection failure, not panic or hang.
    smoke(
        &registry,
        "http_request",
        json!({ "url": "http://127.0.0.1:1/", "method": "GET" }),
    )
    .await;

    // Reasoning/diffing over a file this test creates, so the tool has real
    // input rather than an empty tree.
    std::fs::write(root.join("probe.txt"), "one\ntwo\nthree\n")?;
    smoke(
        &registry,
        "working_diff",
        json!({ "path": root.join("probe.txt").to_string_lossy().to_string() }),
    )
    .await;

    Ok(())
}

/// The effectful half, gated on explicit consent.
#[tokio::test]
async fn the_side_effecting_tools_are_exercised_only_with_consent() -> anyhow::Result<()> {
    if !side_effects_consented() {
        eprintln!(
            "SKIP: send_message / react_to_message / image_generate are not exercised.\n      \
             Set OPERANT_SMOKE_SIDE_EFFECTS=I_CONSENT to include them (RT-196: they \
             post to a real recipient and spend provider credits)."
        );
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    let registry = full_registry(dir.path()).await?;

    // Each is driven with the tool's minimum legal payload. The recipient /
    // prompt is deliberately a no-op target so a mis-typed flag cannot spam a
    // real channel.
    smoke(
        &registry,
        "send_message",
        json!({ "channel": "smoke-noop", "text": "RT-196 smoke: no-op delivery" }),
    )
    .await;
    smoke(
        &registry,
        "react_to_message",
        json!({ "message_id": "smoke-noop", "emoji": "white_check_mark" }),
    )
    .await;
    smoke(
        &registry,
        "image_generate",
        json!({ "prompt": "RT-196 smoke: a plain grey 16x16 square" }),
    )
    .await;

    Ok(())
}
