# Parallel config surface — the schema that is never loaded (2026-10-21)

**Status:** audit (dated, frozen). **Ledger row:** RT-141 in
`docs/audit/2026-06-19-BUGS.md`. **Plan:**
`docs/plans/plan-2026-10-21-config-schema-retirement.md`.

## The claim

`operant_config::schema::Config` — the aggregate config type in
`crates/operant-config/src/schema/` — **is never loaded in production**. The
live config is `operant_core::config::AppConfig` (`crates/operant-core/src/config.rs`),
which is what the CLI parses the operator's `config.toml` into and what every
runtime path reads.

Consequently every field in the schema surface, except the handful the live
`AppConfig` composes, is dead: a knob set in the shape the schema describes
has never been read by a release binary.

## Evidence

Re-derivable with `python3 scripts/audit-parallel-config.py`:

```
schema structs found: 145
with no production reader outside operant-config: 123
fields in those structs: 645
```

The loader check is the load-bearing one, because a struct can be "read"
without the aggregate ever being loaded:

- `Config::load_or_init` — no production caller. The only hits are its own
  definition and the tests in `crates/operant-config/src/schema/core/core_tests.rs`.
- `Config::load` / `from_str::<Config>` / `parse::<Config>` — no production
  construction path.
- `operant_config::schema::Config::default()` — only test fixtures.
- The one production-typed consumer,
  `operant-providers::provider_runtime_options_from_config`, is itself called
  only from its own tests.
- `operant-core/src/doctor.rs` documents the same gap from the other side:
  the checks that need `schema::Config` fields are "named gaps, not fakes".

The live half of the crate — what IS used:

| from `operant-config` | used by |
|---|---|
| `providers::ProvidersConfig`, `schema::ModelProviderConfig`, `schema::FallbackProviderConfig` | the live `AppConfig`'s `[providers]` |
| `schema::ProxyConfig` (+ the runtime proxy helpers) | the provider and tool HTTP clients |
| `schema::ObservabilityConfig` | the live `AppConfig`'s `[observability]` |

A few more types (`ReliabilityConfig`, `MultimodalConfig`, `ModelRouteConfig`,
`SearchMode`, `MemoryPolicyConfig`, `ContextCompressionConfig`) are referenced
by the providers crate, whose own routing is narrow (see the ledger's
provider-namespace row: the runtime has exactly two model clients).

## What this is NOT

- It is **not** an operator-facing break. `operant.example.toml` advertises
  only tables the live `AppConfig` accepts, and the settings structs use
  `deny_unknown_fields`, so a `[telegram]`-style table from the schema
  surface would be REJECTED by the live loader rather than silently ignored.
  The two surfaces cannot be confused at the file level.
- It is **not** new. `docs/R40-12-13-WIRE-OR-RETIRE-REVIEW.md` reached the
  same conclusion ("the 1,043-field schema is dead", "world (2) is the live
  one and is already the de facto answer, by usage rather than by decision")
  and recommended "pick a winner". That recommendation was never executed and
  was never recorded in the live ledger — this audit and RT-141 close that
  gap.
- The count differs from the prior audit's 1,043 fields: this script counts
  fields in structs whose TYPE NAME has no production reader outside the
  crate, and reports the per-struct field counts. The two numbers are not
  comparable field-for-field; the direction and the conclusion are the same.

## Why it matters

Two config schemas in one binary is a divergence waiting to happen: a setting
added to the schema surface looks live to the next reader (and to the next
agent) but is unreachable, and a setting the operator actually needs goes
into the live `AppConfig` while the schema surface keeps claiming it.

## Options (the owner's call)

1. **Retire the schema surface** — delete `operant-config/src/schema/` and
   the `Configurable` derive machinery, keeping only the live types the
   `AppConfig` composes. Biggest deletion in the repo's history (≈30k lines
   plus a 5k-line test file) and it destroys the product spec those structs
   encode (≈15 messaging platforms, tunnels, hardware, autonomy, cost).
2. **Migrate the live config onto the schema surface** — one `Config` for the
   binary, the schema becomes the spine. The larger change, and it needs the
   loader to gain a real caller first.
3. **Label and defer** (what iter-1128 does): say plainly, in the crate, that
   the surface is a spec rather than live config; keep a static guard that
   fails if the surface starts being loaded; leave the retirement to a
   planned, owner-approved change.

The recommendation is 3 now and 1 (or 2) as a planned, reversible change:
the schema surface carries real product intent, so deleting it without a
migration plan would throw that intent away, and the "nothing is deleted"
law in `AGENTS.md` requires an archive path rather than a silent removal.

## Limits of this audit

- The script is a heuristic: identifier-occurrence counting, not type-aware. A
  struct read only through a macro or reflection would be missed; a name
  shared with an unrelated symbol would be a false hit. The loader check
  (`load_or_init`) does not depend on the heuristic.
- "Production" here means "not `#[cfg(test)]`". Code behind a feature flag
  that is not enabled by default counts as production here, and a field read
  only under such a flag would not be flagged by the struct scan but would
  still be unreachable in a default build.
