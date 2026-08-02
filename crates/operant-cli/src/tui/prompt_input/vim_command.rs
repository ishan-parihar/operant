//! Vim command-line mode handler.

use super::{VimFindKind, VimMode, VimOperator, VimPendingState};
use super::PromptInputState;

/// Handle vim command-line mode (`:` prefix).
pub(super) fn vim_command(s: &mut PromptInputState, key: &str) -> bool {
        // ---- Escape always cancels recording, pending state, returns to Normal ----
        if key == "Escape" {
            // If leaving insert mode, finalise dot-repeat insert action
            if self.vim_mode == VimMode::Insert {
                if let Some(before) = self.vim_insert_text_before.take() {
                    // Compute inserted text as the new characters added since mode entry
                    let inserted = if self.text.len() >= before.len() {
                        // Simple case: text only grew (cursor at end of inserted span)
                        let from = before.len().min(self.cursor);
                        let _ = from; // use cursor-based diff below
                        // Find the diff between before/after texts at current cursor
                        // Inserted = text[insert_start..cursor] but we don't track start.
                        // Approximate: whole text minus before, substring at cursor.
                        // Better: store cursor-at-entry and extract.
                        self.text
                            [before.len().min(self.text.len())..self.cursor.min(self.text.len())]
                            .to_string()
                    } else {
                        String::new()
                    };
                    if !inserted.is_empty() {
                        self.vim_dot_action = Some(DotRepeatAction::Insert { text: inserted });
                    }
                }
            }
            self.vim_mode = VimMode::Normal;
            self.vim_pending = VimPendingState::None;
            self.visual_anchor = None;
            self.normalize();
            return;
        }

        // ---- Command-line mode (`:`) ----
        if self.vim_mode == VimMode::Command {
            match key {
                "Backspace" => {
                    if self.vim_command_buf.is_empty() {
                        self.vim_mode = VimMode::Normal;
                    } else {
                        self.vim_command_buf.pop();
                    }
                }
                "Enter" => {
                    let cmd = self.vim_command_buf.trim().to_string();
                    self.vim_command_buf.clear();
                    self.vim_mode = VimMode::Normal;
                    self.execute_vim_cmdline(&cmd);
                }
                _ if key.len() == 1 => {
                    self.vim_command_buf.push(key.chars().next().unwrap());
                }
                _ => {}
            }
            return;
        }

        // ---- In-prompt search mode (`/`) ----
        if self.vim_mode == VimMode::Search {
            match key {
                "Backspace" => {
                    if self.vim_search_buf.is_empty() {
                        self.vim_mode = VimMode::Normal;
                    } else {
                        self.vim_search_buf.pop();
                    }
                }
                "Enter" => {
                    let pattern = self.vim_search_buf.clone();
                    if !pattern.is_empty() {
                        self.vim_search_last = Some(pattern.clone());
                        self.vim_search_forward(&pattern, 0);
                    }
                    self.vim_search_buf.clear();
                    self.vim_mode = VimMode::Normal;
                }
                _ if key.len() == 1 => {
                    self.vim_search_buf.push(key.chars().next().unwrap());
                }
                _ => {}
            }
            return;
        }

        // ---- Accumulate key into macro recording buffer ----
        if let Some(reg) = self.vim_macro_recording {
            // `q` in normal mode stops recording
            if key == "q"
                && self.vim_mode == VimMode::Normal
                && self.vim_pending == VimPendingState::None
            {
                self.stop_macro_recording();
                return;
            }
            if let Some(keys) = self.vim_macro_content.get_mut(&reg) {
                keys.push(key.to_string());
            }
        }

        // ---- Handle new pending states before apply_vim_key ----
        match self.vim_pending.clone() {
            VimPendingState::Register('\0') => {
                // Waiting for register name char after `"`
                if key.len() == 1 {
                    let reg = key.chars().next().unwrap();
                    self.vim_pending = VimPendingState::RegisterOp(reg);
                } else {
                    self.vim_pending = VimPendingState::None;
                }
                return;
            }
            VimPendingState::RegisterOp(reg) => {
                // Waiting for operator after `"<reg>`
                match key {
                    "y" => {
                        // Yank current line to register
                        let ls = self.text[..self.cursor]
                            .rfind('\n')
                            .map(|p| p + 1)
                            .unwrap_or(0);
                        let le = self.text[self.cursor..]
                            .find('\n')
                            .map(|p| self.cursor + p + 1)
                            .unwrap_or(self.text.len());
                        let yanked = self.text[ls..le].to_string();
                        self.yank_to_register(reg, &yanked);
                        self.yank_buf = yanked;
                    }
                    "d" => {
                        // Delete current line to register
                        let ls = self.text[..self.cursor]
                            .rfind('\n')
                            .map(|p| p + 1)
                            .unwrap_or(0);
                        let le = self.text[self.cursor..]
                            .find('\n')
                            .map(|p| self.cursor + p + 1)
                            .unwrap_or(self.text.len());
                        let deleted = self.text[ls..le].to_string();
                        self.push_undo();
                        self.yank_to_register(reg, &deleted);
                        self.yank_buf = deleted;
                        let le = le.min(self.text.len());
                        self.text.drain(ls..le);
                        self.cursor = ls.min(self.text.len());
                        self.vim_pending = VimPendingState::None;
                        self.normalize();
                        return;
                    }
                    "p" => {
                        // Paste from register after cursor
                        if let Some(buf) = self.paste_from_register(reg) {
                            let insert_pos = if self.cursor < self.text.len() {
                                self.text[self.cursor..]
                                    .char_indices()
                                    .nth(1)
                                    .map(|(b, _)| self.cursor + b)
                                    .unwrap_or(self.text.len())
                            } else {
                                self.text.len()
                            };
                            self.push_undo();
                            self.text.insert_str(insert_pos, &buf);
                            self.cursor = (insert_pos + buf.len()).saturating_sub(1);
                            self.vim_pending = VimPendingState::None;
                            self.normalize();
                            return;
                        }
                    }
                    _ => {}
                }
                self.vim_pending = VimPendingState::None;
                return;
            }
            VimPendingState::Mark => {
                // `m<char>` — set mark
                if key.len() == 1 {
                    let name = key.chars().next().unwrap();
                    self.set_mark(name);
                }
                self.vim_pending = VimPendingState::None;
                return;
            }
            VimPendingState::JumpMark => {
                // `'<char>` — jump to mark
                if key.len() == 1 {
                    let name = key.chars().next().unwrap();
                    self.jump_to_mark(name);
                }
                self.vim_pending = VimPendingState::None;
                return;
            }
            VimPendingState::MacroRecord => {
                // `q<char>` — start recording into register; clear pending first.
                self.vim_pending = VimPendingState::None;
                if key.len() == 1 {
                    let reg = key.chars().next().unwrap();
                    self.start_macro_recording(reg);
                }
                return;
            }
            VimPendingState::MacroReplay => {
                // `@<char>` — replay macro; clear pending BEFORE recursing so
                // recursive vim_command calls don't re-enter this arm.
                self.vim_pending = VimPendingState::None;
                if key.len() == 1 {
                    let reg = key.chars().next().unwrap();
                    let keys = self.replay_macro(reg);
                    // Replay each recorded key (avoid infinite loops by cloning)
                    for k in keys {
                        // Guard: don't replay if we somehow entered macro record for same reg
                        if self.vim_macro_recording == Some(reg) {
                            break;
                        }
                        self.vim_command(&k.clone());
                    }
                }
                return;
            }
            _ => {}
        }

        // ---- Dot-repeat `.` — replay last modifying action ----
        if key == "."
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            if let Some(action) = self.vim_dot_action.clone() {
                match action {
                    DotRepeatAction::Insert { text: ins, .. } => {
                        self.push_undo();
                        self.text.insert_str(self.cursor, &ins);
                        self.cursor += ins.len();
                        self.normalize();
                        return;
                    }
                    DotRepeatAction::DeleteChars { count } => {
                        self.push_undo();
                        let mut deleted = 0usize;
                        while deleted < count && self.cursor < self.text.len() {
                            let clen = self.text[self.cursor..]
                                .chars()
                                .next()
                                .map(|c| c.len_utf8())
                                .unwrap_or(1);
                            self.text.drain(self.cursor..self.cursor + clen);
                            deleted += 1;
                        }
                        self.normalize();
                        return;
                    }
                    DotRepeatAction::ReplaceChar { ch } => {
                        if self.cursor < self.text.len() {
                            self.push_undo();
                            let clen = self.text[self.cursor..]
                                .chars()
                                .next()
                                .map(|c| c.len_utf8())
                                .unwrap_or(1);
                            self.text
                                .replace_range(self.cursor..self.cursor + clen, &ch.to_string());
                            self.normalize();
                        }
                        return;
                    }
                }
            }
            return;
        }

        // ---- Track when entering insert mode for dot-repeat ----
        let was_normal = self.vim_mode == VimMode::Normal;
        let prev_text_len = self.text.len();

        // `u` — undo: restore previous text/cursor snapshot
        if key == "u"
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            if let Some((t, c)) = self.undo_stack.pop() {
                self.text = t;
                self.cursor = c;
                self.normalize();
            }
            return;
        }
        // Enter visual mode with `v` — anchor the selection start
        if key == "v"
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            self.vim_mode = VimMode::Visual;
            self.visual_anchor = Some(self.cursor);
            return;
        }
        // Enter command-line mode with `:`
        if key == ":"
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            self.vim_mode = VimMode::Command;
            self.vim_command_buf.clear();
            return;
        }
        // Enter in-prompt search with `/`
        if key == "/"
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            self.vim_mode = VimMode::Search;
            self.vim_search_buf.clear();
            return;
        }
        // Enter visual-line mode with `V`
        if key == "V"
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            self.vim_mode = VimMode::VisualLine;
            let ls = self.text[..self.cursor]
                .rfind('\n')
                .map(|p| p + 1)
                .unwrap_or(0);
            self.visual_anchor = Some(ls);
            return;
        }
        // Enter visual-block mode with Ctrl+V
        if key == "\x16"
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            self.vim_mode = VimMode::VisualBlock;
            self.visual_anchor = Some(self.cursor);
            return;
        }
        // `n` — repeat last search forward
        if key == "n"
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            if let Some(pat) = self.vim_search_last.clone() {
                self.vim_search_forward(&pat, 1);
            }
            return;
        }
        // `N` — repeat last search backward
        if key == "N"
            && self.vim_mode == VimMode::Normal
            && self.vim_pending == VimPendingState::None
        {
            if let Some(pat) = self.vim_search_last.clone() {
                self.vim_search_backward(&pat);
            }
            return;
        }
        // In visual-line mode, `y`/`d`/`c` operate on whole lines, motion keys extend selection
        if self.vim_mode == VimMode::VisualLine {
            if let Some(anchor) = self.visual_anchor {
                let line_start = |pos: usize, s: &str| -> usize {
                    s[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0)
                };
                let line_end = |pos: usize, s: &str| -> usize {
                    s[pos..].find('\n').map(|p| pos + p + 1).unwrap_or(s.len())
                };
                let sel_start = line_start(anchor.min(self.cursor), &self.text);
                let sel_end = line_end(anchor.max(self.cursor), &self.text);
                match key {
                    "y" => {
                        self.yank_buf = self.text[sel_start..sel_end].to_string();
                        self.cursor = sel_start;
                        self.vim_mode = VimMode::Normal;
                        self.visual_anchor = None;
                        return;
                    }
                    "d" | "x" => {
                        self.push_undo();
                        self.yank_buf = self.text[sel_start..sel_end].to_string();
                        let char_count = self.yank_buf.chars().count();
                        self.text.drain(sel_start..sel_end);
                        self.cursor = sel_start.min(self.text.len());
                        self.vim_mode = VimMode::Normal;
                        self.visual_anchor = None;
                        self.vim_dot_action =
                            Some(DotRepeatAction::DeleteChars { count: char_count });
                        self.normalize();
                        return;
                    }
                    "c" => {
                        self.push_undo();
                        self.yank_buf = self.text[sel_start..sel_end].to_string();
                        self.text.drain(sel_start..sel_end);
                        self.cursor = sel_start;
                        self.vim_mode = VimMode::Insert;
                        self.visual_anchor = None;
                        self.vim_insert_text_before = Some(self.text.clone());
                        self.normalize();
                        return;
                    }
                    _ => {
                        // Motion keys extend the selection (handled by apply_vim_key below)
                    }
                }
            }
        }
        // In visual-block mode, treat like character-wise visual for single-line input
        if self.vim_mode == VimMode::VisualBlock {
            if let Some(anchor) = self.visual_anchor {
                let from = anchor.min(self.cursor);
                let to_excl = anchor.max(self.cursor);
                let to = self.text[to_excl..]
                    .char_indices()
                    .nth(1)
                    .map(|(b, _)| to_excl + b)
                    .unwrap_or(self.text.len());
                match key {
                    "y" => {
                        self.yank_buf = self.text[from..to].to_string();
                        self.cursor = from;
                        self.vim_mode = VimMode::Normal;
                        self.visual_anchor = None;
                        return;
                    }
                    "d" | "x" => {
                        self.push_undo();
                        self.yank_buf = self.text[from..to].to_string();
                        let char_count = self.yank_buf.chars().count();
                        self.text.drain(from..to);
                        self.cursor = from.min(self.text.len());
                        self.vim_mode = VimMode::Normal;
                        self.visual_anchor = None;
                        self.vim_dot_action =
                            Some(DotRepeatAction::DeleteChars { count: char_count });
                        self.normalize();
                        return;
                    }
                    "c" => {
                        self.push_undo();
                        self.yank_buf = self.text[from..to].to_string();
                        self.text.drain(from..to);
                        self.cursor = from;
                        self.vim_mode = VimMode::Insert;
                        self.visual_anchor = None;
                        self.vim_insert_text_before = Some(self.text.clone());
                        self.normalize();
                        return;
                    }
                    _ => {}
                }
            }
        }
        // In visual mode, `y`/`d`/`c` operate on the selection, Escape exits
        if self.vim_mode == VimMode::Visual {
            if let Some(anchor) = self.visual_anchor {
                let from = anchor.min(self.cursor);
                let to_excl = anchor.max(self.cursor);
                let to = self.text[to_excl..]
                    .char_indices()
                    .nth(1)
                    .map(|(b, _)| to_excl + b)
                    .unwrap_or(self.text.len());
                match key {
                    "y" => {
                        self.yank_buf = self.text[from..to].to_string();
                        self.cursor = from;
                        self.vim_mode = VimMode::Normal;
                        self.visual_anchor = None;
                        return;
                    }
                    "d" | "x" => {
                        self.push_undo();
                        self.yank_buf = self.text[from..to].to_string();
                        // Count chars to delete BEFORE mutating text
                        let char_count = self.yank_buf.chars().count();
                        self.text.drain(from..to);
                        self.cursor = from.min(self.text.len());
                        self.vim_mode = VimMode::Normal;
                        self.visual_anchor = None;
                        self.vim_dot_action =
                            Some(DotRepeatAction::DeleteChars { count: char_count });
                        self.normalize();
                        return;
                    }
                    "c" => {
                        self.push_undo();
                        self.yank_buf = self.text[from..to].to_string();
                        self.text.drain(from..to);
                        self.cursor = from;
                        self.vim_mode = VimMode::Insert;
                        self.visual_anchor = None;
                        self.vim_insert_text_before = Some(self.text.clone());
                        self.normalize();
                        return;
                    }
                    _ => {
                        // Motion keys still move cursor in visual mode
                    }
                }
            }
        }

        let snapshot_text = self.text.clone();
        let snapshot_cursor = self.cursor;
        let modified = apply_vim_key(
            &mut self.vim_mode,
            &mut self.text,
            &mut self.cursor,
            key,
            &mut self.yank_buf,
            &mut self.vim_pending,
            &mut self.last_find,
        );
        if modified {
            self.undo_stack
                .push((snapshot_text.clone(), snapshot_cursor));
            if self.undo_stack.len() > 100 {
                self.undo_stack.remove(0);
            }
            // Update dot-repeat for simple modifying commands (normal mode only)
            if was_normal {
                match key {
                    "x" => {
                        self.vim_dot_action = Some(DotRepeatAction::DeleteChars { count: 1 });
                    }
                    "X" => {
                        self.vim_dot_action = Some(DotRepeatAction::DeleteChars { count: 1 });
                    }
                    _ => {}
                }
            }
        }

        // If we just entered insert mode from normal mode, record text snapshot for dot-repeat
        if was_normal && self.vim_mode == VimMode::Insert {
            self.vim_insert_text_before = Some(self.text.clone());
        }

        // Handle `r` replace pending → after confirm, store dot action
        if let VimPendingState::None = self.vim_pending {
            if modified && was_normal {
                // Check if a replace happened (text changed by exactly 1 char at cursor)
                if self.text.len() == prev_text_len && self.text != snapshot_text {
                    // Likely a replace — extract the replacement char at snapshot_cursor
                    if let Some(ch) = self.text[snapshot_cursor..].chars().next() {
                        // Verify it's different from what was there before
                        let old_ch = snapshot_text[snapshot_cursor..].chars().next();
                        if old_ch != Some(ch) {
                            self.vim_dot_action = Some(DotRepeatAction::ReplaceChar { ch });
                        }
                    }
                }
            }
        }

        // Update visual anchor tracking when in visual mode
        if (self.vim_mode == VimMode::Visual || self.vim_mode == VimMode::VisualBlock)
            && self.visual_anchor.is_none()
        {
            self.visual_anchor = Some(self.cursor);
        }
        self.normalize();
    }
