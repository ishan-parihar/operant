# Lane I — CLI surface red-team (operant @ 25b86ae5)

- **Lane**: I (of the 2026-11-16 red-team + ponytail audit)
- **Seam**: the CLI surface itself — `main.rs` Commands enum, `commands.rs`, all
  `cmd_*.rs`, `autonomous.rs`, doctor, dashboard, setup, `post_setup.rs`,
  `hardening.rs`, plus the `operant-harness` / `operant-os` / `operant-api` /
  `operant-config` crates.
- **Pinned revision**: `25b86ae5` (confirmed `git rev-parse --short HEAD`).
- **Read-only.** No cargo, no state-mutating `operant` invocations.

## Status

- [x] skeleton written (first file action)
- [ ] command table
- [ ] doctor
- [ ] autonomous
- [ ] service/daemon
- [ ] operant-api / operant-config dead pub items
- [ ] docs checked last

## Files read (exact)

_(pending)_

## Files NOT read

_(pending)_

## THE COMMAND TABLE

_(pending — centrepiece)_

## Concept counts

_(pending)_

## Findings

_(pending)_

## Ponytail cut list

_(pending)_

## Rust discipline table

_(pending)_