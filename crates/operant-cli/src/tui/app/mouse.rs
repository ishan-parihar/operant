//! Mouse, selection, and context menu handling methods.

use super::*;

impl App {
    pub(super) fn is_double_click(&self, current_pos: (u16, u16)) -> bool {
        let now = std::time::Instant::now();
        match (self.last_click_time, self.last_click_position) {
            (Some(last_time), Some(last_pos)) => {
                let elapsed = now.duration_since(last_time);
                let distance = ((current_pos.0 as i32 - last_pos.0 as i32).abs()
                    + (current_pos.1 as i32 - last_pos.1 as i32).abs())
                    as u16;
                elapsed.as_millis() < 500 && distance <= 5
            }
            _ => false,
        }
    }

    // Find word boundaries for the character at (col, row) in the rendered
    // transcript buffer. Returns absolute (start_col, end_col) for the word
    // containing the click. A "word" is a run of non-whitespace characters.
    pub(super) fn find_word_boundaries(&self, col: u16, row: u16) -> Option<(u16, u16)> {
        let cache = self.last_row_text.borrow();
        let line = cache.get(&row)?;
        if line.is_empty() {
            return None;
        }
        let selectable_area = self.last_selectable_area.get();
        if col < selectable_area.x {
            return None;
        }
        let local = (col - selectable_area.x) as usize;
        let chars: Vec<char> = line.chars().collect();
        if local >= chars.len() {
            return None;
        }
        let is_word = |c: char| !c.is_whitespace();
        if !is_word(chars[local]) {
            return None;
        }
        let mut start = local;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = local;
        while end + 1 < chars.len() && is_word(chars[end + 1]) {
            end += 1;
        }
        Some((
            selectable_area.x + start as u16,
            selectable_area.x + end as u16,
        ))
    }

    // Find paragraph boundaries (run of non-blank rows) around `row` and
    // return (start_row, end_row, end_col) where end_col is the trimmed end
    // of the last row's content. Used by triple-click selection so a
    // "paragraph" — a contiguous block of text rows — is selected as a unit
    // instead of a single visual row.
    pub(super) fn find_paragraph_boundaries(&self, row: u16) -> Option<(u16, u16, u16)> {
        let cache = self.last_row_text.borrow();
        let selectable_area = self.last_selectable_area.get();
        if selectable_area.width == 0 || selectable_area.height == 0 {
            return None;
        }
        let row_text = cache.get(&row)?;
        if row_text.trim().is_empty() {
            return None;
        }
        let max_row = selectable_area
            .y
            .saturating_add(selectable_area.height)
            .saturating_sub(1);
        let mut start = row;
        while start > selectable_area.y {
            let prev = start - 1;
            if cache
                .get(&prev)
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                break;
            }
            start = prev;
        }
        let mut end = row;
        while end < max_row {
            let next = end + 1;
            if cache
                .get(&next)
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                break;
            }
            end = next;
        }
        let last_text = cache.get(&end)?;
        let trimmed = last_text.trim_end();
        let end_col = selectable_area.x + trimmed.chars().count().saturating_sub(1) as u16;
        Some((start, end, end_col))
    }

    pub(super) fn context_menu_items(kind: ContextMenuKind) -> &'static [ContextMenuItem] {
        match kind {
            ContextMenuKind::Message { .. } => &[ContextMenuItem::Copy, ContextMenuItem::Fork],
            ContextMenuKind::Selection => &[ContextMenuItem::Copy],
        }
    }

    pub(super) fn message_index_at_row(&self, row: u16) -> Option<usize> {
        self.message_row_map.borrow().get(&row).copied()
    }

    pub(super) fn clear_selection(&mut self) {
        self.selection_anchor = None;
        self.selection_focus = None;
        *self.selection_text.borrow_mut() = String::new();
    }

    // Show context menu at the given position.
    pub(super) fn show_context_menu(&mut self, x: u16, y: u16, kind: ContextMenuKind) {
        self.context_menu_state = Some(ContextMenuState {
            x,
            y,
            selected_index: 0,
            kind,
        });
    }

    // Dismiss the context menu.
    pub(super) fn dismiss_context_menu(&mut self) {
        self.context_menu_state = None;
    }

    // Handle context menu navigation with arrow keys.
    pub(super) fn navigate_context_menu(&mut self, direction: KeyCode) {
        if let Some(mut menu) = self.context_menu_state {
            let item_count = Self::context_menu_items(menu.kind).len();
            if item_count == 0 {
                self.context_menu_state = Some(menu);
                return;
            }
            match direction {
                KeyCode::Up => {
                    if menu.selected_index == 0 {
                        menu.selected_index = item_count - 1;
                    } else {
                        menu.selected_index -= 1;
                    }
                }
                KeyCode::Down => {
                    menu.selected_index = (menu.selected_index + 1) % item_count;
                }
                _ => return,
            }
            self.context_menu_state = Some(menu);
        }
    }

    // Execute the currently selected context menu item.
    pub(super) fn execute_context_menu_item(&mut self) {
        if let Some(menu) = self.context_menu_state {
            let items = Self::context_menu_items(menu.kind);

            if menu.selected_index < items.len() {
                let item = items[menu.selected_index];
                self.handle_context_menu_action(item, menu.kind);
            }
        }
        self.dismiss_context_menu();
    }

    // Open context menu at the current cursor/selection position via keyboard
    // (Ctrl+Shift+M). Uses the current scroll position to determine location,
    // or the current text selection if any.
    pub(super) fn open_context_menu_at_cursor(&mut self) {
        let msg_area = self.last_msg_area.get();
        let has_selection = !self.selection_text.borrow().trim().is_empty();

        // Calculate the row at the current scroll position (top of visible area)
        let visible_row = msg_area.y.saturating_add(self.scroll_offset as u16);

        // Try to find message at the visible scroll position
        if let Some(message_index) = self.message_index_at_row(visible_row) {
            if message_index < self.messages.len() {
                let x = msg_area.x.saturating_add(2);
                let y = msg_area.y.saturating_add(2);
                self.show_context_menu(x, y, ContextMenuKind::Message { message_index });
                return;
            }
        }

        // Fall back to selection if any
        if has_selection {
            let x = msg_area.x.saturating_add(2);
            let y = msg_area.y.saturating_add(2);
            self.show_context_menu(x, y, ContextMenuKind::Selection);
            return;
        }

        // No message at scroll position and no selection - show at bottom of message area
        let x = msg_area.x.saturating_add(2);
        let y = msg_area.y.saturating_add(msg_area.height.saturating_sub(3));
        self.show_context_menu(x, y, ContextMenuKind::Selection);
    }

    // Handle a context menu action.
    pub(super) fn handle_context_menu_action(&mut self, item: ContextMenuItem, kind: ContextMenuKind) {
        match item {
            ContextMenuItem::Copy => {
                let text = match kind {
                    ContextMenuKind::Message { message_index } => self
                        .messages
                        .get(message_index)
                        .map(|message| message.get_all_text()),
                    ContextMenuKind::Selection => {
                        let selected = self.selection_text.borrow().trim().to_string();
                        if selected.is_empty() {
                            None
                        } else {
                            Some(selected)
                        }
                    }
                };

                if let Some(text) = text {
                    if crate::message_copy::copy_to_clipboard(&text) {
                        self.push_notification(
                            NotificationKind::Info,
                            format!("Copied {} chars to clipboard.", text.len()),
                            Some(3),
                        );
                    } else {
                        self.push_notification(
                            NotificationKind::Warning,
                            "Failed to copy to clipboard.".to_string(),
                            Some(3),
                        );
                    }
                    debug!("Copy action triggered, text: {} chars", text.len());
                }
            }
            ContextMenuItem::Fork => {
                if let ContextMenuKind::Message { message_index } = kind {
                    let branch_point = message_index + 1;
                    self.prompt_input
                        .replace_text(format!("/fork {}", branch_point));
                    self.status_message = Some(format!(
                        "Fork at message {} - press Enter to confirm",
                        branch_point
                    ));
                }
            }
        }
    }

    pub(super) fn prompt_can_accept_selection_paste(&self) -> bool {
        !self.is_streaming
            && self.permission_request.is_none()
            && !self.history_search_overlay.visible
            && !matches!(
                self.prompt_input.vim_mode,
                crate::prompt_input::VimMode::Normal
                    | crate::prompt_input::VimMode::Visual
                    | crate::prompt_input::VimMode::VisualBlock
            )
    }

    pub(super) fn paste_primary_into_prompt(&mut self) -> bool {
        if !self.prompt_can_accept_selection_paste() {
            return false;
        }

        if let Some(text) =
            crate::image_paste::read_primary_text().or_else(crate::image_paste::read_clipboard_text)
        {
            self.focus = FocusTarget::Input;
            self.clear_selection();
            self.prompt_input.paste(&text);
            self.refresh_prompt_input();
            return true;
        }

        false
    }

    // Handle a paste data string (from `Event::Paste` or Ctrl+V text fallback).
    //
    // If the pasted text resolves to an existing filesystem path:
    //   - image files (png/jpg/gif/webp/bmp) → added as an image attachment pill
    //   - other files → inserted as `@path` mention text
    //
    // Otherwise the text goes through the normal `prompt_input.paste()` path
    // which applies the multi-line summary placeholder for large pastes.
    pub(super) fn handle_paste_data(&mut self, data: String) {
        use crate::tui::image_paste::PastedImage;
        use crate::tui::prompt_input::detect_pasted_path;

        if let Some(path) = detect_pasted_path(&data) {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            let is_image = matches!(
                ext.as_deref(),
                Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("bmp")
            );
            if is_image {
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image")
                    .to_string();
                let img = PastedImage {
                    path,
                    label: label.clone(),
                    dimensions: None,
                };
                self.prompt_input.add_image(img);
                self.push_notification(
                    crate::notifications::NotificationKind::Info,
                    format!("Image attached: {}", label),
                    Some(3),
                );
            } else {
                // Non-image file: insert as an @mention so the path is visible
                // but clearly marked as a file reference.
                let mention = format!("@{}", path.display());
                self.prompt_input.paste(&mention);
            }
        } else {
            self.prompt_input.paste(&data);
        }
    }

    // Returns `true` when the app is in a state where the prompt can accept
    // regular text input — used to gate paste-burst detection.
    pub(super) fn prompt_is_accepting_text(&self) -> bool {
        !self.is_streaming
            && self.permission_request.is_none()
            && !self.ask_user_dialog.visible
            && !self.history_search_overlay.visible
            && !self.settings_screen.visible
            && !self.theme_screen.visible
            && !self.key_input_dialog.visible
            && self.prompt_input.vim_mode == crate::prompt_input::VimMode::Insert
    }

    // Drain any immediately-available key events from the crossterm event
    // queue (zero-timeout poll) and return them alongside `first` as a single
    // pasted string if the burst is large enough to be a paste.
    //
    // On Windows Terminal, Ctrl+V causes the terminal emulator to write the
    // clipboard content directly to stdin as raw character events — every
    // newline becomes an Enter keypress and stray `v` characters trigger
    // voice PTT.  Because a paste dumps ALL characters into the queue at
    // once, a zero-timeout drain immediately after the first character
    // reliably yields 3+ chars for any non-trivial paste, while normal
    // keyboard typing (even at 120 WPM) almost never queues more than one
    // char in the same 50 ms window.
    //
    // Returns `Some(text)` when a paste burst is detected (caller should
    // route through `handle_paste_data`).  Returns `None` for a normal
    // single keystroke.  If a non-character key is encountered while
    // draining, it is stored in `self.pending_key` and will be replayed at
    // the top of the next event-loop iteration.
    pub(super) fn try_detect_paste_burst(&mut self, first: char) -> Option<String> {
        use crossterm::event::{Event, KeyCode, KeyEventKind};

        // Minimum number of chars (including `first`) to classify as a paste.
        // Two or more is enough: at 120 WPM the inter-key interval is ~60 ms,
        // so a second char in the same zero-timeout drain is extremely unlikely
        // from a human typist but guaranteed from a clipboard paste.
        const BURST_THRESHOLD: usize = 2;

        // Quick exit: don't bother if nothing is queued immediately.
        if !crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
            return None;
        }

        let mut buf = String::new();
        buf.push(first);

        while let Ok(true) = crossterm::event::poll(std::time::Duration::ZERO) {
            match crossterm::event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char(c) => buf.push(c),
                    KeyCode::Enter => buf.push('\n'),
                    _ => {
                        // Non-character key — save it for replay.
                        self.pending_key = Some(k);
                        break;
                    }
                },
                // Non-key event (mouse, resize, …) — leave in queue by
                // not reading it; we already checked poll() so it will
                // be re-read next iteration. But we already read it, so
                // we just break (the event is consumed but benign).
                _ => break,
            }
        }

        if buf.chars().count() >= BURST_THRESHOLD {
            Some(buf)
        } else {
            None
        }
    }

    // Process mouse events (trackpad scroll, text selection, etc.).
}
