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
//! - [`worklog`] — the `UsageAccumulator` (feeds `result.usage`) and
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
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("refusals.jsonl"))
    else {
        return; // a refusal record never kills the turn
    };
    let _ = writeln!(f, "{line}");
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

/// O-18: the policy's `roots` for this run — None when the run carries no
/// policy.json or the policy lists no roots (an empty root list means
/// "the workdir only" upstream, but here None = ungoverned, Some = fenced).
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
/// None = ungoverned (no policy or the policy lists nothing — the fence
/// stays inert, matching the roots fence's convention); Some(names) = only
/// these command names may run.
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
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("learning.jsonl"))
    {
        let _ = writeln!(f, "{line}");
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
