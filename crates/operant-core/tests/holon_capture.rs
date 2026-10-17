//! W6 acceptance (audit O-22, invariant 4): in a Holon run, learning
//! writes become annotations in the run directory — the operant home's
//! skills and memory store stay byte-identical. Drives the real tool
//! execute paths (skill_manage, memory_store) under the run env.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use serde_json::json;

#[tokio::test]
async fn a_holon_run_annotates_instead_of_writing_stores() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = dir.path().join("run");
    let home = dir.path().join("ohome");
    std::fs::create_dir_all(&run).expect("run dir");
    std::fs::create_dir_all(&home).expect("home dir");

    // SAFETY: serialized test (the env is the only variable).
    unsafe { std::env::set_var("HOLON_RUN_DIR", &run) };

    // skill_manage: the create lands as an annotation, no skill dir
    let skills_root = home.join("skills");
    let manage = operant_core::tools::skills_tool::SkillManageTool::new(skills_root.clone());
    let out = operant_core::tools::OperantTool::execute(
        &manage,
        json!({"action": "create", "name": "probe-skill", "content": "# probe"}),
        operant_core::tools::ToolContext::default(),
    )
    .await;
    assert!(
        out.content.contains("skill_changes"),
        "annotate reply: {}",
        out.content
    );
    assert!(
        !skills_root.join("probe-skill").exists(),
        "the operant home's skill store must stay untouched"
    );

    // memory_store: the fact lands as a learning, the provider untouched
    let store = operant_core::tools::memory_tools::MemoryStoreTool;
    let out = operant_core::tools::OperantTool::execute(
        &store,
        json!({"key": "probe", "content": "the organization's memory is Holon's"}),
        operant_core::tools::ToolContext::default(),
    )
    .await;
    assert!(
        out.content.contains("learnings"),
        "annotate reply: {}",
        out.content
    );

    // the annotations are on the record, parsed as JSON lines
    let sink = std::fs::read_to_string(run.join("learning.jsonl")).expect("learning.jsonl");
    let lines: Vec<serde_json::Value> = sink
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert_eq!(lines.len(), 2, "two annotations: {lines:?}");
    assert_eq!(lines[0]["kind"], "skill_changes");
    assert_eq!(lines[0]["payload"]["name"], "probe-skill");
    assert_eq!(lines[1]["kind"], "learnings");

    unsafe { std::env::remove_var("HOLON_RUN_DIR") };

    // personal mode: the same create WRITES the store — the gate must not
    // leak outside Holon runs.
    let out = operant_core::tools::OperantTool::execute(
        &manage,
        json!({"action": "create", "name": "personal-skill", "content": "---\nname: personal-skill\ndescription: probe\n---\n\n# personal"}),
        operant_core::tools::ToolContext::default(),
    )
    .await;
    assert!(
        skills_root.join("personal-skill").join("SKILL.md").exists(),
        "personal mode keeps writing skills: {}",
        out.content
    );
    let _ = Arc::clone(&Arc::new(())); // silence unused Arc import if any
}
