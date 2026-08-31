# Kernel (`kernel_*`) — persistent stateful Python kernel + continual harness

Plan 015. Implements persistent execution and learned state for operant under the
**replace, never duplicate** contract, extended with the RLM-lite tool bridge.

## What you get

| Tool | Purpose |
|---|---|
| `kernel_exec` | Run Python in a **persistent kernel**: variables and imports survive across turns. Cells support top-level `await`. With the bridge enabled, cells can call allowlisted operant tools: `await operant_tool("file_list", {...})` — one call replaces dozens of model turns for sweeps/batches. |
| `kernel_state` | Read the continual-harness store: `prompt` (behavioral policy addendums) and `subagent` (reusable delegation specs), scoped `local` (THIS session) or `global` (cross-session). |
| `kernel_refine` | Record or apply an evidence-backed refinement. With `edits[]`: all-or-nothing CRUD with a snapshot kept for byte-exact rollback. |

## Architecture

```
model ─▶ kernel_* tools (Rust, gated "kernel" toolset)
           │ NDJSON JSON-RPC over stdio
           ▼
       KernelSidecarSupervisor ─ spawns ─▶ kernel-sidecar (python ≥3.11)
                                        ├─ SessionKernel   (persistent namespaces)
                                        └─ HarnessService  (live vendored rlm)
kernel cells ─ await operant_tool(...) ─▶ host dispatches through the SAME
                                          approval gate as direct calls
```

Policy lives in the Rust host (allowlist, caps, approval parity); state lives in
the sidecar. The harness imports vendored `rlm` package from the
pinned `vendor/vendored rlm` submodule — `git submodule update --remote` pulls
upstream improvements with no code changes here.

## Replaces / complements / untouched

- **Replaces** interactive stateless Python execution once
  `[tools.kernel] route_python_to_kernel = true`: `code_execution`
  python requests run in the session kernel (`"via": "kernel"` marker) and
  fall back to the subprocess path if the sidecar is down.
- **Complements** agentmemory/MEMORY.md (facts) and skills (procedures): the
  harness holds ONLY the two kinds operant lacked — prompt notes and subagent
  specs — in per-session local stores plus one global store.
- **Untouched**: curator, SkillImprover, learning graph, SkillForge,
  SubAgentTool/delegation, MCP/cron/gateway.
- **Feed-forward**: with `harness_auto_learn = true`, the existing background
  review fork may additionally emit prompt/subagent lessons via `kernel_refine`.
  No second review loop exists.

## Configuration

```toml
[tools.kernel]
enabled = false                      # flip after provisioning (below)
python = ""                          # optional; ≥3.11 (auto-detected otherwise)
state_dir = ""                       # default ~/.local/share/operant/kernel/harness
sidecar_idle_secs = 1800             # idle child auto-exit (0 = never)
request_timeout_secs = 120           # per-cell wall clock
max_output_bytes = 200_000
route_python_to_kernel = false       # phase-4 cutover switch
harness_auto_learn = false           # phase-5 background-review lane

[tools.kernel.tool_bridge]
enabled = false
allowlist = ["file_list", "file_read", "web_search", "http_request"]  # deny-by-default
max_calls_per_exec = 64
per_call_timeout_secs = 60
```

## Provisioning

```bash
git submodule update --init --recursive     # vendor/vendored rlm @ pinned tag
./scripts/check-kernel-sidecar.sh               # uv pytest — must be green
```

Then set `[tools.kernel] enabled = true` (and the switches above as you
adopt them). `kernel_exec` self-reports diagnostics when the sidecar or
submodule is missing.

## Security honesty (verbatim from upstream)

Neither the sidecar nor the kernel is a security sandbox: model-generated
Python runs with your user permissions. Mitigations are procedural: the tool
sits behind the same interactive approval gate as `code_execution`, bridged
tool calls re-enter that gate per call, and the allowlist denies by default.
Run untrusted workloads in an external sandbox.

Rollback: `/kernel rollback` semantics ship as the `refine_rollback` protocol;
every applied refinement event keeps its before-snapshot in
`<state_dir>/<scope>/refinement-ledger.jsonl`.
