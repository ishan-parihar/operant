# Codex Agent Command Center → operant session hub: the port

Dated-frozen: published `2026-11-13`. Investigator audit:
`docs/audit/2026-11-13-ponytail-audit-operant-session-management.md`.
Supersedes **nothing**. It is the successor to
`docs/plans/plan-2026-11-02-codex-parity-composer-and-session-organiser.md`,
whose Stages A–E all landed (iters 1644–1674) and whose "Resume here" bucket is
now empty. This plan covers what those stages deliberately left open: the tab
*axes*, the per-employee identity filter, the directory column, the
first/last-message preview and the tool-call toggle.

## The request, verbatim

> Currently, its session-picker is very broken, incomplete, and irrelevant. It
> shows a complete aggregate of all sessions across all the directories, which
> makes it confusing to navigate it. Moreover, it does not have separate tabs for
> various filters based on the status of the agentic-loop.
>
> I also want a separate filter for the agentic-os employee sessions, so that
> they can be filtered based on the employee/agent identity. Moreover, a lot of
> session critical information is missing from the session-picker in operant,
> like directory-path, and scrolling mechanism through the user-prompt, like
> first-message/last-message, while an option to hide all the tool-calls, so that
> the prompts, and the end of turn output remains.

## What the investigation found

Three of the five complaints are **data holes, not UI gaps**, and the audit
report measures each one against the live database (3,302 sessions, 73,271
messages):

| complaint | root cause | audit finding |
|---|---|---|
| "aggregate of all sessions across all directories" | `sessions.cwd` is never written by any code path; the hub also caps at 500 of 3,302 | 1, 3 |
| "no tabs for status of the agentic-loop" | `HubFilterMode`'s axes are directory/employee/branch, **not** agentic-loop status; `Interrupted` has no tab and `Live` is unreachable | 2, 4, 10 |
| "separate filter for employee sessions by identity" | `ByEmployee` is a boolean, not an identity filter | 5 |
| "directory-path missing" | same as the first row | 1 |
| "first-message/last-message scrolling" | `window_messages` keeps a tail window only; `Home` reaches the top of the window, not the session | 7 |
| "hide tool calls" | no toggle exists anywhere | 8 |

**The single most important finding for this plan:** the session hub's
organising axes are three-quarters dead *by construction*. Nothing is wrong with
the tab-bar rendering Stage A built — the filters it renders cannot match
anything. `SELECT COUNT(DISTINCT cwd) FROM sessions` = **0**.
`HubFilterMode::Active` selects `RowStatus::Live`, and the only production
assignment of `Live` is inside the `RowKind::Subagent` arm of
`refresh_subagent_rows`. So the port must start with four data fixes and only
then wire the new axes.

## Which codex surface to port from — and which to ignore

Codex has **two** session surfaces and they do different jobs:

| surface | size | what it is | port? |
|---|---|---|---|
| **Resume picker** `tui/src/resume_picker.rs` + `resume_picker/` | 10,368 lines | a standalone alt-screen `codex resume` picker: toolbar (`Filter \| Status \| Sort`, `ToolbarControl`), `SessionListDensity` comfortable/dense rows, zebra + selection backgrounds, `↑ more`/`↓ more` pagination, three-state archive, per-backend source fan-out in `sources.rs`/`catalog.rs` | **selectively.** Operant's hub is an in-TUI overlay, which the operator already prefers ("the right pane is genuinely ahead of codex's" — the 11-02 plan's finding 4). What we take: the `Show more` row, the three-state archive, the toolbar's degradation ladder, `↑ more` affordance. What we do not take: the alt screen, the N-source fan-out, the density toggle. |
| **Agent command center** `tui/src/app/agents_overview*.rs` + `agent_center/` + `agent_navigation.rs` | ~5,400 production lines | an in-TUI full-frame dashboard of background *threads*: status tabs with live counts, four grouping axes, per-row status dot, a details pane, rename, graceful removal | **YES. This is the model.** |

The operator's ask is the command center's ask, item for item. Codex's
`TASK_FILTERS` (`codex-rs/tui/src/app/agent_center/mod.rs:12-18`):

```rust
pub(super) const TASK_FILTERS: &[(&str, Option<AgentsOverviewGroup>)] = &[
    ("All", None),
    ("Needs you", Some(AgentsOverviewGroup::NeedsYou)),
    ("Working", Some(AgentsOverviewGroup::Working)),
    ("Ready", Some(AgentsOverviewGroup::Ready)),
    ("Inactive", Some(AgentsOverviewGroup::Finished)),
];
```

…is literally "separate tabs for various filters based on the status of the
agentic-loop", and `AgentsOverviewGroup::for_thread`
(`agents_overview_view.rs:79-93`) is the status derivation:

```rust
match &thread.status {
    ThreadStatus::Active { active_flags }
        if active_flags.contains(&ThreadActiveFlag::WaitingOnApproval)
            || active_flags.contains(&ThreadActiveFlag::WaitingOnUserInput) =>
    { Self::NeedsYou }
    ThreadStatus::Active { .. } => Self::Working,
    ThreadStatus::Idle => Self::Ready,
    ThreadStatus::SystemError if !is_subagent(thread) => Self::NeedsYou,
    ThreadStatus::SystemError | ThreadStatus::NotLoaded => Self::Finished,
}
```

Operant's equivalent group, from its own `RowStatus`:

```rust
NeedsYou      ← RowStatus::Interrupted  (the session that died mid-turn)
Working       ← RowStatus::Live
Ready         ← RowStatus::Idle
Inactive      ← RowStatus::Archived
```

The mapping is not 1:1 and that is fine: operant's row carries no "waiting for
your answer" flag, so `NeedsYou` maps to `Interrupted`, which is exactly the
state audit finding 4 says has no tab today. Four status tabs, one per
`RowStatus` variant, plus `All`.

## What porting directly would cost — measured

The command center is 5,400 lines of production code and 5,500 lines of tests.
It is not portable file-by-file: it is woven into codex's `App`
(`agents_overview_*.rs` are `impl App` blocks), its `AppEvent` bus, its
`ServerNotification` stream, its `BackendId` multi-backend registry and its
app-server protocol. The honest port is the **model**, not the modules.

| codex component | file:line | port directly? |
|---|---|---|
| `TASK_FILTERS` table | `agent_center/mod.rs:12-18` | **model: yes** — becomes `HubStatusTab` |
| `AgentsOverviewGroup` | `agents_overview_view.rs:70-103` | **model: yes** — becomes `HubRowGroup` |
| `AgentsOverviewGrouping` | `agents_overview_grouping.rs:12-19` | **already ported** as `HubGroupMode` (11-02 Stage C) |
| status tab render | `agent_center/render.rs:158-192` | **already ported** — `render_tab_bar` (11-02 Stage A). Keep it; change the axes it renders |
| details pane | `agents_overview_view.rs:495-616` | **selectively** — the *layout* (title / dot+status / Model / Reasoning / Project / Branch / usage / Prompt / Last message) is the design; the markdown and hyperlink machinery is codex's |
| `agent_nickname` / `agent_role` / `agent_path` | `agent_navigation.rs` | **model: yes** — operant's axis is `employee_id` + the seat roster |
| live notification feed | `agents_overview_details.rs:68-134` | **no** — operant has one process and can read the DB; the feed is codex's cross-process solution |
| multi-backend fan-out | `agent_picker.rs`, `resume_picker/sources.rs`, `catalog.rs` | **no** — operant has one backend (SQLite) |
| worktree creation, rename | `agents_overview_new.rs`, `agents_overview_view.rs:189` | **no** — out of scope |
| "Show more" | `resume_picker/sources.rs`, `agent_center/mod.rs:21-27` | **yes** — audit finding 3 |
| `RawReasoningVisibility` | `thread_transcript.rs:62` | **adapt** — the *idea* (a visibility toggle on the transcript) becomes the hide-tool-calls toggle. Codex has no tool-call hide; this is a new operant control |

---

## Part 1 — The data model (Stage F)

Four producers. No consumer changes; `SessionEntryV2` keeps its one-row-one-shape
contract (`session/listing.rs:1-14`) so the CLI and the gateway keep rendering
the same rows.

**F1. Write `cwd` on the session row.** The seat is already stamped at
turn start, in `crates/operant-core/src/agent/turn_context.rs:301-309`. Add the
cwd next to it, through a new one-line `Database::set_session_cwd` beside the
existing `Database::set_session_employee` (`database.rs:1652-1660`, which is the
pattern to copy verbatim — idempotent, no-op on an unknown id, never fails a
turn). Rows that predate the column cannot be honest about a directory they
never recorded; they stay NULL and the hub already labels them `(no cwd)`.
Regression test: `set_session_cwd_then_list_sessions_rich_returns_it`, in
`database.rs`'s test module, next to `set_session_employee_is_idempotent_and_targets_only_its_row`
(`database.rs:5272`).

**F2. Mark the streaming session `Live`.** In `replace_rows`
(`session_hub/mod.rs:530-555`), after the rows land:
`if row.id == self.current_session_id { row.status = RowStatus::Live }`. The id
comes from the source that already exists for Ctrl+B's anchor —
`SessionSwitchSource::session_id()` (`session_switch.rs:84`); the App already
holds the handle (`app/mod.rs:553`). This also fixes the branch-root
auto-detection at `mod.rs:535-540`, which searches for a `Live` row today and
always misses.

**F3. Add `RowStatus::Interrupted` to the filter cycle.** New
`HubFilterMode::Interrupted`, wired into `next()` (`mod.rs:78-87`), `label()`,
`short_label()` and `row_matches_filter` (`mod.rs:226-252`, one arm:
`matches!(r.status, RowStatus::Interrupted)`).

**F4. Put a page cursor behind `list_sessions_rich`.** Change the signature to
take a `before: Option<&str>` (the last `updated_at` seen) and push the filter
*into the SQL* rather than taking it in Rust:
`WHERE COALESCE(s.ended_at, s.started_at) < ?1 ORDER BY updated_at DESC
LIMIT ?2`, plus the count the header needs in the same connection
(`SELECT COUNT(*) FROM sessions`). All six existing filter modes already have
their axes in the schema except `CurrentDir` and `Archived`, which become
`AND (?1 IS NULL OR s.cwd = ?1)` and `AND s.archived = 0`.

> **The reason the filter must move into SQL:** `filter_counts()` counts over
> the rows actually loaded, so a tab reading 0 means "0 of the newest 500", not
> "0 in the store". The operator cannot tell a broken filter from an empty one
> from a truncated window. With the count coming from the database, the tab
> counts become store-wide and the window stays visible in the header.

## Part 2 — The tab axes (Stage G)

`HubFilterMode` keeps its six modes; they stop being the *tab bar*. Two axes,
matching codex's separation of grouping from filtering:

**Axis 1 — the status tabs, in the tab bar.** New enum, the shape of codex's
`TASK_FILTERS`:

```rust
pub enum HubStatusTab { All, NeedsYou, Working, Ready, Inactive }
pub const HUB_STATUS_TABS: &[HubStatusTab] = &[All, NeedsYou, Working, Ready, Inactive];
```

One group per `RowStatus` variant, so `HubRowGroup::for_row(row)` is five lines
and cannot drift from the status column. The tab bar renders
`render_filled_tab_bar`-style labels with counts, exactly as `render_tab_bar`
does today (`render.rs:436-556`) — **the existing tab renderer is kept
unchanged; only the enum it renders changes.** That is the smallest port of
codex's `TASK_FILTERS` and it costs the renderer nothing.

**Axis 2 — the employee filter.** `employee_filter: Option<String>` on
`SessionHubState`, set from a small picker over `SeatPolicyDb::list()`
(`org/seat_policy_db.rs:187`). `e` cycles, or a picker modal; applied inside
`row_matches_filter` so the counts and the list share one definition
(the Stage A rule: `row_matches_filter` is the one predicate). Two of the six
existing modes become sub-cases when an employee is selected: `CurrentDir`
narrows to `cwd = ? AND employee_id = ?`. When `employee_filter` is `Some`,
the tab bar gains a second clause `🪪 <name>` — codex's filter hint at the right
of the tab row (`agent_center/render.rs:172-178`).

**The old filter modes stay.** They are not replaced — `s` cycles
`All | CurrentDir | Employee | Interrupted | Active | Archived` exactly as it
does today, minus nothing, plus `Interrupted`. The status tabs are a *second*
control on the same rows, the way codex has both `TASK_FILTERS` and
`AgentsOverviewGrouping`.

## Part 3 — The columns (Stage H)

**H1. The directory column.** The row's second line already carries id,
employee, messages, tokens, cost, status and age under a degradation ladder
(`render.rs:709-735`). `cwd` is NOT a field the operator can recover anywhere
else, so it must not join that ladder's tail — it goes in a *named column*
with its header, exactly as Stage B did for `Status`. At 48 columns the heading
is `Dir` and the value is the basename, with the full path in the preview
(the same recoverability rule the id degradation uses). `Dir` heading only
appears when at least one visible row carries a `cwd`, so a legacy database
renders an honest header rather than an empty column.

**H2. The details pane's Project and Prompt lines.** Codex's details pane
(`agents_overview_view.rs:495-616`) is the design; operant's preview pane
(`render.rs:892-1146`) is its own thing and must stay (it is ahead of codex).
Add to the preview header block, above the transcript:

```text
Project  ~/documents/github/my-projects/agentic-harness/operant
```

painted from `row.cwd` — this is where the *full* path lives while the row
carries the basename.

## Part 4 — The preview (Stage I)

**I1. Bound the read.** Replace `get_session_messages_full` + `window_messages`
with `get_session_messages_preview(session_id, limit)`
(`ORDER BY id DESC LIMIT ?` then reverse) plus `count_session_user_prompts`.
Audit finding 6. The `PREVIEW_MESSAGE_LIMIT = 400` constant stays.

**I2. First message.** The tail window is a page; add a first page. Two
options, and the plan takes the cheaper one: **store the first user prompt on
the session row** (a `first_prompt` TEXT column, the way codex's `Thread.preview`
is "usually the first user message in the thread" —
`app-server-protocol/src/protocol/v2/thread_data.rs:227`). It is written once
at turn start beside `cwd`, it costs one read, and it answers the operator's
"what did I ask for?" without loading anything. The *fallback* (reading the
first N messages) is not needed.

**I3. Last message.** The tail window already ends at the last message; the
preview's `End` key lands on it when the liveness bug (F2) is fixed, because
`Home`/`End` are pane-local and the scroll bound is measured
(`mod.rs:905-918`, `render.rs:1108-1137`). No new code beyond F2.

**I4. Hide tool calls.** `hide_tool_calls: bool` on the state, `t` toggles it,
and `render.rs:1069-1074` skips the `for call in &msg.tool_calls` loop when it
is set. **This is a new feature, not a port** — codex has no tool-call hide; its
`RawReasoningVisibility` (`thread_transcript.rs:62`) hides *reasoning*. Best
complement: a third state that shows tool-call *names* only (what the pane does
today) as the middle of three states — `Prompt+output only / names / full`.

## Part 5 — The honest counts (Stage J)

**J1. Header names the window.** `Sessions (500 of 3302)` — three characters of
format string, and it converts a silent lie into a visible fact.

**J2. `Show more`.** A row at the end of the list that loads the next page over
the cursor from F4, `Enter` on it, the disabled variant is `no more`. Selection
preservation by row id is already `replace_rows`' behaviour.

**J3. Cache the counts.** `filter_counts()` recomputes 3,000 predicate calls
and one `current_dir()` syscall per frame. Cache on the state, recompute from
`rebuild_visible`. With F4 the counts also come from the database, so they are
store-wide.

## What landed (2026-11-13) — the execution record

Every row of the iteration table below was executed and pushed to `origin/main`.
The numbers moved because the fleet pushes concurrently: the plan was written
against `iter-2155`, and the work landed as **iters 2166–2189**.

| planned | landed | what actually shipped |
|---|---|---|
| 2156 F1 | **2166** | `sessions.cwd` is written at turn start (`set_session_cwd` + the call beside the seat stamp). Two regression tests. |
| 2157 F2 | **2167** | `RowStatus::Live` is marked on the streaming row (`live_session_id` + `mark_live_row`), which also un-breaks Ctrl+B's branch anchor. Five tests. |
| 2158 F3+F4 | **2180** | `HubFilterMode::Interrupted` + the page cursor (`list_sessions_rich_page`) and `count_sessions`. The seventh tab pushed the sort clause off the 46-column pane; the ladder's priority (tabs first, sort drops first) now has both halves pinned by a test. |
| 2159 H1+H2 | **2183**, **2184** | The preview's `project:` line (the FULL path). The row's directory qualifier went on the TITLE line, not the meta line — the meta line has no room, and the metric model over-estimates the row's real slack by ~3 cells. |
| 2160 G1+G2 | **2185**, **2186** | The tab bar became a STATUS ladder: all, needs you, working, ready, inactive (codex's `TASK_FILTERS` shape). The employee axis (key `e`) filters by IDENTITY, not by boolean. |
| 2161 I1+I2 | **2188** (I2 only) | The preview shows the session's FIRST prompt beside its tail — the load was reading it and `window_messages` was dropping it. **I1 (the bounded SQL read) did NOT land**: the load still reads every message and keeps the last 400. |
| 2162 I4 | **2187** | `t` hides every tool call and every tool result; the prompts and the end-of-turn output remain, and the hidden state is named in the pane. |
| 2163 J1+J2 | **2189** (J1 only) | The header says "500 of 3,302". **J2 (the `Show more` row) did NOT land**: the page cursor exists in the database layer, but nothing in the UI asks for the next page. |
| 2164 J3 | **2189** | The tab counts are computed in `rebuild_visible`, not on every frame. |

Two further iterations followed the table: **2190** committed this audit and
plan as documents, and **2191** advertised the hub's new keys — `e` and `t`
existed in the key handler and on no screen, which the repo's own standard
calls decoration the operator cannot find.

**Two planned items are still open** — I1 (the bounded preview read: the load
reads all 14,103 messages of the largest session and keeps the last 400) and
J2 (the `Show more` row, which the page cursor now makes possible). Both were
scoped out at the end of the execution window, not forgotten; the code each
needs is in place (the cursor, the count, the first-prompt capture).

## Iteration order (as planned)

Each iteration is one commit. `iter-2156` is the next free number
(`git log` on `origin/main` ends at `iter-2155`).

| iter | change | files | verifies by |
|---|---|---|---|
| 2156 | F1 — write `cwd` at turn start + the test | `agent/turn_context.rs`, `database.rs` | the new `database.rs` test |
| 2157 | F2 — mark the streaming session `Live` | `session_hub/mod.rs`, `session_switch.rs`, `app/mod.rs` | new hub test: a loaded row whose id is the agent's session id renders `Live` |
| 2158 | F3+F4 — `Interrupted` tab + SQL filter/count/page cursor | `session_hub/mod.rs`, `session/listing.rs`, `database.rs` | `filter_counts_agree_with_the_visible_set_for_each_mode` extended |
| 2159 | H1+H2 — the `Dir` column + the preview's Project line | `session_hub/render.rs` | `every_stages_claim_holds_on_one_drawn_hub` extended |
| 2160 | G1+G2 — `HubStatusTab` + the employee filter axis + the picker | `session_hub/{mod,render}.rs`, `app/key_handling.rs` | new tests: each tab's group matches the row's `RowStatus`; the employee filter narrows `CurrentDir` |
| 2161 | I1+I2 — bounded preview read + `first_prompt` on the row | `session_hub/mod.rs`, `database.rs` | new test: a 14k-message session previews without loading it all |
| 2162 | I4 — the hide-tool-calls toggle | `session_hub/{mod,render}.rs`, `app/key_handling.rs` | new test: the toggle changes the drawn buffer |
| 2163 | J1+J2 — the honest header + `Show more` | `session_hub/{mod,render}.rs` | new test: 500 rows render `500 of 3302` |
| 2164 | J3 — cached counts | `session_hub/mod.rs` | the existing count tests, unchanged |

## Constraints that must survive every stage

1. **The preview pane is preserved.** It is ahead of codex's (the 11-02 plan's
   finding 4). Every stage here *adds* to it; none replaces it.
2. **`row_matches_filter` stays the one predicate.** No axis may define
   "what does this filter mean" twice — Stage A's rule, and the reason the tab
   counts cannot lie.
3. **`HubGroupMode` is untouched.** Grouping is not filtering, and it is already
   ported (Stage C). The four status tabs are a second control, not a rewrite
   of the grouping ladder.
4. **No new dependency.** The port uses `SeatPolicyDb`, the existing rusqlite
   connection and the existing `theme_colors`. Nothing from codex is vendored.
5. **Hub keys keep working.** The 18 existing bindings stay; the new ones are
   `e` (employee), `t` (tool calls) and `Enter` on `Show more`. The undiscoverable
   `a` (archive) gains a footer affordance in the same pass (audit finding 10;
   the binding is at `app/key_handling.rs:739-763`).

## What "done" looks like — operator-visible acceptance

1. Open the hub on the local database. The header reads `Sessions (500 of 3302)`.
2. The tab bar shows five status tabs, each with a store-wide count, one per
   `RowStatus` variant — and `Interrupted` names the 22 sessions the operator
   could not find before.
3. `e` opens a small employee picker over the seat roster; selecting one makes
   every count and every row that employee's sessions only.
4. `g` still cycles `Auto | directory | employee | status`; under `directory`
   the groups now have real keys because `cwd` is written, and the rows carry a
   `Dir` column.
5. The preview's header block shows the full `Project` path and, for a long
   session, `Home` reaches the first prompt (I2's stored field) and `End`
   reaches the last message.
6. `t` cycles `names → hidden` and the prompts and the end-of-turn answers
   become the readable content of the pane.

## Manual work left for the operator after the automated part

- **Register this plan in `genome.toml`'s `core.living_plans`** (it is currently
  not). The plan carries open iterations, so it takes in-place amendments under
  that key rather than a v2. `./scripts/genome lint` passes either way — the
  registration is a governance choice, not a lint gate, which is why it is left
  to the operator rather than done by the investigator.
- **Regenerate the scenario corpus goldens for `/branch` and the session hub.**
  The 11-02 plan's own closing note: the corpus goldens are pre-Stage-A (iter-1467),
  `_run.py verify --scenario session-hub` reports 3 drifts, and the `simulate`
  harness cannot open the hub at all because the sandboxed config has no provider.
  Run it with a provider configured.
- **Confirm the seat display names.** Finding 11: the hub prints the raw
  `employee_id`; the org's `SeatPolicyDb` holds the names. The plan assumes the
  join is acceptable; if the roster should be the operator's own list rather
  than the DB's, that is a policy decision, not a code one.
- **Release an iteration, then decide `Archived`'s fate.** Audit finding 10: 0
  of 3,302 sessions are archived. After the `a` affordance ships, if the tab is
  still 0, cut the tab and the archive action rather than keep a tab that always
  says 0.
- **Deploy and eyeball it.** Every stage's claim is on the drawn buffer; the
  headless sim can render the hub once a provider is configured, which is how
  findings 1, 2, 10 and 14 get caught (they are all invisible in source and
  obvious on screen).
