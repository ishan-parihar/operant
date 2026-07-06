# UX Audit Report — Operant

**Date**: 2026-07-06
**Audit method**: Fresh-user simulation (Task 105-ux) + 34 prior iterations of code audit
**Auditor**: general-purpose subagent (UX simulation) + main orchestrator (implementation)

## Executive Summary

Operant is a well-engineered agent infrastructure with 34 iterations of bug fixes,
TUI parity work, and gateway improvements. The TUI is production-ready (25/25 audit
bugs closed). The gateway has 8/15 BLOCKERs fixed. However, the **product layer** —
the user-facing experience that makes operant a *personal AI agent* rather than a
*coding tool* — is underdeveloped. The mission ("accelerate evolution and healing in
the individual") is invisible in every user-facing surface.

## Progress Tracker

### Completed (36 iterations: iter-71 through iter-106)

| Phase | Iterations | What was done | Status |
|-------|-----------|---------------|--------|
| TUI bug fixes | iter-71..iter-82 | 4 BLOCKER + 9 HIGH TUI bugs | ✅ 25/25 closed |
| TUI features | iter-73..iter-80 | Banner, syntax highlighting, /skills, /plugins, /journey, /setup, slash commands | ✅ Complete |
| TUI debuggability | iter-83..iter-84 | operant tui debug + action commands | ✅ 8/10 parity gaps closed |
| TUI polish | iter-85..iter-91 | /personality args, /help toggle, keybindings, OpenRouter prefix, /reasoning, --no-mouse, YAGNI cleanup | ✅ Complete |
| Agent API | iter-92..iter-97 | steer_queue_handle, list_subagents, user_question sender | ✅ 25/25 closed |
| Gateway fixes | iter-98..iter-104 | 3 cross-cutting BLOCKERs, streaming, webhook HMAC, Discord chunking, Slack mrkdwn, WhatsApp phone_number_id, typing indicator | ✅ 8/15 BLOCKERs closed |
| UX P0 fixes | iter-106 | README rewrite, tips stub, quick-setup default, help discoverability | ✅ 4/6 P0 fixes |

### Active / In Progress

| Item | Status | Notes |
|------|--------|-------|
| First-run onboarding overlay | Pending (iter-107) | 3-step wizard: provider → model → first message |
| Morning brief cron blueprint | Pending (iter-107) | Agent-initiated messaging — the #1 transformative feature |
| YAGNI cleanup of provider stubs | Pending (iter-108) | Cut from 50+ to 8 well-tested providers |

### Pending (from UX audit recommendations)

| # | Item | Priority | Effort | Impact |
|---|------|----------|--------|--------|
| P0-2 | First-run onboarding overlay | P0 | ~300 LOC | 5× activation rate |
| P0-6 | Fix model picker to use live-fetched list | P0 | ~120 LOC | Stop showing 404 models |
| P1-7 | Replace Rustle mascot with real ASCII creature | P1 | ~150 LOC | Brand identity |
| P1-8 | Real /journey timeline with visual time axis | P1 | ~400 LOC | Makes growth visible |
| P1-9 | /whoami command (what the agent knows about you) | P1 | ~200 LOC | Transparency + trust |
| P1-10 | Extended distillation (personal dimensions) | P1 | ~150 LOC | Agent starts knowing you |
| P1-11 | **Morning brief cron** (agent-initiated messaging) | P1 | ~350 LOC | **Transforms tool → companion** |
| P1-12 | Example prompts on welcome screen | P1 | ~60 LOC | Reduces blank-page paralysis |
| P1-13 | Wire or remove stub slash commands | P1 | ~250 LOC | Trust: help only shows working things |
| P2-14 | "memory · mcp · skills" status pill | P2 | ~40 LOC | Make infrastructure visible |
| P2-15 | Populate spinner/completion verbs | P2 | ~50 LOC | Warmth: UI feels alive |

### YAGNI Candidates (from UX audit)

| # | Item | Action |
|---|------|--------|
| 1 | 50+ provider stubs with fake models | Cut to 8 well-tested |
| 2 | auxiliary_models config section | Remove from example.toml |
| 3 | credential_pool config section | Remove from example.toml |
| 4 | MCP demo entries in example.toml | Move to separate file |
| 5 | /heapdump and /mem in TUI | Move to CLI debug subcommand |
| 6 | 6 of 7 terminal backends (stubs) | Cut to local-only for v0.2 |
| 7 | vision config section | Remove or add /vision command |
| 8 | 22 i18n locale files | Verify if dashboard is shipped |
| 9 | feedback_survey overlay | Remove (no telemetry backend) |
| 10 | pr_body.txt at repo root | Move to .github/ |

### Transformative Missing Features (from UX audit)

| # | Feature | Why it matters | Effort |
|---|---------|----------------|--------|
| 1 | **Morning brief** (agent-initiated messaging) | Tool → companion | ~350 LOC |
| 2 | Real /journey timeline | Makes growth visible | ~400 LOC |
| 3 | End-of-session reflection prompt | Builds a corpus of who you're becoming | ~150 LOC |
| 4 | Skill auto-drafting | Closed learning loop | ~300 LOC |
| 5 | /whoami (what the agent knows about you) | Transparency + trust | ~200 LOC |
| 6 | Goal tracking with weekly check-ins | Agent pushes back, not just answers | ~250 LOC |
| 7 | Emotional vocabulary in status messages | Agent's face becomes expressive | ~80 LOC |
| 8 | Weekly "patterns I've noticed" digest | Agent actively learns about you | ~400 LOC |
| 9 | Cross-session continuity ritual | Every session = picking up where we left off | ~200 LOC |
| 10 | "Challenge me" mode | Growth partner, not just helper | ~150 LOC |
| 11 | /reflect guided reflection | Directly serves the "healing" mission | ~200 LOC |
| 12 | Personalized onboarding interview | Agent starts with a model of who you are | ~150 LOC |

## Verdict

Operant is a competent engineering agent that has not yet decided what it wants to be
when it grows up. The infrastructure is real (TDG memory, gateway, skills, cron). The
product layer isn't there yet. The morning brief is the single highest-leverage feature
— it transforms operant from "a tool I use" to "an agent that knows me."

**Recommended next phase**: implement the morning brief (iter-107), then the
end-of-session reflection prompt + /whoami + personalized onboarding interview.
These four features together would make operant genuinely transformative.
