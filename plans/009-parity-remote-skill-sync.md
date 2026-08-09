# 009 — Parity: skills remote sync (hermes `tools/skills_sync.py` + `skills_sync_client.py`)

Stamped: `d394c136`. Priority: **P1** (core — user-designated "remote-sync").

## Why

Hermes ships two sync layers operant lacks:
1. **Local manifest sync** (`skills_sync.py`, 1410 LOC): sync bundled/optional skills
   into the user skills dir, tracked by a manifest of origin hashes; never re-seeds
   curator-pruned skills; suppresses externally-delegated skills; fsync'd atomic writes.
2. **Cloud sync client** (`skills_sync_client.py`, 2187 LOC): push/pull of skills to a
   remote plane (object store + CAS ref, three-way merge), **inert unless signed in**
   (`SyncInertError`), wired as a debounced push in `skill_manage` + periodic
   `maybe_pull_skills` at curator ticks + a `sync status|pull|push|now` CLI.

Operant's `crates/operant-runtime/src/skills/mod.rs:34` already carries a
`// TODO: update to operant-labs repo when the registry is rebranded`, and
`skill_http.rs` exists — the local sync layer is the missing foundation.

## Files in scope

- New `crates/operant-runtime/src/skills/sync.rs` (local manifest sync — phase 1)
- `crates/operant-runtime/src/skills/mod.rs` (wire `sync_skills()` + manifest helpers)
- `crates/operant-cli/src/cmd_skills.rs` or `main.rs` (a `sync` subcommand for the
  local layer: `operant skills sync`)
- Phase 2 (cloud client): new `crates/operant-runtime/src/skills/sync_client.rs` +
  `sync status|pull|push|now` CLI wiring, gated on auth (reuse the existing auth
  profile system in `operant-providers/src/auth/`)

## Files out of scope

- Registry rebrand (operant-labs) — the TODO stays.
- Curator behavior changes (only the sync hook points).

## Hermes reference (key semantics)

- `_read_manifest()`/`_write_manifest()` with `os.fsync` (Rust: `sync_all()`); v1
  entries get empty hash → migration on next sync.
- `_read_suppressed_names()` — skills the curator pruned must NOT be re-seeded.
- `_build_external_skill_index()` — never shadow externally-delegated skills.
- `_content_hash(directory)` (MD5 in hermes; use SHA-256 in Rust — provenance only).
- `sync_skills()` is a no-op when a marker file is present in HERMES_HOME.
- Cloud client: `build_sync_manifest_bytes`/`parse_sync_manifest`, `wire_address`,
  `canonical_json_bytes` (byte-identical or push fails `422 hash_mismatch`),
  `SyncInertError` when not signed in; debounced push hook + `maybe_pull_skills` at
  curator tick sites.

## Steps

1. **Phase 1 — local manifest sync**: implement `sync.rs` with
   `sync_skills(home) -> Result<SyncReport>`:
   - Discover bundled/optional skill dirs (from the packaged skills dir the runtime
     uses today — find it via the existing skills loading code).
   - Read/write manifest (`~/.operant/skills/.sync-manifest.json`), atomic tmp+rename +
     `sync_all()`, per-skill origin hash.
   - Suppressed-names + external-index exclusions.
   - No-op marker (`~/.operant/skills/.no-sync`) honored.
   - Wire a `operant skills sync` CLI subcommand that prints the report.
2. Tests for phase 1 (below).
3. **Phase 2 — cloud client** (separate round if phase 1 lands clean): manifest
   serialization, wire address, canonical JSON, and push/pull against the configured
   remote **only when authenticated** (inert error otherwise — hermes `SyncInertError`
   parity). Debounced push hook in `skill_manage` (after the write gate) + periodic pull
   at curator tick sites + `operant sync status|pull|push|now`.
4. Update `BUGS.md`; document the remote config keys.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-runtime --all-targets -- -D warnings && cargo clippy -p operant-cli --all-targets -- -D warnings
cargo test -p operant-runtime --lib skills::sync && cargo test -p operant-cli --lib sync
cargo test --workspace --all-features --lib          # final gate
```
Manual: `operant skills sync` on a fresh home creates the manifest and reports
N skills synced / 0 suppressed.

## Test plan

- `sync_creates_manifest_with_hashes`: temp home, seeded bundled dir → manifest exists,
  entries carry origin hashes.
- `sync_skips_suppressed`: curator-pruned name in suppressed list → not re-seeded.
- `sync_respects_noop_marker`: `.no-sync` present → no-op.
- `sync_skips_external`: externally-delegated skill name → not shadowed.
- `manifest_roundtrip_migration`: v1 (empty-hash) entries migrate on next sync.
- Phase 2: `sync_inert_when_not_authenticated` → `SyncInertError`-equivalent;
  `canonical_json_bytes_stable` (byte-identical across runs).

## Maintenance note

- The manifest is user data — never write it non-atomically (crash safety).
- Keep the cloud client inert-by-default; signing in is an explicit opt-in.

## Escape hatches

- If the packaged "bundled skills dir" doesn't exist in the shipped layout (skills come
  from the hub/`~/.operant/skills` only), scope phase 1 to "hub → user dir" sync and
  document it — do not invent a bundling story.
- If a remote endpoint/config is undefined, phase 2 stays behind a feature flag with
  the inert error — never guess an endpoint.
