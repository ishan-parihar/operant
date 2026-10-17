//! O-24 (phase G, W7): the org-store archive is WAL-safe and complete —
//! the regression test iter-851's commit body described but never shipped.
//! A row committed ONLY to the live `-wal` (a connection held open, the
//! transaction committed, no checkpoint) must be IN the snapshot; the copy
//! must pass `integrity_check`; the org/ charter tree and a per-board
//! kanban file must ride along; the manifest must name everything.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use operant_os::archive::archive_org_stores;

#[test]
fn a_wal_committed_row_lands_in_the_snapshot_intact() {
    let home = tempfile::tempdir().expect("tempdir");
    let h = home.path();

    // a live sqlite store in WAL mode with a committed-but-uncheckpointed row
    let db_path = h.join("database.db");
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open");
        conn.pragma_update(None, "journal_mode", "WAL")
            .expect("wal mode");
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);")
            .expect("schema");
        conn.execute("INSERT INTO t (v) VALUES ('committed-to-wal-only')", [])
            .expect("insert");
        // committed, no checkpoint — the -wal holds the only copy of the row;
        // the connection stays OPEN so no close-time checkpoint can fire
        let uncheckpointed = std::fs::read(h.join("database.db-wal")).expect("a -wal file exists");
        assert!(
            !uncheckpointed.is_empty(),
            "the fixture must actually hold pages in the -wal"
        );
        std::mem::forget(conn); // leak the handle: the -wal stays hot
    }

    // a second store, a plain file, to prove non-sqlite copies still work
    std::fs::write(h.join("operant.db"), b"not sqlite at all").expect("plain store");

    // the org/ charter tree (non-regenerable) and a per-board kanban
    std::fs::create_dir_all(h.join("org").join("employees").join("emp_1")).expect("org tree");
    std::fs::write(
        h.join("org")
            .join("employees")
            .join("emp_1")
            .join("MEMORY.md"),
        "# charter — non-regenerable",
    )
    .expect("charter");
    std::fs::write(h.join("operant_kanban_sideboard.db"), b"board bytes").expect("side board");

    let dest = archive_org_stores(h, "2099-12-31").expect("archive");
    assert!(dest.is_dir(), "archive dir: {}", dest.display());

    // the WAL-committed row IS in the snapshot
    let snap = rusqlite::Connection::open(dest.join("database.db")).expect("open snapshot");
    let v: String = snap
        .query_row("SELECT v FROM t LIMIT 1", [], |r| r.get(0))
        .expect("the row landed");
    assert_eq!(v, "committed-to-wal-only");
    let check: String = snap
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity_check");
    assert_eq!(check, "ok", "the snapshot is a sound database");

    // the plain store, the charter, the side board, the manifest
    assert_eq!(
        std::fs::read(dest.join("operant.db")).expect("plain copy"),
        b"not sqlite at all"
    );
    assert_eq!(
        std::fs::read(
            dest.join("org")
                .join("employees")
                .join("emp_1")
                .join("MEMORY.md")
        )
        .expect("charter copy"),
        std::fs::read(
            h.join("org")
                .join("employees")
                .join("emp_1")
                .join("MEMORY.md")
        )
        .expect("charter original")
    );
    assert!(
        dest.join("operant_kanban_sideboard.db").exists(),
        "the side-board db rides"
    );
    let manifest = std::fs::read_to_string(dest.join("MANIFEST.sha256")).expect("manifest");
    assert!(
        manifest.contains("database.db"),
        "the store is digested: {manifest}"
    );
    assert!(
        manifest.contains("org/ tree copied"),
        "the org tree is named: {manifest}"
    );
    assert!(
        manifest.contains("operant_kanban_sideboard.db"),
        "the side board is named: {manifest}"
    );
    assert!(
        manifest.contains("operant_cron.db: MISSING"),
        "missing stores are listed as missing, not skipped silently: {manifest}"
    );

    // one snapshot per date: a second archive on the same date refuses
    assert!(archive_org_stores(h, "2099-12-31").is_err());
}
