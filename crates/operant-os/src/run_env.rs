//! The run-environment contract (audit O-01/O-02), re-exported from
//! `operant_core::platform` — the single definition sits in core because
//! the agent loop and the cron tool read it there. These tests pin the
//! env-var semantics.

pub use operant_core::platform::{holon_employee, holon_run_dir, is_holon_run};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // env-var tests run in one process: use a serial-style guard via a
    // static mutex, matching the repo's `serial_test` idiom where needed.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
