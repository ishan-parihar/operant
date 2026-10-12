# Organism OS Standalone Repo + ZeroClaw Integration — Audit & Implementation Plan

> Date: 2026-10-08  •  Status: Audit + roadmap. No migrations applied until each phase is approved.

## 0. Four decisions this plan answers

1. **Organism standalone GitHub repo** — move the cybernetic organism out of
   `~/.hermes`/`hermes-active` into its own repository with its own release cadence.
2. **Memory-wire plugin** — decide whether ZeroClaw gets a native memory plugin, an MCP
   bundle, or both.
3. **SOP engine as the sole autonomous scheduler** — replicate Hermes `cron/jobs.json`
   automations through ZeroClaw's SOP engine (`crates/zeroclaw-runtime/src/sop`).
4. **Context injection completeness** — guarantee that the DM/feed context-injection
   substrate (iter-683/684 in operant) is fully implemented and survives the SOP port.

Scope constraint: *only* relevant MCPs integrate with ZeroClaw. Irrelevant executables,
legacy paths, and vendored snapshots are excluded from the standalone repo unless they
are explicitly declarated as parity copies.

---

## 1. Audit — surface area discovered

### 1.1 Canonical organism artifacts

| Artifact | Location | Size | Notes |
|---|---|---|---|
| Organism root | `~/.hermes/organism` on RACKNERD | ~801 MB, 30 215 tracked files in `hermes-active` | 4 strata: cortex, swarm, foundations, ventures |
| Git identity | remote `origin` → `https://github.com/ishan-parihar/hermes-active.git` | — | **Not standalone today** |
| Python source | 2 136 `*.py` | skein of scripts across 19 of 94 `_org.yaml` pools | Steady-state overactive; needs split |
| Docs | 18 527 `*.md` | includes Archive, Graveyard, References, AD/RG cores | Must be deduplicated before extraction |
| Config manifests | 94 `_org.yaml` | every pool has one | Structure allowlist ratified in `AGENTS.md`, line ~54 |
| Cron spec overlays | 141 files in `cron-jobs/*` dirs, likely 140 `*.md` | joined to `~/.hermes/cron/jobs.json` for execution | Targets for SOP replication |
| Organism `AGENTS.md` router | 142 copies | cross-pool documentation | Depends on `HERMES_HOME` naming |
| State/vault DBs | `brain.db`, kanban board DBs, evidence/logs, `data/` dirs, `.codegraph` symlink | hundreds of MB | **Excluded from the repo**; staged on dedicated VPS volumes |
| MCPs currently wired in Hermes | 4 servers: `memory-wire` (:8888), `arch` (:3121), `axe` (:3122), `forge` (:3124) | ones used for governance + task-grid + strategy | These are the only ones that should be ported |

### 1.2 Code-level inventory in the operant monorepo

- `operant-core/src/org/` — ~20 000 LOC. Seats, hierarchy, employees registry,
  notices, worklog, audit rows, queries, memory/seat budgets, and
  `context_injection` (iter-683/684).
- `operant-core/src/cronjobs/scheduler.rs` — 1 000+ LOC cron loop;
  `bind_seat_run` semantics (prompt = `session + employee charter + seat_memory_prompt +
  context injection`). **Contract:** errors in any injection step degrade to the
  prompt byte-identical (fail-open but visible). Watermarks advance only on successful cycle.
- `operant-cli/src/gateway_runner.rs` — captures `ContextInjector` into the handler;
  inbound DM tap (gap 5 phase 2) is already live (`iter-684`).
- `crates/operant-plugins/` — generic WASM plugin runtime shared by operant and the CLI.
- `crates/operant-cli/src/plugin_memory.rs` — WASM memory-provider bridge
  (`PluginMemoryProvider`), documented as exactly mirroring Hermes'
  `plugins/memory/<name>` loader behaviour.
- `memory-wire` crate (`memory-wire@0.6.3` via git lockfile) already inside operant,
  with a builtin fallback `"agentmemory" | "memory_wire" | "memory-wire"`.
- Context injection storage schema (`context_items`, `context_watermarks`,
  `seat_context_quotas`) is embedded in `operant-core`, not yet exposed as a standalone
  schema package.

### 1.3 Hermes dependency map discovered

| Category | Examples |
|---|---|
| Path literals | `~/.hermes`, `~/.hermes/organism/...`, `~/.hermes/cron/jobs.json`, `~/.hermes/state.db`, `$HERMES_HOME/...` |
| Python runtime coupling | `~/.hermes/hermes-agent/venv/bin/python3` used by organism scripts |
| Config home | `HERMES_HOME` env var honoured in `operant-cli/src/config.rs` |
| Cron specs ownership | `jobs.json` lives outside organism root today |
| Seeding seams | Specs/queues generated under each pool but verified against `arch validate` |
| MCP servers | arch/axe/forge/memory-wire all serve from `~/.hermes/organism/...` paths |
| Data life-cycles | Worklog JSONL, notice board, delegation chains, evidence, logs, memory DBs, process registry — mixed into one repo |

Governance invariant to preserve: rates/labels/employee registry decisions remain
**data-driven** (`_org.yaml`, AD/RG records, `org converge`); **no** code path is allowed
to help itself to authority through convenience shortcuts.

### 1.4 ZeroClaw reuse surface

| Surface | File | Boug-out-of-repo note |
|---|---|---|
| MCP client | `src/tools/mcp_client.rs`, `mcp_transport.rs`, `mcp_tool.rs` | supports streamable HTTP |  |
| MCP transports | same | streamable + stdio available |
| SOP runtime | `crates/zeroclaw-runtime/src/sop/*` | engine.rs / graph.rs / executor.rs | existing, no context-injection seam |
| Memory config | `crates/zeroclaw-plugins/src/config.rs`, `wasm_memory.rs`, `component.rs` | memory capability exists; the linker is explicitly noted as not yet carrying component-network authority |
| Cron spec import | src/cron/mod.rs + `SopCommands::Cron` triggers | port cron → SOP |
| SOP cron cycle | `SOP_TRIGGER_CRON` | next_trigger boundary |

---

## 2. Phased implementation plan

### Phase 0 — Decide repo split & env surface (stub, no automation yet)

- Create new GitHub repo `github.com/ishan-parihar/organism` (private initially).
- Move organism source subtree with history via `git filter-repo/git subtree` split of
  `hermes-active` under `organism/`:
  - **Keep**: `AGENTS.md`, `_org.yaml`, each pool's `AGENTS.md`, `_org.yaml`, `bin/`, `core/`,
    `lib/`, `references/`, `scripts/`, `mcp/`, `cron-jobs/*.md`, `tests/`, `assets/`, `data/schema*/**.sql`.
  - **Exclude / migrate to ops volumes**: `*.pyc`, `__pycache__/`, `*.egg-info/`, `.pytest_cache/`,
    `data/logs/`, `data/evidence/`, `data/state/*.db`, evidence images, walkthroughs, `.codegraph`.
- Introduce `~/.organism/` runtime home (or `ORGANISM_HOME` env var) — replace all `~/.hermes/organism`
  literals inside the extracted source.
- Update `operant-cli/src/config.rs` to read `ORGANISM_HOME` as a first-class base path,
  keeping `HERMES_HOME` only for migration compatibility through the next two releases.
- Add an AD entry in `organism/AGENTS.md` documenting the standalone-split rationale.

Exit criterion: every `~/.hermes/organism` string in the extracted tree is replaced with
`<organism-home>/organism` or removed.

### Phase 1 — Extract `org` as a standalone crate (depends on Phase 0)

- Create `crates/` inside the new repo.
- Move `operant-core/src/org/*` → `crates/organism-core/src/org/` and adjust public paths.
  *Do not* copy the `crates/operant-plugins` runtime; instead declare operant as the
  owning monorepo until the extraction is proven, then flip the crate `org` dependency in
  `operant-core/Cargo.toml` to the extracted workspace.
- Add `crates/organism-core/src/lib.rs` and public-API boundaries matching operant's usage
  (employees, hierarchy, notice, worklog, context injection, budgets, schema constants).
- Keep all operant tests compiling by temporarily pinning the org module through a re-export
  shim (`pub use organism_core::org::*`). Delete the shim once operant has no direct references.

Exit criterion: `cargo check -p operant-core` passes after the shim swap; no behaviour
change in operant tests.

### Phase 2 — Move organisation SOP specs into the new layout (depends on Phase 0)

- For every `organism/*/cron-jobs/*.md`: write a paired SOP document such that ZC engine
  can schedule it:
  - front-matter: `name`, `description`, `version`, `priority`, `execution_mode`,
    `sop.steps`, `triggers` (cron specs mirrored from the job front-matter),
    `profile: <seat_id>`, `budgets` link.
  - body: markdown story + any imperative prompt chunks; include a `## Steps` block that
    mirrors the current cron markdown's intent.
- Archive `~/.hermes/cron/jobs.json` as the legacy execution record; keep it read-only.
- Generate a mapping table `organism/governance/MIGRATION-cron-to-sop.md` listing job id,
  skill, schedule, seat, deliverable, SOP target path, SOP trigger.

### Phase 3 — ZeroClaw SOP consumer (depends on Phase 2)

Inside ZeroClaw (`crates/zeroclaw-runtime/src/sop`):

- Import the paired SOPs into `~/.zeroclaw/sops/` (or `~/.organism/sops/` for single-source-of-truth).
- Add a **pre-execution hook stage** in `executor.rs` (before first `step_exec`) that:
  1. resolves the SOP's seat (from `profile`),
  2. opens the org DB,
  3. calls `organism_core::org::context_injection::ContextInjector::render_section(...)`
     with the seat + charter + session seed,
  4. prepends the output to the baseline prompt with a deterministic separator
     (`=== Org Context ===` / `>>>` body) so the model can parse it.
  5. On any failure, produces the byte-identical baseline prompt and advances no watermark
     (fail-open visibility).
- Ensure every gateway turn also passes through the same seam — ideally by sharing the same
  `ContextInjector` struct with ZC's turn-engine gateway (`zeroclaw-runtime/src/rpc/dispatch.rs`).
- Unit tests mirror operant's (watermark monotonicity, content-hash dedup, quota caps,
  post-injection failure semantics).

Exit criterion: a short-boot ZC instance runs a seeded SOP and the prompt log contains the
context section distinct from previous cycles.

### Phase 4 — Memory-wire plugin decision & implementation

Three options. Decide per workload.

**Option A (recommended short-term): MCP bundle.** memory-wire already has a serve binary
answering Streamable-HTTP on `:8888`. ZeroClaw already speaks MCP via
`src/tools/mcp_client.rs` and `mcp_bundles`. Add it to ZeroClaw's decision-modelling config
with the least code:

```toml
[[mcp.servers]]
name = "memory-wire"
url = "http://127.0.0.1:8888/mcp"
timeout_ms = 30_000
connect_timeout_ms = 5_000
[mcp_bundles.org-memory]
servers = ["memory-wire"]
```

No WASM component needed. `"memory_wire"` already routes to the same in-process provider
in operant, so parity is preserved without porting.

**Option B: WASM memory plugin**. This is where the `PluginMemoryProvider` bridge in
operant becomes useful. Expose memory-wire as a `MemoryPlugin` WASM component:
`crates/zeroclaw-plugins/src/wasm_memory.rs` expects a WIT `memory` world exporting
`prefetch`, `sync_turn`, `tool_schemas`, `handle_tool_call`. The adapter will need:
- a thin `memory-wire-plugin` crate that wraps the existing `MemoryWireProvider`,
- a linker, state store, warm-store adapter matching `WasmMemory`,
- a linker-time host grant for persistence (the memory WASM module currently runs on an
  empty world with no network/fs authority).

Cost: non-trivial WASM ↔ native bridging, upcoming component-boundary work with egress
policy. Only pursue if zero-sidecar memory latency is an owner requirement.

**Option C: keep operant as the sole memory-wire host** and expose tool-call access
through `MCP client`. This is essentially Option A with operant as the service owner.

Decision: execute **Option A first**. If the owner demands a WASM memory provider later,
drop the MCP bundle and build Option B against `WasmMemory`; both routes remain documented.

**Migration contract**: the `memory` section of operant config already names `"memory_wire"` as
provider name — do not rename it during the migration; ZeroClaw config must reuse the
same provider identifier where parity matters for downstream log analysis.

### Phase 5 — MCP consolidation (only relevant MCPs in ZeroClaw)

From Hermes' `config.yaml:1161`, four MCP services survive into the organism repo:

| MCP | Current owner | Move target inside organism repo | ZeroClaw exposure |
|---|---|---|---|
| `memory-wire` | standalone crate (`memory-wire@0.6.3`) | `services/memory-wire/` as a submodule or crate dependency against the extracted crate | `Option A` above |
| `arch` | `organism/cortex/meta-governance/mcp/arch-mcp` | already inside organism; update paths only | mcp client entry later |
| `axe` | `organism/swarm/task-grid/mcp/axe-mcp` | already inside organism; update paths only | mcp client entry later |
| `forge` | `organism/foundations/strategy-incubator/sub-systems/forge/mcp/forge-mcp` | already inside organism; update paths only | mcp client entry later |

Every other MCP reference in `hermes-active` is excluded — including legacy
`mcp_http.py` dispatcher, redundant `axe_mcp.egg-info` artifacts, and per-pool `http`
shims no longer in use. Keep them in the archive (`organism/archive/`) for one release,
then prune from history with `git filter-repo` if the file count matters.

### Phase 6 — Organism CI/CD dark-launch

- Add `crates/organism-core` unit-test workflow; keep operant CI (`crates/operant-*`)
  green under the shim.
- Add a docs build for `organism/AGENTS.md` mirroring `hermeshermes-agent/website` routing.
- Declare release cadence: `organism-<YYYY.MM.patch>` independent of `operant` cadence.

## 3. Immediate action queue (2026-10-08 stretch)

1. Inventory cron specs: 141 markdown files, 140 likely GUI-readable; write the mapping table
   (`cron id, skill, schedule, seat, deliverable, SOP target, SOP trigger`) in a
   `MIGRATION-cron-to-sop.md` doc root at `organism/governance/`.
2. Run a dry `git subtree split --prefix=organism` on `hermes-active` and share the log with
   the owner before any force-push.
3. Capture the current ZC config (`~/.zeroclaw/config.toml`) snapshot for rollback, then
   add `[[mcp.servers]]` and `mcp_bundles` for the four MCPs through a `backup` branch commit.
4. Write unit tests for at least two SOP cases in ZC that verify context injection fields
   (chars, watermark advances) appear in the prepared prompt and that the absence of org
   DB files fails-open.
5. Decide Option A vs B for memory-wire with the owner. Option A is fully configured in
   the ZC side; Option B requires adding a persistent WIT store grant.

## 4. Acceptance criteria

- Organism has its own git origin and CI.
- No code path inside the extracted crates depends on `HERMES_HOME`.
- `cron/jobs.json` is read-only; runtime execution flows through SOPs.
- Each SOP run whose seat has prior history includes a deterministic `## Injected context`
  section under its combined char cap, with a new non-zero watermark row after success.
- ZeroClaw holds exactly four MCP servers (`memory-wire`, `arch`, `axe`, `forge`) and zero
  legacy hermes-only transports.
- The `memory_wire` provider name resolves the same in operant, ZC, and the organism repo.

## 5. Anti-goals (explicitly out of scope)

- Do not migrate every historical `data/`, `archive/`, `graveyard/` subtree in the same
  change; preserve a dated archive commit in `hermes-active` first.
- Do not rewrite cron specs before Phase 2; they are the load-bearing source of truth for
  existing behaviour.
- Do not port `option-a`/`option-b` for memory plugin before recording the decision in AD.
- Do not let ZeroClaw reference Hermes paths inside its source (`~/.hermes*`) after Phase 3.
