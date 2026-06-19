use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Shortcut {
    Copy,
    Paste,
    Interrupt,
    Exit,
    Clear,
    Redraw,
    Home,
    End,
    HistoryUp,
    HistoryDown,
    Tab,
    ShiftTab,
    Enter,
    Escape,
    Backspace,
    Delete,
    CtrlC,
    CtrlD,
    CtrlL,
    CtrlR,
    CtrlA,
    CtrlE,
    CtrlW,
    CtrlU,
    CtrlK,
    ShiftEnter,
}

pub fn resolve_shortcut(key: KeyEvent) -> Option<Shortcut> {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlC)
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlD)
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlL)
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlR)
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlA)
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlE)
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlW)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlU)
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Shortcut::CtrlK)
        }
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => Some(Shortcut::ShiftEnter),
        KeyCode::Enter => Some(Shortcut::Enter),
        KeyCode::Esc => Some(Shortcut::Escape),
        KeyCode::Backspace => Some(Shortcut::Backspace),
        KeyCode::Delete => Some(Shortcut::Delete),
        KeyCode::Tab => Some(Shortcut::Tab),
        KeyCode::BackTab => Some(Shortcut::ShiftTab),
        KeyCode::Up => Some(Shortcut::HistoryUp),
        KeyCode::Down => Some(Shortcut::HistoryDown),
        KeyCode::Home => Some(Shortcut::Home),
        KeyCode::End => Some(Shortcut::End),
        _ => None,
    }
}

pub fn is_copy_shortcut(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

pub fn is_paste_shortcut(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('v') && key.modifiers.contains(KeyModifiers::CONTROL)
}

pub fn is_exit_shortcut(key: KeyEvent) -> bool {
    (key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL))
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

pub fn is_interrupt_shortcut(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(resolve_shortcut(key), Some(Shortcut::CtrlC));
    }

    #[test]
    fn test_resolve_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(resolve_shortcut(key), Some(Shortcut::Enter));
    }

    #[test]
    fn test_resolve_escape() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(resolve_shortcut(key), Some(Shortcut::Escape));
    }

    #[test]
    fn test_is_exit_shortcut() {
        let key = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(is_exit_shortcut(key));
    }

    #[test]
    fn test_is_interrupt_shortcut() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(is_interrupt_shortcut(key));
    }
}
