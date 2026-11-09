//! RT-426: a write whose failure is discarded leaves the documented state
//! wrong, with nothing said.
//!
//! Three sites shared the shape: the org run's REFUSAL append (a policy
//! denial's audit trail), the org run's LEARNING annotation append (the run's
//! learning record), and the skills tool's ROLLBACK (which restores the
//! previous bytes after the install scan refuses a write). In each, the
//! failure was discarded — a refusal could vanish with no trace, and a
//! scan-refused skill file could stay on disk with the agent told only that
//! the scan refused it.
//!
//! The failures are IO-shaped (a full disk, a permission change), so no
//! deterministic fixture can force them at the write site. This contract
//! pins the repair's SHAPE in the source, and a behavioral test covers the
//! part that CAN be forced: the rollback's failure reaching the caller.

use std::path::Path;

/// The production region of a file (everything before its test modules).
fn production_region(file: &str) -> String {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file))
        .expect("the source file exists");
    match src.find("#[cfg(test)]") {
        Some(i) => src[..i].to_string(),
        None => src,
    }
}

#[test]
fn the_org_run_recorders_report_a_failed_append() {
    let prod = production_region("org/mod.rs");
    // Neither recorder may discard an append.
    assert_eq!(
        prod.matches("let _ = writeln!(").count(),
        0,
        "an append whose failure is discarded loses an audit record"
    );
    // Both must report, and each report must name the consequence.
    // Single-line fragments: a needle that spans the message's own line
    // break does not match (the campaign's recurring needle lesson).
    let refusal = concat!("not appear in result.json's refusals", "");
    assert_eq!(
        prod.matches(refusal).count(),
        2,
        "both refusal reports (open and write) must say the denial will not \
         reach result.json"
    );
    let learning = concat!("will not appear in the run's learning record", "");
    assert_eq!(
        prod.matches(learning).count(),
        2,
        "both learning reports (open and write) must say the annotation will \
         not reach the record"
    );
}

#[test]
fn the_skill_write_rollback_reports_its_failure() {
    let prod = production_region("tools/skills_tool.rs");
    // The rollback's result may not be discarded.
    assert_eq!(
        prod.matches("let _ = fs::write(path, bytes);").count()
            + prod.matches("let _ = fs::remove_file(path);").count()
            + prod
                .matches("let _ = fs::remove_dir_all(&skill_dir);")
                .count(),
        0,
        "the rollback's failure must not be discarded — a scan-refused file \\
         would stay on disk with nothing said"
    );
    // The refusal's message must name the refused-content consequence.
    let consequence = concat!("content is still on disk", "");
    assert_eq!(
        prod.matches(consequence).count(),
        1,
        "the refusal must say the refused content is still on disk when the \\
         rollback failed"
    );
}
