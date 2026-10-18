# operant-os

Operant as Holon's runtime: read-only org views, policy, run supervision, and the Chair's channel.

## Purpose

The org side of `docs/agentic-os/` (`src/lib.rs`). The crate docs describe the current phase as the read API: Holon publishes SQL views named `api_*` over its `work.db`, and this crate is the only sanctioned reader of them on Operant's side. Modules include `api.rs`, `policy.rs`, `run.rs`, `supervisor.rs`, `chair.rs`, `events.rs`, and `tools.rs`. The crate docs list later phases (policy, trajectory and usage capture, supervisor/runner/broker, chair/events) without stating their status.

## Configuration

No Cargo features. The interface contract is Holon's `docs/CONTRACT.md` (cited in `src/lib.rs`). The boundary record is in a private overlay that this repo does not contain.

## Extension points

Add a new read by querying a published `api_*` view through the reader in `src/api.rs`. `read_only_view` rejects any other table name.

## Limitations

- The database is opened with SQLite `mode=ro`. Operant never writes a Holon store, and the connection string enforces this.
- The client refuses an `api_version` it does not recognise.
- Only `api_*` views are reachable.

## Model experience

The crate docs describe no model-facing behaviour. The Chair channel, in `src/chair.rs` and `src/events.rs`, is the part of the design that touches the org layer; read those files for what they expose.
