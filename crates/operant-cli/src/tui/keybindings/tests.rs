// keybindings/tests.rs — Unit tests for the keybinding registry.
//
// Extracted from the keybindings.rs monolith.

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn test_keybinding_registry_basic() {
    let mut registry = KeyBindingRegistry::new();

    registry.add(KeyBinding {
        key: KeyCode::Char('a'),
        modifiers: KeyModifiers::CONTROL,
        action: KeyAction::MoveCursorLeft,
        context: Some(BindingContext::Prompt),
        description: None,
    });

    let event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    let binding = registry.find(&event, BindingContext::Prompt);
    assert!(binding.is_some());
    assert_eq!(binding.unwrap().action, KeyAction::MoveCursorLeft);
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
fn test_fallback_contexts() {
    let registry = KeyBindingRegistry::with_defaults();
    let event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

    // Should find binding in dialog context
    let binding =
        registry.find_with_fallback(&event, &[BindingContext::Dialog, BindingContext::Global]);
    assert!(binding.is_some());
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
fn test_remove_binding() {
    let mut registry = KeyBindingRegistry::new();

    registry.add(KeyBinding {
        key: KeyCode::Char('x'),
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Cancel,
        context: Some(BindingContext::Prompt),
        description: None,
    });

    let event = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(registry.find(&event, BindingContext::Prompt).is_some());

    let removed = registry.remove(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
        Some(BindingContext::Prompt),
    );
    assert!(removed);
    assert!(registry.find(&event, BindingContext::Prompt).is_none());
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
