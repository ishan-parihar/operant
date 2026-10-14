//! Phase D (H4, operant side): the run-dir handoff contract — trajectory.json
//! in the kernel's shape, and the driver-measured usage merged into the
//! agent's result.json. The trajectory half has its pure unit in
//! operant-core (`trajectory::holon_trajectory_json`, tested there); these
//! tests pin the MERGE half: the envelope's own fields must survive, and the
//! kernel's coalescing contract (driver measurement wins) must hold shape.

use operant_core::trajectory::holon_trajectory_json;

#[test]
fn trajectory_shape_is_what_the_kernel_compacts() {
    // mirror of holon.execution.work.Work.compact_trajectory's acceptance:
    // {"steps": [dicts with string thought/tool/args/outcome]}, order preserved
    let mut t = operant_core::trajectory::Trajectory::new(
        "id".to_string(),
        "sess".to_string(),
        "m".to_string(),
    );
    t.steps.push(operant_core::trajectory::TrajectoryStep {
        step: 0,
        thought: Some("need the file".into()),
        action: Some("file_write".into()),
        action_args: Some("{\"path\":\"a\"}".into()),
        observation: Some("wrote 2 bytes".into()),
        response: None,
        success: true,
    });
    let v = holon_trajectory_json(&t);
    let steps = v["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["tool"], "file_write");
    assert_eq!(steps[0]["outcome"], "wrote 2 bytes");
    for key in ["thought", "tool", "args", "outcome"] {
        assert!(steps[0][key].is_string(), "field {key} is a string");
    }
}

#[test]
fn usage_merge_preserves_the_envelope_and_wins_at_the_kernel() {
    // the driver patches {"usage": {...}} into result.json; the kernel's
    // set_run_usage coalesces with the driver's measurement preferred
    let mut env: serde_json::Value = serde_json::from_str(
        r#"{"summary": "did the thing", "outcome": "success", "documentation": "d"}"#,
    )
    .unwrap();
    let obj = env.as_object_mut().unwrap();
    obj.insert(
        "usage".to_string(),
        serde_json::json!({"tokens_in": 100, "tokens_out": 40, "cost": null,
                           "tool_calls": 2, "duration_s": null}),
    );
    // the envelope's own fields survive the merge
    assert_eq!(env["summary"], "did the thing");
    assert_eq!(env["outcome"], "success");
    // usage is the driver's, all numbers or null per the kernel's contract
    assert_eq!(env["usage"]["tool_calls"], 2);
    assert_eq!(env["usage"]["tokens_in"], 100);
    assert!(env["usage"]["cost"].is_null());
}
