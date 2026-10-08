// Vendored from jcode (crates/operant-tui/src/tui/app.rs + tui/app/helpers.rs +
// tui/app/state_ui_input_helpers.rs), MIT License, Copyright (c) 2025 Jeremy Huang.
// Ported verbatim @ 0a9dc7805; PARTIAL — only the state types/free fns/consts
// the ported renderers reference, per the established W1 seam pattern.
// The App struct itself, its impls, and every App-bound fn are NEVER ported:
// the cutover adapts operant's own App state onto these types. See
// operant_app/mod.rs for scope.
//! Included from app.rs: COMMAND_SUGGESTION_VISIBLE_LIMIT (:133),
//! ProcessingStatus (:436), RemoteStartupPhase (:454, plus impl),
//! CopyBadgeUiState (:494) + CopyBadgeFeedback (:509) + impl (:518),
//! RunResult (:555). Included from app/helpers.rs: effort_display_label (:523),
//! effort_display_label_with_root (:529). Included from
//! app/state_ui_input_helpers.rs: RegisteredCommand (:7) + impl (:13),
//! REGISTERED_COMMANDS (:39), registered_command_entries (:240).
//! Not ported (App-bound or unreferenced): App, extract_input_shell_command,
//! has_safe_slash_command_token, SendAction/ImproveMode/MouseScrollTarget, every
//! `impl App` block, reload_persisted_background_tasks_note.
// [port-decision] upstream path is app::helpers::model_names (a subdir of app);
// the renderers' sedded refs keep that shape via this re-export.
#[allow(unused_imports)] // re-export: consumers are the cutover-bound TuiState defaults
pub use crate::tui::operant_app::helpers;

#[allow(unused_imports)] // re-export: consumer lands at the cutover (W4 pickers)
use crate::tui::operant_app::message::ConnectionPhase;
use crate::tui::operant_app::prompt::swarm_root_reasoning_effort;
use std::time::{Duration, Instant};

pub(crate) const COMMAND_SUGGESTION_VISIBLE_LIMIT: usize = 8;

fn active_runtime_provider_key() -> Option<String> {
    std::env::var("OPERANT_RUNTIME_PROVIDER")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

/// Current processing status
#[derive(Clone, Default, Debug)]
pub enum ProcessingStatus {
    #[default]
    Idle,
    /// Sending request to API (with optional connection phase detail)
    Sending,
    /// Connection phase update from transport layer
    Connecting(crate::tui::operant_app::message::ConnectionPhase),
    /// Model is reasoning/thinking (real-time duration tracking)
    Thinking(Instant),
    /// Receiving streaming response
    Streaming,
    /// Waiting for network connectivity before retrying an interrupted request
    WaitingForNetwork { listener: String },
    /// Executing a tool
    RunningTool(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteStartupPhase {
    StartingServer,
    Connecting,
    LoadingSession,
    WaitingForReload,
    Reconnecting { attempt: u32 },
}

impl RemoteStartupPhase {
    pub(crate) fn header_label(&self) -> String {
        match self {
            Self::StartingServer => "starting server…".to_string(),
            Self::Connecting => "connecting to server…".to_string(),
            Self::LoadingSession => "loading session…".to_string(),
            Self::WaitingForReload => "waiting for reload…".to_string(),
            Self::Reconnecting { attempt } => format!("reconnecting ({attempt})…"),
        }
    }

    pub(crate) fn header_label_with_elapsed(&self, elapsed: Duration) -> String {
        let base = self.header_label();
        if elapsed < Duration::from_secs(1) {
            return base;
        }

        let elapsed_str = if elapsed.as_secs() < 60 {
            format!("{}s", elapsed.as_secs())
        } else {
            format!("{}m {}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
        };

        format!("{base} {elapsed_str}")
    }
}

#[derive(Clone, Default)]
pub struct CopyBadgeUiState {
    pub alt_active: bool,
    pub shift_active: bool,
    pub alt_pulse_until: Option<Instant>,
    pub shift_pulse_until: Option<Instant>,
    pub key_active: Option<(char, Instant)>,
    pub copied_feedback: Option<CopyBadgeFeedback>,
    pub expand_feedback_until: Option<Instant>,
    pub expand_feedback_line: Option<usize>,
}

#[derive(Clone)]
pub struct CopyBadgeFeedback {
    pub key: char,
    pub success: bool,
    pub expires_at: Instant,
}

impl CopyBadgeUiState {
    fn pulse_active(expires_at: Option<Instant>, now: Instant) -> bool {
        expires_at.is_some_and(|expires_at| expires_at > now)
    }

    pub(crate) fn alt_is_active(&self, now: Instant) -> bool {
        self.alt_active || Self::pulse_active(self.alt_pulse_until, now)
    }

    pub(crate) fn shift_is_active(&self, now: Instant) -> bool {
        self.alt_is_active(now)
            && (self.shift_active || Self::pulse_active(self.shift_pulse_until, now))
    }

    pub(crate) fn key_is_active(&self, key: char, now: Instant) -> bool {
        self.shift_is_active(now)
            && self
                .key_active
                .as_ref()
                .map(|(active_key, expires_at)| {
                    active_key.eq_ignore_ascii_case(&key) && *expires_at > now
                })
                .unwrap_or(false)
    }

    pub(crate) fn feedback_for_key(&self, key: char, now: Instant) -> Option<bool> {
        self.copied_feedback.as_ref().and_then(|feedback| {
            if feedback.key.eq_ignore_ascii_case(&key) && feedback.expires_at > now {
                Some(feedback.success)
            } else {
                None
            }
        })
    }

    pub(crate) fn expand_feedback_is_active(&self, now: Instant) -> bool {
        self.expand_feedback_until
            .is_some_and(|expires_at| expires_at > now)
    }
}

/// Result from running the TUI
#[derive(Debug, Default)]
pub struct RunResult {
    /// Session ID to reload (hot-reload, no rebuild)
    pub reload_session: Option<String>,
    /// Session ID to rebuild (full git pull + cargo build + tests)
    pub rebuild_session: Option<String>,
    /// Session ID to update (download from GitHub releases and reload)
    pub update_session: Option<String>,
    /// Session ID to restart (exec into current binary, no build)
    pub restart_session: Option<String>,
    /// Exit code to use (for canary wrapper communication)
    pub exit_code: Option<i32>,
    /// The session ID that was active (for resume hints on exit)
    pub session_id: Option<String>,
}

pub(crate) fn effort_display_label(effort: &str) -> &str {
    effort_display_label_with_root(effort, swarm_root_reasoning_effort(effort))
}

// Keep finite, validated effort labels static so autocomplete can share them
// without allocations or leaking dynamically formatted strings.
fn effort_display_label_with_root<'a>(effort: &'a str, root: Option<&str>) -> &'a str {
    macro_rules! swarm_label {
        ($mode:literal, $detail:literal) => {
            match root.unwrap_or("max") {
                "none" => concat!($mode, " (None + ", $detail, ") [Beta]"),
                "minimal" => concat!($mode, " (Minimal + ", $detail, ") [Beta]"),
                "low" => concat!($mode, " (Low + ", $detail, ") [Beta]"),
                "medium" => concat!($mode, " (Medium + ", $detail, ") [Beta]"),
                "high" => concat!($mode, " (High + ", $detail, ") [Beta]"),
                "xhigh" => concat!($mode, " (xHigh + ", $detail, ") [Beta]"),
                _ => concat!($mode, " (Max + ", $detail, ") [Beta]"),
            }
        };
    }
    match effort {
        "swarm" => swarm_label!("Swarm", "light fan-out"),
        "swarm-deep" => swarm_label!("Swarm Deep", "task graph"),
        "max" => "Max",
        "xhigh" => "xHigh",
        "high" => "High",
        "medium" => "Medium",
        "low" => "Low",
        "none" => "None",
        other => other,
    }
}

#[derive(Clone, Copy)]
struct RegisteredCommand {
    name: &'static str,
    help: &'static str,
    hidden: bool,
}

impl RegisteredCommand {
    const fn public(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            hidden: false,
        }
    }

    const fn remote(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            hidden: false,
        }
    }

    const fn hidden(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            hidden: true,
        }
    }
}

const REGISTERED_COMMANDS: &[RegisteredCommand] = &[
    RegisteredCommand::public("/help", "Show help and keyboard shortcuts"),
    RegisteredCommand::public("/?", "Show help and keyboard shortcuts"),
    RegisteredCommand::public("/commands", "Alias for /help"),
    RegisteredCommand::public("/model", "List or switch models"),
    RegisteredCommand::public("/models", "Alias for /model"),
    RegisteredCommand::public(
        "/provider-test-coverage",
        "Show live-test evidence for the current provider/model",
    ),
    RegisteredCommand::hidden("/model-status", "Alias for /provider-test-coverage"),
    RegisteredCommand::public("/refresh-model-list", "Refresh provider model catalogs"),
    RegisteredCommand::public("/agents", "Configure models for agent roles"),
    RegisteredCommand::public(
        "/swarm-prompt",
        "Open the active swarm routing prompt in your editor",
    ),
    RegisteredCommand::public("/subagent", "Launch a subagent manually"),
    RegisteredCommand::public("/observe", "Show the latest tool context in the side panel"),
    RegisteredCommand::public("/todos", "Show the session todo list as a card in the chat"),
    RegisteredCommand::hidden("/todo", "Alias for /todos"),
    RegisteredCommand::public("/splitview", "Mirror the current chat in the side panel"),
    RegisteredCommand::public("/split-view", "Alias for /splitview"),
    RegisteredCommand::public("/btw", "Ask a side question in the side panel"),
    RegisteredCommand::public("/ssh", "Connect to a remote machine using system SSH"),
    RegisteredCommand::public("/git", "Show git status for the session working directory"),
    RegisteredCommand::public("/colors", "List, configure, and score every TUI color"),
    RegisteredCommand::hidden("/color", "Alias for /colors"),
    RegisteredCommand::public("/hotkeys", "List hotkeys with your personal usage"),
    RegisteredCommand::public("/terminal-setup", "Fix Shift+Enter newlines"),
    RegisteredCommand::public("/commit", "Make logical commits from current changes"),
    RegisteredCommand::public(
        "/merge",
        "Merge current branch into main/master and switch to it (no push)",
    ),
    RegisteredCommand::public(
        "/commit-push",
        "Make logical commits from current changes, then push",
    ),
    RegisteredCommand::hidden("/commit-and-push", "Alias for /commit-push"),
    RegisteredCommand::public(
        "/fast-release",
        "Publish Linux immediately from the warm selfdev cache; CI adds other platforms",
    ),
    RegisteredCommand::public(
        "/fast-macos-release",
        "Publish a prepared macOS arm64 build immediately; CI adds other platforms",
    ),
    RegisteredCommand::public("/remote", "Reach this session from another machine"),
    RegisteredCommand::public(
        "/merge-remote-release",
        "Merge into main/master, validate, push, and release remotely",
    ),
    RegisteredCommand::public(
        "/remote-release",
        "Push the release tag immediately; CI builds and publishes every platform",
    ),
    RegisteredCommand::hidden("/cut-release", "Alias for /fast-release"),
    RegisteredCommand::hidden("/commit-push-release", "Alias for /cut-release"),
    RegisteredCommand::public(
        "/triage",
        "Triage new GitHub issues and autonomously fix the safe ones",
    ),
    RegisteredCommand::public("/transcript", "Open the current session transcript file"),
    RegisteredCommand::public("/subagent-model", "Show/change subagent model policy"),
    RegisteredCommand::public("/autoreview", "Show/toggle automatic end-of-turn review"),
    RegisteredCommand::public("/autojudge", "Show/toggle automatic end-of-turn judging"),
    RegisteredCommand::public("/review", "Launch a one-shot headed review session"),
    RegisteredCommand::public("/judge", "Launch a one-shot headed judge session"),
    #[cfg(any())]
    // [port-decision] keybind registry not ported (EFFORT_HELP); re-activate at cutover
    RegisteredCommand::public("/effort", crate::tui::operant_app::keybind::EFFORT_HELP),
    RegisteredCommand::public("/fast", "Toggle fast mode"),
    RegisteredCommand::public("/transport", "Show/change connection transport"),
    RegisteredCommand::public("/alignment", "Show/change default text alignment"),
    RegisteredCommand::public(
        "/compact-notifications",
        "Show/toggle single-line swarm/file-activity notifications",
    ),
    RegisteredCommand::public(
        "/show-agentgrep-output",
        "Show/toggle full agentgrep search output inline in chat",
    ),
    RegisteredCommand::public(
        "/tool-call-details",
        "Show/toggle dimmed technical details on tool rows with an intent",
    ),
    RegisteredCommand::public(
        "/thinking-display",
        "Show/hide the model's thinking text (off/full/current)",
    ),
    RegisteredCommand::hidden("/thinking", "Alias for /thinking-display"),
    RegisteredCommand::hidden("/reasoning", "Alias for /thinking-display"),
    RegisteredCommand::public("/cancel", "Cancel the current prompt or operation"),
    RegisteredCommand::public("/clear", "Clear conversation history"),
    RegisteredCommand::public("/cls", "Clear the view only, keeping context"),
    RegisteredCommand::hidden("/clear-view", "Alias for /cls"),
    RegisteredCommand::public("/rewind", "Rewind conversation to previous message"),
    RegisteredCommand::public("/poke", "Poke model to resume with incomplete todos"),
    RegisteredCommand::public("/plan", "Create a plan-only response as a plan card"),
    RegisteredCommand::public("/improve", "Autonomously improve the repository"),
    RegisteredCommand::public("/refactor", "Run a safe refactor loop"),
    RegisteredCommand::public("/compact", "Compact context"),
    RegisteredCommand::public("/fix", "Recover when the model cannot continue"),
    RegisteredCommand::public("/voice", "Voice input: speak, then send (Ctrl+Space)"),
    RegisteredCommand::public("/dictate", "Run configured external dictation command"),
    RegisteredCommand::public("/dictation", "Alias for /dictate"),
    RegisteredCommand::public("/memory", "Toggle memory feature"),
    RegisteredCommand::public("/test", "Verify a claim/current changes with layered tests"),
    RegisteredCommand::public(
        "/initiatives",
        "Open initiatives overview / resume tracked initiatives",
    ),
    RegisteredCommand::public("/goals", "Legacy alias for /initiatives"),
    RegisteredCommand::public("/swarm", "Toggle swarm feature"),
    RegisteredCommand::public("/overnight", "Run a supervised overnight coordinator"),
    RegisteredCommand::public("/context", "Show the full session context snapshot"),
    RegisteredCommand::public(
        "/skills",
        "Show loaded skills and operant-endorsed recommendations",
    ),
    RegisteredCommand::public("/version", "Show current version"),
    RegisteredCommand::public("/changelog", "Show recent changes in this build"),
    RegisteredCommand::public("/info", "Show session info and tokens"),
    RegisteredCommand::public("/reset", "Review and confirm a banked OpenAI usage reset"),
    RegisteredCommand::public("/usage", "Show connected provider usage limits"),
    RegisteredCommand::public(
        "/productivity",
        "Generate a shareable usage report + dashboard image",
    ),
    RegisteredCommand::public("/wrapped", "Alias for /productivity"),
    RegisteredCommand::public("/feedback", "Send feedback about operant"),
    RegisteredCommand::public("/telemetry", "Show or change what operant sends"),
    RegisteredCommand::public("/support", "Email support with diagnostics prefilled"),
    RegisteredCommand::public("/subscription", "Show operant subscription status"),
    RegisteredCommand::public("/subscribe", "Why and how to subscribe to operant"),
    RegisteredCommand::public("/config", "Show or edit configuration"),
    RegisteredCommand::public("/log", "Mark the current location in the operant logs"),
    RegisteredCommand::public(
        "/keys",
        "Show keybinding conflicts with your terminal and OS (/keys refresh to rescan)",
    ),
    RegisteredCommand::hidden("/keybindings", "Alias for /keys"),
    RegisteredCommand::public(
        "/diff",
        "Cycle or set diff display mode (off/inline/full/file)",
    ),
    RegisteredCommand::public(
        "/onboarding-preview",
        "Preview the first-run onboarding screen",
    ),
    RegisteredCommand::public(
        "/onboarding-sim",
        "Walk through every first-run onboarding screen (Alt+5 reset, Cmd+5 toggle)",
    ),
    RegisteredCommand::public("/reload", "Reload into newest available binary"),
    RegisteredCommand::public("/restart", "Restart with current binary"),
    RegisteredCommand::public("/rebuild", "Background rebuild and auto reload"),
    RegisteredCommand::public("/selfdev", "Open a new self-dev operant session"),
    RegisteredCommand::public("/update", "Background update and auto reload"),
    RegisteredCommand::public("/update-sim", "Preview update UI safely (Alt+_)"),
    RegisteredCommand::public("/resume", "Open session picker"),
    RegisteredCommand::public("/sessions", "Alias for /resume"),
    RegisteredCommand::public("/session", "Alias for /resume"),
    RegisteredCommand::public("/active", "Manage live sessions (working vs ready)"),
    RegisteredCommand::public("/catchup", "Open Catch Up picker"),
    RegisteredCommand::public("/back", "Return to the previous Catch Up session"),
    RegisteredCommand::public("/save", "Bookmark session for easy access"),
    RegisteredCommand::public("/unsave", "Remove bookmark from session"),
    RegisteredCommand::public("/rename", "Rename current session"),
    RegisteredCommand::public("/fork", "Fork session into a new window (optional prompt)"),
    RegisteredCommand::hidden("/split", "Alias for /fork"),
    RegisteredCommand::public("/transfer", "Compact context into a fresh handoff session"),
    RegisteredCommand::public("/workspace", "Niri-style session workspace"),
    RegisteredCommand::public("/quit", "Exit operant"),
    RegisteredCommand::public("/auth", "Show authentication status"),
    RegisteredCommand::public("/login", "Login to a provider"),
    RegisteredCommand::public("/logout", "Log out of a provider"),
    RegisteredCommand::public("/account", "Open the combined account picker"),
    RegisteredCommand::public("/accounts", "Alias for /account"),
    RegisteredCommand::public("/cache", "Show cache stats; extend/5m saves Anthropic TTL"),
    RegisteredCommand::public("/debug-visual", "Toggle visual debug overlay"),
    RegisteredCommand::public("/screenshot-mode", "Toggle screenshot capture mode"),
    RegisteredCommand::public("/screenshot", "Capture a screenshot debug state"),
    RegisteredCommand::public("/record", "Record a demo capture"),
    RegisteredCommand::remote("/client-reload", "Force reload client binary"),
    RegisteredCommand::remote("/server-reload", "Force reload server binary"),
    RegisteredCommand::remote(
        "/continue",
        "Continue every interrupted live session that would auto-resume",
    ),
    RegisteredCommand::remote("/resumeall", "Alias for /continue"),
    RegisteredCommand::hidden("/resume-all", "Alias for /continue"),
    RegisteredCommand::hidden("/z", "Secret premium-mode command"),
    RegisteredCommand::hidden("/zz", "Secret premium-mode command"),
    RegisteredCommand::hidden("/zzz", "Secret premium-mode command"),
    RegisteredCommand::hidden("/zstatus", "Secret premium-mode status command"),
];

/// Every non-hidden slash command with its one-line description, in
/// registration order. The `/help` overlay uses this to list commands its
/// hand-written sections have not covered, so a newly registered command can
/// never be invisible to users.
pub(crate) fn registered_command_entries() -> impl Iterator<Item = (&'static str, &'static str)> {
    REGISTERED_COMMANDS
        .iter()
        .filter(|command| !command.hidden)
        .map(|command| (command.name, command.help))
}

// [port-decision] shell/ slash-command input probes (batch-3): ui_input.rs:41,47
// reference these two helpers. Their upstream implementations (input.rs:87-92,
// slash_command_parser.rs:26-124) port verbatim below; `scan_slash_tokens`
// shares its closure state, so all three travel together as one function block.
// Upstream would otherwise live as an [`crate::tui::input`] module — the whole
// module is not in operant_app at this wave, so these land locally.

#[inline]
pub(crate) fn extract_input_shell_command(input: &str) -> Option<&str> {
    input.trim().strip_prefix('!').map(str::trim)
}

pub(crate) fn has_safe_slash_command_token(input: &str) -> bool {
    active_token_before_cursor(input, input.len()).is_some()
}

pub(super) fn active_token_before_cursor(input: &str, cursor: usize) -> Option<(usize, usize)> {
    let cursor = cursor.min(input.len());
    if !input.is_char_boundary(cursor) {
        return None;
    }

    let mut active = None;
    scan_slash_tokens(&input[..cursor], |start, end| {
        if end == cursor {
            active = Some((start, end));
        }
    });
    active
}

// Slash: scan the input spans of slash/token pieces separated by whitespace,
// honoring quotes, backticks, and fence backticks — deductions from the
// `//            [truncated]` trailing arm of upstream slash_command_parser.rs.
fn scan_slash_tokens(input: &str, mut on_token: impl FnMut(usize, usize)) {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backticks = false;
    let mut in_fenced_code = false;
    let mut escaped = false;
    let mut line_start = true;
    let mut iter = input.char_indices().peekable();
    while let Some((index, ch)) = iter.next() {
        if in_fenced_code {
            if line_start && ch == '`' && input[index..].starts_with("```") {
                iter.next();
                iter.next();
                in_fenced_code = false;
                line_start = false;
                continue;
            }
            line_start = ch == '\n';
            continue;
        }

        if escaped {
            escaped = false;
            line_start = ch == '\n';
            continue;
        }
        if ch == '\\' {
            escaped = true;
            line_start = false;
            continue;
        }

        if !in_single_quote && !in_double_quote && ch == '`' {
            if line_start && input[index..].starts_with("```") {
                iter.next();
                iter.next();
                in_fenced_code = true;
                line_start = false;
                continue;
            }
            in_backticks = !in_backticks;
            line_start = false;
            continue;
        }
        if in_backticks {
            line_start = ch == '\n';
            continue;
        }
        if !in_double_quote && ch == '\'' {
            in_single_quote = !in_single_quote;
            line_start = false;
            continue;
        }
        if !in_single_quote && ch == '"' {
            in_double_quote = !in_double_quote;
            line_start = false;
            continue;
        }

        // Upstream slash_command_parser.rs:100-121 — the `/`-token branch,
        // verbatim (closing the fn; no truncation).
        if !in_single_quote
            && !in_double_quote
            && ch == '/'
            && (index == 0
                || input[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace))
        {
            let end = input[index..]
                .char_indices()
                .find_map(|(offset, value)| value.is_whitespace().then_some(index + offset))
                .unwrap_or(input.len());
            on_token(index, end);
            line_start = false;
            continue;
        }

        line_start = ch == '\n';
    }
}
