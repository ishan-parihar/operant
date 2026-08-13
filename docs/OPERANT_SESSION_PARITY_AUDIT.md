# Operant Session-Management & hermes-lcm Parity Audit

**Date:** 2026-08-13
**Scope:** Session-tool parity vs hermes-agent, remaining hermes-lcm ports, and the
"close this chapter" gap list for production session management.

---

## 1. Executive summary

Operant's session **storage** layer is already a superset of hermes-agent's. The gaps
are concentrated in three places:

1. **The agent-facing `session_search` tool** — operant implements 2 of hermes-agent's
   3 modes (browse + keyword discovery), but lacks the **scroll mode**
   (`session_id` + `around_message_id` + `window`), the **message-window + bookends**
   rendering, and **session-lineage dedupe**. `role_filter` is accepted but unimplemented.
2. **Session UX / slash-command parity** — branching UI exists but is only reachable via
   a keybinding (no `/branch` arm); `/title`, `/status`, `/heartbeat`, `/handoff`, and
   `/quit --delete` are absent.
3. **hermes-lcm** — 7 of 15 tools ported. The **only session-specific tool remaining is
   `lcm_load_session`** (paged raw-transcript loader); the rest are evidence/grep/describe/
   expand tools that are valuable but not session-management. Session **controls**
   (`ignore_session_patterns`, `read_only` scopes, session-boundary rules) are not ported.

---

## 2. Session-tools parity vs hermes-agent

### 2.1 `session_search` agent tool

| hermes-agent (`tools/session_search_tool.py`) | operant (`session_search_tool.rs`) | Status |
|---|---|---|
| **Browse mode** — no args → recent sessions chronologically (title/preview/timestamp), zero LLM cost | ✅ same (`list_recent`) | ✅ |
| **Discovery mode** — `query` → FTS5, dedupe hits by **session lineage**, top sessions + snippets, message **window around the match**, **bookends** (start/end messages), hidden sources excluded, automation sources demoted | ⚠️ FTS5 keyword search returns rows but no lineage dedupe, no window/bookends, no source policy | 🟡 partial |
| **Scroll mode** — `session_id` + `around_message_id` → window of messages centered on anchor, no FTS5 | ❌ absent | 🔴 gap |
| Params: `query`, `limit`, `session_id`, `around_message_id`, `window` | `query`, `role_filter` (TODO stub), `limit` | 🟡 |
| Filters out context-compaction summaries | not applicable (no compaction summaries yet) | ✅ |
| Zero LLM calls (pure SQLite + FTS5) | ✅ same | ✅ |

### 2.2 Session DB schema parity

| Dimension | hermes-agent (`hermes_state.py` SessionDB) | operant (`database.rs`) | Status |
|---|---|---|---|
| Core columns (id, source, model, model_config, system_prompt, cwd, user_id) | ✅ | ✅ | ✅ |
| **`parent_session_id` (branch lineage)** | ✅ | ✅ `sessions.parent_session_id` + `idx_sessions_parent` FK | ✅ |
| Timestamps / accounting (started_at, ended_at, end_reason, message/tool counts, token breakdown, cost) | partial | ✅ superset | ✅ |
| Gateway routing cols (session_key, chat_id, chat_type, thread_id) | ✅ | handled in `operant-infra` JSONL session store + `session_keys` sanitizer | ✅ (different layer) |
| Session meta updates (title, activity, runtime lock, billing route) | ✅ `update_session_meta` etc. | ✅ `update_session_title`, `set/get_session_metadata`, cost/billing | ✅ |
| Search | FTS5 | ✅ `search_sessions` (FTS5) + CLI `sessions search` + tags | ✅ |
| Delete cascade (children) | ✅ `get_session_delete_targets` | ⚠️ `delete_session` — child handling not verified | 🟡 verify |

**Verdict:** storage parity is effectively complete; `parent_session_id` lineage is
already persisted. The deletion-cascade of branched children is the only storage item to
verify.

---

## 3. Session UX / slash-command parity

| hermes-agent command | operant TUI | Status |
|---|---|---|
| `/new` | ✅ `/new` \| `/fresh` | ✅ |
| `/clear` | ✅ `/clear` | ✅ |
| `/resume`, `/sessions` | ✅ `/session` \| `/resume` \| `/sessions` → browser | ✅ |
| `/title` (rename) | ✅ `/rename` + browser rename flow | ✅ (different name) |
| `/branch` | ⚠️ `session_branching.rs` UI exists, opened via keybinding (no slash arm) | 🟡 add `/branch` |
| `/status` (session/model/token/context) | ⚠️ status bar exists; no `/status` command | 🟡 |
| `/heartbeat` (recurring idle prompt) | ❌ absent | 🔴 |
| `/handoff` (session → messaging platform) | ❌ gateway handoff not surfaced as command | 🔴 |
| `/quit --delete` (delete history on exit) | `/quit` exists; no `--delete` | 🟡 |
| `/topic` (Telegram DM topic sessions) | n/a (gateway-only, hermes-specific) | ✅ n/a |

**Session browser resume flow:** Enter in Browse mode → `session_load_pending` →
async load of the selected session. Rename flow confirmed. Branching: `BranchBrowserMode`
with create/delete confirmations, opened at `key_handling.rs:1267`.

---

## 4. hermes-lcm remaining ports

hermes-lcm exposes **15 `lcm_*` tools**; operant has **7**. Parity matrix:

| hermes-lcm tool | operant equivalent | Status |
|---|---|---|
| `lcm_recall` (FTS+vector RRF) | ✅ `lcm_recall` | ✅ |
| `lcm_recent` (temporal) | ✅ `lcm_recent` | ✅ |
| `lcm_doctor` | ✅ `lcm_doctor` | ✅ |
| `lcm_status` | ✅ `lcm_stats` | ✅ |
| `lcm_inspect` | ✅ `lcm_doctor` diagnostics overlap | ✅ partial |
| `lcm_recall_round` (hybrid recall) | ✅ `lcm_recall_round` | ✅ |
| vector recall | ✅ `lcm_vector_recall` | ✅ |
| **`lcm_load_session`** (ordered raw-message transcript page for explicit `session_id`, `after_store_id` cursor → `next_cursor`, `include_exact_ref`) | ❌ | 🔴 **session gap** |
| **`lcm_grep`** (grep across stored state) | ❌ | 🔴 |
| **`lcm_query_state`** (query store internals) | ❌ | 🔴 |
| **`lcm_compute`** (on-demand derived state) | ❌ | 🔴 |
| **`lcm_retrieve`** (targeted node retrieval) | ❌ | 🔴 |
| **`lcm_describe`** (describe node/rollup) | ❌ | 🔴 |
| **`lcm_expand` / `lcm_expand_query`** (query expansion) | ❌ | 🔴 |
| **`lcm_evidence_pack` / `lcm_compile_evidence`** (evidence bundles) | ❌ | 🔴 (YAGNI candidate) |

### Session controls (config-level, not ported)

| hermes-lcm control | description | operant |
|---|---|---|
| `ignore_session_patterns` (glob) | skip noisy sessions in recall/context | ❌ |
| `read_only` session scopes | protect sessions from mutation | ❌ |
| session-boundary rules | source-lineage + boundary constraints in recall + context assembly | 🟡 operant recall is per-session scoped but has no boundary config |

### Portability of `lcm_load_session`

Operant LCM already has the primitives: `list_sessions()`, `recent_message_nodes()`,
`nodes_in_window()` and the `nodes` table (id, session_id, position, kind, role, content,
created_at) + FTS. A `lcm_load_session` tool needs only a cursor-based paged query on
`nodes WHERE session_id = ? AND id > ? ORDER BY position LIMIT ?` plus
`content_hash`-based exact-slice refs. **Low-risk port, ~150 lines + tests.**

---

## 5. Recommendations (priority order)

1. **P0 — `session_search` scroll mode** (`session_id` + `around_message_id` + `window`):
   completes the hermes-agent session tool surface and enables the agent to "read a
   specific part of a past conversation" — directly required for session management.
   Implement `role_filter` at the same time (it's already accepted in the schema).
2. **P1 — `lcm_load_session` tool**: paged raw-transcript loader on the existing `nodes`
   table (cursor = `store_id`, `next_cursor` returned). This is the last session-specific
   hermes-lcm tool and closes the LCM session chapter.
3. **P1 — `/branch` slash arm** wired to the existing `session_branching` overlay.
4. **P2 — session controls**: `ignore_session_patterns` + `read_only` scopes in LCM
   recall/context assembly (config-driven, mirrors hermes-lcm).
5. **P2 — `/status` command** (session/model/token summary from the status bar state).
6. **P3 — `/heartbeat`, `/handoff`, `/quit --delete`**: value depends on gateway roadmap.
7. **P3 — remaining LCM tools** (`lcm_grep`, `lcm_query_state`, `lcm_compute`,
   `lcm_retrieve`, `lcm_describe`, `lcm_expand(_query)`): genuinely useful for deep state
   introspection; `evidence_pack`/`compile_evidence` are **YAGNI** unless evidence
   bundling becomes a product requirement.
8. **Verify** — branched-child cascade on `delete_session`.

---

## 6. Verdict

- **Session storage**: production-grade, superset of hermes-agent (`parent_session_id`
  lineage already persisted).
- **Session tooling**: one real gap (`session_search` scroll mode + window/bookends);
  one missing hermes-lcm session tool (`lcm_load_session`); a handful of UX commands.
- **LCM chapter close**: after P0–P2 (above), the LCM chapter is complete — remaining
  tools are optional introspection utilities, not session-management requirements.
