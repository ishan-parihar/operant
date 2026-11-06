# Master Test Summary — Operant Agentic-Core

**Date:** 2026-11-05T00:34:00Z  
**Workspace:** ./operant-testing/  
**Skill:** project-genome (restored from backup to ~/.agents/skill/project-genome/)

## Executive Summary

| Category | Tools Tested | Passed | Failed | Degraded | Success Rate |
|---|---|---|---|---|---|
| Core Utilities | 8 | 8 | 0 | 0 | 100% |
| Memory & Session | 5 | 5 | 0 | 0 | 100% |
| Config & Task Mgmt | 4 | 3 | 0 | 1 | 75% |
| Code & Repo Ops | 7 | 5 | 1 | 1 | 71% |
| Network & Web | 6 | 3 | 3 | 0 | 50% |
| Execution & Delegation | 3 | 2 | 0 | 1 | 67% |
| **Total** | **33** | **26** | **4** | **3** | **79%** |

## Key Findings

### What Works (26 tools)
- All core utilities (echo, datetime, timestamp, system info, env vars, process registry)
- Memory persistence (store + exact-key recall)
- Session search and insights
- Skill catalog (106 skills)
- Config path retrieval
- Todo and kanban task management
- File write/read/edit cycle
- HTTP requests (GET)
- Web content extraction
- Sandboxed code execution (Python)
- Checkpoint listing

### What's Broken (4 tools)
- `aft_inspect` — returns "unknown error" (codebase health inspection broken)
- `web_search` — MCP response ID mismatch (deferred MCP tool)
- `browser` — MCP response ID mismatch (deferred MCP tool)
- `company_lookup` — MCP response ID mismatch (deferred MCP tool)

### What's Degraded (3 tools/categories)
- `cron` — all 13 jobs paused, several with provider API errors
- `delegate_task` — returned empty result (may have failed silently)
- `aft_grep`/`aft_glob` — time-limited on large roots (incomplete search)

## Architectural Blind Spots Identified

1. **Deferred MCP tools are unreliable** — 6 tools timeout or return ID mismatch errors
2. **Codebase inspection is broken** — `aft_inspect` returns "unknown error"
3. **Repository-wide code search is incomplete** — `aft_grep`/`aft_glob` hit file limits
4. **AST symbol search may not be indexed** — 0 matches for common patterns
5. **Cron jobs are all paused** — scheduled execution not operational
6. **No interactive/blocking tool support** — by design constraint
7. **Secret detection is read-only** — no remediation capability
8. **File state tracking is limited** — `read` operation not supported

## Workspace Structure

```
operant-testing/
├── SKILL.md (project-genome at ~/.agents/skill/project-genome/)
├── kb/
│   ├── feature-inventory.md
│   ├── gap-register.md
│   ├── blind-spots.md
│   └── test-reports/
│       ├── core-utils-report.md
│       ├── memory-session-report.md
│       ├── config-task-report.md
│       ├── code-ops-report.md
│       ├── network-report.md
│       └── execution-delegation-report.md
├── tests/ (empty — test scripts to be added)
├── fixtures/
│   └── code-ops-test.md
└── state/ (empty)