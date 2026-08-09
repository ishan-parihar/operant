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
