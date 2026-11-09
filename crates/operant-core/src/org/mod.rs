//! The org layer after the Holon supersession (docs/agentic-os/,
//! G+1 by owner order 2026-10-15).
//!
//! What remains is what the boundary gives operant:
//! - [`seat_policy`] — the pure policy engine; the enforcement core for
//!   Holon's `policy.json` (phase C);
//! - [`seat_policy_db`] — personal-mode operator seats; org mode never
//!   consults it (the run's `policy.json` is the policy);
//! - [`seat_budgets`] — budget enforcement and window math; caps come from
//!   `policy.json.budget` in org mode, the table is personal-mode only;
//! - [`seat_authority`] — personal mode's interactive escalation core;
//! - [`context_injection`] — channel-sourced aspects (DM, feed);
//!   `self`/`dept`/`global` sources retired to Holon's brief and memory;
//! - [`worklog`] — the `UsageAccumulator` (drained by the cron scheduler;
//!   RT-373: no `result.usage` seam exists, and there is no gateway usage
//!   meter reading it) and
//!   outcome signal types; the worklog store itself retired to Holon's
//!   `runs`/`outputs`/`events`.
//!
//! Everything else — the cast, registries, departments, hierarchy,
//! authority grants, notices, DM threads, decisions, the write barrier —
//! retired to Holon (ORG-LAYER-DISPOSITION.md §1). The org stores were
//! archived first (`operant org holon archive`); the data is history.

pub mod context_injection;
pub mod seat_authority;
pub mod seat_budgets;
pub mod seat_policy;
pub mod seat_policy_db;
pub mod worklog;

// ── Refusal recorder (audit O-17, W3) ─────────────────────────────────
//
// In a Holon run, a policy refusal is not an interactive ask: the
// disposition says it becomes `result.refusals[]`. The agent appends one
// JSON line per refused call into <run dir>/refusals.jsonl; the driver's
// end-of-run writer merges the lines into result.json (like usage). The
// log is a plain append — a crash loses at most the refusal in flight,
// never the run.

/// Appends one refusal record to `<run_dir>/refusals.jsonl`.
pub fn record_refusal(run_dir: &std::path::Path, tool: &str, reason: &str, rule: &str) {
    use std::io::Write;
    let line = serde_json::json!({
        "tool": tool, "reason": reason, "policy_rule": rule,
    });
    // RT-426: the append's failure was discarded, so a refusal could silently
    // never reach result.json's `refusals[]` — a policy denial with no audit
    // trail, and no way to tell. A refusal record still never kills the turn;
    // now it says so when it cannot be recorded.
    let mut f = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("refusals.jsonl"))
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                path = %run_dir.join("refusals.jsonl").display(),
                error = %e,
                "the refusal record could not be opened — this policy denial will \
                 not appear in result.json's refusals"
            );
            return;
        }
    };
    if let Err(e) = writeln!(f, "{line}") {
        tracing::warn!(
            path = %run_dir.join("refusals.jsonl").display(),
            error = %e,
            "the refusal record could not be written — this policy denial will \
             not appear in result.json's refusals"
        );
    }
}

/// Reads a run's recorded refusals (empty when none).
pub fn read_refusals(run_dir: &std::path::Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(run_dir.join("refusals.jsonl")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// O-18: the policy's `roots` for this run — the fence's allowlist.
///
/// None = UNGOVERNED: the run carries no policy.json, or the policy has no
/// `roots` key at all. Some(list) = FENCED: a write must land under one of
/// the listed roots or under the run directory itself.
///
/// RT-422: an empty `roots` array is NOT the ungoverned case — it yields a
/// fence of exactly the run directory (the narrowest fence there is, and
/// upstream's "the workdir only"). The fence therefore STAYS ACTIVE for an
/// empty list, and this note now says so.
pub fn holon_policy_roots(run_dir: &std::path::Path) -> Option<Vec<std::path::PathBuf>> {
    let policy = std::fs::read_to_string(run_dir.join("policy.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&policy).ok()?;
    let roots = doc.get("roots")?.as_array()?;
    let mut fenced: Vec<std::path::PathBuf> = roots
        .iter()
        .filter_map(|r| r.as_str())
        .map(std::path::PathBuf::from)
        .collect();
    // the run directory itself is always workable (the brief, the result)
    fenced.push(run_dir.to_path_buf());
    Some(fenced)
}

/// O-19: this run's token budget cap (policy.json `budget.tokens`), when
/// the policy sets one. None = ungoverned (no policy or no cap).
pub fn holon_budget_cap(run_dir: &std::path::Path) -> Option<u64> {
    let policy = std::fs::read_to_string(run_dir.join("policy.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&policy).ok()?;
    doc.pointer("/budget/tokens")?.as_u64()
}

/// O-18: the policy's terminal allowlist for this run — the kernel merges
/// the grade's `commands` and `act_commands` (act postures only) into one
/// sorted `commands` list at policy generation (holon execution/policy.py).
/// None = ungoverned (no policy, no `commands` key, or an empty list — an
/// empty allowlist upstream means ungoverned, not deny-all); Some(names) =
/// only these command names may run.
///
/// RT-422: the two fences do NOT share one convention. An empty command list
/// leaves THIS fence inert, while an empty `roots` list leaves the roots
/// fence ACTIVE (narrowed to the run directory). The old note claimed the
/// roots fence matched this one.
pub fn holon_policy_commands(run_dir: &std::path::Path) -> Option<Vec<String>> {
    let policy = std::fs::read_to_string(run_dir.join("policy.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&policy).ok()?;
    let commands = doc.get("commands")?.as_array()?;
    let names: Vec<String> = commands
        .iter()
        .filter_map(|c| c.as_str())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return None; // an empty allowlist upstream means ungoverned, not deny-all
    }
    Some(names)
}

/// W6 (audit O-22, invariant 4): in a Holon run, learning writes become
/// annotations in the run directory — never files in the operant home.
/// One JSON line per annotation, keyed by kind: `skill_changes`,
/// `learnings`, `proposals` — the driver's end-of-run writer merges them
/// into result.json the way refusals merge (kernel H-03 reads them as
/// the run's learning record).
pub fn record_learning_annotation(
    run_dir: &std::path::Path,
    kind: &str,
    payload: serde_json::Value,
) {
    use std::io::Write;
    let line = serde_json::json!({ "kind": kind, "payload": payload });
    // RT-426: same shape as the refusal writer — a failed append left the
    // run's learning record silently incomplete, and the end-of-run writer
    // merges these lines into result.json.
    let mut f = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("learning.jsonl"))
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                path = %run_dir.join("learning.jsonl").display(),
                error = %e,
                "the learning annotation could not be opened — this annotation \
                 will not appear in the run's learning record"
            );
            return;
        }
    };
    if let Err(e) = writeln!(f, "{line}") {
        tracing::warn!(
            path = %run_dir.join("learning.jsonl").display(),
            error = %e,
            "the learning annotation could not be written — this annotation \
                 will not appear in the run's learning record"
        );
    }
}

/// W6: whether this process is a Holon run AND its annotations dir.
pub fn holon_learning_sink() -> Option<std::path::PathBuf> {
    crate::platform::holon_run_dir()
}

/// W6: a run's recorded learning annotations (empty when none).
pub fn read_learning_annotations(run_dir: &std::path::Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(run_dir.join("learning.jsonl")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
mod rt422_tests {
    use super::*;

    /// A run directory carrying `policy.json` with the given body.
    fn run_dir_with_policy(body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant-org-rt422-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create run dir");
        std::fs::write(dir.join("policy.json"), body).expect("write policy");
        dir
    }

    /// RT-422: the roots fence's CONVENTION — an empty `roots` list is the
    /// NARROWEST fence (the run directory alone), not the ungoverned case.
    /// BEHAVIORAL: the policy file is a real temp file, so the fence's
    /// behavior is measured, not read off a comment.
    #[test]
    fn an_empty_roots_list_fences_to_the_run_directory() {
        let dir = run_dir_with_policy(r#"{"roots": []}"#);
        let roots = holon_policy_roots(&dir)
            .expect("an empty roots list still yields a fence — it is not ungoverned");
        assert_eq!(
            roots,
            vec![dir.clone()],
            "the fence must be exactly the run directory"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RT-422: and the ungoverned case is the one with NO policy at all.
    #[test]
    fn a_run_without_policy_is_ungoverned() {
        let dir = std::env::temp_dir().join(format!(
            "operant-org-rt422-none-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create run dir");
        assert!(
            holon_policy_roots(&dir).is_none(),
            "no policy means no fence"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RT-422: the two fences do NOT share one convention, and the notes say
    /// so. SOURCE contract: the needles are concatenation-built, and the
    /// assertions read the notes' own claims rather than the code's.
    #[test]
    fn the_notes_state_the_two_fences_have_different_conventions() {
        let src = include_str!("mod.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        let empty_is_fenced = concat!("an empty `roots` array is NOT the ungoverned", " case");
        assert_eq!(
            prod.matches(empty_is_fenced).count(),
            1,
            "the roots note must state that an empty list still fences"
        );
        let not_matching = concat!("the two fences do NOT share one", " convention");
        assert_eq!(
            prod.matches(not_matching).count(),
            1,
            "the commands note must state that the conventions differ"
        );
    }
}
