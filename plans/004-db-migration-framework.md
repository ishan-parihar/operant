# 004 — SQLite migration framework (`PRAGMA user_version`)

Stamped: `d394c136`. Priority: **P0**.

## Why

The three SQLite DBs (`state.db`/sessions, cron DB, kanban DB) have **no versioned
schema**: `CREATE TABLE IF NOT EXISTS` only, no `PRAGMA user_version`. Any shipped
schema change on an existing install is an unmanaged upgrade — a real data-loss risk
once operant is deployed outside dev.

## Files in scope

- `crates/operant-core/src/database.rs` (sessions DB open + schema)
- `crates/operant-core/src/cronjobs/db.rs` (cron DB open + schema)
- `crates/operant-core/src/kanban/db.rs` (kanban DB open + schema)
- New module `crates/operant-core/src/migrations.rs` (shared runner) + `lib.rs`/`mod.rs`
  registration

## Files out of scope

- MEMORY.md / skills-dir migrations (file-based, handled elsewhere — `background_review.rs`,
  `profile.rs`, `write_origin.rs` mention "migration" for file formats; leave those).
- Any schema *change* beyond adding the framework + stamping current version.

## Current state (evidence)

- No `user_version` reference anywhere in `crates/operant-core/src/` (grep verified).
- DB opens use `CREATE TABLE IF NOT EXISTS ...` (see `database.rs` open/schema section).
- `state.db` is created lazily ("will be created on first session" per `operant doctor`).

## Steps

1. Add `crates/operant-core/src/migrations.rs`:
   ```rust
   pub fn migrate(conn: &rusqlite::Connection, db_name: &str, migrations: &[&str]) -> anyhow::Result<()>
   ```
   Behavior: read `PRAGMA user_version`; for each index `i >= current`, execute
   `migrations[i]` inside a transaction, then `PRAGMA user_version = i + 1`. Fail loudly
   (return Err) on any failed step — never silently skip.
2. Stamp the current schema as version 1: in each of the three DBs, define
   `const MIGRATIONS: &[&str] = &[/* the existing CREATE TABLE IF NOT EXISTS block verbatim */];`
   and call `migrate(conn, "sessions", &MIGRATIONS)` right after open (before any other
   statement). Future schema changes append new entries (never edit old ones).
3. Wire into the three open paths (`database.rs`, `cronjobs/db.rs`, `kanban/db.rs`).
4. Add a `docs/migrations.md` convention note (or fold into `AGENTS.md`): "schema
   changes go through migrations.rs; never edit an existing migration entry."

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-core --all-targets -- -D warnings
cargo test -p operant-core --lib migration
cargo test --workspace --all-features --lib          # final gate
```
Existing DBs still open and work (doctor/session flows unaffected).

## Test plan

- `test_migrate_fresh_db`: open temp DB, migrate, assert `PRAGMA user_version == 1` and
  tables exist.
- `test_migrate_idempotent`: migrate twice → version stays 1, no error.
- `test_migrate_applies_append_only`: `MIGRATIONS` v2 appended → fresh DB ends at 2;
  an existing v1 DB upgrades to 2 and the v2 statement (e.g. `ALTER TABLE ... ADD COLUMN`)
  is applied.
- `test_migrate_failure_aborts`: a migration entry with invalid SQL → Err, version
  unchanged.

## Maintenance note

- Every `CREATE TABLE`/`ALTER TABLE` change must go through this runner from now on.
- The lazy-creation note in `operant doctor` ("state.db not created yet") stays valid.

## Escape hatches

- If any existing DB in the wild has drifted schema (columns missing that the code
  assumes), the migration should detect and report — do not silently `ALTER TABLE ADD`
  into a broken DB. Flag in `BUGS.md` instead.
