# Plan — the session hub is a global list; the operator needs three surfaces

**Date:** 2026-11-15 (revision 3 — supersedes revisions 1-2. Revision 2
mis-cited oh-my-pi's `←` gesture and assumed a parent→child carrier that does not
exist. **Revision 3 resolves the open question**: `Shift+Tab` becomes the agent
identity switch and the effort ladder moves to the models page, planned
separately in
[plan-2026-11-15-model-picker-hub-and-effort.md](plan-2026-11-15-model-picker-hub-and-effort.md).)
**Base:** `origin/main` @ `59bab8e0`
**Status:** outline. Writing this file executes nothing.

> **Read with its sibling.** The effort ladder is not deleted by this plan — it
> is *moved*, and the surface it moves to is specified in the model-picker plan.
> T4 below frees `Shift+Tab`; the sibling plan's M1 provides the destination.
> **Land the sibling's M1 before or with T4**, or the operator loses effort
> control with nothing to replace it.

## What the operator asked for

1. The session picker is "fragmented, irrelevant and incomplete" — it cannot
   give the preview needed to choose a session, an employee, or a directory.
2. A filter for employee / agent identities.
3. `Shift+Tab` **and** slash commands that change the identity of the agent the
   operator is interacting with, for agentic-os operationalisation.
4. Session-critical information is missing: the directory path is clipped, and
   there is no separate tab for current-directory sessions.
5. `<-` opens the session picker on a new session, and the subagent
   picker/navigator on the active session, "just like oh-my-pi implements it".

## The structural finding this revision turns on

**Operant has one surface where oh-my-pi has two, and the operator is asking for
both.**

Operant's session hub is a **global** list: every session in the store, all
directories, all agents, with a preview pane. Subagent rows are appended into
that same flat list from the in-process delegation registry.

oh-my-pi's Agent Hub is a **session-scoped** roster: the subagents belonging to
the current session, and explicitly *not* the main agent — "The main agent is not
listed because its conversation is the ambient session view"
(`oh-my-pi/docs/agent-hub.md:3`).

So the subagent navigator is **not a filter on the global list**. Building it as
a filter would put other sessions' subagents into a list that claims to be this
session's children — the same category error the closed plan fixed when it split
the status question from the scope questions. It is a second surface with its own
scope, its own opener, and its own parent→child edge.

## Reference implementations — what each one owns

### oh-my-pi — Agent Hub (the closest match; read directly, then confirmed by a
### separate read-only study)

Source: `oh-my-pi/docs/agent-hub.md`, plus the study's file:line trace through
`packages/tui` and `packages/coding-agent`. oh-my-pi is TypeScript on Bun with an
in-house TUI; nothing relevant is in its Rust crates.

| Fact | Evidence |
|---|---|
| Scope is the current session's subagents; main agent excluded | `agent-hub.md:3`; `#sessionRefs()` filters `ref.id !== MAIN_AGENT_ID` (`overlays/agent-hub.ts:1656-1658`) |
| Openers: `Alt+A` (`app.agents.hub`), `Ctrl+S` (`app.session.observe`), double-tap `←` | `agent-hub.md:12-16`; defaults `app-keybindings.ts:228-235` |
| The `←` gesture is **double-tap only**, window `[40ms, 500ms)`, 3rd tap = burst, never fires | `input-controller.ts:1003-1020`, constants `:275-276` |
| `←` opens the **Agent Hub**, not the session-resume picker | `input-controller.ts:804-813`; the resume picker is `app.session.resume`, **unbound by default** (`app-keybindings.ts:224-227`) |
| While a subagent is focused, double-tap `←` **returns to the main session** | `input-controller.ts:849-853`; listener `:376-385` |
| The hub refuses to mount on an **empty** roster (`requireContent`), and defers if persisted discovery is still running | `selector-controller.ts:2683-2699` |
| `t` toggles a flat roster and a **parent/child tree** | `agent-hub.md:36`; `#toggleViewMode` `agent-hub.ts:2503-2508` |
| `/` filters agents by id or display name | `agent-hub.md:37`; fuzzy match `agent-hub.ts:1681-1685` |
| Enter **switches the main view into** that agent's session (not a modal viewer), reviving a parked/killed agent | `agent-hub.ts:2609-2633`; `session-focus-controller.ts:88-134` |
| Rows are **rank-frozen** so live arrivals never reshuffle under the cursor | `agent-hub.ts:231-244`, `:1666-1680` |
| Roster discovers **parked** subagents from the session's persisted artifacts on resume | `agent-hub.md:7`, `:100`; `persisted-agents.ts:666-723` |
| Missing data renders as `usage —`, never an estimate | `agent-hub.md:70`; fallback `usage · <age>` `agent-hub.ts:2330` |
| `/agents` is a **different** surface — the agent-definition/settings hub, not the live roster | `agent-hub.md:117` |
| `/switch` is the session-only identity switch: "Switch model for this session only … accepts fuzzy ids, provider/id, `@role`, `:level`" | `slash-commands/builtin-modes.ts:457-463` |
| `Shift+Tab` is listed beside `/switch` and `/effort` as a **session-only** switch | `agent-hub.md:78` |
| Parent→child link is `AgentRef.parentId` **plus a filesystem convention** — `<root>/<agentId>.jsonl` — with no database | `agent-registry.ts:72`; `persisted-agents.ts:904-916` |

Four of these decide the shape of this plan:

- **`←` in omp opens the subagent navigator only.** There is no omp precedent for
  `←` opening a *session* picker — `app.session.resume` is unbound. So item 5's
  first half is an **operant extension**, and this plan must not claim omp
  supports it. The second half (navigator on the active session) is omp's, and
  operant copies it.
- **`/agents` and the live roster are deliberately separate in omp too.** Operant's
  `/agents` is already the definition hub (`AgentsMenuState`, `agents_view.rs:202-211`
  — `Vec<AgentDefinition>` from project files). It must not become the identity
  switch; the identity switch is a third thing.
- **`Shift+Tab` in omp is session-scoped, not global.** Operant's `Shift+Tab` is
  the effort ladder (`key_handling.rs:2153` → `cycle_effort`), global to the
  process. That is the divergence item 3 asks to close.
- **The tap-state handoff already exists in operant.** omp's `armCloseTap`
  (`agent-hub.ts:666-677`) — whoever consumes the opening gesture primes the next
  detector, so the operator is not trapped behind a key needing a second press —
  is `SessionHubState.armed_close_tap`, ported already.

### codex — two surfaces, not one

Codex has a **resume picker** (`codex-rs/tui/src/resume_picker.rs`, 8,278 lines,
plus `resume_picker/{layout,sources,catalog,page_loading,archive}.rs`) and a
separate **command center** (`app/agent_center/*` over
`agents_overview*.rs`). They are different surfaces with different jobs, and
conflating them is how operant's hub ended up as one page doing both.

| Fact | Evidence |
|---|---|
| The resume picker has **no side pane** — layout is header/toolbar/search/list/footer | `resume_picker/layout.rs:18-51` |
| Its preview is **inline row expansion** (Ctrl+E) or a **full-screen transcript overlay** (Ctrl+T) | `resume_picker.rs:2410-2434`, `pager_overlay/transcript.rs` |
| The side-by-side detail pane is the **command center's**, and only when `body.width >= 90` (`[Min(46), Length(3), Length(38)]`) | `app/agent_center/render.rs:52-84` |
| That detail pane is **pinned — it has no scroll state at all**; only the list scrolls | `agents_overview_view.rs:495-616`, `:163` |
| `TASK_FILTERS = [All, Needs you, Working, Ready, Inactive]` — already ported to operant's status tabs | `app/agent_center/mod.rs:12-19` (verified directly) |
| Grouping `Project (default) / Directory / Status / Model`, cycled with `g` | `agents_overview_grouping.rs:12-19`; `agents_overview_view.rs:766-779` |
| The **Project** key is `(repository_identity().common_dir, relative_cwd)`, so worktrees of one repo collapse | `agents_overview_view.rs:128-152` |
| `SessionFilterMode::{Cwd, Repo, All}`; **Repo** is offered only when more than one canonical linked-worktree cwd exists | `resume_picker.rs:239-270`, `:2605-2613` |
| The row columns are **`Tasks | Status | Updated`** | `app/agent_center/rows.rs:82-88` |
| Directory renders through `format_directory_display(path, None)`, relativized to `~/…`, prefixed `⌁`, and the italic literal `"no cwd"` when absent | `status/helpers.rs:148-168`; `resume_picker.rs:108`, `:3646` |
| The cwd column width is `(width - 8) / 2` clamped to **[30, 72]**, and overflow **wraps onto extra lines** rather than truncating | `resume_picker.rs:3691-3696`, `pack_footer_parts` `:3659-3689` |
| **Left in an empty composer opens the agents overview** | `bottom_pane/chat_composer/agents_navigation.rs:29-38` |
| `/subagents` opens a picker over **descendants** (`ancestor_thread_id`), paged 100 up to 1000, state-db only, archived and live | `app/agent_picker.rs:21-105` |
| `Alt+←` / `Alt+→` cycle sub-agents, guarded by "no overlay, no modal, composer empty" | `multi_agents.rs:107-113`; `app/input.rs:541-572` |
| The parent link is `Thread.parent_thread_id`, with three server relations: `DirectChildrenOf`, `DescendantsOf`, `AttachedThreadsOf` | `app-server-protocol/.../thread_data.rs:224-225`; `thread_processor.rs:2825-2860` |
| A root's displayed status is the **min over its whole subtree**, so a root reads "Needs input" when any descendant is blocked | `agents_overview.rs:1333-1344` |
| The transcript overlay uses an **anchor model** (`Position::{Latest, Reading(Anchor{…})}`), not a raw offset, and opens at the bottom | `transcript_view.rs:77-81`, `:663-712` |

Three of these change this plan:

- **`Left` on an empty composer is shipped by codex too**, and codex's opens the
  *agent* overview — the navigator half of item 5, not the session half. So both
  references independently put a picker on that key. T6 is a port from two
  sources, not an invention.
- **The ` Tasks | Status | Updated` header is codex's, copied verbatim.** Operant's
  row was codex-shaped until iteration 2343 cut it to
  `#{id}  {employee}  {status}  {date}` and the header was not touched. Finding 4
  of the audit is a half-finished port, not a typo.
- **Both references solve the worktree problem and operant solves neither.**
  jcode matches on `git_common_dir()`; codex keys the Project group on
  `repository_identity().common_dir` and offers a `Repo` filter only when more than
  one worktree exists. Two independent implementations of the same rule is as
  close to a settled answer as this audit gets. T3 adopts it.

Codex has **no user-facing tool-call filter** either (grep for
`ToolFilter|hide_tool` returns nothing outside `SessionFilterMode`); the only knob
is `RawReasoningVisibility`, config-driven. Operant's `PreviewFilter` remains its
own feature, as its comment already claims.

### jcode — `session_picker` (the source the hub's geometry came from)

Location: `jcode/crates/jcode-tui/src/tui/session_picker.rs` (2,560 lines) with
`session_picker/{render,filter,loading,navigation,canvas,preview_chrome}.rs`, and
the shared types in `crates/jcode-tui-session-picker/src/lib.rs`. Rust, ratatui
0.30, no database — all state is JSON on disk.

Checked while writing this, and four of jcode's answers are better than ours:

- **The modern layout is not a side-by-side split.** `render_resume_canvas`
  (`session_picker/canvas.rs:4-25`, `:60-117`) paints the **transcript
  full-screen** and overlays one bounded "control card" — search row plus session
  list — on the **right**, width 36% of the terminal clamped to **30..52 columns**,
  80% height, vertically centred. The `60/40` split at `session_picker.rs:2329-2336`
  (preview **left**, list **right**) is the legacy overlay path.
- **Our provenance comment is wrong about the source.** The hub's doc says
  "horizontal 40/60 split (list | preview)" (`session_hub/mod.rs:6`). jcode's
  legacy split is 60/40 with the preview left and the list right; the modern path
  is not a split at all. Operant is 45/55 list-left (`render.rs:475-479`). The
  ratio, the side, and the design are all different. This matters now because the
  operator is asking about layout again: a reader checking "what does jcode do"
  against that comment gets the opposite answer.
- **`compact_dir()` is the path-shortening rule to copy verbatim**
  (`session_picker/render.rs:848-881`): `~` substitution for `$HOME`; at or under
  **28 characters** print as-is; otherwise keep **trailing components** joined by
  `/` with a `…/` prefix, never cutting a component in half; final fallback
  `…/<first MAX-3 chars>…`.
- **`CurrentDir` is a PROJECT predicate, not a path comparison**
  (`session_picker.rs:645-668`): the same normalized path, **or a subdirectory**
  of it, **or a sibling git worktree of the same repo** via `git_common_dir()`,
  memoized per directory — and it refuses to match everything when the current
  directory is `$HOME` (`:655-658`). Operant compares literal full-path equality.
  **Measured on the live store:** `/home/ishanp/wtc-arch` is a git worktree of
  operant (its `git-common-dir` resolves to `operant/.git`), and one session
  records it as its `cwd`. jcode's rule counts that session as "current
  directory"; operant's does not.
- **`SessionFilterMode`** (`jcode-tui-session-picker/src/lib.rs:37-59`) has `All |
  Cloud | CurrentDir | CatchUp | Saved | Active | ClaudeCode | Codex | Pi |
  OpenCode | Cursor`. Operant reduced it deliberately (documented at
  `session_hub/mod.rs:61-62`). Operant's `CurrentDir` kept jcode's *intent* but
  not its *rule*, and it compares against `std::env::current_dir()`
  (`mod.rs:1250-1252`) — the same source `turn_context.rs:115` stamps onto the
  row, so the tab and the data agree. That part is not a defect.
- **Grouping is deliberately suppressed once a filter or a search is active**
  (`filter.rs:477-499`, with a written rationale: sections promote weak matches
  over better hits). jcode's grouping key is the **server/daemon**, not the
  project — so jcode has no group-by-directory surface either. Operant's
  `Directory` grouping is its own.
- **Children are hidden by default with an honest counter.** Subagent sessions are
  marked `is_debug` (`server/headless.rs:177`), excluded from the list unless `d`
  is pressed, and the header carries `(+N hidden)` (`render.rs:667-672`), with a
  bounded scan budget so a burst of workers cannot crowd out real sessions
  (`loading.rs:1948-1984`).
- **The preview is the real chat renderer**, driven through a synthetic read-only
  `TuiState` and blitted from a `TestBackend` of the pane's exact size
  (`session_picker/session_view.rs:1-7`, `:390-432`), so header, pinned todos,
  composer and status line cannot drift from the opened session. It opens at the
  **bottom** (`session_picker.rs:1797-1807`) — the behaviour operant already
  ported — and has **no filter over message or tool-call kinds**, only a
  trailing 400-message cap (`loading.rs:34`, `:1701-1725`). Operant's
  `PreviewFilter` is its own feature, as its comment already claims.

## Adopted from the references, recorded but not tranched

These are real improvements the references make that no operator item asks for.
Recorded so they are not rediscovered, deliberately **not** given tranches — they
are not what the operator reported, and each would widen a commit without closing
a named gap.

| Idea | Source | Why it is not a tranche |
|---|---|---|
| **Anchor scroll model** — `Position::{Latest, Reading(Anchor{key,index,offset,row_bias})}` instead of a raw `u16` offset | codex `transcript_view.rs:77-81` | Operant's `preview_scroll` + `preview_follow_bottom` works; an anchor survives content changes, which matters only once the transcript can change under the operator |
| **Measure real wrapped heights** for scroll accounting rather than counting rows | codex `rendered_height_between`, `resume_picker.rs:2436-2463` | Prerequisite for variable-height rows, which T3 explicitly declines |
| **Preserve selection by stable identity across page ingest** | codex `seen_key()` + `ingest_page`, `:1285-1290`, `:2010-2079` | Operant already selects by row id (`rebuild_visible`'s keep-id branch), so this is mostly present |
| **Collapse decorative gaps before content** — gaps shrink 3→0 before the list drops below `MIN_LIST_ROWS` | codex `resume_picker/layout.rs:26-51` | Operant has no decorative gaps in the vertical stack |
| **Search that keeps paging until it finds something**, with an honest empty state | codex `continue_search_if_needed`, `:2153-2222`, `:4096-4102` | Operant's search filters the loaded page only, so this is a real gap — but it is a search-quality item, not a session-management one |
| **Memoize an expensive per-directory predicate** | codex `local_cwd_matches`, `:1131-1132`; jcode `same_project_cache` | T3 needs it if the predicate widens; folded into T3's implementation rather than its own unit |
| **Keyboard-vs-text arbitration** — `is_plain_text_key_event` so `j`/`k` never eat search input | codex `resume_picker.rs:1598` | Operant uses an explicit `search_active` mode, which is a different and adequate answer |

## Where operant diverges — measured on the live store

Store: `~/.operant/database.db`, opened read-only, `user_version = 4`.

| Measurement | Value |
|---|---|
| sessions | 3,324 |
| sessions with a `cwd` | **17** (0.5%), across 4 distinct directories |
| sessions with an `employee_id` | **3**, all `emp-cron_*` |
| rows in `employees` (the roster) | **9**, all named seats |
| overlap between roster and session tags | **0** |
| stops on the `e` axis (`list_session_employees` UNION) | **12** |
| …of which match at least one session | **3** |
| …of which render an **empty list** | **9** |
| rows with `parent_session_id` set | **1**, and it is a **rewind fork**, not a subagent |
| durable delegation ledger (`~/.operant/delegations.json`) | **5 records, all test fixtures**, every `parent_session_id: null` |
| hub page size | 500 (`app/mod.rs:1133`) |
| session-hub tests | 139 pass, 0 fail |
| `cargo check -p operant-cli --bin operant` | exit 0, 3 dead-code warnings |

Those counts are the whole complaint. The picker offers nine employee identities
that between them own **no sessions at all**; the directory tab and directory
grouping can match **0.5%** of the store; and the subagent lineage has **no
working carrier in either of the two places it could live**.

### The carrier correction (why revision 1 was wrong)

Revision 1 said T6 could walk `sessions.parent_session_id`. It cannot, and the
reason matters:

- `parent_session_id` is written in production by **rewind/fork only**
  (`agent/rewind.rs:134`, `:212` — `parent_session_id: Some(old_session)`, "New
  session id (same shape as /new), parent = old"). That is *this session was
  forked from that one*, a different relation from *this session is a subagent of
  that one*.
- A delegation **creates no session row at all**: `async_delegation.rs` contains
  no `save_session`, no `Database`, no `upsert`. `mark_session`
  (`async_delegation.rs:414-421`) records a session id on the in-memory record if
  the child produced one; nothing parents it to the dispatcher.
- The durable ledger does carry `parent_session_id` (`async_delegation.rs:83`),
  set from the dispatcher at dispatch — and on the live file **all 5 records are
  null**, and all 5 are test fixtures (`model = test-model`, goals `lively-*`).

So the navigator has no edge to walk until one is written. That is a unit in the
ledger below, not a detail of another one.

Measured while checking this: the current test suite does **not** write the live
ledger. Backed it up, ran `refresh_liveness_tests`, compared — byte-identical,
mtime unchanged. The `lively-*` records are residue from a revision before the
opt-in landed (`iter-2331`), not a live leak.

## Gap ledger — dependency-ordered tranches

Each tranche lands in its own commit and its `done when` is checkable at that
commit.

### T1 — The employee axis stops lying (closes item 2)

**Problem.** `cycle_employee` (`session_hub/mod.rs:844-866`) walks
`list_session_employees`, which UNIONs the session tags with the roster
(`database.rs:1930-1945`) and returns 12 ids ordered by label. Nine match zero
sessions. Press `e` once: the filter selects `chief-of-staff` (first by label),
the list empties, every tab count reads 0, and the header chip says `Chief of
Staff`. The operator's conclusion is that the picker is broken. It is not — the
axis is.

**Change.** Carry a count per stop. The query already has the roster and the tags;
add the per-id session count to the same statement and return `(id, label, count)`.
Render the count in the chip. A zero-count stop renders an explicit empty state
naming the seat ("no sessions for Chief of Staff yet") instead of a blank pane.

**Rejected alternative.** Drop the roster and cycle only the tags. Rejected
because the closed plan already ruled on it: "The roster is the authority on who
the employees ARE; the tags are the authority on which of them have run
something" — and "this seat has run nothing" is a fact the operator wants
(`session_hub/mod.rs:571-583`). The defect is not the roster; it is that the axis
never said which stops were empty.

**done when.** On the live store all 12 stops render, each carries its count, and
no stop produces a blank list without naming why. Guard: the real query shape
(roster + disjoint tags) with every stop's count asserted.

**Touches.** `database.rs` (one query), `session_hub/mod.rs` (roster type,
`cycle_employee`, chip), `render.rs` (chip + empty state), `session_hub/tests.rs`.

### T2 — The preview names its own filter, and the chrome tells the truth
### (closes item 1's "irrelevant")

Three small, measured defects that together make the pane read as unattended.

**2a. The filter label is written and never painted.** `PreviewFilter::label()`
(`session_hub/mod.rs:209-215`) returns `"full session"` / `"your prompts"` /
`"prompts + replies"`, and its doc says "the short label the preview header
paints, so the active filter is visible rather than remembered". Nothing paints
it: `grep preview_filter render.rs` finds four uses, all predicates. The compiler
already reports it — `warning: method label is never used` on the release tip.

Worse than dead: the footer describes a **three-state** enum in **two** states.
`render.rs:1659-1661` picks `"t shows tool calls"` or `"t hides tool calls"` from
`shows_tool_calls()`, which is false for **both** `UserOnly` and
`UserAndTurnEnd`. The default mode and the prompts-only mode are indistinguishable
on screen.

**2b. The column header describes a row layout two iterations old.**
`chrome_items` (`render.rs:83-104`) prints ` Tasks  Status  Updated`. Iteration
2343 reduced the row to `#{id}  {employee}  {status}  {date}`, so ` Tasks` sits
above nothing, and the `id` and `employee` columns the row now leads with are
unnamed.

**2c. `list_pane_width` hardcodes the split it was written to derive.**
`render.rs:46-48` computes `width * 45 / 100` as a literal. The renderer takes its
width from `SPLIT` (`render.rs:475`); the helper is called only from tests
(`tests.rs:873`, `tests.rs:1747`). It exists so "the renderer and the tests cannot
disagree about where the border is", and it can disagree the moment `SPLIT` moves
— the defect iter-2351 claimed to close.

**2d. The module doc misdescribes the geometry it vendored.**
`session_hub/mod.rs:6` says "horizontal 40/60 split (list | preview)". jcode is
60/40 with the preview **left** (`session_picker.rs:2329-2336`). Both the ratio
and the side are inverted. Fix the comment to state what operant does and what
jcode does, as two facts.

**done when.** The preview header paints the active filter's label; the footer
names all three states; the column header names the columns the row actually
carries; `list_pane_width` contains no literal `45`, and a test that moves `SPLIT`
sees the helper follow; the provenance comment no longer contradicts the source
it cites.

**Touches.** `render.rs`, `session_hub/tests.rs`. (`label()` already exists.)

### T3 — The directory, unclipped, and a current-directory tab that can match
### (closes item 4)

**Problem, in two halves.**

*Display.* The row shows the **basename only**, and only when the title line has
room: `dir_span` is `None` when `title_line_cells > inner_width`
(`render.rs:985-993`). The directory appears on short-titled rows and vanishes on
long-titled ones — "clipped", and varying row to row. The full path exists in one
place: the preview's pinned `project:` line (`render.rs:1481-1487`).

*Data.* The `c` tab and the `directory` grouping both compare `row.cwd`, and 17 of
3,324 rows have one. The write exists (`turn_context.rs:319`) and is best-effort
beside the seat stamp, so the tab is correct and the store is empty.

*Predicate.* The tab is also **literal full-path equality**
(`mod.rs`: `HubFilterMode::CurrentDir => r.cwd… == cwd`), where jcode matches a
**project** — same path, subdirectory, or sibling git worktree. Measured on the
live store: `/home/ishanp/wtc-arch` is a worktree of operant and holds one
session; jcode counts it as current-directory, operant does not. A session
started from `operant/crates/operant-cli` is excluded from a TUI launched at the
repo root.

**Change.**

- Give the directory its own field on the meta line instead of a tag hanging off
  the title, so it stops competing with the title's budget. The meta line's worst
  case measured 47 cells against 46 before the date swap and was absorbed by the
  `id_budget` ladder; the directory is the next field to enter that ladder.
- Shorten it with jcode's rule, ported verbatim rather than invented:
  `compact_dir()` (`session_picker/render.rs:848-881`) — `~` for `$HOME`, as-is at
  or under 28 characters, otherwise trailing components with a `…/` prefix, never
  a half-cut component, final fallback `…/<head>…`. A basename is not a directory:
  the live store holds `/home/ishanp/wtc-arch` and `/home/ishanp/.operant`, and a
  basename-only row cannot tell two projects with the same leaf apart.
- Give it a **clamped column and let it wrap**, the way codex does. Codex computes
  `cwd_column_width = (width - 8) / 2` clamped to **[30, 72]** and lets overflow
  wrap onto extra footer lines instead of vanishing
  (`resume_picker.rs:3691-3696`, `pack_footer_parts` `:3659-3689`). Operant's row
  is a fixed two lines, so wrapping needs a decision: either the directory is one
  clamped cell on the meta line, or a row becomes three lines. **Default: one
  clamped cell**, because a variable-height row breaks the selection arithmetic
  the list is built on (`state.selected` indexes body items 1:1). Codex can afford
  variable height because it measures real wrapped heights
  (`rendered_height_between`, `resume_picker.rs:2436-2463`); operant does not.
- Render absence honestly. Both references print an explicit marker rather than
  nothing — codex the italic literal `"no cwd"` (`resume_picker.rs:3646`), operant
  already `project: — (not recorded)` in the preview (`render.rs:1487`).
- Widen the `CurrentDir` predicate to the project rule **both** references use:
  equal, **or** a subdirectory of the current directory, **or** sharing a
  git-common-dir with it, memoized per directory, and never matching everything
  when the current directory is `$HOME`. jcode compares `git_common_dir()`
  directly (`session_picker.rs:152-169`); codex keys on
  `repository_identity().common_dir` plus `relative_cwd` and only offers its `Repo`
  filter when more than one worktree exists (`resume_picker.rs:2605-2613`). Same
  rule, two implementations.
- Backfill is impossible for pre-write sessions. Say so in the tab's empty state
  rather than showing a 0 that reads as broken.

**Rejected alternative.** Widen the pane. Rejected: the pane was rebalanced to
45/55 one iteration ago against a measured row budget, and a path long enough to
need the width (`/home/ishanp/Documents/github/my-projects/agentic-harness/operant`,
65 cells) fits in neither pane. jcode reached the same conclusion from the other
side and made the transcript full-screen with a 30–52-column list card.

**done when.** A row's directory is shortened by `compact_dir`'s rule and never
dropped by the title's budget; the `c` tab's count includes the `wtc-arch` session
when the TUI runs from `operant`; the empty state names the missing write. Guard: a
row at the narrowest supported width whose title is long enough to have evicted the
basename under the old rule, plus a worktree row asserted present in `CurrentDir`.

**Touches.** `render.rs`, `session_hub/tests.rs`.

### T4 — The identity switch: `Shift+Tab` and `/agent` (closes item 3)

**The largest unit and the only genuinely new capability.** Nothing in the tree
switches the acting employee. Measured:

- `set_seat_id` has three production callers: the Holon-run mount
  (`main.rs:2503`), the gateway turn (`gateway_runner.rs:4255`), and the cron
  scheduler (`cronjobs/scheduler.rs:578`). **None is a TUI path.**
- `grep -rn "fn switch_identity|active_identity|current_identity|set_identity"
  crates/operant-cli/src` → **no matches.**
- The employee roster reaches the TUI only as a **filter** over sessions, never as
  an actor.
- `Shift+Tab` is bound to `cycle_effort` (`key_handling.rs:2153`), process-global.

**No reference implementation has this either, and the three differ in a way that
matters.** All three were checked:

| | What switches at runtime | Scope | Mechanism |
|---|---|---|---|
| oh-my-pi | model + `@role`, and effort as a **separate** axis | session-only for `/switch` and `Alt+P`; persistent for `/model <sel>` | `/switch` → compact picker (`builtin-modes.ts:491-497`); `Alt+P` → compact (`input-controller.ts:719-723`); `Alt+M` → hub (`:727-728`); `Ctrl+P` cycles **role** models (`:715-716`); `Shift+Tab` cycles **effort** (`:702-706`) |
| jcode | **model only**, per agent role (`Swarm/Review/Judge/Memory/Ambient`) | session or global | `/agents`, `Ctrl+G` to flip scope (`commands.rs:3342-3368`, `inline_interactive.rs:3548-3568`) |
| jcode | model favourites | session | `Shift+Tab` (`input.rs:3159-3162`) |
| codex | — | — | no identity concept |

jcode's `Shift+Tab` cycles **model favourites** and changes "the model/route and
nothing else — not the system prompt, not the tool set, not the working directory".
Neither jcode nor codex has a persona, a switchable system prompt, or a seat. So
operant's ask is closest to oh-my-pi's `@role` and has no exact precedent — which
is why the atomic pair matters: in operant a seat carries a **charter** (the org
system prompt, `employees.system_prompt`), so switching the seat without switching
the charter would leave the operator talking to one employee with another's
instructions. `set_seat_id` + `set_charter` are documented as that pair
(`events.rs:956-959`) and this is the first TUI path that must honour it.

**The trade the operator is making, stated against omp.** In omp, `Shift+Tab` in the
composer **is** the effort cycle (`custom-editor.ts:142` → `input-controller.ts:702-706`
→ `model-controls.ts:642-654`: `off → auto → minimal…max → off`). Taking that key for
identity in operant means operant ends up with omp's key position and the opposite
function. That is a legitimate choice — operant's effort stays reachable on the
models page — but it is a divergence from the reference, not parity with it, and the
model-picker plan's M1/M2 are what make the move safe.

**Change.** A session-scoped seat, switched from the composer:

- `App` gains the acting seat for the current session (`Option<String>`, seeded
  from `sessions.employee_id`).
- `Shift+Tab` opens a roster picker over the 9 seats — label, department, role,
  and whether this session has run as them. Selecting one sets the seat **for this
  session only**. **This is now decided, not proposed** — see the resolution below.
- `/agent` with no argument opens the same picker; `/agent <seat>` switches
  directly. Both session-scoped, matching omp's `/switch`
  (`builtin-modes.ts:457-463`).
- The switch calls the pair the code documents as atomic — `set_seat_id` **and**
  `set_charter` (`events.rs:956-959`) — and stamps `sessions.employee_id` through
  the existing `set_session_employee` write, so the hub's employee axis
  immediately agrees with who ran the session. **This is also what makes T1's axis
  stop being empty**: T4 is the first path that can write a roster seat onto a
  session.
- The composer's status line names the acting seat.

**Consequence, now decided.** The operator's ruling: "`SHIFT+TAB` must change the
identity of the active-agent, from the employee-list/personas, by replacing the
effort-ladder. The effort-modification must be ported to the models page."

So `Shift+Tab` moves from `cycle_effort` (`key_handling.rs:2153`) to the identity
picker, and effort becomes an axis of the models page rather than a global ladder.
Concretely, T4 must:

- rebind `KeyCode::BackTab if !self.is_streaming` from `cycle_effort` to the
  identity picker, and **delete the `cycle_effort` call site** — leaving the
  function in place as dead code is how the two meanings end up on one key again;
- leave `cycle_effort` itself in the tree only until the sibling plan's M1 lands,
  then remove it with its `Effort: <level>` notice;
- keep `/effort` working through the transition, because it is the only effort
  control until M1 ships. It is a command, not a key, so it does not conflict.

The operator loses one-keypress effort and gains one-keypress identity. That is a
deliberate trade, and it is why the sibling plan's M1 is a hard dependency rather
than a follow-up.

**done when.** On a session, `/agent premiere` then a turn writes
`sessions.employee_id = 'premiere'`, the composer names the seat, and a second
session in the same process keeps its own seat. Guard: two sessions, two switches,
assert each row's `employee_id` independently — a global seat would pass a
one-session test.

**Touches.** `app/mod.rs`, `app/key_handling.rs`, `app/commands.rs`, `app/init.rs`,
a new `tui/agent_identity_picker.rs`, `session_hub/tests.rs`.

### T5 — A carrier for subagent lineage (prerequisite; revision 1 missed this)

**Problem.** Measured above: `parent_session_id` means *forked from*; delegations
write no session row; the durable ledger's `parent_session_id` is null on every
record. The navigator in T6 has no edge to walk.

**Change.** Persist the edge that already exists in memory.
`AsyncDelegationRecord.parent_session_id` is set from the dispatcher at dispatch
(`async_delegation.rs:317`, `:365`) and then thrown away by a store that never
reads it back into a query. Write it where both a live process and a resumed one
can read it:

- Backfill `parent_session_id` in `delegations.json` for records created after
  this lands, and **stop the ledger from being overwritten wholesale**:
  `persist()` rewrites the whole registry from memory
  (`async_delegation.rs:163-172`), so a process whose registry holds five test
  records replaces the operator's real history with them. Merge on read, or write
  per-record.
- On resume, a session's children are the ledger records whose
  `parent_session_id` equals its id — which is exactly omp's "opening the Hub for a
  persisted session scans that session's artifact tree" (`agent-hub.md:100`), with
  a JSON file in place of a directory convention.

**Rejected alternative.** Add a `parent_session_id` write for delegations into
`sessions`. Rejected: it would conflate fork lineage with subagent lineage in one
column, and the column already has a meaning and a writer. One fact, one owner.

**done when.** After a delegation runs, the ledger record carries the dispatcher's
session id; a second process can list that session's children from the file alone;
and a test with two dispatchers proves the records do not cross.

**Touches.** `async_delegation.rs` (persist merge), its tests, `main.rs` if the
recovery report changes.

### T6 — `<-` opens the right surface for the context (closes item 5, first half)

**Problem.** Today a **double**-tap on an empty composer opens the hub
(`key_handling.rs:2029-2060`), window `[40ms, 500ms)`, with a handoff so one
further `←` closes it. A single `←` moves the cursor. The operator is asking for
the single press on a new session.

**Change — and why it costs nothing.** Bind a **single** `←` only in the state
where it has no cursor meaning to lose: composer empty, cursor at 0. In that state
`←` is already a no-op; there is no text to move within. Every other state keeps
today's behaviour, and the double-tap stays as the universal opener.

**This is a port from two sources, not an invention.** jcode ships exactly this
gesture: `maybe_open_active_sessions_on_left` (`inline_interactive.rs:2641-2654`)
requires `input.is_empty() && cursor_pos == 0`, is gated behind a config flag
(`display.active_sessions_manager`) "so the default input behavior is unchanged",
and opens a picker with a filter **pre-activated**. Its key site is
`key_events.rs:788-796`: `Left` moves the cursor while `cursor_pos > 0`, else calls
the gesture. **codex ships it too**, and codex's opens the *agent* overview:
`bottom_pane/chat_composer/agents_navigation.rs:29-38`. Operant should copy the
shape and differ on three points, all stated:

- jcode gates the gesture behind a config flag. Operant binds it only in the state
  where it is already inert, so no flag is needed — the gate *is* the state.
- jcode's opens the **Active** (live sessions) view; codex's opens the **agents**
  overview. Operant's opens the session hub, and the navigator (T7) is reached
  from an active session — which is codex's split.
- codex guards its `Alt+←`/`Alt+→` sub-agent cycling with "no overlay, no modal,
  composer empty" (`app/input.rs:541-572`) so the chord stays word-motion while a
  draft exists. Operant's `←` needs the same guard shape.

**Correction to revision 1.** Revision 1 called this an operant extension because
oh-my-pi has no such gesture. oh-my-pi indeed does not (`app.session.resume` is
unbound, `app-keybindings.ts:224-227`), but jcode and codex both do, and jcode is
the source this hub already vendored its geometry from. The design now rests on two
shipped precedents rather than on argument.

**done when.** On a session with an empty composer one `←` opens the hub; with text
in the composer `←` moves the cursor and nothing opens; the double-tap path still
opens the hub on a session that already has history. Guard: three tests, one per
state — the defect this fixes is precisely a state that was never enumerated.

**Touches.** `app/key_handling.rs`, `app/tests.rs`.

### T7 — The subagent navigator as its own surface (closes item 5, second half)

**Problem.** Subagent rows are appended into the global list from the in-process
registry — `OnceLock<Mutex<HashMap>>` (`async_delegation.rs:125-128`) — so the hub
can only ever show *this process's* delegations, mixed in with 3,324 unrelated
sessions.

**Change.** A second surface, session-scoped, built the way omp builds it:

- Opens from `←` on an active session that has live subagents (T6's context
  table), and from `/subagents` (omp's `Ctrl+S` / `app.session.observe`).
- Lists the current session's children from T5's carrier: the in-process registry
  **plus** the ledger's records for this session id.
- **Scope the walk to this session's own descendants.** jcode's
  `filter_inline_swarm_subtree` (`tui_state.rs:2754-2790`) builds a
  `parent → children` index from the live edge and walks outward from `self_id`,
  cycle-safe, excluding self — with the written rationale that other sessions can
  share a swarm in the same repo without this session having spawned them, and
  showing those "would be noise" (`:2804-2809`). Operant's equivalent hazard is
  real: the delegation ledger is one file for the whole home, so an unscoped list
  would show every session's children. Until the session identity is known the
  surface stays hidden, as jcode's does (`:2818-2822`).
- **Decide flat versus tree deliberately, because the references disagree.**
  codex's `/subagents` picker is a **flat descendant list** — `ancestor_thread_id`
  returns every descendant, paged 100 up to 1000, state-db only, archived and live
  (`app/agent_picker.rs:21-105`) — and it does not indent, even though the indent
  mechanism exists in its `SelectionItem` (`list_selection_view.rs:174-176`).
  jcode's swarm panel is flat too, with `Alt+O` to pop a child out. Only oh-my-pi
  builds a real parent/child **tree**, toggled with `t`
  (`agent-hub-projection.ts:343-438`). Operant's `t` toggle follows oh-my-pi; the
  flat default follows codex and jcode, which is the majority.
- **Carry depth, not just the direct-child edge.** codex's server distinguishes
  `DirectChildrenOf`, `DescendantsOf` and `AttachedThreadsOf`
  (`thread_processor.rs:2825-2860`), and its spawn record models
  `SubAgentSource::ThreadSpawn { parent_thread_id, depth, … }`. Operant's hub has
  the shallow version already: the `CurrentBranch` filter is
  `s.id = ?5 OR s.parent_session_id = ?5` (`database.rs`,
  `list_sessions_rich_filtered_page`) — one level, not recursive. T5 records depth
  so a grandchild stays reachable.
- **`Alt+←` / `Alt+→` to cycle children** is codex's binding
  (`multi_agents.rs:107-113`, `app/input.rs:541-572`), guarded so it stays
  word-motion while a draft exists. Worth having once a navigator exists.
- **Roll status up the subtree.** codex shows a root's status as the min over its
  whole subtree, so a root reads "Needs input" when any descendant is blocked
  (`agents_overview.rs:1333-1344`). If children ever become rows in the global
  list, this is what keeps the `needs you` tab honest.
- **Refuses to mount on an empty roster** (omp's `requireContent`,
  `selector-controller.ts:2683-2695`) — an empty overlay flashing open reads as a
  broken key.
- `t` toggles flat and parent/child tree — omp's `t` (`agent-hub.md:36`). A
  different meaning from the hub's `t`, which is fine: different surfaces,
  different focus.
- Enter focuses a child: the existing `HubSelection::FocusSubagent` →
  `focused_subagent` path (`mod.rs:1683-1688`), lifted out of the global list.
- **Freeze the row order** while the surface is open (omp's `#frozenRank`,
  `agent-hub.ts:231-244`). The hub already has this problem in a milder form: it
  refreshes subagent rows in place every frame
  (`refresh_subagent_rows`), and a status change re-sorts under the cursor.
- Missing usage renders as `—`, not `0` — the rule the delegation rows already
  follow (`render.rs:1128-1136`) and omp states as `usage —` (`agent-hub.md:70`).

**Cheaper alternative worth considering first.** jcode does not build a navigator
for this at all: subagent sessions are marked `is_debug`, excluded from the picker
by default, revealed by a `d` toggle, and counted honestly in the header as
`(+N hidden)` (`render.rs:667-672`). That is far less code than a second surface
and it fixes the same complaint — children currently appear as ordinary flat rows
in a 3,324-row list. If the operator wants the smallest change, do this instead of
T7 and keep T7 for when a per-child inspector is actually needed.

**Depends on.** T5 for its data, T6 for its opener. Can land before T6 with
`/subagents` as the only opener.

**done when.** On a session with one live delegation and one ledger record, the
navigator lists exactly those two, `t` produces a parent/child tree, and a session
with no children never mounts. Guard: a session-scoped assertion — a child of a
*different* session must not appear.

**Touches.** a new `tui/subagent_navigator.rs`, `app/key_handling.rs`,
`app/commands.rs`, `app/mod.rs`.

## Design decisions, with the rejected alternative

| Decision | Rejected | Reopens when |
|---|---|---|
| Subagent navigator is a second surface, not a hub filter | A `Subagents` tab beside the seven | The operator asks for one list containing everything |
| Subagent lineage lives in the ledger, not a new `sessions` column | Reusing `parent_session_id` | Fork lineage and subagent lineage are ever needed in one query |
| `Shift+Tab` becomes the session seat switch; effort moves to the models page | Leaving effort on `Shift+Tab` and adding a second chord for the seat | The operator says effort is reached more often than the seat |
| The directory enters the meta line's `id_budget` ladder, tail-shortened | Widening the pane; showing a basename | A terminal width where a real path fits whole |
| The `e` axis keeps all 12 stops with counts | Cycling only non-empty seats | The operator wants empty seats hidden |
| A single `←` binds only where it is already a no-op | Taking `←` globally with a timeout | The operator accepts losing cursor-left |
| The navigator does not mount on an empty roster | Mounting and showing "no subagents" | The operator wants the empty state discoverable from the key |
| The seat switch swaps the charter with the seat | Setting the seat alone and leaving the prompt | The charter is ever resolved per turn rather than at switch |
| `compact_dir`'s 28-character trailing-component rule | A basename; a proportional font width | The operator names a different budget |

## Non-goals

- No schema change. Every column this plan needs — `cwd`, `employee_id`,
  `parent_session_id`, `started_at`, `ended_at`, `first_prompt` — exists at
  `user_version = 4`, and T5 deliberately does not add one.
- No agent-definition work. `AgentsMenuState` / `/agents` stays the definition
  hub; omp keeps them separate for the same reason (`agent-hub.md:117`).
- No collapse/expand for directory sections.
- No cross-process subagent discovery beyond the ledger.
- No activity log, IRC/unread counts, revive, or kill — omp's roster controls that
  have no operant counterpart yet.

## Open questions

**Resolved by the operator (revision 3):** `Shift+Tab` becomes the agent identity
switch; the effort ladder is replaced and ported to the models page. No modifier
keeps effort. See T4 and the sibling model-picker plan.

**Still open — one question.** When the identity picker is open, what should
`Shift+Tab` do?

The three candidates, and what each rests on:

| Option | Precedent |
|---|---|
| Cycle the seat | **Operant's own**: in-picker `Shift+Tab` cycles model favourites (`providers.rs:609-625`). No reference implementation does this. |
| Toggle pane focus (roster ↔ detail) | omp's **fullscreen hub** binds tab / shift+tab to pane focus (`model-hub.ts:1994-1997`) |
| Do nothing | omp's **compact picker** has no tab branch at all — `Shift+Tab` is silently swallowed inside it (`model-browser.ts` has no tab case; `keys.ts:383-388`) |

**Default if unanswered: cycle the seat.** It is what operant already does with the
key inside a picker, and a picker whose only exit is `Esc` and whose rows must be
reached with arrows is slower than the ladder it replaces. Note the symmetry:
operant's in-picker `Shift+Tab` means "next favourite", while omp's *composer*
`Shift+Tab` means "next effort level" — which is exactly the key this plan is
taking away. Do not copy omp's swallow behaviour into a surface that has a detail
pane.

**One correction to revision 2.** Revision 2 justified this by citing omp's model
picker cycling favourites. That was wrong twice: omp has **no favourites feature at
all** (repo-wide `ctrl+n` returns zero matches, and `favourite|favorite` matches
only unrelated words), and the in-picker favourite cycle is operant's own code. The
citation now names the right source.

## What changes when this is executed

Nothing yet. This file implements nothing. Executing it lands seven commits, in
order T1 → T7. T1 and T2 together are the smallest useful first tranche — T1 makes
the picker answerable and T2 makes it legible — so an operator who wants to stop
early should stop after T2.

T4 is now coupled to the sibling plan: land that plan's M1 (effort on the models
page) in the same tranche or the one before it.