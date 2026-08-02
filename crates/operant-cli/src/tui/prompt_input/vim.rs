//! Vim mode types, motions, and key handler.

//! Complete PromptInput — multi-line text editor for the TUI.
//! Mirrors src/components/PromptInput/ (21 files) and src/vim/ (5 files).
//!
//! Features:
//! - Multi-line editing (Shift+Enter for newlines)
//! - Vim Normal/Insert/Visual modes
//! - History navigation (↑↓ through history.jsonl)
//! - Slash command typeahead
//! - Paste handling (large pastes → placeholder)
//! - Character count + token estimate

// ---------------------------------------------------------------------------
// Vim mode
// ---------------------------------------------------------------------------

/// Vim editor mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VimMode {
    #[default]
    Insert,
    Normal,
    Visual,
    /// Linewise visual selection (V).
    VisualLine,
    /// Block visual selection (Ctrl+V).
    VisualBlock,
    /// Command-line mode (:).
    Command,
    /// In-prompt forward search (/).
    Search,
}

impl VimMode {
    #[allow(dead_code)] // Vim mode label display
    pub fn label(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Normal => "NORMAL",
            Self::Visual => "VISUAL",
            Self::VisualLine => "VISUAL LINE",
            Self::VisualBlock => "VISUAL BLOCK",
            Self::Command => "COMMAND",
            Self::Search => "SEARCH",
        }
    }
}

// ---------------------------------------------------------------------------
// Extended vim state types (full state machine)
// ---------------------------------------------------------------------------

/// Pending multi-key vim command state.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum VimPendingState {
    #[default]
    None,
    /// Accumulating count digits before a command (e.g. `3` before `w`).
    Count { digits: String },
    /// Received `g`, waiting for second key.
    G { count: usize },
    /// Received operator (d/c/y), waiting for motion.
    Operator { op: VimOperator, count: usize },
    /// Received operator then additional count digits.
    OperatorCount {
        op: VimOperator,
        count: usize,
        digits: String,
    },
    /// Received `dg`/`cg`/`yg`, waiting for second g key.
    OperatorG { op: VimOperator, count: usize },
    /// Received `f/F/t/T`, waiting for target char.
    Find { kind: VimFindKind, count: usize },
    /// Received `r`, waiting for replacement char.
    Replace { count: usize },
    /// Received `>` or `<`, waiting for second `>` or `<`.
    Indent { dir: char, count: usize },
    /// Received `"`, waiting for register name char.
    Register(char),
    /// After `"reg`, waiting for operator (y/d/p).
    RegisterOp(char),
    /// Received `m`, waiting for mark name char.
    Mark,
    /// Received `'`, waiting for mark name char for jump.
    JumpMark,
    /// Received `q`, waiting for register char to record into.
    MacroRecord,
    /// Received `@`, waiting for register char to replay.
    MacroReplay,
}

/// Vim operator type used with motion + operator combos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimOperator {
    Delete,
    Change,
    Yank,
    /// Uppercase region (gU).
    Uppercase,
    /// Lowercase region (gu).
    Lowercase,
}

/// Vim character-find direction and variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimFindKind {
    /// `f{c}` — forward, cursor lands on char
    F,
    /// `F{c}` — backward, cursor lands on char
    BigF,
    /// `t{c}` — forward, cursor stops before char
    T,
    /// `T{c}` — backward, cursor stops after char
    BigT,
}

/// Stores enough information to replay the last modifying vim command (`.`).
#[derive(Clone, Debug)]
pub enum DotRepeatAction {
    /// Insert text at current cursor (from i, a, A, o, O, s).
    Insert { text: String },
    /// Simplified: re-delete the same number of chars.
    DeleteChars { count: usize },
    /// Replace char.
    ReplaceChar { ch: char },
}

// ---------------------------------------------------------------------------
// Motion helper functions (byte-safe, work on UTF-8 byte offsets)
// ---------------------------------------------------------------------------

pub(super) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Convert a char-index within `text` to a byte offset.
pub(super) fn char_idx_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

/// `w` — start of next word.
pub(super) fn motion_w(text: &str, cursor: usize) -> usize {
    let rest = &text[cursor..];
    let chars: Vec<char> = rest.chars().collect();
    let n = chars.len();
    if n == 0 {
        return cursor;
    }
    let mut i = 0;
    if is_word_char(chars[0]) {
        while i < n && is_word_char(chars[i]) {
            i += 1;
        }
    } else if !chars[0].is_whitespace() {
        while i < n && !is_word_char(chars[i]) && !chars[i].is_whitespace() {
            i += 1;
        }
    }
    while i < n && chars[i].is_whitespace() {
        i += 1;
    }
    cursor + char_idx_to_byte(rest, i)
}

/// `b` — start of previous word.
pub(super) fn motion_b(text: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let before = &text[..cursor];
    let chars: Vec<char> = before.chars().collect();
    let n = chars.len();
    if n == 0 {
        return 0;
    }
    let mut i = n;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    if is_word_char(chars[i - 1]) {
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
    } else {
        while i > 0 && !is_word_char(chars[i - 1]) && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
    }
    char_idx_to_byte(before, i)
}

/// `e` — end of current/next word.
pub(super) fn motion_e(text: &str, cursor: usize) -> usize {
    let chars: Vec<(usize, char)> = text[cursor..]
        .char_indices()
        .map(|(b, c)| (cursor + b, c))
        .collect();
    let n = chars.len();
    if n == 0 {
        return cursor;
    }
    let at_end = n == 1
        || chars[1].1.is_whitespace()
        || is_word_char(chars[0].1) != is_word_char(chars[1].1);
    let mut i = 0;
    if at_end {
        i = 1;
        while i < n && chars[i].1.is_whitespace() {
            i += 1;
        }
    }
    if i >= n {
        return cursor;
    }
    let wc = is_word_char(chars[i].1);
    while i + 1 < n && !chars[i + 1].1.is_whitespace() && is_word_char(chars[i + 1].1) == wc {
        i += 1;
    }
    chars[i].0
}

/// `W` — start of next WORD (any non-whitespace run).
#[allow(non_snake_case)]
pub(super) fn motion_W(text: &str, cursor: usize) -> usize {
    let rest = &text[cursor..];
    let chars: Vec<char> = rest.chars().collect();
    let n = chars.len();
    if n == 0 {
        return cursor;
    }
    let mut i = 0;
    while i < n && !chars[i].is_whitespace() {
        i += 1;
    }
    while i < n && chars[i].is_whitespace() {
        i += 1;
    }
    cursor + char_idx_to_byte(rest, i)
}

/// `B` — start of previous WORD.
#[allow(non_snake_case)]
pub(super) fn motion_B(text: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let before = &text[..cursor];
    let chars: Vec<char> = before.chars().collect();
    let n = chars.len();
    let mut i = n;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    char_idx_to_byte(before, i)
}

/// `E` — end of current/next WORD.
#[allow(non_snake_case)]
pub(super) fn motion_E(text: &str, cursor: usize) -> usize {
    let chars: Vec<(usize, char)> = text[cursor..]
        .char_indices()
        .map(|(b, c)| (cursor + b, c))
        .collect();
    let n = chars.len();
    if n == 0 {
        return cursor;
    }
    let at_end = n == 1 || chars[1].1.is_whitespace();
    let mut i = 0;
    if at_end {
        i = 1;
        while i < n && chars[i].1.is_whitespace() {
            i += 1;
        }
    }
    if i >= n {
        return cursor;
    }
    while i + 1 < n && !chars[i + 1].1.is_whitespace() {
        i += 1;
    }
    chars[i].0
}

/// `^` — first non-blank character on the current line.
pub(super) fn motion_first_nonblank(text: &str, cursor: usize) -> usize {
    let line_start = text[..cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let rest = &text[line_start..];
    let skip_bytes = rest
        .char_indices()
        .take_while(|(_, c)| *c == ' ' || *c == '\t')
        .last()
        .map(|(b, c)| b + c.len_utf8())
        .unwrap_or(0);
    line_start + skip_bytes
}

/// `G` — first char of the last line.
#[allow(non_snake_case)]
pub(super) fn motion_G(text: &str) -> usize {
    text.rfind('\n').map(|p| p + 1).unwrap_or(0)
}

/// `gg` / line-N — go to start of line `line_num` (1-indexed; 0 or 1 → start of text).
pub(super) fn motion_gg(text: &str, line_num: usize) -> usize {
    if line_num <= 1 {
        return 0;
    }
    let mut line = 1usize;
    for (b, c) in text.char_indices() {
        if c == '\n' {
            line += 1;
            if line == line_num {
                return b + 1;
            }
        }
    }
    text.rfind('\n').map(|p| p + 1).unwrap_or(0)
}

/// `f/F/t/T{char}` — find character in text. Returns new cursor byte offset.
pub(super) fn motion_find_char(
    text: &str,
    cursor: usize,
    target: char,
    kind: VimFindKind,
    count: usize,
) -> Option<usize> {
    match kind {
        VimFindKind::F | VimFindKind::T => {
            let search_start = text[cursor..]
                .char_indices()
                .nth(1)
                .map(|(b, _)| cursor + b)?;
            let mut hits = 0usize;
            for (b, c) in text[search_start..].char_indices() {
                if c == target {
                    hits += 1;
                    if hits == count {
                        let pos = search_start + b;
                        if matches!(kind, VimFindKind::T) {
                            return text[cursor..pos]
                                .char_indices()
                                .last()
                                .map(|(lb, _)| cursor + lb);
                        }
                        return Some(pos);
                    }
                }
            }
            None
        }
        VimFindKind::BigF | VimFindKind::BigT => {
            let before = &text[..cursor];
            let mut hits = 0usize;
            for (b, c) in before.char_indices().rev() {
                if c == target {
                    hits += 1;
                    if hits == count {
                        if matches!(kind, VimFindKind::BigT) {
                            return text[b..]
                                .char_indices()
                                .nth(1)
                                .map(|(nb, _)| b + nb)
                                .or(Some(cursor));
                        }
                        return Some(b);
                    }
                }
            }
            None
        }
    }
}

/// Convert text region to uppercase.
pub(super) fn uppercase_region(text: &str) -> String {
    text.chars()
        .map(|c| c.to_uppercase().next().unwrap_or(c))
        .collect()
}

/// Convert text region to lowercase.
pub(super) fn lowercase_region(text: &str) -> String {
    text.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

/// Apply an operator (d/c/y/gU/gu) to the range [from, to) in text.
/// Returns `(new_text, new_cursor)`. For Change, sets mode to Insert.
pub(super) fn apply_operator_range(
    op: VimOperator,
    text: &str,
    from: usize,
    to: usize,
    yank_buf: &mut String,
    mode: &mut VimMode,
) -> (String, usize) {
    let to = to.min(text.len());
    let from = from.min(to);
    let selected = &text[from..to];
    *yank_buf = selected.to_string();
    match op {
        VimOperator::Yank => (text.to_string(), from),
        VimOperator::Delete => {
            let new_text = format!("{}{}", &text[..from], &text[to..]);
            let new_cursor =
                from.min(
                    new_text
                        .len()
                        .saturating_sub(if new_text.is_empty() { 0 } else { 1 }),
                );
            (new_text, new_cursor)
        }
        VimOperator::Change => {
            let new_text = format!("{}{}", &text[..from], &text[to..]);
            *mode = VimMode::Insert;
            (new_text, from)
        }
        VimOperator::Uppercase => {
            let upper = uppercase_region(selected);
            let new_text = format!("{}{}{}", &text[..from], upper, &text[to..]);
            (new_text, from)
        }
        VimOperator::Lowercase => {
            let lower = lowercase_region(selected);
            let new_text = format!("{}{}{}", &text[..from], lower, &text[to..]);
            (new_text, from)
        }
    }
}

// ---------------------------------------------------------------------------
// Full vim key handler (state machine)
// ---------------------------------------------------------------------------

/// Process a single key press in vim mode.
/// Returns `true` when text was modified (caller should push undo snapshot).
pub fn apply_vim_key(
    mode: &mut VimMode,
    text: &mut String,
    cursor: &mut usize,
    key: &str,
    yank_buf: &mut String,
    pending: &mut VimPendingState,
    last_find: &mut Option<(VimFindKind, char)>,
) -> bool {
    // Escape always cancels pending state and returns to Normal
    if key == "Escape" {
        *mode = VimMode::Normal;
        *pending = VimPendingState::None;
        return false;
    }

    match std::mem::replace(pending, VimPendingState::None) {
        VimPendingState::None => vim_idle(mode, text, cursor, key, yank_buf, pending, last_find),
        VimPendingState::Count { digits } => vim_count(
            mode, text, cursor, key, yank_buf, pending, last_find, digits,
        ),
        VimPendingState::G { count } => vim_g(text, cursor, key, pending, count),
        VimPendingState::Operator { op, count } => vim_operator(
            mode, text, cursor, key, yank_buf, pending, last_find, op, count,
        ),
        VimPendingState::OperatorCount { op, count, digits } => vim_operator_count(
            mode, text, cursor, key, yank_buf, pending, last_find, op, count, digits,
        ),
        VimPendingState::OperatorG { op, count } => {
            vim_operator_g(mode, text, cursor, key, yank_buf, op, count)
        }
        VimPendingState::Find { kind, count } => {
            if key.len() == 1 {
                let c = key.chars().next().unwrap();
                if let Some(new_pos) = motion_find_char(text, *cursor, c, kind, count) {
                    *cursor = new_pos;
                    *last_find = Some((kind, c));
                }
            }
            false
        }
        VimPendingState::Replace { count } => {
            if key.len() == 1 {
                let c = key.chars().next().unwrap();
                let mut modified = false;
                let mut pos = *cursor;
                for _ in 0..count.max(1) {
                    if pos >= text.len() {
                        break;
                    }
                    let clen = text[pos..]
                        .chars()
                        .next()
                        .map(|ch| ch.len_utf8())
                        .unwrap_or(1);
                    text.replace_range(pos..pos + clen, &c.to_string());
                    pos += c.len_utf8();
                    modified = true;
                }
                *cursor =
                    (*cursor).min(
                        text.len()
                            .saturating_sub(if text.is_empty() { 0 } else { 1 }),
                    );
                modified
            } else {
                false
            }
        }
        VimPendingState::Indent { dir, count } => {
            if key == dir.to_string().as_str() {
                let indent = "  ";
                let current_line = text[..*cursor].chars().filter(|&c| c == '\n').count();
                let mut new_lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
                for i in 0..count.max(1) {
                    let idx = current_line + i;
                    if idx >= new_lines.len() {
                        break;
                    }
                    if dir == '>' {
                        new_lines[idx] = format!("{}{}", indent, new_lines[idx]);
                    } else if new_lines[idx].starts_with(indent) {
                        new_lines[idx] = new_lines[idx][indent.len()..].to_string();
                    } else {
                        let trimmed = new_lines[idx]
                            .trim_start_matches('\t')
                            .trim_start_matches(' ');
                        new_lines[idx] = trimmed.to_string();
                    }
                }
                *text = new_lines.join("\n");
                *cursor = (*cursor).min(text.len());
                true
            } else {
                false
            }
        }
        // These pending states are fully handled in PromptInputState::vim_command
        // before apply_vim_key is called, but we need arms for exhaustiveness.
        VimPendingState::Register(_)
        | VimPendingState::RegisterOp(_)
        | VimPendingState::Mark
        | VimPendingState::JumpMark
        | VimPendingState::MacroRecord
        | VimPendingState::MacroReplay => false,
    }
}

pub(super) fn vim_idle(
    mode: &mut VimMode,
    text: &mut String,
    cursor: &mut usize,
    key: &str,
    yank_buf: &mut String,
    pending: &mut VimPendingState,
    last_find: &mut Option<(VimFindKind, char)>,
) -> bool {
    // Count prefix (1-9 only; 0 is the line-start motion)
    if key.len() == 1 {
        let ch = key.chars().next().unwrap();
        if ch.is_ascii_digit() && ch != '0' {
            *pending = VimPendingState::Count {
                digits: key.to_string(),
            };
            return false;
        }
    }
    vim_normal(mode, text, cursor, key, yank_buf, pending, last_find, 1)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn vim_count(
    mode: &mut VimMode,
    text: &mut String,
    cursor: &mut usize,
    key: &str,
    yank_buf: &mut String,
    pending: &mut VimPendingState,
    last_find: &mut Option<(VimFindKind, char)>,
    digits: String,
) -> bool {
    if key.len() == 1 && key.chars().next().unwrap().is_ascii_digit() {
        let new_digits = format!("{}{}", digits, key);
        let count: usize = new_digits.parse().unwrap_or(10000).min(10000);
        *pending = VimPendingState::Count {
            digits: count.to_string(),
        };
        return false;
    }
    let count: usize = digits.parse().unwrap_or(1);
    vim_normal(mode, text, cursor, key, yank_buf, pending, last_find, count)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn vim_normal(
    mode: &mut VimMode,
    text: &mut String,
    cursor: &mut usize,
    key: &str,
    yank_buf: &mut String,
    pending: &mut VimPendingState,
    last_find: &mut Option<(VimFindKind, char)>,
    count: usize,
) -> bool {
    let n = count.max(1);
    match key {
        // ---- Mode transitions ----
        "i" => {
            *mode = VimMode::Insert;
            false
        }
        "a" => {
            *mode = VimMode::Insert;
            if *cursor < text.len() {
                *cursor = text[*cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(b, _)| *cursor + b)
                    .unwrap_or(text.len());
            }
            false
        }
        "I" => {
            *mode = VimMode::Insert;
            *cursor = motion_first_nonblank(text, *cursor);
            false
        }
        "A" => {
            *mode = VimMode::Insert;
            *cursor = text[*cursor..]
                .find('\n')
                .map(|p| *cursor + p)
                .unwrap_or(text.len());
            false
        }
        "v" => {
            *mode = VimMode::Visual;
            false
        }
        // ---- Simple motions ----
        "h" => {
            for _ in 0..n {
                if *cursor > 0 {
                    let prev = text[..*cursor]
                        .char_indices()
                        .last()
                        .map(|(b, _)| b)
                        .unwrap_or(0);
                    *cursor = prev;
                }
            }
            false
        }
        "l" => {
            for _ in 0..n {
                if *cursor < text.len() {
                    *cursor = text[*cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(b, _)| *cursor + b)
                        .unwrap_or(text.len());
                }
            }
            false
        }
        "0" => {
            *cursor = text[..*cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
            false
        }
        "^" => {
            *cursor = motion_first_nonblank(text, *cursor);
            false
        }
        "$" => {
            *cursor = text[*cursor..]
                .find('\n')
                .map(|p| *cursor + p)
                .unwrap_or(text.len());
            false
        }
        "w" => {
            for _ in 0..n {
                *cursor = motion_w(text, *cursor);
            }
            false
        }
        "b" => {
            for _ in 0..n {
                *cursor = motion_b(text, *cursor);
            }
            false
        }
        "e" => {
            for _ in 0..n {
                *cursor = motion_e(text, *cursor);
            }
            false
        }
        "W" => {
            for _ in 0..n {
                *cursor = motion_W(text, *cursor);
            }
            false
        }
        "B" => {
            for _ in 0..n {
                *cursor = motion_B(text, *cursor);
            }
            false
        }
        "E" => {
            for _ in 0..n {
                *cursor = motion_E(text, *cursor);
            }
            false
        }
        "G" => {
            *cursor = if n == 1 {
                motion_G(text)
            } else {
                motion_gg(text, n)
            };
            false
        }
        "g" => {
            *pending = VimPendingState::G { count: n };
            false
        }
        // ---- Find motions ----
        "f" => {
            *pending = VimPendingState::Find {
                kind: VimFindKind::F,
                count: n,
            };
            false
        }
        "F" => {
            *pending = VimPendingState::Find {
                kind: VimFindKind::BigF,
                count: n,
            };
            false
        }
        "t" => {
            *pending = VimPendingState::Find {
                kind: VimFindKind::T,
                count: n,
            };
            false
        }
        "T" => {
            *pending = VimPendingState::Find {
                kind: VimFindKind::BigT,
                count: n,
            };
            false
        }
        ";" => {
            if let Some((kind, c)) = *last_find {
                if let Some(pos) = motion_find_char(text, *cursor, c, kind, n) {
                    *cursor = pos;
                }
            }
            false
        }
        "," => {
            if let Some((kind, c)) = *last_find {
                let rev = match kind {
                    VimFindKind::F => VimFindKind::BigF,
                    VimFindKind::BigF => VimFindKind::F,
                    VimFindKind::T => VimFindKind::BigT,
                    VimFindKind::BigT => VimFindKind::T,
                };
                if let Some(pos) = motion_find_char(text, *cursor, c, rev, n) {
                    *cursor = pos;
                }
            }
            false
        }
        // ---- Operators ----
        "d" => {
            *pending = VimPendingState::Operator {
                op: VimOperator::Delete,
                count: n,
            };
            false
        }
        "c" => {
            *pending = VimPendingState::Operator {
                op: VimOperator::Change,
                count: n,
            };
            false
        }
        "y" => {
            *pending = VimPendingState::Operator {
                op: VimOperator::Yank,
                count: n,
            };
            false
        }
        // ---- Single-char delete/change shortcuts ----
        "x" => {
            if *cursor < text.len() {
                let clen = text[*cursor..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                *yank_buf = text[*cursor..*cursor + clen].to_string();
                text.drain(*cursor..*cursor + clen);
                *cursor =
                    (*cursor).min(
                        text.len()
                            .saturating_sub(if text.is_empty() { 0 } else { 1 }),
                    );
                return true;
            }
            false
        }
        "X" => {
            if *cursor > 0 {
                let prev = text[..*cursor]
                    .char_indices()
                    .last()
                    .map(|(b, _)| b)
                    .unwrap_or(0);
                *yank_buf = text[prev..*cursor].to_string();
                text.drain(prev..*cursor);
                *cursor = prev;
                return true;
            }
            false
        }
        "D" => {
            let end = text[*cursor..]
                .find('\n')
                .map(|p| *cursor + p)
                .unwrap_or(text.len());
            if end > *cursor {
                *yank_buf = text[*cursor..end].to_string();
                text.drain(*cursor..end);
                return true;
            }
            false
        }
        "C" => {
            let end = text[*cursor..]
                .find('\n')
                .map(|p| *cursor + p)
                .unwrap_or(text.len());
            *yank_buf = text[*cursor..end].to_string();
            text.drain(*cursor..end);
            *mode = VimMode::Insert;
            true
        }
        "s" => {
            if *cursor < text.len() {
                let clen = text[*cursor..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                *yank_buf = text[*cursor..*cursor + clen].to_string();
                text.drain(*cursor..*cursor + clen);
                *mode = VimMode::Insert;
                return true;
            }
            false
        }
        "S" => {
            let ls = text[..*cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let le = text[*cursor..]
                .find('\n')
                .map(|p| *cursor + p)
                .unwrap_or(text.len());
            *yank_buf = text[ls..le].to_string();
            text.drain(ls..le);
            *cursor = ls;
            *mode = VimMode::Insert;
            true
        }
        // ---- Yank shortcuts ----
        "Y" | "yy" => {
            let ls = text[..*cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let le = text[*cursor..]
                .find('\n')
                .map(|p| *cursor + p + 1)
                .unwrap_or(text.len());
            *yank_buf = text[ls..le].to_string();
            false
        }
        // ---- Paste ----
        "p" => {
            if !yank_buf.is_empty() {
                let buf = yank_buf.clone();
                let insert_pos = if *cursor < text.len() {
                    text[*cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(b, _)| *cursor + b)
                        .unwrap_or(text.len())
                } else {
                    text.len()
                };
                text.insert_str(insert_pos, &buf);
                *cursor = (insert_pos + buf.len()).saturating_sub(1);
                return true;
            }
            false
        }
        "P" => {
            if !yank_buf.is_empty() {
                let buf = yank_buf.clone();
                text.insert_str(*cursor, &buf);
                *cursor = (*cursor + buf.len()).saturating_sub(1);
                return true;
            }
            false
        }
        // ---- Replace ----
        "r" => {
            *pending = VimPendingState::Replace { count: n };
            false
        }
        // ---- Toggle case ----
        "~" => {
            if *cursor < text.len() {
                let clen = text[*cursor..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                let old: String = text[*cursor..*cursor + clen].to_string();
                let new: String = old
                    .chars()
                    .map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().next().unwrap_or(c)
                        } else {
                            c.to_uppercase().next().unwrap_or(c)
                        }
                    })
                    .collect();
                text.replace_range(*cursor..*cursor + clen, &new);
                if *cursor < text.len() {
                    *cursor = text[*cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(b, _)| *cursor + b)
                        .unwrap_or(text.len());
                }
                return true;
            }
            false
        }
        // ---- Indent ----
        ">" => {
            *pending = VimPendingState::Indent { dir: '>', count: n };
            false
        }
        "<" => {
            *pending = VimPendingState::Indent { dir: '<', count: n };
            false
        }
        // ---- Join lines ----
        "J" => {
            if let Some(nl_pos) = text[*cursor..].find('\n').map(|p| *cursor + p) {
                text.remove(nl_pos);
                if text.as_bytes().get(nl_pos) != Some(&b' ') {
                    text.insert(nl_pos, ' ');
                }
                return true;
            }
            false
        }
        // ---- Open line ----
        "o" => {
            let le = text[*cursor..]
                .find('\n')
                .map(|p| *cursor + p)
                .unwrap_or(text.len());
            text.insert(le, '\n');
            *cursor = le + 1;
            *mode = VimMode::Insert;
            true
        }
        "O" => {
            let ls = text[..*cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
            text.insert(ls, '\n');
            *cursor = ls;
            *mode = VimMode::Insert;
            true
        }
        // ---- dd/yy (multi-char fallthrough from legacy apply_vim_command) ----
        "dd" => {
            let ls = text[..*cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let le = text[*cursor..]
                .find('\n')
                .map(|p| *cursor + p + 1)
                .unwrap_or(text.len());
            *yank_buf = text[ls..le].to_string();
            text.drain(ls..le);
            *cursor = ls.min(text.len());
            true
        }
        // ---- Register, marks, macros — set pending; actual work done in vim_command ----
        "\"" => {
            *pending = VimPendingState::Register('\0');
            false
        }
        "m" => {
            *pending = VimPendingState::Mark;
            false
        }
        "'" => {
            *pending = VimPendingState::JumpMark;
            false
        }
        "q" => {
            *pending = VimPendingState::MacroRecord;
            false
        }
        "@" => {
            *pending = VimPendingState::MacroReplay;
            false
        }
        _ => false,
    }
}

pub(super) fn vim_g(
    text: &mut str,
    cursor: &mut usize,
    key: &str,
    pending: &mut VimPendingState,
    count: usize,
) -> bool {
    match key {
        "g" => {
            *cursor = if count > 1 { motion_gg(text, count) } else { 0 };
            false
        }
        "e" => {
            // `ge` — end of previous word
            for _ in 0..count.max(1) {
                if *cursor == 0 {
                    break;
                }
                let before = &text[..*cursor];
                let chars: Vec<char> = before.chars().collect();
                let n = chars.len();
                let mut i = n;
                while i > 0 && chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                if i == 0 {
                    *cursor = 0;
                    break;
                }
                let is_wc = is_word_char(chars[i - 1]);
                while i > 1 && is_word_char(chars[i - 2]) == is_wc && !chars[i - 2].is_whitespace()
                {
                    i -= 1;
                }
                *cursor = char_idx_to_byte(before, i - 1);
            }
            false
        }
        "E" => {
            // `gE` — end of previous WORD
            for _ in 0..count.max(1) {
                if *cursor == 0 {
                    break;
                }
                let before = &text[..*cursor];
                let chars: Vec<char> = before.chars().collect();
                let n = chars.len();
                let mut i = n;
                while i > 0 && chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                while i > 1 && !chars[i - 2].is_whitespace() {
                    i -= 1;
                }
                *cursor = char_idx_to_byte(before, i - 1);
            }
            false
        }
        "U" => {
            // `gU` — start case conversion uppercase operator
            *pending = VimPendingState::Operator {
                op: VimOperator::Uppercase,
                count,
            };
            false
        }
        "u" => {
            // `gu` — start case conversion lowercase operator
            *pending = VimPendingState::Operator {
                op: VimOperator::Lowercase,
                count,
            };
            false
        }
        _ => {
            *pending = VimPendingState::None;
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn vim_operator(
    mode: &mut VimMode,
    text: &mut String,
    cursor: &mut usize,
    key: &str,
    yank_buf: &mut String,
    pending: &mut VimPendingState,
    _last_find: &mut Option<(VimFindKind, char)>,
    op: VimOperator,
    count: usize,
) -> bool {
    let op_char = match op {
        VimOperator::Delete => "d",
        VimOperator::Change => "c",
        VimOperator::Yank => "y",
        VimOperator::Uppercase => "U",
        VimOperator::Lowercase => "u",
    };
    // Doubled operator = line op (dd, cc, yy, gUU, guu)
    if key == op_char {
        let ls = text[..*cursor].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let mut le = *cursor;
        for _ in 0..count.max(1) {
            match text[le..].find('\n') {
                Some(n) => le += n + 1,
                None => {
                    le = text.len();
                    break;
                }
            }
        }
        let le = le.min(text.len());
        let selected = &text[ls..le];
        *yank_buf = selected.to_string();
        if op != VimOperator::Yank {
            let new_content = match op {
                VimOperator::Delete => String::new(),
                VimOperator::Change => {
                    *mode = VimMode::Insert;
                    String::new()
                }
                VimOperator::Uppercase => uppercase_region(selected),
                VimOperator::Lowercase => lowercase_region(selected),
                VimOperator::Yank => unreachable!(),
            };
            text.drain(ls..le);
            text.insert_str(ls, &new_content);
            *cursor = ls;
            return true;
        }
        return false;
    }
    // Count prefix after operator (e.g. d3w)
    if key.len() == 1 && key.chars().next().unwrap().is_ascii_digit() {
        *pending = VimPendingState::OperatorCount {
            op,
            count,
            digits: key.to_string(),
        };
        return false;
    }
    // `g` prefix
    if key == "g" {
        *pending = VimPendingState::OperatorG { op, count };
        return false;
    }
    // Simple motions
    let target = match key {
        "h" => {
            let mut p = *cursor;
            for _ in 0..count.max(1) {
                p = p.saturating_sub(1);
            }
            p
        }
        "l" => {
            let mut p = *cursor;
            for _ in 0..count.max(1) {
                if p < text.len() {
                    p = text[p..]
                        .char_indices()
                        .nth(1)
                        .map(|(b, _)| p + b)
                        .unwrap_or(text.len());
                }
            }
            p
        }
        "w" => {
            let mut p = *cursor;
            for _ in 0..count.max(1) {
                p = motion_w(text, p);
            }
            p
        }
        "b" => {
            let mut p = *cursor;
            for _ in 0..count.max(1) {
                p = motion_b(text, p);
            }
            p
        }
        "e" => {
            let mut p = *cursor;
            for _ in 0..count.max(1) {
                p = motion_e(text, p);
            }
            p
        }
        "W" => {
            let mut p = *cursor;
            for _ in 0..count.max(1) {
                p = motion_W(text, p);
            }
            p
        }
        "B" => {
            let mut p = *cursor;
            for _ in 0..count.max(1) {
                p = motion_B(text, p);
            }
            p
        }
        "E" => {
            let mut p = *cursor;
            for _ in 0..count.max(1) {
                p = motion_E(text, p);
            }
            p
        }
        "0" => text[..*cursor].rfind('\n').map(|p| p + 1).unwrap_or(0),
        "^" => motion_first_nonblank(text, *cursor),
        "$" => text[*cursor..]
            .find('\n')
            .map(|p| *cursor + p)
            .unwrap_or(text.len()),
        "G" => {
            if count == 1 {
                motion_G(text)
            } else {
                motion_gg(text, count)
            }
        }
        _ => {
            return false;
        }
    };
    if target == *cursor {
        return false;
    }
    let (from, to) = if target < *cursor {
        (target, *cursor)
    } else {
        (*cursor, target)
    };
    // Inclusive adjustment for e, E, $
    let to_adj = if matches!(key, "e" | "E" | "$") {
        text[to..]
            .char_indices()
            .nth(1)
            .map(|(b, _)| to + b)
            .unwrap_or(text.len())
    } else {
        to
    };
    let (new_text, new_cursor) = apply_operator_range(op, text, from, to_adj, yank_buf, mode);
    *text = new_text;
    *cursor = new_cursor.min(text.len());
    op != VimOperator::Yank
}

#[allow(clippy::too_many_arguments)]
pub(super) fn vim_operator_count(
    mode: &mut VimMode,
    text: &mut String,
    cursor: &mut usize,
    key: &str,
    yank_buf: &mut String,
    pending: &mut VimPendingState,
    last_find: &mut Option<(VimFindKind, char)>,
    op: VimOperator,
    count: usize,
    digits: String,
) -> bool {
    if key.len() == 1 && key.chars().next().unwrap().is_ascii_digit() {
        let new_digits = format!("{}{}", digits, key);
        let d: usize = new_digits.parse().unwrap_or(10000).min(10000);
        *pending = VimPendingState::OperatorCount {
            op,
            count,
            digits: d.to_string(),
        };
        return false;
    }
    let motion_count: usize = digits.parse().unwrap_or(1);
    let effective = count.saturating_mul(motion_count).min(10000);
    *pending = VimPendingState::Operator {
        op,
        count: effective,
    };
    vim_operator(
        mode, text, cursor, key, yank_buf, pending, last_find, op, effective,
    )
}

pub(super) fn vim_operator_g(
    mode: &mut VimMode,
    text: &mut String,
    cursor: &mut usize,
    key: &str,
    yank_buf: &mut String,
    op: VimOperator,
    count: usize,
) -> bool {
    match key {
        "g" => {
            let target = if count > 1 { motion_gg(text, count) } else { 0 };
            let (from, to) = (target.min(*cursor), target.max(*cursor));
            let to_le = text[to..]
                .find('\n')
                .map(|p| to + p + 1)
                .unwrap_or(text.len());
            let (new_text, new_cursor) =
                apply_operator_range(op, text, from, to_le, yank_buf, mode);
            *text = new_text;
            *cursor = new_cursor.min(text.len());
            op != VimOperator::Yank
        }
        _ => false,
    }
}
