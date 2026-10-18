// Vendored from jcode (crates/operant-base/src/safety.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805 (batch-4
// sweep tail): only the file-backed permission-decision surface the TUI
// permissions overlay consumes — Urgency/PermissionRequest/PermissionResult/
// Decision plus record_permission_via_file and its queue/history helpers.
// The in-process SafetySystem (Mutex/OnceLock state machine) is NOT ported;
// nothing in the TUI layer reaches it.

// ---------------------------------------------------------------------------
// SafetySystem (operant-base/src/safety.rs:150-290) — bounded set: the
// constructor + the two methods the permissions TUI entry consumes.
// request_permission / record_decision / log_action / generate_summary stay
// unported (server-side surfaces, unreached from the TUI layer).
// ---------------------------------------------------------------------------
