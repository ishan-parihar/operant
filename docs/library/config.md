# Config — the one config system

Owns: operant's configuration mechanism — precedence, the TOML surface,
the one-home resolver, and profiles.
Not for: any specific subsystem's config keys (each library page names
its own), setup wizard flows ([operability.md](operability.md)).

Source: `crates/operant-core/src/config.rs` (AppConfig — THE live
config), `crates/operant-core/src/platform.rs` (operant_home),
`crates/operant-core/src/profile.rs`,
`crates/operant-config/src/` (the shared type crate),
`crates/operant-cli/src/config.rs` (CLI layer),
`crates/operant-cli/src/env_store.rs`,
`crates/operant-core/src/env_passthrough.rs`,
`operant.example.toml` (the template — update in the same change as any
config field), `crates/operant-core/tests/one_home_resolver.rs` (the
guard).

## One system

`operant_core::config::AppConfig` is the one config. The parallel
aggregate (`operant_config::schema::Config` with its loader, migration,
17 dormant sections and Configurable-derive machinery — ~25K lines) was
RETIRED (iters 1138-1141, RT-141): none of it ever loaded in
production. `operant-config` now holds only the shared types the live
config composes.

## Precedence

1. **Process env** (`OPENAI_BASE_URL`, `HERMES_MODEL`, `OPERANT_*`) —
   beats the file on every load; `operant config show` marks such keys
   and the loader warns once per key.
2. **`~/.operant/operant.toml`** — the TOML file (0600; plaintext
   provider keys are a documented accepted state, RT-154's migration
   path).
3. **`~/.operant/.env`** — credential pins (written by `/connect`),
   re-read each turn by BOTH the gateway runner and the TUI
   (`env_passthrough.rs::reload_dotenv`).
4. `~/.operant/config.toml` is NOT read by the binary; proxy
   environment variables never redirect provider traffic.

When config fields change, `operant.example.toml` updates in the same
change (repo law).

## The one-home resolver (iter-1137)

`operant_core::platform::operant_home()` is THE resolver:
`HERMES_HOME` (explicit — it wins) → the sticky active profile →
`~/.operant`. `platform::default_operant_root()` holds `profiles/` and
the sticky `active_profile` file, so switching profiles does not move
the switch. EVERY store under the operant home resolves through it —
database, config, `settings.json`, `auth.json`, `.env` pins, logs,
caches, approval allowlist, agents directory. The guard
`crates/operant-core/tests/one_home_resolver.rs` scans the workspace
and fails on any site building an operant-home path from the OS home;
genuine OS-home needs are listed there WITH a reason, in the same
commit.

## Profiles (`profile.rs`)

Multiple isolated operant instances: each profile is a full HERMES_HOME
directory tree under `~/.operant/profiles/<name>/`; the "default"
profile is `~/.operant` itself (backward compatible, zero migration).
`operant profile` manages them.

## Secrets discipline

Files that may hold secrets are written 0600 bypassing umask
(`fs_secrets.rs`), existing loose files are tightened in place, and
`auth.json` routes through the same writer (RT-154 residual closed
iter-1185). Secrets never ride the config template.

## Intention

One config system with one resolver exists because the fleet measured
the alternative: nine resolvers and one false promise (`operant profile
use` wrote a sticky file nothing read). The precedence order keeps
deployment overrides (env) honest and visible, the `.env` re-read each
turn keeps credential rotation live without restarts, and the guard
test makes the resolver a machine-checked invariant instead of a
convention.
