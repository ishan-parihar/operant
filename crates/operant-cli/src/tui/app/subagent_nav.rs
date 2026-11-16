//! The subagent navigator's App methods: collect this session's descendants and
//! mount the surface.
//!
//! (session-hub plan T7) The ledger is ONE file for the whole home
//! (`async_delegation.rs`), so the collection step is where the session scoping
//! happens: walk `parent → children` outward from the CURRENT session id and
//! keep only what that walk reaches. jcode's rationale is the same — other
//! sessions can share a swarm in the same repo without this session having
//! spawned them, and showing those "would be noise"
//! (`tui_state.rs:2804-2809`).

use super::*;
use crate::tui::session_hub::SubagentScope;
use crate::tui::subagent_navigator::{NavigatorEntry, descendants};

impl App {
    /// The delegation ids that belong to THIS session: its descendants, plus any
    /// registry-only child it spawned that the ledger has not recorded yet.
    ///
    /// `None` when there is no current session, which every caller must read as
    /// "show nothing" rather than "show everything": the delegation registry is
    /// process-wide, and an unscoped read is how a fresh session came to list
    /// another session's subagents as its own (the inline swarm gallery did
    /// exactly that). One walk, two surfaces — the navigator modal and the inline
    /// gallery — because two scoping rules is how they drift apart.
    pub(super) fn session_descendant_ids(&self) -> Option<std::collections::HashSet<String>> {
        let root = self.current_session.as_ref()?;
        let records = operant_core::tools::async_delegation::list_records();
        let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        // The parent edge is the child's `parent_session_id`; a record with none
        // is a top-level delegation this session did not spawn.
        let mut edges: Vec<(String, String)> = records
            .iter()
            .filter_map(|r| {
                r.parent_session_id
                    .clone()
                    .map(|parent| (parent, r.delegation_id.clone()))
            })
            .collect();
        // A delegation is not a session: the child it spawned runs in its OWN
        // session, so the next hop is `delegation → that session`. The session
        // ids are TRANSPARENT — the hop connects the tree without spending a
        // depth level, because a session has no row of its own.
        let session_ids: Vec<&str> = records
            .iter()
            .filter_map(|r| r.session_id.as_deref())
            .collect();
        edges.extend(
            records
                .iter()
                .filter_map(|r| r.session_id.clone().map(|s| (r.delegation_id.clone(), s))),
        );

        for (delegation_id, _depth, _parent) in descendants(root, &edges, &session_ids) {
            ids.insert(delegation_id);
        }
        // A registry-only child (dispatched in-process, not yet in the ledger) is
        // still this session's child: the registry carries the same edge.
        for record in &records {
            if record.parent_session_id.as_deref() == Some(root.as_str()) {
                ids.insert(record.delegation_id.clone());
            }
        }
        Some(ids)
    }

    /// Mount the navigator over this session's own descendants, or refuse.
    ///
    /// Returns `true` when the surface mounted. An empty roster never mounts
    /// (omp's `requireContent`): an overlay flashing open reads as a broken key.
    pub fn open_subagent_navigator(&mut self) -> bool {
        let Some(root) = self.current_session.clone() else {
            self.set_status_notice(
                "no session to inspect — the navigator is session-scoped".to_string(),
            );
            return false;
        };
        let records = operant_core::tools::async_delegation::list_records();
        // The parent edge is the child's `parent_session_id`; a record with none
        // is a top-level delegation this session did not spawn, so it never
        // enters the walk.
        let mut edges: Vec<(String, String)> = records
            .iter()
            .filter_map(|r| {
                r.parent_session_id
                    .clone()
                    .map(|parent| (parent, r.delegation_id.clone()))
            })
            .collect();
        // A delegation is not a session: the child it spawned runs in its OWN
        // session, so the next hop down the tree is `delegation → that session`.
        // Without it the walk asked for `children[dlg-1]` while the ledger held
        // the key `sess-1` (the child's `session_id`), so no nested delegation
        // was reachable and the walk stopped at depth 1 — the tree view and the
        // depth indent could never render past the first level.
        // MEASURED 2026-11-15 (seventh-pass audit, finding 2).
        //
        // The session ids are TRANSPARENT: the hop connects the tree without
        // spending a depth level, because a session has no row of its own and a
        // depth is what indents a visible row.
        let session_ids: Vec<&str> = records
            .iter()
            .filter_map(|r| r.session_id.as_deref())
            .collect();
        edges.extend(
            records
                .iter()
                .filter_map(|r| r.session_id.clone().map(|s| (r.delegation_id.clone(), s))),
        );
        // (audit finding) Index the ledger once. The lookup below ran per
        // descendant over every record in a home-wide file (n x m) on the
        // interface thread; the ledger merges across processes now, so it only
        // grows.
        let by_id: std::collections::HashMap<
            &str,
            &operant_core::tools::async_delegation::AsyncDelegationRecord,
        > = records
            .iter()
            .map(|record| (record.delegation_id.as_str(), record))
            .collect();
        let mut entries: Vec<NavigatorEntry> = descendants(&root, &edges, &session_ids)
            .into_iter()
            .filter_map(|(delegation_id, depth, parent)| {
                let record = by_id.get(delegation_id.as_str()).copied()?;
                Some(NavigatorEntry {
                    id: delegation_id,
                    goal: record.goal.clone(),
                    status: format!("{:?}", record.status).to_lowercase(),
                    // Missing usage renders as an em dash, never `0`: the rule
                    // the delegation rows already follow (`render.rs:1128-1136`).
                    model: if record.model.trim().is_empty() {
                        "—".to_string()
                    } else {
                        record.model.clone()
                    },
                    depth,
                    parent,
                })
            })
            .collect();
        // A registry-only child (dispatched in-process, not yet in the ledger)
        // is still this session's child: the registry carries the same edge.
        for record in &records {
            if record.parent_session_id.as_deref() != Some(root.as_str()) {
                continue;
            }
            if entries.iter().any(|e| e.id == record.delegation_id) {
                continue;
            }
            entries.push(NavigatorEntry {
                id: record.delegation_id.clone(),
                goal: record.goal.clone(),
                status: format!("{:?}", record.status).to_lowercase(),
                model: if record.model.trim().is_empty() {
                    "—".to_string()
                } else {
                    record.model.clone()
                },
                depth: 1,
                parent: Some(root.clone()),
            });
        }
        let mounted = self.subagent_navigator.open(Some(&root), entries);
        if !mounted {
            self.set_status_notice("this session has no subagents".to_string());
        }
        mounted
    }

    /// Mount the SESSION PICKER restricted to this session's own subagents —
    /// the `<-` gesture on an ongoing session.
    ///
    /// The operator's ask: "if a session is ongoing, and not a new session,
    /// then the `<-` left arrow must open the subagent filtered session-picker,
    /// rather than the global session picker."
    ///
    /// # Why the hub and not a bespoke surface
    ///
    /// The rows, the preview pane, the rename/archive guards and the
    /// `FocusSubagent` selection are all already the hub's, so a second surface
    /// would re-implement them to answer the same question. Scoping the picker
    /// also matches codex, whose `/subagents` is the generic `SelectionView`
    /// with a subagent row source (`source_kinds = SubAgentThreadSpawn`,
    /// `ancestor_thread_id = root`) rather than a bespoke widget.
    ///
    /// # Why it can refuse
    ///
    /// Returns `false` — and leaves the hub closed — when there is no current
    /// session or this session spawned nobody (omp's `requireContent`). An
    /// empty filtered picker reads as a broken key, and the caller's `false` is
    /// what lets `←` fall through to the global hub instead of being swallowed.
    pub fn open_subagent_scoped_hub(&mut self) -> bool {
        // The root is cloned FIRST: `session_descendant_ids` borrows `self`, and
        // the scope has to outlive that borrow.
        let Some(root) = self.current_session.clone() else {
            self.set_status_notice(
                "no session to inspect — the picker is session-scoped".to_string(),
            );
            return false;
        };
        let Some(ids) = self.session_descendant_ids() else {
            return false;
        };
        if ids.is_empty() {
            // Not an error: a session with no children is the common case, and
            // the ← gesture is simply the global picker for it.
            self.set_status_notice("this session has no subagents".to_string());
            return false;
        }
        // One scope, two fields, from one source: the id set is what the hub
        // filters on AND what the open guard counted, so the picker can never
        // open empty or refuse a full one.
        // A `Show more` page still in flight leaves `session_load_more` true,
        // and the drain reads it to decide APPEND vs REPLACE. Carrying it into a
        // scoped open would append the scoped page to the previous session's
        // rows at a global cursor — and skip `records_snapshot()` entirely, so
        // no delegation row would ever arrive.
        self.session_load_more = false;
        self.session_hub
            .open_subagent_scoped(Some(std::time::Instant::now()), SubagentScope { root, ids });
        self.session_list_pending = true;
        true
    }

    /// Enter on a navigator row focuses that child, through the hub's own
    /// selection path (`HubSelection::FocusSubagent`).
    pub fn focus_navigator_selection(&mut self) {
        let Some(entry) = self.subagent_navigator.selected().cloned() else {
            self.subagent_navigator.close();
            return;
        };
        self.session_hub.focused_subagent = Some(entry.id.clone());
        self.subagent_navigator.close();
        // (audit finding) A BYTE offset into a string panics when the cut lands
        // inside a multi-byte character. Ids minted by `new_id` are ASCII, but
        // the id comes from the ledger, so the truncation walks chars instead.
        let short: String = entry.id.chars().take(8).collect();
        self.set_status_notice(format!("focused subagent {short}"));
    }
}
