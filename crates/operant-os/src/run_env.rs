//! The run-environment contract (audit O-01/O-02), re-exported from
//! `operant_core::platform` — the single definition sits in core because
//! the agent loop and the cron tool read it there. These tests pin the
//! env-var semantics.

pub use operant_core::platform::{holon_employee, holon_run_dir, is_holon_run};

/// R-09 (AUDIT-2026-10-16-capability-layer): the run-local,
/// department-resolved skills directory the kernel names in the prepared
/// run's brief.json ("skills" — `runs/<id>/skills`, written at claim).
/// `None` = not a Holon run, no brief, or a kernel that predates the field:
/// callers then list no skills for the run (the brief is the render),
/// never the org-wide standard export with a stale twin beside every
/// department fork.
pub fn holon_skills_dir() -> Option<std::path::PathBuf> {
    let dir = holon_run_dir()?;
    let brief = std::fs::read_to_string(dir.join("brief.json")).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&brief).ok()?;
    let named = parsed.get("skills")?.as_str()?;
    let path = std::path::PathBuf::from(named);
    (!path.as_os_str().is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // env-var tests run in one process: use a serial-style guard via a
    // static mutex, matching the repo's `serial_test` idiom where needed.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn skills_dir_follows_the_brief_and_absent_means_none_r09() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        std::fs::create_dir_all(&run).unwrap();

        // no brief at all → None (not a Holon run's run dir)
        // SAFETY: serialized test, cleaned below.
        unsafe { std::env::set_var("HOLON_RUN_DIR", &run) };
        assert_eq!(holon_skills_dir(), None);

        // a brief naming no skills (kernel older than the field) → None
        std::fs::write(
            run.join("brief.json"),
            "{\"run_id\": \"r1\", \"stage\": \"plan\"}",
        )
        .unwrap();
        assert_eq!(holon_skills_dir(), None);

        // the field: the department-resolved run library, absolute
        std::fs::write(
            run.join("brief.json"),
            "{\"run_id\": \"r1\", \"skills\": \"/runs/r1/skills\"}",
        )
        .unwrap();
        assert_eq!(holon_skills_dir(), Some(PathBuf::from("/runs/r1/skills")));

        // an empty string is a missing field, not a skill root
        std::fs::write(
            run.join("brief.json"),
            "{\"run_id\": \"r1\", \"skills\": \"\"}",
        )
        .unwrap();
        assert_eq!(holon_skills_dir(), None);
        unsafe { std::env::remove_var("HOLON_RUN_DIR") };
    }

    #[test]
    fn run_dir_prefers_the_canonical_name() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: serialized test, cleaned below.
        unsafe {
            std::env::set_var("HOLON_RUN_DIR", "/canonical");
            std::env::set_var("HOLON_BRIEF_DIR", "/alias");
        }
        assert_eq!(holon_run_dir(), Some(PathBuf::from("/canonical")));
        unsafe {
            std::env::remove_var("HOLON_RUN_DIR");
        }
        assert_eq!(holon_run_dir(), Some(PathBuf::from("/alias")));
        unsafe {
            std::env::remove_var("HOLON_BRIEF_DIR");
        }
        assert_eq!(holon_run_dir(), None);
    }

    #[test]
    fn employee_prefers_the_canonical_name() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: serialized test, cleaned below.
        unsafe {
            std::env::set_var("HOLON_EMPLOYEE_ID", "emp_real");
            std::env::set_var("HOLON_EMPLOYEE", "emp_legacy");
        }
        assert_eq!(holon_employee().as_deref(), Some("emp_real"));
        unsafe {
            std::env::remove_var("HOLON_EMPLOYEE_ID");
        }
        assert_eq!(holon_employee().as_deref(), Some("emp_legacy"));
        unsafe {
            std::env::remove_var("HOLON_EMPLOYEE");
        }
        assert_eq!(holon_employee(), None);
    }
}
