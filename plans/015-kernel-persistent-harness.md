# 015 — Prime Kernel & Self-Learning Harness (RLM-lite): persistent programmatic kernel + the two missing learning kinds

Stamped against operant `main`. Priority: **P1** (core capability upgrade).
Upstream sources of truth:
- `../hermes-plugins/hermes-prime-bridge/` (v0.2.0) — minimal-port reference
- `../prime-agent/` v0.8.1 — full reference (`refinement.ts`, `rlm/harness.py`, `docs/rlm-runtime.md`)

## Terrain reality: operant ALREADY has most of the learning loop

This plan was revised after mapping operant's existing machinery. Any component below
that duplicates an existing one was cut or converted into a wiring task. What operant
already ships (do **not** rebuild any of this):

| Existing operant machinery | Location | What it already does |
|---|---|---|
| **Background review** | `crates/operant-core/src/agent/background_review.rs` (1,010 LOC) | After **every turn**, forks the agent to evaluate whether skills/memories should be saved or updated; writes straight to stores; main conversation untouched. Aux-model routing with cache-aware digest policy ("auto" = same-model warm replay; other model = compact digest). Actively biased toward updating something every session. |
| **Curator** | `crates/operant-core/src/curator/` | Skill lifecycle (active→stale→archived, 14d/30d), LLM-driven consolidation into umbrella skills, whole-dir tar.gz backup/rollback, cron-reference rewrites, interval runs (24h default). |
| **SkillImprover** | `crates/operant-runtime/src/skills/improver.rs` | Auto-patches SKILL.md after successful usage; cooldowns; validate-then-atomic-rename. |
| **Learning graph** | `crates/operant-core/src/agent/learning_graph.rs` + `tools/learning_mutation_tool.rs` | Skills+memories as first-class graph nodes w/ edges (`/journey` overlay); model-callable `learning_manage` edit/delete mutations. |
| **SkillForge** | `crates/operant-runtime/src/skillforge/` | External skill discovery: Scout→Evaluate→Integrate pipeline. |
| **Memory lanes** | `agent_memory.rs`, `memory_provider.rs`, `context/lcm.rs` | File MEMORY.md/USER.md + agentmemory (BM25+vector+graph) + lossless LCM DAG with bounded auto-recall injection each turn. |
| **Insights** | `agent/insights.rs` via `session_insights` tool | Session analytics the agent can query on demand. |

Conclusion that reshapes this plan: operant's gap versus prime-agent is **not** the
refine loop (background review ≈ auto-refine, curator ≈ curation/rollback) and **not**
the execution loop alone — it is:

1. **No programmatic orchestration surface.** Nothing lets the model run *code around
   the tools* (loops/batching/error-handling in one turn). Every tool call costs a full
   model turn. ← biggest architectural gap, nothing overlaps it.
2. **No persistent execution state.** `code_execution` is tempfile-per-call; bash/files
   persist artifacts but not live interpreter state across turns.
3. **Two missing learning KINDS + one missing SCOPE.** Existing learning covers
   *skills* (markdown procedures) and *memories* (facts) at workspace/global scope.
   Prime-agent's harness adds: **prompt notes** (narrow behavioral-policy addendums),
   **subagent/delegation specs** (reusable roles), and **session-local scope** (lessons
   that must not leak into other sessions' global stores). Plus a **per-event
   refinement ledger** — curator backs up the whole skills dir per run; individual
   `learning_manage` edits carry no snapshot.
4. **Learned skills are not executable.** Skills are prose; prime-agent skills are
   importable Python callables (`await skill(...)`).

So this plan builds #1/#2 outright, adds exactly #3's missing pieces (no second skill
store, no second memory store), and makes #4 possible by extending the existing skills
surface — never duplicating it.

## Architecture

```
 model ──▶ kernel_exec / kernel_state / kernel_refine        Rust tools, gated "kernel" toolset
   ▲                │ NDJSON JSON-RPC over stdio (id-correlated, per-request timeout)
   │                ▼
   │        PkSidecarSupervisor (Rust, operant-core/src/tools/pk/)
   │          spawn-on-demand · ping health · crash-restart (≤2 consecutive, then fail closed)
   │          idle auto-exit · process-group teardown · serialization per session_key
   │                │ spawns
   │                ▼
   │        kernel-sidecar (Python ≥3.11, ./kernel-sidecar/, uv-managed)
   │          SessionKernel  — persistent namespace/session_key + awaitable operant_tool() shim
   │          HarnessService — live import vendor/prime-agent .../rlm (HarnessState CRUD,
   │            snapshot(), record_refinement(); diagnosable degrade when submodule absent)
   │                │ {"method":"tool_call","params":{"name","args"}} over stdio
   │                └─▶ Rust dispatches ToolRegistry.execute() under the SAME approval
   │                     policy as direct calls ─▶ result returns into the kernel program
   │
   ├── Learning feed-forward (REUSES existing engines — no new refiner):
   │     background_review.rs gains an optional harness pass: its existing post-turn fork
   │     may ALSO emit prompt-note/subagent-spec edits via the harness ledger (snapshot →
   │     apply → record_refinement). Pre-compression hook at compress.rs triggers one
   │     review pass before compression. No second review agent, no second trigger loop.
   └── InjectionLane: "[continual harness]" block beside "[agentmemory context]"
         (bounded budget; local-scope first) — prompt-notes/subagent-specs currently have
         NO injection path anywhere in operant; this is the feed-forward link.
```

Rejected alternatives (for posterity): PyO3 embed (GIL/crash coupling); ipykernel+ZMQ
from Rust (heavy Jupyter framing — future opt-in backend, as upstream notes); native-Rust
HarnessState clone (freezes upstream; loses live-vendor upgrade path); **a dedicated
RefinerEngine with its own triggers/prompts (v1 draft) — cut: duplicate of
background_review.rs, violates replace-never-duplicate**.

## The replaces-not-duplicates contract

| # | Operant today | Action |
|---|---|---|
| 1 | `code_execution` python path (stateless tempfile+subprocess) | **Replaced as default interactive Python path** once P4 flips `route_python_to_kernel`; js/shell/rust + explicit-batch mode unchanged. |
| 2 | `execute_pipeline` / SOP (declarative orchestration) | Complemented — kernel adds imperative model-authored programs; pipelines stay pre-authored. |
| 3 | Background review (post-turn skill/memory writer) | **Reused, extended**: gains an opt-in harness pass emitting prompt-note/subagent-spec edits into the ledger. Its prompts, aux-routing, and cadence remain authoritative. |
| 4 | Curator + SkillImprover + learning graph + SkillForge | Untouched. They own the skill/memory lanes end-to-end. Harness store deliberately carries NO skill/memory kinds. |
| 5 | `agentmemory` + MEMORY.md + LCM | Untouched. Harness is a third, separate lane (own root, own schema, own injection block); tests assert agentmemory bytes unchanged. |
| 6 | `SubAgentTool` / delegation runtime | Untouched. Harness may hold *specs* (reusable role descriptions consumed as prompt material), never a second spawn engine. In-kernel recursion into kernel_*/delegate is depth-guarded off. |
| 7 | Skills system (SKILL.toml/SKILL.md, creator, improver) | Extended in Phase 6: executable `reference` field → kernel-loaded Python callable. Prose skills keep working identically. |

## Self-learning loop completeness contract (grounded)

| Link | Mechanism | Owner | Phase |
|---|---|---|---|
| Learn | Post-turn review fork evaluates trajectory | **existing** background_review.rs (+harness pass) | 5 |
| Gate | Review agent's own judgment + digest/routing policy | **existing** | — |
| Persist | New kinds land in scoped versioned stores w/ per-event ledger | **new** harness store (prompt+subagent kinds ONLY) | 3 |
| Feed-forward | `[continual harness]` injection block each turn | **new** InjectionLane (skills/memories already inject via their lanes) | 4 |
| Grow capability | Executable learned skills callable in-kernel | **extension** of skills schema | 6 |
| Recover | Per-refinement snapshot rollback (scoped to harness entries) | **new** (curator's tar.gz covers skills only) | 3,5 |

## Files in scope

New:
- `kernel-sidecar/` — `pyproject.toml` (uv), `kernel_sidecar/{__init__,server,kernel,harness,vendor}.py`,
  `test/`. Methods: `ping`, `exec{session_key,code}`, `reset{session_key}`,
  `harness_{list,get,upsert,delete}{scope,...}` (kinds: `prompt`,`subagent` only),
  `refine_record{scope,evidence,trigger}`, `refine_apply{scope,edits[]}` (snapshot→apply→record,
  all-or-nothing), `refine_rollback{scope,event_id}`, `shutdown`.
- `.gitmodules` + `vendor/prime-agent` pinned @v0.8.1.
- `crates/operant-core/src/tools/pk/{mod,sidecar,kernel_tool,harness_tools,tool_bridge}.rs`
  — supervisor/client, tools, bridge dispatch (allowlist, per-call timeout, result caps,
  depth guard, audit lines).
- `docs/pk-kernel.md` — user doc incl. replaces-table + terrain map above + security honesty.

Modified:
- `crates/operant-core/src/tools/builtin.rs` — register tools behind config gate.
- `crates/operant-core/src/config.rs` — `[tools.kernel]`: enabled(false until P4),
  python, vendor_dir, state_dir(`<home>/pk/harness`), sidecar_idle_secs(1800),
  request_timeout_secs(120), max_output_bytes(200_000), route_python_to_kernel(false→true P4),
  tool_bridge{enabled, allowlist(default read-only set), max_calls_per_exec(64),
  per_call_timeout_secs(60)}, harness{auto_learn(bool, default false until soak)} +
  auxiliary slot if the harness pass needs a distinct model (default: inherit
  background_review's routing).
- `crates/operant-cli/src/config.rs` — mirror fields; round-trip test asserts both parse one TOML.
- `crates/operant-core/src/agent/run.rs` (+stream.rs) — populate
  `ToolContext.metadata["session_id"]` at with_metadata sites (~195, ~936). Also required
  by background-review scoping, not just kernels.
- `crates/operant-core/src/agent/background_review.rs` — add optional harness pass: when
  `harness.auto_learn`, the existing fork's instruction set gains the harness emission
  contract (upstream `REFINEMENT_SYSTEM_PROMPT` semantics, restricted to prompt/subagent
  kinds, local-default scope, base-prompt-immutability rule verbatim); edits route through
  `refine_apply` (snapshot→apply→record). Same-model/digest routing policy untouched.
- `crates/operant-core/src/agent/compress.rs` — pre-compression nudge: fire one review pass
  best-effort before compression (small timeout, never blocks compression).
- `crates/operant-core/src/memory_provider.rs` or `agent_memory.rs` injection site —
  append `[continual harness]` block (budget default 1200 chars; local-first; empty renders nothing).
- `crates/operant-config/src/policy.rs` — permission-list entries for `kernel_exec`,
  `kernel_refine`; bridged calls inherit target-tool gating.
- `README.md` — count + paragraph.

Out of scope: `code_execution.rs` internals (doc note only), curator/improver internals,
operant-memory internals, WASM plugins, MCP/cron/gateway, ipykernel backend, in-kernel
subagents, any new skill/memory store.

## Steps

### Phase 0 — terrain (dark merge)
Submodule pin @v0.8.1 (+gate assert when enabled); `kernel-sidecar` scaffold +
`scripts/check-kernel-sidecar.sh`; config surface in BOTH copies, all default-off;
`session_id` metadata wiring (independent value: background review scoping benefits too).
**Accept:** workspace green dark; sidecar pings standalone.

### Phase 1 — sidecar kernel service
SessionKernel port (RLock, safe-builtins-with-`__import__`, vars echo, 200KB head/tail cap);
NDJSON server; supervisor (health/restart/idle/process-group teardown).
**Accept:** state survives across execs; reset clears; kill -9 mid-request recovers; idle exit fires.

### Phase 2 — `kernel_exec`
Args `{code, namespace?=session_id|"default"}`; gated registration; permission entry.
**Accept:** cross-turn persistence in TUI; independent namespaces; code_execution byte-identical behavior.

### Phase 2.5 — tool bridge (RLM-lite) ⚠️ core architectural upgrade
Kernel builtin `await operant_tool(name, args_json)`; host-side dispatch into
`ToolRegistry::execute` with ToolContext `{origin:"kernel_bridge", session_id}`.
Hard invariants: allowlist deny-by-default (read-only set first); **bridged calls pass the
same approval gate as direct calls**; max_calls_per_exec(64) + per-call timeout → structured
partial results; no kernel_*/delegate re-entrancy; audit line per call; oversized results
truncated with marker; errors returned as values, never transport failures.
**Accept:** 40-call repo sweep completes in one turn; denied tool returns permission-error
value; call-cap yields partial-result payload; slow bridged tool cannot deadlock the kernel.

### Phase 3 — harness store (two kinds, scoped, ledgered)
Tools `kernel_state` / `kernel_refine` (manual). Store carries ONLY `prompt` and `subagent`
kinds × local/global scopes + refinement event ledger w/ snapshots (vendored HarnessState;
unused upstream kinds left dormant, not surfaced). Storage `<home>/pk/harness/<scope>/`.
**Accept:** roundtrip persists across restart; agentmemory DB + skills dir + MEMORY.md
bytes provably unchanged after operations; rollback restores byte-identical prior state.

### Phase 4 — feed-forward + python-routing cutover
`[continual harness]` injection block (bounded, local-first, empty-safe); flip
`route_python_to_kernel` (transparent kernel execution for python code_execution calls,
`"via":"kernel"` marker, stateless fallback when sidecar down); session-end teardown;
`/pk reset|harness|rollback` command surface (discovery: confirm slash registry location);
docs truthfulness pass (plan-014 rules).
**Accept:** learned entry appears in next-turn prompt block; python code_execution persists
state; sidecar kill mid-session degrades gracefully without failing the turn.

### Phase 5 — learning wiring (extend, don't build)
Opt-in `harness.auto_learn`: extend background_review.rs's existing fork instructions with
the harness-emission contract (restricted JSON shape; local-default; small-evidence rule);
edits flow through `refine_apply`; pre-compression nudge at compress.rs; `/pk rollback`;
metrics (proposed/applied/rolled-back) into tracing + cmd_status. Ship default-off; soak
before enabling.
**Accept:** scripted 30-turn session with auto_learn produces ≥1 applied prompt-note whose
text appears in the following turns' injection block; noisy checkpoint demonstrably emits
nothing; rollback undoes it.

### Phase 6 (stretch) — executable learned skills (pyskill)
Extend the existing skill schema (SKILL.toml) with optional `reference{type:"python",
import, callable, call_pattern}`; kernel imports qualified skill references into the
namespace; injection block advertises them as callables; improver's validation learns the
new field; SkillForge can later scout executable candidates. Prose-only skills unaffected.
Ship separately after Phase 5 soaks.

## Local validation gates (per plans/README.md)

```bash
source scripts/dev-env.sh 2>/dev/null || true
cargo fmt --all && cargo fmt --all --check
export LIBCLANG_PATH=/usr/lib/llvm21/lib
cargo clippy -p operant-core --all-targets -- -D warnings
cargo test  -p operant-core --lib
uv run pytest kernel-sidecar/test
cargo test --workspace --all-features --lib   # final per-plan gate
```

## Demo scenarios (observable definition of success)

1. **Stateful analysis**: CSV loaded turn 3, filters iterated turns 4–7, summary turn 8 —
   zero reloads.
2. **Programmatic sweep**: one `kernel_exec` runs a 40-tool-call scan that previously
   cost ~40 model turns.
3. **Scoped lesson**: session learns "this project pins LIBCLANG_PATH"; with auto_learn on,
   background review writes a LOCAL prompt-note; next turn's injection shows it; other
   sessions' stores untouched (scope test).
4. **Recovery**: bad lesson causes misbehavior; `/pk rollback` restores prior harness
   state byte-for-byte; misbehavior gone; skills/MEMORY.md untouched throughout.

## Risks & mitigations

- **Duplication regression risk (the big one)**: any temptation to grow the harness store
  back into a skill/memory store, or to add a second review loop, is a protocol violation
  against curator/background-review ownership. Guardrail: store kinds enum locked to
  prompt/subagent in code review; CI grep-test asserting no new review-fork spawn sites.
- **Runaway model-authored programs**: host-side call cap + wall-clock deadline + output
  caps + partial-result contract.
- **Permission laundering via bridge**: same-gate re-entry, allowlist deny-by-default,
  depth guard; tested explicitly.
- **Injection bloat**: hard budget; empty-state silent; local-first priority; metric logged.
- **Review-fork prompt growth**: harness pass adds bounded tokens only when auto_learn on;
  off-by-default keeps existing behavior byte-stable.
- **Sidecar lifetime vs daemon/gateway sessions**: v1 kernels scoped to owning process;
  documented; snapshot/revival is future (upstream has machinery).
- **Config drift between core/cli copies**: round-trip parity test.
- **Security honesty**: sidecar/kernel do NOT sandbox model-generated code (upstream states
  this verbatim; repeat in docs + tool description); approval-gate membership mandatory
  before P4 default-enable.
