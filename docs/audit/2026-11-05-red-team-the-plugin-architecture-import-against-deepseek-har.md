# red-team the plugin architecture import against deepseek-harness

Dated-frozen: audited `2026-11-05`; findings mark verified vs asserted.

## Scope

The four iterations that imported deepseek-harness's (dsh) plugin mechanism
(iters 1720-1723: host wake, requires-gating, tool-seam interception +
observe export, docs) are red-teamed against the dsh source at
`../external-repos/deepseek-harness` (dsh citations are `dsh:<path>:<line>`,
operant citations are workspace paths). The question: what is missing for
the port to operationalize at full efficacy — and which gaps are ponytail
fixes vs named skips.

dsh mechanism facts established during the survey (all verified in source):
plugins are RESIDENT fibers (loaded once, handlers live closures); the
agent loop exposes turn-level interception (`agent/pre-step` veto/mutate at
`dsh:packages/core/agent-loop/src/agent.ts:296-306`); the tool pipeline has
four seams (`tools/pre-execute` allow/deny/cancel/`ask`-escalate,
`tools/execute` signal wrapper, `tools/post-execute` accept/replace/block,
`tools/result` observe — `dsh:packages/core/tools/src/pipeline.ts:199-510`);
composition is service-named (`ctx.provide(name)` / `inject: [name]`,
`dsh:vendor/cordis/src/reflect.ts:291-327`), with PENDING fibers waking when
a provider arrives and dependents unloading when one leaves
(`dsh:vendor/cordis/src/fiber.ts:603-736`); plugin Config is a validated
schema passed to `apply(ctx, config)` (`dsh:vendor/cordis/src/fiber.ts:16-62`).

## Findings

- **[verified] F1 — every plugin invocation recompiles the WASM module.**
  Tools: `crates/operant-plugins/src/wasm_tool.rs:118-124` instantiates a
  fresh `extism::Plugin` per `execute` call, justified by a comment claiming
  "Extism Plugin is !Send". That claim is false for the pinned resolution
  (extism 1.41.0 in Cargo.lock): `unsafe impl Send for Plugin` AND
  `unsafe impl Sync for Plugin` (`dsh`-external: extism-1.41.0/src/plugin.rs:188-189),
  and extism ships a first-class resident `Pool` (checkout/return, capacity
  = available_parallelism, condvar wait) for exactly this pattern
  (extism-1.41.0/src/pool.rs). Observers: the iter-1722 shortcut documents
  one WASM load PER TOOL CALL PER POINT (pre + post = two loads per call,
  `plugin_tools.rs:172-176`). dsh pays none of this — a resident fiber's
  handler is a closure. Cost: every plugin-tool call and every observed
  built-in tool call pays compile + instantiate + teardown.
  **Verdict: fix (iter-1725)** — swap both call sites to resident
  `extism::Pool`s (the installed dependency's own house pattern). No timing
  measurement is offered: no wasm32 fixture exists in-tree to run one; the
  change is structural (one compile per module per process vs one per call).
- **[verified] F2 — `requires` matches plugin NAMES only; the service seam
  collapsed too far.** dsh's `inject` references PROVIDED SERVICE names
  (`ctx.provide('memory.backend')` then `inject: ['memory.backend']`), so a
  dependent composes on a seam, not on a concrete plugin. Operant's iter-1721
  `requires: Vec<String>` (`lib.rs:47-52`, fixpoint in `host.rs:125-157`)
  names plugins only — two plugins cannot compose over a service name, and
  a plugin cannot declare what it offers.
  **Verdict: fix (iter-1726)** — `provides: Vec<String>` on the manifest;
  the fixpoint matches requires against plugin names ∪ all provided names.
  Still a discovery filter (no fibers), still transitive, cycles stay fine.
- **[verified] F3 — observers are unparameterizable.** Every dsh policy
  plugin (timeout-policy, permission, sandbox-policy) is config-driven;
  config is schema-validated and reaches `apply`. Operant's observe envelope
  carries `{point, tool, args, result}` only (`plugin_tools.rs:211-216`) —
  a policy observer must hardcode its thresholds.
  **Verdict: fix (iter-1725)** — optional free-form `[config]` table on the
  manifest, passed verbatim as `"config"` in the observe envelope.
- **[verified] F4 — `operant plugins list` bypasses signature policy.**
  `list_plugin_tools` builds the host with `PluginHost::new` (no security,
  `plugin_tools.rs:302`) while the loader uses `with_security` — under
  strict, the CLI can list plugins the runtime refuses to load.
  **Verdict: fix (iter-1726)** — one-line alignment to `with_security`.
- **[verified] S1 — no turn-level interception.** dsh `agent/pre-step` can
  veto a step or replace admitted input before the model call; `agent/request`
  can rewrite the LLM call config. Operant's seam is tools-only.
  **Verdict: skip (named)** — zero consumers; the tool seam covers every
  veto use case that exists today. Upgrade trigger: a policy that must gate
  or rewrite whole turns.
- **[verified] S2 — post-execute can only BLOCK, not REPLACE.** dsh's
  `tools/post-execute` accepts/replaces/enriches/blocks a result (redaction
  lives there); operant's verdict set is allow|block only. Needs a
  Replace verdict + PipelineAction variant. **Verdict: skip (named)** —
  no consumer; same trigger as S1.
- **[verified] S3 — "watcher re-scan covers it" is half-true.** The
  watcher (`main.rs spawn_wasm_watcher`) feeds ONLY the kernel harness seam
  (`Harness::replace` of ArchitectureRows): a newly-installed plugin is
  never re-registered into the live ToolRegistry, SkillManager roots, or
  interceptor list — that needs a restart. The docs' lifecycle shortcut
  claims restart OR watcher; amended in docs/library/plugins.md by iter-1725.
  Live re-registration is the real upgrade (registry-handle plumbing into
  the watcher path) — not built.
- **[asserted → confirmed by port design] S4 — the deliberate non-ports
  stand**: fiber state machine (PENDING wake / disable-cascade unload /
  effect disposers), 5-mode dispatch, per-scope isolation realms, per-agent
  event scoping, `ask`-escalation to approval. Restart + config toggles
  cover their roles with less code; named in docs/library/plugins.md.
- **[verified] S5 — dsh web UI verdict re-confirmed.** The client boots
  browser-local shell (module system, theme/layout) but EVERY functional
  capability is an RPC client over the server's session/controller
  implementations (Typert unary + SSE/control streams + `api-session/*`
  events, `dsh:packages/api/session-controller/src/client/index.ts:98-137`);
  without reimplementing that server surface it boots into an empty
  reconnecting shell. Import stays opt-in (plan Phase 5); cost unchanged.
- **[verified] F5 — fail-open vs fail-loud postures differ, deliberately.**
  dsh is fail-loud at the process boundary (unhandled rejection → exit 1)
  and lets pre-step throws abort a turn structurally; operant observers
  fail open (a broken observer allows, never bricks the loop). Correct for
  a veto seam on the hot tool path; no change — recorded so the difference
  is a decision, not an accident.

## Cost

Survey: one Explore pass over dsh (8 mechanism areas, file:line evidence) +
re-read of the operant plugin surface (plugin_tools.rs, host.rs, runtime.rs,
wasm_tool.rs, watcher consumer path). Fixes: iter-1725 (F1+F3, ~80 lines +
tests), iter-1726 (F2+F4, ~40 lines + tests), docs ride the same iterations.
Skipped (named): S1-S5 above, live watcher→registry re-registration, a real
WASM observe fixture (wasm32-wasip1 target not installed; contract stays
pinned at the serde boundary), any timing measurement for F1.
