---
ID: "AD-037"
Title: "A rotated credential replaces the stored secret, and the two rotation operations are separate"
Status: "Superseded by AD-038"
Date: "2026-10-19"
Description: "The gateway's per-turn .env reload wrote the new key into the process environment, where nothing read it back, so credential rotation did not work at all; the reload now reports the value and the gateway applies it to the running client"
---

# AD-037: A rotated credential is pushed into the live client, not just written into the environment

Immutable once numbered: a correction is a new record that supersedes this one.

> **SUPERSEDED by AD-038 (iter-986).** The claim this record made — "a key
> rotated in `.env` now takes effect on the next turn" — was FALSE when
> published. The rotation did not reach the credential pool, which
> re-selects the startup-seeded key before every request, and the reload
> read a CWD-relative `.env` rather than the documented
> `~/.operant/.env`. Both are fixed in AD-038. The diagnosis in this
> record — that the rotation did not work at all — was correct, and the
> seams it identified are the ones AD-038 builds on.

## Context

RT-6c was filed as a race:

> OPEN residual: a real rotation still writes the process env while the
> runtime is live, and a C-level `getenv` in a child can race that write.

Reading the code found that the race was not the problem. **The rotation
did not work at all.**

`reload_dotenv` (`operant-core/src/env_passthrough.rs`) reads the `.env` file
and writes each `KEY=VALUE` pair into the process environment. Its own
module doc claims this "enabl[es] credential rotation without restarting the
long-lived gateway daemon", and `gateway_runner.rs` calls it before every
turn.

But nothing in the running agent read the environment back:

- The client captured its key when it was constructed. `ClientConfig.api_key`
  → `OpenAIClient::new` → `Client.config.api_key`. `build_headers` reads
  `self.config.api_key`, with `api_key_override` on top of it. The
  environment is not consulted again.
- The credential pool seeded its keys at startup. `build_pool_registry`
  calls `std::env::var(provider_env_var(provider))` once, when the registry
  is built.
- The gateway never called `apply_env_overrides` per turn. That function
  exists and maps `OPENAI_API_KEY` onto `config.client.api_key`, but the
  per-turn path did not call it, and even if it had, the client had already
  been built from the old value.

So a key rotated in `.env` was written into a place nobody read. The
feature was a no-op with a plausible doc comment.

The seam that DOES rotate a live credential already existed:
`OperantAgent::set_endpoint` → `ModelClient::set_endpoint` → the client's
`set_api_key`, with the pooled client dropping the old endpoint's keys and
seeding the new one. `/connect` used it. The `.env` reload did not.

## Decision

**The reload reports; the caller applies.**

`reload_dotenv` now returns a `DotenvReload { api_key, base_url }` carrying
the credential and the endpoint the file names. The gateway's per-turn path
pushes them into the live client through a new `OperantAgent::set_api_key`
— the same path `/connect` already used.

**The TUI does the same.** The TUI is the other long-lived surface: it
builds its client at startup and keeps it for the session, so a key
rotated in `.env` while a TUI session was open did not reach it either.
`run_pending_agent_actions` now reloads and applies, once per turn, next to
the `/connect` retarget it already did.

**The process-environment write is kept.** Tools and child processes read the
environment, so the write is still what makes a rotated key visible to them.
The returned value is what makes it visible to the agent.

**The reported value is the file's value, not the environment's.** The
"did it change?" check against the environment governs the WRITE, not the
REPORT. An operator who rotated the key has a new key in the file; the
caller must apply it even when the process environment still holds the old
one, or the rotation is hidden by a comparison against a value that was
never applied.

**The residual C-level `getenv` race in a child process stays.** Closing it
needs an explicit child environment across the ~206 `Command::new` sites.
That is a larger change with its own risk profile, and it is now a race on
a value that is actually used: the value the agent authenticates with is
swapped through a lock (`api_key_override`), not through the environment.

## Evidence

- `the_reload_reports_the_credential_it_found`: a `.env` carrying
  `OPENAI_API_KEY="rotated-key"` and `OPENAI_BASE_URL` reports both, so the
  caller can push them into the live client.
- `a_reload_without_a_credential_reports_nothing`: a `.env` with no
  credential reports nothing, so the caller does not clear a working
  credential.
- The two tests are serialized by one lock, because the reload reads its
  path from the process environment and parallel tests would read each
  other's file.
- `cargo test --workspace`: 0 failed.
- `cargo fmt --all`. Clippy gate: `seen=24 allowlist=24 new=0 stale=0`.
- `./scripts/genome lint`: clean.

## Consequences

- A key rotated in `.env` takes effect on the next gateway turn, without
  restarting the daemon. Before this, it took effect never.
- The gateway now holds a handle to the handler for the turn, so the
  rotation can reach the live client.
- `OperantAgent` gained `set_api_key`, alongside the existing `set_endpoint`.

## Open

- The C-level `getenv` race in a child process is unchanged. It needs an
  explicit child environment across the `Command::new` sites.
