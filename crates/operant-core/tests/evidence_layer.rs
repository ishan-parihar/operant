//! End-to-end acceptance for the evidence layer.
//!
//! These are the six properties the layer promised in its design plan
//! (`docs/plans/plan-2026-11-12-rtk-token-efficient-tool-loop-integration.md`).
//! Every one is a pass/fail condition, so the layer can be falsified rather
//! than trusted.

use operant_core::agent::evidence::{EvidenceConfig, ShellMode, Subject, compress};

fn on() -> EvidenceConfig {
    EvidenceConfig {
        enabled: true,
        min_size: 1,
        shell: ShellMode::Native,
        ..EvidenceConfig::default()
    }
}

fn off() -> EvidenceConfig {
    EvidenceConfig::default()
}

/// A payload large enough that the layer will act on it.
fn lines(n: usize) -> String {
    (0..n)
        .map(|i| format!("compiling item {i} of {n}\n"))
        .collect()
}

#[test]
fn the_layer_does_nothing_until_the_operator_turns_it_on() {
    let text = lines(900);
    let out = compress("terminal", &text, &off());
    assert!(!out.changed(), "the default must be a no-op");
    assert_eq!(out.text, text);
}

#[test]
fn a_web_payload_shrinks_losing_nothing_the_model_reads() {
    // A realistic page: a lot of markup and boilerplate around a little prose.
    let mut html = String::from("<html><body><!-- a tracking pixel -->\n");
    for i in 0..300 {
        html.push_str(&format!(
            "<div class=\"row row-{i}\">   item {i}   </div>\n"
        ));
    }
    html.push_str("<!-- a newsletter footer -->\n</body></html>");
    let out = compress("web_fetch", &html, &on());
    assert!(out.changed(), "expected the web chain to fire");
    assert!(out.tokens_after < out.tokens_before);
    assert!(
        out.text.contains("item 299"),
        "the real content must survive: {}",
        out.text
    );
}

#[test]
fn a_match_list_groups_by_file() {
    let mut text = String::new();
    for i in 0..120 {
        text.push_str(&format!("src/alpha.rs:{i}:fn alpha_{i}() {{}}\n"));
        text.push_str(&format!("src/beta.rs:{i}:fn beta_{i}() {{}}\n"));
    }
    let out = compress("aft_grep", &text, &on());
    assert!(out.changed(), "expected grep-group to fire");
    assert!(
        out.text.contains("src/alpha.rs (120 matches)"),
        "got {}",
        &out.text[..out.text.len().min(200)]
    );
    assert!(out.text.contains("src/beta.rs (120 matches)"));
}

#[test]
fn a_json_api_result_encodes_as_toon() {
    let items: Vec<serde_json::Value> = (0..200)
        .map(|i| {
            serde_json::json!({
                "id": i,
                "name": format!("record-{i}"),
                "state": if i % 2 == 0 { "open" } else { "closed" },
            })
        })
        .collect();
    let text = serde_json::to_string(&items).expect("serialises");
    let out = compress("http_request", &text, &on());
    assert!(out.changed(), "expected toon to fire");
    assert!(
        out.text.contains("[200]{id,name,state}:"),
        "got {}",
        &out.text[..out.text.len().min(120)]
    );
}

#[test]
fn compression_never_makes_a_payload_bigger() {
    let cases: Vec<(&str, String)> = vec![
        ("terminal", lines(900)),
        ("file_read", lines(2000)),
        ("web_fetch", lines(2000)),
        (
            "http_request",
            serde_json::to_string(
                &(0..500)
                    .map(|i| serde_json::json!({ "i": i, "name": format!("row-{i}") }))
                    .collect::<Vec<serde_json::Value>>(),
            )
            .expect("serialises"),
        ),
    ];
    for (tool, payload) in &cases {
        let out = compress(tool, payload, &on());
        assert!(
            out.text.len() <= payload.len(),
            "{tool}: the result grew from {} to {} bytes",
            payload.len(),
            out.text.len()
        );
    }
}

#[test]
fn a_tool_result_envelope_survives_compression_intact() {
    // The terminal tool returns metadata the model reasons over; only the
    // payload may be reduced.
    let stdout = lines(900);
    let payload = serde_json::json!({
        "exit_code": 128,
        "stdout": stdout,
        "stderr": "fatal: not a git repository\n",
        "runtime": "local",
        "hint": "not a git repository (or any of the parent directories)\n",
    });
    let text = serde_json::to_string(&payload).expect("serialises");
    let out = compress("terminal", &text, &on());
    assert!(out.changed(), "expected compression");
    let parsed: serde_json::Value = serde_json::from_str(&out.text).expect("still valid JSON");
    assert_eq!(parsed["exit_code"], 128, "exit_code must survive");
    assert_eq!(parsed["runtime"], "local", "runtime must survive");
    assert_eq!(parsed["hint"], payload["hint"], "the hint must survive");
    assert!(
        parsed["stderr"]
            .as_str()
            .expect("string")
            .contains("not a git repository"),
        "stderr must survive"
    );
    assert!(
        parsed["stdout"].as_str().expect("string").len() < stdout.len(),
        "the payload must shrink"
    );
}

#[test]
fn an_image_payload_is_never_compressed() {
    // The existing spend guard exempts inline images; the layer must not
    // second-guess it.
    let mut payload = String::from("{\"mime\":\"image/png\",\"data\":\"data:image/png;base64,");
    payload.push_str(&"iVBORw0KGgo=".repeat(4000));
    payload.push_str("\"}");
    let out = compress("image_generate", &payload, &on());
    assert!(
        !out.changed(),
        "an image payload must be left alone, got {}",
        &out.text[..out.text.len().min(80)]
    );
}

#[test]
fn every_result_reports_what_produced_it() {
    let out = compress("aft_read", &lines(2000), &on());
    assert_eq!(out.subject, Subject::Read);
}

#[test]
fn a_short_result_is_left_alone() {
    let out = compress("terminal", "done\n", &on());
    assert!(!out.changed());
    assert_eq!(out.text, "done\n");
}
