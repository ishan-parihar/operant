# Audit: the todo card and the subagent preview are both renderer-only — their producers are unwired

**Date:** 2026-10-31
**Kind:** audit (dated-frozen)
**Amends:** `docs/plans/2026-10-30-subagent-preview-and-management-upgrade.md` § 3.1 and § 3.2
**Prior art:** `docs/audit/2026-10-30-subagent-wave-3-deferral-rests-on-a-false-premise.md` (which already corrected one false premise in that plan), the `[port-excision]` / `[port-decision]` annotations in `crates/operant-cli/src/tui/operant_app/todo.rs`, and the header comment at `crates/operant-cli/src/tui/operant_ui/ui_todo_changes.rs:5-19`.

> **AMENDED 2026-10-31, after the re-read.** § Gap 4 below is WRONG and is retracted
> in full: the gateway already ships a delegation roster. `/subagents` was
> registered and implemented in iter-1500 (`git log -L 290,293` on
> `gateway_commands.rs` → `010109f6 feat(iter-1500): /subagents — the gateway
> finally reads the delegation registry`), and `gateway_commands::subagents_command_tests`
> pins it — 5 tests, all green today. The audit asserted "no todo card, no
> delegation roster, no `/todos`, no `/subagents`" after grepping `gateway/`
> for the word `todo` and `gateway_runner.rs` for `subagent`; `/subagents`
> lives in `gateway_commands.rs`, which neither grep covered, and the plan § 2.2
> row it cited as "still unshipped" had already landed 89 iterations earlier.
> What survives of Gap 4 is narrower and is restated at the bottom of that
> section: the gateway carries a `todo` progress line with an emoji and no
> counts, and no command reads the todo list. Gaps 1 and 2 stand unchanged
> and are now fixed (iter-1590 / iter-1593).
>
> **AMENDED AGAIN 2026-11-01 (iter-1597) — the "no command reads the todo
> list" half of Gap 4 is now CLOSED, which makes the sentence above stale on
> purpose: it is left in place because the amendment record is the history of
> what this audit believed, not a live claim.** `/todos` (alias `/todo`) is
> registered at `gateway_commands.rs:298` and its handler (`:1615`) reads
> `operant_core::tools::todo_list_for_session` — the SAME accessor the TUI tick
> uses, so the channel reply and the TUI card read one truth rather than two
> definitions of it. The session it reads is the one the agent writes: the
> dispatch loop's routing `session_key` feeds the same `pin → derived` chain
> `run_session_scoped_command` uses (`gateway_runner.rs:2245-2251`). Three
> tests pin it (counts by status, the empty arm cross-linking `/subagents`,
> markdown neutralisation) and 4 in operant-core pin the per-session keying.
>
> **What is still open from Gap 4 is item 2 only:** the `⏰ todo…` progress line
> still carries an emoji and no counts, because `extract_tool_arg`
> (`gateway_runner.rs:241`) only knows the kanban tool's argument names.
>
> **AMENDED A THIRD TIME 2026-11-01 (iter-1602) — item 2 is now CLOSED and
> this audit has no open gaps.** `todo_progress_summary(name, args)`
> (`gateway_runner.rs:363`) counts the four states the tool emits and renders
> `⏰ todo · N tasks · D done · A active · P to do · X dropped`. The root cause
> was not merely that the argument names were unknown: the todo payload's keys
> (`todos` array, `merge` bool, `sessionId` string) hold no string the key lists
> recognise, so the extractor's "first string value from any key" fallback
> returned the model's own `intent` — already on the line. Six tests in
> `tool_progress_tests` pin it, both directions mutation-proven.
>
> **AMENDED A FOURTH TIME 2026-11-01, retracting an overstated claim in the
> THIRD amendment: "this audit has no open gaps" was WRONG.** That sentence
> closed Gap 4's two items and then silently extended itself to Gap 3, which
> was still open — and which is the **TUI subagent preview**, one of the two
> surfaces the operator named. The closed claims stand (Gap 1, Gap 2, Gap 4
> items 1 and 2 are genuinely fixed); the sweep does not.
>
> **Gap 3 is open and is larger than its section above states.** The section
> frames it as "`todo_progress` / `todo_items` have no producer", but the
> verified state (2026-11-01, re-read pass) is that the whole inline swarm
> gallery is inert: four trait defaults in `tui_state.rs` return nothing and
> **no `impl App` overrides any of them** —
>
> | Default | Line | Returns |
> |---|---|---|
> | `inline_swarm_gallery_active` | `tui_state.rs:407` | `false` — gates the entire strip and gallery |
> | `inline_swarm_members` | `tui_state.rs:412` | `Vec::new()` — the gallery's only data source |
> | `swarm_panel_focused` | `tui_state.rs:430` | `false` |
> | `swarm_panel_full_page` | `tui_state.rs:434` | `false` |
>
> `swarm_members_for_transcript` (`tui_state.rs:420`) delegates to the empty
> one, so the transcript cards are dead too. The renderers are all wired
> (`operant_ui/mod.rs:2494` reads the strip gate, `:2498` and `:2836` read the
> members) — it is purely a missing producer, which is the same defect class
> this audit was written about.
>
> **The producer is cheap because the data already exists.**
> `async_delegation::list_records()` is a live process-global registry and
> `async_delegation::records_snapshot()` already maps it to the unified
> listing's row shape. The TUI's own session hub reads exactly that
> (`session_hub/mod.rs:274`), and `background_tasks.rs:184` syncs it. So the
> gallery's source is a mapping over an existing source, not new machinery.
>
> **AMENDED A FIFTH TIME 2026-11-01 (iter-1607) — Gap 3 is now CLOSED.**
> `swarm_gallery_members(live_only)` (`tui/app/mod.rs:1174`) maps the registry
> to `SwarmMemberStatus`, and `tui/app/tui_state_impl.rs` overrides
> `inline_swarm_gallery_active`, `inline_swarm_members` and
> `swarm_members_for_transcript` to read it. The strip now shows live children
> with a `C/T` counter scoped to each child's own `assigned_to` items.
> Four tests pin it; reverting either override to the trait default turns all
> four red.
>
> **What is deliberately NOT done, and recorded in the code as a
> `[port-decision]`:** `swarm_panel_focused` and `swarm_panel_full_page` stay
> at their `false` defaults. No keybind and no overlay state for a swarm panel
> exists anywhere in the tree, so wiring them would mean inventing a
> keyboard-driven full-page view the operator never asked for. The preview the
> request names renders from the unfocused default already.
>
> **Label collision, noted per the numbering protocol:** two commits on
> `origin/main` carry `iter-1597` — this audit's closure (`c8c34fbf`) and a
> peer's process-hardening change (`5bf97977`). Both stay; history is
> append-only.

## The operator's report

> "[T]odo list, and subagent preview within TUI and gateway-channels (telegram, slack, etc) is not complete. I cannot see the todo list or the subagent preview like that of `jcode`."

Both halves turn out to be the same defect class, and it is the class this repo has already been bitten by twice: **the renderer was ported and the producer was not.** `ui_todo_changes.rs`'s own header documents the concept for the delta block ("The transcript renderer is stateless per message…"), and `2026-10-30-subagent-wave-3-deferral-rests-on-a-false-premise.md` documents it for the live tail. Neither doc examined the *todo card* or the *`todo` mark on a subagent*, which are the two surfaces the operator actually named.

## Verified: the TUI renderer for the todo card is complete and green

This half is NOT the gap, and saying so is load-bearing because the operator's report would otherwise suggest the renderer is missing. All of the following exist and pass today (`cargo test -p operant-cli --bin operant todos` → **21 passed, 0 failed**):

| Piece | Location | State |
|---|---|---|
| `TodoItem` / `TodoPlan` / `TodoGoal` + 11 semantic-state enums | `tui/operant_app/todo.rs` | vendored, `phase`/`lane`→`group` aliases accepted |
| The card renderer (`render_todos_message`) | `tui/operant_ui/ui_messages.rs:1110` | renders groups, phase headers, glyphs, confidence, goal feedback loops |
| The payload parser (todos + `Plan:` + `Goals:` + updates) | `tui/operant_ui/ui_messages.rs:995` | full multi-section parse |
| The "what changed" delta block under a `todo` tool row | `tui/operant_ui/ui_todo_changes.rs` | 7 tests, Form A / Form B / cap |
| The pinned band that reads the card payload every frame | `tui/operant_ui/ui_viewport.rs:1466` | live |
| `pin_todos` config, `Alt+X` toggle, `/todos` command | `config_shim.rs:332`, `key_handling.rs:1666`, `commands.rs:1604` | live |
| The info-widget Todos band | `tui/operant_app/info_widget/info_widget_todos.rs` | live |

The card is fed from exactly ONE event — `AgentEvent::ToolComplete` when `result.name == "todo"` (`tui/app/agent_events.rs:244-256`), which sets `self.todos` and `self.todos_card_payload`.

## Gap 1 — the todo card is only ever written, never refreshed

`refresh_todo_card_if_needed` / `refresh_pinned_todos_if_needed` / `todo_card_rendered_hash` do not exist anywhere in operant:

```
$ grep -rn "refresh_todo_card\|todo_card_rendered_hash" crates/operant-cli/src/
(no output)
```

jcode has all three (`crates/jcode-tui/src/tui/app/todos_view.rs:72-143`), and they are what make the card a *view* rather than a *one-shot record*. The mechanism:

- `refresh_pinned_todos_if_needed` throttles a **disk read to once per second**, hashes `(session, todos, plan, goals)`, and only rewrites the payload when the hash moved.
- `refresh_todo_card_if_needed` does the same for the inline chat card, replacing the existing card's content in place (`replace_display_message_content`) so the transcript never stacks duplicates.

**The observable consequence is the bug the operator is reporting.** The core's todo tool persists to disk on every write (`todo_tool.rs:140-172`, `persist_session`, atomic rename). So the list on disk is always current. But the TUI only ever reads it when the *parent's* `todo` tool completes. Which means:

- A **subagent** that writes a todo list produces zero visible change in the operator's card. Its `ToolComplete` never reaches the parent's event stream (`sub_agent_tool.rs:660-663`: "Child agents run HEADLESS: their events are NOT forwarded to the parent channel… the delegation result still returns to the parent as the tool result text").
- A `/new`, a rewind, or a fresh TUI process also shows a stale or empty card until the model happens to call `todo` again — and `todo_tool.rs:112-134` (`load_session`) reads the file back on first access precisely so a fresh process *can* rehydrate, which makes the staleness a TUI-side miss rather than a store defect.

**Root cause:** `crates/operant-core/src/tools/todo_tool.rs` exports only `todo_injection_for_session`, `todo_counts_from_result`, and `is_todo_injection_row` — **no read accessor**. A TUI tick cannot ask "what is this session's list?" without going through a tool result. `crates/operant-cli/src/tui/operant_app/todo.rs:729` records the intended producer set as `load_todos (:849), … save_todos (:860)` and calls them *"the persisted-io slice the picker reads"* — the persisted-io slice was never ported at all, so the picker half reads nothing.

This also explains a second, unrelated-seeming defect: `operant` has no `/todos card` subcommand (no inline chat card that can be dismissed), so the operator's only view of the list is the pinned band — which is the half that is live. If the band is scrolled off or the list is long, there is no other surface.

## Gap 2 — `assigned_to` is carried but never rendered, and nothing matches a live subagent to a todo

oh-my-pi's `#5873` (`packages/tui/src/tools/todo.ts:120-131, 376-397, 655-675`) defines the feature the operator is calling "the subagent preview":

```ts
/** A todo the collapsed viewport treats as current work: the literal
 *  `in_progress` task or a pending task a live subagent is executing. Both
 *  collapsed views (transient tool result + sticky HUD) run this same policy so
 *  they can never disagree about what the agent is doing (#5873). */
function isActiveTodo(task, isMatched) {
  return task.status === "in_progress" || (task.status === "pending" && isMatched(task));
}
```

…and then renders a matched pending row in the **accent** colour, the same as `in_progress` (`todo.ts:391`). That is a subagent preview *inside the todo card* — no separate roster needed.

In operant:

- `TodoItem.assigned_to` exists on **both** sides: the TUI type (`operant_app/todo.rs:324`) and the core tool's wire schema (`todo_tool.rs:219`, carried into `todo_summary` at `:288`). The round trip works.
- **No renderer reads it.** `render_todo_card_item_line` (`ui_messages.rs:2113-2141`) builds spans from `status`, `blocked_by`, `content`, and the confidence label — `assigned_to` is never mentioned anywhere under `tui/`.
- `blocked_by` is rendered only as a glyph (`⊳`) with the blocker's text never shown.

jcode's equivalent does render it: `todo_card_agents` (`todos_view.rs:200-216`) snapshots live swarm members, filters to those any todo's `assigned_to` names, and feeds them into the card payload.

## Gap 3 — the swarm gallery's `todo_progress` / `todo_items` have no producer

`SwarmMemberStatus` already carries the jcode-parity fields, documented in `tui/operant_app/protocol.rs:47-55`:

```rust
/// Todo/plan progress as (completed, total) for this member, when known.
/// Surfaced on the inline swarm strip as a compact "C/T" counter.
pub todo_progress: Option<(u32, u32)>,
/// Compact snapshot of this member's todo list (content + status), capped
/// by the producer. Rendered in the focused inline swarm panel so the
/// coordinator can see what each agent is working through.
pub todo_items: Vec<SwarmTodoItem>,
```

`operant_render/swarm_gallery.rs:218-260` defines `GalleryMember.todo`, `GalleryMember.todo_items` and `GalleryTodoItem` for exactly this. **And the only constructor of `SwarmMemberStatus` in the entire crate sets both to empty** (`info_widget/info_widget_swarm_gallery.rs:702-703`, a test helper).

The gallery's whole access path is a trait default returning nothing:

```rust
// tui/operant_app/tui_state.rs:412
fn inline_swarm_members(&self) -> Vec<...SwarmMemberStatus> { Vec::new() }
fn inline_swarm_gallery_active(&self) -> bool { false }
```

No `impl App` overrides them (verified: `grep -n "inline_swarm_members" crates/operant-cli/src/tui/app/` → empty). So the gallery, the strip, and the focused panel are all wired to a source that returns no members. jcode's `members_to_gallery` (`info_widget_swarm_gallery.rs:100-135`) maps `member.todo_progress` → `GalleryMember.todo` and `member.todo_items` → `GalleryMember.todo_items`; operant has the mapper and `Default` for those fields.

## Gap 4 — RETRACTED. The gateway ships a delegation roster; only the todo list is missing

> **This section was wrong on the first pass and is rewritten.** See the
> amendment at the top of this document. The original claimed "no todo card, no
> delegation roster, no `/todos`, no `/subagents`", which is false for the
> roster. What the evidence supports, after looking in the file the original
> greps never covered:

**The delegation roster exists and is the right surface.** `/subagents` is a
registered gateway command (`gateway_commands.rs:290`, alias `subagent`) whose
handler (`:1504`) reads `async_delegation::list_records()` and marks each row
with `⏳ running` / `✅ completed` / `❌ failed`, an elapsed age measured at the
display layer, the untruncated one-lined goal, the model, and a 120-char result
or error snippet. `gateway_commands::subagents_command_tests` pins it — 5 tests,
all green — including that `/agents` and `/subagents` resolve to distinct
commands. It landed in iter-1500, 89 iterations before this audit, which is why
the plan's § 2.2 row that this audit cited as "still unshipped" was stale.
So the operator's "I cannot see the subagent preview" is **not** the gateway
roster: on every one of the seven platforms they already have it.

**What the gateway does lack, narrowed to what the evidence supports:**

1. **No todo surface.** `crates/operant-core/src/gateway/` and
   `gateway_commands.rs` between them have no command that reads the todo
   store, so a channel operator cannot ask for the plan. The todo store now has
   the read accessor to build one on (`todo_list_for_session`, iter-1590), which
   is the same accessor the TUI tick uses — so a `/todos` would read the same
   truth as the TUI card, not a second definition of it.
2. **The `todo` progress line carries an emoji and nothing else.**
   `tool_preview_line` (`gateway_runner.rs:310`) renders `⏰ todo…` because
   `extract_tool_arg` (`:241`) only knows `action` / `task_id` / `title` — the
   kanban tool's argument names. The todo tool's actual payload (`todos`, a JSON
   array) is not one of them, so the line has no counts and no item text. The
   `ToolProgressTracker` it feeds (`:537-603`) collapses repeated identical lines
   into `(×N)`, which means a five-iteration todo loop prints five identical
   `⏰ todo` bubbles — the exact case the counter exists for, with no useful text
   to collapse.

This is the honest remaining gateway work, and it is materially smaller than the
gap the original audit described.

## What jcode does that operant does not, in one table

| Mechanism | jcode | operant | Verdict |
|---|---|---|---|
| Card fed from a tool result | yes (`show_todo_card`) | yes | parity |
| Card fed from a **disk tick** | `refresh_todo_card_if_needed`, hashed | none | **Gap 1** |
| Pinned band fed from a **disk tick** | `refresh_pinned_todos_if_needed`, 1s throttle | ToolComplete only | **Gap 1** |
| Inline chat card + dismiss gesture | `/todos card`, jcode's `todo_card_key_label()` | no card command | **Gap 1b** |
| Subagent → todo match, accent | `todoMatched` / `#5873` | none | **Gap 2** |
| `assigned_to` on the card | `todo_card_agents` | carried, unrendered | **Gap 2** |
| Subagent `todo_progress` / `todo_items` on the gallery | `member_display_status` + `members_to_gallery` map them | fields exist, always empty | **Gap 3** |
| `/todos` in the gateway | n/a (jcode has no channel surface) | no todo command — but `/subagents` **exists and works** (iter-1500) | **Gap 4 (todo only)** |
| Delegation roster in the gateway | n/a | `/subagents`, 5 tests green | **parity** |

## What to take from oh-my-pi, and what not to

oh-my-pi's todo tool (`packages/coding-agent/src/tools/todo.ts`) is a *verb* tool, not a state tool: `op: init | start | done | drop | rm | block | unblock | append | view`, addressing tasks **by content** rather than by id, with a single-`in_progress` normalisation after every op (`normalizeInProgressTask`, :139-158).

**Take, cheaply:**

1. **`waiting_on`** — jcode's `todo_card_background_tasks` (`todos_view.rs:144-191`) lights a blocked todo from a *live background task* it names. operant has `blocked_by` (id list) and already renders a glyph; the missing half is a resolver that maps those ids to `BackgroundTaskRegistry` rows and prints "⊳ … · swarm await 40%".
2. **Closed-row context in a collapsed list** — oh-my-pi keeps {@link COLLAPSED_CLOSED_CONTEXT} (one) closed task above the open window so completions are visible as they happen. operant's `render_todos_message` shows the whole list; a long card makes that expensive, and the pinned band already truncates at 1/3 of the viewport (`ui_viewport.rs:1471`), so this is a *card-side* improvement, not a band one.
3. **`blocked` as a first-class status with a reason note** — oh-my-pi's `blocker` rides the row as text, its markdown round-trip puts it in a trailing HTML comment so it survives edit/export. operant has `blocked_by: Vec<String>` (ids) and drops the note entirely.

**Do not take:** the verb/op schema. operant's tool already models phase/lane/goal/confidence, which jcode's schema is the parity target for (AGENTS.md design preferences name jcode as the geometry baseline), and swapping the tool's whole shape would invalidate the persisted per-session files and the model-facing contract for zero gain on the surfaces the operator named. `blocked_by` stays id-based; the *renderer* gains the resolver above.

## The recommendation, in ponytail terms

Four gaps, four small diffs, no core-schema change, no migration:

- **Gap 1** is one public read accessor in `todo_tool.rs` (`pub fn load_session_public(session) -> Vec<TodoItem>`, wrapping the existing private `load_session`) plus a 1s-throttled hashed tick on `App` that calls it, and it fixes *both* the subagent-blind card *and* the stale-after-restart card. Largest visible win per line.
- **Gap 2** is matching `App`'s live subagent rows (feeable from the delegation registry that `session_hub` already reads: `async_delegation::list_records`, `session_hub/mod.rs:279`) against `todo.assigned_to` and accenting the matched row. ~30 lines.
- **Gap 3** is populating `SwarmMemberStatus` from that same registry + the child's todo file. The child's session id is not currently carried on the record (`AsyncDelegationRecord` has no `session_id` field), so the todo snapshot per member needs either that field or a content-hash join — this is the one gap with a real design decision, and it should be scoped separately.
- **Gap 4** is the todo half only, and it is now cheap: a `/todos` that reads
  `todo_list_for_session` (which iter-1590 added) renders through the same
  formatting path `/subagents` already uses, so all seven platforms get it at
  once. The plan's § 2.2 row is CLOSED and should be struck — `/subagents`
  shipped in iter-1500. `extract_tool_arg` also needs `todos` added so
  `⏰ todo…` stops printing an information-free bubble.

Gaps 1 and 2 are fixed (iter-1590 code, iter-1593 tests); Gap 3 is the one with
a real design decision and stays open.
