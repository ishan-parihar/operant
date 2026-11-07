# Feature Inventory — Operant Agentic-Core

**Reconciled to `origin/main` @ `758dd4c5` (iter-1818).** Supersedes the round-1 sweep
(see `red-team/02-round1-summary.md` for the raw counts and `red-team/01-round1-reports/`).

## Local primitives — all verified working

| Capability | Tool | Verified by |
|---|---|---|
| Clock / timestamp | `datetime`, `timestamp` | round 1 + round 2 |
| System + environment | `debug_system`, `debug_env`, `env_probe` | round 1; `env_probe` names only, never values |
| Background process registry | `process` | spawn → wait → `exit_code 0` + captured output |
| Persistent memory | `memory_store`, `memory_search` | exact-key recall returns the stored text |
| Session recall | `session_search`, `session_insights` | 30-day aggregate: 1934 sessions, 26821 tool calls |
| Skill catalogue | `skills_list` | 106 skills |
| Config | `config_manage` | read/write/reload; **runtime-only — not persisted to the TOML** (verified) |
| Task boards | `todo`, `kanban` | write → read round trip |
| Scheduler | `cron` | create → `scheduled` → script actually ran (`/tmp/rt-probe-cron.log`) |
| Checkpoints | `checkpoint`, `aft_checkpoint*` | list + restore |
| MCP registry | `mcp_management`, `tool_search`, `tool_describe` | schema loads |
| Delegation | `delegate_task` | schema-validated JSON |
| Sandbox | `code_execution` | Python + JS |
| Files | `aft_read/write/edit/glob/grep/bash/outline` | round 1; see G-004/G-011 for the degraded paths |
| Browser / web | `browser`, `http_request`, `web_extract`, `web_scrape`, `web_crawl` | round 1 + round 2 |
| Notifications / messaging | `notify`, `send_message`, `react_to_message` | round 1 |

## TUI capabilities now working (operator-verified)

Standing session goal (`/goal` survives every turn until cleared) · the advisor card
renders one block per note · ctrl+enter queues while a turn streams and the band shows
the live queue · queued and goal messages reach the prompt history (Up/Down, Ctrl+R) ·
the input area carries nothing but the operator's own text.

## Degraded / broken — the gap register owns these

| Tool | Symptom | Row |
|---|---|---|
| deferred MCP tools (`papers_search`, `news_search`, `data_query`, `security_vuln`, `patents_search`) | 30 s timeout | G-007 |
| `company_lookup` | SEC EDGAR 403 — needs `SOURCEHOUND_CONTACT_EMAIL` | external, documented at row G-007's sibling |
| `aft_inspect` | `unknown error` — deployed bridge predates the fix | G-004 |
| `aft_grep` / `aft_glob` | index not used; capped walk | G-011 |
| `file_state` | no `read` operation | G-012 |
| `cron` (13 agent jobs) | all `paused` — bare model, no provider prefix | G-008 |
| `working_diff` outside a repo | expected behaviour, not a defect | — |

## Counts

- Local primitives: **25 capabilities, 25 working.**
- Network-backed: **6 capabilities, 1 reliable** (`web_search`/`browser` recovered once the
  MCP client stopped wedging — G-005), 5 degraded by the timeout budget.
- Scheduled/autonomous: **1 capability, 1 broken** (G-008) — the scheduler itself works
  (a no-agent probe job ran on schedule).
