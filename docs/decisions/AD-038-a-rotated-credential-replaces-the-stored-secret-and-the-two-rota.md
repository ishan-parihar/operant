---
ID: "AD-038"
Title: "A rotated credential replaces the stored secret, and the two rotation operations are separate"
Status: "Active"
Date: "2026-10-19"
Description: "The rotation did not reach the credential pool, which re-selects the startup-seeded key before every request, and the reload read a CWD-relative .env rather than the documented one; both are fixed, and 'use this key now' and 'replace the stored secret' are now separate operations"
---

# AD-038: A rotated credential replaces the stored secret, and the two rotation operations are separate

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

AD-037 diagnosed the credential rotation correctly: the gateway's per-turn
`.env` reload wrote the new key into the process environment, and nothing
read it back. But its FIX was wrong in three ways, and the claim it
published — "a key rotated in `.env` now takes effect on the next turn" —
was false.

**1. The rotation never reached the pool.** The default client is a
`PooledModelClient`. Its request loop does:

```rust
let Some(cred) = self.pool.select() else { break };
self.inner.set_api_key(&cred.value);   // <- overwrites the rotated key
```

So a key set only on the inner client is replaced by the pool's
startup-seeded value on the very next turn. AD-037 called
`agent.set_api_key(key)`, which reached `PooledModelClient::set_api_key` →
`inner.set_api_key` — and nothing else. The rotation was a no-op.

**2. The pool's credential could not be found by source.** The obvious fix
— "replace the `env:`-sourced credential" — does not work, because
`CredentialPoolRegistry::add_seed` DEDUPES BY VALUE, and at startup:

- `apply_env_overrides` copies `OPENAI_API_KEY` into `config.client.api_key`
  (`operant-core/src/config.rs:2403`).
- `build_pool_registry` seeds "primary" with that value, source
  `"config (client.api_key)"`, and then seeds the provider's env var with
  the same value — which the dedupe DROPS.

So the pool holds ONE credential, named "primary", whose source is the
config seed but whose VALUE came from the environment. There is no
`env:`-sourced credential to find. Adding a new credential would put it
BEHIND the stale primary under `FillFirst` (the default strategy), so the
rotation would still not take effect.

**3. The reload read the wrong file.** `reload_dotenv` read
`OPERANT_ENV_FILE` → `HERMES_ENV_FILE` → **`.env` in the current working
directory**. The startup loader (`CliConfig::default`) reads
`<config_dir>/.env`, where `config_dir` is `HERMES_HOME` or `~/.operant`.
In the documented setup the key lives in `~/.operant/.env` — a file the
per-turn reload never opened unless the gateway's working directory
happened to be `~/.operant`.

There was a fourth problem, found while fixing the third: **the two
rotation operations had been conflated.** `set_api_key` means two different
things:

- the agent-loop rotation path (`try_rotate_credential`) calls it with a
  DIFFERENT existing credential to switch to;
- a runtime credential rotation wants to REPLACE the stored secret.

Doing both in one method corrupts the pool on every 401 and every 429: the
benched credential would take the next key's value, and the pool would end
up holding a duplicate.

## Decision

**1. "Use this key now" and "replace the stored secret" are separate
methods.**

- `ModelClient::set_api_key` means "use this key now". It does NOT touch
  the pool. The agent-loop rotation path needs exactly this.
- `ModelClient::rotate_credential_value` means "the operator rotated the
  secret; replace the stored one". The pooled client implements it.

`OperantAgent::set_api_key` calls `rotate_credential_value`, because the
only caller of that method is the `.env` rotation.

**2. The pooled client replaces the credential it is actually USING.**

`PooledModelClient` tracks `last_credential_id` — the id of the credential
the pool last selected — and `rotate_credential_value` replaces THAT
credential's value in place (new `CredentialPool::replace_value`: the id,
the source, the ordering and the failure bookkeeping stay; only the secret
changes). When no request has run yet, it falls back to the FIRST
credential, which is the one `FillFirst` will pick next.

Two guards keep it safe:

- **The pool already holds this value → no-op.** The agent-loop rotation
  path passes a credential it just selected FROM the pool; replacing
  anything there would overwrite the benched credential's secret and leave
  a duplicate.
- **The value is not in the pool → replace in place, never add.** Adding
  would put the new key behind the stale primary.

**3. The reload reads the same file the startup loader reads.**

`dotenv_path()` resolves `OPERANT_ENV_FILE` → `HERMES_ENV_FILE` →
`<operant_home>/.env` → `.env` in the working directory. The third entry
is the documented location and the one `CliConfig::default` uses.

**4. A retarget only happens on a CHANGE, and keeps the current key.**

`set_endpoint` on a pooled client drops every credential and re-seeds only
when a key is present. A `.env` carrying only a base URL (the key lives in
`operant.toml`) reaches the retarget with no key, so the pool would be
left EMPTY and the next request would fail with "no available keys". Two
guards:

- retarget only when the base URL actually CHANGED (`ModelClient::base_url`
  was added for this);
- pass the key the client is CURRENTLY USING (`ModelClient::current_api_key`)
  when the file does not carry one.

## Evidence

Every claim above is pinned by a test. The pool tests:

- `a_rotated_key_is_the_key_the_next_request_sends` — the next request
  carries the rotated key.
- `a_rotation_reaches_the_single_credential_the_gateway_builds` — the
  production shape (ONE credential, named "primary", source
  `"config (client.api_key)"`, holding the environment's value).
- `a_rotation_before_any_request_reaches_the_first_credential` — a
  rotation before any request still reaches the pool.
- `a_rotation_rescues_a_pool_with_no_credentials` — a pool with no
  credentials at all.
- `a_rotation_keeps_the_config_sourced_keys` — a rotation does not
  discard the other keys.
- `a_key_the_pool_already_holds_does_not_change_the_pool` — the agent-loop
  path does not rewrite the pool.
- `the_agent_loop_rotation_does_not_rewrite_the_benched_credential` — the
  full `try_rotate_credential` sequence: use k1, invalidate it, select k2,
  switch. k1's secret survives.
- `an_internal_pool_rotation_changes_no_credential_value` — a 429 moves to
  the next credential and changes no secret.
- `a_retarget_with_the_current_key_keeps_the_pool_usable` — a retarget
  with the current key leaves the pool usable.

The reload tests:

- `the_reload_reads_the_documented_env_file` — `HERMES_HOME` is honoured
  and the reload reads `<HERMES_HOME>/.env`.
- `the_reload_reports_the_credential_it_found` — the credential is
  reported.
- `a_reload_without_a_credential_reports_nothing` — a file with no
  credential reports nothing.

**Mutation evidence.** Two mutations were run against the pool tests, and
the new tests caught both:

- `rotate_credential_value` not touching the pool → 5 tests fail
  (`a_rotation_rescues_a_pool_with_no_credentials`,
  `a_rotated_key_is_the_key_the_next_request_sends`,
  `a_rotation_before_any_request_reaches_the_first_credential`,
  `a_rotation_keeps_the_config_sourced_keys`,
  `a_rotation_reaches_the_single_credential_the_gateway_builds`).
- `set_api_key` also rewriting the pool → `the_agent_loop_rotation_does_not_rewrite_the_benched_credential`
  fails. The first attempt at this mutation was NOT caught, which is why
  that test exists: the corrupting sequence needs an invalidation first,
  exactly as `try_rotate_credential` does it.

## Consequences

- A key rotated in `~/.operant/.env` takes effect on the next turn, without
  restarting the gateway daemon, and without a 401 in between.
- A 401 or a 429 no longer risks corrupting the credential pool.
- A `.env` that carries only a base URL no longer empties the pool.
- `ModelClient` gained three methods: `rotate_credential_value`,
  `base_url` and `current_api_key`. All three default to no-ops, so a
  client that cannot support them is unaffected.

## Open

- The C-level `getenv` race in a child process is unchanged. It needs an
  explicit child environment across the `Command::new` sites.
- The rotation replaces the credential the client is using. If a user has
  several keys in `client.additional_api_keys` and rotates the primary in
  `.env`, the rotation applies to whichever credential was selected — the
  primary in the common case, but the selection is the pool's strategy's
  decision. Making the rotation target the primary specifically would need
  the pool to expose which credential came from which source.
