//! Emacs-style kill ring for cut/yank operations.

#[derive(Debug, Clone)]
pub struct KillRing {
    /// List of killed text entries. Most recent is last.
    pub entries: Vec<String>,
    /// Maximum number of entries to keep (prevents unbounded growth).
    max_size: usize,
    /// Current position in kill ring when cycling with Alt+Y (None = most recent).
    pub current_index: Option<usize>,
    /// Tracks whether the last action was a kill (for combining consecutive kills).
    pub last_was_kill: bool,
}

impl KillRing {
    /// Create a new kill ring with default capacity.
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(32),
            max_size: 64,
            current_index: None,
            last_was_kill: false,
        }
    }

    #[expect(
        clippy::unwrap_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Add a kill entry. If the last operation was a kill, append to the most recent entry
    /// instead of creating a new one (for combining consecutive kills).
    pub fn push(&mut self, text: String) {
        if text.is_empty() {
            return;
        }

        if self.last_was_kill && !self.entries.is_empty() {
            // Append to the most recent entry (last_was_kill combines consecutive kills)
            self.entries.last_mut().unwrap().push_str(&text);
        } else {
            // New kill entry
            self.entries.push(text);
            if self.entries.len() > self.max_size {
                self.entries.remove(0);
            }
        }
        self.current_index = None; // Reset cycling to most recent
        self.last_was_kill = true;
    }

    /// Get the current kill to paste (most recent or current index if cycling).
    pub fn get_current(&self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }

        match self.current_index {
            None => self.entries.last().map(|s| s.as_str()),
            Some(idx) => self.entries.get(idx).map(|s| s.as_str()),
        }
    }

    /// Cycle backward through kill ring (Alt+Y after paste).
    pub fn cycle_backward(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        match self.current_index {
            None => {
                // Start cycling from the second-to-last entry
                if self.entries.len() > 1 {
                    self.current_index = Some(self.entries.len() - 2);
                }
            }
            Some(0) => {
                // Wrap around to the end
                self.current_index = Some(self.entries.len() - 1);
            }
            Some(idx) => {
                self.current_index = Some(idx - 1);
            }
        }
    }

    /// Mark that a non-kill action occurred (resets consecutive kill combination).
    pub fn mark_non_kill(&mut self) {
        self.last_was_kill = false;
    }
}

impl Default for KillRing {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PromptInput state
// ---------------------------------------------------------------------------
