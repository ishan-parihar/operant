//! The subagent navigator: this session's own children, as their own surface.
//!
//! (session-hub plan T7, closing item 5's second half) Subagent rows were
//! appended into the GLOBAL hub list from the in-process registry, so the hub
//! could only ever show this process's delegations, mixed in with every
//! unrelated session. This surface is session-scoped instead: it lists the
//! CURRENT session's descendants, from the registry plus T5's durable ledger.
//!
//! Three rules carry the semantics:
//!
//! 1. **Scope the walk to this session's own descendants.** The ledger is ONE
//!    file for the whole home, so an unscoped list shows every session's
//!    children. jcode builds a `parent → children` index from the live edge and
//!    walks outward from `self_id`, cycle-safe, excluding self
//!    (`tui_state.rs:2754-2790`), because other sessions can share a swarm in the
//!    same repo without this session having spawned them — and showing those
//!    "would be noise" (`:2804-2809`).
//! 2. **Flat by default, tree on `t`.** codex's `/subagents` picker is a flat
//!    descendant list and does not indent (`app/agent_picker.rs:21-105`); jcode's
//!    swarm panel is flat too. Only oh-my-pi builds a real tree, toggled with `t`
//!    (`agent-hub-projection.ts:343-438`). The flat default follows the majority.
//! 3. **Never mount on an empty roster** (omp's `requireContent`), and render
//!    missing usage as `—` rather than `0`, the rule the delegation rows already
//!    follow (`render.rs:1128-1136`).

use std::collections::HashMap;

/// One child in the navigator, with the depth the subtree walk found it at.
#[derive(Debug, Clone)]
pub struct NavigatorEntry {
    /// The delegation id (or the child's session id when there is no record).
    pub id: String,
    /// What the child was asked to do.
    pub goal: String,
    /// `pending` | `completed` | `failed` — the delegation status, as text.
    pub status: String,
    /// The model the child ran on, or `—` when nothing recorded one.
    pub model: String,
    /// Zero for a direct child; one per extra hop.
    pub depth: usize,
    /// The parent of this entry within the walk, for the tree view.
    pub parent: Option<String>,
}

/// The navigator's state.
#[derive(Debug, Default)]
pub struct SubagentNavigatorState {
    pub visible: bool,
    pub entries: Vec<NavigatorEntry>,
    /// Index into `entries` (the flat order), so the cursor survives the toggle.
    pub selected: usize,
    /// `true` = flat descendant list (codex/jcode), `false` = parent/child tree.
    pub flat: bool,
    /// The session this surface was opened for. `None` means it never mounted.
    pub session_id: Option<String>,
    /// The row order when the surface opened (omp's `#frozenRank`): a status
    /// change must not re-sort rows under the operator's cursor.
    frozen_order: Vec<String>,
}

impl SubagentNavigatorState {
    /// Collect this session's own descendants from the registry plus the ledger,
    /// and mount. Returns `false` — and stays hidden — when the roster is empty
    /// (omp's `requireContent`).
    pub fn open(&mut self, session_id: Option<&str>, records: Vec<NavigatorEntry>) -> bool {
        if session_id.is_none() || records.is_empty() {
            self.close();
            return false;
        }
        self.session_id = session_id.map(str::to_string);
        // Flat is the default EVERY time the surface mounts (codex and jcode are
        // both flat; only oh-my-pi builds a tree). Deriving `Default` would leave
        // `flat: false`, so the first mount opened in tree view.
        self.flat = true;
        self.entries = records;
        self.frozen_order = self.entries.iter().map(|e| e.id.clone()).collect();
        self.selected = 0;
        self.visible = true;
        true
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.entries.clear();
        self.frozen_order.clear();
        self.session_id = None;
        self.selected = 0;
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
    }

    pub fn selected(&self) -> Option<&NavigatorEntry> {
        self.entries.get(self.selected)
    }

    /// `t`: flat descendant list ⇄ parent/child tree. The order is frozen either
    /// way; the toggle changes the INDENT, not the ranking.
    pub fn toggle_view(&mut self) {
        self.flat = !self.flat;
    }

    /// The indent for a row in the current view.
    pub fn indent(&self, entry: &NavigatorEntry) -> usize {
        if self.flat { 0 } else { entry.depth }
    }
}

/// Walk `parent → children` outward from `root`, cycle-safe, excluding the root.
///
/// (session-hub plan T7) jcode's `filter_inline_swarm_subtree`
/// (`tui_state.rs:2754-2790`): a visited set, so a ledger with a cycle cannot
/// hang the surface, and `root` never appears as its own descendant.
///
/// An edge whose `child` is in `transparent` is a hop THROUGH a node the surface
/// never draws — the delegation → its own session hop, where the session has no
/// row of its own. The hop keeps the walk connected without spending a depth
/// level, because a depth is what indents a visible row: counting it would put
/// every grandchild one column too far right and name an invisible node as its
/// parent.
pub fn descendants(
    root: &str,
    edges: &[(String, String)],
    transparent: &[&str],
) -> Vec<(String, usize, Option<String>)> {
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for (parent, child) in edges {
        children
            .entry(parent.as_str())
            .or_default()
            .push(child.as_str());
    }
    let mut out: Vec<(String, usize, Option<String>)> = Vec::new();
    let mut seen: Vec<String> = vec![root.to_string()];
    // (node, depth, the nearest VISIBLE ancestor)
    let mut frontier: Vec<(&str, usize, Option<String>)> = vec![(root, 0, None)];
    while let Some((parent, depth, visible_parent)) = frontier.pop() {
        let parent: &str = parent;
        if let Some(kids) = children.get(parent) {
            for kid in kids {
                if seen.iter().any(|s| s == kid) {
                    continue;
                }
                seen.push((*kid).to_string());
                if transparent.contains(kid) {
                    // Pass through: same depth, and the child inherits the
                    // visible ancestor rather than naming this phantom node.
                    frontier.push((kid, depth, visible_parent.clone()));
                    continue;
                }
                let parent_name = visible_parent.clone().unwrap_or_else(|| parent.to_string());
                out.push(((*kid).to_string(), depth + 1, Some(parent_name.clone())));
                frontier.push((kid, depth + 1, Some((*kid).to_string())));
            }
        }
    }
    out
}

/// Order entries by the frozen opening order, so a status change cannot re-sort
/// rows under the cursor (omp's `#frozenRank`).
pub fn apply_frozen_order(entries: &mut Vec<NavigatorEntry>, order: &[String]) {
    if order.is_empty() {
        return;
    }
    entries.sort_by_key(|e| {
        order
            .iter()
            .position(|id| id == &e.id)
            .unwrap_or(usize::MAX)
    });
}

/// Draw the navigator: one row per descendant, indented by depth in tree view.
pub fn draw_subagent_navigator(
    frame: &mut ratatui::Frame,
    state: &SubagentNavigatorState,
    area: ratatui::layout::Rect,
) {
    if !state.visible {
        return;
    }
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;

    use crate::tui::overlays::{ModalSpec, modal_frame};
    use crate::tui::theme_colors;

    let rows = state.entries.len() as u16;
    let height = (rows + 4).clamp(6, 24);
    let layout = modal_frame(
        frame,
        area,
        &ModalSpec {
            title: "Subagents",
            hint: "t view · Enter focus · Esc close",
            width: 78,
            height,
            header_height: 1,
            footer_height: 0,
            border_fg: theme_colors::accent(),
        },
    );
    let mut lines: Vec<Line> = Vec::with_capacity(state.entries.len());
    for (index, entry) in state.entries.iter().enumerate() {
        let indent = "  ".repeat(state.indent(entry));
        let marker = if index == state.selected {
            "› "
        } else {
            "  "
        };
        let selected_style = if index == state.selected {
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        // `usage —`, never `usage 0`: missing data renders as an em dash.
        lines.push(Line::from(Span::styled(
            format!(
                "{marker}{indent}{}  {}  {}",
                entry.status, entry.model, entry.goal
            ),
            selected_style,
        )));
    }
    frame.render_widget(Paragraph::new(lines), layout.body_area);
}
