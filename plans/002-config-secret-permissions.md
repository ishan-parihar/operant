# 002 — Config & state secrets must be 0600

Stamped: `d394c136`. Priority: **P0** (security).

## Why

The main config file (contains provider API keys) is written with plain `fs::write`
→ default permissions (0644 under umask 022) — world-readable on a shared machine.
`operant-cli/src/cmd_setup.rs:1131` and `:1316` are the two known write sites.
`mcp_oauth` tokens were already 0600-hardened in earlier rounds; the main config was
missed. State DB / `.env` need an audit too.

## Files in scope

- `crates/operant-cli/src/cmd_setup.rs` (config + .env write paths, ~1131 and ~1316)
- `crates/operant-core/src/config.rs` (config write path ~1229 and any other `fs::write`
  of user config)
- Any `.env` writer in `crates/operant-cli/src/` (grep for `".env"` / `env_file`)
- `state.db` / WAL creation path (grep `state.db` in `crates/operant-core/src/`)

## Files out of scope

- Session/message DB content (non-secret) — leave perms as-is unless the audit shows
  secrets stored there.
- OAuth token files (already 0600, verified R29).

## Current state (evidence)

- `crates/operant-cli/src/cmd_setup.rs:1316`: `std::fs::write(&config_path, &toml_str)`
  — no permission hardening. `:1131`: `std::fs::write(dest, &bytes)?` (likely .env).
- `crates/operant-core/src/config.rs:1229`: another `fs::write` (test fixture context).
- No `set_permissions` / `0o600` / `0o700` anywhere in `cmd_setup.rs` or `config.rs`
  (grep verified).

## Steps

1. Add a helper `write_secret_file(path, bytes) -> io::Result<()>` in
   `crates/operant-cli/src/cmd_setup.rs` (or a shared `secret_utils` module if the CLI
   already has one): write via `OpenOptions::new().write(true).create(true).truncate(true)`
   then `set_permissions(0o600)` on the created file (on unix). On existing files that
   are already 0600 or stricter, leave them; if looser, tighten to 0600.
2. Route the config write (`:1316`), the `.env` write (`:1131`), and the `config.rs`
   write path through the helper.
3. Audit + tighten `state.db` + `-wal`/`-shm` creation: if the DB can hold auth/token
   material (check `database.rs` schema for token/secret columns), create with 0600
   (SQLite `CREATE` honors the process umask; set perms explicitly after open).
4. Add unit tests (see below). Do not use `std::fs::write` for any file that may
   contain a secret going forward.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-cli --all-targets -- -D warnings && cargo clippy -p operant-core --all-targets -- -D warnings
cargo test -p operant-cli --lib secret && cargo test -p operant-core --lib secret
cargo test --workspace --all-features --lib          # final gate
```
Manual: `operant setup` (or the config-write path) produces a config file whose
`stat -c %a` is `600`.

## Test plan

- `test_config_write_sets_0600`: call the write helper (or the setup path with a temp
  home), assert `metadata.permissions().mode() & 0o077 == 0` on unix.
- `test_env_write_sets_0600`: same for the .env path.
- `test_existing_loose_file_tightened`: pre-create a 0644 file, run the helper, assert 0600.
- Follow the existing test style in `cmd_setup.rs` (temp-dir based).

## Maintenance note

- Any future file that may hold credentials must use the secret-write helper.
- Review this whenever new auth flows are added (provider OAuth, gateway pairing tokens).

## Escape hatches

- If the config write path is shared with a non-secret write (permissions must stay
  0644 for some reader), split the paths rather than forcing 0600.
- Non-unix platforms: keep the helper a no-op or use best-effort ACLs — the CI matrix
  includes Windows/macOS; do not break the build there.
