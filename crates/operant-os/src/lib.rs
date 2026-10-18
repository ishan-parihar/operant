//! Operant as Holon's runtime — the org side of `docs/agentic-os/`. The
//! interface contract is Holon's public `docs/CONTRACT.md` (§§1, 8–12);
//! the boundary record itself lives in the private `holon-personal`
//! overlay (docs/design/2026-10-14-holon-operant-boundary.md) — cite the
//! CONTRACT for interfaces and the overlay for the record (audit O-29).
//!
//! Phase B (work package H2's consumer): the read API. Holon publishes SQL views
//! (`api_*`, `api_version`) over its `work.db`; this crate is the ONLY sanctioned
//! reader surface on operant's side:
//!
//! - the database is opened with SQLite `mode=ro` (invariant 1: one writer, Holon;
//!   operant never writes a Holon store) — the connection string cannot express a
//!   writable handle, so the invariant is structural, not behavioral;
//! - the client refuses an `api_version` it does not know (boundary doc §4.1);
//! - every query names an `api_*` view — `read_only_view` rejects any other table
//!   name, so a future refactor cannot silently reach under the published API.
//!
//! Later phases land here: `policy` (phase C), trajectory + usage capture (phase D),
//! `supervisor`/`runner`/`broker` (phase E), `chair`/`events` (phase F).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod api;
pub mod archive;
pub mod chair;
pub mod contract;
pub mod enforcement;
pub mod events;
pub mod organs;
pub mod policy;
pub mod run;
pub mod run_env;
pub mod service_index;
pub mod supervisor;
pub mod tools;

/// Read-API versions this build understands. v2 (holon 0010_event_triggers)
/// keeps every view shape of v1 — api_triggers may simply carry event rows now
/// (the `pattern` column holds the event pattern where a cron held the
/// schedule); the client reads both identically. A Holon database publishing a
/// version outside this set is refused (the caller surfaces a doctor line and
/// org mode stays off — a newer Holon must not be read through stale view shapes).
/// v3 (the hardening wave) adds the guards/genome-era views without
/// changing any v1/v2 shape — the client reads all three identically.
/// v4 (the capability-layer audit §2, iter-885) is accepted BEFORE the kernel
/// publishes it: the C1/R-04 migration (surface units, `api_pool`, the
/// service columns) adds views without changing any v1–v3 shape, and the
/// cross-repo order is operant-knows-4 FIRST, kernel-migrates SECOND — a
/// kernel that bumps to v4 against a runtime that does not know it turns
/// org mode off on the live box.
pub const KNOWN_API_VERSIONS: &[&str] = &["1", "2", "3", "4"];

/// Errors surfaced by the org-runtime seam. Degrade, don't die: every consumer
/// turns these into a doctor line or a helpful error, never a dead turn.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("holon home is not configured — set [holon] home in operant.toml")]
    NoHome,
    #[error("holon work database not found at {0} — is the instance initialized?")]
    WorkDbMissing(std::path::PathBuf),
    #[error(
        "holon read API version {published:?} is not known to this operant build (knows {known:?}) — update operant or point [holon] home at an older instance"
    )]
    UnknownApiVersion {
        published: String,
        known: &'static [&'static str],
    },
    #[error(
        "read API not published at {path} ({source}) — the instance predates migration 0008; update holon"
    )]
    ApiNotPublished {
        path: std::path::PathBuf,
        source: rusqlite::Error,
    },
    #[error("holon read failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("policy.json missing at {0} — a Holon run always claims before it runs")]
    PolicyMissing(std::path::PathBuf),
    #[error("policy.json is not the shape Holon writes: {0}")]
    PolicyParse(#[source] serde_json::Error),
    #[error("policy.json version {0} is not known to this build (knows 1)")]
    PolicyVersion(u32),
    #[error("policy.json posture {0:?} is neither observe nor act")]
    PolicyPosture(String),
    #[error("holon enforcement could not mount: {0}")]
    Enforcement(String),
    #[error("supervisor protocol: {0}")]
    Protocol(String),
    #[error("org archive: {0}")]
    Archive(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
