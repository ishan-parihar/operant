# Lane D — Memory, Context Management & Persistence (red-team + ponytail audit)

- **Lane**: D (memory / context management / persistence)
- **Repo**: operant, root `/home/ishanp/Documents/github/my-projects/agentic-harness/operant`
- **Pinned revision**: `25b86ae5` (confirmed with `git rev-parse --short HEAD`)
- **Seam**:
  - `crates/operant-core/src/context_management.rs` (tiered eviction T1/T2/T3 + decay curve)
  - `crates/operant-core/src/context/` (module tree), `context_window.rs`, `context_references.rs`
  - `crates/operant-core/src/memory_wire.rs`, `memory.rs`, `memory_provider.rs`
  - `crates/operant-core/src/database.rs`, `migrations.rs`, `historian.rs`, `active_sessions.rs`
  - `crates/operant-core/src/session/`
- **Status**: IN PROGRESS — this document is written first and edited after each confirmed finding.

## Header: files read / not read

| file | lines | read? | notes |
|---|---|---|---|
| `crates/operant-core/src/context_management.rs` | 4053 | YES (in progress) | |
| `crates/operant-core/src/context/` | 4750 | partial | |
| `crates/operant-core/src/context_window.rs` | 706 | TODO | |
| `crates/operant-core/src/context_references.rs` | 1117 | TODO | |
| `crates/operant-core/src/memory_wire.rs` | 1773 | TODO | |
| `crates/operant-core/src/memory.rs` | 1370 | TODO | |
| `crates/operant-core/src/memory_provider.rs` | ~600 | TODO | |
| `crates/operant-core/src/database.rs` | 7192 | TODO | |
| `crates/operant-core/src/migrations.rs` | — | TODO | |
| `crates/operant-core/src/historian.rs` | 720 | TODO | |
| `crates/operant-core/src/active_sessions.rs` | 755 | TODO | |
| `crates/operant-core/src/session/` | 1380 | TODO | |

NOT READ (declare on completion): everything outside the seam.

## Concept counts

### context_management.rs constants (eviction/decay/budget)

| const | value | line |
|---|---|---|
| `DEFAULT_MAX_TOOL_RESULT_TOTAL_SHARE` | 0.40 | 61 |
| `TOOL_TRIM_PROTECT_LAST` | 8 | 66 |
| `TOOL_TRIM_KEEP_CHARS` | 400 | 70 |
| `DEFAULT_FITMED_VIEW_TOKEN_BUDGET` | 8000 | 387 |
| `MIN_FITTED_OUTPUT_TOKENS` | 1024 | 432 |
| `OUTPUT_FIT_HEADWAY_TOKENS` | 64 | 439 |
| `PROMPT_ESTIMATE_MARGIN_DIVISOR` | 10 | 446 |
| `SEMANTIC_GROUP_CHARS` | 512 | 638 |
| `SEMANTIC_MAX_GROUPS` | 64 | 643 |
| `RESPONSE_RESERVE_TOKENS` | 4096 | 1145 |
| `CONTEXT_DISCLOSURE_RESERVE_TOKENS` | 256 | 1152 |
| `FORCE_BUST_PERCENT` | 85 | 1160 |
| `REACTIVE_DECAY_H50` | 200 | 1246 |
| `REACTIVE_DECAY_CONSTANT` | 30.0 | 1248 |
| `MICRO_COMPACTION_THRESHOLD_PERCENT` | 40 | 1254 |
| `MICRO_COMPACTION_HEAD_KEEP` | 4 | 1256 |
| `MICRO_COMPACTION_TAIL_KEEP` | 8 | 1258 |
| `MICRO_COMPACTION_FOLD_MAX_CHARS` | 240 | 1260 |
| `MICRO_COMPACTION_FOLD_HEADROOM_PERCENT` | 25 | 1275 |
| `DEFAULT_MAX_TOOL_RESULT_SHARE` | 0.05 | 1836 |
| `SPEND_MARKER_TOKEN_RESERVE` | 48 | 1840 |
| `ADVERTISED_SKILLS_TOKEN_BUDGET` | 4000 | 1874 |
| `ADVERTISED_SKILLS_WINDOW_SHARE_NUM/DEN` | 1/8 | 1895-6 |
| `ADVERTISED_SKILL_DESCRIPTION_CHARS` | 120 | 1935 |
| `MEMORY_INJECTION_TOKEN_BUDGET` | 4000 | 2022 |
| `MEMORY_INJECTION_WRAP_TOKENS` | 20 | 2033 |
| `MIN_TOOL_RESULT_TOKENS` | 192 | 2068 |
| `PREFLIGHT_DECAY_H50` / `_CONSTANT` | 100 / 20.0 | turn_finalizer |

### Message shape (client.rs:1217-1226)

`role, content, reasoning, name, tool_call_id, tool_calls, extra_content`

Key structural fact for the decay audit: **tool call arguments live in
`tool_calls[].function.arguments: String` (client.rs:1491-1496), a SEPARATE
field from `content`.** `decay_render` truncates `content` only (1811-1815) and
carries every other field through `..msg`. Therefore decay CANNOT truncate a
tool call's argument JSON. The `tool_call` shape the work order worried about
is preserved.

## Findings

### D1 — `compression_locks` table: written by nothing, read by nothing [VERIFIED] — LOW (dead store)

`crates/operant-core/src/database.rs:106-111` declares `compression_locks`
(session_id, holder, acquired_at, expires_at) plus
`idx_compression_locks_expires` (:125). It has a full Rust API —
`acquire_compression_lock` (:3755), `is_compression_locked` (:3773),
`release_compression_lock` (:3789) — and **three tests** (:4877, :5242).

An exhaustive workspace grep for those three method names outside
`database.rs` returns **zero hits**. No production path acquires, checks, or
releases the lock, so concurrent compression of one session is ungated while
the schema advertises a lock. Cost: one table, one index, one API, three tests
that assert a lock nothing takes.

### D2 — `compartments` table + `historian.rs`: a store with no production reader or writer [VERIFIED] — LOW/MED (dead store, 720-line module)

`database.rs:209-226` (migration v5) creates `compartments` with 2 indexes;
`database.rs:1007/1034/1074` expose `insert_compartment` /
`compartments_for_session` / `last_compartment_span_end`. The ONLY callers of
any of the three are `database.rs` tests (:7074-7190). The writer was deleted
in iter-2404 and the deletion note itself says so
(`agent/compress.rs:497` — "`compartments_for_session` had ZERO callers").
`historian.rs` (720 lines of pure span-planning logic) now has no caller either.
The note argues the pure logic is "independently useful"; measured, it is
unreferenced. Same for the `compartments` DDL — every new database pays for a
table, 2 indexes and a UNIQUE index nothing writes.

### D3 — `messages` columns written but never read: `codex_reasoning_items`, `codex_message_items` [VERIFIED] — LOW

`database.rs:93-94` declares both. They are written by `save_message_full`
(:1425-1445) and read back by `get_session_messages_full` (:1691-1715), but
every construction site outside `database.rs` sets them to `None`
(run.rs:3540, session_recap.rs:296, tools/evidence_recall.rs:212/314, plus
tests). No code ever populates them, so the columns are always NULL: a dead
column pair riding every message INSERT and every full-row SELECT.

`platform_message_id` (:95) is in the same class outside `database.rs` — grep
for the identifier across `crates/` finds only its own declaration and the
`save_message_full`/read plumbing.

### D4 — `messages_fts` external-content triggers: `content='messages'` with a manual-rebuild backfill [ASSERTED → mechanism verified, cost not measured] — MED

`database.rs:626-633` builds `messages_fts` as an FTS5 external-content table
over `messages`, and (:638-652) adds `messages_ai` / `messages_ad` /
`messages_au` triggers. The `messages_au` trigger deletes and reinserts on EVERY
UPDATE of `messages` — and `messages` UPDATEs are the hot path:
`save_tool_call_in_flight` (:1335) writes `content`, `complete_tool_call_in_flight`
(:1396) writes `content` + `full_content` + `token_count`, both on every tool
call. Each such UPDATE now costs two FTS b-tree writes (delete-all + insert).

Also: `content='messages'` means the FTS table does NOT store the text, so
`messages.content` is authoritative — and the trigger reads `old.content` for
the delete. A tool row's content is rewritten twice per call (in-flight
placeholder, then the real result), so the FTS index churns 3 writes per tool
call. What I could NOT confirm: the resulting write amplification in wall-clock
or WAL bytes — I did not run the DB or benchmark.

### D5 — every table has no retention except `checkpoints` [VERIFIED] — MED

`DELETE FROM` appears 10 times in `database.rs`; every one is either
session-scoped deletion by explicit id (:2625, :3681), a single-row delete by
key (:2707, :2833, :2873, :2884, :2908), or a key delete (:3736, :3792).
**No time-based retention exists anywhere** (grep for
`retention|prune|vacuum|purge|cleanup` in `database.rs` → zero hits).
So `messages`, `context_ledger` (one row per REQUEST, `database.rs:138-156`),
`session_events`, `verification_events` and `compartments` grow without bound
for the life of the file. `context_ledger` is the sharpest: it appends one row
per model request with 16 numeric columns, and the measured note at
`context_management.rs:52-56` describes a 988-message session. The CLI has
`operant checkpoints prune|clear` (`cmd_checkpoints.rs:3-4`) — the one table
that does have an operator-facing retention verb.

## Ponytail cut list

## Rust discipline