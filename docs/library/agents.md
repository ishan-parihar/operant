# Agents: the definition, the roster, the worker, and the pane

Owns: what an "agent" means in this harness, as four separate things that the
word is used for: the **definition** (a markdown file an operator edits), the
**roster** (the live list of who is working), the **worker** (a child the model
spawns to do a bounded task), and the **pane report** (what an outside process
is told about this one).
Not for: the mechanics of delegation itself, which
[sub-agents.md](sub-agents.md) owns, and the TUI's overlay catalogue, which
[tui.md](tui.md) owns. This page owns the agent-shaped surfaces and the seam
between them.

Source: `crates/operant-cli/src/tui/agents_view.rs` (the definition record and
its editor), `crates/operant-cli/src/tui/app/agent_events.rs` (the live roster),
`crates/operant-core/src/tools/sub_agent_tool.rs` (the worker), and
`crates/operant-core/src/herdr.rs` (the pane report).

## Contents

- [The four things "agent" means here](#the-four-things-agent-means-here)
- [The definition record](#the-definition-record-agents_viewrs)
- [The live roster](#the-live-roster-agent_eventsrs)
- [The worker](#the-worker-sub_agent_toolrs)
- [The pane report](#the-pane-report-herdrs-and-herdr_lifecyclers)
- [The pane guard](#the-pane-guard-herdr_pluginrs)
- [Configuration](#configuration)
- [Surfaces](#surfaces)
- [Failure modes](#failure-modes)
- [Intention](#intention)

## The four things "agent" means here

The word carries four loads, and they are separate code with separate lifetimes.
Conflating them is the most common way to misread this area:

| sense | what it is | where it lives | lifetime |
|---|---|---|---|
| definition | a markdown file describing a reusable agent | `.operant/agents/*.md` and the operant home | on disk |
| roster entry | one live delegation, by tool call id | `App.agent_status` | the process |
| worker | a child model loop with its own history | `sub_agent_tool.rs` | one task |
| pane report | a line to an outside program naming this agent | `herdr.rs` | the process |

A definition is not a worker. Nothing in the delegation path reads
`.operant/agents/`: the only readers of that directory are
`load_agent_definitions` and its own tests
(`crates/operant-cli/src/tui/agents_view.rs`), and the tool that spawns a
worker takes no agent-name argument at all (its fields are `goal`, `context`,
`toolsets`, `tasks`, `role`, `max_iterations`, `timeout_seconds`,
`output_schema`, `working_dir`, `background`, `query`, `request_key`). The
definition files are a catalog an operator edits and reads. They are not a
dispatch input, and no code claims otherwise.

## The definition record (`agents_view.rs`)

`AgentDefinition` (`:67`) is what one markdown file parses into:

| field | source | note |
|---|---|---|
| `name` | YAML `name:`, else the file stem | the stem is the fallback, so a file with no front matter still loads |
| `model` | YAML `model:` | `None` means the runtime default; the editor seeds `claude-sonnet-4-6` |
| `memory_scope` | YAML `memory_scope:`, else `memory:` | both spellings are accepted on read; only the first is written |
| `description` | YAML `description:` | quoted with JSON escapes on write so a multi-line value stays one line |
| `tools` | YAML `tools: [a, b]` | empty means all tools |
| `instructions` | the markdown body after the closing `---` | the prompt |
| `source` | hard-coded `"user"` | see the caveat below |
| `shadowed_by` | always `None` on load | see the caveat below |

The parser is hand-rolled, not a YAML crate: `extract_yaml_str` scans for a
`key:` prefix and, if the value starts with a double quote, reads it back with
`serde_json` so a description written by the editor round-trips exactly
(`:430`). A file without front matter is not an error: the stem becomes the
name, the first line becomes the description, and the whole file becomes the
instructions (`:406`).

Two fields are read but never computed. `source` is only ever assigned
`"user"` (`:420`), even though the doc comment on `:65` says the value may be
`plugin:{name}` or `builtin`. `shadowed_by` is only ever assigned `None`
(`:425`), so the shadow marker the list renders (`:650`) and the warning the
detail view renders (`:727`) are both unreachable in a normal run. Both are
honest dead ends rather than broken behaviour: the editor refuses to overwrite
a file that already exists (`:331`), so two definitions cannot collide on one
path in the first place.

### Loading, and the order two roots are read in

`load_agent_definitions` (`:365`) reads exactly two directories:

1. the operant home's `agents/` directory, resolved through
   `operant_core::platform::operant_home()` (so a profile or `HERMES_HOME`
   moves it), then
2. the project's `.operant/agents/`.

Both are read with `read_dir` and only `*.md` files are parsed. A directory
that does not exist is skipped rather than reported, so a project with no
`.operant/agents/` is not an error. There is no deduplication pass and no
priority rule: a name present in both roots appears twice in the list, and the
project copy appears second.

### Editing and saving

`AgentsMenuState` (`:202`) holds the list, the live roster, and the editor.
The editor is a six-field form (`AgentEditorState`, `:123`) walked in the
order Name, Model, Memory, Tools, Description, Prompt
(`AgentEditorField::next`, `:93`). Newlines are accepted only in Description
and Prompt; elsewhere Enter advances a field (`:293`).

`save_editor` (`:310`) does four things in order: validate, resolve the path,
refuse a collision, write. The path is the existing file's path when editing,
otherwise `<project>/.operant/agents/<slug>.md` where the slug is lowercase
ASCII with runs of space, dash, underscore and dot collapsed to one dash
(`slugify_agent_name` at `:461`). The collision check uses `symlink_metadata`,
not `exists`, so a dangling symlink is refused too: `fs::write` would have
followed it to whatever it points at.

Validation (`validate_editor`, `:473`) requires a non-empty name that slugifies
to something, a model, a description, and a prompt body. `memory_scope` and
`tools` are optional.

## The live roster (`agent_events.rs`)

The roster is `App.agent_status`, a `Vec<(String, String)>` of tool call id to
a status string (`crates/operant-cli/src/tui/app/mod.rs:398`). It is not a
list of agents in the definition sense: it is one row per **delegation the
model has in flight**, keyed by the tool call that started it.

Three events drive it:

- **ToolStart** (`agent_events.rs:179`) - when the tool name is
  `delegate_task` or `spawn_subagent`, the id is inserted (or replaced) as
  `running`, or `queued` if the call has been parsed but not yet admitted by
  the concurrent pool.
- **ToolComplete** (`:213`) - the matching row becomes `done`, or `error`
  when the result was a failure. This is an id match, not a name match, so it
  works even after `/new` or `/rewind` cleared the transcript block
  (`:207`).
- **ToolError** (`:293`) - the same id is set to `error`.

`clear_tool_blocks` (`:901`) is the fourth writer. Clearing the transcript
removes the block a running delegation would have reported against, so every
row still `running` or `queued` becomes `abandoned`. Without it a delegation
silently disappears from the roster with no terminal state.

`open_agents_menu` (`crates/operant-cli/src/tui/app/commands.rs:2626`) is the
one converter from that string to the `AgentStatus` enum the view renders
(`crates/operant-cli/src/tui/agents_view.rs:26`, with `Idle`/`Running`/`WaitingForTool`/`Complete`/`Failed`
and a colour per state). The mapping is liberal on purpose: `complete`,
`completed` and `done` all map to Complete; `failed` and `error` both map to
Failed; anything unrecognised falls through to Idle.

`App::subagent_status` (`tui_state_impl.rs:492`) is the one-line summary the
status strip shows: the first row still running or queued, else the first row
whatever it says. A finished or abandoned delegation must not hide a live one.

Note the two names the roster keys on. `delegate_task` is the registered tool
(`TOOL_NAME` in `sub_agent_tool.rs:50`, and it is in `builtin_tool_names`). The
string `spawn_subagent` is also matched in the TUI and in
`agent::builders::list_subagents` (`:852`), but no tool module registers it: a
grep for the literal finds only those three consumer sites. Matching a name
nothing registers is harmless and future-facing, not a live path.

## The worker (`sub_agent_tool.rs`)

The worker is a real second agent loop. `SubAgentTool::new` (`:587`) takes the
parent's client, the model, the parent's depth, the parent's toolsets and the
parent's disabled tools and toolsets, and `with_parent_approval` (`:647`) adds
the parent's approval allowlist and the file it persists to. A child built from
`AgentConfig::default()` would otherwise have an empty allowlist, so a gated
call would be refused with advice a headless agent cannot follow.

Two guards run before any worker exists:

- **Depth.** `SpawnPermit::reserve(1, child_depth)` (`:685`) refuses when the
  child depth is past the configured maximum. The message names both numbers
  and says workers cannot recurse past the floor.
- **Breadth.** `LIVE_WORKERS` is a process-wide `AtomicUsize`, and `reserve`
  claims its slots in one `fetch_update`. A plain load-then-store would let N
  parents each observe free capacity and each admit M children, which is the
  N*M blow-up the counter exists to prevent. Release lives in `Drop` and
  nowhere else, so every exit path returns the slots.

A swarm reserves `tasks.len()` slots at once (`:1020`). Workers past the
reservation are reported as refused rather than dropped silently, and because
they were never built they cost no model stream. The refusal text tells the
parent to retry or lower the fan-out; an earlier version told it to wait for a
worker to finish, which a refused swarm cannot do because `query=` resolves
background delegations only.

One worker property surprises people: a child's working directory is the
process working directory, because every file tool resolves relative paths
through the process. `resolve_child_cwd` (`:2016`) therefore answers "the
process cwd" or refuses. A caller that passed a different path used to get
silence: the child ran wherever the process happened to be, so the parent
believed it had isolated the child while the child edited the shared tree.

## The identity switch (`agent_identity.rs`) and the subagent navigator (`subagent_nav.rs`, `subagent_navigator.rs`)

Two surfaces carry the agent identity rather than the loop.

- `agent_identity.rs` (`tui/app/`) is the session-scoped employee switch behind `Shift+Tab` and `/agent`. It sets the acting agent for THIS conversation only — the agent's bylaws seat and charter, and the `sessions.employee_id` binding — and writes nothing to the config, so two sessions in the same process keep their own seats and an operator instruction is never silently re-pointed across conversations.
- `subagent_nav.rs` (`tui/app/`) is the App-side seam: it collects this session's own descendants outward from the current session id and mounts the navigator surface on them.
- `subagent_navigator.rs` is the surface itself, listing the current session's children from the in-process registry plus the durable ledger. Sessions that share a swarm in the repo but were not spawned by this session are deliberately excluded — the hub shows what this session owns.

## The pane report (`herdr.rs` and `herdr_lifecycle.rs`)

Herdr is a terminal workspace manager that shows which agent is running in
each pane. It can recognise an agent two ways: by reading the agent's screen
against a detection manifest, or by the agent reporting its own state. Herdr's
own documentation is explicit that a detection manifest cannot teach it a
brand-new agent name, so self-reporting is the only route, and that is what
this integration does.

`herdr.rs` owns the wire. `HerdrContext::from_env` (`:112`) reads
`HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_BIN_PATH` and `HERDR_SOCKET_PATH`; an
incomplete context yields `None`, because a half-present context makes every
report fail. `HerdrReporter::start` (`:246`) spawns a dedicated OS thread
rather than a tokio task, so the reporter works whether or not the caller has a
runtime. `enqueue` (`:396`) keeps only the newest pending command: Herdr only
cares about the latest state, and a queue would let a stale `working` land
after a newer `idle`. `run_with_timeout` (`:489`) bounds one invocation at
`REPORT_TIMEOUT` (2s) and retries a non-zero exit once after 150ms, because
Herdr exits non-zero when its server is momentarily busy. A report that fails
twice is lost, which is the same outcome as before the retry, and never
touches the agent.

`herdr_lifecycle.rs` owns the timing. `report_herdr_event` (`:141`) is called
from `handle_agent_event` first, before anything else, and maps events to
states through the pure function `herdr_state_for_event` (`:199`): Thinking,
Reasoning, Content, ToolStart, ToolComplete, ToolError, IterationComplete and
TurnStarted mean working; Done and Error mean idle; anything else maps to
`None` and reports nothing, so a usage or rate-limit event leaves the last true
state on screen. A completed tool is working, not idle, because the model
usually runs another iteration after one.

The session id is synced at turn boundaries only. A turn emits hundreds of
content deltas and re-reading the agent's session id on each would put a lock
acquisition in the hot path for no benefit: the id cannot change mid-turn.

Blocked is the state that matters most, and it is reported from the two places
that actually block: a permission request (`channel_drains.rs:406`) and a
gateway blocked message (`:558`). Both carry the question itself, truncated to
120 characters, because that is what Herdr renders next to the badge.

The config switch is read in one place, `herdr_reporter` (`:52`), so
`[integrations.herdr] enabled = false` turns off the reports and the release
in one decision. `release_herdr_agent` (`:224`) runs at TUI teardown: without
it the pane keeps an `operant` badge for a second or two after the process is
gone, which is the kind of small lie that makes an operator stop trusting the
badge.

## The pane guard (`herdr_plugin.rs`)

The reporter is only half the story: it makes the pane visible, and
`herdr_plugin.rs` is what tells the operator the pane is *correctly* visible.
`live_agents` (`:258`) runs `herdr agent list` and parses its JSON with
`:270`'s parser, split out so the shape can be tested without a Herdr server.
`mislabelled_operant_panes` (`:297`) finds panes Herdr attributes to some
other agent while Operant is the foreground process, by asking `herdr pane
process-info` per pane. The scan is bounded at 16 panes, because it is one CLI
call each; a machine with more panes still gets its Operant panes reported
(those need no scan) and the mislabel check degrades to not-checked.

This is the guard that catches the failure the integration exists for. Without
a reporter, Herdr's screen-based detection matches the Pi manifest against
Operant's TUI and the pane shows up as a Pi agent. Nothing is wrong with
Operant and nothing is wrong with Herdr: the binary in that pane is older than
the integration, or it was started outside the reporter's reach.

## Configuration

There is no configuration for definitions, the roster or the worker. The one
key in this area is:

| key | type | default | what it changes |
|---|---|---|---|
| `integrations.herdr.enabled` | bool | `true` | report this process's lifecycle state and session id to the Herdr pane that owns it. On by default, because without it a Herdr pane running operant looks like a plain terminal. Off keeps Operant out of Herdr's sidebar entirely. |

Read at `crates/operant-core/src/config.rs` (`HerdrSettings`, `:365`) and
applied once at startup (`crates/operant-cli/src/main.rs:3451`). Everything
else here is code constants: the worker's default timeout (600s, floor 30s),
the default child fan-out (3), the process-wide live-worker default, the
retained-terminal cap (50), and the report timeout (2s). Those are not
operator-facing knobs and are not config keys.

## Surfaces

There is no `operant agents` command. The `Commands` enum in
`crates/operant-cli/src/main.rs` has no Agents variant, and no `cmd_agent*.rs`
module exists. The agent surfaces are:

| surface | what it does | where |
|---|---|---|
| `/agents` (alias `/tasks`) in the TUI | opens the agents menu: live roster on top, then the definition list, then an editor | `commands.rs:236`, `tui/app/commands.rs:566` |
| `ctrl+s` in the editor | validate and save the definition to disk | `tui/app/key_handling.rs:2752` |
| `/agents` over a gateway | lists that gateway's platform sessions, not delegations | `gateway_commands.rs:1887` |
| `/subagents` over a gateway | lists the background delegations this process dispatched, with goal, model, age and status | `gateway_commands.rs:1907` |
| `GET /api/subagents` | the same registry as JSON, for the dashboard's peer rows | `gateway_api.rs:219` |
| `operant doctor` | reports the Herdr plugin's install state, what Herdr currently sees, and any mislabelled pane | `cmd_doctor/checks_integrations.rs:88` |
| `operant doctor --fix` | installs or refreshes the Herdr and Omarchy integrations | `integrations/mod.rs` |

The gateway's `/agents` and `/subagents` are deliberately different commands.
`/agents` lists the gateway's platform sessions (a Telegram chat, a Discord
channel), while `/subagents` lists the background delegations the process
dispatched. The two never overlap, which is why `/agents` alone left an
operator with no view of a running subagent.

## Failure modes

- **The definitions directory does not exist.** `read_dir` fails and is
  skipped. The menu opens with an empty list and a working "create" row. This
  is the normal first-run state, not an error.
- **A definition file has no front matter.** The file stem becomes the name,
  the first line becomes the description, the whole file becomes the prompt. It
  loads.
- **A definition file has malformed front matter** (no closing `---`).
  `parse_agent_def` returns `None` and the file is silently skipped. The menu
  shows no row and no error, so an operator who can see the file on disk may
  conclude the menu is broken.
- **Saving over an existing path.** Refused by name, with the path in the
  message, because the check is `symlink_metadata` and covers dangling
  symlinks. This is the intended behaviour and the test
  `creating_an_agent_refuses_to_overwrite_an_existing_file` pins it.
- **Two definitions with the same slug in different roots.** Both load, both
  render, and there is no warning, because nothing computes `shadowed_by`.
- **A delegation the model never completes.** The roster row stays `running`
  until `clear_tool_blocks` marks it `abandoned`. It is never removed, so a
  long session accumulates terminal rows.
- **The Herdr binary is absent.** `reporter()` returns `None` and every call
  is a no-op. The agent runs normally; the pane simply looks like a plain
  terminal to Herdr.
- **Herdr is present but its server is busy.** The report is retried once,
  then dropped. No user-visible signal, by design.
- **The pane shows the wrong agent name.** `operant doctor` reports it as a
  fault with the pane id and the label Herdr is using, and tells the operator
  the binary in that pane predates the integration.
- **A child's working directory is requested as something else.** The spawn is
  refused with the process working directory in the message, rather than
  accepted and ignored.

## Intention

The four senses of "agent" are separate code because they fail separately. A
definition that stops parsing costs an operator a row in a menu; a roster that
stops updating costs them visibility into work that is still running; a worker
that loses its permit costs the process capacity it never gets back; a pane
that stops reporting costs an outside program the ability to see Operant at
all. Merging them would make all four fail together.

The definitions are a catalog rather than a dispatch input on purpose, and that
is the thing most likely to surprise a reader who arrives expecting agent
files to configure workers. They do not, and the honest response is to say so
rather than to imply a wiring that does not exist. If that wiring is ever
wanted, `SubAgentArgs` is the place it would land, and the field would have to
be threaded through `call_unguarded` and the child prompt builder, because
today the child is built from the parent's configuration plus its own goal.

The pane report is designed around one asymmetry: Herdr cannot be taught a new
agent name, but it can be told one. Every design choice in `herdr.rs` follows
from that, including the ones that look like over-engineering. The single-slot
pending queue, the dedicated thread, the two-second bound and the silent retry
are all there so that a workspace manager's opinion of this process can never
become this process's problem.
