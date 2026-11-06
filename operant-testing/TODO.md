# TODO — the register

One row per work item; rows update in place, verdicts append — never delete.
`genome status <id> <state> --evidence <path>` is the supervised transition.

The canonical detail for each row lives in `kb/gap-register.md` (anchored, with the
repro and the fix). This file is the *queue*: what is next, in what order, and what
is already done.

## Pending

| id | status | item / verdict | notes |
|---|---|---|---|
| G-006 | open | Widen the MCP client's response `id` from `u64` to `serde_json::Value` | `mcp.rs:356` (client) vs `mcp_serve.rs:39` (server). A string-echo server cannot be deserialised today. Smallest, most self-contained win in the queue. |
| G-010 | open | Add a cron/scheduler pane to the TUI, registered in `App::overlay_flags()` | `/cron` exists as a command (`operant_app/app.rs:409`) with no pane. Register it in the ONE catalogue — never a second list (RT-251). |
| G-007 | open | MCP-backed tools must not inherit the 30 s tool timeout | Tool timeout `cmd_setup.rs:979`; MCP HTTP timeout `mcp.rs:418` (60 s). Give MCP tools the MCP budget. |
| G-009 | open | Register the 72 TUI-only commands as gateway commands | Canonical ledger RT-179. This is what makes the TUI the *entire* interface. |
| G-005 | open | Land and push the MCP drain-until-match fix | iter-1822, in this workspace; `cargo test -p operant-core --lib mcp::` = 27 passed. Needs its own commit + push. |
| G-004 | open | Re-deploy the aft bridge (v0.63.5 predates the iter-1799 fix) | DEPLOY gap: the source is fixed, the binary is stale. |
| G-008 | open | Prefix the 13 paused cron job models (or route them through `resolve_model`) | DEPLOY/config gap. The scheduler itself works — a no-agent probe job ran on schedule. |
| G-011 | open | Make `aft_grep`/`aft_glob` use the index, or say they did not | Index-backed search silently degrades to a 50k-file walk. RT-195 family. |
| G-012 | open | Add `read` to `file_state`, or remove it from the docs | The arm is documented but unsupported. |
| G-013 | open | Gitignore or delete the junk dirs (`.serena/ .tmp-test/ .zcode/ .tmpwork/ _kernel/`) | Not part of any protocol; they make `git status` unreadable. |

## Implemented

| id | status | item / verdict | notes |
|---|---|---|---|
| G-001 | done | The "Turn ended early" notice names the real cause and the harness continues the turn itself → [2026-11-06] `kb/red-team/03-reconciliation-vs-upstream.md` | Closed upstream: RT-234 iter-1783 (`8264cdea`) + RT-225 iter-1769 (`6bc1b7af`). |
| G-002 | done | The wrap-up is kept in the conversation/transcript and a provider cut-off is continued (bounded at 2) → [2026-11-06] `kb/red-team/03-reconciliation-vs-upstream.md` | Closed upstream: iter-1769. |
| G-003 | done | A delegation that produced nothing is a NAMED failure, not an empty result → [2026-11-06] `kb/red-team/03-reconciliation-vs-upstream.md` | Closed upstream: RT-249 iter-1800 (`c1870d71`); `sub_agent_tool.rs:1404`. |
| G-004a | done | The aft bridge's error path reads the child's 64-line stderr ring → [2026-11-06] `kb/red-team/03-reconciliation-vs-upstream.md` | Closed upstream: RT-247 iter-1799 (`e08a643c`). The remaining G-004 is the DEPLOY gap above. |
| G-014 | done | The `session_search` SQL error did not reproduce → [2026-11-06] `kb/red-team/02-round1-summary.md` | Transient; the same call returned results on retry. |
| G-015 | done | The knowledge base itself: reconciled, anchored, and self-verifying → [2026-11-06] `kb/gap-register.md` | Canonical register, reconciliation, TUI map and the `verify.sh` harness. |

## Review

(rows needing an owner decision — no row yet; add one with `genome q add`)
