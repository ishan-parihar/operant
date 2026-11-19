# Hydrogen architecture review, round 2 (2026-11-19) — bottlenecks, redundancies, fragmentation, integration gaps

A fresh-context review round (Gemini 4 Hydrogen, `gemini-4-hydrogen`, effort `xhigh`)
over a **100,723 B context pack** cut from `AGENTS.md` slices, `docs/handoff/STATE.md`,
`docs/OUTLINE.md` and the mechanism docs for the surfaces round-1 did not cover
(tools, skills, plugins, autonomous, tui, org, security) plus a measured-surface delta.

Round-1 (`docs/audit/2026-11-19-hydrogen-architecture-review-12-findings-verified.md`)
covered agent-loop, gateway, memory, context and config. **This round is a delta round**:
it asks about the surfaces round-1 never opened, and its delta section carries only what
landed since (`df1ea4c0..origin/main`, four commits) plus a re-derivable measurement.

**Every finding below was re-verified against the live tree before adoption.** The
reviewer worked from documentation, so each carries a CONFIRMED / CONFIRMED-NARROWER /
REFUTED verdict with `file:line` evidence. Where the reviewer's own inference was wrong,
that is recorded rather than quietly dropped.

Round artefacts: `research/audits/context-pack-2026-11-19-round2-arch-bottlenecks/`
(`pack.spec`, `PACK-AS-SENT.md`, `REVIEWER-PROMPT.md`, `RESPONSE.md`, `DISPATCH.json`,
`measure_surface.py` + `measure2.py` + `measure3.py`).

## The round's parameters, as recorded

| fact | value |
|---|---|
| venue | AI Studio (`aistudio.google.com`), fresh chat, model pinned in URL |
| model / effort | `gemini-4-hydrogen` / `xhigh` (confirmed by `doctor` and by `send`) |
| pack size | 100,723 B (cap raised 90k → 110k deliberately; ~28k tokens of a 1M shared window) |
| transport | inline, sentinel last (the venue has no pack-file attach path) |
| reply | 32,303 chars, DOM scrape (`DISPATCH.json` records this; the three-dots copy was not used) |
| response provenance | `fetched_at 2026-11-19T19:15:02`, page 239 |

## Verification method

Each finding was checked against `origin/main` with targeted greps. The reviewer's own
proposed falsifier was run first; a finding is **CONFIRMED** only when the falsifier fails
against the tree. Where the reviewer's reasoning overstated the impact, the finding is
marked **CONFIRMED (narrower than claimed)** and the correct scope is stated — an
overstated finding adopted unqualified is a defect the next agent inherits.

## Findings

### F1 — the `[cli]` CLI-admission surface has a full producer and no agent consumer — CONFIRMED (cross-crate, Severity 3)

`operant context cli list|add|remove|toggle` (`crates/operant-cli/src/cmd_context.rs`),
`tools/cli_roster.rs`, `tools/cli_help_parser.rs`, `tools/cli_discovery.rs`,
`cli_roster.toml`, `[cli.tools]` and `[cli] max_help_lines` all exist. But the
agent-facing half does not: `rg 'cli_exec|cli_reference'` over `crates/` returns **only
comments and doc text** — `config.rs:1687-1688` ("the agent-facing half is still not
wired: `cli_exec` and the `<cli_reference>` prompt injection are the plan's Phase C.
Nothing in ..."), `config.rs:1702`, `cmd_context.rs:1488`. No `cli_exec` tool is
registered, and no `<cli_reference>` block is injected into any prompt.

So an operator can run admission scans, and the grants are recorded and costed but
**not invocable**. `[cli] max_help_lines` is parsed and never applied (RT-278).
Falsifier: `rg -n "cli_exec|<cli_reference>|max_help_lines" crates/` — the only hits are
prose. **The fix needs a product/security decision**, not a patch: either define how
`cli_exec` enforces `--allow-subcommands`/`--global-args` against `ApprovalGuard` without
shell/flag-injection bypasses, or hide `operant context cli` and `[cli]` until Phase C
ships.

### F2 — two interactive human-question tools with incompatible host plumbing — CONFIRMED (cross-crate, Severity 3)

Two tools target the same TUI dialog by different mechanisms:

- `clarify` (`tools/clarify_tool.rs` over `user_question.rs`, unconditional builtin)
  pushes onto a **process-global** `static USER_QUESTION_TX: OnceLock<...>`
  (`user_question.rs:52`).
- `ui_request_input` (`tools/ui_tool.rs`, gated by `[ui] generative_enabled`) uses a
  **per-host** request channel resolved via `agent.transport`, drained by both the TUI and
  the gateway, and guarded by `crates/operant-core/tests/surface_host_receivers.rs`.

Measured: `rg 'USER_QUESTION_TX' crates/operant-cli/src/gateway_runner.rs` returns
**nothing** — the gateway never drains it.

**The reviewer's inference was wrong about the failure mode, and the correction matters.**
It claimed "any non-draining host risks a closed-channel error or a 300 s turn stall".
Reading `user_question.rs:83-98`: when the `OnceLock` is unset (no drainer registered)
`try_send_user_question` returns `None`, and the caller "should fall back to returning the
question as a tool result". So there is **no stall** — the tool degrades to a plain
result. The real defect is narrower and still real: `clarify`'s `session_id` field rides
a process-global channel, so in a multi-chat gateway process **the questions are not
isolated by chat**, and the `has_user_question_sender()` gate means a gateway host that
never registers a sender silently loses the interactive path for every chat.

Fix: route `clarify` through the same per-host/per-transport receiver as
`ui_request_input`, removing the global `OnceLock`.

### F3 — `builtin_tool_names()` disagrees with the registered name for the notify tool — CONFIRMED (narrower than claimed, Severity 3)

The reviewer claimed three consumers use `"notification"` while the tool registers
`"notify"`. Measured:

- `tools/builtin.rs:354` lists `"notification"`.
- `tools/notification_tool.rs:26,40,49,63` registers **`"notify"`**.
- `agent/evidence/classify.rs:107-109` lists **`"notify"` AND `"notification"`** — a
  deliberate phantom alias ("The phantom aliases are kept so an older name still
  classifies"). So the classifier is **not** broken; the reviewer's third consumer is
  wrong.
- `crates/operant-cli/src/gateway_runner.rs` has **no** `"notification"` icon entry
  (the reviewer's TUI-icon claim does not hold at that path).

The real defect: `builtin_tool_names()` is a hand-maintained list, and
`tools/tool_state.rs::disabled_tool_names()` returns a plain `HashSet<String>` with **no
validation** — so `operant tools disable notification` writes a name that matches nothing,
while the live tool is `notify`. The count-based guard
(`builtin.rs:471`: `count + 4 == builtin_tool_names().len() + 3`) catches only a change
in the *number* of names, never a rename.

Fix: generate `builtin_tool_names()` from the registered descriptors, add a `"notify"`
alias in `ToolRegistry` + `tool_state` so existing `tool_state.json` files keep working,
and make `test_register_all_builtin_tools` assert **set equality** rather than a count.

### F4 — slash commands duplicated across three operant-cli surfaces — CONFIRMED (cross-crate, Severity 3)

`/effort`, `/plan`, `/context`, `/export`, `/memory`, `/skill`, `/search` and `/ponytail`
have live implementations in `gateway_runner.rs` (10,461 LOC) and a **second** copy in
`gateway_commands.rs` (5,553 LOC) that returns fallback strings when no live agent is
attached, while the TUI carries its own handlers in `tui/app/commands.rs` — inside a TUI
tree measured at 193,996 LOC / 351 files. `/cron`, `/kanban`, `/curator`, `/org` stay
CLI-only shell-outs on both sides.

The cost is felt whenever a command or per-chat control is added to one host and must be
rebuilt across the others (including Telegram's command-menu collision filtering, which
gives built-ins priority over vendor skills of the same name).

Fix: an `AgentCommandRouter` in `operant-core` returning structured `CommandResult`
values, leaving `tui/app` and `gateway_runner.rs` as thin renderers, and deleting the
duplicates in `gateway_commands.rs`.

### F5 — configuration split across four places — CONFIRMED (narrower than claimed, Severity 2)

Measured: `operant-config` 1,854 LOC / 11 files; `operant-core/src/config.rs` 7,319 LOC
plus `config_registry.rs` 4,186 LOC (11,505 LOC of config in core = **6.2x** the crate
that declares the schema); `operant-cli/src/config.rs`; plus a TUI config shim. Three
non-unified surfaces: disk TOML + `operant config set`; the in-loop `config_manage` tool;
and the TUI `/config`, which exposes visual settings only.

**The reviewer's "cross-chat state leak" claim is CONFIRMED and is the sharpest part of
this finding.** `config_tool.rs` calls `set_config_value` → `install_runtime_config`,
which writes a **process-global** `static RUNTIME_CONFIG: OnceLock<RwLock<AppConfig>>`
(`config.rs:41`). So one chat's `config_manage set` is visible to every chat in the
gateway process. The non-persistence is *documented by design*
(`config_tool.rs:21-22`), so this is not a bug in the tool — it is an architectural
consequence of a process-global runtime config serving multi-tenant chats.

Smallest unifying move: `docs/OUTLINE.md` shows `CORE --> CFG` and `CLI --> CFG`
already exist, so `AppConfig`, `BehaviorSettings`, `ToolSettings` and `config_registry`
can move down into `operant-config` with **no new crate edge**. The product decision it
needs: should `config_manage` be session-scoped (like `/ponytail` is per-chat) or
disabled in multi-chat gateway mode?

### F6 — repeated runtime costs that could be paid once — CONFIRMED (narrower than claimed, Severity 2)

Three real re-pays, verified:

- `SkillManager::is_available` (`skills.rs:989`) runs `which::which(cmd)` for every
  `prerequisites_commands` entry, uncached. **Narrower than claimed**: callers are CLI
  list paths (`cmd_skills.rs:1238`, `cmd_context.rs:881`) and one gateway path
  (`gateway_runner.rs:1997`), not every render.
- `find_skill_in_roots` (`skills.rs:129`) walks roots on disk per lookup and bypasses
  `SkillManager`'s in-memory index. Confirmed.
- `autonomous.rs:238` calls `create_agent_without_events` **inside the 300 s tick**,
  rebuilding the agent (and its MCP manager) instead of resetting only conversation
  state. Confirmed — this is the highest-value item in this finding, because re-spawning
  MCP subprocesses and re-handshaking `tools/list` every 300 s is a per-tick cost that
  runs forever.

### F7 — two live subsystems stop at their cross-surface boundary — CONFIRMED (cross-crate, Severity 2)

- **RT-190** (dual memory stores, embedded `MemoryWireProvider` vs `memory-wire serve`
  daemon) — no bridge. Needs an **owner ruling**; this session already asked it (Q6's
  sibling rows) and the decision is the operator's.
- **Plugin hot-reload half-wires `SkillManager`**: `watcher.rs` → `apply_plugin_changes`
  live-reloads plugin *tools* and *observers*, but `SkillManager` is a boot-scanned owned
  struct, so newly hot-loaded plugin skills stay invisible until restart or
  `/reload-skills` (`plugins.md:84-86` names the fix: `Arc<RwLock<SkillManager>>`).
- The reviewer's claim that "the canonical vendor skill-root list lives in
  `operant_cli::skill_roots::vendor_skill_roots`" is **half right and worth correcting**:
  there are **two** functions with that name. `operant-core/src/skills.rs:113` defines
  `vendor_skill_roots(host_roots)`; `operant-cli/src/skill_roots.rs:23` defines
  `vendor_skill_roots(config)` as a thin adapter over the core one. It is a same-named
  wrapper, not a duplicate implementation — so this is **naming fragmentation**, not
  divergent logic.

### F8 — declared surfaces with no reachable consumer — CONFIRMED (cross-crate, Severity 1)

`PluginCapability::Channel`: `host.rs:362 channel_plugins()` has **only test callers**
(`host.rs:727/918/953`); no production path registers a dynamic gateway adapter, while
gateway adapters are a compile-time 7-platform registry. `[pk]` and `[socialization]`
are parsed, reported by `unused_setting_warnings`, and read by nothing (RT-136, carried
as a record). Plugin `manifest.toml [config]` is forwarded to observe envelopes only;
the tool and memory bridges drop it with no validation.

### F9 — terminal truncation detected by string-sniffing two markers — CONFIRMED (local, Severity 2)

`tools/terminal_tool.rs:28-33` declares
`const TRUNCATION_MARKERS: [&str; 2] = ["[output truncated]", "...[truncated]"]` and the
header states the local backend writes the first while the docker and ssh backends write
the second. `tools/terminal_backend.rs:23 struct CommandOutput` has **no `truncated:
bool`** field (verified: `grep 'truncated'` in that file returns nothing). So a backend
that changes its marker silently disables truncation detection.

Second half confirmed: with `aft_enabled = true`, basic file/terminal tools are
auto-disabled, but the shadow-git checkpoint tool (`checkpoint_tool.rs`, whose
store-drift bug this session fixed as RT-612 / iter-2673) and `patch_tool.rs` stay
enabled alongside `aft_checkpoint`/`aft_restore_checkpoint`/`aft_undo` and
`aft_edit`/`aft_apply_patch` — two checkpoint stores are exposed to the model.

Fix: add `truncated: bool` to `CommandOutput`; decide whether `aft_enabled` should also
suppress `patch` and the shadow-git checkpoint, or unify `operant checkpoints` with AFT
checkpoints.

### F10 (Q4, open to outside sources) — client-side tool re-injection busts the prompt cache — CONFIRMED

The reviewer's citation was **verified at the source**, not taken on trust:
`https://www.anthropic.com/engineering/advanced-tool-use` (read 2026-11-19) says tools
are marked `defer_loading: true`, that deferred tools "aren't loaded into Claude's
context initially", and — the load-bearing sentence — "**Tool Search Tool doesn't break
prompt caching** because deferred tools are excluded from the initial prompt entirely."

Measured against operant: `rg 'defer_loading' crates/operant-core/src/agent/clients/`
returns **nothing** — no provider passes native `defer_loading`. Instead
`tools/tool_search/deferred.rs:128 materialise_for_turn(turn, catalog)` selects tools by
turn keyword overlap and `:150` re-injects them into the model-visible array for that
request. Combined with `ponytail inject="always_on"` (which AGENTS.md says mutates the
frozen prefix on level/install changes), the tool array is not byte-stable across turns.

This is a real structural divergence from current practice, and it is the finding with
the clearest measurable cost: re-prefills across long sessions.

### F11 (Q4) — stateful legacy MCP + bespoke UI IR vs MCP 2026-07-28 — CONFIRMED (with the citation verified)

`https://modelcontextprotocol.io/specification/2026-07-28` (read 2026-11-19) states
"**Stateless, self-contained requests**", "**Per-request capability negotiation**", and
lists "**MCP Apps**: Interactive UI elements (charts, forms, video players) rendered
inline within conversations" as a notable extension.

Measured: `operant-core/src/mcp.rs` (4,777 LOC) has 148 SSE/Sse references, and
`tools/code_execution.rs:6` says "This is NOT a sandbox: code runs with the operant
process's own ...". So the gap is real: operant keeps stateful per-tick MCP connections
(autonomous.md:28) and a bespoke Surface IR (`operant-core/src/ui`, 2,892 LOC)
disconnected from MCP Apps.

### F12 (Q4) — four orchestration layers with ephemeral swarm state — CONFIRMED (cross-crate, Severity 2)

Measured: `delegate_task` (`tools/sub_agent_tool.rs` 4,175 LOC + `foreground_swarm.rs`,
whose live state "dies with the process"), `autonomous.rs` (1,852 LOC) polling `TODO.md`
every 300 s, `kanban/` (2,186 LOC) + `cronjobs/` (5,284 LOC), and `operant-os` (2,829 LOC)
+ `operant-core/org` (3,965 LOC) driving Holon. Four substrates for one concept:
"get work done".

### The reviewer's own LOC critique — ACCEPTED, and it corrects my number

The reviewer noticed the pack's `528,878` figure counts inline `tests.rs` files as
authored source. **It is right, and this is a correction to my measurement, not to the
reviewer's.** Measured: 26 inline `tests.rs` / `*_tests.rs` files inside `src/` total
**47,795 LOC** (the largest: `tui/app/tests.rs` 19,286). The honest production figure is
therefore **≈481,000 LOC** of authored src, not 528,878. The measurement scripts kept in
the round directory can be re-run with that exclusion to confirm.

## Tally

| id | class | severity | locality | verdict |
|----|-------|----------|----------|---------|
| F1 | integration gap | 3 | cross-crate | CONFIRMED (dead surface, decision-gated) |
| F2 | integration gap + redundancy | 3 | cross-crate | CONFIRMED (stall inference REFUTED; isolation is the defect) |
| F3 | fragmentation | 3 | cross-crate | CONFIRMED, narrower (2 of 3 consumers wrong) |
| F4 | redundancy + fragmentation | 3 | cross-crate | CONFIRMED |
| F5 | fragmentation | 2 | cross-crate | CONFIRMED (process-global runtime config proven) |
| F6 | bottleneck | 2 | cross-crate | CONFIRMED, narrower (not per-render) |
| F7 | integration gap | 2 | cross-crate | CONFIRMED (skill-root claim half-wrong: wrapper not duplicate) |
| F8 | redundancy | 1 | cross-crate | CONFIRMED |
| F9 | integration gap | 2 | local | CONFIRMED |
| F10 | bottleneck | 3 | cross-crate | CONFIRMED + citation verified at source |
| F11 | redundancy + integration gap | 2 | cross-crate | CONFIRMED + citation verified at source |
| F12 | fragmentation + redundancy | 2 | cross-crate | CONFIRMED |

## What the project should do first (reviewer's ordering, re-ranked by measured cost)

1. **F3** — the `notification`/`notify` split is an operator-visible bug (`operant tools
   disable notification` silently does nothing) with a small fix.
2. **F6** — keep `McpManager` warm across `autonomous.rs` ticks. It is the only finding
   that costs something on every tick forever.
3. **F10** — `materialise_for_turn` mutating the tool array per turn is the clearest
   measurable cache cost.
4. **F1 + F8** — the dead `[cli]` surface and the unread config tables: decide (ship
   `cli_exec`, or hide the surface) rather than carrying both.
5. **F4 + F5** — the command-router and config consolidation, both of which need a
   product decision about multi-chat session scope.

## What I could not assess (pack gaps, stated honestly)

The reviewer named these and they are correct: `agent/mod.rs` (6,370 LOC), `agent/run.rs`
(5,769 LOC), `agent/stream.rs`, `client.rs` (4,427 LOC), `database.rs` (7,517 LOC) and
`docs/library/{loop,context,memory,mcp,persistence,sub-agents}.md` were **not in this
round's pack** — round-1 covered them, but this round deliberately swapped them out to fit
the byte cap. So per-turn allocation severity in `build_messages`,
`ToolRegistry::get_schemas()` caching, streaming backpressure and SQLite lock contention
remain unmeasured here. A round-3 delta should carry them.