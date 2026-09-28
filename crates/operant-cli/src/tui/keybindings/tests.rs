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
