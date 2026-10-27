# Session management red-team audit: all surfaces vs jcode + oh-my-pi

Dated-frozen: published `2026-10-28`. Audit of the session-management
architecture across every operant surface (TUI, CLI, slash commands, gateway
channels, operant-os, employee management), contrasted against the two parents:
`../external-repos/jcode` (Rust TUI parent) and `../external-repos/oh-my-pi`
(omp — parent of several operant features). Companion plan:
`docs/plans/2026-10-28-unified-session-management-jcode-style-session-picker-pane-o.md`.

Every claim below was source-verified at tip `251ffdb6` (2026-10-27) with file
cites. Where a prior feed claim (F0066) turned out to be already fixed, this
audit says so — the audit re-derives, it does not copy forward.

## 1. What operant has today (the inventory)

### 1.1 Core: TWO session tables, TWO stores, no unified record

| surface | table / store | key | fields |
|---|---|---|---|
| TUI/CLI sessions | `sessions` (`database.rs:34-66`) | `id TEXT PK` | source, title, model, parent_session_id, cwd, token/cost columns, rewind_count, archived |
| gateway chats | `gateway_sessions` (`gateway_session.rs:616-644`) | `session_key TEXT PK` | session_id, origin, platform, chat_type, tokens, cost, suspended, resume_pending, **employee_id** (added by the wave-2 ALTER at :432), conversation_id |
| TUI warm store | `SessionStore` (`session/mod.rs:177`) | `SessionKey` | in-memory Vec\<Message> per key, persist() to DB, acquire/release refcounts |
| gateway warm store | in-memory `SessionStore` metadata map + the persistent `GatewaySessionStore` (sqlite) | | |

Red-team finding **RT-A (fragmentation)**: there is no single
`SessionRecord` the way jcode has `SessionInfo`
(`jcode-tui-session-picker/src/lib.rs:114-151`). The TUI's own list shape is
`SessionEntry` (`session_browser.rs:29-37`) with **5 fields** (id, title,
last_updated, message_count, cost_usd) and its loader even carries a
`ponytail:` comment admitting `messages` is a count-carrier hack
(`adapter_types/history.rs:91-97`). The TUI list loads at most **50 sessions**
(`history.rs:75`), the CLI dumps use 10,000, the gateway has its own
`list_sessions(active_minutes)` (`gateway_session.rs:1493`). Three shapes, three
limits, one DB.

### 1.2 TUI surface: the redundant modal

`/session` `/resume` `/sessions` (commands.rs:526-531) open
`SessionBrowserState` rendered through `render_session_browser`
(`session_browser.rs:197-`), which is a **centered 70x20 modal**
(`modal_frame_buf` with `width: 70, height: 20`) listed in the overlay guard
table (`render/operant_overlays.rs:197`). It has: up/down navigation, `r`ename,
Enter to switch, Esc to close (key_handling.rs:681-713). That is ALL it has.

Contrast with jcode's picker — same role, different universe
(`jcode-tui/src/tui/session_picker.rs`, 2,699 lines + 5 submodules ≈ 6,300
lines total):

- full-frame layout: vertical stack (crash banner / search bar / main) then
  horizontal **40/60 split: session list | live preview pane**
  (`session_picker.rs:2382-2470`);
- server-grouped rows (`ServerGroup` headers, orphan section, saved section);
- filter-mode cycle (`SessionFilterMode`: All / CurrentDir / CatchUp / Saved /
  **Active** / external CLIs) (`jcode-tui-session-picker/src/lib.rs:37-109`);
- tokenized search with highlight spans (`session_picker/render.rs:97-`);
- message preview with tool-call rendering (`render_preview`);
- live presence: a session-pid registry snapshot refreshed on a tick so rows
  show working/ready badges (`session_picker_presence.rs:1-40`);
- background load with cached-then-refresh (show cache instantly, rescan in
  `spawn_blocking` — `inline_interactive.rs:2655-2733`).

Contrast with omp's hub (`overlays/agent-hub.ts`, 2,589 lines): one overlay,
**two views** — a roster table of every registered agent (main + subagents,
live status, unread count, task, cost) and a per-agent chat view that tails the
agent's transcript and can prompt/steer it. omp REPLACED its separate
session-observer overlay with this (file header: "Replaces the old
SessionObserverOverlayComponent").

Red-team finding **RT-B (the redundant modal)**: the 70x20 modal is the
weakest session surface of the three projects and the user is right to call it
redundant. It shows 5 fields, no preview, no search, no grouping, no
subagents, no employee identity, no live state, and caps at 50 rows.

### 1.3 The TUI's session identity is a display string

`tui_state_impl.rs:545-555`: `current_session_id()` **returns None** ("no
stable id on App; wire when a session id is retained" — the port-decision
comment says it outright), `session_display_name()` is `session_title.clone()`.
So the TUI cannot mark the current session in a list, cannot group by anything
real, and `/rename` is the only identity operation. The agent underneath HAS
the id (`agent.session_id()`, used by the switch path) — App just never kept
it.

Red-team finding **RT-C (identity gap)**: the picker-to-be needs the current
session id on App; it exists one seam away (`adapter_types/tui_app.rs:1097`
already calls `agent.session_id()` when switching). One field + one sync point.

### 1.4 Session switch mechanism: exists and is correct

`set_session_id` (`agent/events.rs:748-794`) swaps the hot conversation slot,
persists the outgoing session, rehydrates the incoming transcript, resets the
per-session compressor. The TUI drives it via `pending_switch_session`
(tui_app.rs:1125-1147) with display rebuild. `/new` (commands.rs:665-674),
`/branch` (session_branching overlay → also `pending_switch_session`,
key_handling / channel_drains), rewind forks — all flow through the same
retarget. **This part is jcode-grade already and the plan must NOT touch it.**

### 1.5 Subagents: exist, but invisible and second-class

- `delegate_task` (`tools/sub_agent_tool.rs`): single/swarm/background modes,
  `SpawnPermit` recursion guard, cost disclosure. Real sessions with their own
  loops.
- Background workers land in `async_delegation.rs`'s process-local registry:
  `AsyncDelegationRecord { delegation_id, status, goal, model, dispatch_time,
  result, error }` — **no session id, no transcript pointer, no live progress,
  no way to open one**.
- The TUI has **no surface at all** for subagents: no roster, no transcript, no
  steer. omp's whole point (hub = one page where Main and every subagent are
  peers, `AgentRecordLike { id, kind: "main" | "sub" | "advisor", parentId,
  status, sessionFile, ... }` — `agent-hub-types.ts:39-65`) is missing.

Red-team finding **RT-D (subagents are a dead end UI-wise)**: a background
delegation can only be polled by the model via `delegate_task(query="<id>")`.
The human operator cannot see, watch, or steer a running child from the TUI.
This is the single biggest parity gap vs omp.

### 1.6 Employee identity: exists ONLY on the gateway path

- `gateway_sessions.employee_id` (wave-2 ALTER, gateway_session.rs:432) is
  bound per chat at turn time (`gateway_runner.rs:2706-2770`): `set_seat_id`,
  seat-scoped memory bank, per-seat budget lookups, rolling summaries.
- `set_seat_id` on the agent (events.rs:694) — but the TUI/CLI `sessions` table
  has **no employee/seat column at all**, and the personal-mode seat policy db
  (`org/seat_policy_db.rs`) is keyed by employee id but never linked to
  sessions.
- operant-os publishes employees (`api.rs:35-46` Employee) but sessions are
  not tagged with who executed them anywhere outside the gateway.

Red-team finding **RT-E (employee tags missing where the user wants them)**:
"who is executing the session" is a gateway-only concept today. A TUI session
run by seat `premiere` vs the operator vs a cron-derived employee is
indistinguishable in `sessions`. The plan adds `employee_id` (nullable) to the
unified record and one ALTER for the TUI table — NOT a second employee
directory (org/seat tables already exist and stay the source of truth).

### 1.7 Prior feed claims re-derived (F0066 verification)

- F0066 gap-1 "no single-instance flock on gateway run": **CLOSED** —
  `GatewayDaemonLock` + `acquire_gateway_daemon_lock()` now exist
  (gateway_runner.rs:1812-1880, flock + kernel-release semantics, exactly the
  jcode shape, including the iter-1345 process-lifetime lesson).
- F0066 gap-2 "one shared agent + global turn_lock serializes all chats":
  **STILL LIVE** — `turn_lock: tokio::sync::Mutex<()>` at gateway_runner.rs:2172
  with the documented agent-per-seat upgrade path. The live RSS numbers said
  the risk is duplication + unbounded maps, not the one-agent design; this
  audit does NOT reopen it (out of scope for session UX; the gate comment
  names the starvation signal to watch).
- F0066 gap-3 idle RAM / unbounded maps, gap-4 every surface builds its own
  agent+MCP+memory: unchanged, but both are deployment concerns, not session-UX
  concerns; the plan below notes where the unified record makes gap-4's
  thin-client seam easier later, and stops there.

## 2. Contrast summary (what each parent contributes to the plan)

| capability | jcode | omp | operant today |
|---|---|---|---|
| full-frame picker w/ list+preview | ✅ 40/60 split | ✅ (selector overlay, fillHeight) | ❌ 70x20 modal |
| search + filter modes | ✅ tokenized, 10 modes | ✅ fuzzy + history-ranked | ❌ |
| server/origin grouping | ✅ ServerGroup headers | ✅ (per-agent rows) | ❌ |
| live presence (running/idle) | ✅ pid registry + streaming badges | ✅ status via observer registry | ❌ |
| unified sessions+subagents page | ❌ (picker is sessions; swarm lives elsewhere) | ✅ AgentHub: one record kind for main+sub, chat view per agent | ❌ (no subagent surface) |
| open gesture | Left on empty input (config-gated, `inline_interactive.rs:2705-2713`), `/resume`, `/active` | **double-tap ← on empty editor** (`custom-editor.ts:1706-1712` fires `onLeftAtStart`; `input-controller.ts:763-772` counts taps in a 500ms window, `armCloseTap` hands the tap state to the hub so the same ←← that opened also closes), alt+a, ctrl+s | `/resume` typing only |
| who runs the session | ❌ | ❌ | ❌ (gateway-only employee_id) |
| switch/resume plumbing | server-side sessions | in-process session manager | ✅ already correct (set_session_id retarget) |

The user's ask maps cleanly: **jcode's pane geometry + picker mechanics,
omp's unified record/hub model and the ←← gesture, operant's existing switch
plumbing and seat/employee tags.** None of the three needs the other two's
worst parts (jcode's external-CLI import machinery, omp's IRC bus).

## 3. Findings (ranked)

1. **RT-B redundant modal** — replace `session_browser.rs` (611 lines) with a
   full-frame picker. The old modal is deleted, not wrapped.
2. **RT-D no subagent surface** — add the omp-style roster+chat unified page;
   the record is one type for main sessions, branches, and background
   delegations.
3. **RT-A record fragmentation** — one `SessionEntry` v2 struct (id, title,
   updated_at, employee_id, source/origin, parent_id, status, message_count,
   cost, tokens, cwd) with one loader; SessionBrowserState, BranchInfo, and the
   gateway's list_sessions feed it.
4. **RT-C no current-session id on App** — one field, sync at switch/new/boot.
5. **RT-E employee tags** — `sessions.employee_id` column (nullable, one
   ALTER + backfill `NULL`), set at session start from the agent's seat when
   present; group/filter rows by it in the picker.
6. **←← gesture** — port omp's detection (empty text + plain Left + 500ms
   window + tap-state handoff to the opened page so the same gesture closes).
   Files: `key_handling.rs` Left arm (the `else` branch at :1617), a tap
   counter + `Instant` on App, `prompt_input` untouched (move_left is
   already a no-op at cursor 0 — the detector lives one level up, matching
   omp's editor→controller split).
7. **Preview pane** — jcode's 40/60 split with per-session message preview;
   needs `db.list_session_messages(id, cap)` which the DB already has the
   messages table for.

## 4. What this audit deliberately does NOT include (with reasons)

- No server/daemon (jcode's harness-api-server bridge, sockets, translate
  layer): operant's sessions are same-process today; a session server is a
  deployment feature with its own risk profile, and the F0066 flock lesson
  shows the fleet already lands those as focused iterations. The unified record
  is the seam a future thin-client needs; building the server now would be
  speculative.
- No agent-per-seat gateway rework (gap-2): the gate comment names the
  starvation signal; not session-UX.
- No external-CLI importers (jcode's codex/claude/pi/opencode/cursor loaders):
  operant has no such sessions to import; YAGNI.
- No IRC bus / advisor agents (omp): no consumer here.

## 5. Risks the plan must respect

- **Peer WIP overlap**: `key_handling.rs`, `app/mod.rs`, `app/commands.rs`,
  `app/init.rs`, `tui_app.rs`, `terminal_setup.rs` are dirty with a peer's
  keyboard-enhancement work (F0094). The plan's iteration order starts with
  non-overlapping files (new picker module, DB loader) and lands the keybind
  last, re-grepping at execution time.
- **Clippy gate**: no unwrap/expect in new lib code; `#[expect]` annotations
  via the script.
- **One renderer rule**: the picker is a full-frame overlay mode (like
  journey_view/skills_view), rendered INSTEAD of the chat frame — never both
  (AGENTS TUI invariant).
- **Frozen prefix / cache**: session switching already handles memory
  `on_session_switch`; the picker must not add per-frame DB work on the chat
  path (loads are spawn_blocking + mpsc, the plan-03/05B pattern App already
  uses for the browser and model picker).
