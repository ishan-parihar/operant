# Audit: the todo card and the subagent preview are both renderer-only — their producers are unwired

**Date:** 2026-10-31
**Kind:** audit (dated-frozen)
**Amends:** `docs/plans/2026-10-30-subagent-preview-and-management-upgrade.md` § 3.1 and § 3.2
**Prior art:** `docs/audit/2026-10-30-subagent-wave-3-deferral-rests-on-a-false-premise.md` (which already corrected one false premise in that plan), the `[port-excision]` / `[port-decision]` annotations in `crates/operant-cli/src/tui/operant_app/todo.rs`, and the header comment at `crates/operant-cli/src/tui/operant_ui/ui_todo_changes.rs:5-19`.

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

## Gap 4 — the gateway has no todo or subagent surface at all

The operator named "gateway-channels (telegram, slack, etc)" specifically. In `crates/operant-core/src/gateway/` the word "todo" appears in **zero** files; `crates/operant-cli/src/gateway_runner.rs` has only emoji/argument plumbing (`:165`, `:241`) and tests (`:6479-6489`). No todo card, no delegation roster, no `/todos`, no `/subagents`.

The plan's § 2.2 already scoped the gateway's `/subagents` command and it is still unshipped. The gateway's progress mechanism is a *tool-line* tracker (`gateway_runner.rs:537-603`: `msgs: ProgressMap`, `on_tool_start`, `attach_message_id`, `close_group`) that collapses repeated identical tool lines into `(×N)` counters — a natural place for a todo status line, and the natural place for the delegation roster the plan scoped.

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
| `/todos` in the gateway | n/a (jcode has no channel surface) | nothing | **Gap 4** |

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
- **Gap 4** is `/subagents` per the existing plan § 2.2 (~40 lines, one formatting path, all seven platforms).

Gaps 1 and 2 are the operator's report and should land together; 3 and 4 are already scoped in the 2026-10-30 plan and are not blocking.
