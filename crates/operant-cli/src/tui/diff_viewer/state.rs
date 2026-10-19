// diff_viewer/state.rs — DiffViewerState methods + Default.
//
// Extracted from the diff_viewer.rs monolith.

use super::*;

impl DiffViewerState {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            turn_files: Vec::new(),
            selected_file: 0,
            active_pane: DiffPane::FileList,
            diff_type: DiffType::GitDiff,
            detail_scroll: 0,
            render_cache: HashMap::new(),
            loading: false,
            git_rx: None,
            visible: false,
            collapsed: Vec::new(),
        }
    }

    /// Toggle collapsed state for the currently selected file.
    pub fn toggle_file_collapse(&mut self) {
        if let Some(c) = self.collapsed.get_mut(self.selected_file) {
            *c = !*c;
            self.detail_scroll = 0;
        }
    }

    /// Open the dialog and load diffs from the project root.
    pub fn open(&mut self, project_root: &std::path::Path) {
        self.open_for_type(DiffType::GitDiff, project_root);
    }

    /// Open directly in turn-diff mode.
    pub fn open_turn(&mut self, project_root: &std::path::Path) {
        self.open_for_type(DiffType::TurnDiff, project_root);
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        let count = self.files.len();
        if count == 0 {
            return;
        }
        if self.selected_file == 0 {
            self.selected_file = count - 1;
        } else {
            self.selected_file -= 1;
        }
        self.detail_scroll = 0;
    }

    pub fn select_next(&mut self) {
        let count = self.files.len();
        if count == 0 {
            return;
        }
        self.selected_file = (self.selected_file + 1) % count;
        self.detail_scroll = 0;
    }

    pub fn switch_pane(&mut self) {
        self.active_pane = match self.active_pane {
            DiffPane::FileList => DiffPane::Detail,
            DiffPane::Detail => DiffPane::FileList,
        };
    }

    pub fn toggle_diff_type(&mut self, project_root: &std::path::Path) {
        self.diff_type = match self.diff_type {
            DiffType::GitDiff => DiffType::TurnDiff,
            DiffType::TurnDiff => DiffType::GitDiff,
        };
        self.reload_files(project_root);
    }

    pub fn scroll_detail_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(3);
    }

    pub fn scroll_detail_down(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_add(3);
    }

    pub fn set_turn_diff(&mut self, files: Vec<FileDiffStats>) {
        self.turn_files = files;
        if self.diff_type == DiffType::TurnDiff {
            self.files = self.turn_files.clone();
            self.selected_file = 0;
            self.detail_scroll = 0;
            self.render_cache.clear();
            self.collapsed = vec![false; self.files.len()];
        }
    }

    fn open_for_type(&mut self, diff_type: DiffType, project_root: &std::path::Path) {
        self.diff_type = diff_type;
        self.reload_files(project_root);
        self.visible = true;
    }

    fn reload_files(&mut self, project_root: &std::path::Path) {
        self.selected_file = 0;
        self.detail_scroll = 0;
        self.render_cache.clear();
        match self.diff_type {
            DiffType::GitDiff => {
                // TUI-11: `git diff` runs on a worker thread, so a large repository
                // does not stall the event loop. A new load replaces the receiver,
                // so the result of an older load is dropped when it arrives.
                self.files.clear();
                self.collapsed.clear();
                self.loading = true;
                let (tx, rx) = std::sync::mpsc::channel();
                self.git_rx = Some(std::sync::Arc::new(std::sync::Mutex::new(rx)));
                let root = project_root.to_path_buf();
                std::thread::spawn(move || {
                    let _ = tx.send(load_git_diff(&root));
                });
            }
            DiffType::TurnDiff => {
                self.git_rx = None;
                self.loading = false;
                self.files = self.turn_files.clone();
                self.collapsed = vec![false; self.files.len()];
            }
        }
    }

    /// Takes the git diff once the worker has sent it. Returns true when the list changed.
    pub fn drain_git_diff(&mut self) -> bool {
        let Some(rx) = &self.git_rx else {
            return false;
        };
        let received = rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_recv();
        let files = match received {
            Ok(files) => files,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        self.git_rx = None;
        self.loading = false;
        self.collapsed = vec![false; files.len()];
        self.files = files;
        true
    }
}

impl Default for DiffViewerState {
    fn default() -> Self {
        Self::new()
    }
}
