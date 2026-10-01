//! Idempotent schema reconciliation for the org layer.
//!
//! ## Why this module exists
//!
//! The org tables (`employees`, `notices`, `departments`, `assignments`,
//! `authority_grants`, `dm_threads`, `artifacts`) live in the **shared**
//! `operant_kanban.db`, which sits at `PRAGMA user_version = 1` because the
//! kanban family claims that counter. `PRAGMA user_version` is file-wide, so
//! the org layer cannot use `crate::migrations::migrate` for its own
//! evolution: appending an entry would move the board's version and make
//! kanban's one-entry family *look* downgraded the next time kanban opens the
//! same file, which is exactly the "refusing to downgrade" hard-fail that
//! guard exists to prevent (documented in `migrations.rs` after R39-7).
//!
//! [`docs/ORG-AUTHORITY-ARCHITECTURE.md`](../../../docs/ORG-AUTHORITY-ARCHITECTURE.md)
//! §12 resolves this the same way: **never use `user_version` for the org
//! layer.** New tables are `CREATE TABLE IF NOT EXISTS`. New columns go
//! through [`ensure_column`], which keys on `PRAGMA table_info` and is
//! idempotent by construction — safe to run on every open, no version marker
//! needed.
//!
//! ## What this guarantees
//!
//! - [`ensure_column`] is a no-op when the column exists, and applies exactly
//!   one `ALTER TABLE ... ADD COLUMN` when it does not.
//! - It never rewrites, drops, or reorders existing data.
//! - It refuses a column definition that is not purely additive, so a future
//!   caller cannot smuggle a `NOT NULL` without a default past a populated
//!   table.
//!
//! ## Deliberately not here
//!
//! - No down-migrations. An org column removal is a data decision, not a
//!   mechanical one; nothing in the design needs it.
//! - No version marker. If one is ever needed, `org_schema_migrations` is the
//!   table to add — keyed by `(table_name, column_name)` rather than a
//!   file-wide counter, so it cannot collide with kanban's claim.

use rusqlite::Connection;

/// What [`ensure_column`] actually did, for the caller to log or assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaReport {
    /// Columns added by this call.
    pub added: Vec<String>,
    /// Columns already present, left untouched.
    pub present: Vec<String>,
}

impl SchemaReport {
    /// True when nothing had to be added.
    pub fn is_noop(&self) -> bool {
        self.added.is_empty()
    }
}

/// Ensure `table.column` exists with `definition`, adding it if absent.
///
/// `definition` is the **column type and constraints only** (e.g.
/// `"TEXT"`, `"TEXT NOT NULL DEFAULT '{}'"`) — not the `ADD COLUMN` keyword
/// and not the column name. It is interpolated into the DDL, so it must be a
/// compile-time literal from a caller in this crate, never user input.
///
/// # Errors
///
/// Returns an error if the table is absent (this module never creates tables
/// it was not asked to), if the `definition` is not additive-safe, or if
/// SQLite rejects the `ALTER TABLE`.
///
/// # Idempotence
///
/// Keyed on `PRAGMA table_info`, so calling it on every open costs one query
/// per column and mutates nothing when the column is present.
pub fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<SchemaReport, String> {
    if !is_additive_safe(definition) {
        return Err(format!(
            "org schema: refusing non-additive definition for {table}.{column}: {definition}"
        ));
    }

    let existing = table_columns(conn, table)?;
    if existing.is_empty() {
        return Err(format!(
            "org schema: table {table} does not exist (ensure the base table before adding columns)"
        ));
    }

    if existing.iter().any(|c| c == column) {
        return Ok(SchemaReport {
            added: vec![],
            present: vec![column.to_string()],
        });
    }

    // `table` and `column` reach here only as compile-time literals from
    // in-crate callers, and `definition` was just checked. SQLite cannot bind
    // identifiers in DDL, so the interpolation is unavoidable; the
    // additive-safety check above is what makes it safe.
    conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    ))
    .map_err(|e| format!("org schema: ALTER {table}.ADD {column}: {e}"))?;

    Ok(SchemaReport {
        added: vec![column.to_string()],
        present: vec![],
    })
}

/// Ensure several columns on one table, in order, reporting all of them.
///
/// Ordering matters when a later definition depends on an earlier one (it
/// cannot, for SQLite, but the report is easier to read when the order is
/// explicit).
///
/// # Errors
///
/// Propagates the first [`ensure_column`] error, having already applied
/// earlier columns. The operation is idempotent, so a retry completes the
/// remainder.
pub fn ensure_columns(
    conn: &Connection,
    table: &str,
    columns: &[(&str, &str)],
) -> Result<SchemaReport, String> {
    let mut added = Vec::new();
    let mut present = Vec::new();
    for (column, definition) in columns {
        let report = ensure_column(conn, table, column, definition)?;
        added.extend(report.added);
        present.extend(report.present);
    }
    Ok(SchemaReport { added, present })
}

/// Column names on `table`, or empty if the table does not exist.
///
/// # Errors
///
/// Propagates a `PRAGMA table_info` failure other than "no such table".
pub fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    // `PRAGMA table_info` does not accept a bound parameter, and `table` is
    // an in-crate literal at every call site.
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("org schema: prepare PRAGMA table_info({table}): {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("org schema: query PRAGMA table_info({table}): {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("org schema: read column name from {table}: {e}"))?);
    }
    Ok(out)
}

/// True when `definition` can be added to a table that already has rows.
///
/// SQLite rejects `ADD COLUMN NOT NULL` without a default on a populated
/// table. Rather than let that surface as a runtime failure at some later
/// open, reject it here where the caller sees a named error.
///
/// Also rejects the primary-key spellings, which SQLite refuses to add to an
/// existing table at all.
fn is_additive_safe(definition: &str) -> bool {
    let upper = definition.to_ascii_uppercase();
    if upper.contains("PRIMARY KEY") {
        return false;
    }
    let requires_default = upper.contains("NOT NULL");
    if requires_default && !upper.contains("DEFAULT") {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory");
        conn.execute_batch(
            "CREATE TABLE employees (
                 employee_id TEXT PRIMARY KEY,
                 department   TEXT
             )",
        )
        .expect("base table");
        conn
    }

    #[test]
    fn adds_absent_column_and_reports_it() {
        let conn = conn();
        let report = ensure_column(&conn, "employees", "reports_to", "TEXT").expect("add");
        assert_eq!(report.added, vec!["reports_to".to_string()]);
        assert!(report.present.is_empty());
        assert!(!report.is_noop());
        assert!(
            table_columns(&conn, "employees")
                .expect("cols")
                .contains(&"reports_to".to_string())
        );
    }

    #[test]
    fn existing_column_is_a_noop() {
        let conn = conn();
        ensure_column(&conn, "employees", "department", "TEXT").expect("first");
        let report = ensure_column(&conn, "employees", "department", "TEXT").expect("second");
        assert!(report.is_noop());
        assert!(report.added.is_empty());
        assert_eq!(report.present, vec!["department".to_string()]);
    }

    /// The core property: running the same reconciliation on every open must
    /// not fail, and must not duplicate the column.
    #[test]
    fn repeated_runs_are_idempotent() {
        let conn = conn();
        for _ in 0..5 {
            ensure_column(&conn, "employees", "peers", "TEXT NOT NULL DEFAULT '[]'")
                .expect("idempotent add");
        }
        let cols = table_columns(&conn, "employees").expect("cols");
        assert_eq!(
            cols.iter().filter(|c| *c == "peers").count(),
            1,
            "column added more than once"
        );
    }

    #[test]
    fn preserves_existing_rows() {
        let conn = conn();
        conn.execute(
            "INSERT INTO employees (employee_id, department) VALUES ('emp-1', 'infra')",
            [],
        )
        .expect("seed");
        ensure_column(&conn, "employees", "reports_to", "TEXT").expect("add");
        let dept: Option<String> = conn
            .query_row(
                "SELECT department FROM employees WHERE employee_id = 'emp-1'",
                [],
                |r| r.get(0),
            )
            .expect("read back");
        assert_eq!(dept.as_deref(), Some("infra"), "existing data must survive");
    }

    #[test]
    fn rejects_non_additive_definition() {
        let conn = conn();
        // NOT NULL with no DEFAULT cannot be added to a populated table.
        let err = ensure_column(&conn, "employees", "reports_to", "TEXT NOT NULL")
            .expect_err("must refuse");
        assert!(err.contains("non-additive"), "unhelpful error: {err}");
    }

    #[test]
    fn rejects_primary_key_addition() {
        let conn = conn();
        let err = ensure_column(&conn, "employees", "reports_to", "TEXT PRIMARY KEY")
            .expect_err("must refuse");
        assert!(err.contains("non-additive"), "unhelpful error: {err}");
    }

    #[test]
    fn missing_table_is_a_named_error_not_a_panic() {
        let conn = conn();
        let err = ensure_column(&conn, "departments", "mandate", "TEXT").expect_err("must fail");
        assert!(err.contains("does not exist"), "unhelpful error: {err}");
    }

    #[test]
    fn ensure_columns_applies_all_in_order() {
        let conn = conn();
        let report = ensure_columns(
            &conn,
            "employees",
            &[
                ("reports_to", "TEXT"),
                ("peers", "TEXT NOT NULL DEFAULT '[]'"),
            ],
        )
        .expect("both");
        assert_eq!(report.added.len(), 2);
        let cols = table_columns(&conn, "employees").expect("cols");
        assert!(cols.contains(&"reports_to".to_string()));
        assert!(cols.contains(&"peers".to_string()));
    }

    #[test]
    fn not_null_with_default_is_allowed() {
        let conn = conn();
        conn.execute("INSERT INTO employees (employee_id) VALUES ('emp-1')", [])
            .expect("seed");
        ensure_column(&conn, "employees", "peers", "TEXT NOT NULL DEFAULT '[]'")
            .expect("must be allowed on a populated table");
        let peers: String = conn
            .query_row(
                "SELECT peers FROM employees WHERE employee_id = 'emp-1'",
                [],
                |r| r.get(0),
            )
            .expect("read back");
        assert_eq!(peers, "[]", "default must be applied to existing rows");
    }

    #[test]
    fn table_columns_on_absent_table_is_empty_not_error() {
        let conn = conn();
        assert!(table_columns(&conn, "nope").expect("query").is_empty());
    }
}
