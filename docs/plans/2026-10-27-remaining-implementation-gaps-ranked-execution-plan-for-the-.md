# Plan — remaining implementation gaps: a ranked execution plan for the open ledger rows

**Minted**: 2026-10-27 · **By**: agent-context · **For**: the operator's "outline-plan for executing the remaining implementation gaps"

**Method**: this plan was derived from the authoritative open-gap index —
`docs/audit/2026-06-19-BUGS.md`'s "What is OPEN right now" table plus the
8 unchecked rows (`grep -c '^- \[ \]'` = 8) — and every candidate row was
**re-verified against the live code before being scheduled**. That check
mattered: two rows are stale (RT-181's subject is already implemented), one is
deliberately accepted (RT-135), and one (RT-136) is a confirmed
not-a-defect. Scheduling a stale row is the most expensive mistake available
here, so the order below reflects verified state, not the ledger's labels.

**Scope discipline**: ordered by risk-adjusted value. Tiers 0-1 are surgical and
safe (no architecture decisions). Tier 2 is architecture (needs an owner ruling
before code). Tier 3 is deferred-by-design. Nothing here starts without the
operator's go-ahead on the tier.

---

## Tier 0 — one-line honesty fixes (each its own iteration, all independent)

These are model-facing contract bugs: the tool description promises something the
tool does not do. The 2026-10-25 audit classed them "one-line description change
+ a contract pin test". Zero behavior risk, real cost saved (a model that
believes a fallback exists will trust an error-shaped response).

| # | Row | Fix | Pin |
|---|---|---|---|
| 0.1 | **RT-195 (a)** — `aft_search`'s description promises "(lexical fallback when the semantic index is not enabled)"; live aft 0.62.0 returns `[search_lanes_unavailable]` with **both** lanes `runtime_not_observed`. Verified at `aft_tools.rs:236` | Either drop the promise, or dispatch `aft_grep` on unavailability and report the substitution (the audit's recommended shape, since `aft_grep` has a real transparent fallback) | Description contract test: no tool description may name a fallback the tool cannot produce |
| 0.2 | **RT-196 (a)** — `process` "list" says "list all processes" but lists the agent's own spawned background sessions | Description says what it lists | Same pin family |
| 0.3 | **RT-196 (b)** — `aft_checkpoint` captures only git-tracked files; an untracked file yields `file_count: 0` and a *silent* no-op restore | Description names the tracked-files-only limit | Same pin family |

**Tier 0 evidence**: all three description sites confirmed live at
`aft_tools.rs:236` and the tool description constants; each is a string change
plus a test, so a regression in the same family cannot recur silently.

## Tier 1 — small, verified, no decisions needed

| # | Row | State | Work |
|---|---|---|---|
| 1.1 | **RT-181** — ledger says `on_pre_compress` "always inserts nothing". **Stale: it is implemented** at `memory_wire.rs:707` (folds the conversation, retains it tagged `["session","pre-compact"]`, returns the `[memory context before compaction]` block) and the ladder inserts it after the system prefix (`run.rs:2760`). | Row correction | Update the ledger row to CLOSED with the cited line numbers, and add the missing pin: an integration test that a compaction pass actually re-injects a retained pre-compact insight |
| 1.2 | **RT-135** — `operant cron run <id>` marks the job and does not run it | Owner-accepted stub | Defer to Tier 3 unless the operator rules otherwise. The row itself names the shape: a public `run_job_now(id)` on `CronScheduler` (today only `tick()` fires due jobs) + the CLI building the same agent the scheduler builds |
| 1.3 | **RT-180** — `tui_scenarios/_run.py verify` dies `KeyError: 'baseline'` on scenarios without a `baseline` key, so verify mode cannot run the full corpus | Test infra, low | Make verify mode skip baseline-less scenarios with a note (or gate the scenario). Small, but it unblocks the golden corpus for everyone |
| 1.4 | **RT-166** — the memory-plane headroom tracker has moved off the ledger twice; check `docs/kb/` for its current home before scheduling | Verify-first | Not an implementation task until re-located |
| 1.5 | **S2 residual** — the WIDE (emoji) glyph class is still unproven (the AMBIGUOUS class was cleared live at iter-1139) | One live check | A live wide-terminal probe, then close the row or split it honestly |
| 1.6 | **S-1** — `emotional-memory.md` cited by `human-conversation` exists nowhere | Doc integrity, low | Either add the file or delete the citation. The cite was verified against the repo's skills pool in the audit window and remains open |

## Tier 2 — architecture decisions (owner ruling required BEFORE code)

Each of these is a documented fork, not an implementation choice. The ledger
rows already name the options; the plan is to get the ruling, then execute.

### 2.1 RT-190 — which memory bank is the truth (M, integration)

The 2026-10-25 audit's hardest row. operant embeds memory-wire in-process
(`~/.operant/memory_wire.sqlite`, bank `operant`, 2157 rows) while `memory-wire
serve` runs `~/.local/share/memory-wire/memory.db` and serves **every other
harness** via `memory-wire connect`. So knowledge captured in an operant session
is invisible to claude-code / codex / opencode / copilot and vice versa.

The fork (the row's own words): (a) point operant's provider at the serve via
the HTTP client (`--endpoint`, v0.8.0) at the cost of the in-process
`recall_with_weights` / `retain_doc` calls the HTTP surface does not expose, or
(b) keep in-process and bridge the two stores (one-time bank merge + a sync
seam), or (c) accept two families and document the boundary.

**Recommended default if the operator wants a direction**: (c) **now** (document
the boundary — it costs nothing and stops the confusion), with (b) as the next
tranche if cross-surface recall turns out to matter. (a) trades the whole
in-process advantage for a shared store and should not be the first move.

### 2.2 RT-194 — sourcehound's 23 composites vs the always-on token budget (L→M, integration)

Currently reachable only if the user hand-configures sourcehound as an MCP
server; the built-in surface registers only the four web tools. Eager
registration costs **4,738 tokens per request** (measured: the live
`tools/list` serialized in chat-request wire format, heaviest `news.fetch`
~810). `tool_search`'s deferral partition is `is_deferrable()`
(`tool_search.rs:67`) and it currently defers only `mcp_*`-prefixed tools.

The audit's recommended shape is the cheap one: extend `is_deferrable` to the
sourcehound composites, so the 23 appear as catalog lines (~760 tokens, on the
searching turn only — saving ~3,978 on non-searching turns) and the model
discovers `papers.search` on demand at the cost of one `tool_search` call.
`is_deferrable` is one function — that is the whole seam.

**Sequencing note from the audit itself**: do RT-188/189's production
verification first. Check their rows' current state in the ledger before
scheduling this.

### 2.3 RT-179 — 64 TUI commands with no defensible gateway twin (M, feature)

72 TUI commands are absent from the gateway by name; 8 have semantic twins
(a few real — `cancel`→`stop`; several theater — `compact`→`compress`,
`rename`→`title`); 64 have no twin at all, including the entire functional core
(`/plan` `/context` `/effort` `/memory` `/skill`-invoke `/export` `/search`,
the provider/auth set). The audit doc is the scoped list; the row says it is
blocked on RT-175's mechanism — **which is now CLOSED (iter-1298)**:
`run_session_scoped_command` (turn lock + the invoker's conversation resolved
and restored) is the exact seam named as the blocker.

**So this row's blocker is gone.** The work is: pick the functional-core subset
first (`/plan`, `/context`, `/memory`, `/effort` — the ones an operator
actually wants from Telegram), wire them through `run_session_scoped_command`,
and leave rendering commands (`/theme`, `/vim`, `/mouse`) legitimately
twin-less. That is a multi-iteration slice, not one.

## Tier 3 — deferred by design / accepted

- **RT-135** `cron run` (also listed at 1.2) — accepted stub, closes as a feature when the operator wants it
- **RT-136** `[pk]` / `[socialization]` — confirmed not-a-defect at iter-1176; both labelled in-code and reported by `unused_setting_warnings`. `[pk]`'s three keys additionally now warn per the iter-1355 work
- **D-5b, D-6** — accepted limits (cron emits no memory session-boundary signal; per-agent sub-agent budgets are process-global statics)
- **The 9 remaining inert config keys** (my own iter-1338/1342 audit) — already warn at load; struct-field deletion is a breaking-release cleanup
- **E303 × 9 from `genome lint`** — all from the peer's untracked `docs/library/` pages citing paths that do not exist (`docs/audit/2026-08-02.md`, `agent/moa_loop.py`, `crates/operant-core/src/tools/skills_guard.rs`, `gateway/hooks.py`, `plans/016-harness-kernel-paradigm.md`, +4 more). Theirs to fix; listed here so the next agent does not re-derive it
- **W006** — the two `2026-10-25-residual-*` plans answer the same question; one owner per question is the genome's rule. Whoever owns those plans picks the survivor

---

## Suggested execution order (each item is one iteration: change → scoped test → crate battery → clippy gate → commit + push → deploy)

```
Tier 0 →  0.1  0.2  0.3                          (safe now; no decisions)
Tier 1 →  1.1  1.3  1.5  1.6                     (row corrections first — they
                                                 change what the ledger claims)
Tier 2 →  2.1 ruling → document-the-boundary now (c), bridge later (b)
          2.2 ruling → extend is_deferrable if yes
          2.3 → functional-core subset through run_session_scoped_command
Tier 3 →  leave; revisit on the next breaking release / owner request
```

**What this plan deliberately does NOT do**: it does not close RT-135 without
the operator's word (it is an accepted stub), does not touch the peer's
`docs/library/` E303s, and does not pre-empt the three Tier-2 architectural
rulings. Every item above carries its verified state so the next agent cannot
mistake a stale ledger row for live work — which is exactly the trap RT-181 set.
