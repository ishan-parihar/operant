# Real organism manifest fixtures

**Harvest date: 2026-09-30.**

Every `.yaml` in this directory is a **byte-for-byte copy** of a real
`~/.hermes/organism/**/_org.yaml` or `_pool.yaml`. Nothing was normalized,
redacted, re-indented, or re-generated. The table below records the exact
**source path** for each fixture, so a drifted copy is detectable by comparing
`sha256sum` of the fixture against the named file:

```sh
sha256sum crates/operant-harness/tests/fixtures/task-grid__org.yaml \
  ~/.hermes/organism/swarm/task-grid/_org.yaml
```

This comparison was verified byte-identical for all 22 fixtures at harvest
time. Note it is **not** enforced by a test, because the organism tree is not
guaranteed to exist on every machine; `tests/pool_real_manifests.rs` exercises
these fixtures for *parse and compile* behavior, which is the property that
actually regressed.

The point of these files: the shipped `PoolManifest` struct was developed
against a **hand-written fixture matching a schema no real pool has**
(`pool.rs` legacy test manifest, dotted verb names like `query.contacts`).
It was green in CI and inert in production. These fixtures are the cure —
`tests/pool_real_manifests.rs` refuses to pass if any of them stops parsing.

## Why there are two dialects

Measured over the organism tree on 2026-09-30 (physical files only,
de-duplicated by `realpath` because `strategy-incubator/sub-systems/graduates/`
symlink-fans replicate the same real file many times):

| Dialect | Physical files | Where |
|---|---|---|
| `_org.yaml` (canonical) | **94** | every live pool, 6 graveyard, 61 nested sub-systems, 3 archive, 1 root |
| `_pool.yaml` (legacy) | **32** | 22 in `cortex/meta-governance/archive/`, 9 graveyard, **1 live** |

The organism's own resolver (`cortex/meta-governance/lib/core.py:217`
`find_pool_config`) treats `_org.yaml` as canonical and `_pool.yaml` as a
legacy fallback that prints a WARN. **All 24 live top-level pools carry
`_org.yaml`; exactly one (`foundations/presentation-engine`) carries both.**
So `_org.yaml` is the schema that matters for live pools, and `_pool.yaml`
is the schema the historical docs and the shipped struct were written
against. Both must parse.

## Fixture inventory

### `_org.yaml` — canonical dialect (15 files)

| Fixture | Source path | Why it is here |
|---|---|---|
| `meta-governance__org.yaml` | `cortex/meta-governance/_org.yaml` | L0 governance. Only pool with `tier: meta-system`; no `relationships`; `required: false` on most dirs. |
| `task-grid__org.yaml` | `swarm/task-grid/_org.yaml` | AXE. `sub_system: (root)` sentinel; `access: import`; has `relationships.loops`. |
| `workforce-ops__org.yaml` | `swarm/workforce-ops/_org.yaml` | HR. Flow-style `accepts: [runs, review]`, `returns:` as a LIST, `purpose:` on every offered service, `sub_dir` written as an inline flow map. |
| `agent-fabric__org.yaml` | `swarm/agent-fabric/_org.yaml` | FABRIC. Three `access: cli` consumed edges; long single-line `charter`. |
| `identity-core__org.yaml` | `foundations/identity-core/_org.yaml` | Foundation. `accepts: []` empty list; multi-line folded `entry_point` with an embedded `writes via identity CLI contracts` note. |
| `platform-infra__org.yaml` | `foundations/platform-infra/_org.yaml` | Only live file carrying `sub_systems:` (a plain string list); `allowed_root_files` at top level; `required: true` dirs. |
| `research-vault__org.yaml` | `foundations/research-vault/_org.yaml` | Foundation with multiple offered services. |
| `career-pipeline__org.yaml` | `ventures/career-pipeline/_org.yaml` | Graduate. `pool.graduation` block, `surface_registry` with a nested `gate` block, `pooled_sub_systems` string list, `access: symlink+cli` (compound value). |
| `brand-integrating__org.yaml` | `ventures/brand-integrating/_org.yaml` | Graduate with `pool.prefix`, `pool.graduation.genome_template_passed` (map of `G1..G9: bool`), `pool.graduation.shared_services`, `surface_registry` gate with 5 fields. |
| `graveyard__org.yaml` | `graveyard/_org.yaml` | Graveyard pool. **No `tier`**, `_legacy_tier: null`, `directories` with a `.` key and per-subtree nesting. `class: utility`. |
| `research-engine-20260907-retired__org.yaml` | `graveyard/research-engine-20260907-retired/_org.yaml` | Retired foundation; `access: file` (only file-access edge in the tree). |
| `alignment-engine__org.yaml` | `foundations/content-campaigns/sub-systems/alignment-engine/_org.yaml` | `services_offered` is a list of **bare strings** and `services_consumed` is a **`{service: provider}` consumer map** (4 entries) — both shapes in one file, in one dialect. Also carries a top-level `interfaces:` list of bare `{input,output}` strings, and `rules:` (not enforced by the linter). |
| `forge__org.yaml` | `foundations/strategy-incubator/sub-systems/forge/_org.yaml` | `services_offered` key is **absent entirely** (not null, not empty) while `services_consumed` is a **bare-string list** (`[trajectory, notion]`) — a third consumed shape, distinct from the consumer-map above. `suffix:` appearing as both a scalar and a block list, plus a `shebang:` key and a `pattern:` glob dir. |
| `living-architecture__org.yaml` | `cortex/meta-governance/sub-systems/living-architecture/_org.yaml` | The only live fixture with just four top-level keys (`pool`, `directories`, `services_offered`, `required_files` — no consumed, no relationships). Its one offered service's `returns:` is a **long bare scalar string**, not a flow list, so `returns` had to accept both. |
| `contacts-dup-conflict__org.yaml` | `foundations/relationship-crm/sub-systems/contacts/_org.yaml` | The real file with a genuine **duplicate key whose values disagree**: under `directories.core/` it declares `required: false` and then `required: true`. Parsed leniently (PyY-compatible last-wins) it parses fine and the later value wins; `parse_organism_strict` is what refuses it, naming `directories.core/.required`. Verified by running `duplicate_key_report` against the file — the only fixture in the set that reports a conflict at all. |

### `_pool.yaml` — legacy dialect (7 files)

Added in the second harvest pass (2026-09-30, same day) after a live
whole-tree run surfaced five shapes the first 18 fixtures missed. Four were
kept; the fifth (`scratch-test-null-services__pool.yaml`) was **deleted
because it turned out to be byte-identical to `scratch-test__pool.yaml`**
(`sha256 cb5860e5…`) — the tree has no second file with null service lists,
so keeping it would have been a duplicate claiming to be a distinct case.

| Fixture | Source path | Why it is here |
|---|---|---|
| `presentation-engine__pool.yaml` | `foundations/presentation-engine/_pool.yaml` | The ONLY live `_pool.yaml`. Richest legacy file: `genome{produce,operate,observe,align}`, `services_offered` with `interface:` + `sla:` and NO `entry_point`, `pooled_sub_systems[{name,target_pool,sub_system,note}]` as a **list of maps**, `sub_systems[{name,kind,target,role,version}]`, `pool.ad_prefix`, `pool.description` as a `>` block scalar. |
| `financial-os__pool.yaml` | `cortex/meta-governance/archive/financial-os/_pool.yaml` | Parse-fail path (offered service has no `name`). `relationships.depends_on`, `accepts: []` empty flow list, `suffix: [.py, .sh]` unquoted. |
| `personal-os__pool.yaml` | `cortex/meta-governance/archive/personal-os/_pool.yaml` | Parse-fail path. `access: symlink`, `accepts: [path_ref or contact_id]` — an accepts entry containing a SPACE and `or`. |
| `scratch-test__pool.yaml` | `cortex/meta-governance/archive/scratch-test/_pool.yaml` | Minimal scaffold. `services_offered` **and** `services_consumed` present but each contains only YAML **comments** → both deserialize to `null`, not to an empty list. (scanned the whole tree: the only null-service manifests are `scratch-test`, `scratch-test2`, `scratch-test3`, and 2/3 differ only in their own name and AD prefix — so one fixture covers the shape.) |
| `personal-brand-tech__pool.yaml` | `graveyard/personal-brand-tech/_pool.yaml` | `compass_graduate` top-level block (a dialect the `_org.yaml` files spell as `pool.graduation`), flow-style `accepts:`/`returns:`, `note:` on an offered service, consumed edges WITH `access`. |
| `content__pool.yaml` | `graveyard/rel-intel-content-20260823-deprecated/_pool.yaml` | Sub-system: `pool.type: sub-system`, `pool.parent: relationship-intel`, `rules:` (not enforced by the linter), `required: true` dirs. |
| `revenue-engine__pool.yaml` | `graveyard/revenue-engine-pre-split/_pool.yaml` | No `services_offered` key at all (so not even an empty one), consumed edges with `access`, inline flow `sub_dirs` maps. |

## sha256

```
cca8aa01dc5a52f7a3a6e776ee317e440c06f16f0414fd65165394512cb3d1da  agent-fabric__org.yaml
34a075f7929bda1cbc07dd8e3957f8a1dc63f4e2cb9e79a61b25b50825956ce3  alignment-engine__org.yaml
6d8d63e26f2dc36b068de923f48955048eb760abbb84a7e364dc93c3947a146f  brand-integrating__org.yaml
004f7193593ccf388da663ca083f45454c73b237bfdfd51f845ecc1321a720a3  career-pipeline__org.yaml
cee655b2c5fa023cdf664429ef9f93f1af2ad685d38da353e9d654736a53e9b5  content__pool.yaml
5533907d23b41c5918fd150282a183773cdff4198bac318fe6c24ec8bf6d1724  contacts-dup-conflict__org.yaml
fbd66aa1db50a7d9557d576be34d0a7728a2bae316cc94d9449df3839141606c  financial-os__pool.yaml
7a08411d83d03e75ac3fe96b6ba5c8ceeb9b1c1ba5b6d9bc27b808fe7480fe7a  forge__org.yaml
6d663df4666f84f41702372bfeed6cec92562e177711193486ac33485b4b5411  graveyard__org.yaml
4ca6aafaa639b170fccdba57715d11f183a9cd56bc494a9aea46da4339f26f51  identity-core__org.yaml
6db98576864b19fd58c5abab1c4245b75502c1d78e020a85b6394e747c03a6d6  living-architecture__org.yaml
618792f729bff9211a5bbc5f53e1a375ca6c5fd4b5f741d98548173cc4812425  meta-governance__org.yaml
fb0db9c96a7f022065e10e904a478e8b5234d242eb97c20a8020345d29f0b59d  personal-brand-tech__pool.yaml
e8627b79f0c35caaca303446f3c923a453946b95b88affdc2aaf5efbae142a3b  personal-os__pool.yaml
191a4ddd55d720fd40736216a7ac65efdf9bc3feca4cd88b3df9fdc1f3bc8676  platform-infra__org.yaml
1f629eadf1bbf35ee5297798e2838d654528353b9e65111a97c7d96c55c3064a  presentation-engine__pool.yaml
967356512bd77c2d06d93179d5341959f008f8703695e262a33864fa8200da09  research-engine-20260907-retired__org.yaml
2ec25502e2f65c9d535b8662c9c29ff6a77d1ff620387e462c026c7189f7de0b  research-vault__org.yaml
fce0851db06b5e3dd3c8436d3125919d3a1e3ef2768b1ca96cafaed495ac8538  revenue-engine__pool.yaml
cb5860e5bb193aa34b50446179910ad82c294b6246e2ba88c04b61b27c42712a  scratch-test__pool.yaml
84404d1a5f643e42a4306e6edb4cfca78433a313787292db7074d14676474bcd  task-grid__org.yaml
8d4c61437965731196e2aa083c4f61591786809fdf2e89db808eb977e4c91135  workforce-ops__org.yaml
```

## Do not "clean these up"

If a fixture stops parsing, the bug is in the parser, not the file. These
bytes are the contract.

Every "Source path" and every "why it is here" claim above was checked by
loading the YAML and inspecting the actual types, not by reading the file
with the eye. Three rounds of correction happened before this README was
committed:

1. All four second-harvest source paths were wrong on the first draft —
   each file turned out to live under a `sub-systems/` parent rather than
   at the top level of its domain.
2. Three of the four shape descriptions were wrong on the first draft. The
   draft claimed `alignment-engine` carried both sub-system dialects and
   that `forge` had a bare-string offered list; neither is true. Both files
   have **no** `pooled_sub_systems` or `sub_systems` key at all, and the
   bare-string offered list is `alignment-engine`, not `forge`.
3. `living-architecture` was first claimed to carry an `interfaces:` block
   it does not have, and `contacts` was first claimed to carry duplicate
   keys at the top level; it carries none. Its only duplicate is nested at
   `directories.core/.required`.

Re-verify by hash-matching the fixture against the tree, then asserting the
type with a parser, rather than trusting either the path or the prose.
