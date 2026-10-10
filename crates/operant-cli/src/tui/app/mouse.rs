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

    // ---- Content-space selection points (iter-672) ------------------------
    //
    // `selection_anchor`/`selection_focus` store `(col, content_line)`, where
    // `content_line` is the scroll-stable rendered-line index — the same line
    // keeps its index as the viewport scrolls, so a selection made mid-drag
    // stays anchored to its text instead of to a screen row the text has
    // already scrolled away from (the jcode `CopySelectionPoint` model).
    // Projection goes through the last rendered scroll, exactly like jcode's
    // `copy_point_from_screen`.

    /// Screen row inside the selectable area → scroll-stable content line.
    /// Returns the input row as-is when the area is empty (unselectable state).
    pub(super) fn content_line_of_screen_row(&self, screen_row: u16) -> usize {
        let area = self.last_selectable_area.get();
        if area.width == 0 || area.height == 0 || screen_row < area.y {
            return screen_row as usize;
        }
        screen_row.saturating_sub(area.y) as usize + self.last_render_scroll_offset.get() as usize
    }

    // ---- Drag-select copy mode (Ctrl+T) ----------------------------------
    //
    // Copy mode does not model a selection of its own: it reuses the
    // selection the mouse already drives (`selection_anchor` /
    // `selection_focus` / `selection_text`) and only adds the behaviour on
    // top — every drag frame streams the selection to the clipboard, and
    // dragging past the viewport edge scrolls so the drag can reach text that
    // is not currently on screen. Enter and exit both go through the clipboard
    // module, which owns the saved state.

    /// Is drag-select copy mode on?
    pub fn copy_mode_active(&self) -> bool {
        crate::tui::clipboard::copy_mode_active()
    }

    /// Turn copy mode on, remembering the scroll position, the tail-follow
    /// flag and any live selection so `exit_copy_mode` can put them back.
    pub fn enter_copy_mode(&mut self) {
        crate::tui::clipboard::enter_copy_mode(
            self.selection_anchor,
            self.selection_focus,
            &self.selection_text.borrow(),
            self.scroll_offset,
            self.auto_scroll,
        );
        self.status_message =
            Some("Copy mode: drag to select (Esc or Ctrl+T to exit).".to_string());
    }

    /// Turn copy mode off and restore scroll offset, tail-follow and the
    /// selection that was live before it was entered.
    pub fn exit_copy_mode(&mut self) {
        let saved = crate::tui::clipboard::exit_copy_mode();
        self.scroll_offset = saved.saved_scroll_offset;
        self.auto_scroll = saved.saved_auto_scroll;
        self.selection_anchor = saved.saved_anchor;
        self.selection_focus = saved.saved_focus;
        *self.selection_text.borrow_mut() = saved.saved_selection_text;
        self.status_message = Some("Copy mode off.".to_string());
    }

    /// Copy the current selection and report it, leaving the selection intact
    /// (copy mode is a copy mode, not a one-shot).
    pub fn copy_current_selection(&mut self) {
        let sel_text = self.selection_text.borrow().clone();
        if sel_text.is_empty() {
            self.status_message = Some("Nothing selected to copy.".to_string());
            return;
        }
        let outcome = crate::tui::clipboard::copy(&sel_text);
        self.status_message = Some(outcome.status_message());
    }

    /// Scroll the transcript by one step when a drag has gone past the top or
    /// bottom edge of the selectable viewport, so a drag can pull in text that
    /// is off screen. Returns `true` when it scrolled.
    pub fn copy_mode_edge_autoscroll(&mut self, raw_row: u16) -> bool {
        let area = self.last_selectable_area.get();
        if area.width == 0 || area.height == 0 {
            return false;
        }
        let last_row = area.y.saturating_add(area.height).saturating_sub(1);
        let step = self.scroll_step();
        if raw_row < area.y {
            let off = self.scroll_offset.saturating_add(step);
            self.auto_scroll = false;
            if off != self.scroll_offset {
                self.scroll_offset = off;
                return true;
            }
        } else if raw_row > last_row {
            let off = self.scroll_offset.saturating_sub(step);
            if off != self.scroll_offset {
                self.scroll_offset = off;
                if off == 0 {
                    self.auto_scroll = true;
                }
                return true;
            }
        }
        false
    }

    // ---- Copy-mode keyboard navigation (jcode parity, iter-672) ----------

    /// The keyboard copy cursor: the selection focus, else the anchor, else
    /// the first visible line.
    pub(super) fn copy_cursor_point(&self) -> (u16, usize) {
        let area = self.last_selectable_area.get();
        let scroll = self.last_render_scroll_offset.get() as usize;
        if let Some((c, l)) = self.selection_focus.or(self.selection_anchor) {
            return (c, l);
        }
        (area.x, scroll)
    }

    /// One page for PageUp/PageDown in copy mode.
    pub(super) fn copy_cursor_page(&self) -> usize {
        (self.last_selectable_area.get().height.saturating_sub(2)).max(1) as usize
    }

    /// Move the copy cursor by `(dcol, dline)` within the visible window.
    /// A plain move collapses the selection onto the cursor; `extend` keeps
    /// the anchor and moves the focus (jcode's SHIFT semantics).
    pub(super) fn move_copy_cursor(&mut self, dcol: i32, dline: i64, extend: bool) {
        let area = self.last_selectable_area.get();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let scroll = self.last_render_scroll_offset.get() as usize;
        let (col, line) = self.copy_cursor_point();
        let max_col = area.x.saturating_add(area.width).saturating_sub(1);
        let new_col = (col as i32)
            .saturating_add(dcol)
            .clamp(area.x as i32, max_col as i32) as u16;
        let first = scroll;
        let last = scroll + area.height as usize - 1;
        let new_line = (line as i64)
            .saturating_add(dline)
            .clamp(first as i64, last as i64) as usize;
        if extend {
            self.selection_focus = Some((new_col, new_line));
        } else {
            self.selection_anchor = Some((new_col, new_line));
            self.selection_focus = Some((new_col, new_line));
        }
    }

    /// Home/End: cursor to the start/end of its line (columns only).
    pub(super) fn move_copy_cursor_to_edge(&mut self, end: bool, extend: bool) {
        let area = self.last_selectable_area.get();
        if area.width == 0 {
            return;
        }
        let max_col = area.x.saturating_add(area.width).saturating_sub(1);
        let col = if end { max_col } else { area.x };
        let line = self.copy_cursor_point().1;
        if extend {
            self.selection_focus = Some((col, line));
        } else {
            self.selection_anchor = Some((col, line));
            self.selection_focus = Some((col, line));
        }
    }

    /// g/G: cursor to the first/last visible line.
    pub(super) fn move_copy_cursor_to_line_edge(&mut self, bottom: bool, extend: bool) {
        let area = self.last_selectable_area.get();
        if area.height == 0 {
            return;
        }
        let scroll = self.last_render_scroll_offset.get() as usize;
        let line = if bottom {
            scroll + area.height as usize - 1
        } else {
            scroll
        };
        let col = self.copy_cursor_point().0;
        if extend {
            self.selection_focus = Some((col, line));
        } else {
            self.selection_anchor = Some((col, line));
            self.selection_focus = Some((col, line));
        }
    }

    /// Plain A: select everything currently on screen.
    pub(super) fn copy_select_all_visible(&mut self) {
        let area = self.last_selectable_area.get();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let scroll = self.last_render_scroll_offset.get() as usize;
        let max_col = area.x.saturating_add(area.width).saturating_sub(1);
        self.selection_anchor = Some((area.x, scroll));
        self.selection_focus = Some((max_col, scroll + area.height as usize - 1));
    }

    /// Ctrl+A: copy the cursor's line ± 4 lines of context (jcode's
    /// `COPY_VIEWPORT_CONTEXT_LINES`) and exit copy mode.
    pub(super) fn copy_viewport_context_and_exit(&mut self) {
        const CONTEXT: usize = 4;
        let area = self.last_selectable_area.get();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let scroll = self.last_render_scroll_offset.get() as usize;
        let (_, line) = self.copy_cursor_point();
        let first = line.saturating_sub(CONTEXT).max(scroll);
        let last = (line + CONTEXT).min(scroll + area.height as usize - 1);
        let mut text = String::new();
        let cache = self.last_row_text.borrow();
        for l in first..=last {
            let row = area.y + (l - scroll) as u16;
            if let Some(t) = cache.get(&row) {
                text.push_str(t.trim_end());
                text.push('\n');
            }
        }
        drop(cache);
        let trimmed = text.trim_end();
        let ok = !trimmed.is_empty() && crate::tui::clipboard::copy(trimmed).is_copied();
        self.status_message = Some(
            if ok {
                "Copied viewport context".to_string()
            } else {
                "Failed to copy viewport context".to_string()
            },
        );
        self.exit_copy_mode();
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
        if let Some(message_index) = self.message_index_at_row(visible_row)
            && message_index < self.messages.len()
        {
            let x = msg_area.x.saturating_add(2);
            let y = msg_area.y.saturating_add(2);
            self.show_context_menu(x, y, ContextMenuKind::Message { message_index });
            return;
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
    pub(super) fn handle_context_menu_action(
        &mut self,
        item: ContextMenuItem,
        kind: ContextMenuKind,
    ) {
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
                    let outcome = crate::tui::clipboard::copy(&text);
                    self.push_notification(
                        if outcome.is_copied() {
                            NotificationKind::Info
                        } else {
                            NotificationKind::Warning
                        },
                        if outcome.is_copied() {
                            format!("Copied {} chars to clipboard.", text.len())
                        } else {
                            outcome.status_message()
                        },
                        Some(3),
                    );
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
                // Same render/degrade path as a clipboard paste.
                let msg = crate::tui::image_paste::describe_rendered(
                    &img,
                    &crate::tui::image_paste::render_attachment(&img),
                );
                self.prompt_input.add_image(img.clone());
                self.pending_inline_images.borrow_mut().push(img);
                self.push_notification(crate::notifications::NotificationKind::Info, msg, Some(3));
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
    // newline becomes an Enter keypress and stray `v` characters.
    // Because a paste dumps ALL characters into the queue at
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

    // Handle terminal focus-change events (Phase 2.3 focus-aware rendering).
    // FocusGained resumes the full redraw cadence and restarts the
    // idle/deep-idle timers; FocusLost drops to the slowest cadence so a
    // backgrounded tab doesn't burn CPU re-rendering invisible animations.
    pub(super) fn handle_focus_event(&mut self, focused: bool) {
        self.client_focused = focused;
        if focused {
            // Restart idle detection from this moment so the cadence stays
            // at animation speed while the user is actively watching.
            self.last_activity = std::time::Instant::now();
        }
    }

    // Process mouse events (trackpad scroll, text selection, etc.).

    #[expect(
        clippy::unwrap_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Detect if a click is a double-click based on timing and position.
    /// Returns true if the click is within ~500ms and ~5px of the last click.
    pub fn handle_mouse_event(&mut self, mouse_event: MouseEvent) {
        use crossterm::event::MouseButton;

        // Fast-reject mouse-move events — they flood at 60+ Hz and we don't
        // need hover tracking. Exception: context menu needs hover to update
        // the selected item highlight.
        if matches!(mouse_event.kind, MouseEventKind::Moved) {
            if let Some(menu) = self.context_menu_state.as_mut() {
                let items = Self::context_menu_items(menu.kind);
                let item_labels: Vec<&str> = items
                    .iter()
                    .map(|i| match i {
                        ContextMenuItem::Copy => "Copy",
                        ContextMenuItem::Fork => "Fork new chat",
                    })
                    .collect();
                let menu_width =
                    (item_labels.iter().map(|l| l.len()).max().unwrap_or(4) + 4) as u16;
                let menu_height = items.len() as u16 + 2;
                let screen = self.last_msg_area.get();
                let menu_x = menu.x.min(
                    screen
                        .x
                        .saturating_add(screen.width)
                        .saturating_sub(menu_width + 1),
                );
                let menu_y = menu.y.min(
                    screen
                        .y
                        .saturating_add(screen.height)
                        .saturating_sub(menu_height + 1),
                );
                let inner_y = menu_y + 1;
                let col = mouse_event.column;
                let row = mouse_event.row;
                if col >= menu_x
                    && col < menu_x.saturating_add(menu_width)
                    && row >= inner_y
                    && row < inner_y.saturating_add(items.len() as u16)
                {
                    let hovered = (row - inner_y) as usize;
                    if hovered < items.len() {
                        menu.selected_index = hovered;
                    }
                }
            }
            return;
        }

        // ---- Dialog interaction: dismiss on click-outside, scroll/click inside ----
        // Key-input and device-auth stay outside this gate so their visible text
        // can still be selected and copied with the mouse.
        let any_dialog = self.connect_dialog.visible
            || self.import_config_picker.visible
            || self.import_config_dialog.visible
            || self.command_palette.visible
            || self.model_picker.visible
            || self.export_dialog.visible
            || self.settings_screen.visible
            || self.stats_dialog.visible
            || self.context_viz.visible
            || self.session_browser.visible;

        if any_dialog {
            match mouse_event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    // DialogSelect dialogs — check if click is inside for item selection
                    let in_dialog = if self.connect_dialog.visible {
                        self.connect_dialog
                            .contains(mouse_event.column, mouse_event.row)
                    } else if self.import_config_picker.visible {
                        self.import_config_picker
                            .contains(mouse_event.column, mouse_event.row)
                    } else if self.command_palette.visible {
                        self.command_palette
                            .contains(mouse_event.column, mouse_event.row)
                    } else {
                        // Other dialogs (model_picker, settings, export, etc.) —
                        // treat any click as "inside" to prevent accidental dismiss.
                        // User must press Esc to close these.
                        true
                    };

                    if in_dialog {
                        // Click inside a DialogSelect — select the clicked item
                        if self.connect_dialog.visible {
                            self.connect_dialog.handle_mouse_click(mouse_event.row);
                        } else if self.import_config_picker.visible {
                            self.import_config_picker
                                .handle_mouse_click(mouse_event.row);
                        } else if self.command_palette.visible {
                            self.command_palette.handle_mouse_click(mouse_event.row);
                        }
                        // Other dialogs: click absorbed, no action needed
                    } else {
                        // Click outside a DialogSelect — dismiss and restore input focus
                        self.close_secondary_views();
                        self.focus = FocusTarget::Input;
                    }
                }
                MouseEventKind::ScrollUp => {
                    // Scroll through dialog items
                    if self.connect_dialog.visible {
                        self.connect_dialog.move_up();
                    } else if self.import_config_picker.visible {
                        self.import_config_picker.move_up();
                    } else if self.command_palette.visible {
                        self.command_palette.move_up();
                    }
                }
                MouseEventKind::ScrollDown => {
                    if self.connect_dialog.visible {
                        self.connect_dialog.move_down();
                    } else if self.import_config_picker.visible {
                        self.import_config_picker.move_down();
                    } else if self.command_palette.visible {
                        self.command_palette.move_down();
                    }
                }
                _ => {}
            }
            return; // Don't process any other mouse events when a dialog is open
        }

        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                // Don't consume Ctrl+Scroll — let the terminal handle zoom.
                if !mouse_event.modifiers.contains(KeyModifiers::CONTROL) {
                    let step = self.scroll_step();
                    self.scroll_offset = self.scroll_offset.saturating_add(step);
                    self.auto_scroll = false;
                }
            }
            MouseEventKind::ScrollDown => {
                if !mouse_event.modifiers.contains(KeyModifiers::CONTROL) {
                    let step = self.scroll_step();
                    let new_off = self.scroll_offset.saturating_sub(step);
                    self.scroll_offset = new_off;
                    if new_off == 0 {
                        self.auto_scroll = true;
                        self.new_messages_while_scrolled = 0;
                    }
                }
            }
            // ---- Right-click context menu ----------------------------------
            MouseEventKind::Down(MouseButton::Right) => {
                let msg_area = self.last_msg_area.get();
                let has_selection = !self.selection_text.borrow().trim().is_empty();
                if mouse_event.column >= msg_area.x
                    && mouse_event.column < msg_area.x.saturating_add(msg_area.width)
                    && mouse_event.row >= msg_area.y
                    && mouse_event.row < msg_area.y.saturating_add(msg_area.height)
                {
                    if let Some(message_index) = self.message_index_at_row(mouse_event.row) {
                        self.show_context_menu(
                            mouse_event.column,
                            mouse_event.row,
                            ContextMenuKind::Message { message_index },
                        );
                    } else {
                        self.dismiss_context_menu();
                    }
                } else if has_selection {
                    self.show_context_menu(
                        mouse_event.column,
                        mouse_event.row,
                        ContextMenuKind::Selection,
                    );
                } else {
                    self.dismiss_context_menu();
                }
            }

            // ---- Primary-selection paste into the prompt ---------------
            MouseEventKind::Down(MouseButton::Middle) => {
                let _ = self.paste_primary_into_prompt();
            }

            // ---- Text selection / focus routing -------------------------
            MouseEventKind::Down(MouseButton::Left) => {
                // If a context menu is open, check if the click is on a menu item.
                // Must replicate the same position clamping as the renderer.
                if let Some(menu) = self.context_menu_state {
                    let items = Self::context_menu_items(menu.kind);
                    let item_labels: Vec<&str> = items
                        .iter()
                        .map(|i| match i {
                            ContextMenuItem::Copy => "Copy",
                            ContextMenuItem::Fork => "Fork new chat",
                        })
                        .collect();
                    let menu_width =
                        (item_labels.iter().map(|l| l.len()).max().unwrap_or(4) + 4) as u16;
                    let menu_height = items.len() as u16 + 2; // +2 for border
                    // Clamp to screen bounds (same as render_context_menu)
                    let screen = self.last_msg_area.get();
                    let menu_x = menu.x.min(
                        screen
                            .x
                            .saturating_add(screen.width)
                            .saturating_sub(menu_width + 1),
                    );
                    let menu_y = menu.y.min(
                        screen
                            .y
                            .saturating_add(screen.height)
                            .saturating_sub(menu_height + 1),
                    );
                    let col = mouse_event.column;
                    let row = mouse_event.row;
                    // Inner area starts 1 past the border
                    let inner_y = menu_y + 1;
                    if col >= menu_x
                        && col < menu_x.saturating_add(menu_width)
                        && row >= inner_y
                        && row < inner_y.saturating_add(items.len() as u16)
                    {
                        let clicked_index = (row - inner_y) as usize;
                        if clicked_index < items.len() {
                            self.context_menu_state.as_mut().unwrap().selected_index =
                                clicked_index;
                            self.execute_context_menu_item();
                            return;
                        }
                    }
                    // Click was outside the menu — just dismiss it
                    self.dismiss_context_menu();
                    return;
                }

                let input_area = self.last_input_area.get();
                let selectable_area = self.last_selectable_area.get();

                let in_input = input_area.width > 0
                    && input_area.height > 0
                    && mouse_event.row >= input_area.y
                    && mouse_event.row < input_area.y.saturating_add(input_area.height)
                    && mouse_event.column >= input_area.x
                    && mouse_event.column < input_area.x.saturating_add(input_area.width);

                let in_selectable = selectable_area.width > 0
                    && selectable_area.height > 0
                    && mouse_event.row >= selectable_area.y
                    && mouse_event.row < selectable_area.y.saturating_add(selectable_area.height)
                    && mouse_event.column >= selectable_area.x
                    && mouse_event.column < selectable_area.x.saturating_add(selectable_area.width);

                // Check for click on a thinking block header (takes priority over text selection).
                if let Some(&hash) = self.thinking_row_map.borrow().get(&mouse_event.row) {
                    if self.thinking_expanded.contains(&hash) {
                        self.thinking_expanded.remove(&hash);
                    } else {
                        self.thinking_expanded.insert(hash);
                    }
                    self.invalidate_transcript();
                    return;
                }

                if in_input {
                    self.focus = FocusTarget::Input;
                    self.clear_selection();
                } else if selectable_area.width == 0 || selectable_area.height == 0 {
                    self.click_count = 0;
                } else if in_selectable {
                    self.focus = FocusTarget::Transcript;

                    let current_pos = (mouse_event.column, mouse_event.row);
                    let now = std::time::Instant::now();

                    // Check for double-click
                    if self.is_double_click(current_pos) {
                        self.click_count += 1;
                        if self.click_count >= 3 {
                            // Triple-click: select the paragraph (run of
                            // non-blank rows) containing the click. Falls back
                            // to a single line if no paragraph is detected.
                            if let Some((start_row, end_row, end_col)) =
                                self.find_paragraph_boundaries(current_pos.1)
                            {
                                self.selection_anchor =
                                    Some((selectable_area.x, self.content_line_of_screen_row(start_row)));
                                self.selection_focus =
                                    Some((end_col, self.content_line_of_screen_row(end_row)));
                            } else {
                                self.selection_anchor = Some((
                                    selectable_area.x,
                                    self.content_line_of_screen_row(current_pos.1),
                                ));
                                self.selection_focus = Some((
                                    selectable_area
                                        .x
                                        .saturating_add(selectable_area.width)
                                        .saturating_sub(1),
                                    self.content_line_of_screen_row(current_pos.1),
                                ));
                            }
                            self.click_count = 0; // Reset for next click sequence
                        } else {
                            // Double-click: select word
                            if let Some((start, end)) =
                                self.find_word_boundaries(current_pos.0, current_pos.1)
                            {
                                let line = self.content_line_of_screen_row(current_pos.1);
                                self.selection_anchor = Some((start, line));
                                self.selection_focus = Some((end, line));
                            }
                        }
                    } else {
                        // Single click or new click sequence
                        self.click_count = 1;
                        let line = self.content_line_of_screen_row(current_pos.1);
                        self.selection_anchor = Some((current_pos.0, line));
                        self.selection_focus = Some((current_pos.0, line));
                        *self.selection_text.borrow_mut() = String::new();
                    }

                    self.last_click_time = Some(now);
                    self.last_click_position = Some(current_pos);
                } else {
                    self.click_count = 0;
                    self.clear_selection();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Dismiss context menu on drag
                self.dismiss_context_menu();

                // Continue drag — clamp to the selectable frame bounds so dragging
                // outside extends selection to the edge rather than cancelling.
                if self.selection_anchor.is_some() {
                    let selectable_area = self.last_selectable_area.get();
                    if selectable_area.width > 0 && selectable_area.height > 0 {
                        // Edge autoscroll: a drag held past the viewport edge
                        // scrolls so the selection can reach text that is not
                        // currently on screen — in copy mode AND in normal
                        // drags (jcode autoscrolls on any edge drag).
                        // Done before the clamp below, which would otherwise
                        // throw the out-of-bounds row away.
                        self.copy_mode_edge_autoscroll(mouse_event.row);
                        let clamped_col = mouse_event.column.max(selectable_area.x).min(
                            selectable_area
                                .x
                                .saturating_add(selectable_area.width)
                                .saturating_sub(1),
                        );
                        let clamped_row = mouse_event.row.max(selectable_area.y).min(
                            selectable_area
                                .y
                                .saturating_add(selectable_area.height)
                                .saturating_sub(1),
                        );
                        self.selection_focus = Some((clamped_col, self.content_line_of_screen_row(clamped_row)));
                        self.click_count = 0; // Reset on drag to prevent further double-clicks
                        // Stream the selection to the clipboard as it grows.
                        // `selection_text` is filled by the renderer from the
                        // previous frame, so this trails the pointer by one
                        // frame — imperceptible, and it avoids re-extracting
                        // the text the renderer already has.
                        if self.copy_mode_active() {
                            let sel_text = self.selection_text.borrow().clone();
                            crate::tui::clipboard::copy_selection_live(&sel_text);
                        }
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // Copy mode stays on across clicks so a second drag can start
                // without re-entering; Esc or Ctrl+T leaves it. Flush the final
                // selection first — the last drag frame may not have moved the
                // text, and the user is done dragging now.
                if self.copy_mode_active() && self.selection_anchor != self.selection_focus {
                    let sel_text = self.selection_text.borrow().clone();
                    crate::tui::clipboard::copy_selection_live(&sel_text);
                    return;
                }
                // Clear if no actual drag (single click = no selection)
                if self.selection_anchor == self.selection_focus {
                    self.clear_selection();
                } else {
                    // jcode parity (iter-672): a finalized drag selection ALWAYS
                    // copies on release — there is no opt-out setting — and the
                    // highlight stays visible until the next click, so a
                    // successful copy does not look like it failed.
                    let sel_text = self.selection_text.borrow().clone();
                    if !sel_text.is_empty() {
                        let outcome = crate::tui::clipboard::copy(&sel_text);
                        if outcome.is_copied() {
                            self.push_notification(
                                NotificationKind::Info,
                                outcome.status_message(),
                                Some(1),
                            );
                        } else {
                            self.push_notification(
                                NotificationKind::Warning,
                                outcome.status_message(),
                                Some(4),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
