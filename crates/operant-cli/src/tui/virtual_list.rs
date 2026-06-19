use ratatui::{buffer::Buffer, layout::Rect};
use std::collections::HashMap;

pub trait VirtualItem {
    fn measure_height(&self, width: u16) -> u16;
    fn render(&self, area: Rect, buf: &mut Buffer, selected: bool);
    fn search_text(&self) -> String;
    fn is_section_header(&self) -> bool {
        false
    }
}

pub struct VirtualList<T: VirtualItem> {
    pub items: Vec<T>,
    height_cache: HashMap<(usize, u16), u16>,
    pub scroll_offset: u16,
    pub viewport_height: u16,
    pub sticky_bottom: bool,
    pub selected_index: Option<usize>,
    search_index: Vec<String>,
    last_search: Option<String>,
    search_matches: Vec<usize>,
}

impl<T: VirtualItem> VirtualList<T> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            height_cache: HashMap::new(),
            scroll_offset: 0,
            viewport_height: 24,
            sticky_bottom: true,
            selected_index: None,
            search_index: Vec::new(),
            last_search: None,
            search_matches: Vec::new(),
        }
    }

    pub fn set_items(&mut self, items: Vec<T>) {
        self.search_index = items.iter().map(|i| i.search_text()).collect();
        self.items = items;
        self.height_cache.clear();
        if self.sticky_bottom {
            self.jump_to_bottom();
        }
        self.last_search = None;
        self.search_matches.clear();
    }

    pub fn push_item(&mut self, item: T) {
        self.search_index.push(item.search_text());
        self.items.push(item);
        if self.sticky_bottom {
            self.jump_to_bottom();
        }
    }

    pub fn on_resize(&mut self, new_viewport_height: u16) {
        self.viewport_height = new_viewport_height;
        self.height_cache.clear();
    }

    fn item_height(&mut self, idx: usize, width: u16) -> u16 {
        let key = (idx, width);
        if let Some(&h) = self.height_cache.get(&key) {
            return h;
        }
        let h = if idx < self.items.len() {
            self.items[idx].measure_height(width).max(1)
        } else {
            1
        };
        self.height_cache.insert(key, h);
        h
    }

    pub fn total_height(&mut self, width: u16) -> u16 {
        (0..self.items.len())
            .map(|i| self.item_height(i, width))
            .sum::<u16>()
    }

    pub fn scroll_to_index(&mut self, idx: usize, width: u16) {
        let mut row = 0u16;
        for i in 0..idx.min(self.items.len()) {
            row = row.saturating_add(self.item_height(i, width));
        }
        self.scroll_offset = row.saturating_sub(3);
    }

    pub fn jump_to_bottom(&mut self) {
        self.scroll_offset = u16::MAX;
    }

    pub fn scroll_up(&mut self, rows: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(rows);
        self.sticky_bottom = false;
    }

    pub fn scroll_down(&mut self, rows: u16, width: u16) {
        let total = self.total_height(width);
        let max_offset = total.saturating_sub(self.viewport_height);
        self.scroll_offset = (self.scroll_offset + rows).min(max_offset);
        if self.scroll_offset >= max_offset {
            self.sticky_bottom = true;
        }
    }

    pub fn sticky_header_index(&mut self, width: u16) -> Option<usize> {
        let mut row = 0u16;
        let mut last_header: Option<usize> = None;
        for i in 0..self.items.len() {
            let h = self.item_height(i, width);
            if row + h > self.scroll_offset {
                break;
            }
            if self.items[i].is_section_header() {
                last_header = Some(i);
            }
            row += h;
        }
        last_header
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        if self.items.is_empty() || area.height == 0 {
            return;
        }

        self.viewport_height = area.height;
        let width = area.width;

        let total = self.total_height(width);
        let max_offset = total.saturating_sub(area.height);
        if self.scroll_offset > max_offset {
            self.scroll_offset = max_offset;
        }

        let mut current_row = 0u16;
        let mut screen_row = area.y;

        for idx in 0..self.items.len() {
            let h = self.item_height(idx, width);
            let item_end = current_row + h;

            if item_end <= self.scroll_offset {
                current_row = item_end;
                continue;
            }

            if current_row >= self.scroll_offset + area.height {
                break;
            }

            let visible_start = if current_row < self.scroll_offset {
                self.scroll_offset - current_row
            } else {
                0
            };
            let visible_rows = h
                .saturating_sub(visible_start)
                .min(area.y + area.height - screen_row);

            if visible_rows == 0 {
                current_row = item_end;
                continue;
            }

            let item_area = Rect {
                x: area.x,
                y: screen_row,
                width: area.width,
                height: visible_rows,
            };

            let selected = self.selected_index == Some(idx);
            self.items[idx].render(item_area, buf, selected);

            screen_row += visible_rows;
            current_row = item_end;
        }

        if let Some(header_idx) = self.sticky_header_index(width) {
            let mut row = 0u16;
            for i in 0..header_idx {
                row = row.saturating_add(self.item_height(i, width));
            }
            if row < self.scroll_offset {
                let h = self.item_height(header_idx, width).min(area.height);
                let header_area = Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: h,
                };
                for by in header_area.y..header_area.y + h {
                    for bx in header_area.x..header_area.x + header_area.width {
                        if let Some(cell) = buf.cell_mut((bx, by)) {
                            cell.set_char(' ');
                        }
                    }
                }
                self.items[header_idx].render(header_area, buf, false);
            }
        }
    }

    pub fn warm_search_index(&mut self) {
        self.search_index = self.items.iter().map(|i| i.search_text()).collect();
    }

    pub fn find_matches(&mut self, query: &str) -> &[usize] {
        if self.last_search.as_deref() == Some(query) {
            return &self.search_matches;
        }
        let q = query.to_lowercase();
        self.search_matches = self
            .search_index
            .iter()
            .enumerate()
            .filter(|(_, text)| text.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.last_search = Some(query.to_string());
        &self.search_matches
    }

    pub fn next_match(&mut self, query: &str, current_idx: usize, width: u16) -> Option<usize> {
        let matches = self.find_matches(query).to_vec();
        let next = matches.iter().find(|&&i| i > current_idx).copied()
            .or_else(|| matches.first().copied());
        if let Some(idx) = next {
            self.scroll_to_index(idx, width);
        }
        next
    }

    pub fn prev_match(&mut self, query: &str, current_idx: usize, width: u16) -> Option<usize> {
        let matches = self.find_matches(query).to_vec();
        let prev = matches.iter().rev().find(|&&i| i < current_idx).copied()
            .or_else(|| matches.last().copied());
        if let Some(idx) = prev {
            self.scroll_to_index(idx, width);
        }
        prev
    }
}

impl<T: VirtualItem> Default for VirtualList<T> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestItem {
        text: String,
        height: u16,
    }

    impl VirtualItem for TestItem {
        fn measure_height(&self, _width: u16) -> u16 { self.height }
        fn render(&self, _area: Rect, _buf: &mut Buffer, _selected: bool) {}
        fn search_text(&self) -> String { self.text.clone() }
    }

    #[test]
    fn new_list_is_empty() {
        let list: VirtualList<TestItem> = VirtualList::new();
        assert!(list.items.is_empty());
    }

    #[test]
    fn push_item_and_count() {
        let mut list: VirtualList<TestItem> = VirtualList::new();
        list.push_item(TestItem { text: "a".into(), height: 1 });
        list.push_item(TestItem { text: "b".into(), height: 2 });
        assert_eq!(list.items.len(), 2);
    }

    #[test]
    fn scroll_up_does_not_go_negative() {
        let mut list: VirtualList<TestItem> = VirtualList::new();
        list.scroll_offset = 0;
        list.scroll_up(5);
        assert_eq!(list.scroll_offset, 0);
    }

    #[test]
    fn find_matches_case_insensitive() {
        let mut list: VirtualList<TestItem> = VirtualList::new();
        list.push_item(TestItem { text: "Hello World".into(), height: 1 });
        list.push_item(TestItem { text: "goodbye".into(), height: 1 });
        let matches = list.find_matches("hello");
        assert_eq!(matches, &[0]);
    }
}
