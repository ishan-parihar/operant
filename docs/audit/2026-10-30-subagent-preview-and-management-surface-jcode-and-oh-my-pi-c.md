# Subagent preview and management surface: jcode and oh-my-pi contrast

**Date:** 2026-10-30
**Kind:** audit (frozen — read, do not edit; corrections go in a new dated audit)
**Why it exists:** the operator asked for "an audit of the surface area of
subagent-previews from jcode, an investigation of the gaps against omp, and a
plan to upgrade the TUI and the gateway". The prior session delivered code
without any of the three. This is the audit; the plan is
`docs/plans/plan-2026-10-30-subagent-preview-and-management-upgrade.md`.

**Method:** every claim below is a grep/wc against the checked-out siblings at
`../external-repos/{jcode,oh-my-pi}` and this tree, with `file:line` where a
number is quoted. Where a claim could not be settled by grep it is marked
**UNVERIFIED** rather than asserted.

---

## 0. The headline, before the detail

**The instruction's premise is inverted. Oh-my-pi owns the rich subagent
surface; jcode has none.**

- `oh-my-pi` ships a documented **Agent Hub** (`docs/agent-hub.md`, 123 lines):
  a live roster, a per-agent inspector, a combined activity log, steer/revive/
  kill, a pinned live-preview block above the editor, persisted-agent discovery
  and read-only advisor rows.
- `jcode`'s session picker has **no subagent concept at all.** Its row model is
  `PickerItem { ServerHeader, Session, OrphanHeader, SavedHeader }`
  (`jcode-tui-session-picker/src/lib.rs:177-191`) — sessions grouped by server.
  `grep -in "subagent\|delegation"` over the whole 108,757-byte
  `jcode-tui/src/tui/session_picker.rs` returns **zero hits**.
- **operant is already structurally closer to omp than to jcode.** Its picker
  row model is `RowKind { Session, Branch, Subagent { goal, model,
  dispatch_time, status } }` (`crates/operant-core/src/session/listing.rs:80-95`)
  — a subagent variant jcode does not have, joined at load time from the
  async-delegation registry (`async_delegation.rs:188 records_snapshot()`).

So the audit direction is corrected: **port from omp, cite jcode only as the
geometric baseline (40/60 split, two-line ListItem, search tokens, pane focus)
that this repo already vendored.**

---

## 1. What oh-my-pi actually has (the target surface)

All citations are `oh-my-pi/docs/agent-hub.md` unless a file path is given.

### 1.1 The roster (`:19-27`)

Rows show, per agent:

| field group | contents |
|---|---|
| identity | status (`running` `idle` `parked` `aborted`), agent identity, **parent**, unread IRC count |
| model | **model role, resolved model**, age since last activity |
| work | assigned task **or current activity** |
| cost | cost, active time / elapsed span, request count, tool-call count, tokens |

Header aggregates status + usage across measured agents. `t` switches a flat
roster and a **parent/child tree**. `/` filters by id or display name.

### 1.2 The inspector (`:29-37`)

Beside the roster on a wide terminal, in place of it on a narrow one (`Tab`):

- current tool **and arguments**, last intent, **retry state**
- **context-window use**
- parent and child lineage
- output and patch paths, plus isolated-worktree branch metadata

**Metrics discipline worth copying verbatim:** "Missing data appears as
`usage —` rather than an estimate" (`:36`). omp refuses to guess; operant's
hub currently prints `$0.0000`-style zeros for every subagent row.

### 1.3 Controls (`:39-56`)

`j/k`/arrows/wheel select; `Enter`/click open; `t` flat↔tree; `/` filter;
`1`/`2` roster↔activity; `Tab` inspector on narrow; `PageUp/Down` scroll;
`r` **revive**; `x` **abort then kill**; `Esc` clears filter → closes
inspector → closes Hub.

Revive semantics are the interesting part: "the transcript is on disk, so a
killed agent comes back as a **fresh instance with its full history**" (`:55`).

### 1.4 The activity log (`:58-64`)

`Enter` opens the selected entry's transcript **at that entry**. `f` cycles
All/Errors/Responses/Tools, `s` cycles scope (all / selected / subtree), `/`
searches, `Space` follows the newest entry, `Esc` clears search then closes.

### 1.5 The pinned live-preview block — the item operant has no analogue for

> While subagents run, a pinned `Subagents` block above the editor lists every
> live agent — sync task calls and detached background spawns alike. (`:86`)
>
> Set `display.subagentLivePreview: true` to add a second line under each row
> showing what that agent is doing: its **current tool call** (or, between
> calls, the most recent one) with a one-line detail, plus the **elapsed time
> once a call runs longer than five seconds**. Off by default. (`:90`)

Implementation: `packages/coding-agent/src/modes/interactive-mode.ts`
(pinned-block rendering; settings key `display.pinnedAgents` =
`collapsed|full|off`, `:86`).

This is a *live tail*, distinct from the hub: it answers "what is running right
now" without leaving the conversation.

### 1.6 Focus and steer (`:71-84`)

`Enter` focuses the subagent's session and closes the Hub; steering writes to
the subagent's persisted history; `Esc`/double-`←` returns to the main session
**without interrupting** the subagent (`:80`).

### 1.7 Persisted discovery + advisor rows (`:96-118`)

Opening the Hub for a persisted session scans that session's artifact tree:
historical subagent JSONL becomes `parked` rows, a killed agent's tombstone
keeps it `aborted` until revived, **nested subagents retain parent/child
lineage**. Advisor transcripts (`__advisor*.jsonl`) appear as read-only rows
that cannot be messaged, revived or killed (`:110-118`).

---

## 2. What jcode actually has (the geometric baseline, and nothing more)

- Picker row model: `PickerItem { ServerHeader, Session, OrphanHeader,
  SavedHeader }` — `jcode-tui-session-picker/src/lib.rs:177-191`. **No
  subagent variant.**
- `jcode-tui/src/tui/session_picker.rs` (108,757 bytes): zero hits for
  `subagent`/`delegation` (case-insensitive).
- No agent-hub module: `grep -rl "AgentHub|agent.*hub"` over
  `jcode-tui/src/tui/` returns nothing.
- The subagent code that DOES exist in jcode is worker-side, not UI:
  `jcode-app-core/src/agent/inline_tail.rs` (286 lines) is the live-tail
  primitives module, and `agent/turn_loops.rs`, `ambient/runner.rs`,
  `overnight.rs`, `catchup.rs` reference subagents as schedulers.

**Conclusion:** jcode contributes layout to operant, not subagent semantics.
Any claim that operant's subagent hub was "ported from jcode" is wrong; the hub
is operant's own (iter-B/C, plan 2026-10-28), inspired by omp's geometry and
jcode's controls.

---

## 3. What operant has today, against the omp checklist

Verified in the working tree. "Gap" = omp has it, operant does not.

| omp capability (`agent-hub.md` §) | operant today | verdict |
|---|---|---|
| subagent rows in a roster (`:19`) | `RowKind::Subagent` joined from `async_delegation::records_snapshot()` (`listing.rs:80-95`, `session_hub/mod.rs` load) | **have** |
| status glyphs running/idle/parked/aborted (`:19`) | `status_glyph` maps only `RowStatus{Live,Idle,Archived}` → `⚡ · 📦` (`render.rs:341` region); the subagent's own `status: &'static str` ("Pending\|Completed\|Failed") is rendered only in the focused preview format string | **partial** — the subagent's *delegation* status is not on its row |
| parent / lineage (`:19`, `:31`) | `RowKind::Branch` shows `↳`; subagent rows carry **no parent field** | **gap** |
| model role + resolved model (`:19`) | `RowKind::Subagent.model` exists and is shown in the focused preview, not on the row | **partial** |
| assigned task / current activity (`:19`, `:82`) | row shows the `goal` (48 chars) | **have (static)** |
| current tool call + one-line detail + >5s elapsed (`:90`) | nothing | **gap — the biggest one** |
| pinned live block above the editor (`:86`) | `background_tasks.rs` renders docked rows, but from the background-task registry, not from the subagent registry | **partial** |
| cost / tokens / request count / tool-call count (`:19`) | row meta shows `id · employee · msg count · tokens · age · cost` | **have** |
| inspector: tool + args, retry state, context use (`:29-31`) | the focused-subagent preview shows goal/model/dispatched/status only | **gap** |
| `usage —` instead of an estimate (`:36`) | subagent rows print `$0.0000`, `0 msg`, `0 tok` — all fabricated zeros | **gap** |
| flat ↔ parent/child tree (`:25`) | `s` cycles `HubFilterMode{All,CurrentDir,ByEmployee,Live,Branch}` — filters, not a tree | **gap** |
| filter by id/display name (`:26`) | tokenized `search_query` with per-char highlight | **have** |
| activity log (`:58-64`) | nothing | **gap** |
| revive (`:53`) | nothing; `delegate_task` records are terminal-only here | **gap** |
| abort/kill (`:54`) | nothing | **gap** |
| steer into a subagent (`:75`) | nothing; `_args` in the gateway `/steer` goes to the main queue | **gap** |
| read-only advisor rows (`:110-118`) | operant has `BackgroundReview` summaries, no advisor rows | **n/a** |
| double-`←` returns from a focused subagent (`:22`, `:84`) | `left_tap_close()` + `focused_subagent` (`mod.rs:575`, `key_handling.rs:677-683`) | **have** |

**Count: 4 have, 4 partial, 8 gaps, 1 n/a.** The gaps cluster in two places:
(a) **live** activity (the thing that makes a preview a preview) and (b)
**control** (revive/kill/steer).

---

## 4. The gaps that actually matter, ranked

### Gap A — there is no live tail of a subagent (HIGHEST)

omp's `subagentLivePreview` second line is the difference between a roster and a
window. operant's `RowKind::Subagent` carries `goal/model/dispatch_time/status`
and **no event stream**. The focused preview is a static card
(`render.rs:411-431`): goal, model, dispatched, "←← / Esc returns to the list".

To close it operant needs a per-delegation event feed. The raw material exists:
`async_delegation` holds records, and the agent emits `AgentEvent`s during
`delegate_task` execution — but nothing keys those events by delegation id.
**This is a core-side gap, not a render-side one**, and it is the only gap that
cannot be closed inside the hub.

### Gap B — subagent status is invisible on the row (HIGH)

`RowKind::Subagent.status` is a `&'static str` handed to a `format!` only in the
focused card. A `Completed` and a `Failed` subagent look identical in the list
unless focused. `RowStatus` (Live/Idle/Archived) and the delegation status are
**two different axes and omp treats them as one** (`:19`).

### Gap C — the row fabricates zeros (MEDIUM, cheap fix)

`$0.0000 · 0 msg · 0.000k tok` on a subagent row asserts facts operant does not
have. omp's `usage —` (`:36`) is the precedent: print the dash.

### Gap D — no lineage (MEDIUM)

No parent field on `RowKind::Subagent`, so `↳` (branch prefix) and tree view
are impossible for subagents. omp keeps nested lineage (`:116`).

### Gap E — no control surface (MEDIUM, scope decision)

Rebuilding revive/kill/steer is a features ask, not a bug ask, and the
instruction's plan-shaped deliverable makes it a plan row rather than an
implementation. It needs a core API that does not exist yet.

---

## 5. The gateway surface: what exists, what is missing

The instruction named "slash commands based exploration for telegram, slack,
etc" explicitly. Verified against the registry:

`crates/operant-cli/src/gateway_commands.rs:159-330` registers (name — aliases):

- **Session:** `new`(reset,clear) · `stop` · `status` · `retry` · `undo` ·
  `resume` · `session` · `title` · `branch` · `rollback` · `compress` ·
  `background`
- **Session (agents):** `agents`(tasks) · `steer` · `goal` · `subgoal` · `topic` ·
  `whoami`
- plus admin/pause/unpause/queue and platform plumbing

**`/agents` today lists PLATFORM SESSIONS, not subagents.** Its handler
(`:1476-1494`) calls `gateway.get_session_store()` and formats
`Active sessions: <count>` plus each session's platform and platform user id.
It never touches the delegation registry.

That is the central gateway gap: the two "agent" surfaces mean different
things — the TUI hub means "background `delegate_task` workers", the gateway
`/agents` means "chats I am serving on Telegram/Slack". A gateway operator
asking `/agents` gets the wrong list, and has **no command at all** for the
right one.

The registry data is reachable: `async_delegation::list_records()` and
`records_snapshot()` are module-level (`async_delegation.rs:171,188`), so a
gateway handler can read them without a gateway-state change. What is missing
is (a) a command that does it and (b) a formatting layer — the gateway speaks
Markdown to seven platforms (`asterisk_dialect`), so a hub-style two-pane
render is not the right target there.

---

## 6. Scope recommendation for the plan

Ranked by value per line changed:

1. **Row status + honest `usage —`** (Gaps B, C). ~40 lines in `render.rs`,
   zero core changes, immediately visible. Ships with regression tests.
2. **`/agents` split into `/agents` (platform sessions) and `/subagents`
   (delegation registry)**. ~80 lines in `gateway_commands.rs`, reads an
   existing API, closes the mislabeled surface. Telegram/Slack-equivalent by
   construction because all seven adapters share the command layer.
3. **A per-row live tail read from the delegation registry's existing record**
   (the cheap half of Gap A: `status` transitions + `dispatch_time` age, not
   tool calls). Needs one new field on the registry read path.
4. **Lineage + tree view** (Gap D). Needs a parent field; do it only if (3)
   lands cleanly.
5. **Live tool-call streaming, revive/kill/steer** (Gaps A-control, E). Each is
   a core-side feature with its own plan. Not in this iteration.

**Explicitly NOT recommended:** porting jcode's picker wholesale. Its
`PickerItem` has no subagent variant, and operant's hub already implements the
jcode geometry (SPLIT, `SessionHubState`, `PaneFocus`, search tokens,
`preview_max_scroll`) plus the omp close-tap handoff. Porting jcode's model
would delete the subagent rows.

---

## 7. What was NOT verified

- omp's runtime behavior of the pinned block and the activity log — read from
  `docs/agent-hub.md` and two file references, not executed. A behavioural
  comparison would need omp running.
- jcode's `inline_tail.rs` primitives were counted (286 lines) but not read
  end to end; the claim here is only that they are worker-side.
- operant's subagent rendering in a LIVE run was not re-captured for this
  audit; the frame-level evidence is the `TestBackend` rows quoted in the
  red-team report (`⚡ 🤖 🤖 do research`), which the accompanying fix removes.
