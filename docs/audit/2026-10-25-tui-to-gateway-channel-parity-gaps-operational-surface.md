# TUI to gateway channel parity gaps — operational surface

Dated-frozen: audited `2026-10-25`; findings mark verified vs asserted.
Investigator: operator agent (session 2026-10-25, iter-1276 candidate).

## Scope

Answer one question: **which parts of the operational surface a TUI user can
reach are also reachable from the chat platforms (telegram, discord, slack,
whatsapp, email, sms, webhooks), and which of the gateway's advertised
commands actually work?** Everything below is static analysis with file:line
citations; no live bots were driven. The audience is the fix plan that follows:
every row names the mechanism that would close it.

Two inventories were extracted mechanically:

- **TUI**: 110 `RegisteredCommand` entries (109 unique names) in
  `crates/operant-cli/src/tui/operant_app/app.rs:190-362`, all pinned wired by
  the `no_registered_tui_command_is_unwired` guard test
  (`crates/operant-cli/src/tui/app/tests.rs:5923`).
- **Gateway**: 52 `CommandDef` entries (61 unique names incl. aliases) in
  `COMMAND_REGISTRY` (`crates/operant-cli/src/gateway_commands.rs:120-541`),
  dispatched from the inbound loop at `gateway_runner.rs:3898` via
  `resolve_command` (three-state; unknown tokens are rejected, never forwarded
  to the model — `gateway_commands.rs:587`).

## Findings

### F1 — Eleven gateway commands are metadata-flag theater [verified]

`gateway_commands.rs::handle_command` replies "success" for commands that only
write a session-metadata key **no runtime consumer reads**. Verified by
grepping every key across all crates — each appears ONLY in its own handler:

| command | dead flag | handler line | promise in reply text |
|---|---|---|---|
| `/undo` | `undo_requested` | 1001-1003 | "rolled back 2 messages" |
| `/retry` | `retry_requested` | 979-980 | "flagged for retry" |
| `/rollback` | `rollback_requested` | 1209-1212 | "flagged for rollback" |
| `/compress` | `compress_requested` | 1229-1231 | "flagged for compression" |
| `/queue` | `queued_prompts` | 1031-1033 | "now has queued prompts" |
| `/background` | `background_task`/`background_queued` | 1253-1256 | "pending background task" |
| `/steer` | `steer_message`/`steer_active` | 1294-1297 | "steer updated" |
| `/topic` | `topic_mode`/`topic_active` | 1437-1448 | "separate conversation context" |
| `/reload-mcp` | `mcp_reload_requested` | 1830-1832 | "Servers will be reconnected on next turn" |
| `/reload-skills` | `skills_reload_requested` | 1849-1851 | "directory will be rescanned" |
| `/goal` + `/subgoal` | `goal`, `goal_paused`, `subgoal` | 1336-1366, 1375+ | "will guide the agent's responses" |

The real `SessionStore::update_session_metadata`
(`crates/operant-core/src/gateway/types.rs:471-489`) does store these in
memory, but **nothing anywhere reads them back** except `/usage`'s read of
`message_count` (which itself is only ever written by `/undo`'s decrement —
`gateway_commands.rs:1002` — so the usage numbers are self-referential
`/undo`-decrement theater too).

Ten of these have a REAL TUI counterpart against real machinery
(`/undo` → rewind (`tui/app/commands.rs:685`), `/retry` → rewind+resubmit
(:702), `/compact` → real context fold (:765), `/steer` → the live
`steer_queue_handle` drained by `drain_steers` (`operant-core/src/agent/
builders.rs:755`, `run.rs:2415`), `/queue` → the real queue, `/reload-mcp` →
real reconnect). The gateway forms lie by design: they reply success without
touching the runtime handle the TUI command holds.

**Mechanism to close**: the `CommandContext` (`gateway_commands.rs:16-46`)
needs the runtime handles the TUI app owns (steer queue, rewind, compact,
MCP manager, skills registry). `GatewayMessageHandler` has an
`Arc<OperantAgent>` — a `with_agent_handle` on the context plus per-command
wiring closes ten of the eleven. The `PersistentSessionStore` variant
(`gateway_session.rs:1760`) discards `_updates` entirely and
`find_session` (:1654) returns `metadata: HashMap::new()` — the in-memory
`SessionStore` is the only live metadata path; commands should stop writing
flags and start calling machinery.

### F2 — `/title` and `/branch` mutate clones then drop them [verified]

`/title` (gateway_commands.rs:1146-1176) inserts into a session it then
discards, with its own comment admitting "we can't persist metadata changes
through the current SessionStore API". `/branch` (:1178-1200) calls
`update_session_metadata` — but with the SOURCE session's (platform, user,
channel) triple, which re-annotates the source row and never touches the new
branch session the code just created. Reply says "New branch session: `<id>`".

**Mechanism**: `SessionStore` needs `set_title`/`update_metadata_by_id` (it
keys sessions by uuid — `types.rs:344-356`); `/branch` must write to
`new.session_id`, not the source triple.

### F3 — Even the "fixed" override path is half-live [verified]

`/model` writes `model_override` (`gateway_commands.rs:1522-1526`) and the
turn path DOES read it back (`gateway_runner.rs:1525-1547`, applies via
`agent.set_model()`). That path is real. But the same turn-path audit shows
**only `model_override` is read** — no other metadata key is consulted at turn
start (`grep 'metadata.get(' gateway_runner.rs` → exactly one hit). So of all
the "15 slash commands wrote to metadata but the runner never read it back.
Now /model, /yolo, /fast etc. actually take effect" claim
(`gateway_runner.rs:1519-1521`), only `/model` and `/yolo` (R17's live
`YOLO_CHANNELS` set) are live; the comment overstates the rest.

**Mechanism**: either wire the consumers (see F1) or stop advertising the
commands.

### F4 — Inbound reality per platform [verified]

Only three surfaces can ever reach `handle_command`:

| platform | inbound | evidence |
|---|---|---|
| telegram | ✅ long-poll → `start_with_channel` (`telegram.rs:847`) | real |
| discord | ✅ gateway WebSocket MESSAGE_CREATE (`discord.rs:361-373`) | real |
| webhook | ✅ axum routes POST/GET `/webhook/{route}` (`webhook.rs:379-383`) | real, inbound-only |
| slack | ❌ `start_with_channel` is `self.start()` — logs and returns (`slack.rs:403-408`) | no Socket Mode |
| whatsapp | ❌ `start_with_channel` → `start()` → log line only (`whatsapp.rs:219-224`) | outbound-only |
| email | ❌ same (`email.rs:311-316`; RT-86 admission at :299-304) | outbound-only |
| sms | ❌ same (`sms.rs:191-196`) | outbound-only |

AGENTS.md documents Slack/WhatsApp/Email/SMS as outbound-only, so this is a
KNOWN limit, not a new bug — but it means the whole command surface is
currently a **Telegram+Discord+webhook feature**. Any "operate the agent from
Slack" story requires the Socket Mode WebSocket first (RT-86 tracks it).
`IncomingMessage.media_urls` (`gateway/types.rs:162-167`) is populated by
Telegram (photo/voice/document enrichment, `gateway_runner.rs:4272-4288`) and
injected into the query (:1475-1490) — media input parity exists on Telegram
only.

### F5 — estop blocks the resume path from channels [verified]

The global pause (`operant pause` → `estop::engage`) gates NEW TURNS inside
`Gateway::handle_message` (`operant-core/src/gateway/mod.rs:607-618`): while
engaged, every inbound message gets "⏸️ Operant is paused… Resume with
`operant resume`." But command dispatch happens EARLIER, in the inbound loop
(`gateway_runner.rs:3898`), so commands still work while paused — EXCEPT the
one you need: there is no `/resume` gateway command at all, so a channel user
who paused the agent cannot resume it from the channel. The reply text
literally instructs them to use the CLI.

**Mechanism**: add `/pause` and `/resume` to `COMMAND_REGISTRY` calling
`estop::engage/disengage` directly — the dispatch point already precedes the
gate, so it works while paused. (Gate: admin_only.)

### F6 — TUI-only operational surface (no channel equivalent) [verified]

46 TUI commands have no gateway counterpart (set-diff of the two registries;
TUI names normalized minus gateway names+aliases). Grouped by what would close
each:

- **Agent-flow (needs runtime handles — same F1 mechanism)**: `/cancel`,
  `/compact`, `/fix`, `/improve`, `/refactor`, `/plan`, `/context`,
  `/context-files`, `/effort`, `/thinking`, `/memory`, `/rewind`,
  `/rewind`/`/rollback` (TUI has rewind machinery via `agent/rewind.rs`),
  `/transcript`, `/review`/`/diff`/`/changes`, `/judge`, `/bundle`,
  `/output-style`, `/skin`, `/theme`, `/statusbar`, `/hotkeys`, `/vim`,
  `/mouse`, `/terminal-setup`, `/redraw`, `/stats` (gateway `/usage` exists),
  `/cost`, `/info`
- **Provider/auth**: `/auth`, `/login`, `/logout`, `/connect`,
  `/import-config`, `/init`, `/setup`, `/refresh-model-list`, `/doctor`
- **Session views**: `/rename` (gateway `/title` exists but is broken — F2),
  `/save`, `/export`, `/search`/`/find`, `/journey`, `/logs`, `/sessions`
  picker
- **Shell-out (TUI suspends to the CLI subcommand, output lands in the
  terminal)**: `/cron`, `/kanban`, `/curator`, `/suggestions`, `/pause`,
  `/unpause`, `/org`, `/platforms` — via `shell_out_to_subcommand`
  (`tui/app/commands.rs:1554-1562`). The gateway has `/kanban` and
  `/curator` entries but they are "use the CLI" redirect stubs
  (`gateway_commands.rs:1896-1926`).

Not every TUI command NEEDS a channel twin (rendering commands — `/theme`,
`/skin`, `/redraw`, `/vim`, `/mouse` — are meaningless on a chat platform;
their state is the user's terminal). The functional core that does need one
for "full operational functionality navigable from channels": session
undo/retry/compact/steer/queue (F1), pause/resume (F5), model/effort/
thinking, memory ops, skills (`/skill <name>` runs a skill by building the
invocation message — `tui/app/commands.rs:394-420`; a channel `/skill` would
inject the same message into the turn), MCP reload, and the read-only
inspectors (`/context`, `/stats`, `/cost`, `/info`, `/transcript`).

### F7 — Gateway-only commands (no TUI twin) [verified]

14: `/approve`, `/deny`, `/background`, `/restart`, `/rollback`,
`/compress`, `/deny`, `/grant`, `/revoke`, `/insights`, `/footer`,
`/voice`, `/verbose`, `/codex-runtime`, `/permissions`, `/platform`,
`/start`, `/sessions`-as-browser. Most are channel-appropriate
(approvals via inline keyboard — Telegram real at `telegram.rs:1244-1271`;
`/grant`+`/revoke` correctly defer to Holon now, `gateway_commands.rs:2216`).
No action needed except the F1/F2 members.

### F8 — Registry hygiene [verified]

`COMMAND_REGISTRY` carries a duplicate `session` entry
(`gateway_commands.rs:171-183` real employee-binding version; `:288-295` a
dead "Show session info" stub with alias `status` that can never match —
`resolve_command` finds the FIRST match). Harmless but confusing; delete the
second.

## Priority ranking (for the fix plan)

1. **F1 metadata-theater commands** — highest operator-harm: replies claim
   success; nothing happens. Ten commands, one shared mechanism (runtime
   handles on `CommandContext`).
2. **F5 `/pause` + `/resume` from channels** — small, self-contained, closes
   a genuine operational dead-end (paused agent unreachable from every
   channel).
3. **F2 `/title` + `/branch`** — session-store API gap; small surface.
4. **F6 functional-core channel twins** — the actual "full scale of
   operational functionality" work; largest scope, needs the F1 mechanism
   anyway.
5. **F8 registry hygiene** — one-line deletion.
6. **F4 Slack Socket Mode (RT-86)** — known, tracked separately; unlocks the
   other four platforms' command surface only after their inbound paths
   exist.

## Method notes

Every "verified" above means: grep across all crates for the metadata key or
symbol found no consumer outside the writing handler, with the exact file:line
named. No claims rest on doc text. The AGENTS.md fleet rules were followed
(never touching peer files; the working tree already carried dirty
`Cargo.lock`, `STATE.md`, `feed.md` — untouched, left as found).
