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

## The upgrade contract (iters 2254/2257)

Four layers answer one question: *what happens to this config when a newer
binary meets an older file, and how does an agent adding a key know
everything it must update?*

```
L1 schema   : typed field, serde default, doc comment (the NOTE), range
L2 heal     : heal() -> Vec<Healed{key, old, new, why}>   [config_heal.rs]
              load -> heal -> persist-once -> one warn! naming every row
L3 describe : operant config describe | config notes <key> (+ config_manage)
L4 doctor   : the upgrade sweep [config.resilience] — seven named rows
```

**L1 — the note is the single source.** The doc comment on a settings field
is the note. `scripts/gen-config-registry.py` extracts every field, its dotted
path, its default and its note into `config_registry.rs` (272 keys today). The
registry is generated, never hand-edited; `gen-config-registry.py --check`
fails when it is stale, and `scripts/self-test.sh` runs that check.

**L2 — every heal is named.** `AppConfig::heal()` sums per-struct tables
(client, agent, gateway, tool_search, rate_limit). A healed value is a row
carrying the key, the old and new value, and the reason, so an agent reads
*what was healed and why* rather than that something was. `persist_healed()`
writes the file once, atomically (`write_secret_file` + rename, mode
carried), and only when a heal actually changed something. A second load heals
nothing — idempotence is a contract, not an accident.

**The unknown-key case (the upgrade case).** A key this binary does not know
never fails the load. It is *parked*, not dropped: the parse recovers from
serde's `unknown field`/`unknown variant` errors, writes the recovered pairs to
a sidecar (`operant.unknown.toml`, mode 0600), and names them. The operator's
own file stays clean and a downgrade round-trips. An unparseable file is
reported with the offending source line quoted, never a silent default.

**L3 — the map.** `operant config describe [section] [--json]` lists every key
with its default, its current value and its note; the current value is marked
non-default. `operant config notes <key>` gives the full note for one key, and
an unknown key gets the near-miss suggestion (`Did you mean: ...?`) instead of
a bare miss. The same pair is on the in-loop `config_manage` tool, so an agent
that only has the tool reads the same map. The describe output is *generated
from* the note, never hand-copied.

**L4 — the doctor's upgrade sweep.** `check_config_resilience()` reports a new
`[config.resilience]` group with named rows: `unknown-key`, `deprecated-key`,
`missing-section`, `range`, `two-files`, `unparseable`. It is silent on a
healthy config and each row names the key, the value and the path.

**The forward gate — a ratchet, not a wall.**
`scripts/audit-config-keys.py --check` fails when a config key does not reach
its note, heal table, template or a doctor row. Applied as a wall it would
fail on all 272 existing keys, which predates the note discipline — and a wall
of red is how a gate stops being run. So it ratchets: a **new** key must reach
all four surfaces (measured, so an agent adding a key cannot skip them), while a
**legacy** key may not get *worse* (enforced). The baseline can only shrink, so
the backlog is tracked rather than hidden; `--list-legacy` regenerates it.
`scripts/tests/config_key_gate_test.sh` mutation-tests both halves, because a
gate that has never fired is not known to work.

## Intention

One config system with one resolver exists because the fleet measured
the alternative: nine resolvers and one false promise (`operant profile
use` wrote a sticky file nothing read). The precedence order keeps
deployment overrides (env) honest and visible, the `.env` re-read each
turn keeps credential rotation live without restarts, and the guard
test makes the resolver a machine-checked invariant instead of a
convention.
