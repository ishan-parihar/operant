// Vendored from jcode (crates/jcode-tui/src/tui/app/onboarding_flow.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// [port-decision] the first-run onboarding flow proper (744 upstream LOC:
// the OAuth-detection state machine and the "continue where you left off" page
// builder) is not ported. The only thing the ported session picker reaches is
// the `ExternalCli` selector (:32-:51), which scopes that picker's external-CLI
// view. It is a 5-variant pure-data enum, so it is ported verbatim into its own
// leaf rather than gated at its two call sites
// (jcode_ui/session_picker/loading.rs:3072/:3074 and loading_tests.rs:1275) —
// `onboarding_scoped_loader_returns_only_codex_sessions` is a live test that
// matches on every variant, so gating the surface would have disabled a real
// behavioural check instead of narrowing a subsystem.
/// Which external CLI an OAuth login was detected for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalCli {
    Codex,
    ClaudeCode,
    Pi,
    OpenCode,
    Cursor,
}

impl ExternalCli {
    pub fn label(self) -> &'static str {
        match self {
            ExternalCli::Codex => "Codex",
            ExternalCli::ClaudeCode => "Claude Code",
            ExternalCli::Pi => "Pi",
            ExternalCli::OpenCode => "OpenCode",
            ExternalCli::Cursor => "Cursor",
        }
    }
}
