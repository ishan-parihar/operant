# remaining plugin-architecture gaps — full efficacy and operationalization roadmap

Dated-frozen: published `2026-11-05`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Owner of the post-import gap ledger (iters 1720-1726 landed the port + red-team
closures F1-F4). Evidence base: `docs/audit/2026-11-05-red-team-the-plugin-architecture-import-against-deepseek-har.md`
(the dsh mechanism survey, findings F1-F5, skips S1-S5). This plan does not restate
the audit; it sequences what remains. The original import plan (Phase 5 web-UI
sketch) is `docs/plans/2026-11-05-import-deepseek-harness-plugin-mechanism-architecture-into-o.md`.

## Two direct answers (verified in source, 2026-11-05)

**No creator mode exists.** The plugin surface is lifecycle-only:
`PluginsSubcommand` = List / Install / Remove / Enable / Disable / Toggle
(`crates/operant-cli/src/cmd_plugins.rs:17`), the TUI plugins hub is a viewer +
enable-toggle (`tui/plugins_hub.rs`). Nothing scaffolds, templates, or validates a
plugin under construction, and no doc walks the observe/tool/memory contracts
from an author's seat. dsh's canon is plugin-manager + HMR (install, edit, hot
reload) — operant has the install half only.

**No web dashboard imports dsh's UI, and nothing previews plugins.** The dsh web
UI was deliberately NOT imported (audit S5: every functional capability is an RPC
client over the server's `session.*` controllers — without reimplementing that
surface it boots into an empty reconnecting shell). Operant's first-party
dashboard is the four-route status view (`/api/status`, `/api/boards`,
`/api/health`, `/api/config`, `dashboard_server.rs:4-8`, the RT-44 contract) —
zero plugin routes, no plugin preview, no UX surface. Both are Gap G7 and G10
below.

## Mechanism

The gap ledger, sequenced. Two axes: **efficacy** (what a plugin can DO — dsh
semantic parity) and **operationalization** (the author → install → hot-update →
observe loop working without restarts or recompiles).

### A. Operationalization (the loop that makes plugins real)

- **G6 — live watcher→agent re-registration (the S3 half-truth made true).**
  Today a newly-installed or changed plugin needs a RESTART to appear as a live
  tool/skill/observer registration; the watcher feeds only the kernel harness
  seam (`Harness::replace`, `main.rs spawn_wasm_watcher`). The fix: plumb the
  live surfaces into the watcher callback — ToolRegistry (register/skip by
  KB-11), SkillManager roots, interceptor list. Moderate: the registry handles
  must reach the watcher path, and a CHANGED module must replace its own prior
  registration (the boot path's collision rule keeps the OLD copy today).
  This is the single biggest operationalization gap.
- **G7 — creator mode (does not exist; verified).** Smallest complete shape:
  `operant plugins new <name> --capability <tool|observer|memory|skill>` writing
  a scaffold directory (manifest.toml with the chosen capability + a stub Rust
  PDK skeleton + a README naming the export contract: `tool_metadata`/`execute`
  for tools, `observe` + envelope shape for observers) + the `wasm32-wasip1`
  build line. Cheap — pure templating, no new architecture; it unblocks the
  authoring loop that G6/G8 complete. A `--validate` pass reusing the
  installer's manifest parser closes the edit loop.
- **G8 — an in-tree efficacy fixture.** The observe/tool contracts are pinned
  at the serde boundary only (no wasm32-wasip1 target is installed; no built
  module exists anywhere in the tree). One tiny fixture module (a few hundred
  bytes of WASM exporting `observe` + `tool_metadata`) built by a script and
  exercised by integration tests would turn "contract pinned" into "loop
  proven", and would let G6's hot-swap be tested end-to-end.
- **G9 — plugin observability.** dsh emits `plugin-manager/*` events; operant
  logs. An honest `operant plugins status` (per plugin: loaded?, capabilities
  active?, interceptor registered?, resident-pool instance count) is the
  operator's view into the loop G6 creates. Cheap once G6 exists.

### B. Efficacy (what a plugin can do — dsh parity the port named as skips)

- **G1 — turn-level interception (audit S1).** dsh `agent/pre-step` veto/mutate
  + `agent/request` config rewrite + `agent/turn-stopping` steer
  (`dsh:packages/core/agent-loop/src/agent.ts:296-640`). Operant's seam is
  tools-only. Trigger: a policy that must gate or rewrite WHOLE turns. Moderate
  core surgery in the agent loop; keep the veto typed-result pattern iter-1722
  established.
- **G2 — post-execute result REPLACE (audit S2).** dsh's post-execute accepts/
  replaces/enriches/blocks (the redaction seat); operant's verdict set is
  allow|block. Needs a Replace verdict (observe contract v2) + a PipelineAction
  variant (the plan said "no new variants" for the first port — this is the
  named upgrade trigger). Small-to-moderate.
- **G3 — `ask` escalation (audit F5 note).** dsh pre-execute escalates to its
  approval service; operant observers can only block. Ties the plugin seam to
  the existing approval layer. Small once a consumer asks for it.
- **G4 — the channel capability.** Declared in `PluginCapability::Channel`,
  deliberately deferred (gateway adapters are compile-time). Decide: implement
  a dynamic channel seam or formally descope the variant. A decision, not code.
- **G5 — config reach + validation.** The manifest `[config]` table reaches the
  observe envelope only (iter-1725); tools/memory receive nothing, and nothing
  validates it (dsh validates a typed schema). Ponytail posture: free-form
  tables are fine until a second consumer appears; then decide table-vs-schema.

### C. Distribution / UX

- **G10 — the web UI (Phase 5 of the import plan; NOT built).** The dsh UI
  import remains opt-in and multi-iteration; its sketch stands in the import
  plan: static + boot-graph serving → Typert unary envelope → mux WS `$events`
  frame → `session.*` subset. The USER-VISIBLE PAYOFF for plugins specifically
  is the dsh canon's `ui-plugin-manager` panel: install/enable/preview installed
  plugins with their capabilities from the browser. Recommendation unchanged:
  skip until the operator surface (G6/G7/G9) makes plugins worth previewing;
  a cheaper first step is a dashboard `/api/plugins` route on the EXISTING
  four-route server (one route + one panel row) — no dsh protocol work.
- **G11 — inventory/marketplace.** dsh ships plugin-package-inventory +
  install-state events; operant installs from git/path. Marginal; revisit only
  if a second publisher appears.

## Sequence (ponytail-ordered: cheapest unblocking first)

1. G7 creator mode (template + validate — cheap, unblocks everything else)
2. G8 fixture module (makes every later gate provable)
3. G6 live re-registration (the real operationalization meat)
4. G9 plugins status (reads G6's state)
5. G2 Replace verdict → G1 turn-level seam (efficacy, consumer-driven)
6. G10 dashboard `/api/plugins` route (cheap web answer) — full dsh UI stays opt-in
7. G4/G5/G3/G11 decisions as consumers appear

## Gates / success criteria

(fixed before the run)

- G6: a plugin installed into `plugin_dirs` while the gateway runs becomes
  callable (tool) / active (interceptor) / listed (skill root) WITHOUT restart,
  proven by an integration test against the watcher callback; a CHANGED module
  replaces its own registration.
- G7: `operant plugins new demo --capability observer` produces a directory the
  installer's own validator accepts, and the README names every export
  contract; `--validate` rejects a broken scaffold.
- G8: integration tests exercise `observe` allow/block and `execute` against a
  REAL module (no serde-only pins) in the main tree.
- Every code gap lands with the regression test that fails without it; docs
  (`docs/library/plugins.md`) change in the same iteration as the code they
  describe; no iteration deploys without the §7 provenance gate.

## Cost

This plan: outline only (this document). Implementation estimates per gap:
G7 ~1 iteration (templating + tests), G8 ~1-2 (fixture + script + tests), G6
~2-3 (registry-handle plumbing + replace-on-change semantics + tests), G9 ~1,
G2 ~1, G1 ~2-3 (agent-loop surgery), G10 cheap-route ~1 / full dsh UI 5+
(if ever opted in). Skipped deliberately: G11 (no publisher), marketplace,
isolation realms/per-agent scoping (audit S4 — restart + config toggles cover
their roles until multi-tenancy is real).
