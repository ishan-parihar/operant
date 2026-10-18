# operant-infra

Channel infrastructure: session backends, message debouncing, and a stall watchdog.

## Purpose

Cross-cutting utilities used by several channel implementations (`src/lib.rs`). The session backend is in `src/session_backend.rs` with a SQLite implementation in `src/session_sqlite.rs` and a store in `src/session_store.rs`. Debouncing is in `src/debounce.rs` and the stall watchdog is in `src/stall_watchdog.rs`.

## Configuration

No Cargo features and no configuration types named in the crate docs. The SQLite backend uses bundled `rusqlite` (`Cargo.toml`). This crate does not state which backend a deployment selects; read `src/session_backend.rs`.

## Extension points

- `SessionBackend` trait in `src/session_backend.rs`. Implement it to add a session store alongside the SQLite one.

## Limitations

- The crate docs do not give debounce timing or watchdog thresholds; read `src/debounce.rs` and `src/stall_watchdog.rs`.

## Model experience

None. The crate docs describe channel plumbing (session storage, debouncing, stall detection) and no model-facing behaviour.
