# Memory Budget

Measured numbers, the caps that enforce them, and the method — the
authoritative footprint record for the 2026-10-14 batch (plan 07).
`operant debug memory` prints the live per-process numbers at any time.

## Measured (2026-10-15, release binary, idle)

| Process | RSS | Notes |
|---|---|---|
| Gateway daemon (idle, telegram up) | 26.7 MB | systemd unit, `/proc/<pid>/status` |
| TUI (idle, composer focus) | 24.0–25.5 MB | tmux session, 110×45, two runs |
| TUI, `MALLOC_ARENA_MAX=2` | 24.0 MB | **no delta** — arenas multiply under churn, not at idle; the env var is not shipped (the measurement closed the question) |

Idle targets from the plan (~35 MB) are already met by the release
binary. The budget below exists to keep them met under load.

## The budget table (caps enforced in code)

| Subsystem | Cap | Where enforced |
|---|---|---|
| Retained tool outputs (Ctrl+O) | 32 calls × 128 KB, **4 MB total**, oldest-first eviction | `App::remember_tool_output` (tui/app/prompt.rs) — regression: `tool_output_budget_evicts_oldest_first` |
| aft bridge stderr ring | 64 lines | spawn-time drain task (aft_bridge.rs) |
| Child stderr pipes | drained live — the 64 KB OS buffer can never fill and freeze a child | aft_bridge.rs + mcp.rs spawn paths — regression: `megabyte_stderr_child_still_serves_requests` |
| Detached warmups | reaped in-thread; no `<defunct>` accumulates | `warm_callgraph_detached` |
| memory-wire store | on-disk sqlite in the operant home; bounded content (USER_TURN_CHARS/ASSISTANT_TURN_CHARS) | memory_wire.rs |
| `[tools.web]` fetch bodies | `max_results` (20) + `fetch_timeout_secs` (30) | web_tools.rs |

## What is NOT capped (deliberately)

- The conversation transcript in the App: it IS the user's context;
  eviction is plan 06's territory (tiered decay), not a byte cap.
- Corpus goldens and render caches: measured idle RSS shows them inside
  the 24 MB envelope; revisit only if the idle number regresses past 35 MB.

## Method

All numbers are `/proc/<pid>/status` VmRSS, measured on a quiet box
(loadavg < 20 on 24 cores — the memory-wire measurement discipline applies
here too: a contended box inflates RSS with transient page-cache noise).
Gateway RSS: `systemctl --user show -p MainPID --value operant-gateway`
then read its `/proc`. TUI RSS: launch under a dedicated tmux socket,
poll `pgrep -n -x operant` until the process appears, read once at idle.

## Phase 2 (07.2, pending)

The multiplier is process count: every TUI spawns its own aft bridge and
MCP children. The shared-services design (one gateway-daemon pool over
`$XDG_RUNTIME_DIR/operant/services.sock`) converts N×children into
1×children + N thin clients; its acceptance gate is 5×TUI ≤ 5×40 MB +
daemon.
