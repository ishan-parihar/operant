# ponytail-audit — operant's session-management and agent-management surface

Audited on `main` at `d71f642f` (iter-2155), against `../external-repos/codex`
(codex-rs at the checkout present on this host).

## What this repo does

Operant is a Rust ratatui coding-agent CLI. It records every agent turn into a
local SQLite database (`~/.operant/database.db`) and offers TUI panels over
those records: a full-frame **session hub** (`tui/session_hub/`) for browsing,
filtering, previewing and switching sessions, and an **agents menu**
(`tui/agents_view.rs`) for editing agent definition files under
`.operant/agents/`.

Load assumed: ONE operator, ONE terminal, ONE local SQLite database, one
`operant` process streaming at most one turn at a time. Measured against that
load on this host: **3,302 sessions, 73,271 messages, 3 employee-tagged
sessions, 3 distinct employees, 0 distinct cwds, 0 archived sessions.**

Surface counts (the concepts a reader must hold):

| axis | operant count |
|---|---|
| row fields (`SessionEntryV2`) | 12 |
| row kinds (`RowKind`) | 3 (+4 sub-fields on the third) |
| statuses (`RowStatus`) | 4 |
| filter modes (`HubFilterMode`) | 6 |
| grouping modes (`HubGroupMode`) | 4 |
| sort keys (`HubSortMode`) | 4 |
| panes (`PaneFocus`) | 2 |
| hub key bindings | 18 |
| production files in the panel | 6 (`session_hub/{mod,render,row_style}.rs`, `session_switch.rs`, `agents_view.rs`, + the app-side loader and key handler) |
| panel line count | 4,900 (`session_hub` 4,790 incl. 2,377 tests; `agents_view` 923) |
| named hub tests | 98 |

---

## Must fix

### 1. `sessions.cwd` is never written, so the directory filter and the directory grouping can never match anything

`crates/operant-core/src/database.rs:688-709` (`save_session`),
`crates/operant-core/src/agent/run.rs:2634` (the live turn path),
`crates/operant-core/src/agent/rewind.rs:148` (the only `save_session_full`
caller)

- **What this is:** The `sessions` table has a `cwd` column. The hub reads it
  into `SessionEntryV2.cwd` and uses it for `HubFilterMode::CurrentDir`
  (`session_hub/mod.rs:234`) and `HubGroupMode::Directory`
  (`session_hub/mod.rs:568`).
- **Problem:** No production code path ever writes that column. The live turn
  path calls `save_session`, a five-column INSERT whose statement is
  `INSERT INTO sessions (id, title, source, started_at, ended_at)` — `cwd` is
  not in the column list and not in the parameter list. The only runtime caller
  of `save_session_full` (which *does* list `cwd`) is the rewind fork, and it
  hardcodes `cwd: None` at `rewind.rs:148`. Measured on the live database:
  `SELECT COUNT(DISTINCT cwd) FROM sessions` = **0**, and `cwd IS NULL` on all
  **3,302** rows.
- **Fix:** Write it where the seat is already stamped, one UPDATE in
  `agent/turn_context.rs:301-309`:
  `UPDATE sessions SET cwd = ?2 WHERE id = ?1`, called with the agent's cwd
  right after `set_session_employee`. Rows that existed before the column was
  captured cannot be honest about a directory they never recorded — they stay
  NULL, and the hub already labels them `(no cwd)`.
- **If we skip it:** "current dir" is a permanent 0 and "directory" is one
  permanent `(no cwd)` group, so the operator's "show me this project's
  sessions" question has no answer, and the directory column the operator
  asked for would be a column of empty cells.

### 2. A *session* row can never be `RowStatus::Live`, so the "active" tab is always empty and the ⚡ glyph is dead

`crates/operant-cli/src/tui/session_hub/mod.rs:236-245` (the `Active` filter),
`crates/operant-core/src/database.rs:1115-1117` (the comment that claims it)

- **What this is:** `RowStatus::Live` is documented as "the session this process
  is actively streaming into right now", and `HubFilterMode::Active` selects
  `matches!(r.status, RowStatus::Live)`.
- **Problem:** The only production assignment of `RowStatus::Live` is inside the
  `RowKind::Subagent` arm of `refresh_subagent_rows` (`mod.rs:500`) — subagent
  rows only. No code path ever marks a *session* row `Live`. The core comment
  at `database.rs:1115-1117` says "the hub sets `Live` on the current session id
  after loading; the DB cannot know it" — that hub code does not exist. The
  follow-on damage: `replace_rows`' automatic branch-root detection
  (`mod.rs:535-540`) searches for a `Live` row to anchor Ctrl+B's
  "this branch's tree" filter, and that search always misses.
- **Fix:** One assignment, in `replace_rows` (or in the app's channel drain
  right after it): `if row.id == current_session_id { row.status = RowStatus::Live }`.
  The current session id is already available through
  `SessionSwitchSource::session_id()` (`session_switch.rs:84`).
- **If we skip it:** The one tab that answers "what is running right now"
  always says 0, and the operator cannot find their own live session in a
  500-row list.

### 3. The hub silently shows only the newest 500 of 3,302 sessions

`crates/operant-cli/src/tui/app/mod.rs:1932` (`.list_sessions_rich(500)`),
`crates/operant-core/src/database.rs:1064-1075` (the SQL)

- **What this is:** Opening the hub loads `list_sessions_rich(500)` and renders
  exactly those rows; all tab counts are computed over them.
- **Problem:** The SQL applies `LIMIT 500` *after* the `GROUP BY`, so the 500
  are the 500 most recent and **2,802 sessions (85% of the store) are
  unreachable from the hub, permanently.** Worse, `filter_counts()`
  (`mod.rs:672-700`) counts within that 500-row window, so a tab reading
  `interrupted 0` would mean "0 of the newest 500", not "0 in the store". The
  operator cannot tell the difference between "this filter is broken" and
  "this filter has nothing" and "this filter's rows are older than the cutoff".
- **Fix:** Two lines. (a) The header must name the window: `Sessions (500 of
  3302)`. (b) Add a `Show more` row at the end of the list — codex's resume
  picker does exactly this with `↑ more` / `↓ more` over its per-source cursor
  (`resume_picker/sources.rs`, `catalog.rs`); operant needs one
  `WHERE COALESCE(ended_at,started_at) < ?` page cursor, not codex's N-source
  fan-out.
- **If we skip it:** The operator believes they have seen everything. A session
  they need is invisible, and the tab count lies about the store.

### 4. `RowStatus::Interrupted` — the state the operator opens the hub to find — has no filter

`crates/operant-cli/src/tui/session_hub/mod.rs:62-75` (the mode list),
`crates/operant-core/src/session/listing.rs:30-41` (the status enum)

- **What this is:** `RowStatus` carries four values; `Interrupted` is the
  persisted `end_reason = 'interrupted'` marker that says "this session was
  live when the owning process went away".
- **Problem:** `HubFilterMode` cycles `All | CurrentDir | ByEmployee | Active |
  CurrentBranch | Archived`. `Interrupted` is in no mode. Measured on the live
  database: **22 sessions carry `end_reason = 'interrupted'`**, and the only
  way to see them in the hub is to read all 500 rows by eye for a ⚠-adjacent
  status label.
- **Fix:** Add `HubFilterMode::Interrupted` to the enum, the `next()` cycle,
  `label()`, `short_label()` and `row_matches_filter` — five lines — and put it
  between `Active` and `CurrentBranch`, which is where the operator's question
  sits (what needs me, then what was cut off).
- **If we skip it:** The hub's own list says `Interrupted` is worth a dedicated
  glyph (`render.rs:214`) and a distinct doc comment, but the only way to find
  them is to scan.

### 5. The employee filter cannot select *which* employee

`crates/operant-cli/src/tui/session_hub/mod.rs:67`, `mod.rs:235`

- **What this is:** `HubFilterMode::ByEmployee` matches any row that has an
  `employee_id`. It is a *boolean*, not an identity filter.
- **Problem:** With 3 distinct employees on the live database, the operator
  cannot say "show me only `emp-cron_fa1e44c`". `HubGroupMode::Employee`
  (`mod.rs:569`) *groups* by identity, but grouping is not filtering: the
  operator still reads every employee's rows to find one employee's.
- **Fix:** A second, orthogonal axis — `employee_filter: Option<String>` on the
  state, set from a small picker over `SeatPolicyDb::list()`
  (`org/seat_policy_db.rs:187`, which already returns
  `Vec<(String, SeatPolicy)>`), applied inside the same `row_matches_filter`
  predicate so the tab counts stay honest. This is what operant needs to answer
  "agentic-os employee sessions" that codex answers through
  `agent_nickname` / `agent_role` on the thread record.
- **If we skip it:** The employee tab returns a mixed list of every employee
  plus nothing else, which reads as a broken filter rather than a coarse one.

---

## Should fix

### 6. The preview loads every message of a session, then keeps the last 400

`crates/operant-cli/src/tui/app/mod.rs:1966-1977`,
`crates/operant-core/src/database.rs:924-964` (`get_session_messages_full`),
`crates/operant-cli/src/tui/session_hub/mod.rs:1024-1031` (`window_messages`)

- **What this is:** Selecting a row loads the session's *entire* stored
  transcript — every column: `content`, `reasoning`, `reasoning_content`,
  `reasoning_details`, `codex_reasoning_items`, `codex_message_items` — and then
  `window_messages` throws away everything but the trailing 400 messages.
- **Problem:** Measured on the live database, the largest session holds
  **14,103 messages** and the whole table holds 73,271. Every preview selection
  on a large session materialises all of it to show 400. The largest sessions
  are exactly the ones an operator selects.
- **Fix:** One bounded read:
  `SELECT … FROM messages WHERE session_id = ?1 ORDER BY id DESC LIMIT 400`,
  then reverse. The dropped-prompt count becomes a second cheap
  `SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = 'user'`.
- **If we skip it:** The hub's preview can allocate hundreds of megabytes and
  stall the frame, on the sessions the operator most wants to inspect.

### 7. The preview cannot reach the FIRST user message

`crates/operant-cli/src/tui/session_hub/mod.rs:1024-1031`

- **What this is:** `window_messages` keeps the trailing 400 messages and
  counts how many user prompts fell off the head; the pane prints
  "— N earlier prompt(s) not shown (tail window) —".
- **Problem:** For a 14,103-message session, the first prompt is unreachable
  forever. `Home` on the preview jumps to the top of the *tail window*, not the
  top of the session. So the picker cannot answer "what did I originally ask
  for?" — which is the question a session picker exists to answer.
- **Fix:** Two moves: keep the tail window but report it as a page, and add a
  "first message" load — a bound of the first N user prompts with their ages.
  Codex's Thread record stores the answer on the row itself
  (`app-server-protocol/src/protocol/v2/thread_data.rs`: `preview` — "usually
  the first user message"), which is the cheaper port: store it once on the
  `sessions` row rather than re-deriving it.
- **If we skip it:** The operator reads the last 400 messages of a 14,103
  message session and never sees what the work was for.

### 8. Tool calls are always rendered in the preview and can never be hidden

`crates/operant-cli/src/tui/session_hub/render.rs:1069-1074`

- **What this is:** Any message with tool calls prints `[name] [name] …`
  prefixes inline between the prompt text.
- **Problem:** On a 400-message tail window that is hundreds of
  `[read_file] [bash] [edit_file]` fragments separating the prompts and the
  end-of-turn answers. There is no toggle, so the operator's "hide the tool
  calls, show me the prompts and the output" request has no control.
- **Fix:** One `hide_tool_calls: bool` on `SessionHubState`, one key
  (`t`), and skip the `for call in &msg.tool_calls` loop when it is set. Note
  for the plan: codex does **not** have this toggle — its transcript carries
  `RawReasoningVisibility::{Hidden, Visible}`
  (`thread_transcript.rs:62`) which hides *reasoning*, and it *groups* adjacent
  tool calls (`history_cell/activity_group.rs`) rather than hiding them. So the
  hide-tool-calls control is a new operant feature, not a port.
- **If we skip it:** The preview pane is mostly tool-call noise; the prompt and
  the final answer are the two hardest things to find in it.

### 9. `filter_counts()` runs a syscall and 3,000 predicate calls every frame

`crates/operant-cli/src/tui/session_hub/mod.rs:672-700`,
`crates/operant-cli/src/tui/session_hub/render.rs:436` (`render_tab_bar`)

- **What this is:** The tab bar is re-rendered on every frame; it calls
  `filter_counts()`, which calls `std::env::current_dir()` and then filters the
  whole row set once per mode.
- **Problem:** `current_dir()` is a filesystem syscall per frame (twice — once
  here, once in `rebuild_visible` at `mod.rs:731`). At 500 rows × 6 modes that
  is 3,000 predicate calls per frame, 60 times a second, for numbers that only
  change when the row set changes — on a load, or one keystroke.
- **Fix:** Cache the counts on the state and recompute them from
  `rebuild_visible`, which is already the single place `visible_idx` changes.
- **If we skip it:** Steady CPU burn in a panel that should be idle, and a
  one-frame disagreement between the count and the list.

### 10. Almost every hub tab is permanently zero, and the counts read as data

`crates/operant-cli/src/tui/session_hub/mod.rs:62-75`, `mod.rs:672-700`

- **What this is:** The six tabs are `all 3302-cap, current dir 0, by employee
  0, active 0, this branch's tree N, archived 0`.
- **Problem:** Measured against the live database, three of the six tabs are
  *structurally* zero and not because the data is empty: `current dir` is zero
  because `cwd` is never written (finding 1), `active` is zero because no
  session is ever marked `Live` (finding 2), and `archived` is zero because
  `archived = 1` on **0 of 3,302** sessions. The tab bar's own doc comment
  ("`active 0` is the answer to 'is anything running'") assumes the answer is
  meaningful; here it is a constant.
- **Fix:** Two things. Make the counts honest by fixing findings 1, 2 and 4 —
  `active`, `current dir` and `interrupted` then carry real numbers. Then
  measure again: if `archived` is still 0 after the archive key is advertised,
  either the feature is not wanted (cut the tab and the archive action) or the
  key is undiscoverable (`a` is bound at `key_handling.rs:739-801` but appears
  nowhere in the hub's chrome, which advertises only `s` and `g`).
- **If we skip it:** The operator learns that a tab reading 0 means nothing,
  and stops reading the tab bar at all — which was the complaint that started
  this work.

### 11. The hub prints the raw `employee_id` string; the org's display name is never joined

`crates/operant-core/src/session/listing.rs:85-87`,
`crates/operant-core/src/org/seat_policy_db.rs:187`,
`crates/operant-cli/src/tui/session_hub/render.rs:669-674`

- **What this is:** The hub tags a row with `sessions.employee_id` and prints it
  as `🪪 emp-cron_fa1e44c`. The seat roster lives in a separate database with
  the policy and the human-readable fields.
- **Problem:** The two are never joined. The hub's own doc comment
  (`listing.rs:10-14`) says "org/seat_policy_db stays the source of truth for
  names" — no code reads it. On the live database all three tagged rows happen
  to use the id as the tag, so the missing join is invisible in the current
  data; the moment a real seat name differs from the id, the hub shows an
  opaque handle where the org knows a name.
- **Fix:** One lookup at row-load time: `SeatPolicyDb::list()` is already a
  Vec in memory; map `id → display name` and show the name with the id as the
  tooltip/detail. The preview pane already prints the employee on its own line
  (`render.rs:1022-1027`) — that is where the full id stays.
- **If we skip it:** The operator cannot tell which agent they are looking at
  without decoding a raw id, which is the whole point of the employee filter.

### 12. "Agent" means three unrelated things across the two panels

`crates/operant-cli/src/tui/agents_view.rs:26-86`,
`crates/operant-cli/src/tui/session_hub/mod.rs:236`,
`crates/operant-core/src/session/listing.rs:100-120`

- **What this is:** (a) the agents menu's "agent" is an agent *definition file*
  (`.operant/agents/*.md`, `load_agent_definitions` at `agents_view.rs:365`);
  (b) the hub's "agent" is the `employee_id` *seat* stamped on a session row;
  (c) the hub's "subagent" is a *live delegation* worker in the process-local
  registry.
- **Problem:** Three unrelated concepts, no shared type, and the agent panel's
  `AgentStatus` enum (`Idle | Running | WaitingForTool | Complete | Failed`,
  `agents_view.rs:26-32`) duplicates a status vocabulary the hub already has in
  `RowStatus` and `RowKind::Subagent.status`, with *different labels for the
  same word*: the panel says `done` and the hub says `idle`; the panel says
  `failed` and the hub's row says `failed`; the panel has no `archived` and the
  hub does.
- **Fix:** Keep the definition-file panel separate (it is a different thing and
  should stay), but give the two *runtime* statuses one owner. One enum,
  exported from core, used by both the subagent rows and any live roster.
- **If we skip it:** An operator learns two status vocabularies and cannot tell
  which one a panel is speaking, which is how "agentic-loop status" ends up
  unanswerable.

### 13. `AgentInfo` cannot be joined to a session, and the codex port target has a real one

`crates/operant-cli/src/tui/agents_view.rs:58-63`,
`crates/operant-cli/src/tui/app/commands.rs:2276-2289`,
`crates/operant-cli/src/tui/agents_view.rs:604-627`

- **What this is:** `AgentInfo { name, status }` is built from the app's
  `agent_status` map and rendered as a three-line "Active now" band at the top
  of the agents list (`take(3)`).
- **Problem:** It carries no session id, no cwd, no parent and no identity
  beyond a display name, so it can never be joined to anything in the hub and
  can never answer "which session is this agent running?". It also silently
  truncates at 3. Codex's equivalent (`agent_navigation.rs`,
  `AgentPickerThreadEntry { agent_nickname, agent_role, agent_path, is_running,
  is_closed }`) carries the thread id and the agent path for exactly this
  reason — the popover can route to the thread.
- **Fix:** Either give `AgentInfo` the session id and cwd so it deep-links
  into the hub, or delete it and let the hub's subagent rows be the one live
  agent surface. Do not keep an unjoinable third list.
- **If we skip it:** The operator has three lists of agents and none of them
  opens the thing they are looking at.

---

## Nice to have

### 14. The row's session id shrinks to 3 characters and becomes unusable

`crates/operant-cli/src/tui/session_hub/render.rs:730-737`

- **What this is:** When the metrics do not fit a 46-column pane, the id budget
  shrinks from 12 characters toward 3.
- **Problem:** A 3-character prefix is not a handle. Two sessions can share it,
  and `/resume` / `sessions show` need the full string. The degradation rule
  (mod.rs:703-708) drops the status first and the id last, but the id's floor
  is lower than "still reads as a handle".
- **Fix:** Raise the floor to 6 and drop the id to nothing below it — the
  preview prints it in full, which is the recoverability argument the rule
  already uses.
- **If we skip it:** An unreadable id fragment is worse than no id.

### 15. The list pane is a fixed 40% at every width, which is what forced the id truncation

`crates/operant-cli/src/tui/session_hub/render.rs:361-367`

- **What this is:** A fixed 40/60 horizontal split of the body.
- **Problem:** At 120 columns the list is 48 wide — the exact width the plan's
  own validation pass found truncating rows mid-word (`docs/plans/
  plan-2026-11-02-codex-parity-composer-and-session-organiser.md:869-899`).
  Codex's command center uses `Constraint::Min(46)` plus a 38-column details
  pane and *drops* the details pane below 90 columns (`agent_center/render.rs:60-69`).
- **Fix:** `Min(56)` for the list, and hide the preview pane below ~100 columns
  rather than squeezing both.
- **If we skip it:** Every row of a session list is a compromise between its
  own columns.

### 16. The hub never re-reads the row set while it is open

`crates/operant-cli/src/tui/app/mod.rs:1924-1943`

- **What this is:** Rows are loaded once when the hub opens.
- **Problem:** A session started in another process while the hub is open never
  appears, and the `Live` marking (finding 2) cannot be current. Codex refreshes
  the thread list on a timer and on server notifications
  (`agent_picker.rs:refresh_agent_picker_threads`), and operant's hub already has
  a per-frame tick it could hang the refresh off.
- **Fix:** Re-run the load on a long cadence (every 10s) while the hub is
  visible, preserving the selection by row id — `replace_rows` already does
  exactly that preservation.
- **If we skip it:** A hub left open is a stale view of a moving system.

### 17. The search haystack includes a field that is always NULL

`crates/operant-cli/src/tui/session_hub/mod.rs:766-769`

- **What this is:** The search haystack is built from id, source, employee,
  title, cwd and the delegation goal, AND-matched.
- **Problem:** `cwd` is NULL in every row (finding 1), so the cwd term can
  never match and the haystack pays for the branch on every row. Searching for
  a directory path — the operator's first instinct — silently fails.
- **Fix:** Free with finding 1. Until then the search should not claim to cover
  it.
- **If we skip it:** The operator searches for `my-project`, gets nothing, and
  concludes the session does not exist.

---

## Recorded data — who reads each field and what it changes

`SessionEntryV2` is the one projection every surface renders (`session_hub`,
`cmd_sessions`, `gateway_api`), so the audit has to check the whole row, not the
hub:

| field | consumer | decision it changes | cadence |
|---|---|---|---|
| `id` | resume/switch, `/resume` completion, CLI `sessions show` | which session the agent retargets to | written at session create, read at switch |
| `parent_id` | `RowKind::Branch`, Ctrl+B's CurrentBranch anchor | which rows the branch filter shows | written at fork |
| `title` | row title, search haystack | what the operator reads in the list | updated per turn's `save_session` |
| `updated_at` | default sort, `Auto` sort key, age column, `Sort: cost/messages/tokens` tie-break | the order of 500 rows | written per turn |
| `employee_id` | 🪪 badge, `ByEmployee` filter (boolean only), search haystack | which rows an operator attributes to an agent | written once at turn start (`turn_context.rs:301`) when a seat is set; NULL otherwise — honest, and the operator's own sessions are NULL by design |
| `source` | row meta, search haystack, `cmd_sessions --source` | where the row came from | written at create |
| `message_count` | `N msg` column, `Sort: messages` | which sessions were busiest | incremented per message write |
| `cost_usd` | `$cost` column, `Sort: cost` | where the money went | written per turn via `update_session_cost` |
| `tokens` | `k tok` column, `Sort: tokens` | which sessions ate the context | written per turn |
| `cwd` | **nothing, today** — `CurrentDir` filter, `Directory` group, search haystack | it *would* change "show me this directory" | **never written** (finding 1) |
| `status` | status column, status grouping, `Active` filter | what needs attention | derived at read time; `Live` unreachable for sessions (finding 2) |
| `kind` | row title badge, `HubSelection::FocusSubagent`, the Subagent branch in every render | Enter's action | `Branch` from `parent_id`; `Subagent` from the in-process registry |

`cwd` is the one field with consumers and no producer — the audit's clearest
case. Everything else has at least one real reader and one real writer.

---

## Verdict

**Correctness first, and it is a data bug, not a UI bug.** The session hub's
organising axes are three-quarters dead *by construction*: `cwd` is never
written (finding 1), `Live` is never set on a session row (finding 2), the store
is silently capped at 500 of 3,302 rows (finding 3), and 22 interrupted
sessions have no filter (finding 4). Fix the four producers/paths and the six
existing tab axes start answering real questions; then add the per-employee
identity filter (finding 5) and the preview controls (findings 7, 8) that the
operator asked for.

The codex port target is the **Agents Overview / "Agent command center"**
(`codex-rs/tui/src/app/agents_overview*.rs` + `agent_center/`), not the resume
picker: its `TASK_FILTERS` (`agent_center/mod.rs:12-18`) is the
status-tab model the operator is asking for, its `AgentsOverviewGrouping`
(`agents_overview_grouping.rs:12-19`) is the four-axis grouping model operant's
`HubGroupMode` already copied, and its details pane
(`agents_overview_view.rs:495-616`) is where directory, prompt, model and last
message live. Port plan: `docs/plans/plan-2026-11-13-codex-command-center-port.md`.

## Lean

**-34 lines, 0 dependencies possible** from the unused/duplicate surface:
`AgentInfo` + `AgentStatus` + its `commands.rs` builder and `render_agents_list`
band (34 lines, `agents_view.rs:26-63`, `58-63`, `604-627`,
`commands.rs:2276-2289`) if finding 13 resolves toward deletion; the
`HubGroupMode::Directory` / `HubFilterMode::CurrentDir` axis pair (8 lines)
becomes live again with finding 1 rather than deleted. No second dependency does
anything this surface needs.

## Not checked

- The gateway's own session API (`gateway_api.rs`, `gateway_session.rs`,
  `gateway_commands.rs`) reads the same `list_sessions_rich`; its callers and
  limits were not audited beyond noting that they share the one loader.
- `cmd_sessions.rs` (the CLI subcommands) was not audited line by line.
- Nothing was compiled or tested in this pass — the audit is read-only against
  the source at `d71f642f` plus the live database. All measurements come from
  direct SQLite queries against `~/.operant/database.db`, not from a test run.
- `agents_view.rs`'s editor (routing 200-535, 646-923) was read only for its
  data model and its live-agent band, not audited for editor-key correctness.
- Codex was read for its session-management and agent-management surfaces
  only; its composer, exec, networking and MCP surfaces were not audited.
