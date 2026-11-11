// keybindings/tests.rs — Unit tests for the keybinding registry.
//
// Extracted from the keybindings.rs monolith.

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

/// iter-1263: the transcript scroll ladder's catalogue entries must match
/// their real dispatch arms — KEY and MODIFIER, not just the keycode token
/// the loose guard above accepts. The Shift+Home/End entries were absent from
/// /hotkeys until iter-1263 (the chords worked live but were never advertised),
/// and the PageUp/Down descriptions still promised a fixed "10" the dispatch
/// never used. This pin normalizes whitespace and matches the full arm
/// pattern so a deleted or modifier-changed arm fails the catalogue honestly.
/// (iter-1959) The catalogue makes exactly one promise for the Dialog context:
/// `Esc` is bound to "Close dialog". Every MODAL dialog must keep it.
///
/// `key_handling.rs` names `KeyCode::Esc` in 38 arms, so the promise is
/// plausible — but a chord being somewhere in the file is not the same claim as
/// it being on every surface. The audit learned that twice over: a bare `x` arm
/// satisfies an advertised `Ctrl+X`, and a chord that dispatches on one surface
/// can simply be absent from another. This asks the code: open the modal
/// dialogs, press the ONE key the catalogue promises, and require them to close.
///
/// Each surface exposes the same `visible: bool` the renderer reads, so the
/// fixture is one field per surface and the assertion is unambiguous.
///
/// **The usage overlay is deliberately EXCLUDED, and the exclusion is pinned.**
/// `key_handling.rs`'s own comment on the F8 arm reads "Not modal, so it stays
/// up until the next press", and `usage_overlay::toggle_flips_visibility` pins
/// that F8 toggles it. A panel that persists until the next press is not a
/// dialog, and the catalogue does not list it as one — so Esc is not its
/// contract. My first draft asserted all six and reported the usage overlay as
/// the defect, which was my test's premise being wrong, not the code's: the
/// fourth time this audit's own probe manufactured a defect the code does not
/// have. Rather than delete the case, the exception is asserted as an
/// exception, so a future change that makes the overlay modal has to notice.
/// (iter-1961) Every chord the Completion context describes must do the thing
/// its description says.
///
/// Round 75's behavioural sweep covers Global, Prompt, Dialog and Completion,
/// but its signal is "the key typed itself" — so it catches a chord nothing
/// consumed, and it cannot distinguish a chord that swallows the key from one
/// that does what it says. Completion is exactly that risk: `Down`, `Up`, `Tab`,
/// `Enter` and `Esc` are all advertised as motion and acceptance, and an arrow
/// that consumed the key while doing nothing would round 75 green and the
/// operator stuck.
///
/// This drives each chord through the real `handle_key_event` on a fresh app
/// with the typeahead popup OPEN, and requires the observable effect each
/// description names: the index moves, the text fills, or the popup closes.
///
/// There is no vim-mode or pane state to construct — the popup is opened by
/// setting the two fields the renderer and the state machine both read, which
/// is how this audit's last four false positives were avoided.
/// (iter-1962) `Ctrl+Y` ("Paste") must either paste or say why not.
///
/// RT-472's rule for this audit: an affordance a surface offers either works or
/// says why not. `yank()` used to return nothing at all, so with an empty kill
/// ring the operator pressed `Ctrl+Y`, the key was consumed, and neither the
/// composer nor a status notice moved — the affordance was offered by /keys and
/// silently did nothing. The dialog paste path already did the right thing
/// ("Clipboard is empty"), so the composer path was the odd one out.
///
/// `yank()` now reports whether it pasted and the arm emits the notice. This
/// pins the behaviour from the operator's side: press the key the catalogue
/// advertises, in the state where there is genuinely nothing to paste, and
/// require it to say so.
/// (iter-1963) Every chord the VimNormal context describes must either WORK or
/// SAY WHY NOT.
///
/// Rounds 75/76/78 closed this for Global, Prompt, Dialog and Completion; the
/// vim contexts were deliberately parked because the source-level sweep
/// (`every_string_dispatched_vim_binding_is_reachable`) already covers them and
/// a cross-app behavioural sweep is unsound — a process-global crosses the
/// `make_app()` boundary (RT-527), so a fresh app does not buy fresh behaviour.
///
/// This one is driven INSIDE a single app: for each catalogue row the app's own
/// fields are reset (text, cursor, mode, registers), which is the app state and
/// nothing else. That keeps the oracle sound while still driving the real
/// handler, so a handler that consumes the key and does nothing is visible —
/// which the source-level sweep cannot see.
///
/// The rule is RT-472's: an affordance either works or says why not. So the
/// assertion is that SOME observable changed; a chord that changes nothing and
/// says nothing is the silent no-op.
#[test]
fn every_vimnormal_chord_either_works_or_says_why_not() {
    use crate::tui::keybindings::{BindingContext, DEFAULT_KEYBINDINGS};
    use crate::tui::operant_app::tui_state::TuiState;

    // One app for the whole test — that is the point (see RT-527).
    let mut app = crate::tui::app::tests::make_app();

    let reset = |app: &mut crate::tui::app::App| {
        app.prompt_input.text = "the quick brown fox".to_string();
        app.prompt_input.cursor = 4;
        app.prompt_input.vim_enabled = true;
        app.prompt_input.vim_mode = crate::tui::prompt_input::VimMode::Normal;
        app.prompt_input.vim_pending = crate::tui::prompt_input::VimPendingState::None;
        app.prompt_input.vim_registers = std::collections::HashMap::new();
        app.prompt_input.vim_marks.clear();
    };
    let observable = |app: &crate::tui::app::App| {
        (
            app.prompt_input.text.clone(),
            app.prompt_input.cursor,
            app.prompt_input.vim_mode,
            format!("{:?}", app.prompt_input.vim_pending),
            app.prompt_input.vim_marks.len(),
            app.prompt_input.vim_registers.len(),
            app.prompt_input.yank_buf.clone(),
            app.status_notice(),
        )
    };

    let mut silent: Vec<String> = Vec::new();
    for binding in DEFAULT_KEYBINDINGS.get_bindings(BindingContext::VimNormal) {
        reset(&mut app);
        let before = observable(&app);

        app.handle_key_event(crossterm::event::KeyEvent::new(
            binding.key,
            binding.modifiers,
        ));

        if observable(&app) == before {
            // RT-472's exception, pinned rather than omitted: `Esc` in VimNormal
            // is "Enter Normal mode" and we ARE in Normal mode, so the honest
            // outcome is that nothing changes. That is the chord doing its job in
            // the state that already satisfies it, not a silent no-op — the same
            // reasoning round 76 pinned for the non-modal usage overlay.
            let desc = binding.description.clone().unwrap_or_default();
            // "Enter X mode" is already satisfied ONLY when X names the mode we
            // are in. Comparing the NAMED TARGET is the whole distinction:
            // `Esc` ("Enter Normal mode") is already in Normal, while
            // `Ctrl+V` ("Enter Visual Block mode") is not — an exception that
            // matched the description's prefix instead swallowed the one real
            // defect this sweep exists to find.
            let target = desc
                .strip_prefix("Enter ")
                .and_then(|r| r.strip_suffix(" mode"))
                .map(|m| m.chars().filter(|c| !c.is_whitespace()).collect::<String>())
                .map(|m| m.to_lowercase());
            let current = format!("{:?}", app.prompt_input.vim_mode).to_lowercase();
            let already_satisfied = target.as_deref() == Some(current.as_str());
            // Chords that REPLAY a previous action have nothing to replay on a
            // first press — that is vim's own semantics, not a silent no-op:
            // `.` with no prior change, `;`/`,` with no prior find, `n`/`N`
            // with no active search. Round 76 pinned the non-modal usage
            // overlay as an exception rather than omitting it; this is the same
            // treatment for a different, equally legitimate, reason.
            let replays_a_previous_action = desc.starts_with("Repeat ")
                || desc.starts_with("Reverse find")
                || desc.starts_with("Next match")
                || desc.starts_with("Previous match");
            // `j` in a ONE-line composer has nowhere to go. The composer is a
            // single line, so the honest outcome of "Move down" is no movement.
            let single_line_no_target = desc == "Move down" || desc == "Move up";
            if !already_satisfied && !replays_a_previous_action && !single_line_no_target {
                silent.push(format!(
                    "{:?} + {:?} registered as {:?} changed no vim state and set no notice",
                    binding.modifiers, binding.key, desc
                ));
            }
        }
    }
    assert!(
        silent.is_empty(),
        "these VimNormal chords consume the key and do nothing, and say nothing: \
         either wire the handler or drop the catalogue entry:\n  {}",
        silent.join("\n  ")
    );
}

#[test]
fn ctrl_y_never_silently_fails_to_paste() {
    use crate::tui::operant_app::tui_state::TuiState;

    let mut app = crate::tui::app::tests::make_app();
    // A fresh app has an empty kill ring, which is exactly the state where the
    // affordance has nothing to act on.
    app.prompt_input.text = "fix the parser".to_string();
    app.prompt_input.cursor = 5;

    app.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('y'),
        crossterm::event::KeyModifiers::CONTROL,
    ));

    assert!(
        app.status_notice()
            .as_deref()
            .is_some_and(|n| n.contains("Nothing to paste")),
        "Ctrl+Y is advertised as \"Paste\" but with an empty kill ring it \
         consumed the key and said nothing: {:?}",
        app.status_notice()
    );
    // The composer must be untouched — the notice explains, it does not edit.
    assert_eq!(app.prompt_input.text, "fix the parser");
}

#[test]
fn every_completion_chord_does_what_its_description_says() {
    use crate::tui::prompt_input::TypeaheadSuggestion;
    use crossterm::event::{KeyCode, KeyModifiers};

    let open_popup = |app: &mut crate::tui::app::App| {
        app.prompt_input.suggestions = vec![
            TypeaheadSuggestion::command("/help".to_string(), "Show help".to_string()),
            TypeaheadSuggestion::command("/model".to_string(), "Pick a model".to_string()),
        ];
        app.prompt_input.suggestion_index = Some(0);
        app.prompt_input.text = "/h".to_string();
        app.prompt_input.cursor = 2;
    };
    let press = |app: &mut crate::tui::app::App, code: KeyCode| {
        app.handle_key_event(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE));
    };

    // "Next completion": the selection must actually move.
    let mut app = crate::tui::app::tests::make_app();
    open_popup(&mut app);
    let before = app.prompt_input.suggestion_index;
    press(&mut app, KeyCode::Down);
    assert_ne!(
        app.prompt_input.suggestion_index, before,
        "Down is advertised as \"Next completion\" but the selection did not move"
    );

    // "Previous completion": back where it started.
    let after_down = app.prompt_input.suggestion_index;
    press(&mut app, KeyCode::Up);
    assert_ne!(
        app.prompt_input.suggestion_index, after_down,
        "Up is advertised as \"Previous completion\" but the selection did not move"
    );

    // "Accept completion" via Tab: the composer must hold the suggestion.
    let mut app = crate::tui::app::tests::make_app();
    open_popup(&mut app);
    press(&mut app, KeyCode::Tab);
    assert_eq!(
        app.prompt_input.text, "/help",
        "Tab is advertised as \"Accept completion\" but the composer is {:?}",
        app.prompt_input.text
    );

    // "Accept completion" via Enter: same promise, different chord.
    let mut app = crate::tui::app::tests::make_app();
    open_popup(&mut app);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.prompt_input.text, "/help",
        "Enter is advertised as \"Accept completion\" but the composer is {:?}",
        app.prompt_input.text
    );

    // "Dismiss completion": the popup must go away.
    let mut app = crate::tui::app::tests::make_app();
    open_popup(&mut app);
    assert!(
        !app.prompt_input.suggestions.is_empty(),
        "the popup must open"
    );
    press(&mut app, KeyCode::Esc);
    assert!(
        app.prompt_input.suggestions.is_empty(),
        "Esc is advertised as \"Dismiss completion\" but the popup stayed open"
    );
}

#[test]
fn every_modal_dialog_closes_on_the_escape_the_catalogue_promises() {
    use crossterm::event::{KeyCode, KeyModifiers};

    let open_modal = |app: &mut crate::tui::app::App| {
        app.stats_dialog.visible = true;
        app.export_dialog.visible = true;
        app.free_mode_dialog.visible = true;
        app.key_input_dialog.visible = true;
        app.custom_provider_dialog.visible = true;
        app.bypass_permissions_dialog.visible = true;
        app.connect_dialog.visible = true;
        app.import_config_dialog.visible = true;
        app.ask_user_dialog.visible = true;
        app.help_overlay.visible = true;
        app.history_search_overlay.visible = true;
    };
    let modal_still_open = |app: &crate::tui::app::App| -> Vec<&'static str> {
        let mut open = Vec::new();
        if app.stats_dialog.visible {
            open.push("stats_dialog");
        }
        if app.export_dialog.visible {
            open.push("export_dialog");
        }
        if app.free_mode_dialog.visible {
            open.push("free_mode_dialog");
        }
        if app.key_input_dialog.visible {
            open.push("key_input_dialog");
        }
        if app.custom_provider_dialog.visible {
            open.push("custom_provider_dialog");
        }
        if app.bypass_permissions_dialog.visible {
            open.push("bypass_permissions_dialog");
        }
        if app.connect_dialog.visible {
            open.push("connect_dialog");
        }
        if app.import_config_dialog.visible {
            open.push("import_config_dialog");
        }
        if app.ask_user_dialog.visible {
            open.push("ask_user_dialog");
        }
        if app.help_overlay.visible {
            open.push("help_overlay");
        }
        if app.history_search_overlay.visible {
            open.push("history_search_overlay");
        }
        open
    };

    let mut app = crate::tui::app::tests::make_app();
    open_modal(&mut app);
    assert!(
        !modal_still_open(&app).is_empty(),
        "the fixture must open something"
    );

    app.handle_key_event(crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    ));

    // Routing closes the highest-priority surface first, so repeat until every
    // modal dialog has had its turn. A surface that never closes is the defect;
    // a surface that takes two presses is a different contract and would make
    // this loop bound, which it does not.
    for _ in 0..modal_still_open(&app).len() + 4 {
        if modal_still_open(&app).is_empty() {
            break;
        }
        app.handle_key_event(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ));
    }

    assert!(
        modal_still_open(&app).is_empty(),
        "these dialogs stayed open through repeated Esc: {:?}. Pressing the key \
         the catalogue promises for the Dialog context does nothing on them, so \
         the operator's only advertised way out is to find another key.",
        modal_still_open(&app)
    );

    // The pinned exception: the usage overlay is a PANEL, not a dialog. It must
    // NOT close on Esc (that is its documented contract), and it MUST close on
    // F8, the key that owns it.
    app.usage_overlay.visible = true;
    app.handle_key_event(crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    ));
    assert!(
        app.usage_overlay.visible,
        "the usage overlay closed on Esc, so it stopped being the non-modal \
         panel key_handling.rs documents and toggle_flips_visibility pins — \
         either that comment and its test are now stale, or this is a change \
         nobody decided"
    );
    app.handle_key_event(crossterm::event::KeyEvent::new(
        KeyCode::F(8),
        KeyModifiers::NONE,
    ));
    assert!(
        !app.usage_overlay.visible,
        "F8 must close the usage overlay it opens"
    );
}

#[test]
fn scroll_ladder_catalogue_matches_dispatch() {
    let dispatch = include_str!("../app/key_handling.rs");
    let dispatch = dispatch.split_whitespace().collect::<Vec<_>>().join(" ");

    let globals = DEFAULT_KEYBINDINGS.get_bindings(BindingContext::Global);
    let find = |key: KeyCode, mods: KeyModifiers| -> &KeyBinding {
        globals
            .iter()
            .find(|b| b.key == key && b.modifiers == mods)
            .unwrap_or_else(|| panic!("catalogue is missing {key:?}+{mods:?}"))
    };
    let described = |key: KeyCode, mods: KeyModifiers| -> &str {
        find(key, mods)
            .description
            .as_deref()
            .unwrap_or_else(|| panic!("catalogue entry {key:?}+{mods:?} has no description"))
    };

    // PageUp/PageDown: unconditional transcript paging (iter-1226).
    assert_eq!(
        described(KeyCode::PageUp, KeyModifiers::NONE),
        "Scroll transcript up one page"
    );
    assert_eq!(
        described(KeyCode::PageDown, KeyModifiers::NONE),
        "Scroll transcript down one page"
    );
    // The page step is the live viewport height minus two — the description
    // must not name a fixed number.
    assert!(!described(KeyCode::PageUp, KeyModifiers::NONE).contains('1'));

    // Shift+Home/Shift+End: jump to top/tail (iter-1226), catalogued iter-1263.
    assert_eq!(
        described(KeyCode::Home, KeyModifiers::SHIFT),
        "Jump to transcript top"
    );
    assert_eq!(
        described(KeyCode::End, KeyModifiers::SHIFT),
        "Jump to transcript tail"
    );

    // Each catalogued chord must have a real dispatch arm — the full guard
    // pattern including the modifier, whitespace-normalized to survive fmt.
    for (key, mods, arm_prefix) in [
        (KeyCode::PageUp, KeyModifiers::NONE, "KeyCode::PageUp =>"),
        (
            KeyCode::PageDown,
            KeyModifiers::NONE,
            "KeyCode::PageDown =>",
        ),
        (
            KeyCode::Home,
            KeyModifiers::SHIFT,
            "KeyCode::Home if key.modifiers.contains(KeyModifiers::SHIFT) =>",
        ),
        (
            KeyCode::End,
            KeyModifiers::SHIFT,
            "KeyCode::End if key.modifiers.contains(KeyModifiers::SHIFT) =>",
        ),
    ] {
        let needle = arm_prefix.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            dispatch.contains(&needle),
            "catalogued scroll chord {key:?}+{mods:?} has no dispatch arm in key_handling.rs"
        );
    }

    // (iter-1267) The scroll bookmark moved from Ctrl+G to Alt+G when Ctrl+G
    // became the external editor (key_handling.rs:1311 vs 1314) — but only the
    // Prompt table got the correction; the Global table advertised the dead
    // chord until this validation pass caught it. Pin BOTH contexts so the
    // drift can't ride a context split again.
    for context in [BindingContext::Global, BindingContext::Prompt] {
        let bindings = DEFAULT_KEYBINDINGS.get_bindings(context);
        let described = |key: KeyCode, mods: KeyModifiers| -> &str {
            bindings
                .iter()
                .find(|b| b.key == key && b.modifiers == mods)
                .and_then(|b| b.description.as_deref())
                .unwrap_or("")
        };
        assert_eq!(
            described(KeyCode::Char('g'), KeyModifiers::CONTROL),
            "Edit prompt in external editor ([tui] external_editor)",
            "{context:?} still advertises Ctrl+G as the scroll bookmark — the chord moved to Alt+G"
        );
        assert_eq!(
            described(KeyCode::Char('g'), KeyModifiers::ALT),
            "Toggle scroll bookmark",
            "{context:?} lost the Alt+G bookmark chord"
        );
    }
    // And the real dispatch arms for the g chords, modifier-exact.
    assert!(dispatch.contains(
        "KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => { self.pending_external_editor = true; }"
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .as_str()
    ));
    assert!(dispatch.contains(
        "KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::ALT) => { self.toggle_scroll_bookmark(); }"
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .as_str()
    ));
}

#[test]
fn test_keybinding_registry_basic() {
    let mut registry = KeyBindingRegistry::new();

    registry.add(KeyBinding {
        key: KeyCode::Char('a'),
        modifiers: KeyModifiers::CONTROL,
        // (iter-1890) MoveCursorEnd, not MoveCursorLeft: the latter is no
        // longer bound by any context (the generic Dialog rows that named it
        // are gone), and constructing it here would make the enum's
        // `expect(dead_code)` unfulfilled in the test target.
        action: KeyAction::MoveCursorEnd,
        context: Some(BindingContext::Prompt),
        description: None,
    });

    let event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    let binding = registry.find(&event, BindingContext::Prompt);
    assert!(binding.is_some());
    assert_eq!(binding.unwrap().action, KeyAction::MoveCursorEnd);
}

#[test]
fn test_global_vs_context_binding() {
    let mut registry = KeyBindingRegistry::new();

    // Global binding
    registry.add(KeyBinding {
        key: KeyCode::F(1),
        modifiers: KeyModifiers::NONE,
        action: KeyAction::ShowHelp,
        context: None,
        description: None,
    });

    // Context-specific binding that overrides
    registry.add(KeyBinding {
        key: KeyCode::F(1),
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Cancel,
        context: Some(BindingContext::Dialog),
        description: None,
    });

    let event = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);

    // In dialog, context-specific binding should win
    let binding = registry.find(&event, BindingContext::Dialog);
    assert!(binding.is_some());
    assert_eq!(binding.unwrap().action, KeyAction::Cancel);

    // In prompt, global binding should apply
    let binding = registry.find(&event, BindingContext::Prompt);
    assert!(binding.is_some());
    assert_eq!(binding.unwrap().action, KeyAction::ShowHelp);
}

#[test]
fn test_default_registry_has_bindings() {
    let registry = KeyBindingRegistry::with_defaults();

    // Check some expected bindings exist
    let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let binding = registry.find(&event, BindingContext::Prompt);
    assert!(binding.is_some());

    let event = KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE);
    let binding = registry.find(&event, BindingContext::VimNormal);
    assert!(binding.is_some());
    assert_eq!(binding.unwrap().action, KeyAction::VimEnterInsert);
}

#[test]
fn test_custom_action() {
    let mut registry = KeyBindingRegistry::new();

    registry.add(KeyBinding {
        key: KeyCode::Char('x'),
        modifiers: KeyModifiers::CONTROL,
        action: KeyAction::Custom(42),
        context: Some(BindingContext::Prompt),
        description: Some("Custom action".to_string()),
    });

    let event = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
    let binding = registry.find(&event, BindingContext::Prompt);
    assert!(binding.is_some());
    assert_eq!(binding.unwrap().action, KeyAction::Custom(42));
}

/// `/keys` and `/hotkeys` read the registry, but the registry does not
/// DISPATCH anything — `app/key_handling.rs` does, and it is still a
/// context-dispatched if-chain rather than a registry lookup. So a binding can
/// be advertised in the help output while pressing it does nothing, which is
/// the worst shape a help screen can have: it asserts a capability that does
/// not exist.
///
/// This pins the Global-context chords, which is the honest scope. Global
/// chords are the ones a user can press from anywhere and expect to work, and
/// they are the ones dispatched in `key_handling.rs`. The other eleven
/// `BindingContext`s deliberately do NOT dispatch there — vim modes live in
/// `prompt_input/vim.rs` and completion has its own module — so grepping
/// `key_handling.rs` for them would fail on correct code.
///
/// WHAT THIS DOES NOT CATCH, stated plainly:
///   - that the right MODIFIER is handled. It checks the key token only, so a
///     binding of `Ctrl+X` is satisfied by an unrelated bare `x` press.
///   - that the dispatch happens in the right BRANCH. `Ctrl+P` appears three
///     times in `key_handling.rs` for three different contexts; this cannot tell
///     them apart.
///   - anything about the other eleven contexts.
///
/// It catches the regression that actually happens: someone deletes or renames
/// a dispatch arm and leaves the catalogue entry behind.
///
/// A real behavioural test would build a `KeyEvent` and assert the resulting
/// `App` state change. That is the right test, and it is not written here
/// because each `App` construction in this crate is a large fixture — the
/// source-level pin is the cheap half of the guarantee, not the whole of it.
#[test]
fn global_bindings_should_be_dispatched_in_key_handling() {
    let dispatch = include_str!("../app/key_handling.rs");
    let globals = DEFAULT_KEYBINDINGS.get_bindings(BindingContext::Global);
    assert!(
        !globals.is_empty(),
        "the Global table is empty — `/keys` would show no global bindings"
    );

    let mut missing = Vec::new();
    for binding in globals {
        // `KeyCode` derives Debug, and for the variants the Global table uses
        // the Debug form IS the source token: `Char('c')`, `F(1)`, `Esc`.
        let token = format!("KeyCode::{:?}", binding.key);
        if !dispatch.contains(&token) {
            missing.push(format!(
                "{token} ({:?}, described as {:?})",
                binding.action, binding.description
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "these Global bindings are advertised by /keys and /hotkeys but no \
         dispatch arm in tui/app/key_handling.rs handles them. Pressing them \
         does nothing while the help screen claims otherwise. Either add the \
         dispatch arm or drop the catalogue entry:\n  {}",
        missing.join("\n  ")
    );
}

/// The MODIFIER half of the same guarantee, which the test above names as a
/// known limit: it accepts a bare `x` arm for an advertised `Ctrl+X`.
///
/// That limit is not theoretical. iter-1758 found `Ctrl+Q → "Quit
/// application"` in the Global table with NO dispatch arm at all, and the
/// token check above passed it because `KeyCode::Char('c')` appears elsewhere
/// in the file — so an operator pressed the chord the help screen promised and
/// nothing happened. The modifier is half of the binding's identity: a
/// catalogue entry for `Ctrl+G` is a promise about the CHORD, not about the
/// letter.
///
/// Two arm shapes count as dispatching a modifier:
///   1. `KeyCode::Char('x') if ... KeyModifiers::CONTROL ...` — the guard names
///      the modifier on the arm or within the next two lines.
///   2. `KeyCode::Char(c) if (c == 'x' || c == 'X') && ... CONTROL ...` — the
///      generic-character arm whose guard names both the character and the
///      modifier (this is how the Ctrl+C arm is written).
///
/// `KeyCode::BackTab` is listed as a known fold: crossterm reports Shift+Tab AS
/// BackTab, so the modifier is not in the token and cannot be checked here.
#[test]
fn global_bindings_should_be_dispatched_with_their_modifier() {
    let dispatch = include_str!("../app/key_handling.rs");
    let lines: Vec<&str> = dispatch.lines().collect();
    let globals = DEFAULT_KEYBINDINGS.get_bindings(BindingContext::Global);

    let mut missing = Vec::new();
    for binding in globals {
        let token = format!("KeyCode::{:?}", binding.key);
        // What modifier this binding promises, if any. A NONE binding has no
        // modifier to check — the existing token test covers it.
        let modifier = if binding.modifiers.contains(KeyModifiers::CONTROL) {
            "CONTROL"
        } else if binding.modifiers.contains(KeyModifiers::ALT) {
            "ALT"
        } else if binding.modifiers.contains(KeyModifiers::SHIFT) {
            "SHIFT"
        } else {
            continue;
        };
        // crossterm folds Shift into BackTab; the modifier is not in the token.
        if token == "KeyCode::BackTab" {
            continue;
        }

        let mut handled = false;
        // The character this binding promises, when it is a character at all —
        // a generic `KeyCode::Char(c)` arm only dispatches THIS chord if its
        // guard names THIS character.
        let promised_char = token
            .strip_prefix("KeyCode::Char('")
            .and_then(|rest| rest.strip_suffix("')"))
            .and_then(|c| c.chars().next());
        for (index, line) in lines.iter().enumerate() {
            if line.contains(&token) {
                // Shape 1: the exact keycode, with the modifier in its guard
                // (on this line or the guard lines that follow it).
                let arm = lines[index..(index + 3).min(lines.len())].join("\n");
                if arm.contains(modifier) {
                    handled = true;
                    break;
                }
                continue;
            }
            // Shape 2: a generic `KeyCode::Char(c)` arm whose guard names the
            // character AND the modifier (this is how the Ctrl+C arm is
            // written: `KeyCode::Char(c) if (c == 'c' || c == 'C') && CONTROL`).
            if line.contains("KeyCode::Char(c)") {
                let wider = lines[index..(index + 5).min(lines.len())].join("\n");
                let names_this_char = promised_char.is_some_and(|c| {
                    wider.contains(&format!("c == '{c}'"))
                        || wider
                            .contains(&format!("c == '{c}' || c == '{}'", c.to_ascii_uppercase()))
                });
                if wider.contains(modifier) && names_this_char {
                    handled = true;
                    break;
                }
            }
        }

        if !handled {
            missing.push(format!(
                "{token} + {modifier} ({:?}, described as {:?})",
                binding.action, binding.description
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "these Global bindings are advertised by /keys and /hotkeys with a \
         MODIFIER, but no arm in tui/app/key_handling.rs dispatches that \
         chord — a bare-key arm is not the same promise. Pressing the chord \
         does nothing while the help screen claims otherwise:\n  {}",
        missing.join("\n  ")
    );
}

/// Every `VimNormal` entry in the catalogue must correspond to a string the vim
/// state machine actually matches on.
///
/// iter-426 exists because this check was missing and the drift was REAL: the
/// catalogue advertised `VimMotionDown` / `VimMotionUp` for `j` and `k`, and
/// pressing them did nothing — no arm in either dispatch file, and no test
/// anywhere covering vertical motion. `/keys` was describing keys that no path
/// could reach.
///
/// The scope is deliberately loose — a COMPLETE string literal in either
/// dispatch file — and that is a correction, not laziness. A first attempt
/// pinned `match` arms in `prompt_input/vim.rs` alone and reported nine
/// discrepancies. Reading each one:
///   - `Escape` and `"` were false positives: the first is an `if key == "…"`
///     at `apply_vim_key`, the second tripped the extractor's escaping.
///   - `:`, `/`, `V` and `.` are real, and all four are handled in
///     `prompt_input/vim_command.rs` by the same `if key == "…"` shape.
///   - exactly `j` and `k` were genuinely missing from BOTH files.
///
/// So the state machine spans two files and two dispatch shapes. Matching
/// complete literals rather than arm grammar also means this survives
/// reformatting, an arm being split or combined, or a key moving between the
/// match and the equality tests — none of which are the defect being guarded.
///
/// Extended across the visual, command and search contexts after auditing each
/// remaining one: all 33 of their `Char`/`Esc` entries resolve in the same two
/// files, and the registry has no unreachable duplicate (the `Tab`+NONE vs
/// `Tab`+SHIFT and `v`+NONE vs `V`+SHIFT pairs are disambiguated by modifiers,
/// so `find()` reaches both).
#[test]
fn every_string_dispatched_vim_binding_is_reachable() {
    let registry = KeyBindingRegistry::with_defaults();
    let sources = [
        include_str!("../prompt_input/vim.rs"),
        include_str!("../prompt_input/vim_command.rs"),
    ];

    // Complete literals only, with comments skipped so a commented-out arm
    // cannot satisfy this. A plain `contains` is satisfied by prose.
    let live_literals: Vec<String> = sources
        .iter()
        .flat_map(|src| rust_string_literals(src))
        .collect();

    // Arrow keys are EXCLUDED on purpose, and it is worth being explicit about
    // why rather than leaving a silent hole. The vim state machine receives a
    // `&str`, so a literal is the right check for it — but the arrow bindings
    // advertised by the three visual contexts are dispatched by the CALLER on
    // `KeyCode`, at `key_handling.rs`'s `KeyCode::Up` arm, which checks SHIFT
    // plus a visual-mode match and calls `move_visual_up`. They appear as no
    // literal in either vim file, so including them here would report four false
    // positives per visual context.
    const STRING_DISPATCHED: [BindingContext; 6] = [
        BindingContext::VimNormal,
        BindingContext::VimVisual,
        BindingContext::VimVisualLine,
        BindingContext::VimVisualBlock,
        BindingContext::VimCommand,
        BindingContext::VimSearch,
    ];

    let mut missing = Vec::new();
    let mut checked = 0usize;
    for context in STRING_DISPATCHED {
        for binding in registry.get_bindings(context) {
            // The catalogue is `KeyCode`-shaped; the dispatcher is `&str`-shaped.
            // `Esc` and `Enter` both have string forms in the two files — the
            // first is an `if key == "Escape"` at `apply_vim_key`, the second a
            // pair of literals in `vim_command.rs` backing "Execute command" and
            // "Next match".
            let wanted = match binding.key {
                KeyCode::Char(c) => c.to_string(),
                KeyCode::Esc => "Escape".to_string(),
                KeyCode::Enter => "Enter".to_string(),
                _ => continue,
            };
            checked += 1;
            if !live_literals.contains(&wanted) {
                missing.push(format!(
                    "{:?} {context:?} {} — \"{wanted}\" appears in no match arm or \
                     `if key ==` in prompt_input/vim.rs or \
                     prompt_input/vim_command.rs",
                    binding.key, binding.modifiers
                ));
            }
        }
    }

    // A pin that silently stops covering its contexts is the failure mode this
    // assertion exists to prevent, and a HARDCODED floor goes stale the moment a
    // binding is added. So the bound is derived from the registry: the pin must
    // have examined the large majority of the bindings in these six contexts.
    // It is `<`, not `==`, because the arrow keys are deliberately excluded.
    let total: usize = STRING_DISPATCHED
        .iter()
        .map(|context| registry.get_bindings(*context).len())
        .sum();
    assert!(
        checked * 2 >= total,
        "the pin examined only {checked} of {total} bindings across the vim \
         contexts — too few for a green result here to mean anything"
    );

    assert!(
        missing.is_empty(),
        "these vim bindings are advertised by /keys but unreachable by the \
         state machine. Pressing them does nothing while the help screen \
         claims otherwise. Either add the dispatch or drop the catalogue \
         entry:\n  {}",
        missing.join("\n  ")
    );
}

/// Collect every complete string literal in Rust source, skipping comments,
/// char literals and lifetimes.
///
/// A tokenizer rather than `split('"')`, and the reason is specific: vim.rs
/// contains `'"' => {`, a CHAR literal holding a double quote. Naive
/// quote-pairing treats that quote as a string delimiter and desynchronises for
/// the rest of the file — which made an earlier version of this pin report `j`,
/// `k`, `m`, `'` and `"` as unreachable when all five are live arms. Skipping
/// char literals also keeps lifetimes (`&'a`) from being read as quotes, and
/// skipping comments keeps prose from being read as an arm.
fn rust_string_literals(src: &str) -> Vec<String> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(c.len());
            }
            // A char literal closes within one or two characters; a lifetime
            // does not, and that is the whole test for telling them apart.
            '\'' => {
                let closes = match c.get(i + 1) {
                    Some('\\') => c.get(i + 3) == Some(&'\''),
                    Some(_) => c.get(i + 2) == Some(&'\''),
                    None => false,
                };
                if closes {
                    i += 4;
                } else {
                    i += 1;
                }
            }
            '"' => {
                // Unescape as we go: the register arm is written `"\"" =>`, so
                // the raw body is `\"` and a raw comparison would report the one
                // binding whose key IS a quote as unreachable. That was a real
                // false positive here, not a hypothetical.
                let mut lit = String::new();
                i += 1;
                while i < c.len() && c[i] != '"' {
                    if c[i] == '\\' && i + 1 < c.len() {
                        i += 1;
                        lit.push(match c[i] {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            '0' => '\0',
                            other => other,
                        });
                    } else {
                        lit.push(c[i]);
                    }
                    i += 1;
                }
                out.push(lit);
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// The extractor must actually be a tokenizer, or every pin built on it is
/// decorative — and worse, actively wrong. Both failure modes below were real:
/// a commented-out arm satisfying a `contains`, and `'"' =>` desynchronising
/// quote-pairing so live arms looked absent.
#[test]
fn the_literal_extractor_ignores_comments_quotes_in_chars_and_lifetimes() {
    // A commented-out arm must not count.
    assert!(!rust_string_literals("let a = 1; // \"k\" => done").contains(&"k".to_string()));
    assert!(!rust_string_literals("/* \"k\" => done */ let a = 1;").contains(&"k".to_string()));
    // The char literal that broke the naive version. Live code AFTER it must
    // still be found, and `"` must not appear as a literal of its own.
    let tricky = "match k { '\"' => 1, \"j\" => 2, _ => 0 }";
    let found = rust_string_literals(tricky);
    assert!(
        found.contains(&"j".to_string()),
        "live literal after a quote-in-char was lost: {found:?}"
    );
    // An escaped quote must UNESCAPE, or the register arm `"\"" =>` — whose key
    // IS a literal quote — is reported unreachable. This was a real false
    // positive, not a hypothetical one.
    assert!(
        rust_string_literals("match k { \"\\\"\" => 1 }").contains(&"\"".to_string()),
        "an escaped quote was not unescaped: {:?}",
        rust_string_literals("match k { \"\\\"\" => 1 }")
    );
    // A lifetime is not a quote.
    let lifetime = "fn f(x: &'a str) -> &'a str { x }";
    assert!(
        rust_string_literals(lifetime).is_empty(),
        "a lifetime was misread as a quote: {:?}",
        rust_string_literals(lifetime)
    );
}

#[test]
fn the_default_keymap_has_no_conflicts() {
    // One key, one action, per context — including Global bindings, which
    // apply in every context. Deliberate overrides are listed here.
    let allowed_overrides = [
        (
            BindingContext::VimCommand,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ),
        (
            BindingContext::VimSearch,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ),
    ];
    let contexts = [
        BindingContext::Global,
        BindingContext::Prompt,
        BindingContext::Transcript,
        BindingContext::Dialog,
        BindingContext::VimNormal,
        BindingContext::VimInsert,
        BindingContext::VimVisual,
        BindingContext::VimVisualLine,
        BindingContext::VimVisualBlock,
        BindingContext::VimCommand,
        BindingContext::VimSearch,
        BindingContext::Completion,
    ];
    let mut conflicts = Vec::new();
    for context in contexts {
        let mut seen: std::collections::HashMap<(KeyCode, KeyModifiers), &KeyBinding> =
            std::collections::HashMap::new();
        for binding in DEFAULT_KEYBINDINGS.get_bindings(context) {
            let key = (binding.key, binding.modifiers);
            if let Some(prev) = seen.get(&key) {
                if prev.action != binding.action
                    && !allowed_overrides.contains(&(context, binding.key, binding.modifiers))
                {
                    conflicts.push(format!(
                        "{context:?} {:?}+{:?}: {:?} ({:?}) vs {:?} ({:?})",
                        binding.modifiers,
                        binding.key,
                        prev.action,
                        prev.description,
                        binding.action,
                        binding.description
                    ));
                }
            } else {
                seen.insert(key, binding);
            }
        }
    }
    assert!(
        conflicts.is_empty(),
        "conflicting bindings:\n  {}",
        conflicts.join("\n  ")
    );
}
/// (iter-1957) One chord in one context may carry exactly one claim.
///
/// `register` PUSHES, and `find` returns the FIRST match, so two entries for
/// the same chord in the same context leave the second unreachable while
/// `/keys` and `/hotkeys` render BOTH. The operator then sees one chord with
/// two promised effects and usually gets only one of them.
///
/// This was real: `Ctrl+O` was registered twice in `Global` — "Expand latest
/// tool output" and "Toggle tool-call visibility". The arm is
/// `toggle_latest_tool_expansion()`, and no `toggle_tool_call_visibility`
/// exists anywhere in the crate, so the second entry promised a mechanism that
/// does not exist while `/keys` displayed it as a live binding.
///
/// The vim sweep (see `every_string_dispatched_vim_binding_is_reachable`)
/// states the rule for its own contexts — "the registry has no unreachable
/// duplicate" — without a machine check. This is that check, for every context.
#[test]
fn no_context_registers_one_chord_twice() {
    let mut seen: std::collections::HashMap<
        (
            crate::tui::keybindings::BindingContext,
            crossterm::event::KeyCode,
            crossterm::event::KeyModifiers,
        ),
        Vec<String>,
    > = std::collections::HashMap::new();
    for context in crate::tui::keybindings::BindingContext::ALL {
        for binding in DEFAULT_KEYBINDINGS.get_bindings(context) {
            seen.entry((context, binding.key, binding.modifiers))
                .or_default()
                .push(binding.description.clone().unwrap_or_default());
        }
    }
    let dupes: Vec<String> = seen
        .iter()
        .filter(|(_, descriptions)| descriptions.len() > 1)
        .map(|((context, key, modifiers), descriptions)| {
            format!(
                "{:?} + {:?} in {:?}: {} entries, described as {:?} —                  register() pushes them all but find() returns the first",
                modifiers, key, context,
                descriptions.len(), descriptions
            )
        })
        .collect();
    assert!(
        dupes.is_empty(),
        "these chords are registered more than once in the SAME context. /keys          lists every entry, but only the first one dispatches:
  {}",
        dupes.join("\n  ")
    );
}
/// (iter-1958) Every chord the catalogue advertises must be CONSUMED.
///
/// The audit has pinned the catalogue's tokens source-to-source — the Global
/// context by `global_bindings_should_be_dispatched_in_key_handling` and its
/// modifier half, the vim contexts by
/// `every_string_dispatched_vim_binding_is_reachable`. Both share a limit their
/// own doc comments name: a bare `x` arm can satisfy a chord advertised as
/// `Ctrl+X`, so "the token appears somewhere in the file" is not the same
/// claim as "this chord does something".
///
/// This asks the CODE instead. The composer's contract is that an UNHANDLED key
/// inserts its character, so an advertised chord that dispatches nothing is
/// observable without knowing anything about dispatch: the composer gains the
/// letter. Driving the real `handle_key_event` and requiring the composer to
/// stay empty therefore tests every catalogued chord at once, for the state the
/// operator is actually in when they read `/keys`.
///
/// Chords that legitimately insert text (vim modes, completion) are driven in
/// the state that owns them, so a legitimate handler still consumes the key.
#[test]
fn no_catalogued_chord_types_itself_into_the_composer() {
    let mut leakers: Vec<String> = Vec::new();
    // Scoped to the contexts WITHOUT their own dispatch pin. The vim contexts
    // have `every_string_dispatched_vim_binding_is_reachable` — the check that
    // found iter-426's real `j`/`k` defect — and duplicating it here would mean
    // constructing the vim state per context, which is this test's own trap:
    // in Insert mode `h`, `j` and `.` legitimately insert themselves, so a wrong
    // `vim_mode` manufactures a leak out of a working handler.
    use crate::tui::keybindings::BindingContext;
    for context in [
        BindingContext::Global,
        BindingContext::Prompt,
        BindingContext::Dialog,
        BindingContext::Completion,
    ] {
        for binding in DEFAULT_KEYBINDINGS.get_bindings(context) {
            let mut app = super::super::app::tests::make_app();
            // An EMPTY composer for every context, so the only text that can
            // appear is text the key itself produced.
            app.prompt_input.text = String::new();
            // The operator's state when reading /keys: an empty composer, no
            // vim mode, no selection. No per-context state construction.
            app.prompt_input.visual_anchor = None;

            app.handle_key_event(crossterm::event::KeyEvent::new(
                binding.key,
                binding.modifiers,
            ));

            let gained = app.prompt_input.text.as_str();
            if gained.is_empty() {
                continue;
            }
            // The composer gained text. That is a leak ONLY IF the text is the
            // key's own character: a handler that legitimately writes the
            // composer (history recall filling it from a remembered prompt, a
            // completion filling the word) writes something else, and that is
            // the handler doing its advertised job.
            let key_char = match binding.key {
                crossterm::event::KeyCode::Char(c) => Some(c),
                _ => None,
            };
            let Some(key_char) = key_char else { continue };
            let typed_itself = gained == key_char.to_string()
                || gained == key_char.to_ascii_uppercase().to_string();
            if !typed_itself {
                continue;
            }
            let desc = binding.description.clone().unwrap_or_default();
            let verb = desc.split_whitespace().next().unwrap_or("").to_lowercase();
            let is_an_insert = desc.starts_with("Enter ")
                || desc.starts_with("Insert ")
                || desc.starts_with("Type ")
                || verb == "insert";
            if !is_an_insert {
                leakers.push(format!(
                    "{:?} + {:?} in {:?} advertised as {:?} but the composer kept \
                     the character: {:?}",
                    binding.modifiers, binding.key, context, desc, app.prompt_input.text
                ));
            }
        }
    }
    assert!(
        leakers.is_empty(),
        "these chords are advertised by /keys but nothing consumed them — the key \
         fell through to the composer and typed itself. Either wire the handler or \
         drop the catalogue entry:\n  {}",
        leakers.join("\n  ")
    );
}
