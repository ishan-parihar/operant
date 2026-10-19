// app/channel_drains.rs — the nine background-channel drains the run loop
// performs every frame, one named method each.
//
// They used to be nine anonymous `if let Some(ref mut rx) = … { match rx.try_recv() { … } }`
// blocks inline in `App::run`, interleaved with the work that *spawns* the work
// they collect (the two `session_*_pending` spawns, the `/mcp r` dispatch) and
// with `notifications.tick()`. Naming them makes the loop readable, and — the
// reason this file exists as its own module rather than as nine private methods
// on `App` — makes the ORDER they are called in visible in one place.
//
// ── Order is load-bearing. Do not re-sort. ────────────────────────────────
//
// `App::run` calls them in this order:
//
//   1. session_list        2. model_fetch         3. user_questions
//   4. mcp_reconnect       5. agent_events        6. permission_requests
//   7. run_complete
//
// and the sequence is not alphabetical or numeric for a reason:
//
//   * `agent_events` (7) must run before `run_complete` (9). The turn's final
//     `AgentEvent` deltas and the `run_complete` oneshot are produced by the
//     same task; draining the event queue first means the transcript shows the
//     last content/tool events *before* `is_streaming` is cleared and the
//     completion path runs. Reversing the pair drops the final frame's text
//     into the already-idle footer.
//   * `permission_requests` (8) sits between them because it opens a modal the
//     agent is blocked on, and it reads `permission_request.is_some()` as its
//     own re-entrancy guard — a drain that ran before `agent_events` would see
//     a half-applied turn.
//   * `mcp_reconnect` (5) is where it is because it is the tick drain for the
//     `/mcp r` dispatch immediately above it (iter-326): the dispatch arms the
//     channel, and this is the first frame that can read it. Moving it earlier
//     would drain a channel the dispatch has not written yet.
//
// Each method returns `true` when it changed something the next frame will
// render. That is the signal `App::redraw_reason` reads, so "changed" has to
// mean "visible", not merely "assigned": clearing a dead receiver back to
// `None` mutates state but paints nothing, and reporting it as a repaint would
// make the reason table lie about which state forces a frame.

use super::*;

impl App {
    /// 5. Drain MCP reconnect status messages from the background reconnect
    ///    task. Runs on EVERY frame (tick drain) so a completion message renders
    ///    the moment it is posted — no keystroke required. The last message wins;
    ///    the channel is drained to empty so a burst of updates collapses to the
    ///    final state. (iter-326 — tick-based status drain.)
    ///    (plan 11B) Drain the shared binary updater's once-per-session notice
    ///    into the status line — "sourcehound updated 1.6.0 → 1.7.0" or the
    ///    failure reason, visible instead of debug-only.
    pub(super) fn drain_update_notice(&mut self) -> bool {
        use operant_core::binary_updater::UpdateNotice;
        let Some(notice) = operant_core::binary_updater::take_pending_notice() else {
            return false;
        };
        match notice {
            UpdateNotice::Updated { name, from, to } => {
                self.set_status_notice(format!("{name} updated {from} → {to}"));
            }
            UpdateNotice::Available { name, to } => {
                self.set_status_notice(format!(
                    "{name} {to} available (no asset for this platform)"
                ));
            }
            UpdateNotice::Failed { name, reason } => {
                self.set_status_notice(format!("{name} update failed: {reason}"));
            }
        }
        true
    }

    pub(super) fn drain_mcp_reconnect_status(&mut self) -> bool {
        let Some(ref mut rx) = self.mcp_reconnect_rx else {
            return false;
        };
        let mut changed = false;
        let mut msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
            changed = true;
        }
        // The drain borrows self via rx; apply the last message AFTER the
        // borrow ends — the channel is drained to empty, last wins (iter-326
        // semantics kept).
        if let Some(msg) = msgs.pop() {
            self.set_status_notice(msg);
        }
        changed
    }

    /// Opens the Ctrl+B branch browser once the fork tree has loaded. A
    /// database error is shown in the transcript and the browser stays closed.
    pub(super) fn drain_branch_list(&mut self) -> bool {
        let Some(ref mut rx) = self.branch_list_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(branches)) => {
                self.branch_list_rx = None;
                let at = self.session_branching.branch_at_message;
                self.session_branching.open(branches, at);
                true
            }
            Ok(Err(e)) => {
                self.branch_list_rx = None;
                self.push_system_message(
                    format!("Could not load this session's branches: {e}"),
                    SystemMessageStyle::Error,
                );
                true
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                self.branch_list_rx = None;
                false
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => false,
        }
    }

    /// 1. Drain background session-list results into the `/resume` browser.
    pub(super) fn drain_session_list(&mut self) -> bool {
        let Some(ref mut rx) = self.session_list_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(entries) => {
                self.debug_hub
                    .publish(crate::tui::debug::TuiEvent::SessionList {
                        count: entries.len(),
                        at: crate::tui::debug::event_bus::now_secs(),
                    });
                self.session_browser.sessions = entries;
                self.session_browser.selected_idx = 0;
                self.session_list_rx = None;
                if std::mem::take(&mut self.rename_on_list_load) {
                    self.session_browser.start_rename();
                }
                true
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // Dropping the dead receiver paints nothing.
                self.session_list_rx = None;
                false
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => false,
        }
    }

    /// Apply a connected provider's default model once its live list resolves. A
    /// dropped sender means the catalog was empty: say so, the model stays as it was.
    pub(super) fn drain_default_model(&mut self) -> bool {
        let Some(ref mut rx) = self.default_model_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(model) => {
                self.default_model_rx = None;
                self.pending_model = Some(model);
                true
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => false,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.default_model_rx = None;
                self.set_status_notice(
                    "No model list for this provider. Choose one with /model.".to_string(),
                );
                true
            }
        }
    }

    /// 2. Drain background model-fetch results into the model picker.
    pub(super) fn drain_model_fetch(&mut self) -> bool {
        let Some(ref mut rx) = self.model_fetch_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(models)) => {
                self.debug_hub
                    .publish(crate::tui::debug::TuiEvent::ModelFetch {
                        ok: true,
                        count: models.len(),
                        at: crate::tui::debug::event_bus::now_secs(),
                    });
                // (plan 05B) the fetch refreshes the INLINE picker's rows
                // in place (configured rows stay, dedup by name, current
                // row re-flagged); the legacy overlay is retired.
                if let Some(picker) = self.inline_interactive.as_mut() {
                    for m in models {
                        if !picker.entries.iter().any(|e| e.name == m.name) {
                            picker.entries.push(m);
                        }
                    }
                    let current = self.config.agent.model.clone();
                    for e in &mut picker.entries {
                        e.is_current = e.name == current;
                    }
                    picker.filter.clear();
                    crate::tui::app::App::apply_inline_interactive_filter(picker);
                }
                self.model_fetch_rx = None;
                self.model_picker_fetch_pending = false;
                true
            }
            Ok(Err(reason)) => {
                self.model_fetch_rx = None;
                self.model_picker_fetch_pending = false;
                self.set_status_notice(format!(
                    "Could not list models ({reason}). Showing configured models only."
                ));
                true
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                self.model_fetch_rx = None;
                self.model_picker_fetch_pending = false;
                self.status_message =
                    Some("Model fetch task disconnected unexpectedly.".to_string());
                true
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => false,
        }
    }

    /// 4. Drain user-question requests from the clarify tool. The
    ///    clarify tool pushes a UserQuestionRequest (question, choices
    ///    and reply_tx) and blocks awaiting the reply. We open the
    ///    ask_user_dialog with the reply_tx — when the user confirms,
    ///    the dialog sends the answer via reply_tx, and the clarify
    ///    tool receives it and returns it as the tool result.
    ///    (iter-97 — closes Bug #2 from iter-82 audit. The sender side
    ///    is wired in TuiApp::run via set_user_question_sender.)
    ///
    /// At most one request per frame (if-let, not while-let) so a burst of
    /// questions cannot starve the render loop.
    pub(super) fn drain_user_questions(&mut self) -> bool {
        let Some(ref mut rx) = self.user_question_rx else {
            return false;
        };
        if let Ok(req) = rx.try_recv() {
            self.debug_hub
                .publish(crate::tui::debug::TuiEvent::UserQuestion {
                    question_preview: req.question.chars().take(40).collect(),
                    at: crate::tui::debug::event_bus::now_secs(),
                });
            // Open the ask_user_dialog with the real reply_tx.
            // The dialog stores it and sends the user's answer when
            // confirm() is called. If the user presses Esc, the
            // dialog is dismissed and reply_tx is dropped — the
            // clarify tool receives a RecvError and returns
            // "[user dismissed the question]".
            self.ask_user_dialog
                .open(req.question, req.choices, req.reply_tx);
            true
        } else {
            false
        }
    }

    /// 7. Drain query events from the agent bridge task.
    pub(super) fn drain_agent_events(&mut self) -> bool {
        let mut events = Vec::new();
        if let Some(rx) = &mut self.agent_event_rx {
            // (plan 01) one-event-per-frame mode: the scripted events drive
            // the sim one loop pass at a time so --frame-per-event captures
            // the true frame BETWEEN every pair of events (the mid-stream
            // state the duplicate-row oracles need).
            if self.is_simulating && self.simulation_one_event_per_frame {
                if let Ok(ev) = rx.try_recv() {
                    events.push(ev);
                }
            } else {
                while let Ok(ev) = rx.try_recv() {
                    events.push(ev);
                }
            }
        }
        let drained = !events.is_empty();
        for ev in events {
            self.handle_agent_event(ev);
        }
        drained
    }

    /// 8. Drain pending tool permission requests from the agent. Each
    ///    request is converted into a `PermissionRequest` dialog and shown
    ///    to the user; the per-request `response_tx` is stashed in
    ///    `pending_permission_response_tx` so the user's choice can be
    ///    routed back when the dialog is dismissed. If a dialog is already
    ///    active (rare — the agent blocks on each request), the new
    ///    request is denied to avoid deadlock.
    ///
    /// Returns `true` only when a dialog was actually opened: a request
    /// auto-approved or auto-denied above paints nothing.
    pub(super) fn drain_permission_requests(&mut self) -> bool {
        // Take the receiver out for the loop (plan 03 §3): the loop body
        // calls methods on `self` (set_status_notice, open_permission_dialog),
        // which a live `ref mut` borrow of the permission_rx FIELD would
        // conflict with (E0499). No early return inside the loop, so the
        // receiver is always restored below.
        let mut rx = match self.permission_rx.take() {
            Some(rx) => rx,
            None => return false,
        };
        let mut opened = false;
        while let Ok(req) = rx.try_recv() {
            self.debug_hub
                .publish(crate::tui::debug::TuiEvent::PermissionRequest {
                    tool_name: req.tool_name.clone(),
                    at: crate::tui::debug::event_bus::now_secs(),
                });
            // (plan 03 §3) The mode the user picked IS the permission
            // decision — the drain consults it for every request. Before
            // this, `settings.permission_mode` had zero readers on the
            // decision path: BypassPermissions still prompted and Plan
            // still let gated tools run. The core's hardline/blocked checks
            // run BEFORE the channel (agent/stream.rs), so bypass can never
            // widen a hardline block — it only skips the prompt.
            use crate::tui::adapter_types::config::PermissionMode;
            match self.settings.permission_mode {
                PermissionMode::BypassPermissions => {
                    let _ = req
                        .response_tx
                        .send(operant_core::agent::ToolPermissionResponse::AllowSession);
                    continue;
                }
                PermissionMode::Plan => {
                    let _ = req
                        .response_tx
                        .send(operant_core::agent::ToolPermissionResponse::Deny);
                    self.set_status_notice(
                        "Plan mode is read-only — tool denied. /permissions to change.".to_string(),
                    );
                    continue;
                }
                _ => {}
            }
            if self.permission_request.is_some() {
                // A dialog is already shown — QUEUE the request (plan 03
                // §3). Tools run up to 8 in parallel (stream.rs), so one
                // batch can carry two gated tools; the old auto-deny
                // silently killed every request but the first.
                self.pending_permission_queue.push_back(req);
                continue;
            }

            // bash_prefix_allowlist: a plain command whose two-token prefix is
            // in the allowlist auto-approves without a dialog. "Always allow"
            // in the dialog populates it (maybe_record_bash_prefix). This is
            // the read side, and it shares plain_command_prefix with the
            // dialog's suggestion, so the two cannot disagree.
            if SHELL_TOOLS.contains(&req.tool_name.as_str())
                && let Some(entry) = req.input_preview.as_deref().and_then(plain_command_prefix)
            {
                // A two-token entry covers the command's two-token prefix ("git
                // status" covers "git status --short"); a legacy one-token entry
                // covers the first token.
                let first_token = entry.split_whitespace().next().unwrap_or_default();
                if self.bash_prefix_allowlist.contains(&entry)
                    || self.bash_prefix_allowlist.contains(first_token)
                {
                    let _ = req
                        .response_tx
                        .send(operant_core::agent::ToolPermissionResponse::AllowSession);
                    continue;
                }
            }

            self.open_permission_dialog(req);
            opened = true;
        }
        self.permission_rx = Some(rx);
        opened
    }

    /// Open the permission dialog for `req` (plan 03 §3 extracted — the
    /// drain and the queue-pop share one path). Caller guarantees no
    /// dialog is currently open.
    pub(super) fn open_permission_dialog(
        &mut self,
        req: operant_core::agent::ToolPermissionRequest,
    ) {
        let reason = if req.danger_explanation.is_empty() {
            req.description.clone()
        } else {
            format!("{}\n{}", req.description, req.danger_explanation)
        };
        let dialog = match req.input_preview {
            Some(command) if SHELL_TOOLS.contains(&req.tool_name.as_str()) => {
                let suggested_prefix = plain_command_prefix(&command);
                crate::tui::dialogs::PermissionRequest::bash(
                    req.tool_name,
                    reason,
                    command,
                    suggested_prefix,
                )
            }
            Some(command) if req.tool_name == "powershell" => {
                crate::tui::dialogs::PermissionRequest::powershell(req.tool_name, reason, command)
            }
            input_preview => crate::tui::dialogs::PermissionRequest::from_reason(
                req.tool_name,
                reason,
                input_preview,
            ),
        };
        self.permission_request = Some(dialog);
        self.pending_permission_response_tx = Some(req.response_tx);
        // The turn is blocked on the user's answer, not on the
        // model — the status line says so.
        self.turn_state = TurnState::WaitingForApproval;
    }

    /// (plan 03 §3) Called when a permission dialog closes: open the next
    /// queued request, if any. Returns true if a new dialog opened.
    pub(super) fn open_next_permission_request(&mut self) -> bool {
        if self.permission_request.is_some() {
            return false;
        }
        match self.pending_permission_queue.pop_front() {
            Some(req) => {
                self.open_permission_dialog(req);
                true
            }
            None => false,
        }
    }

    /// 9. Check if background agent.run() completed.
    pub(super) fn drain_run_complete(&mut self) -> bool {
        // (plan 01) one-event-per-frame mode: the mock's completion is
        // pre-resolved, but scripted events may still be buffered — re-arm
        // so the loop paints one frame per remaining event and only lets
        // is_streaming flip when the channel is empty.
        if self.is_simulating
            && self.simulation_one_event_per_frame
            && self
                .agent_event_rx
                .as_ref()
                .is_some_and(|rx| !rx.is_empty())
        {
            if let Some(rx) = &mut self.run_complete_rx
                && rx.try_recv().is_ok()
            {
                let (tx, fresh) = tokio::sync::oneshot::channel();
                let _ = tx.send(Ok(()));
                self.run_complete_rx = Some(fresh);
            }
            return true;
        }
        let Some(rx) = &mut self.run_complete_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.is_streaming = false;
                // (plan 04) a graceful cancel is success-with-no-error-card:
                // the tail was already committed and "Cancelled." is on
                // the status line; an error card would read as failure.
                let interrupted = matches!(result, Err(operant_core::error::Error::Interrupted));
                if let Err(e) = result
                    && !self.run_error_surfaced
                    && !interrupted
                {
                    self.handle_agent_event(AgentEvent::Error {
                        error: e.user_message(),
                    });
                }
                self.run_error_surfaced = false;
                self.run_complete_rx = None;
                self.agent_task_handle.take(); // Clear completed handle.
                self.cancel_deadline = None; // graceful return beat the watchdog
                true
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.is_streaming = false;
                self.run_error_surfaced = false;
                self.run_complete_rx = None;
                self.agent_task_handle.take(); // Clear completed handle.
                // (plan 04) the task ended without the oneshot resolving —
                // the watchdog aborted it (a tool ignored cancellation) or
                // it panicked. Either way the store needs the same repair;
                // route it through the pending-action channel so the
                // agent handle (if any) is in scope.
                if self.cancel_deadline.take().is_some() {
                    self.pending_repair = true;
                }
                true
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => false,
        }
    }
}

/// Tools whose requests are judged by their command text.
pub(super) const SHELL_TOOLS: [&str; 3] = ["bash", "shell", "terminal"];

/// The allowlist entry a plain shell command stores: its first two tokens, so
/// "git status" covers "git status --short" but not "git push --force".
/// `None` when the command carries a shell metacharacter (an injection tail)
/// or a shape the core's danger classifier flags. The read side and the
/// "Allow commands matching …*" option both come through here, so a stored
/// prefix can never approve a command the option would not have offered.
pub(super) fn plain_command_prefix(command: &str) -> Option<String> {
    let has_meta = [";", "&&", "||", "|", "\\", "$(", ">", "<", "\n"]
        .iter()
        .any(|m| command.contains(m));
    let dangerous =
        operant_core::approval::ApprovalGuard::new(operant_core::approval::ApprovalMode::Smart)
            .check_dangerous(command)
            .is_some();
    if has_meta || dangerous {
        return None;
    }
    let entry = command
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    (!entry.is_empty()).then_some(entry)
}
