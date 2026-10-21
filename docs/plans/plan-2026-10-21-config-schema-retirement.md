# Plan — retire (or adopt) the parallel config schema surface (2026-10-21)

**Status:** plan (dated, frozen). **Audit:** `docs/audit/2026-10-21-PARALLEL-CONFIG-SURFACE.md`.
**Ledger row:** RT-141 in `docs/audit/2026-06-19-BUGS.md`.

## Problem

`operant_config::schema::Config` is never loaded in production; the live
config is `operant_core::config::AppConfig`. 123 of the schema's 145 structs
(645 fields) have no production reader outside `operant-config`. Two config
surfaces in one binary is a divergence waiting to happen.

## What is NOT in scope

- No change to the live `AppConfig` or to any behaviour the operator sees.
- No deletion before the owner rules on option 1 vs option 2 below.
- The `operant-config` types the live `AppConfig` composes (`ProvidersConfig`,
  `ProxyConfig`, `ObservabilityConfig`, `ModelProviderConfig`,
  `FallbackProviderConfig`) stay exactly as they are.

## Phase A — label (shipped in iter-1128)

1. A module doc at the top of `crates/operant-config/src/schema/` states, in
   the first lines a reader sees, that the surface is a SPEC and not the live
   config; the live config is `operant_core::config::AppConfig`; the loader
   `Config::load_or_init` has no production caller; the retirement is planned
   in this file.
2. A static guard test: if any non-test file in the workspace starts calling
   `load_or_init` (or otherwise deserializes `schema::Config`), the test
   fails and points at this plan — so the surface cannot start being loaded
   by accident, and the moment it IS wired on purpose, the guard is updated
   in the same commit.
3. `scripts/audit-parallel-config.py` re-derives the counts.

## Phase B0 — the AGGREGATE is gone (shipped in iter-1138)

The owner's decision (delegated: "choose recommended best decisions on your
own") was Option 1, retire. Step 1 of it shipped in iter-1138:

- The aggregate `Config` (~30 sections, `crates/operant-config/src/schema/proxy.rs`),
  its loader and its whole impl (`schema/config_impl.rs`, 1.9K lines), the
  v0→v1 migration of that shape (`migration.rs`'s `V1Compat`), and its
  7.1K-line test file (`schema/core/core_tests.rs`) are DELETED — about
  9.7K lines, and with them the second loader, the second `config.toml`
  resolution and the second migration.
- The two production-typed consumers of the aggregate
  (`operant-providers::provider_runtime_options_from_config`,
  `operant-providers::auth::{from_config, state_dir_from_config}`) were
  called only from their own tests; they are deleted with it.
- The guard test is now stronger: it fails if `schema::Config` is USED
  anywhere in production code, and a second test fails if the type is
  DEFINED again.
- What remains in `schema/` are the TYPES the live config composes. The
  production surface that uses them, measured rather than assumed, is
  exactly 12 types and 5 functions:
  `ProxyConfig`, `ProxyScope`, `ObservabilityConfig`, `ModelProviderConfig`,
  `FallbackProviderConfig`, `ModelRouteConfig`, `EmbeddingRouteConfig`,
  `ReliabilityConfig`, `MultimodalConfig`, `ModelPricing`,
  `MemoryPolicyConfig`, `SearchMode`; plus
  `build_runtime_proxy_client_with_timeouts`, `apply_runtime_proxy_to_builder`,
  `build_runtime_proxy_client`, `client_config` and
  `set_runtime_proxy_config`.

Step 2 (the dormant SECTION structs — 123 of them, 645 fields) is the rest
of Option 1 and is NOT yet done.

## Phase B — the owner's choice

### Option 1: retire the schema surface

- Archive the module (`genome archive` / the repo's tombstone path) rather
  than deleting it outright: the structs encode product intent for ~15
  messaging platforms, tunnels, hardware, autonomy and cost, and that intent
  is worth keeping in an archive branch.
- Keep the live types the `AppConfig` composes; move them to a
  `operant-config/src/live.rs` (or fold them into `operant-core`).
- Delete the `Configurable` derive machinery, `migration.rs` (the v0→v1
  migration of the DEAD shape) and the 5k-line `core_tests.rs` that tests the
  dead shape.
- Risk: `operant-providers` and `operant-memory` depend on the crate;
  `schema-export` (a default feature) and the `schema::Config` type in
  `operant-providers::auth` need re-pointing. A prior review measured this
  crate at ~31k lines.

### Option 2: adopt the schema surface as the spine

- Give `Config::load_or_init` a real production caller and make the CLI's
  `AppConfig` a thin projection of it.
- Requires reconciling the two shapes field by field (the live `AppConfig`
  has keys the schema surface lacks, e.g. `[tui]`, `[tools]`, `[bylaws]`,
  `[holon]`; the schema surface has ~25 sections the live one lacks).
- Larger, and it must not break `deny_unknown_fields` for existing operator
  configs.

### Option 3: keep both, permanently

- Only defensible if the schema surface is actively maintained as a spec and
  a future migration is committed. It is not defensible without a date and
  an owner, because the next agent will read it as live config.

## Verification for any option

- `python3 scripts/audit-parallel-config.py` — the counts must move in the
  expected direction (fewer dormant structs, or a live reader for the
  aggregate).
- `cargo test -p operant-config --lib` and the workspace suite.
- The live-config behaviour is unchanged: `operant config show` still prints
  the operator's config, `operant doctor` still reports on it, and the
  example template still loads.

## Recommendation

Phase A now (label + guard + script), then option 1 as a planned, reversible
change — with the schema surface archived, not deleted outright, and the
product intent it encodes kept as a document. Option 2 only if the operator
wants the schema surface to become the config spine.
