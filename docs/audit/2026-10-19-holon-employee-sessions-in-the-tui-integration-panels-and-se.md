# Holon employee sessions in the TUI: integration, panels, and session outline

Dated-frozen: audited `2026-10-19`; findings mark verified vs asserted.

## Scope

Questions:
1. How Holon (agentic-os) is wired into the TUI options and panels, and what is operational.
2. How a session is built for an employee agent.
3. How session history carries employee and run tags, and what filters it offers.
4. What parallel agent management, engagement, and workflow surfaces exist.

Basis: working tree at `main` `313ec31a`, 2026-10-19. Three files were modified by another writer in the same tree during this audit (`cmd_backup.rs`, `cmd_service.rs`, `gateway_runner.rs`); the `gateway_runner.rs` line ranges below were read from that working tree. No code was changed and no tests were run.

Tags: **[verified]** = read in code or confirmed by a command in this audit. **[closure]** = taken from `docs/agentic-os/AUDIT-2026-10-15.md` §7–§9, not re-run here. **[asserted]** = inferred or from plan text, not checked.

## Findings

### A. TUI options and panels

- **[verified]** No TUI panel reads Holon. A search of `crates/operant-cli/src/tui` (excluding `vendor/` and tests) for `operant_os`, `HolonApi`, and `holon` finds one hit: the `/org` registry entry at `crates/operant-cli/src/tui/operant_app/app.rs` (`REGISTERED_COMMANDS`, "Operant org views (budget, holon)").
- **[verified]** `/org` reaches the CLI only by shelling out. `crates/operant-cli/src/tui/app/commands.rs:1508-1509` routes `org` to `shell_out_to_subcommand`, which suspends the TUI and runs `operant org …` in the terminal.
- **[closure]** TUI panels are still unbuilt. AUDIT-2026-10-15 §8 and §9 list `O-31 (TUI panels — the TUI wave)` as open. `BOUNDARY.md` §3.8 names TUI panels as the target, not the current state.
- **[verified]** The `operant org holon` surface (`crates/operant-cli/src/cmd_org.rs:194`, enum `HolonAction`) has these subcommands: `roster`, `board`, `approvals`, `messages`, `triggers`, `overview` (read views; the first five take `--json`), `supervise`, `tick`, `archive`, `notify`. The read views take no filter flags. `supervise` takes `--employee` and `--stage`, but those scope which stage a body claims, not what a view shows.
- **[verified]** `/kanban` is the personal board. Org work is not shown there, and `ORG-LAYER-DISPOSITION.md` §2 retires the org kanban.
- **[verified]** `/tasks` is an alias of `/agents` in the CLI (`crates/operant-cli/src/commands.rs:227`) and in the TUI (`crates/operant-cli/src/tui/app/commands.rs:340`). The TUI registry describes `/tasks` as "Show the autonomous task ledger", which is stale.

### B. Parallel agent surfaces (live in the TUI)

- **[verified]** `/agents` menu. The live `agent_status` vector is pushed when `delegate_task` or `spawn_subagent` starts and updated on completion (`crates/operant-cli/src/tui/app/agent_events.rs:157-160`, `:196`, `:263`). `open_agents_menu` (`app/commands.rs:1869-1885`) maps status strings to Running, WaitingForTool, Complete, Failed, or Idle. Its definitions come from `.operant/agents/*.md`. `AgentInfo` is `{name, status}` (`agents_view.rs:58-62`): it has no employee, department, or run field.
- **[verified]** Background-task rows. `crates/operant-cli/src/tui/render/mod.rs:96-97` refreshes `BackgroundTaskRegistry` from `async_delegation` records on every frame. Only `delegate_task(background = true)` produces a row (`background_tasks.rs` header comment).
- **[verified]** Swarm gallery is dormant. `inline_swarm_gallery_active` returns `false` and `inline_swarm_members` returns `Vec::new()` (`operant_app/tui_state.rs:378-384`). `SwarmInfo` (`info_widget/mod.rs:280`) has no live constructor. The jcode-ported gallery has no producer in the running app.
- **[closure]** Holon's parallelism lives in the operant daemon, not the TUI. `[holon] max_concurrent_runs = 2` in `operant.example.toml` bounds bodies. The gateway spawns bodies as tasks that hold permits (AUDIT-2026-10-16 §8, iter-855). The CLI `supervise` pass runs employees sequentially (`cmd_org.rs`, "sequential pass").

### C. Session construction: chat-bound employee

Per inbound message, `crates/operant-cli/src/gateway_runner.rs`:

1. **[verified]** Route to a session by origin triple, `store.entry_for_source(platform, user_id, channel_id)` (~`:1215-1300`).
2. **[verified]** Apply the employee binding. `gateway_sessions.employee_id` (`operant-core/src/gateway_session.rs:403`) decides the seat. With an id, the turn calls `agent.set_seat_id(emp)` and sets memory scope `Seat{identity}` (bank `emp:<id>`). Without one, scope is `Personal`. The charter is set to `None`: job descriptions are Holon's.
3. **[verified]** Record the inbound DM into the seat's context store (`record_inbound_dm`, same range).
4. **[verified]** On reload (restart, `/resume`, session switch), prepend `<continuation_summary>` from `valid_summary`, TTL-guarded (~`:1300-1345`). A live conversation is not given the summary.
5. **[verified]** Budget gate. A per-seat override over `[bylaws].budget`. A hard cap refuses the turn at start; a soft cap injects `<budget_state>` (~`:1346-1430`).
6. **[verified]** Context injection. `render_section(employee_id, charter, query, exclude)` adds channel-sourced context: DM and feed (~`:1440-1460`).
7. **[verified]** The turn runs under `set_session_id`.

Binding rules:
- **[verified]** Pre-existing rows are backfilled to `premiere` by the schema migration (`gateway_session.rs:415-432`; `gateway_runner.rs` comment near `:1221`).
- **[verified]** Chat `/session <emp>` binds any id. The code says "no local registry to validate against" (`gateway_commands.rs:~815-830`).
- **[asserted]** A mistyped id therefore binds an unknown seat. Holon resolves the id only when a turn runs. Not exercised.

### D. Session construction: Holon-run employee body

1. **[verified]** The supervisor claims with `holon --json dispatch --as <emp> --prepare` (`crates/operant-os/src/run.rs:55-95`). It gets a `Prepared` value: `run`, `stage`, `task`, `employee`, `driver`, `command`, `cwd`, `env`, `timeout_s`, `brief_md`, `result_path`, `policy_json`, `lease_until` (`run.rs:23-41`).
2. **[verified]** `run_body` (`crates/operant-os/src/supervisor.rs:41-217`) spawns `sh -c <command>` with `env_clear`, the OS baseline (`PATH`, `HOME`, `LANG`, `LC_*`, `TERM`, `TMPDIR`, `XDG_RUNTIME_DIR`), and the prepared env (`HOLON_RUN_TOKEN`, `HOLON_RUN_DIR`, `HOLON_BRIEF_DIR`, `HOLON_EMPLOYEE_ID`). It runs in its own process group and tees stdout and stderr to `driver.log`.
3. **[verified]** Lease. Heartbeat every lease/3 (floor 10 s). `run show` is polled every 15 s. A body that already submitted is stopped. A timeout kills the group and reports exit 137.
4. **[verified]** Driver form, from `docs/agentic-os/IMPLEMENTATION-PLAN.md:102`: `operant -c {posture} run --no-stream -s {identity} -q {briefing}`. `operant run --help` confirms `-s` is `--system` and `-q` is `--query`, so the identity is the system prompt and the stage brief is the query. `run` has no session flag, so a body is one headless invocation, not a resumed session. **[asserted]** The installer that renders this line lives in Holon's repository and was not checked here.
5. **[verified]** Brief is the only org guidance. `load_context_file_prompt` returns `""` when `is_holon_run()` (`crates/operant-core/src/agent/prompting.rs:14-24`).
6. **[verified]** Native tools. The 23 `holon_*` tools are generated from `crates/operant-os/vendor/holon-tools.json`. They register when the run env is set and are refused outside a Holon run (`crates/operant-os/src/tools.rs:158-171`).
7. **[closure]** Enforcement. `run_policy()` reads `policy.json` (`operant-os/src/enforcement.rs:35-41`). Posture `observe` maps to lockdown; `act` maps to yolo with the document's deny list. The roots, writes, and commands fences and the budget cap are closed under W3 (AUDIT §7, §8 O-18 and O-19). A Holon run opens no memory provider (AUDIT §9, R-08).
8. **[closure]** Finish. The result is validated against the vendored schema before `holon run finish` (`supervisor.rs`, finish path). `usage.json` and `trajectory.json` are written (AUDIT §8, O-19).
9. **[verified]** The history of a body is the run folder (`BRIEF.md`, `policy.json`, `driver.log`, `trajectory.json`, `usage.json`, `result.json`) plus Holon's `runs` and `events`. It is not the `~/.operant` sessions table.
10. **[asserted]** Whether a headless `operant run` writes a `sessions` row at all is not established here.

### E. Session history: tags and filters

- **[verified]** The `session_tags` table and index exist (`operant-core/src/database.rs:499-516`). The API is `add_session_tag`, `remove_session_tag`, `get_session_tags`, and `find_sessions_by_tag` (`database.rs:1388-1430`). `enrich_session_tags` fills `SessionSummary.tags` on list and search paths (`database.rs:1954-1968`).
- **[verified]** No live caller writes or reads tags. Every caller is in `database.rs` tests. No TUI or CLI surface prints tags. `cmd_sessions.rs` has no tag output.
- **[verified]** The TUI session browser (`/session`, `/resume`, `/sessions`) fills `SessionEntry {id, title, last_updated, message_count, cost_usd}` (`crates/operant-cli/src/tui/session_browser.rs:29-37`). The source is `history::list_sessions()` → `Database::list_sessions(50)` on the `sessions` table (`crates/operant-cli/src/tui/app/mod.rs:~1127-1160`; `adapter_types/history.rs:16-33`). It shows no tags, employee, platform, or origin. Its rename mode is the only mode. A comment at `session_browser.rs:225` says "this surface has no filter".
- **[verified]** `gateway_sessions` is the only table with `employee_id`, `platform`, `chat_type`, `origin`, and `display_name` (`gateway_session.rs:335-403`). Only chat `/session` shows the binding ("This session runs as `<emp>`", `gateway_commands.rs:~806`).
- **[verified]** `operant sessions` (`crates/operant-cli/src/cmd_sessions.rs:17-80`) has `list`, `show`, `delete`, `stats`, `export`, `prune`, `rename`, `browse`, `search` (FTS5 over message content), and `recap`. None takes a tag, employee, or platform filter.
- **[verified]** The only filter over session content is the rewind selector: a case-insensitive substring over prompt texts, opened with `f` (`crates/operant-cli/src/tui/rewind_selector.rs:36-38`, `:101-108`, `:129-149`).

### F. Gaps against "operationalize effectively"

1. **[verified]** No TUI org panel (O-31). The owner sees Holon through the CLI and the `/org` shell-out only.
2. **[verified]** A session row carries no employee, department, run, or stage in the browser, and cannot be filtered by any of them.
3. **[verified]** The tag store is dormant: the schema, API, and summary enrichment exist, but no live code writes a tag.
4. **[verified]** An employee body's history is in run folders, not the session browser.
5. **[verified]** The parallel view covers in-turn delegations (`/agents`) and background delegations (rows). Neither carries an employee identity. The swarm gallery has no producer.
6. **[verified code, asserted effect]** `enforcement.rs:57` says Holon-run databases live under `HERMES_HOME`. `platform.rs:96-99` resolves `HERMES_HOME` first. The TUI list reads the same resolver. Whether the owner's TUI sees body sessions therefore depends on the environment at TUI launch. Not run-tested.
7. **[verified]** The `/tasks` registry description is stale (see A).
8. **[asserted]** Chat `/session <emp>` accepts unknown ids (see C).

## Outline: session construction and session history (proposal, not built)

This is a proposal. Nothing below is implemented.

### 1. Session identity contract

Every session has tags in fixed namespaces:

| Namespace | Example | Source of truth today | Written at |
|---|---|---|---|
| `emp:` | `emp:rowan-vale` | `gateway_sessions.employee_id` (chat); `HOLON_EMPLOYEE_ID` (body) | chat: derive at read time; body: at `prepare` |
| `dept:` | `dept:platform` | Holon employee record | at `prepare` |
| `run:` | `run:run_0412` | `Prepared.run` | at `prepare` |
| `stage:` | `stage:execute` | `Prepared.stage` | at `prepare` |
| `origin:` | `origin:chat`, `origin:run`, `origin:cron` | session path | at creation |
| `platform:` | `platform:telegram` | `gateway_sessions.platform` | derive at read time |

Chat tags are derived, not stored. The `designation` rule in `ORG-AUTHORITY-ARCHITECTURE.md` §1 says the same about rendered identity. Body tags are written once, at claim, into the existing `session_tags` table. Its `INSERT OR IGNORE` already gives idempotent writes.

### 2. Session browser

- Add `tags: Vec<String>` to `SessionEntry`. Show the `emp:` and `stage:` tags as columns.
- Add a filter that reuses the rewind selector's pattern (`f` opens it). Tokens are `key:value`. Tokens AND together. Each token is resolved through `find_sessions_by_tag`.
- Gate the source change. Today's `list_sessions` reads only the `sessions` table. Chat sessions live in `gateway_sessions`, so the browser needs one more source before it can list them with their tags.

### 3. CLI parity

- `operant sessions list --tag emp:<id>` (uses the same tag API).
- `operant org holon runs --employee <id> --stage <s>` as a view over Holon's read API. This depends on read API v4 (R-03 and R-04 in `AUDIT-2026-10-16-capability-layer.md`), which the kernel has not yet published.

### 4. Parallel management panel

- Do not revive the jcode swarm gallery. It has no producer, and its feed type (`SwarmMemberStatus`) has no employee field.
- Extend the `/agents` menu with one org tab. Its rows come from the read API (`board`, `approvals`, `messages`). Tab rows carry the same `emp:` label as the session browser. The TUI reads the API in-process through `HolonApi`, as `BOUNDARY.md` §3.2 allows, instead of shelling out.

### 5. Operator workflow (works today, no code)

- Overview and queues: `operant org holon overview`, `board`, `approvals`, `messages`.
- Decisions: approve or reject via the Chair channel (`plan approve` or `plan reject`, routed by the decision-verb router).
- Supervision: `operant org holon supervise --employee <id> --loop`.
- Chat binding: `/session` shows the binding; `/session <id>` sets it.

### Decisions the owner needs to make

- **D1.** Do body runs write a `sessions` row? If not, body history stays in run folders, and the browser shows only chat sessions.
- **D2.** Does the TUI org tab read `HolonApi` in-process (recommended: no spawn, same as the boundary's read path) or shell out to `operant org holon --json` (what `/org` does)?
- **D3.** Which `HERMES_HOME` does the owner's TUI launch under? This decides whether body sessions are visible in the browser at all.

### Verify before relying on this outline

- **[asserted]** The installer's driver form (Holon repository).
- **[asserted]** Whether `operant run` writes a `sessions` row.
- **[asserted]** Chat `/session <emp>` in org mode: whether an unknown id fails at the first turn.
- **[asserted]** Body session database routing under `HERMES_HOME`.

## Verification

- No code changed. No tests run.
- Commands run: `rg` over `crates/operant-cli/src/tui`, `crates/operant-cli/src`, `crates/operant-core/src`, and `crates/operant-os/src`; `operant run --help`; `./scripts/genome audit`; `./scripts/genome lint` (0 errors).
