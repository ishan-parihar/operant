//! `seat_budgets` — per-employee budget overrides (Wave 4,
//! ORGANISM-ARCHITECTURE §5).
//!
//! One row per seat. Fields are `NULL`-able so an override can pin just one
//! knob (say, the cap) and inherit the rest from `[bylaws].budget` —
//! precedence is per-field, resolved by [`resolve_budget`]. `cap` is NOT
//! nullable: an override row exists to cap, so a row with `cap = 0` means
//! "explicitly ungoverned" (an intentional release valve — the bylaws's
//! fail-open rule applies to budgets too, by the owner's "totally free or
//! ungoverned" directive).
//!
//! Enforcement is TURN-BOUNDARY v1: the gateway turn computes remaining
//! budget from the persistent session accumulator
//! ([`crate::gateway_session::PersistentSessionStore::employee_window_usage`])
//! and either refuses (hard) or injects the remainder into the turn (soft).
//! RT-376: this note used to end "Nothing here reaches into the agent loop
//! mid-flight — that is a recorded follow-up, not this contract". The first
//! half is still true of THIS module (it never reaches into the loop), but
//! the follow-up has since shipped: the agent stops a token-basis turn
//! mid-flight once the hard cap is crossed (`agent::run` — "a hard token cap
//! must stop a runaway turn AT the boundary, not after it"), scoped per-turn
//! by the envelope the gateway runner sets. What REMAINS a recorded
//! follow-up is the usd-basis per-call catalog cost inside the loop.

use crate::config::BudgetSettings;
use crate::error::{Error, Result};
use chrono::Datelike;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The override row. `None` fields inherit `[bylaws].budget`.
#[derive(Debug, Clone, Default)]
pub struct SeatBudget {
    pub employee_id: String,
    pub basis: Option<String>,
    pub window: Option<String>,
    pub cap: f64,
    pub mode: Option<String>,
}

impl SeatBudget {
    // NOTE (design): no "noop" special-casing. A present row with
    // `cap <= 0` is the DELIBERATE release valve — "this seat runs free
    // even though the org default caps" — and a row that sets nothing
    // meaningful is a write nobody makes. Resolution is simple: row
    // present → its per-field values win (cap 0 = ungoverned); row absent
    // → `[bylaws].budget`.
}

/// The resolved budget a seat actually runs under, after per-field
/// precedence. `None` = ungoverned (no budget consulted at all).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveBudget {
    /// "tokens" | "usd".
    pub basis: String,
    /// "daily" | "weekly" | "monthly".
    pub window: String,
    pub cap: f64,
    /// "soft" | "hard".
    pub mode: String,
}

/// Per-field precedence: the seat row's set fields override the org-wide
/// config defaults. Ungoverned when the resolved `cap <= 0` — that is the
/// default posture (bylaws rule 2: ungoverned is byte-identical legacy).
pub fn resolve_budget(
    seat: Option<&SeatBudget>,
    global: &BudgetSettings,
) -> Option<EffectiveBudget> {
    let basis = seat
        .and_then(|s| s.basis.clone())
        .unwrap_or_else(|| global.basis.clone());
    let window = seat
        .and_then(|s| s.window.clone())
        .unwrap_or_else(|| global.window.clone());
    let mode = seat
        .and_then(|s| s.mode.clone())
        .unwrap_or_else(|| global.mode.clone());
    let cap = seat.map(|s| s.cap).unwrap_or(global.cap);
    if cap <= 0.0 {
        return None;
    }
    Some(EffectiveBudget {
        basis,
        window,
        cap,
        mode,
    })
}

/// The UTC instant a window started, as RFC3339 (the same format the
/// session store compares against). Unknown windows fall back to `daily` —
/// fail-open on a typo'd window keeps the operator's turns running while
/// the doctor can flag the value.
pub fn window_start(window: &str) -> String {
    let now = chrono::Utc::now();
    let start = match window {
        "weekly" => {
            // Monday 00:00 UTC.
            let days_since_monday = now.date_naive().format("%u").to_string();
            let days_since_monday: i64 = days_since_monday.parse().unwrap_or(1) - 1;
            (now - chrono::Duration::days(days_since_monday))
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .map(|naive| {
                    naive
                        .and_local_timezone(chrono::Utc)
                        .single()
                        .unwrap_or(now)
                })
                .unwrap_or(now)
        }
        "monthly" => now
            .date_naive()
            .with_day(1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|naive| naive.and_local_timezone(chrono::Utc).single())
            .unwrap_or(now),
        _ => now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| naive.and_local_timezone(chrono::Utc).single())
            .unwrap_or(now),
    };
    start.to_rfc3339()
}

pub struct SeatBudgetDb {
    conn: Arc<Mutex<Connection>>,
}

impl SeatBudgetDb {
    pub fn init(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| Error::Agent(format!("Failed to open seat budget DB: {e}")))?;
        Self::from_connection(conn)
    }

    /// Open the seat-budget store where the ORG-FAMILY rows live.
    ///
    /// The org family's tables live in the SIBLING of the app database
    /// (`<dir>/operant_kanban.db`) — the seat policies, the context
    /// injection tables and the seat budgets all live there, and both
    /// enforcement paths (the gateway's turn gate and the cron scheduler's
    /// budget gate) read them from there. The app database
    /// (`<dir>/database.db`) is a DIFFERENT file: a store opened there is a
    /// store the enforcement never sees, so an override written into it is
    /// a silent no-op (RT-288).
    ///
    /// One-time migration: rows an earlier build wrote into the app
    /// database's `seat_budgets` table are copied across once (recorded in
    /// the sibling), so an operator's existing override is not orphaned by
    /// the fix.
    pub fn init_org_sibling(app_db_path: &Path) -> Result<Self> {
        // The org-sibling rule inlined (the same rule
        // `org::context_injection::ContextInjectionDb::open` follows).
        let org_sibling = app_db_path
            .parent()
            .map(|p| p.join("operant_kanban.db"))
            .unwrap_or_else(|| PathBuf::from("operant_kanban.db"));
        let db = Self::init(&org_sibling)?;
        db.migrate_from_app_db(app_db_path);
        Ok(db)
    }

    /// Copy the app database's `seat_budgets` rows into the sibling, once.
    ///
    /// Best-effort: a failure here is reported (warn) and never blocks the
    /// store — the enforcement must still run, and the operator's own
    /// `budget list` will show what is and is not there.
    fn migrate_from_app_db(&self, app_db_path: &Path) {
        const MIGRATION_KEY: &str = "seat_budgets_from_app_db";
        let conn = match self.lock() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "seat budget migration skipped — the store lock is poisoned");
                return;
            }
        };
        if let Err(e) = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS seat_budget_migrations (
                key TEXT PRIMARY KEY,
                applied_at TEXT NOT NULL
            );",
        ) {
            tracing::warn!(error = %e, "seat budget migration skipped — could not create the marker table");
            return;
        }
        let already = conn
            .query_row(
                "SELECT COUNT(*) FROM seat_budget_migrations WHERE key = ?1",
                params![MIGRATION_KEY],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        if already > 0 {
            return;
        }
        // The app database's table, if an earlier build ever created it.
        let app_conn = match Connection::open_with_flags(
            app_db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(_) => return,
        };
        let legacy: Vec<SeatBudget> = match app_conn
            .prepare(
                "SELECT employee_id, basis, window, cap, mode
                 FROM seat_budgets ORDER BY employee_id",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok(SeatBudget {
                        employee_id: row.get(0)?,
                        basis: row.get(1)?,
                        window: row.get(2)?,
                        cap: row.get(3)?,
                        mode: row.get(4)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()
            }) {
            Ok(rows) => rows,
            Err(_) => {
                // No legacy table — nothing to migrate. The marker write is
                // REPORTED, not swallowed: `let _ =` made a failed marker
                // silent, and the copy then re-ran at every boot against the
                // doc's own "best-effort: a failure here is reported (warn)"
                // contract (RT-340).
                if let Err(e) = conn.execute(
                    "INSERT OR REPLACE INTO seat_budget_migrations (key, applied_at) VALUES (?1, ?2)",
                    params![MIGRATION_KEY, chrono::Utc::now().to_rfc3339()],
                ) {
                    tracing::warn!(error = %e, "seat budget migration marker not written — the copy will repeat on the next boot");
                }
                return;
            }
        };
        for row in &legacy {
            if let Err(e) = conn.execute(
                "INSERT OR REPLACE INTO seat_budgets (employee_id, basis, window, cap, mode)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![row.employee_id, row.basis, row.window, row.cap, row.mode],
            ) {
                tracing::warn!(error = %e, seat = %row.employee_id, "seat budget migration could not copy a row");
            }
        }
        if let Err(e) = conn.execute(
            "INSERT OR REPLACE INTO seat_budget_migrations (key, applied_at) VALUES (?1, ?2)",
            params![MIGRATION_KEY, chrono::Utc::now().to_rfc3339()],
        ) {
            tracing::warn!(error = %e, "seat budget migration marker not written — the copy will repeat on the next boot");
        }
        tracing::info!(
            rows = legacy.len(),
            "migrated seat budget overrides from the app database to the org sibling"
        );
    }

    pub fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS seat_budgets (
                employee_id TEXT PRIMARY KEY,
                basis TEXT,
                window TEXT,
                cap REAL NOT NULL DEFAULT 0.0,
                mode TEXT
            );",
        )
        .map_err(|e| Error::Agent(format!("Failed to init seat budget DB: {e}")))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("seat budget DB lock poisoned".to_string()))
    }

    /// Upsert the override row for a seat (hrmaster's fine-tuning surface).
    pub fn upsert(&self, budget: &SeatBudget) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO seat_budgets (employee_id, basis, window, cap, mode)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(employee_id) DO UPDATE SET
                 basis = excluded.basis,
                 window = excluded.window,
                 cap = excluded.cap,
                 mode = excluded.mode",
            params![
                budget.employee_id,
                budget.basis,
                budget.window,
                budget.cap,
                budget.mode
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to upsert seat budget: {e}")))?;
        Ok(())
    }

    pub fn get(&self, employee_id: &str) -> Result<Option<SeatBudget>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT employee_id, basis, window, cap, mode
                 FROM seat_budgets WHERE employee_id = ?1",
                params![employee_id],
                |row| {
                    Ok(SeatBudget {
                        employee_id: row.get(0)?,
                        basis: row.get(1)?,
                        window: row.get(2)?,
                        cap: row.get(3)?,
                        mode: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|e| Error::Agent(format!("Failed to read seat budget: {e}")))?;
        Ok(row)
    }

    /// Remove a seat's override row entirely — the seat then inherits the
    /// `[bylaws].budget` default. Deleting a nonexistent row is a no-op.
    pub fn delete(&self, employee_id: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM seat_budgets WHERE employee_id = ?1",
            params![employee_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to delete seat budget: {e}")))?;
        Ok(())
    }

    /// Every override row, ordered by seat id — the `operant budget list`
    /// surface.
    pub fn list_all(&self) -> Result<Vec<SeatBudget>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT employee_id, basis, window, cap, mode
                 FROM seat_budgets ORDER BY employee_id",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare seat budget list: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SeatBudget {
                    employee_id: row.get(0)?,
                    basis: row.get(1)?,
                    window: row.get(2)?,
                    cap: row.get(3)?,
                    mode: row.get(4)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Failed to list seat budgets: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(
                row.map_err(|e| Error::Agent(format!("Failed to read seat budget row: {e}")))?,
            );
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    /// RT-340: the migration marker writes used to DISCARD their Result, silent
    /// against the doc's own "best-effort: a failure here is reported (warn)"
    /// contract, so a failed marker re-ran the copy at every boot with no
    /// trace. Source contract: the swallow shape must be gone, and the needle is
    /// built in pieces so this test's own prose cannot satisfy it.
    #[test]
    fn a_failed_migration_marker_is_reported_not_swallowed() {
        let src = include_str!("seat_budgets.rs");
        let swallow = format!("let _ = {}{}execute(", "_", "conn.");
        assert_eq!(
            src.matches(&swallow).count(),
            0,
            "the marker writes must report a failure, not swallow it"
        );
        let warning = format!("marker not written{}", "");
        assert!(
            src.matches(&warning).count() >= 2,
            "both marker writes must warn on failure (count: {})",
            src.matches(&warning).count()
        );
    }

    use super::*;

    fn conn() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    /// RT-288: the seat-budget store the ENFORCEMENT reads must be the same
    /// store the operator's command writes.
    ///
    /// `operant budget set` wrote into the app database
    /// (`<dir>/database.db`) while both enforcement paths read the org
    /// SIBLING (`<dir>/operant_kanban.db`) — so every per-seat override the
    /// operator set was a silent no-op, while the command printed "Budget
    /// override set for '<seat>'".
    #[test]
    fn the_org_sibling_store_sees_the_rows_the_app_db_store_wrote() {
        let dir = std::env::temp_dir().join(format!(
            "operant_seat_budget_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let app_db = dir.join("database.db");

        // What an earlier build's command wrote: the app database.
        let legacy = SeatBudgetDb::init(&app_db).expect("legacy store");
        legacy
            .upsert(&SeatBudget {
                employee_id: "dispatcher".to_string(),
                basis: None,
                window: None,
                cap: 50_000.0,
                mode: None,
            })
            .expect("legacy upsert");

        // What the enforcement mounts: the org sibling.
        let enforcement = SeatBudgetDb::init_org_sibling(&app_db).expect("the sibling store");
        assert!(
            dir.join("operant_kanban.db").exists(),
            "the enforcement store lives in the org sibling"
        );
        let row = enforcement
            .get("dispatcher")
            .expect("read the override")
            .expect("the override must have crossed over");
        assert_eq!(
            row.cap, 50_000.0,
            "the override must be the one enforcement sees"
        );

        // And the migration runs ONCE: a row written to the app database
        // after the migration must NOT come back (a cleared override must
        // stay cleared).
        legacy
            .upsert(&SeatBudget {
                employee_id: "latecomer".to_string(),
                basis: None,
                window: None,
                cap: 1.0,
                mode: None,
            })
            .expect("late upsert");
        let again = SeatBudgetDb::init_org_sibling(&app_db).expect("the sibling store again");
        assert!(
            again.get("latecomer").expect("read").is_none(),
            "the migration must not run twice"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_global_default_capped() {
        let global = BudgetSettings {
            basis: "tokens".into(),
            window: "daily".into(),
            cap: 100_000.0,
            mode: "hard".into(),
        };
        let effective = resolve_budget(None, &global).unwrap();
        assert_eq!(effective.basis, "tokens");
        assert_eq!(effective.window, "daily");
        assert_eq!(effective.cap, 100_000.0);
        assert_eq!(effective.mode, "hard");
    }

    // ── iter-670: the write-path fns behind `operant budget` ──

    fn budget(id: &str, cap: f64) -> SeatBudget {
        SeatBudget {
            employee_id: id.to_string(),
            basis: Some("tokens".to_string()),
            window: Some("daily".to_string()),
            cap,
            mode: Some("hard".to_string()),
        }
    }

    #[test]
    fn delete_removes_the_row_and_is_a_noop_for_a_missing_one() {
        let db = SeatBudgetDb::from_connection(conn()).unwrap();
        db.upsert(&budget("dispatcher", 50_000.0)).unwrap();
        assert_eq!(db.list_all().unwrap().len(), 1, "upserted row must list");
        db.delete("dispatcher").unwrap();
        assert!(
            db.get("dispatcher").unwrap().is_none(),
            "deleted row must not resolve"
        );
        db.delete("dispatcher")
            .expect("deleting a missing row must be a no-op, not an error");
    }

    #[test]
    fn list_all_orders_by_seat_id() {
        let db = SeatBudgetDb::from_connection(conn()).unwrap();
        assert!(db.list_all().unwrap().is_empty());
        db.upsert(&budget("premiere", 10.0)).unwrap();
        db.upsert(&budget("dispatcher", 5.0)).unwrap();
        let rows = db.list_all().unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.employee_id.as_str())
                .collect::<Vec<_>>(),
            vec!["dispatcher", "premiere"],
            "ordered by seat id, cap material preserved"
        );
        assert_eq!(rows[1].cap, 10.0);
    }

    #[test]
    fn resolve_global_default_ungoverned() {
        // cap = 0 (the shipped default) = no budget consulted at all.
        let global = BudgetSettings::default();
        assert!(resolve_budget(None, &global).is_none());
    }

    #[test]
    fn resolve_seat_overrides_per_field() {
        let global = BudgetSettings {
            basis: "tokens".into(),
            window: "daily".into(),
            cap: 100_000.0,
            mode: "soft".into(),
        };
        // Seat pins cap + mode, inherits basis/window.
        let seat = SeatBudget {
            employee_id: "dp-the-program".into(),
            basis: None,
            window: Some("monthly".into()),
            cap: 5.0,
            mode: Some("hard".into()),
        };
        let effective = resolve_budget(Some(&seat), &global).unwrap();
        assert_eq!(effective.basis, "tokens");
        assert_eq!(effective.window, "monthly");
        assert_eq!(effective.cap, 5.0);
        assert_eq!(effective.mode, "hard");
    }

    #[test]
    fn resolve_seat_zero_cap_is_explicit_ungoverned() {
        // The owner's release valve: a row with cap = 0 means "this seat
        // runs free even though the org default caps".
        let global = BudgetSettings {
            basis: "tokens".into(),
            window: "daily".into(),
            cap: 100_000.0,
            mode: "hard".into(),
        };
        let seat = SeatBudget {
            employee_id: "premiere".into(),
            basis: None,
            window: None,
            cap: 0.0,
            mode: None,
        };
        assert!(resolve_budget(Some(&seat), &global).is_none());
    }

    #[test]
    fn resolve_all_null_zero_cap_row_is_ungoverned() {
        // A row that exists with cap = 0 is the release valve — "run free" —
        // NOT a fall-through to the capped org default.
        let global = BudgetSettings {
            basis: "tokens".into(),
            window: "daily".into(),
            cap: 50.0,
            mode: "soft".into(),
        };
        let seat = SeatBudget::default();
        assert!(resolve_budget(Some(&seat), &global).is_none());
    }

    #[test]
    fn store_roundtrip() {
        let db = SeatBudgetDb::from_connection(conn()).unwrap();
        assert!(db.get("hrmaster").unwrap().is_none());
        db.upsert(&SeatBudget {
            employee_id: "hrmaster".into(),
            basis: Some("usd".into()),
            window: Some("weekly".into()),
            cap: 1.5,
            mode: Some("hard".into()),
        })
        .unwrap();
        let got = db.get("hrmaster").unwrap().unwrap();
        assert_eq!(got.basis.as_deref(), Some("usd"));
        assert_eq!(got.window.as_deref(), Some("weekly"));
        assert_eq!(got.cap, 1.5);
        assert_eq!(got.mode.as_deref(), Some("hard"));
    }

    #[test]
    fn window_starts_are_utc_aligned() {
        let daily = window_start("daily");
        assert!(
            daily.ends_with("00:00:00+00:00"),
            "daily = today 00:00 UTC: {daily}"
        );
        let weekly = window_start("weekly");
        assert!(
            weekly.ends_with("00:00:00+00:00"),
            "weekly = Monday 00:00 UTC: {weekly}"
        );
        let monthly = window_start("monthly");
        assert!(
            monthly.contains("-01T00:00:00"),
            "monthly = 1st 00:00 UTC: {monthly}"
        );
        // A typo'd window fails OPEN to daily — turns keep running; the
        // doctor can flag the value. A silent refusal would be worse.
        let typo = window_start("weakly");
        assert!(typo.ends_with("00:00:00+00:00"));
    }
}

mod rt376_tests {
    /// RT-376: the seat-budgets doc claimed mid-flight enforcement is "a
    /// recorded follow-up, not this contract" — but the agent stops a
    /// token-basis turn mid-flight once the hard cap is crossed (run.rs), so
    /// the claim is half-stale. The doc must name the shipped path and what
    /// remains a follow-up. Source contract, needle concatenation-built.
    #[test]
    fn the_budget_doc_names_what_mid_flight_enforcement_really_does() {
        let src = include_str!("seat_budgets.rs");
        let stale = format!("Nothing here reaches into the agent loop {}mid-flight", "");
        assert!(
            !src.contains(&stale),
            "the doc must not claim mid-flight enforcement does not exist"
        );
        let shipped = format!("the follow-up has since {}shipped", "");
        assert!(
            src.contains(&shipped),
            "the doc must say the mid-flight follow-up has shipped"
        );
        let crossing = format!("mid-flight once the hard cap is {}crossed", "");
        assert!(
            src.contains(&crossing),
            "the doc must name when the shipped mid-flight stop fires"
        );
        let remains = format!("follow-up is the usd-basis per-call {}catalog cost", "");
        assert!(
            src.contains(&remains),
            "the doc must say what is still a follow-up (the usd per-call cost)"
        );
        // The operator-facing config doc carries the same contract.
        let config_doc = include_str!("../config.rs");
        let stale_config = format!(
            "Mid-flight enforcement is\n/// a recorded follow-up, not this {}contract",
            ""
        );
        assert!(
            !config_doc.contains(&stale_config),
            "the bylaws doc must not repeat the stale follow-up-only claim"
        );
        let corrected = format!("half-{}historical", "");
        assert!(
            config_doc.contains(&corrected),
            "the bylaws doc must name the correction it carries"
        );
    }
}
