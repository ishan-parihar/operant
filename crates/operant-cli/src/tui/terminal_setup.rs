//! `/terminal-setup` — tell the user how to make Shift+Enter reach operant.
//!
//! ## Why this exists
//!
//! `app/key_handling.rs` maps Enter + (Shift | Alt | Ctrl) to
//! `PromptInputState::insert_newline`, and plain Enter to submit. That handler
//! has been live since iter-120. The gap is entirely terminal-side: most
//! terminals and multiplexers send a bare `\r` for Shift+Enter, which crossterm
//! reports as an unmodified `KeyCode::Enter` — indistinguishable from a plain
//! submit. Nothing in the app can recover the distinction, because the
//! information was never transmitted.
//!
//! So the fix is a terminal or multiplexer configuration change, and the only
//! useful thing operant can do is say which one applies here.
//!
//! ## What this deliberately does NOT do
//!
//! It does not rewrite the user's config files. A slash command that silently
//! edits `~/.tmux.conf` or a WezTerm Lua config is a surprise with a blast
//! radius the user did not author, and the change is two lines they can paste.
//! So this prints the exact snippet and where it belongs.
//!
//! ## What is always true
//!
//! Alt+Enter and Ctrl+Enter take the same branch as Shift+Enter, and Ctrl+J
//! arrives as a Control-modified key in the terminals that matter. Operant has
//! three newline routes that need no configuration at all; this command exists
//! only for the one that does.

use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// Environment snapshot
// ---------------------------------------------------------------------------

/// The handful of environment facts the advice depends on, captured up front.
///
/// Taking a snapshot rather than reading `std::env` inline keeps [`advice`]
/// a pure function, so every branch below is testable without mutating the
/// test process's environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    /// `$TMUX` — set inside a tmux server session.
    pub tmux: bool,
    /// `$STY` — set inside GNU screen.
    pub screen: bool,
    /// `$TERM_PROGRAM` — set by terminals that advertise themselves
    /// (WezTerm, iTerm2, Apple_Terminal, vscode).
    pub term_program: Option<String>,
    /// `$TERM`.
    pub term: Option<String>,
    /// `$KITTY_WINDOW_ID` — set by kitty.
    pub kitty_window: bool,
}

impl EnvSnapshot {
    /// Read the real process environment.
    pub fn from_process() -> Self {
        let var = |k: &str| std::env::var(k).ok();
        Self {
            tmux: var("TMUX").is_some(),
            screen: var("STY").is_some(),
            term_program: var("TERM_PROGRAM"),
            term: var("TERM"),
            kitty_window: var("KITTY_WINDOW_ID").is_some(),
        }
    }
}

// ---------------------------------------------------------------------------
// Advice
// ---------------------------------------------------------------------------

/// One actionable recommendation for the environment the user is actually in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    /// What was detected, e.g. `"tmux 3.2+"`. Shown as the headline.
    pub target: String,
    /// What to change and why it is needed. Always present.
    pub detail: String,
    /// The exact lines to add, and where. `None` when there is nothing to add.
    pub snippet: Option<String>,
}

/// Classify the environment and produce the advice for it.
///
/// Ordering is deliberate: a multiplexer is reported before the terminal it
/// wraps, because if the user is inside tmux then tmux is what has to forward
/// the modified key, and fixing only the inner terminal would not help.
pub fn advice(env: &EnvSnapshot) -> Vec<Advice> {
    let mut out = Vec::new();

    if env.tmux {
        out.push(Advice {
            target: "tmux".to_string(),
            detail: "tmux does not forward modified Enter unless extended keys are on, so \
                     Shift+Enter arrives as a plain submit. `extended-keys` needs tmux 3.2 or \
                     newer; on older tmux the bind alone still works."
                .to_string(),
            snippet: Some(
                "# ~/.tmux.conf  (or ~/.config/tmux/tmux.conf)\n\
                 set -s extended-keys on\n\
                 bind -n S-Enter send-keys \"\\x1b[13;2u\"\n"
                    .to_string(),
            ),
        });
    }

    if env.screen {
        out.push(Advice {
            target: "GNU screen".to_string(),
            detail: "screen has no documented way to forward a modified Enter distinctly, so \
                     Shift+Enter cannot be made to work here. Use Ctrl+J or Alt+Enter instead — \
                     both are handled with no configuration."
                .to_string(),
            snippet: None,
        });
    }

    match env.term_program.as_deref() {
        Some("WezTerm") => out.push(Advice {
            target: "WezTerm".to_string(),
            detail: "WezTerm can send any escape sequence for a key combination. This makes \
                     Shift+Enter emit the CSI-u form crossterm decodes as Enter+SHIFT."
                .to_string(),
            snippet: Some(
                "-- wezterm.lua\n\
                 config.key_tables = {\n\
                 \x20 {\n\
                 \x20   { key = \"Enter\", mods = \"SHIFT\",\n\
                 \x20     action = WezTerm.action.SendEscapeSequence(\"\\x1b[13;2u\") },\n\
                 \x20 },\n\
                 }\n"
                .to_string(),
            ),
        }),
        Some("Apple_Terminal") => out.push(Advice {
            target: "Apple Terminal".to_string(),
            detail: "Apple Terminal has no key-mapping facility and sends a bare carriage \
                     return for Shift+Enter, so this cannot be configured. Use Ctrl+J or \
                     Alt+Enter, both of which operant already handles."
                .to_string(),
            snippet: None,
        }),
        Some(other) => out.push(Advice {
            target: other.to_string(),
            detail: "This terminal advertises itself but operant has no verified key-mapping \
                     recipe for it, so no snippet is offered rather than an unverified one. \
                     Ctrl+J and Alt+Enter work with no configuration."
                .to_string(),
            snippet: None,
        }),
        None => {}
    }

    if env.kitty_window {
        out.push(Advice {
            target: "kitty".to_string(),
            detail: "kitty only reports Shift+Enter distinctly once the application enables the \
                     progressive keyboard protocol, which operant does not currently do. Until \
                     it does, use Ctrl+J or Alt+Enter."
                .to_string(),
            snippet: None,
        });
    }

    // The last-resort terminals: no TERM_PROGRAM, not a multiplexer, not kitty.
    if out.is_empty() {
        out.push(Advice {
            target: env
                .term
                .clone()
                .unwrap_or_else(|| "unknown terminal".to_string()),
            detail: "Nothing was detected that operant has a verified recipe for, and most \
                     terminals cannot be configured to send a distinct Shift+Enter. Use Ctrl+J \
                     or Alt+Enter — both are handled with no configuration at all."
                .to_string(),
            snippet: None,
        });
    }

    out
}

/// A note about terminal graphics, which has nothing to do with Shift+Enter but
/// is the same class of problem: a terminal-side capability operant cannot fix
/// from inside the process.
///
/// Returned separately from [`advice`] because that type is scoped to key
/// mapping, and because the answer is a sentence rather than a snippet — there
/// is no config change that makes tmux forward Sixel for us.
pub fn graphics_note(env: &EnvSnapshot) -> Option<String> {
    if !env.tmux {
        return None;
    }
    Some(
        "Images: inside tmux, operant does not claim a graphics protocol, because \
         $TERM describes tmux's own terminfo rather than the terminal that would \
         receive the bytes — whether Sixel passes through depends on how tmux was \
         BUILT and on your outer terminal, and neither is visible from here. So \
         pasted images, diagrams and rendered formulas show as text inside tmux, \
         and the same content renders outside it."
            .to_string(),
    )
}

/// Render the advice for display in the status line / command output.
pub fn render(env: &EnvSnapshot) -> String {
    let items = advice(env);
    let mut out = String::new();

    // Always true, and the most important line: the user may have pressed
    // Shift+Enter, seen it submit, and concluded operant is broken.
    let _ = writeln!(
        out,
        "Shift+Enter inserts a newline in operant. It only works if your terminal sends a \
         distinct code for it."
    );
    let _ = writeln!(
        out,
        "Ctrl+J and Alt+Enter always work, with no configuration.\n"
    );

    for item in &items {
        let _ = writeln!(out, "{}: {}", item.target, item.detail);
        if let Some(snippet) = &item.snippet {
            let _ = writeln!(out, "\n{snippet}");
        }
        let _ = writeln!(out);
    }

    // A separate section, not another `Advice`: it is not about Shift+Enter.
    if let Some(note) = graphics_note(env) {
        let _ = writeln!(out, "{note}");
    }

    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet_of(env: &EnvSnapshot, target: &str) -> Option<String> {
        advice(env)
            .into_iter()
            .find(|a| a.target == target)
            .and_then(|a| a.snippet)
    }

    #[test]
    fn a_multiplexer_is_told_why_images_do_not_render() {
        let env = EnvSnapshot {
            tmux: true,
            ..Default::default()
        };
        let note = graphics_note(&env).expect("tmux must get the graphics note");
        assert!(note.contains("tmux"), "the note must name the multiplexer");
        assert!(
            note.contains("does not claim a graphics protocol"),
            "the note must say what operant DOES, not only what is wrong"
        );
        // It has to reach the user, not just exist — the whole point of adding
        // it alongside the Sixel detection fix.
        assert!(
            render(&env).contains("does not claim a graphics protocol"),
            "the graphics note must actually be rendered by /terminal-setup"
        );
    }

    #[test]
    fn a_plain_terminal_gets_no_graphics_note() {
        assert!(graphics_note(&EnvSnapshot::default()).is_none());
    }

    #[test]
    fn tmux_is_detected_and_gets_the_extended_keys_recipe() {
        let env = EnvSnapshot {
            tmux: true,
            ..Default::default()
        };
        let got = snippet_of(&env, "tmux").expect("tmux advice must carry a snippet");
        assert!(got.contains("set -s extended-keys on"), "got: {got}");
        assert!(got.contains("S-Enter"), "got: {got}");
    }

    #[test]
    fn wezterm_is_detected_from_term_program() {
        let env = EnvSnapshot {
            term_program: Some("WezTerm".to_string()),
            ..Default::default()
        };
        let got = snippet_of(&env, "WezTerm").expect("WezTerm advice must carry a snippet");
        assert!(got.contains("SendEscapeSequence"), "got: {got}");
        assert!(got.contains("13;2u"), "got: {got}");
    }

    #[test]
    fn apple_terminal_is_honest_rather_than_inventing_a_snippet() {
        let env = EnvSnapshot {
            term_program: Some("Apple_Terminal".to_string()),
            ..Default::default()
        };
        let got = snippet_of(&env, "Apple Terminal");
        assert!(got.is_none(), "Apple Terminal cannot be configured");
        let detail = advice(&env)
            .into_iter()
            .find(|a| a.target == "Apple Terminal")
            .map(|a| a.detail)
            .unwrap();
        assert!(
            detail.contains("Ctrl+J"),
            "must point at a route that works today: {detail}"
        );
    }

    /// The regression that matters: the multiplexer is reported BEFORE the
    /// terminal it wraps, because if tmux swallows the modified key then
    /// fixing only the inner terminal achieves nothing.
    #[test]
    fn tmux_is_reported_before_the_inner_terminal() {
        let env = EnvSnapshot {
            tmux: true,
            term_program: Some("WezTerm".to_string()),
            ..Default::default()
        };
        let targets: Vec<String> = advice(&env).into_iter().map(|a| a.target).collect();
        assert_eq!(targets, vec!["tmux".to_string(), "WezTerm".to_string()]);
    }

    #[test]
    fn an_unrecognised_term_program_gets_no_unverified_snippet() {
        let env = EnvSnapshot {
            term_program: Some("SomeNewTerminal".to_string()),
            ..Default::default()
        };
        assert!(snippet_of(&env, "SomeNewTerminal").is_none());
    }

    #[test]
    fn a_bare_environment_still_explains_the_working_alternatives() {
        let env = EnvSnapshot {
            term: Some("xterm-256color".to_string()),
            ..Default::default()
        };
        let items = advice(&env);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, "xterm-256color");
        assert!(items[0].detail.contains("Ctrl+J"));
    }

    #[test]
    fn kitty_is_named_but_not_claimed_to_be_fixable() {
        let env = EnvSnapshot {
            kitty_window: true,
            ..Default::default()
        };
        let detail = advice(&env)
            .into_iter()
            .find(|a| a.target == "kitty")
            .map(|a| a.detail)
            .expect("kitty must be reported");
        assert!(detail.contains("Ctrl+J"), "got: {detail}");
    }

    /// Ctrl+J is the route that needs no configuration, so it must be stated on
    /// every render regardless of what else was detected.
    #[test]
    fn render_always_states_the_zero_config_routes() {
        for env in [
            EnvSnapshot::default(),
            EnvSnapshot {
                tmux: true,
                ..Default::default()
            },
            EnvSnapshot {
                term_program: Some("Apple_Terminal".to_string()),
                ..Default::default()
            },
        ] {
            let text = render(&env);
            assert!(text.contains("Ctrl+J"), "missing from render for {env:?}");
            assert!(
                text.contains("Alt+Enter"),
                "missing from render for {env:?}"
            );
        }
    }

    #[test]
    fn render_includes_the_snippet_body_when_one_exists() {
        let env = EnvSnapshot {
            tmux: true,
            ..Default::default()
        };
        assert!(render(&env).contains("extended-keys"));
    }

    /// Without this, every test above still passes if `/terminal-setup` goes
    /// back to printing "no manual setup needed": the module is `pub`, so
    /// nothing reports it as unreachable, and the user is told there is
    /// nothing to fix while Shift+Enter keeps submitting. Pin the call site.
    #[test]
    fn the_slash_command_still_calls_this_module() {
        let commands = include_str!("app/commands.rs");
        assert!(
            commands.contains("terminal_setup::render"),
            "app/commands.rs no longer calls `terminal_setup::render` — `/terminal-setup` \
             would keep its old stub message while this module's tests all pass"
        );
        assert!(
            !commands.contains("No manual setup needed"),
            "the old `/terminal-setup` stub message is back — it tells the user there is \
             nothing to fix while Shift+Enter silently submits"
        );
    }
}
