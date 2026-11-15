//! The identity switch: `Shift+Tab` and `/agent` pick the acting employee for
//! the CURRENT session only.
//!
//! (session-hub plan T4) Nothing in the tree switched the acting employee
//! before this. The switch here is session-scoped on purpose: it sets the
//! agent's bylaws seat and charter for this conversation and stamps the
//! `sessions.employee_id` binding — the same pair the gateway performs from
//! the turn prologue — and writes NOTHING to the config. A second session in
//! the same process keeps its own seat, and the operator's instructions are
//! never silently re-pointed at another employee on another conversation.

use super::*;

impl App {
    /// Roster rows for the picker, from the store's authoritative employees
    /// table and the session-tag counts that show how many sessions each seat
    /// has run.
    fn load_agent_roster(
        &self,
    ) -> Option<(
        Vec<operant_core::database::EmployeeRecord>,
        Vec<(String, usize)>,
    )> {
        let db_path = self.config.database_path.clone();
        let db = operant_core::database::Database::init(db_path).ok()?;
        let roster = db.list_employee_roster().ok()?;
        // The counts are decoration: a store whose session tags cannot be read
        // must still offer the roster rather than refuse the picker.
        let counts = db
            .list_session_employees()
            .unwrap_or_default()
            .into_iter()
            .map(|(id, _, n)| (id, n))
            .collect();
        Some((roster, counts))
    }

    /// Open the picker over the roster (the key path and `/agent` with no
    /// argument). A silently empty roster says so rather than opening an
    /// empty menu.
    pub fn open_agent_picker(&mut self) {
        let Some((roster, counts)) = self.load_agent_roster() else {
            self.set_status_notice("the org database is unavailable".to_string());
            return;
        };
        self.open_agent_picker_with(roster, &counts);
    }

    /// Testable entry: build the picker from an explicit roster.
    pub fn open_agent_picker_with(
        &mut self,
        roster: Vec<operant_core::database::EmployeeRecord>,
        session_counts: &[(String, usize)],
    ) {
        if roster.is_empty() {
            self.set_status_notice("the roster is empty".to_string());
            return;
        }
        self.agent_roster_cache = roster.clone();
        let items: Vec<crate::tui::dialog_select::SelectItem> = roster
            .into_iter()
            .map(|rec| {
                let count = session_counts
                    .iter()
                    .find(|(id, _)| id == &rec.employee_id)
                    .map(|(_, n)| *n)
                    .unwrap_or(0);
                let dept = rec.department.clone().unwrap_or_else(|| "—".to_string());
                crate::tui::dialog_select::SelectItem {
                    id: rec.employee_id.clone(),
                    title: rec.name.clone(),
                    description: format!("{dept} · {}", rec.role),
                    category: dept.clone(),
                    badge: Some(format!(
                        "{count} session{}",
                        if count == 1 { "" } else { "s" }
                    )),
                }
            })
            .collect();
        self.agent_picker = crate::tui::dialog_select::DialogSelectState::new(
            "Acting employee",
            items,
        );
        self.agent_picker.open();
    }

    /// Apply a seat the operator picked: the seat and its charter go onto the
    /// agent at the next submit as one pair, and the session's employee row
    /// is stamped so the hub's axis agrees with who ran it.
    pub fn apply_acting_seat(&mut self, employee_id: &str) {
        let Some(record) = self
            .agent_roster_cache
            .iter()
            .find(|r| r.employee_id == employee_id)
            .cloned()
        else {
            self.set_status_notice(format!("unknown seat '{employee_id}'"));
            return;
        };
        let name = record.name.clone();
        let charter = record.system_prompt.clone();
        self.acting_seat = Some(record.employee_id.clone());
        self.acting_charter = charter;
        if let Some(session_id) = self.current_session.clone() {
            let db_path = self.config.database_path.clone();
            match operant_core::database::Database::init(db_path).and_then(|db| {
                db.set_session_employee(&session_id, &record.employee_id)
            }) {
                Ok(()) => {}
                Err(error) => {
                    self.set_status_notice(format!(
                        "acting as {name} — but the session row was not stamped ({})",
                        error
                    ));
                    return;
                }
            }
        }
        self.set_status_notice(format!("acting as {name} (this session only)"));
    }

    /// `/agent <seat>` — quick switch by id or by name, case-insensitively.
    pub fn apply_acting_seat_by_query(&mut self, query: &str) {
        if self.agent_roster_cache.is_empty() {
            let Some((roster, _)) = self.load_agent_roster() else {
                self.set_status_notice("the org database is unavailable".to_string());
                return;
            };
            self.agent_roster_cache = roster;
        }
        let query_lower = query.to_lowercase();
        let match_id = self
            .agent_roster_cache
            .iter()
            .find(|r| r.employee_id.to_lowercase() == query_lower)
            .map(|r| r.employee_id.clone())
            .or_else(|| {
                self.agent_roster_cache
                    .iter()
                    .find(|r| r.name.to_lowercase() == query_lower)
                    .map(|r| r.employee_id.clone())
            });
        match match_id {
            Some(id) => self.apply_acting_seat(&id),
            None => self.set_status_notice(format!(
                "no seat matches '{query}' (try: /agent with no argument)"
            )),
        }
    }
}
