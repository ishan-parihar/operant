# Prime Kernel (`pk_*`) — persistent stateful Python kernel + continual harness

Plan 015. Ports prime-agent's two signature capabilities into operant under the
**replace, never duplicate** contract, extended with the RLM-lite tool bridge.

## What you get

| Tool | Purpose |
|---|---|
| `pk_kernel_exec` | Run Python in a **persistent kernel**: variables and imports survive across turns. Cells support top-level `await`. With the bridge enabled, cells can call allowlisted operant tools: `await operant_tool("file_list", {...})` — one call replaces dozens of model turns for sweeps/batches. |
| `pk_harness_get` | Read the continual-harness store: `prompt` (behavioral policy addendums) and `subagent` (reusable delegation specs), scoped `local` (THIS session) or `global` (cross-session). |
| `pk_refine` | Record or apply an evidence-backed refinement. With `edits[]`: all-or-nothing CRUD with a snapshot kept for byte-exact rollback. |

## Architecture

```
model ─▶ pk_* tools (Rust, gated "prime_kernel" toolset)
           │ NDJSON JSON-RPC over stdio
           ▼
       PkSidecarSupervisor ─ spawns ─▶ pk-sidecar (python ≥3.11)
                                        ├─ SessionKernel   (persistent namespaces)
                                        └─ HarnessService  (live vendored rlm)
kernel cells ─ await operant_tool(...) ─▶ host dispatches through the SAME
                                          approval gate as direct calls
```

Policy lives in the Rust host (allowlist, caps, approval parity); state lives in
the sidecar. The harness imports prime-agent's real `rlm` package from the
pinned `vendor/prime-agent` submodule — `git submodule update --remote` pulls
upstream improvements with no code changes here.

## Replaces / complements / untouched

- **Replaces** interactive stateless Python execution once
  `[tools.prime_kernel] route_python_to_kernel = true`: `code_execution`
  python requests run in the session kernel (`"via": "pk_kernel"` marker) and
  fall back to the subprocess path if the sidecar is down.
- **Complements** agentmemory/MEMORY.md (facts) and skills (procedures): the
  harness holds ONLY the two kinds operant lacked — prompt notes and subagent
  specs — in per-session local stores plus one global store.
- **Untouched**: curator, SkillImprover, learning graph, SkillForge,
  SubAgentTool/delegation, MCP/cron/gateway.
- **Feed-forward**: with `harness_auto_learn = true`, the existing background
  review fork may additionally emit prompt/subagent lessons via `pk_refine`.
  No second review loop exists.

## Configuration

```toml
[tools.prime_kernel]
enabled = false                      # flip after provisioning (below)
python = ""                          # optional; ≥3.11 (auto-detected otherwise)
state_dir = ""                       # default ~/.local/share/operant/pk/harness
sidecar_idle_secs = 1800             # idle child auto-exit (0 = never)
request_timeout_secs = 120           # per-cell wall clock
max_output_bytes = 200_000
route_python_to_kernel = false       # phase-4 cutover switch
harness_auto_learn = false           # phase-5 background-review lane

[tools.prime_kernel.tool_bridge]
enabled = false
allowlist = ["file_list", "file_read", "web_search", "http_request"]  # deny-by-default
max_calls_per_exec = 64
per_call_timeout_secs = 60
```

## Provisioning

```bash
git submodule update --init --recursive     # vendor/prime-agent @ pinned tag
./scripts/check-pk-sidecar.sh               # uv pytest — must be green
```

Then set `[tools.prime_kernel] enabled = true` (and the switches above as you
adopt them). `pk_kernel_exec` self-reports diagnostics when the sidecar or
submodule is missing.

## Security honesty (verbatim from upstream)

Neither the sidecar nor the kernel is a security sandbox: model-generated
Python runs with your user permissions. Mitigations are procedural: the tool
sits behind the same interactive approval gate as `code_execution`, bridged
tool calls re-enter that gate per call, and the allowlist denies by default.
Run untrusted workloads in an external sandbox.

Rollback: `/pk rollback` semantics ship as the `refine_rollback` protocol;
every applied refinement event keeps its before-snapshot in
`<state_dir>/<scope>/refinement-ledger.jsonl`.
