# Organism Artifact Reference

**Read-only evidence harvest. Not a design document.**

Every claim below was measured against `~/.hermes/organism/` on **2026-09-30** and
carries a `path:line` citation or a verbatim quote. Where something could not be
determined it is marked **UNVERIFIED** rather than guessed.

**Files read in full or parsed programmatically:**

| Artifact class | Files discovered | Files parsed |
|---|---:|---:|
| `_org.yaml` | 94 | 94 (0 parse errors) |
| `smoke.yaml` | 49 | 49 (0 parse errors) |
| AD records (`*/core/decisions/*.md`) | 266 | 266 frontmatter blocks parsed |
| RG records (`*/core/regressions/*.md`) | 261 | 261 frontmatter blocks parsed |
| `employees.json` | 2 | 2 (231 employee records) |
| notice-board JSONL | 2 non-empty | 2,023 records |
| worklog JSONL | 31 files | 1,127 records |
| Python source (`lib/cli.py`, `lib/engine.py`, `bin/arch`, `bin/hr`, `axe_lib.py`, `axe_smoke.py`, `agent_fabric.py`, `hr_lib.py`) | 8 | read at cited line ranges |

**How to reproduce:** every table below is the output of a `yaml.safe_load` +
key-presence walk, or a `json.loads` + key-union walk, over the full file set.
Counts are exact, not sampled.

---

## 0. Tree map (measured)

```
~/.hermes/organism/
├── _org.yaml                     # ORG-GLOBAL, NOT a pool manifest
├── archive/
├── cortex/           2 pools
├── foundations/      45 pools/sub-systems
├── graveyard/         7 pools/sub-systems
├── swarm/             3 pools
├── ventures/         36 pools/sub-systems
```

`_org.yaml` count by first path segment: root 1, `cortex` 2, `foundations` 45,
`graveyard` 7, `swarm` 3, `ventures` 36. **Total 94.**

**The root `_org.yaml` is not a pool manifest.** It declares strata, teams, and
the structural allowlist — it has no `pool:` block:

`~/.hermes/organism/_org.yaml:24-32` (verbatim):
```yaml
structure:
  strata: [cortex, swarm, foundations, ventures]
  allowed_top_level: [cortex, swarm, foundations, ventures, archive, graveyard, design-aesthetics]
  # design-aesthetics = known husk (cron-agent orphan), still orphan-linted
  orphan_policy:
    husk_no_config: WARN
    legacy_duplicate_subtree: WARN
    registry_no_disk: WARN
    sidecar_unreachable: WARN
```

It also declares the team graph at `_org.yaml:9-19`:
```yaml
teams:
  revenue-ops:
    team_id: revenue-ops
    mission: "Cross-department revenue operations: job pipeline + freelance scouting share signal and learnings"
    members:
      - emp-4f6e7073860f
      - emp-c194c2788b19
    lead: emp-4f6e7073860f
    departments: [career-pipeline, freelance-agency]
    cross_dept: true
    status: active
```

---

# 1. `_org.yaml` — schema and variance

## 1.1 Top-level key union (N = 94 files)

| Key | Type | Presence | Always / sometimes / never |
|---|---|---|---|
| `pool` | map | 93/94 | **sometimes** (omitted by the root file only) |
| `directories` | map | 93/94 | **sometimes** (omitted by the root file only) |
| `required_files` | list | 81/94 | sometimes |
| `forbidden` | list | 72/94 | sometimes |
| `services_offered` | list | 29/94 | **sparse** |
| `services_consumed` | list | 29/94 | **sparse** |
| `department` | map | 24/94 | **sparse** |
| `relationships` | map | 23/94 | **sparse** |
| `rules` | list-or-map | 11/94 | rare |
| `surface_registry` | — | 9/94 | rare, ventures-only |
| `defaults` | map | 1/94 | root only |
| `teams` | map | 1/94 | root only |
| `structure` | map | 1/94 | root only |

**No top-level key is `always` present** unless you exclude the root file. Every
implementer should treat all of these as optional with defaults.

**`surface_registry`** appears in exactly 9 files, all `ventures/`:
`architectural-leads`, `brand-integrating`, `brand-polarizing`, `career-pipeline`,
`community-loo`, `credibility-funnel`, `freelance-agency`, `quant-trading`,
`saas-studio`.

**`rules` is polymorphic.** It is a list in some files (mixing bare strings and
single-key maps) and its element type is not stable. Verbatim from
`foundations/content-campaigns/_org.yaml`:
```yaml
rules:
- "CLI commands must include --reason flag on all mutations (AD-032)"
- "No external dependencies — all data local (RG-020)"
- "System is git-tracked (RG-021)"
- Content pipeline uses FSM validation: idea→brief→draft→review→scheduled→published→measured→archived
- Campaign lifecycle uses FSM: planning→targeted→engaged→converted→archived
- "Each campaign and content piece must declare a surface": "technology|family-support|wealth|community|consciousness"
```

## 1.2 `pool` block (N = 93 files that have one)

| Key | Type(s) | Presence | Notes |
|---|---|---|---|
| `name` | str | 93/93 | **the only truly universal pool key** |
| `category` | str | 93/93 | 21 distinct values |
| `parent` | str \| null | 92/93 | 23 distinct values + `null` |
| `type` | str | 91/93 | 6 distinct values |
| `role` | str | 91/93 | 6 distinct values |
| `domain` | str | 91/93 | |
| `_legacy_tier` | str \| null | 86/93 | **migration shadow field** |
| `_legacy_type` | str \| null | 86/93 | **migration shadow field** |
| `_legacy_category` | str | 86/93 | **migration shadow field** |
| `genome_layer` | str | 50/93 | |
| `ad_scope` | str | 48/93 | |
| `tier` | str | 28/93 | **only 30% — see §1.7** |
| `description` | str | 11/93 | |
| `graduation` | map | 9/93 | ventures only |
| `status` | str | 7/93 | |
| `layer` | str | 5/93 | |
| `prefix` | str | 2/93 | |
| `ad_prefix` | str | 1/93 | |
| `created` | date | 1/93 | YAML date scalar |

**Verdict on the user's hypothesis:** `role` and `domain` are *widely* but **not
universally** present (91/93). `tier` is present in only **28/93** — the
`swarm/task-grid` sample that shows `tier` alongside `role`/`domain` is
representative of only 30% of pools. `type` and `category` are near-universal.
`ad_scope` is under half.

### Distinct values

`pool.tier` — 8 values:
`compass-graduate` (9), `foundational` (9), `infrastructure` (1), `meta-system`
(1), `native-organ` (3), `project` (1), `sub-system` (1), `swarm` (3).

`pool.type` — 6 values: `sub-system` (67), `system` (21), `foundational` (1),
`meta-system` (1), `project` (1).

`pool.role` — 6 values: `foundational`, `meta-system`, `research-venture`,
`sub-system`, `swarm`, `venture`.

`pool.category` — 21 values: `alignment`, `bridge`, `communication`,
`content-campaigns`, `data-intelligence`, `delivery`, `execution`,
`facilitation`, `financial-intelligence`, `identity`, `infrastructure`,
`live-ops`, `observation`, `publishing`, `rd`, `relationship-intelligence`,
`research`, `research-intelligence`, `strategy`, `task-execution`,
`website-development`.

`pool.ad_scope` — 33 distinct values including the degenerate literal
`presentation` (not an AD id): `AD-021`, `AF-AD-001`, `AXE-AD-001`, `CAM-AD-001`,
`CAM-AD-002`, `CP-AD-001`, `CP-AD-003`, `CP-AD-008`, `CP-AD-016`, `CP-AD-024`,
`DA-AD-031`, `DA-AD-046`, `FE-AD-001`, `HI-AD-038`, `HI-AD-040`, `HR-AD-001`,
`ID-AD-001`, `ID-AD-002`, `ID-AD-004`, `JE-AD-001`, `KOSMOS-AD-001`, `LB-AD-001`,
`LO-AD-001`, `MD-AD-001`, `PBI-AD-001`, `PBP-AD-001`, `QF-AD-001`, `RE-AD-001`,
`RI-AD-035`, `SE-AD-001`, `TA-AD-001`, `WD-AD-001`, `presentation`.

**Counterexample — `ad_scope` is typed `str` but carries two different shapes.**
Most are AD ids; `foundations/presentation-engine/_org.yaml` uses the bare word
`presentation`. There is no validator visible in `_org.yaml` that constrains it.

**Counterexample — `tier` and `type` disagree.** `pool.tier` has 8 values while
`pool.type` has 6, and they are not in bijection. e.g. `pool.tier: project`
coexists with `pool.type: project`; `pool.tier: compass-graduate` coexists with
`pool.type: project`. The `_legacy_tier` / `_legacy_type` / `_legacy_category`
trio in 86/93 files is a **partially-completed migration**: the new canonical
axis is `role` + `domain`, and the old ones are being nulled out. An implementer
must read `role` first and treat `tier` as historical.

## 1.3 `services_offered[]` (29/94 files declare it, 67 items)

| Key | Type | Presence across items | Files |
|---|---|---|---|
| `id` | str | 67/67 map items | 25/29 |
| `entry_point` | str | 63/67 | 24/29 |
| `accepts` | list | 63/67 | 24/29 |
| `returns` | list \| **str** | 62/67 | 23/29 |
| `purpose` | str | 14/67 | 4/29 |
| `sub_system` | str | 12/67 | 6/29 |
| `outputs` | list | 1/67 | 1/29 |

**Type counterexample:** `returns` is `list` in most items but **`str`** in at
least one. `returns` is therefore not a reliable list.

**Shape counterexample — `services_offered` is not always a list of maps.** 12
items across 4 `content-campaigns` sub-systems are bare **strings**:
```
foundations/content-campaigns/sub-systems/alignment-engine/_org.yaml[0..2]  -> str
foundations/content-campaigns/sub-systems/content-seeding/_org.yaml[0..2]   -> str
foundations/content-campaigns/sub-systems/feedback-engine/_org.yaml[0..2]   -> str
foundations/content-campaigns/sub-systems/research-protocols/_org.yaml[0..2] -> str
```

Key-set fingerprints (67 items):
```
37 x  {id, entry_point, accepts, returns}
13 x  {id, entry_point, accepts, purpose, returns}
10 x  {id, entry_point, accepts, returns, sub_system}
 4 x  {id}                                    <- bare stubs, no contract at all
 1 x  {id, entry_point, accepts, outputs, returns}
 1 x  {id, entry_point, accepts, purpose, returns, sub_system}
 1 x  {id, entry_point, accepts, sub_system}   <- swarm/task-grid, no `returns`
```

**Direct answer to the question asked:** `entry_point`, `accepts`, `returns` are
**not reliably present** — each is missing from 4–5 of 67 items, and 4 items have
only `id`. `sub_system` is present on only 12/67. Real service ids carry no verb
prefix (e.g. `identity`, `values`, `logbook`, `employee-telemetry`).

`services_offered[].sub_system` distinct values (10): `(root)` (3),
`alignment`, `applications`, `contacts`, `delivery`, `forecast`, `forge`,
`grounding`, `strategy`, `vps-operations`.

## 1.4 `services_consumed[]` (29/94 files, 69 items)

| Key | Type | Presence | Files |
|---|---|---|---|
| `id` | str | 54/69 | 22/29 |
| `provider` | str | 54/69 | 22/29 |
| `purpose` | str | 51/69 | 21/29 |
| `access` | str | 51/69 | 21/29 |
| `entry_point` | str | 2/69 | 2/29 |
| `root_holon`, `program`, `engine_namespace`, `queue_dir`, `skill`, `shared_maintenance_cron` | str | 2/69 | 2/29 |
| `crons_registered` | list | 2/69 | 2/29 |
| `sub_holons` | **int \| list** | 2/69 | 2/29 |
| 18 other one-off keys (`research_protocols`, `compass_alignment`, `feedback_engine`, `identity`, `kosmos`, …) | str | 1–2/69 | 1–2/29 |

**`access` is NOT universal** — 51/69 items, 21/29 files. 15 items omit it.

`services_consumed[].access` — exactly 5 distinct values:
`cli` (26), `symlink` (11), `import` (8), `symlink+cli` (4), `file` (2).
**`access: read` does not exist in the organism.** The read/write distinction the
outline proposes is not modelled here at all.

`services_consumed[].provider` — 14 distinct: `identity-core` (11),
`platform-infra` (10), `strategy-incubator` (7), `flight-ledger` (5),
`research-vault` (5), `meta-governance` (4), `content-campaigns` (3),
`relationship-crm` (2), `task-grid` (2), `career-pipeline` (1),
`freelance-agency` (1), `hermes-agent-infra` (1), `logbook` (1), `saas-studio` (1).

**Shape counterexample — `services_consumed` is also not always maps.**
`foundations/strategy-incubator/sub-systems/forge/_org.yaml[0..1]` are bare
strings.

**Type counterexample:** `sub_holons` is `int` in one file and `list` in another.

**The key-as-name anti-pattern.** The 4 `content-campaigns` sub-systems use
`services_consumed` as a flat `{ <service-id>: "<purpose string>" }` map, e.g.
`- research_protocols: "..."`, `- compass_alignment: "..."`. A parser that reads
`services_consumed` as a list of edges will silently produce zero edges for these
4 files.

Fingerprints:
```
49 x  {id, provider, purpose, access}
 3 x  {id, provider}
 2 x  {<bare key>}
 1 x  15-key holonic record (ventures/credibility-funnel[3])
 1 x  12-key holonic record (ventures/vitality-research[0])
```

## 1.5 `relationships` (23/94 files, mapping in all 23)

| Key | Type | Presence |
|---|---|---|
| `depends_on` | list | 23/23 |
| `loops` | list | **4/23** |

The user-confirmed `relationships: {depends_on: [], loops: []}` block is real
(`swarm/task-grid/_org.yaml:25-29`) but **`loops` is present in only 4 of the 23
files that have `relationships` at all**. It is optional, not structural.

## 1.6 `department` (24/94 files)

| Key | Type | Presence |
|---|---|---|
| `charter` | str | 24/24 |
| `class` | str | 24/24 |
| `reframe` | str | 1/24 |

`department.class` values: `staffed` (21), `utility` (2), `active` (1).
**There is no `unstaffed` value** — the root `_org.yaml:6-7` sets
`defaults.default_class: staffed`.

## 1.7 `directories` (93/94 files, 563 directory rules, all maps)

Rule-key frequency across 563 dir entries:

| Rule key | Type | Dir entries |
|---|---|---|
| `required` | bool | 490 |
| `suffix` | str \| list | 286 |
| `allow_any` | bool | 236 |
| `sub_dirs` | map | 75 |
| `description` | str | 66 |
| `pattern` | str | 42 |
| `shebang` | bool | 17 |
| `purposes` | list | 10 |
| `frontmatter_required` | list | 2 |
| `allow_root_files` | list | 1 |
| `prefix` | str | 1 |

**Type counterexample — `suffix` is `str` or `list`.** `swarm/task-grid/_org.yaml:37-62`
uses both in one file:
```yaml
  core/:
    suffix: .py          # str
  scripts/:
    suffix:
    - .py                # list
    - .sh
```

`allow_root_files` appears exactly once, as a **list** (not bool).

Directory names — 61 distinct, top by frequency:
`data/` 83, `core/` 79, `references/` 66, `scripts/` 59, `cron-jobs/` 51,
`sub-systems/` 41, `skills/` 37, `archive/` 26, `bin/` 25, `tests/` 11, `lib/` 10,
`kb/` 10, `docs/` 7, `templates/` 4, `mcp/` 4, `ontology/` 2, `systemd/` 2,
`config/` 2, `ea-source/` 2. Everything else is a bespoke pool-specific name
(`Emergence/`, `engines/`, `pipelines/`, `hpo/`, `parity/`, `vps/`, …).

There is also a directory literally named `data//` (double trailing slash) in 83
files, and one named `./` — **directory keys are not normalized**. See §1.9.

## 1.8 `required_files` and `forbidden`

`required_files` — present in 81/94, always a `list`. Only 5 distinct values:
```
65 x  [AGENTS.md]
11 x  [AGENTS.md, STATUS.md]
 2 x  [AGENTS.md, smoke.yaml]
 2 x  [_org.yaml]
 1 x  [AGENTS.md, AGENTS.md]      <- duplicate, a real defect in one pool
```

`forbidden` — present in 72/94, always a `list` of `str`. **Every single one of
the 72 files has the identical value** `[__pycache__/, *.pyc]`. Zero variance.

## 1.9 Counterexamples that violate the pattern

1. **Root `_org.yaml` is structurally different** — it has no `pool:`, no
   `directories:`, and instead has `defaults` / `teams` / `structure`. Any loader
   that assumes every `_org.yaml` has a `pool.name` will fail on it.
2. **`directories` keys are unnormalized** — `data//` (83 files), `./` (1 file),
   `core/decisions//` (1 file, flattened path used *instead of* `sub_dirs`).
3. **`rules` element type is unstable** — bare string vs single-key map, mixed in
   the same list.
4. **`services_offered` / `services_consumed` items are sometimes bare strings**
   (12 + 2 items).
5. **`suffix`, `returns`, `sub_holons` are polymorphically typed.**
6. **`required_files` contains a duplicate** in one pool: `[AGENTS.md, AGENTS.md]`.
7. **`services_consumed` is used as a name→string map** in 4 content-campaigns
   sub-systems.
8. **`_org.yaml` in `ventures/quant-trading/archive/pre-pioneer-pool-2026-09-14/`**
   — archived pools still carry live-shaped manifests, so a recursive walker will
   pick up 3 dead pools as live.

---

# 2. `smoke.yaml` — the fail-closed gate schema

## 2.1 Shape

**49 files** exist (4 in `archive/`, 45 live). **All 49 are a mapping.** Root key
union:

| Key | Type | Presence |
|---|---|---|
| `checks` | list | **49/49 — always** |
| `version` | int | 41/49 |

`version` is `1` in every file that has it. **8 files omit `version` entirely**
despite the gate docstring calling it part of the contract:
`foundations/identity-core/sub-systems/{platforms,professional,subjective}`,
`foundations/presentation-engine`, `swarm/agent-fabric`, `swarm/workforce-ops`,
`ventures/credibility-funnel/sub-systems/{proof-architecture,topic-anchors}`.

## 2.2 Check-item field union (185 items across 49 files, 0 non-map items)

| Key | Type | Presence | Notes |
|---|---|---|---|
| `name` | str | **185/185 — always** | |
| `cmd` | str | **185/185 — always** | |
| `reason` | str | **181/185** | 4 items omit it — see counterexample below |
| `critical` | bool | 119/185 | 66 items omit it; **default `true`** at runtime |
| `timeout` | int | 94/185 | |
| `expect_exit` | int | 4/185 | **DEAD FIELD — see §2.5** |
| `expect_exit_codes` | list | 1/185 | the only field the runner actually reads |

**The timeout field is called `timeout`**, an `int` in seconds. Runtime default
is 30 s (`axe_smoke.py:186` — `int(c.get("timeout", DEFAULT_TIMEOUT))`).
Observed values: `30` (35×), `15` (19×), `10` (12×), `60` (8×), `5` (6×), `20`
(4×), `120` (4×), `180` (3×), `300` (1×), `90` (1×), `700` (1×). 700 s is real —
`swarm/task-grid/smoke.yaml` gives the board-guard check 700 s because the suite
runs 300–600 s under cron load.

Fingerprints:
```
91 x  {name, cmd, reason, critical, timeout}
59 x  {name, cmd, reason}                  <- no critical, no timeout
27 x  {name, cmd, reason, critical}         <- no timeout
 4 x  {name, cmd, expect_exit}             <- no reason, no critical
 3 x  {name, cmd, reason, timeout}
 1 x  {name, cmd, reason, critical, expect_exit_codes}
```

## 2.3 How `critical` is expressed

**A YAML boolean, `critical: true` / `critical: false`.** Not a string, not a
level. Measured distribution: `true` 102, `false` 17, **absent 66** (and the
runner defaults absent to `true`).

Runtime semantics, `~/.hermes/scripts/axe_smoke.py:187`:
```python
"critical": bool(c.get("critical", True)),
```

## 2.4 Does `reason` exist, and what does it hold? — YES, load-bearing

`reason` is present on **181/185 checks across all 49/49 files**, and holds a
free-text string stating **what breaks if this check is allowed to fail**, very
often citing the AD/RG that mandates it. 151 distinct values. Examples:

- `"syntax gate — every script must parse before the tick relies on it"` (×17)
- `"arch is the ONLY AD/RG lifecycle entry point (AD-051) — a broken arch blocks all governance"`
- `"engine_lib is the shared FSM library; if it can't be imported, all 12 engine scripts fail"`
- `"AXE-AD-012 §3: every cron-employee must carry employee, agent_type, continuity, registers_on_spawn"`
- `"ID-RG-001 — no rel-intel coupling"`
- `"org roster drift (orphan employees / jobs without employees / hollow teams) blocks the tick — AXE-RG-012/013 fail-closed"`

The mandate is in `swarm/task-grid/core/decisions/AXE-AD-010-axe-ad-010-fail-closed-operational-smoke-gate.md:26-30` (verbatim):
> Every pool with a CLI surface declares a **smoke.yaml** (version 1) listing
> named checks: `cmd` (exit 0 = pass), `critical` (default true),
> `timeout`, and `reason`. The AXE tick runs the gate (`axe smoke`) before
> spawning ANY agent

The `reason` is not decoration in practice: it carries the guard's identity
(`AXE-RG-037`, `AXE-RG-039`, `AXE-RG-038`) and the remediation context. The linter
does **not** enforce `reason` — see §2.5.

## 2.5 Counterexample: the `reason` counterexample, and a dead field

**`foundations/presentation-engine/smoke.yaml` has 4 checks, all with
`{name, cmd, expect_exit: 0}` and NO `reason` at all** (verbatim):
```yaml
checks:
- name: pe CLI on PATH
  cmd: pe doctor
  expect_exit: 0
- name: archify binary present
  cmd: test -x "$HOME/.agents/skills/archify/bin/archify.mjs"
  expect_exit: 0
- name: arch registry readable
  cmd: test -r "$HOME/.hermes/organism/cortex/meta-governance/data/architecture-registry.yaml"
  expect_exit: 0
- name: relations.yaml readable
  cmd: test -r "$HOME/.hermes/organism/cortex/meta-governance/data/relations.yaml"
  expect_exit: 0
```
**`expect_exit` is a DEAD FIELD.** The runner only ever reads
`expect_exit_codes` (`axe_smoke.py:190` and `:369`):
```python
_eec = c.get("expect_exit_codes")
if _eec is not None:
    result["passed"] = (not timed_out) and (rc in _eec)
else:
    result["passed"] = (rc == 0) and not timed_out
```
`grep -n "expect_exit\b" axe_smoke.py` returns **zero** hits. The 4
`expect_exit: 0` checks are therefore evaluated as plain `rc == 0` — they happen
to agree, but the field is inert. The 1 real `expect_exit_codes` user is
`foundations/strategy-incubator/sub-systems/forge/smoke.yaml[3]`.

## 2.6 The linter's canonical schema (MD-RG-001) — and its blind spot

`cortex/meta-governance/lib/engine.py:2997-3047`, the docstring verbatim:
> **MD-RG-001: smoke.yaml MUST use the canonical schema
> (version: 1 + checks: list of {name, cmd, reason, critical?}).
> Non-canonical manifests pass 0/0 against the gate while executing ZERO
> checks — the pool is operationally blind. Fail-closed.**

But the **code** only enforces `name` (or `id`) and `cmd` (or `run`):
`engine.py:3038-3045`:
```python
name_val = chk.get("name") or chk.get("id")
cmd_val = chk.get("cmd") or chk.get("run")
if not name_val or not cmd_val:
    missing = []
    if not name_val: missing.append("name (or id)")
    if not cmd_val: missing.append("cmd (or run)")
```
`reason`, `critical`, and `timeout` are **not** linted. So the
`presentation-engine` file with 4 `reason`-less checks passes the linter. Also
note the aliases `id`/`run` are accepted by the linter but never emitted by any
real manifest.

## 2.7 Exit-code contract (the part that actually blocks)

`~/.hermes/scripts/axe_smoke.py:22-32` (verbatim):
> ```
> Exit codes (for the tick gate):
>   0  all checks passed (or only non-critical failures)
>   2  one or more CRITICAL checks failed → DO NOT SPAWN AGENTS
> ```

`axe_smoke.py:978` (verbatim):
```python
    # Exit 2 = critical failure → blocks the tick. 0 = pass or non-critical only.
    return 2 if gate.get("blocked") else 0
```
`blocked` is `len(critical_failed) > 0` (`axe_smoke.py:703`). The outline's
"exit 2 blocks the tick" claim is **verified**.

## 2.8 Three full verbatim smoke.yaml examples

### Example A — `swarm/task-grid/smoke.yaml` (11 checks, all `critical: true`)

```yaml
version: 1
checks:
- name: axe agent list works
  cmd: axe agent list --stale >/dev/null 2>&1
  timeout: 30
  critical: true
  reason: 'agent introspection is the swarm''s eyes (AXE-AD-010; regression: AttributeError on list fixed 2026-08-18)'
- name: axe notice list works
  cmd: axe notice list --limit 5 >/dev/null 2>&1
  timeout: 30
  critical: true
  reason: notice board is the agent messaging backbone
- name: axe task list works
  cmd: axe task list --status ready --limit 5 >/dev/null 2>&1
  timeout: 30
  critical: true
  reason: task lifecycle is the swarm's work queue
- name: axe agent register positional + --name alias
  cmd: axe agent register smoke-check --caps test --status active >/dev/null 2>&1 && axe agent deregister --name smoke-check >/dev/null 2>&1
  timeout: 30
  critical: true
  reason: 'both arg forms must resolve (regression: --name alias crash fixed 2026-08-18)'
- name: axe agent heartbeat both forms
  cmd: axe agent register smoke-hb --caps test >/dev/null 2>&1 && axe agent heartbeat --name smoke-hb --status idle >/dev/null 2>&1 && axe agent deregister smoke-hb >/dev/null 2>&1
  timeout: 30
  critical: true
  reason: heartbeat is the liveness signal the tick depends on
- name: axe_smoke self-check parses
  cmd: python3 -c "import sys, pathlib; sys.path.insert(0, str(pathlib.Path.home() / '.hermes/scripts')); import axe_smoke; assert axe_smoke.discover_manifests(); print('ok')" >/dev/null 2>&1
  timeout: 30
  critical: true
  reason: the gate must be able to gate itself
- name: unit tests pass
  cmd: cd ~/.hermes && python3 -m pytest organism/swarm/task-grid/tests/ -q >/dev/null 2>&1
  timeout: 120
  critical: true
  reason: regression suite must stay green (37 tests added 2026-09-09)
- name: unit tests never touch production board (AXE-RG-037)
  cmd: python3 ~/.hermes/organism/swarm/task-grid/scripts/smoke_board_guard.py
  timeout: 700
  critical: true
  reason: "AXE-RG-037 (AXE-AD-027 P0): conftest sandboxes AXE_DATA_DIR but the gate must PROVE it — checksum the notice board around the suite; any drift fails the tick (2026-09: ~640 fake tester notices leaked into production). timeout 700s because the suite runs 300-600s under cron-load spikes."
- name: status taxonomy guard (AXE-RG-039)
  cmd: python3 ~/.hermes/organism/swarm/task-grid/scripts/smoke_status_guard.py >/dev/null 2>&1
  timeout: 30
  critical: true
  reason: "AXE-RG-039 (AXE-AD-027 P3): 'completed' status drift + untyped blocked tasks — 7 migrated + 13 backfilled 2026-09-27; guard fails closed on recurrence so queries/recovery never miss rows."
- name: run-log secrecy audit (AXE-RG-038)
  cmd: python3 ~/.hermes/organism/swarm/task-grid/scripts/smoke_secrecy_audit.py
  timeout: 120
  critical: true
  reason: "AXE-RG-038 (AXE-AD-027 P2): run logs + state are read by every spawned agent; any secret-shaped span fails the tick. Writers scrub at write-time; this audit catches pre-existing or new leaks (2026-09-12: GOG_KEYRING_PASSWORD in a run log, redacted 2026-09-27)."
- name: axe org check clean
  cmd: set -o pipefail; axe org check 2>&1 | tail -5 || (axe org dedupe --apply >/dev/null 2>&1 && axe org check 2>&1 | tail -5)
  timeout: 60
  critical: true
  reason: "org roster drift (orphan employees / jobs without employees / hollow teams) blocks the tick — AXE-RG-012/013 fail-closed (2026-08-25: output kept in failure report so root cause is visible; timeout 30→60s because the check scans 19 pools + jobs.json and timed out under cron-load spikes)"
```

### Example B — `foundations/identity-core/smoke.yaml` (7 checks, **no `timeout` at all**)

```yaml
version: 1
checks:
- name: aspects-parse
  cmd: python3 -c "import yaml,glob; fs=glob.glob('$HOME/.hermes/organism/foundations/identity-core/sub-systems/profile-core/data/personal/aspects/*.yaml');
    assert len(fs)>=4; [yaml.safe_load(open(f)) for f in fs]"
  critical: true
  reason: "ID-AD-004 D1 — fragmented aspect store parses (>=4 aspects)"
- name: dialectic-present
  cmd: python3 -c "import yaml; d=yaml.safe_load(open('$HOME/.hermes/organism/foundations/identity-core/sub-systems/profile-core/data/personal/aspects/dialectic.yaml'));
    dia=d['dialectical']; assert isinstance(dia.get('thesis'),list) and dia['thesis']
    and isinstance(dia.get('antithesis'),list) and dia['antithesis']"
  critical: true
  reason: thesis/antithesis are the guard foundation (ID-AD-003)
- name: cli-live
  cmd: identity get >/dev/null
  critical: true
  reason: sole mutation surface must resolve (agentic-loop write path)
- name: subjective-engine-imports
  cmd: python3 -c "import sys; sys.path.insert(0,'$HOME/.hermes/organism/foundations/identity-core/sub-systems/subjective/core');
    import diagnosis, alignment, healing"
  critical: false
  reason: self-diagnostics pipeline loadable
- name: orthogonality-lint
  cmd: '! grep -rn ''import relationship'' $HOME/.hermes/organism/foundations/identity-core/sub-systems/subjective/core/'
  critical: true
  reason: "ID-RG-001 — no rel-intel coupling"
- name: rule7-bypass-guarded-bare-spelling
  cmd: '! identity set --path values.creativity --value 9.9 --reason smoke >/dev/null
    2>&1'
  critical: true
  reason: "ID-RG-002 — dialectical sections reject direct set"
- name: rule7-bypass-guarded-alias-spelling
  cmd: '! identity set --path identity.values.creativity --value 9.9 --reason smoke
    >/dev/null 2>&1'
  critical: true
  reason: "ID-RG-002 — alias prefix must not traverse into dialectical sections"
```
Note: `cmd` here is a **YAML folded multi-line scalar**, and `!` negation is used
as a shell idiom. `identity-core` also **demonstrates the `--reason` mandate
operating inside a smoke check**.

### Example C — `graveyard/research-engine-20260907-retired/smoke.yaml` (graveyard stratum, 2-space-indented list style)

```yaml
version: 1
checks:
  - name: "CLI parses"
    cmd: "python3 -m py_compile ~/.hermes/organism/graveyard/research-engine-20260907-retired/bin/research"
    timeout: 30
    critical: true
    reason: "the CLI is the research engine operational surface"
  - name: "emerge.py parses"
    cmd: "python3 -m py_compile ~/.hermes/organism/graveyard/research-engine-20260907-retired/emerge.py"
    timeout: 30
    critical: true
    reason: "emergence detection is the genome scalability mechanism"
  - name: "domain_manager.py parses"
    cmd: "python3 -m py_compile ~/.hermes/organism/graveyard/research-engine-20260907-retired/domain_manager.py"
    timeout: 30
    critical: true
    reason: "domain management is the genome scaffolding mechanism"
  - name: "KB has items"
    cmd: "test $(find ~/.hermes/organism/graveyard/research-engine-20260907-retired/sub-systems -path */kb/*.md 2>/dev/null | wc -l) -gt 100"
    timeout: 30
    critical: false
    reason: "KB should have substantial research items"
  - name: "At least 2 sub-systems"
    cmd: "test $(ls -d ~/.hermes/organism/graveyard/research-engine-20260907-retired/sub-systems/*/ 2>/dev/null | wc -l) -ge 2"
    timeout: 30
    critical: true
    reason: "research engine needs multiple domains to function"
```
**A retired pool still carries a live-shaped gate** — a recursive walker will run
graveyard gates. The only other graveyard gate files are the 3 sub-systems.

---

# 3. Decision-record (AD) and regression-guard (RG) lifecycle

## 3.1 Real record locations and the cited IDs — all 7 VERIFIED

All 7 IDs cited by the plan exist, all in
`cortex/meta-governance/core/decisions/`:

| Cited | Real path |
|---|---|
| AD-014 | `cortex/meta-governance/core/decisions/AD-014-system-pool-architecture.md` |
| AD-022 | `cortex/meta-governance/core/decisions/AD-022-pool-linter-pool-yaml-per-system-ad-rg.md` |
| AD-032 | `cortex/meta-governance/core/decisions/AD-032-universal-reason-flag-mandate.md` |
| AD-060 | `cortex/meta-governance/core/decisions/AD-060-compass-graduate-sub-system-pooling-from-foundatio.md` |
| AD-070 | `cortex/meta-governance/core/decisions/AD-070-operational-onboarding-contract.md` |
| AD-074 | `cortex/meta-governance/core/decisions/AD-074-ontology-refactor-3-tier-canonical-vocabulary.md` |
| AD-078 | `cortex/meta-governance/core/decisions/AD-078-per-department-maintenance-contract-maintenance-md.md` |

Records live in **42 `core/decisions/` directories** and **29
`core/regressions/` directories** (3 decisions dirs and 2 regressions dirs are
empty; 2 more live under `graveyard/`). Counts: **266 AD `.md`, 261 RG `.md`**
(265/260 if `archive/` is included; 259/256 if `_index.md` is excluded).

`cortex/meta-governance/core/decisions/` alone holds 78 records;
`core/regressions/` holds 49.

## 3.2 Filename convention and numbering

Pattern: `<PREFIX>-(AD|RG)-<NNN><-kebab-slug>.md`, e.g.
`AXE-AD-010-axe-ad-010-fail-closed-operational-smoke-gate.md`.

The bare `AD-` prefix (no pool scope) is used **only** by
`cortex/meta-governance`. AD filename prefixes observed (26): `AD`, `AF-AD`,
`AXE-AD`, `CAM-AD`, `CP-AD`, `DA-AD`, `FE-AD`, `HI-AD`, `HR-AD`, `ID-AD`, `JE-AD`,
`KOSMOS-AD`, `LB-AD`, `LO-AD`, `MD-AD`, `PBI-AD`, `PBP-AD`, `PBT-AD`, `PR-AD`,
`QF-AD`, `RE-AD`, `RI-AD`, `SE-AD`, `TA-AD`, `WD-AD`.

**Numbering is per-prefix, not global.** `AD-010` (meta-governance) and
`AXE-AD-010` (task-grid) are unrelated records. `arch new` allocates via
`get_next_id(pool, item_type, ads, rgs)` (`lib/cli.py:263`).

**Numbering has gaps.** `AD-078` exists but `swarm/task-grid/core/decisions/`
jumps `AXE-AD-021` → `AXE-AD-023` (no `AXE-AD-022`). The tombstone file records
this class of problem: `| CP-AD-054 | unresolvable legacy citation (pre-AD-031
numbering-gap era); no successor file exists anywhere; cited only by CP-AD-016
Related |`.

## 3.3 AD frontmatter — exact field set and variance (N = 266)

| Key | Type | Presence | Verdict |
|---|---|---|---|
| `Description` | str | 247/266 (93%) | effectively required |
| `Deciders` | str | 247/266 (93%) | effectively required |
| `Date` | str\|date | 244/266 (92%) | effectively required |
| `Title` | str | 242/266 (91%) | effectively required |
| `ID` | str | 241/266 (91%) | effectively required |
| `Status` | str | 240/266 (90%) | effectively required |
| `Related` | list | 92/266 (35%) | optional |
| `Supersedes` | str | 7/266 (3%) | lifecycle |
| `Superseded-By` | str | 1/266 (0.4%) | lifecycle — see §3.6 |
| `Execution_Status` | str | 4/266 | |
| `Recon` | str | 2/266 | |
| `Severity` | str | 1/266 | one AD carries it |
| `Tags` | str | 2/266 | |
| **lowercase variants** `id`,`status`,`title`,`date`,`deciders`,`description`,`related` | — | 10–17/266 each | **case is unstable** |
| other one-offs | — | 1–7 each | `tags`, `type`, `pool`, `author`, `created`, `verdict`, `loop`, `phase`, `item_id`, `outcome`, `signals`, `categories`, `severity`, `worklog_note`, `context_refs`, `system`, `count` |

**Top fingerprints:**
```
130 x  {ID, Title, Description, Status, Date, Deciders}
 73 x  {ID, Title, Description, Status, Date, Deciders, Related}
 14 x  {ID, Title, Description, Status, Date, Deciders, Related, tags}
  5 x  {ID, Title, Description, Status, Date, Deciders, Supersedes}
  5 x  {}                                  <- NO frontmatter at all
  4 x  {ID, Title, Description, Status, Date, Deciders, Execution_Status}
  3 x  {id, title, description, status, date, deciders, related, tags, verdict}  <- all-lowercase
```
There are **28 distinct key-set fingerprints** across 266 ADs.

`Status` — 7 distinct values: `"Active"` (157), `Active` (57),
`"Superseded"` (9), `"Accepted"` (9), `Superseded` (5), `Proposed` (2),
`"Active — partially superseded by PBP-AMD-002"` (1).
**The `arch list --status` filter documents choices `["Active", "Superseded",
"Draft"]` (`lib/cli.py:4325`) — `Accepted` and `Proposed`, the two values that
actually occur, are not in the filter's advertised choices.**

## 3.4 RG frontmatter — exact field set and variance (N = 261)

| Key | Type | Presence | Verdict |
|---|---|---|---|
| `Description` | str | 240/261 (92%) | effectively required |
| `Severity` | str | 238/261 (91%) | effectively required |
| `Date` | str\|date | 237/261 (91%) | effectively required |
| `Title` | str | 237/261 (91%) | effectively required |
| `ID` | str | 233/261 (89%) | effectively required |
| `Status` | str | 218/261 (84%) | required for ADs, **not** for RGs |
| `Related` | list | 25/261 (10%) | |
| `Deciders` | str | 17/261 (7%) | **not an RG field** — leaks from AD template |
| lowercase variants | — | 6–24/261 | case unstable |
| resolution fields | — | 2/261 | `Resolved`, `Resolved-In`, `Resolution` |
| provenance one-offs | — | 1 each | `Trigger`, `Reporter`, `Discovered-by`, `Guards`, `LastUpdated`, `related_ad`, `affected_pools`, `recon` |

Top fingerprints:
```
168 x  {ID, Title, Description, Severity, Date, Status}
 20 x  {ID, Title, Description, Severity, Date, Status, Related}
 12 x  {ID, Title, Description, Severity, Date, Status, Deciders}
 10 x  {ID, Title, Description, Severity, Date}          <- no Status
  9 x  {ID, Title, Description, Severity, Date, status}   <- BOTH cases at once
  8 x  {}                                                  <- no frontmatter
```
**The 9 files with both `Status` and `status` is a real defect class** — a
case-sensitive validator will see only one.

## 3.5 The `directories.core.decisions` / `regressions` convention — VERIFIED and WIDESPREAD

The block the user saw in `swarm/task-grid/_org.yaml:37-56` is **real and is the
dominant convention**: **122 `(file, subdir)` declarations across 61 of the 94
`_org.yaml` files** (63 files declare a `decisions/` directory at all, so 61/63
of pools that have one also declare the prefix).

Prefixes and the required-frontmatter lists are, with **2 exceptions**, exactly
as the user reported:

```yaml
      decisions/:
        suffix: .md
        prefix: AXE-AD-
        frontmatter_required:
        - Status
        - Date
        - Deciders
        - Description
        required: false
      regressions/:
        suffix: .md
        prefix: AXE-RG-
        frontmatter_required:
        - Severity
        - Date
        - Description
        required: false
```

**Distinct prefixes (43):** `AD-`, `RG-`, `AF-AD-`, `AF-RG-`, `AXE-AD-`,
`AXE-RG-`, `CAM-AD-`, `CAM-RG-`, `CP-AD-`, `CP-RG-`, `DA-AD-`, `DA-RG-`,
`FE-AD-`, `FE-RG-`, `HI-AD-`, `HI-RG-`, `HR-AD-`, `HR-RG-`, `ID-AD-`, `ID-RG-`,
`JE-AD-`, `JE-RG-`, `KOSMOS-AD-`, `KOSMOS-RG-`, `LB-AD-`, `LB-RG-`, `LO-AD-`,
`LO-RG-`, `MD-AD-`, `MD-RG-`, `PBI-AD-`, `PBI-RG-`, `PBP-AD-`, `PBP-RG-`,
`QF-AD-`, `QF-RG-`, `RE-AD-`, `RE-RG-`, `RI-AD-`, `RI-RG-`, `SE-AD-`, `SE-RG-`,
`TA-AD-`, `TA-RG-`, `WD-AD-`, `WD-RG-`.

**Prefix ≠ pool name (surprising, worth knowing).** `ventures/career-pipeline`
declares `JE-AD-` (job-engine), `ventures/credibility-funnel` declares `TA-AD-`,
`ventures/vitality-research` declares `MD-AD-`, `ventures/saas-studio` declares
`SE-AD-`, `foundations/research-vault` declares `KOSMOS-AD-`. A compiler that
derives the prefix from the pool slug will be wrong for at least 5 pools.

**Sub-systems inherit the parent pool's prefix** — e.g. all 8
`foundations/platform-infra/sub-systems/*` declare `HI-AD-` / `HI-RG-`.

**Exceptions (2 distinct `frontmatter_required` sets deviate):**
- `('Severity', 'Date')` — missing `Description`
- `('Status', 'Date', 'Deciders')` — missing `Description`
- `()` — empty list, no requirement at all

So the `frontmatter_required` list is **contractually declared but not
uniformly populated**, and (per §3.3/§3.4) the records themselves honour it only
~91–93% of the time anyway. `required: false` on both sub_dirs means a pool with
no `core/decisions/` is legal.

The same mechanism also governs cron jobs — 2 pools declare
`directories.cron-jobs.frontmatter_required: [cron_id, name, schedule, system,
status, description]` (`foundations/content-campaigns/_org.yaml:170`,
`ventures/architectural-leads/_org.yaml`).

## 3.6 Lifecycle verbs — read from the real argument parser

Entrypoint: `cortex/meta-governance/bin/arch` (23 lines), which does
`from cli import main` and `sys.exit(main())`. Its docstring, verbatim
(`bin/arch:2-12`):
> ```
> arch — Unified global system management CLI (AD-051, AD-059).
>
> Thin entry point. All logic lives in `lib/cli.py` (command surface) and
> `lib/engine.py` (system-integrity engine). See `lib/cli.py` for the full
> command reference.
>
> The ONLY entry point for AD/RG lifecycle. Agents must NOT manually create
> or edit AD/RG markdown files. All writes go through this CLI, which enforces
> recon-first workflow.
> ```

**Read commands** (`lib/cli.py:4322-4373`): `list` (`--pool`, `--status`,
`--type ad|rg`), `show <id>`, `search <query>`, `stats`, `gaps`, `related <id>`,
`worklog {list,append,split,new}`, `recon <scope>` (`--headings`).

**Write commands — ALL require `--recon`** (`lib/cli.py:4375-4412`):

| Verb | Required args | Optional |
|---|---|---|
| `new` | `--pool`, `--type ad\|rg`, `--title`, `--desc`, `--recon` | `--sub`, `--severity` (RG only), `--operator`, `--override-overlap` |
| `update <id>` | `--field key=value` (repeatable), `--reason`, `--recon` | `--summary`, `--operator` |
| `supersede <id>` | `--by`, `--reason`, `--recon` | `--operator` |
| `merge` | `--from`, `--into`, `--reason`, `--recon` | |
| `split <id>` | `--into`, `--reason`, `--recon` | |

`--severity` is documented as `High, Medium, Low, Critical` (`lib/cli.py:4382`).
`--field` help text: `"Field to update: key=value (repeatable)"`.

**Other subcommands present** (not AD/RG lifecycle but same CLI): `pool
{list,show,create,archive}`, `sub {list,show,deps,tree,create,pool}`,
`services {list,link,unlink}`, `graph`, `validate {--pool,--fix}`, `emit
{--agents,--agents-all,--registry,--living-arch,--tool-index,--cron,--references,--all}`,
`agents check-size`, `references {list,organize,emit}`, `clean
{superseded,trash}`, `cli {sync,check}`, `cron {list,show,sync}`, `staff
{list,show,audit,cleanup,deploy,retire,scale,runs,artifacts,review}`, `memory
{worklog,feed,context,ledger,trace,stats,validate,backfill,gc}`, `code-scan`,
`route`, `audit-ctx`, `guide`, `journal` (implied — **UNVERIFIED**, not located
in the 4590-line window I read), `--version`.

**`recon` is the enforced precondition.** `require_recon(args)` is called at the
top of every write command (e.g. `cli.py:1209`). `verify_recon(recon_id)` at
`cli.py:301`. `arch recon <scope>` scopes as `all`, `pool=<name>`,
`topic=<keyword>`, `id=<ID>`.

## 3.7 The `supersede` pointer — verbatim

`lib/cli.py:1220-1259` (verbatim):
```python
    # Update the superseded item
    filepath = Path(item['path'])
    text = filepath.read_text(encoding="utf-8")
    fm = parse_frontmatter(text)
    fm['Status'] = 'Superseded'
    fm['Superseded-By'] = args.by

    # Add Related entry if not present
    related = fm_get(fm, 'Related', 'related', default=[])
    if not isinstance(related, list):
        related = [related] if related else []
    related.append(f"{args.by} (superseded by)")
    fm['Related'] = related
```
```python
    # Add superseded notice at top of body
    body = f"\n**Status: Superseded by {args.by} on {datetime.now().strftime('%Y-%m-%d')}**\n\nReason: {args.reason or '(not specified)'}\n" + body
```
and the reverse pointer, `cli.py:1253`:
```python
            tgt_fm['Supersedes'] = item['id']
```

**So a supersede is a bidirectional, 3-part pointer:**
1. old record: `Status: Superseded` + `Superseded-By: <NEW-ID>` + a
   `Related` entry `"<NEW-ID> (superseded by)"`
2. old record **body**: a bolded line prepended:
   `**Status: Superseded by <NEW-ID> on <YYYY-MM-DD>**` then `Reason: <reason>`
3. new record: `Supersedes: <OLD-ID>`

Plus a **cascade warning** printed to stdout when any non-superseded record
still textually mentions the dead ID (`cli.py:1264-1276`) — a warning, not a
failure.

**The organism's compliance with its own pointer contract is weak.** Measured:
`Supersedes` appears in **7/266** ADs; `Superseded-By` in **1/266**;
`Status: Superseded` in **14/266**; `Status: Accepted` in **9/266**. The
`Supersedes`/`Superseded-By` mechanism is effectively unused; retirement is done
via the **tombstone file** instead (§3.8). The one real `Superseded-By` is
`PBP-AD-001` with `Status: "Active — partially superseded by PBP-AMD-002"`.

## 3.8 Tombstones — YES, they exist, in 3 places, with 2 different formats

### `graveyard/TOMBSTONES.md` (with frontmatter, the primary registry)

```markdown
---
id: "TOMBSTONES"
title: "Retired Pool & Record Tombstones"
description: "Registry of dissolved pools/records whose IDs still appear in Active citations. Validators resolve dangling references through this file. Each row: dead ID -> where its authority went."
status: active
date: 2026-08-23
---

# Tombstones — Retired Pools & Records

| Dead ID | Successor / disposition |
|---|---|
| `PO-AD-038` | personal-os dissolved 2026-08-21→23; trajectory/evidence sublimated into compass per `CP-AD-012`; identity foundation per `ID-AD-001` |
| `AD-068` | superseded by `KOSMOS-AD-001` (kosmos pool) — misfiled record re-homed, see RG-055 |
| `RG-053` | superseded by `KOSMOS-RG-001` (kosmos pool) — see RG-055 |
```

Its stated policy, verbatim:
> ```
> Policy: when a pool dissolves or a record moves namespace, add one row here.
> Validators treat tombstoned IDs as RESOLVED citations. Never delete history —
> point at where it went.
> ```

Note the structural defect: the policy paragraph is inserted **in the middle of
the markdown table** (a `CP-AD-054` row and a `financial-os` row follow it), so
the table is malformed. Any table parser must handle prose inside a table.

### `archive/TOMBSTONES.md` (no frontmatter, different columns)

```markdown
# Governance Tombstones (Retired / Renamed AD/RG IDs)

| ID | Note |
|---|---|
| `PBT-RG-001` | Renamed pool `personal-brand-tactical` → `personal-brand-integrating` (PBI) + `personal-brand-polarizing` (PBP); superseded by PBI-RG-001, PBP-RG-001 |
| `AXE-RG-028` | Transient guard extension referenced in notice board (2026-09-07); never materialized as standalone file; superseded by AXE-RG-032 |
```

**Two column names: `Dead ID | Successor / disposition` vs `ID | Note`.**
A validator must accept both.

### `graveyard/research-engine-20260907-retired/TOMBSTONE.md` (pool-level, prose)

```
# TOMBSTONE — research-engine (RETIRED 2026-09-07)

This pool is **retired and archived**. Do not route work here. Do not restore
it without a new AD superseding KOSMOS-AD-008.
```

**There are two distinct tombstone concepts**: an *ID-level* registry (the two
`TOMBSTONES.md` files, 2-column tables) and a *pool-level* retirement notice
(singular `TOMBSTONE.md`, prose). Both exist.

---

# 4. Employee identity, notice board, worklog

## 4.1 Employee registry

**Canonical location: `swarm/task-grid/data/employees.json`.**
A second, retired copy exists at
`graveyard/autonomous-executor-20260909-retired/data/employees.json` (98 records).

Root shape is a **map keyed by employee id**, not a list:
```json
{ "employees": { "emp-a02e3f692fb0": { … } }, "updated_at": 1790583900 }
```

**Count: 133 employees** (not 117). `updated_at` is an `int` unix epoch.

Field union over the 133 records:

| Key | Type | Presence | Notes |
|---|---|---|---|
| `employee_id` | str | **133/133** | `emp-<12 hex>` |
| `name` | str | 133/133 | slug |
| `display_name` | str | 133/133 | |
| `role` | str | 133/133 | 2 values only: `operator` (96), `automator` (37) |
| `department` | str | 133/133 | 28 distinct values |
| `skills` | list | 133/133 | |
| `home_crons` | list | 133/133 | **exactly 1 entry every time** |
| `status` | str | 133/133 | `paused` (54), `retired` (44), `active` (35) |
| `created` | int | 133/133 | unix epoch |
| `agent_type` | str | 115/133 | `session` (84), `service` (27), `fixer` (4) |
| `persona` | **dict \| str** | 109/133 | 24 are the *string* `"false"` |
| `continuity` | **str** | 104/133 | `"true"` (57) / `"false"` (47) — **strings, not bools** |
| `registers_on_spawn` | **str** | 104/133 | same string encoding |
| `retired_at` | int \| str | 11/133 | |
| `retired_reason` | str | 11/133 | |
| `department_legacy` | str | 1/133 | one-off migration field |

`home_crons[]` sub-keys, all 133/133: `cron_id`, `name`, `schedule`, `enabled`
(`enabled` is a real bool).

`persona` sub-keys when it is a dict: `greeting` 109, `working_style` 109,
`interests` 108, `voice_notes` 107. **The persona is 1 key short of complete on
some records** — an implementer must default the missing one.

**Type counterexamples that will break a strict deserializer:**
- `continuity` and `registers_on_spawn` are **JSON strings** `"true"`/`"false"`,
  not booleans.
- `persona` is `{"greeting":…}` on 109 records and the **bare string `"false"`**
  on 24.
- `retired_at` is `int` in 7 records and `str` in 4.

The HR code compensates explicitly, `swarm/workforce-ops/lib/hr_lib.py:217`
(verbatim):
```python
                r["agent_type"], "Y" if r["continuity"] in (True, "true") else "N",
```
and `hr_lib.py:121-122`:
```python
def persona_valid(rec: Dict[str, Any]) -> bool:
    p = rec.get("persona")
```

**The identity contract lives on the CRON JOB, not the employee record.**
`/home/ishanp/.hermes/cron/jobs.json` — 102 jobs, **100 carry an `employee`
field, 99 distinct employee ids**. Job field union includes `employee`,
`agent_type`, `continuity`, `registers_on_spawn`, `skills` (100/100 each) plus
`skill`, `model`, `script`, `no_agent`, `schedule`, `schedule_display`,
`enabled`, `deliver`, `enabled_toolsets`, `workdir` (100/100).
`agent_type` distribution on jobs: `session` 64, `service` 32, `fixer` 4.

`swarm/workforce-ops/bin/hr:5-6` (verbatim):
> ```
>   - the 117-employee staff record (canonical, read-only consumer of
>     data/employees.json)
> ```
**The 117 figure is the docstring's claim and is now stale — the file holds
133.** The outline inherited 117 from this docstring.

### Verbatim employee record

```json
{
  "employee_id": "emp-a02e3f692fb0",
  "name": "vps-health-watchdog",
  "display_name": "VPS Health Watchdog",
  "role": "automator",
  "department": "platform-infra",
  "skills": [
    "vps-infra-audit",
    "automaton-automation-substrate",
    "axe-command-center"
  ],
  "agent_type": "service",
  "continuity": "false",
  "registers_on_spawn": "false",
  "home_crons": [
    {
      "cron_id": "a02e3f692fb0",
      "name": "VPS Health Watchdog",
      "schedule": "*/15 * * * *",
      "enabled": true
    }
  ],
  "status": "active",
  "created": 1787434775,
  "persona": {
    "greeting": "I work hermes-agent-infra. My role is automator in hermes-agent-infra.",
    "working_style": "Infrastructure-native, tool-builder, automation-substrate. I build the runtime.",
    "interests": [
      "browser-automation",
      "CLI-tools",
      "VPS-infra",
      "gateway-ops",
      "vps"
    ],
    "voice_notes": "Terse. I execute. Results speak."
  }
}
```

Minimal record (no persona / no identity-contract fields), verbatim:
```json
{
  "employee_id": "emp-6cfb879d50aa",
  "name": "forge-reminder-twin-labs-click-submit-on",
  "display_name": "Forge Reminder: Twin Labs — click Submit on th",
  "role": "automator",
  "department": "job-engine",
  "skills": [],
  "home_crons": [
    {
      "cron_id": "6cfb879d50aa",
      "name": "Forge Reminder: Twin Labs — click Submit on th",
      "schedule": "0 10 * * *",
      "enabled": true
    }
  ],
  "status": "retired",
  "created": 1787434775
}
```

### The HR CLI surface (`swarm/workforce-ops/bin/hr`, 17 subcommands)

Verbatim from `bin/hr:13-29`:
> ```
> CLI surface (subcommands):
>   list            - list all employees across all pools
>   show <emp>      - show full detail (record + crons + bonds)
>   audit           - cross-pool health check (fail-closed ERROR / WARN)
>   cleanup         - retire duplicate + dormant employees (DRY RUN default)
>   deploy <emp>    - manually trigger a cron run (--reason required)
>   retire <emp>    - mark an employee as retired (--reason required)
>   scale           - view / set per-pool max-concurrent cap (--reason)
>   runs <emp>      - past cron executions for one employee
>   artifacts <emp> - recent output files for one employee
>   review <emp>    - HR-level performance review
>   review --pool X - org-level rollup (aggregate over a pool's employees)
>   evolve <emp>    - suggest persona updates from run outcomes (--apply + --reason)
>   coverage        - skill-coverage audit (declared vs needed)
>   capabilities    - capability registry search
>   digest          - org-health digest (daily / weekly)
>   help            - long-form guide
> ```
The same surface is reachable as `arch staff <sub>` (`lib/cli.py:4557-4560`).
`lib/hr_lib.py` is the **implementation module** (`arch_staff.py` per its own
docstring at `hr_lib.py:2`) — `bin/hr` is the argparse layer, `hr_lib.py` is
imported by both `arch` and `bin/hr`. Confirmed dispatch points:
`bin/hr:543-619` (argparse) and `lib/cli.py:2897-2905` (`staff_subcmd`).

## 4.2 Notice board

**Location: `swarm/task-grid/data/notice-board.jsonl`.**
**The live file is 0 lines / 0 bytes.** The last populated copy is
`swarm/task-grid/data/notice-board.jsonl.bak-before-backfill` (1,984 lines);
the retired pool's board
(`graveyard/autonomous-executor-20260909-retired/data/notice-board.jsonl`)
has 39 lines. **Combined measured over 2,023 records.**

Field union (records / count):

| Key | Count | Notes |
|---|---:|---|
| `id` | 2023 | `str(uuid4())[:8]` — 8 hex chars |
| `timestamp` | 2023 | ISO-8601 with tz |
| `epoch` | 2023 | int |
| `agent` | 2023 | sender; `"user"` for human notices |
| `message` | 2023 | |
| `tags` | 2008 | list |
| `from_dept` | 1899 | |
| `to` | 1029 | **typed list** |
| `correlation_id` | 998 | |
| `metadata` | 226 | free map |
| `ack_required` | 21 | only written when true |
| `ttl` | 6 | only written when truthy |
| `thread_to` | 2 | |
| `acked_by` | **0** | declared in the class docstring, **never actually written** |

Authoritative writer, `swarm/task-grid/scripts/axe_lib.py:333-354` (verbatim):
```python
        notice = {
            "id": str(uuid.uuid4())[:8],
            "timestamp": now_iso(),
            "epoch": now_epoch(),
            "agent": agent or "axe-system",
            "from_dept": from_dept or "unknown",
            "message": message,
        }
        if to_list is not None:
            notice["to"] = to_list
        if tags:
            notice["tags"] = tags if isinstance(tags, list) else [tags]
        if thread_to:
            notice["thread_to"] = thread_to
        if metadata:
            notice["metadata"] = metadata
        if correlation_id:
            notice["correlation_id"] = correlation_id
        if ack_required:
            notice["ack_required"] = True
        if ttl:
            notice["ttl"] = int(ttl)
```

**Append-only is an explicit invariant**, `axe_lib.py:173-174` (verbatim):
> ```
>     The board is append-only. Notices can never be deleted or edited.
>     This is intentional — it's an audit trail.
> ```
Retention GC therefore **rewrites** the file rather than deleting rows, which is
why it needs a lock (see §4.4).

### Recipient typing convention

`axe_lib.py:292-294` (verbatim) — **four** types, not three:
> ```
>             to: Target recipients — string (legacy) or list of typed selectors:
>                   "agent:<name>" | "dept:<slug>" | "team:<slug>" | "role:<cap>"
>                 None or "broadcast" = broadcast to all
> ```

Normalisation rules, `axe_lib.py:191-217`: `None`/empty/`"broadcast"` →
broadcast (field omitted); a bare string → `agent:<s>`; a string containing `:` →
passed through as already-typed.

**Measured distribution over 2,023 real records** (`to` prefix):

| Recipient | Count |
|---|---:|
| *field absent* (broadcast) | 994 |
| `dept:` | 954 |
| `agent:` | 12 |
| `role:` | 5 |
| `team:` | **0** |
| literal string `"broadcast"` (un-normalised legacy) | 62 |
| legacy untyped string (`"axe-test2"`) | 3 |

**`team:` is documented but has zero occurrences**, and `agent:` — the type the
normaliser *defaults to* — is used only 12 times against `dept:`'s 954. The
recipient vocabulary is real but the traffic is overwhelmingly `dept:` + broadcast.

### Verbatim notice record (typed recipient + correlation chain)

From `graveyard/autonomous-executor-20260909-retired/data/notice-board.jsonl:1`:
```json
{"id": "81d1b114", "timestamp": "2026-09-08T06:45:34.422889+00:00", "epoch": 1788849934, "agent": "emp-dfdf0a014c3c", "from_dept": "freelance-engine", "message": "emp-dfdf0a014c3c (job=cron:dfdf0a014c3c:6252e44de9424d448472bf89ae52c0bc) — completed — model=nemotron-3-ultra-550b-a55b:free", "to": ["dept:freelance-engine"], "tags": ["agent-fabric-observe", "dept:freelance-engine"], "correlation_id": "cron_dfdf0a014c3c_20260908_115943"}
```

A weekly-summary record (the GC's product shape), from
`swarm/task-grid/data/notice-board.jsonl.bak-before-backfill:1`:
```json
{"id": "788a0ad6", "timestamp": "2026-08-27T00:01:06.691587+00:00", "epoch": 1787788866, "agent": "axe-summary-system", "message": "WEEKLY SUMMARY (epoch 1785974400-1786579200):\n  Total notices: 8\n  Top tags: spawn(4), task(4), investigation(2), status(1), announce(1)\n  Top agents: axe-system(7), infra-maintenance(1)\n  Outcomes: 0 succeeded, 0 failed, 0 blocked\n  Failed agents: []", "tags": ["weekly-summary", "dept:task-grid"], "metadata": {"week_start": 1785974400, "week_end": 1786579200, "entry_count": 8, "tag_distribution": {"status": 1, "spawn": 4, "task": 4, "investigation": 2, "announce": 1}, "agent_distribution": {"infra-maintenance": 1, "axe-system": 7}, "outcomes": {"failures": 0, "successes": 0, "blocks": 0}}, "from_dept": "task-grid"}
```
**Weekly summaries are notices on the same board, tagged `weekly-summary` and
carrying a `metadata` block with `week_start`/`week_end`/`entry_count`/
`*_distribution`/`outcomes`.** They are not a separate store.

## 4.3 Retention policy — **VERIFIED, exactly as the outline claims**

Source of truth, `swarm/task-grid/scripts/axe_lib.py:90-92` (verbatim):
```python
# Notice board retention policy (AXE-AD-006)
RAW_RETENTION_DAYS = 14       # Keep 14 days of raw entries
SUMMARY_RETENTION_WEEKS = 16  # Keep 16 weeks of weekly summaries (~4 months)
```

**14-day raw + 16-week weekly summaries is CORRECT**, and it is also stated in
the governing record, `swarm/task-grid/core/decisions/AXE-AD-006-notice-board-entry-lifecycle.md:66-69` (verbatim):
> ```
> 1. **Raw entries**: 14 days retention in the live `notice-board.jsonl`
> 2. **Weekly summaries**: Generated for any completed week whose raw entries are all ≥14 days old. Retained for 16 weeks (~4 months) in the live board.
> ```
…and `:69`:
> ```
> 4. **Pruned**: Weekly summaries older than 16 weeks are removed entirely.
> ```

**Per-tag retention overrides the global default**, `axe_lib.py:109-115` (verbatim):
```python
TAG_RETENTION_SECONDS = {
    "status": 7 * 86400,
    "request": 30 * 86400,
    "ack": 30 * 86400,
    "result": 30 * 86400,
    "fix-pattern": 24 * 3600,
}
```
and the effective window is the **minimum** across tags, `axe_lib.py:908-914`:
```python
        def retention_seconds(tags):
                ...
                for tag, seconds in TAG_RETENTION_SECONDS.items()
            ...
            return min(windows) if windows else RAW_RETENTION_DAYS * 86400
```

Other retention machinery the outline omits:
- `RETENTION_LOCK_PATH = DATA_DIR / ".retention.lock"` — a file lock, because GC
  **rewrites** the board and a concurrent reader would see it empty
  (`axe_lib.py:94-98`, race observed 2026-09-04 with a 1,663-entry sweep).
- `RETENTION_PINS_PATH = DATA_DIR / "retention-pins.jsonl"` — a pin sidecar
  (`axe_lib.py:100-102`): `{"notice_id": "..."}` per line.
- `axe notice summarize` and `axe notice retention --dry-run`
  (`AXE-AD-006:75`).
- `NoticeBoard.retention_gc(now_val=None, dry_run=False, max_weekly_summaries_per_tick=4, log=print)`
  (`axe_lib.py:1113`) — **GC is rate-limited to 4 weekly summaries per tick.**
- Related employee-side TTLs, `axe_lib.py:85-87,104`: `FIXER_AGENT_TTL_SECONDS =
  1800`, `HEARTBEAT_INTERVAL = 300`, `EMPLOYEE_MEMORY_TTL_SECONDS = 30 * 86400`.

## 4.4 Worklog

**Two distinct worklog systems exist.** Do not conflate them.

### (a) FABRIC worklog — per-department agent self-reports

Location: `swarm/agent-fabric/data/worklog/<dept>.jsonl`. **31 files,
1,127 records.** (A 32nd file, `backfill-career-pipeline.json`, is not JSONL.)

Field union over 1,127 records:

| Key | Type | Count | Notes |
|---|---|---:|---|
| `id` | str | 1127 | uuid-ish, e.g. `ffa7a811-378` |
| `ts` | int | 1127 | epoch |
| `employee` | str | 1127 | |
| `department` | str | 1127 | canonical key via `DEPT_ALIASES` |
| `job_id` | str | 1127 | |
| `iteration` | int | 1127 | |
| `workflow_kind` | str | 1127 | enum, see below |
| `what_done` | str | 1127 | |
| `outcome` | str | 1127 | enum, see below |
| `artifacts` | list | 1127 | |
| `tokens_in` | int | 1127 | |
| `tokens_out` | int | 1127 | |
| `tool_calls` | int | 1127 | |
| `duration_s` | float | 1127 | |
| `next_intent` | str | 1127 | |
| `blockers` | list | 1127 | |
| `improvement_proposal` | str\|null | 1127 | the kaizen field |
| `correlation_id` | str | **327** | "framework-only" join key to the board |
| `notice_ids` | list | **327** | |
| `session_id` | str | **327** | |

**The outline's 15-field list is right but incomplete: there are 3 more fields
(`correlation_id`, `notice_ids`, `session_id`), and they are populated on only
29% of records** — exactly the framework-written subset, consistent with the
"framework writes these, the agent writes the rest" design.

Type source, `swarm/agent-fabric/lib/agent_fabric.py:37-70` (verbatim class
names): `AgentType(str, Enum)` (`:37-41`: `session` / `service` / `fixer`),
`WorkflowKind(str, Enum)` (`:44-51`, **members now verified**:
`gather`, `process`, `publish`, `monitor`, `remediate`, `synthesize`,
`coordinate`), `OutcomeKind(str, Enum)` (`:54-58`: `success`, `partial`,
`failure`, `noop`), `FeedbackLevel(str, Enum)` (`:61-66`: `self`, `peer`,
`dept`, `org`).
`WorklogEntry` is declared at `agent_fabric.py:141-167` — its docstring is the
design intent in one line, verbatim:
> ```
>     One row in a dept worklog. Append-only.
>
>     Objective temporal log (framework-appended, AF-AD-007) — must never carry
>     board-routing fields (to/ack_required). correlation_id/notice_ids/session_id
>     are foreign-key links to the subjective board, populated by the framework only.
> ```
**This docstring is the formal statement of the split-brain boundary**: the
fabric worklog is the *objective* log and is contractually forbidden from holding
board-routing fields. My earlier note that the 3 extra fields are populated on
only 29% of records is the *measured* form of the same contract the docstring
declares. `OutcomeKind` members are independently confirmed by the smoke check
at `swarm/agent-fabric/smoke.yaml`:
```python
assert all(e.outcome.value in ("success","partial","failure","noop") for e in ents)
```

Also in `agent_fabric.py` — all four structs now fully verified:
- `Persona` (`:71-76`): `greeting`, `working_style`, `interests`, `voice_notes`
  — **this is where the `employees.json` persona shape is defined**
- `WorkflowStep` (`:80-87`), `Workflow` (`:91-98`)
- `LoopContract` (`:102-114`): `self_reflect`, `peer_observe`, `dept_audit`,
  `org_review`, `improvement_proposal` — all `bool`, all default `False`
- `AgentSpec` (`:118-137`): `employee_id`, `name`, `role`, `department`,
  `agent_type` (default `SESSION`), `persona`, `workflow` (default
  `kind=PROCESS`), `loop`, `skills`, `reports_to`, `peers`, `created_at`,
  `updated_at`. Its docstring is the only spec of the whole agent model, verbatim:
  > ```
  >     The general template for a cron-agent. Every agent IS an instance of this.
  >
  >     Generated by `af spec generate --employee <id>`, mutated by HR/persona
  >     evolution, persisted as <pool>/data/agent-specs/<emp>.yaml.
  > ```
  So there is a **fourth** persisted agent shape:
  `<pool>/data/agent-specs/<emp>.yaml`, generated by `af spec generate`.
- `DEPT_ALIASES` (`:174`), `canonical_dept` (`:192`), `worklog_path` (`:197`),
  `read_worklog` (`:220`), `make_entry` (`:242`), `dept_stats` (`:255`),
  `org_stats` (`:279`).

Verbatim worklog record (`swarm/agent-fabric/data/worklog/agent-fabric.jsonl:1`):
```json
{
    "id": "ffa7a811-378",
    "ts": 1788530905,
    "employee": "emp-af-canary-001",
    "department": "agent-fabric",
    "job_id": "manual",
    "iteration": 1,
    "workflow_kind": "process",
    "what_done": "canary: pre-run check, awaiting cron dispatch",
    "outcome": "success",
    "artifacts": [],
    "tokens_in": 0,
    "tokens_out": 0,
    "tool_calls": 0,
    "duration_s": 0.0,
    "next_intent": "",
    "blockers": [],
    "improvement_proposal": null
}
```

### (b) ARCH worklog — architectural mutation ledger

Different purpose, different shape. Locations: **23** `*/core/worklogs/`
directories (one per pool that participates — `cortex/meta-governance`,
13 `foundations/*`, 3 `swarm/*`, 6 `ventures/*`), each dir-native with one file
per entry plus an `INDEX.md`; plus the legacy single file
`cortex/meta-governance/data/worklog.jsonl` (the **only** `worklog.jsonl` in the
tree).

Writer: `cortex/meta-governance/lib/worklog.py`, surfaced by
`arch worklog {list,append,split,new}` (`lib/cli.py:4339-4365`). Append args:
`--pool` (required), `--action` (required; choices documented as `create, update,
supersede, merge, heal, organize, refactor, sync`), `--target` (required),
`--reason` (**required**), `--title`, `--summary`, `--body`,
`--type {ad,rg,pool,service,sub-system,cron,references,general}`,
`--operator`, `--sub`.

`record_mutation(...)` is called from every write verb — e.g. `cli.py:1280-1289`
inside `cmd_supersede`:
```python
    record_mutation(
        pool=item['pool'],
        target_type=item.get('type', 'AD').lower(),
        target_id=item['id'],
        action="supersede",
        reason=args.reason or "",
        fields_changed={"Status": "Superseded", "SupersededBy": args.by},
        summary=f"Superseded by {args.by}",
        operator=operator,
    )
```

**UNVERIFIED:** the exact `worklog.jsonl` record field set was not enumerated
(this pass read `lib/cli.py` and `lib/worklog.py` by grep, not the record
struct). Only the *write* signature above is verified.

---

# 5. Surprises — things that contradict `docs/ORGANISM-OS-UPGRADE-OUTLINE.md`

Ordered by how much they would cost an implementer.

1. **"~117 employees" is stale — the registry holds 133.**
   `swarm/task-grid/data/employees.json` = 133. The 117 comes from a stale
   docstring at `swarm/workforce-ops/bin/hr:5-6` ("the 117-employee staff
   record"). 99 of them have a live cron. 54 are `paused` and 44 `retired` —
   **only 35 are `active`**. Backfill logic must not assume 133 live employees.

2. **"272 AD records, 149 RG records" does not match any count I can produce.**
   Measured: 266 AD / 261 RG `.md` files (excl `archive/`: 260/256; excl
   `_index.md`: 259/256). The sqlite FTS index at
   `cortex/meta-governance/data/ad_rg_index.db` holds 607 rows — a third number.
   The nearest source is `AD-074`'s own `worklog_note`:
   `"L1 recompiled (24 pools/114 sub-systems/217 ADs/149 RGs/95 crons)"` — so
   **149 RGs is a 2026-09-24 snapshot of *active* records, not a file count**,
   and 217 vs 259 live AD files is a real 42-file gap. The outline's "272 ADs"
   is not reproducible from the filesystem. **Use the file counts, not the
   outline's.**

3. **The outline's citation `cortex/meta-governance/AGENTS.md:66-93` is wrong for
   the pool/AD counts.** Lines 66–93 are the *cron-employee identity contract*
   enforcement list and the Skill Reference Policy. The nearest real
   count statement is `AGENTS.md:154` — a 2026-08-era snapshot: `"A full audit of
   all 108 active governance documents (46 ADs + 62 RGs) across 9 s[ystems]"`.

4. **`services_consumed[].access` has no `read` value — it is an *integration
   mode*, not a permission.** The 5 real values are `cli`, `symlink`, `import`,
   `symlink+cli`, `file`. The outline's Wave-0 item 2 ("Replace `READ_ONLY_VERBS`
   with `access: read`") is inventing a value that does not exist in the
   organism. Operant's design is arguably better, but it is a *new* axis, not a
   port.

5. **`pool.tier` exists in only 28 of 93 manifests (30%).** The outline treats
   `pool{name,type,tier,category,ad_scope,parent}` as the target schema; on real
   data `role` (91) and `domain` (91) have replaced it, and 86 files carry
   `_legacy_tier` / `_legacy_type` / `_legacy_category` null-shadow fields
   because the migration is unfinished. **Model `role` + `domain`; keep `tier`
   as an optional deprecated field.**

6. **`entry_point` / `accepts` / `returns` are NOT reliably present** (63/67,
   63/67, 62/67) and `returns` is sometimes a `str`. The outline's Wave-0 item 3
   says "Keep `entry_point` / `accepts` / `returns` in the compiled row" — sound,
   but they must be `Option<T>`, and 4 services declare only an `id`.

7. **`expect_exit` in the outline's smoke schema is a DEAD FIELD.** 4 real
   manifests use it; the runner reads only `expect_exit_codes` (1 real user).
   The outline's `{name, cmd, timeout, critical, reason}` is otherwise exactly
   right, and the `reason` field is genuinely load-bearing (181/185 records, 49/49
   files) — but **the linter does not enforce it**, and one pool ships without it.

8. **The `Supersedes` / `Superseded-By` pointer contract is ~5% adopted**
   (7/266 and 1/266). The organism's real retirement mechanism is
   `TOMBSTONES.md` prose tables, of which there are **two files with two
   different column schemas** and a third pool-level prose `TOMBSTONE.md`. One
   tombstone table has prose spliced into the middle of it. If operant ports
   "supersede writes a pointer, never a delete", it should port the *tombstone
   file* too, and normalise the columns — the organism did not.

9. **AD/RG frontmatter key case is unstable.** ~40 records use all-lowercase
   (`id`/`status`/`title`/`date`/`deciders`/`description`) while the majority use
   Title-Case; 9 RGs carry **both** `Status` and `status` simultaneously. A
   case-sensitive validator will pass half the fleet silently. There are 28
   distinct AD and 34 distinct RG key-set fingerprints.

10. **AD `Status` values in real use are not the ones `arch list --status`
    advertises.** Measured: `Active`, `Superseded`, `Accepted`, `Proposed` (+ one
    free-text value). The parser offers only `Active`, `Superseded`, `Draft` —
    `Draft` never occurs and `Accepted`/`Proposed` are not offered.

11. **The root `_org.yaml` is not a pool manifest** and has no `pool:` block. Any
    loader that walks for `_org.yaml` and requires `pool.name` crashes on it.

12. **`directories` keys are not normalized** — `data//` in 83 files, `./` in
    one, `core/decisions//` in one. And `suffix` is `str` in some files and
    `list` in others, *within the same file*.

13. **Retired pools still carry live-shaped artifacts** — `graveyard/` ships
    7 `_org.yaml`, 4 `smoke.yaml`, 1 `employees.json`, 1 `notice-board.jsonl`,
    2 `TOMBSTONE*`, and 3 archived `core/decisions/` dirs. Any recursive walker
    must scope to the 4 live strata; the outline's `allowed_top_level` is the
    mechanism (`organism/_org.yaml:26`) and graveyard is explicitly *allowed*,
    so scope is a per-stratum decision, not a global one.

14. **`team:` recipients are documented but unused (0 of 2,023 notices), while
    `dept:` carries 954 and broadcast carries 994.** The outline's "99.55%
    broadcast stat" for AXE-AD-013 is not what the data shows today — the live
    board's dominant shape is `dept:`-scoped.

15. **`notice-board.jsonl` is currently 0 bytes** and the newest populated copy
    is a `.bak-before-backfill` file. There are **three distinct worklog
    locations**: `swarm/agent-fabric/data/worklog/` (31 files, objective
    per-dept log), the 23 `*/core/worklogs/` directories (architectural mutation
    ledger), and one legacy `cortex/meta-governance/data/worklog.jsonl` — the
    same split-brain class the outline flags as BUGS.md R5-1. Note that
    `WorklogEntry`'s own docstring formally forbids the fabric worklog from
    carrying board-routing fields, so the split is *contractual*, not accidental.

16. **`--reason` is enforced by the gate itself.** `axe_smoke.py:965-968`
    declares `--reason` on the *smoke* CLI with the comment
    `"AD-032: state-modifying CLIs must accept --reason (the checks' commands
    can mutate external state, so the gate is not read-only)"`. The mandate
    extends to the read-mostly tools, which is stronger than the outline claims.

---

## 6. Quick reference for implementers

```rust
// Wave 0 — the manifest a real parser must accept
struct OrgManifest {
    pool: Option<PoolBlock>,              // absent in the ROOT _org.yaml
    directories: Option<BTreeMap<String, DirRule>>,  // keys are UNNORMALIZED
    required_files: Option<Vec<String>>,  // one pool has a duplicate entry
    forbidden: Option<Vec<String>>,       // always exactly [__pycache__/, *.pyc]
    services_offered: Option<Vec<ServiceEntry>>,   // items may be STRINGS
    services_consumed: Option<Vec<ConsumedEntry>>, // may be a name->string MAP
    department: Option<Department>,        // charter + class
    relationships: Option<Relationships>,  // depends_on always; loops in 4/23
    rules: Option<Rules>,                  // list or map; unstable element type
    surface_registry: Option<…>,           // ventures only
}
// ServiceEntry: id, entry_point?, accepts?, returns? (list OR str),
//               purpose?, sub_system?, outputs?
// ConsumedEntry: id?, provider?, purpose?, access? (cli|symlink|import|symlink+cli|file),
//                + up to 15 holonic one-off keys

// Wave 2 — the gate
struct SmokeCheck {          // 185 real items
    name: String,            // 185/185
    cmd: String,             // 185/185
    reason: Option<String>,  // 181/185 — absent is real, unlinted
    critical: Option<bool>,  // 119/185 — DEFAULT TRUE
    timeout: Option<u32>,    // 94/185  — DEFAULT 30s; real range 5..700
    expect_exit_codes: Option<Vec<i32>>,  // 1/185 — the ONLY one read
    // expect_exit: 4/185 — DEAD FIELD, never read
}
// exit 0 = pass or non-critical only; exit 2 = >=1 critical failed -> do not spawn
```

## 7. Unresolved / UNVERIFIED

| Item | Status |
|---|---|
| `WorkflowKind` / `OutcomeKind` / `AgentType` / `FeedbackLevel` enum members | **RESOLVED** — `agent_fabric.py:37-66`; all 4 enums fully enumerated in §4.4 |
| ARCH worklog (`worklog.jsonl` / `core/worklogs/*.md`) record field set | **PARTIAL** — write signature verified via `cli.py:1280-1289`; the *record* struct in `lib/worklog.py` was not enumerated |
| `agent_fabric` `LoopContract` / `AgentSpec` / `WorklogEntry` field lists | **RESOLVED** — read in full at `agent_fabric.py:102-167`; see §4.4 |
| `arch journal` subcommand | **UNVERIFIED** — not located in the `cli.py:4311-4787` window read |
| The "65s → 0.4s for 1.8k entries" retention perf claim | **UNVERIFIED** — `swarm/task-grid/scripts/optimize-retention-gc.py` exists but was not read |
| The "2/117 employees have a persona" claim | **CONTRADICTED** — measured 109/133 have a persona dict; 24 have the string `"false"`. The likely real number is *24 missing*, not 2. |
