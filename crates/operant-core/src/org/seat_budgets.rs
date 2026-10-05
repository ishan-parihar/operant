//! `seat_budgets` — per-employee budget overrides (Wave 4,
//! ORGANISM-ARCHITECTURE §5).
//!
//! One row per seat. Fields are `NULL`-able so an override can pin just one
//! knob (say, the cap) and inherit the rest from `[genome].budget` —
//! precedence is per-field, resolved by [`resolve_budget`]. `cap` is NOT
//! nullable: an override row exists to cap, so a row with `cap = 0` means
//! "explicitly ungoverned" (an intentional release valve — the genome's
//! fail-open rule applies to budgets too, by the owner's "totally free or
//! ungoverned" directive).
//!
//! Enforcement is TURN-BOUNDARY v1: the gateway turn computes remaining
//! budget from the persistent session accumulator
//! ([`crate::gateway_session::PersistentSessionStore::employee_window_usage`])
//! and either refuses (hard) or injects the remainder into the turn (soft).
//! Nothing here reaches into the agent loop mid-flight — that is a recorded
//! follow-up, not this contract.

use crate::config::BudgetSettings;
use crate::error::{Error, Result};
use chrono::Datelike;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The override row. `None` fields inherit `[genome].budget`.
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
    // → `[genome].budget`.
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
/// default posture (genome rule 2: ungoverned is byte-identical legacy).
pub fn resolve_budget(
    seat: Option<&SeatBudget>,
    global: &BudgetSettings,
) -> Option<EffectiveBudget> {
    let seat = seat;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        Connection::open_in_memory().unwrap()
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
