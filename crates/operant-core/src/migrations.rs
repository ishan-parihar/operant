//! Plan 004 — append-only SQLite migration runner.
//!
//! The runner reads `PRAGMA user_version` (the canonical SQLite
//! version PRAGMA), then walks the supplied `MIGRATIONS` array
//! applying any entries whose index >= current version. Each
//! migration runs in its own transaction; on error the version
//! stays at the last successful index and the error is returned.
//!
//! ## Contract
//!
//! - `MIGRATIONS` is append-only. Never edit a past entry; add a
//!   new entry. The runner fingerprints each entry with its index,
//!   so an in-place edit would either be re-run (if version
//!   unchanged) or skipped (if version already past) — neither is
//!   what an edit would want.
//! - Each entry MUST be a self-contained SQL block. It can include
//!   `CREATE TABLE` / `CREATE INDEX` / `ALTER TABLE` / etc. — the
//!   runner uses `execute_batch` under a single transaction.
//! - Each entry SHOULD be idempotent at the SQL level (`IF NOT
//!   EXISTS` / `IF EXISTS` / `COLUMNFROMSCHEMA`-style guards). The
//!   runner itself is idempotent (re-runs are no-ops) but the user
//!   can still call `migrate()` after a crash and expect the world
//!   to be sane.
//!
//! ## What lives here
//!
//! Just the runner. The per-DB `MIGRATIONS` arrays live next to the
//! DB they govern (e.g. `database.rs`, `cronjobs/db.rs`,
//! `kanban/db.rs`); this module is the engine that consumes them.

use rusqlite::Connection;

/// Migrate the database to the latest version represented by
/// `migrations`. Each `migrations[i]` is a SQL block applied in
/// version order. Failures abort the entire migration and return
/// the error without bumping the user_version pragma.
///
/// `db_name` is used only for log lines; the runner doesn't key
/// state by name (each DB is its own `user_version`).
pub fn migrate(
    conn: &Connection,
    db_name: &str,
    migrations: &[&str],
) -> anyhow::Result<()> {
    let current = current_version(conn)?;
    let target = migrations.len() as i64;

    if current == target {
        tracing::debug!(db = %db_name, version = current, "migrate: already at target");
        return Ok(());
    }

    if current > target {
        anyhow::bail!(
            "schema for {db_name} is at version {current} but only {target} migrations are \
             declared (refusing to downgrade or run an old migration against a newer DB)"
        );
    }

    for (idx, sql) in migrations.iter().enumerate().skip(current as usize) {
        let version = (idx as i64) + 1;
        tracing::info!(db = %db_name, from = idx, to = version, "migrate: applying entry");
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql).map_err(|e| {
            anyhow::anyhow!("{db_name} migration v{version} failed: {e}")
        })?;
        // Bump the pragma inside the same transaction so a failed
        // migration leaves the world at the previous version.
        tx.pragma_update(None, "user_version", version)
            .map_err(|e| anyhow::anyhow!("{db_name} migration v{version} user_version bump failed: {e}"))?;
        tx.commit()?;
        tracing::info!(db = %db_name, version, "migrate: entry applied");
    }

    Ok(())
}

/// Read the current `PRAGMA user_version`. Returns 0 on a fresh DB
/// (the PRAGMA defaults to 0 on a never-versioned SQLite file).
pub fn current_version(conn: &Connection) -> anyhow::Result<i64> {
    conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|e| anyhow::anyhow!("failed to read user_version: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh() -> Connection {
        Connection::open_in_memory().expect("in-memory SQLite")
    }

    #[test]
    fn migrate_fresh_db_reaches_target_and_creates_tables() {
        let conn = fresh();
        migrate(
            &conn,
            "test",
            &[
                "CREATE TABLE foo (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
                "ALTER TABLE foo ADD COLUMN created_at TEXT;",
            ],
        )
        .expect("migrate ok");
        assert_eq!(current_version(&conn).unwrap(), 2);
        // The v2 entry actually ran.
        let stmt = conn
            .prepare("SELECT name, created_at FROM foo")
            .expect("select ok");
        let cols: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(|c| c.to_string())
            .collect();
        assert!(
            cols.contains(&"created_at".to_string()),
            "v2 ALTER added the column, got {cols:?}"
        );
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = fresh();
        let migs = &["CREATE TABLE bar (x INTEGER);"];
        migrate(&conn, "test", migs).unwrap();
        // Second call: same target, no-op.
        migrate(&conn, "test", migs).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 1);
    }

    #[test]
    fn migrate_appends_to_an_existing_db() {
        let conn = fresh();
        // Land at v1.
        migrate(
            &conn,
            "test",
            &["CREATE TABLE baz (id INTEGER PRIMARY KEY);"],
        )
        .unwrap();
        assert_eq!(current_version(&conn).unwrap(), 1);

        // Append a v2 — the runner should apply v2 only.
        migrate(
            &conn,
            "test",
            &[
                "CREATE TABLE baz (id INTEGER PRIMARY KEY);",
                "ALTER TABLE baz ADD COLUMN note TEXT;",
            ],
        )
        .unwrap();
        assert_eq!(current_version(&conn).unwrap(), 2);

        let cols: Vec<String> = conn
            .prepare("SELECT * FROM baz")
            .unwrap()
            .column_names()
            .into_iter()
            .map(|c| c.to_string())
            .collect();
        assert!(cols.contains(&"note".to_string()));
    }

    #[test]
    fn migrate_failure_aborts_and_keeps_version() {
        let conn = fresh();
        let migs = &["CREATE TABLE qux (id INTEGER PRIMARY KEY);", "NOT VALID SQL;"];
        let err = migrate(&conn, "test", migs).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("v2") && msg.contains("failed"),
            "error should reference the failing v2 entry, got: {msg}"
        );
        // v1 ran successfully; the v2 failure did not bump past v1.
        assert_eq!(current_version(&conn).unwrap(), 1);
    }

    #[test]
    fn migrate_rejects_downgrade() {
        let conn = fresh();
        // Hand-craft a DB at version 5 with no migration array.
        conn.pragma_update(None, "user_version", 5).unwrap();
        let err = migrate(&conn, "test", &["CREATE TABLE a (x);"]).unwrap_err();
        assert!(
            format!("{err}").contains("refusing to downgrade"),
            "downgrade error should explain the refusal, got: {err}"
        );
    }
}
