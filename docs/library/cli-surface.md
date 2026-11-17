# The command surface

Owns: the operator's CLI — every `operant <command>`, what it does, its
most useful flag, and whether it runs in-process or shells out.
Not for: the mechanisms behind the commands (each library page owns its
own: [operability.md](operability.md) for doctor/setup/service,
[gateway.md](gateway.md) for channels, [skills.md](skills.md),
[org.md](org.md), [context-management.md](context-management.md)), the
slash commands typed INSIDE a session (that half is the last section), the
environment traps (`docs/RUNBOOK.md`).

Source: `crates/operant-cli/src/main.rs` (the `Commands` enum + dispatch),
`crates/operant-cli/src/cmd_*.rs` (the handlers),
`crates/operant-cli/src/commands.rs` (the slash-command registry),
`crates/operant-cli/src/gateway_api.rs`,
`crates/operant-cli/src/gateway_runner/allocator.rs`,
`crates/operant-cli/src/gateway_runner/enrichment.rs`,
`crates/operant-cli/src/cmd_channel.rs` (the `channel` subcommand:
list/start/doctor/add/remove/bind-telegram/send),
`crates/operant-cli/src/skill_roots.rs`, `prompt_helpers.rs`,
`hardening.rs`, `post_setup.rs`, `memory_provider_tools.rs`,
`crates/operant-cli/src/integrations/`,
`crates/operant-core/src/shell_hooks.rs` (the `operant hooks` runtime).

## The shape of the surface

`Cli` (`main.rs`) is a `clap::Parser` with 16 GLOBAL flags that apply to
every subcommand, derived from the fields above `command`: `--verbose`,
`--log-level`, `--config`, `--api-key` (env `OPENAI_API_KEY`),
`--base-url` (env `OPENAI_BASE_URL`), `--model`, `--max-iterations`,
`--tool-timeout`, `--request-timeout`, `--context-window`,
`--max-healing-attempts`, `--stream` / `--no-stream` (two flags, one
conflicting pair), `--debug-tui`, `--no-mouse`,
`--dangerously-skip-permissions` — plus the `Commands` enum.

### The count, and where it comes from

**`Commands` has 52 variants**, derived from the enum body in
`crates/operant-cli/src/main.rs` by parsing its top-level comma-separated
variants with the doc comments stripped. Each variant has exactly one
dispatch arm in `main()`; `main()` also has a `None` arm that launches
the TUI, so the binary has 52 named subcommands plus the default TUI
launch.

Some variants carry their own subcommand enum, so the LEAF count is much
higher than 52. Derived from the same files:

| Command | Its subcommands (count) |
|---|---|
| `config` | 10 (`show`, `set`, `path`, `check`, `edit`, `env-path`, `migrate`, `backups`, `describe`, `notes` — `backups` itself nests `list`/`restore`) |
| `sessions` | 12 (`list`, `show`, `delete`, `stats`, `export`, `prune`, `rename`, `browse`, `search`, `recap`, `resume`, `fix-titles`) |
| `mcp` | 7 (`list`, `add`, `remove`, `test`, `serve`, `login`, `configure`) |
| `gateway` | 15 (`run`, `status`, `sessions`, `channels`, `stats`, `webhook`, `hooks`, `pairing`, `start`, `stop`, `restart`, `install`, `uninstall`, `list`, `migrate-legacy`) |
| `cron` | 12 (`list`, `create`, `get`, `history`, `update`, `delete`, `pause`, `resume`, `run`, `status`, `tick`, `blueprint`) |
| `kanban` | 32 (`list`, `show`, `create`, `complete`, `block`, `comment`, `link`, `unlink`, `stats`, `init`, `assign`, `assignees`, `unblock`, `archive`, `tail`, `runs`, `log`, `notify`, `dispatch`, `gc`, `diagnostics`, `triage`, `reclaim`, `reassign`, `edit`, `claim`, `heartbeat`, `context`, `list-boards`, `create-board`, `delete-board`) |
| `channel` | 8 (`list`, `start`, `doctor`, `add`, `remove`, `bind-telegram`, `send`, `delete-message`) |
| `skills` | 17 (`list`, `search`, `inspect`, `install`, `uninstall`, `update`, `browse`, `check`, `audit`, `reset`, `publish`, `snapshot`, `tap`, `toggle`, `market`, `seed`) |
| `curator` | 12 (`status`, `run`, `pause`, `resume`, `pin`, `unpin`, `restore`, `list-archived`, `archive`, `prune`, `backup`, `rollback`) |
| `plugins` | 9 (`list`, `install`, `remove`, `enable`, `disable`, `toggle`, `new`, `validate`, `status`) |
| `architecture` | 3 (`validate`, `dump`, `pool-import`) |
| `memory` | 9 (`list`, `get`, `search`, `store`, `delete`, `stats`, `import`, `export`, `clear`) |
| `cookies` | 5 (`import`, `list`, `export`, `clear`, `discover`) |
| `context` | 11 (`status`, `inspect`, `recall`, `sessions`, `rollup`, `rollups`, `rollup-maintenance`, `window`, `budget`, `optimize`, `cli` — `cli` nests `list`/`add`/`remove`/`toggle`) |
| `org` | 2 families (`budget` → `set`/`list`/`clear`; `holon` → `roster`, `board`, `approvals`, `messages`, `triggers`, `overview`, `supervise`, `tick`, `archive`, `notify`) |
| `tui` | 7 (`debug`, `effort`, `mode`, `output-style`, `theme`, `vim`, `keybindings`) — `debug` nests 12 read-only panels |
| `logs` / `webhook` / `suggestions` / `checkpoints` / `acp` / `trajectory` / `debug` | 2 / 4 / 5 / 4 / 2 / 5 / 3 |

Two variants take no subcommand: `pause` (an optional reason string) and
`resume`. `login`, `logout`, `version`, `status`, `dump`, `insights`,
`setup`, `dashboard`, `service`, `sop`, `update`, `backup`, `import`,
`uninstall`, `completion`, `hooks`, `migrate`, `tools`, `model`, `auth`,
`profile`, `continue`, `test`, `run`, `autonomous`, `chat`, `doctor`,
`donate` are all direct.

## Startup order (what happens before any command runs)

`main()` (`crates/operant-cli/src/main.rs`) is explicit about its order
because two of its steps are unsafe to reorder:

1. `restore_default_sigpipe_disposition()`.
2. **`hardening::harden_process()`** — `RLIMIT_CORE = 0` and
   `PR_SET_DUMPABLE = 0`, BEFORE config, before the runtime, before any
   daemon. A refused step LOGS and continues rather than refusing to start
   (the codex divergence documented in `hardening.rs`).
3. `Cli::parse()`.
4. `CliConfig::load()` (the CLI layer: `.env` + `HERMES_*` overrides)
   BEFORE the tokio runtime exists, because it writes into the process
   environment.
5. `load_app_config(cli.config.as_deref())` — the one config load, so
   each load-time warning prints once.
6. `apply_startup_proxy`, the Herdr switch, env overrides, and
   `apply_cli_overrides` (the global `--api-key` / `--base-url` /
   `--model` stamp).
7. `install_runtime_config`, the first-run bundled-skill seed, the
   delegation statics, then `init_logging`.
8. Dispatch on `cli.command`.

## The commands, by purpose

### Agent turns

| Command | What it does | Most useful flag |
|---|---|---|
| `run` | One agent turn from a query and exit. | `--query` (or `--system` for the system prompt); `--autonomous` hands the turn to the TODO.md loop, `--moa` enables a Mixture-of-Agents turn, `--record-trajectory` saves ReAct steps under `~/.operant/trajectories/` |
| `chat` | Interactive session, or one prompt then exit. | `--attach <socket>` attaches to a RUNNING server's session socket instead of building a runtime (no registry, no DB, no MCP children, no AFT) |
| `autonomous` | The autonomous coding loop (`crates/operant-cli/src/autonomous.rs`). | `--system` |
| `continue` | Reopen the last session in the TUI. | `[id]` defaults to the interrupted-session marker, else the most recent — this is the exact line the TUI prints when it closes |
| `test` | Run one tool by name with JSON args, for smoke-testing a tool without a model. | `--args '{"…": …}'` |
| `donate` | Prints the repo / sponsors / UPI links. | none |

### Config, model and identity

| Command | What it does | Most useful flag |
|---|---|---|
| `config` | Read and write `operant.toml`. See the table below. | `describe` — every key, its default, its note, and whether the RUNNING value differs |
| `model` | `show` the active model, `set` a new one for this session. | `set <name>` is runtime-only, never written to a file |
| `auth` | Manage provider credentials: `list`, `add`, `remove`, `reset`, `status`. | `add <provider> <key>`; `status` shows which providers have credentials |
| `login` / `logout` | Provider OAuth / credential teardown. | none |
| `profile` | `list`, `show`, `create`, `use`, `delete`, `clone` the operant profiles. | `create <name> --clone <source>` |
| `setup` | The interactive first-run wizard (`cmd_setup.rs`), with `post_setup.rs` as the post-save menu. | `--non-interactive` for a report; plain `operant setup` is ALREADY quick mode, and `--reconfigure` is the flag that actually changes behaviour |
| `completion` | Generate a shell completion script. | `shell bash\|zsh\|fish` |

`config` in full (`cmd_config.rs`, `ConfigSubcommand`): `show`,
`set <key> <value>`, `path`, `check`, `edit` (opens `$EDITOR`, falls back
to `nano`), `env-path`, `migrate [--apply]`, `backups list|restore`,
`describe [section] [--json]`, `notes <key>`.

`describe` is generated from the SAME key registry the loader uses, not a
hand-written list, so it cannot drift from what the binary accepts. It
marks NON-DEFAULT rows with `*`; a wall of 293 rows in which the
operator's own three settings are invisible would not be a map. `notes`
FAILS with the closest key matches when a key is unknown, rather than
printing a blank.

### Sessions and history

| Command | What it does | Most useful flag |
|---|---|---|
| `sessions` | `list`, `show`, `delete`, `stats`, `export`, `prune`, `rename`, `browse`, `search`, `recap`, `resume`, `fix-titles`. | `--json` on the parent; `list --interrupted` filters to sessions an exit marker flagged; `prune --older-than-days N` (default 30) |
| `insights` | Session statistics over the last N days. | `--days N` (default 7) |
| `trajectory` | `list`, `show`, `export`, `delete`, `clear` the ReAct recordings. | `export --format ndjson` for a fine-tuning corpus |
| `checkpoints` | `status`, `list`, `prune`, `clear` the shadow-git snapshots. | `prune --keep N` (default 10) |
| `memory` | `list`, `get`, `search`, `store`, `delete`, `stats`, `import`, `export`, `clear` long-term memory. | `store "<text>"`; `search "<query>"` |

### Skills, MCP, plugins, SOPs

| Command | What it does | Most useful flag |
|---|---|---|
| `skills` | 17 subcommands: `list`, `search`, `inspect`, `install`, `uninstall`, `update`, `browse`, `check`, `audit`, `reset`, `publish`, `snapshot`, `tap`, `toggle`, `market`, `seed`. | `list --tree` renders the meta-skill hierarchy; `install <path\|url>` runs the pre-install security scan and `--force` overrides its verdict |
| `mcp` | `list`, `add`, `remove`, `test`, `serve`, `login`, `configure` the MCP servers. | `serve` runs operant AS an MCP server over stdio (the symmetric half of the client story); `login` does OAuth 2.1 + PKCE |
| `plugins` | `list`, `install`, `remove`, `enable`, `disable`, `toggle`, `new`, `validate`, `status` the WASM plugins. | `new <name> --capability tool\|observer\|memory\|skill` scaffolds a project |
| `sop` | `list`, `validate`, `show` the operator's standard operating procedures. | `validate [name]` |
| `curator` | 12 subcommands over the skill curator. | `run --dry-run --consolidate` |

### Gateway and channels

| Command | What it does | Most useful flag |
|---|---|---|
| `gateway` | The daemon lifecycle and views: `run`, `status`, `sessions`, `channels`, `stats`, `webhook`, `hooks`, `pairing`, `start`, `stop`, `restart`, `install`, `uninstall`, `list`, `migrate-legacy`. | `run --socket <path>` also serves the session socket so a terminal client can attach to THIS daemon (RT-542); `status --deep` includes systemd state |
| `channel` | `list`, `start`, `doctor`, `add`, `remove`, `bind-telegram`, `send`, `delete-message`. | `add telegram --token <bot token>`; `--api-key` must NEVER be used for a channel token (it is the LLM key) and the handler restores the file's own values before persisting |
| `webhook` | `subscribe`, `list`, `remove`, `test` the webhook subscriptions. | `subscribe <url> --route <segment> --prompt <text> --events message,reaction` |
| `pause` / `resume` | Engage / clear the global emergency stop (new work only). | `pause "<reason>"` records the reason in the sentinel |

### Org

| Command | What it does | Most useful flag |
|---|---|---|
| `org budget` | `set`, `list`, `clear` the per-seat budget overrides. | `set <seat> --cap N [--basis tokens\|usd] [--window daily\|weekly\|monthly] [--mode soft\|hard]`; unset fields inherit `[bylaws].budget`, and `--cap 0` is the documented release valve |
| `org holon` | Read-only views over a Holon organization: `roster`, `board`, `approvals`, `messages`, `triggers`, `overview`, plus the phase-E `supervise` loop, `tick`, `archive`, `notify`. | `overview` is the one-screen org state; every panel needs `[holon] enabled` |

### Ops and diagnostics

| Command | What it does | Most useful flag |
|---|---|---|
| `doctor` | The check battery: config, tools, integrations, API connectivity, platform, binaries. | `--fix` runs the repair legs; `--tools-tokens` prints the tool-token ranking the evidence layer's savings are measured against |
| `status` | System overview, harness summary, ESTOP state. | `--deep`, `--json` |
| `dump` | A setup summary; `--all` dumps every config key as YAML. | `--all --json` for the full config as JSON |
| `version` | Version plus the compiled memory-wire rev. | `--detailed` adds OS, arch, shell, build profile and integration versions |
| `logs` | `show [N]` and `follow` the configured log file. | `--level ERROR --component <name>` |
| `debug` | `share`, `delete`, `memory` (this process's RSS subsystem by subsystem). | `share --local` prints the report without uploading |
| `hooks` | `list`, `add`, `test`, `revoke`, `doctor` the operator's own shell hooks. | `add --event turn_end -- <command>` |
| `migrate` | `hermes-agent [--source <path>] [--dry-run]` imports a Hermes workspace's MEMORY.md / USER.md. | `--dry-run` previews without writing |
| `update` | `check`, `apply`, `integrations [--only sourcehound\|aft\|ponytail]`. | `integrations --only ponytail` refreshes content WITHOUT swapping the aft binary out (the other two are binaries) |
| `service` | `install`, `start`, `stop`, `restart`, `status`, `uninstall`, `logs` the one daemon (`operant-gateway`). | `logs -n 100 -f` |
| `backup` / `import` / `uninstall` | Snapshot the operant home; restore one; remove the install. | `uninstall data --yes` keeps config, `uninstall full --yes` removes everything; without `--yes` it only prints what it WOULD remove |
| `architecture` | `validate`, `dump`, `pool-import` the harness architecture file. | `dump --live` builds a live Harness and dumps the kernel's `DumpTree` |
| `suggestions` | `list`, `accept`, `dismiss`, `catalog`, `clear` the suggested automations. | `accept <n>` creates the cron job |
| `cookies` | `import`, `list`, `export`, `clear`, `discover` browser cookies. | `import --browser chrome` pulls straight from a browser profile |
| `acp` | `server [--socket <path>]` runs the ACP server; `attach --socket <path> --command <text>` sends one command to a running one. | `--socket` is how MANY clients share ONE process instead of one operant per client (RT-513) |
| `dashboard` | `server [--port 9119] [--host 127.0.0.1]`. | `--stop` / `--status` use the pid file; `--insecure` disables auth; `--no-open` skips the browser launch |
| `tui` | `debug <panel>`, `effort`, `mode`, `output-style`, `theme`, `vim`, `keybindings` — TUI↔CLI parity. | `debug simulate --keys '<enter>' --assert-screen 'count:NEEDLE==1'` drives the real event handlers against a TestBackend |

### In-process vs shell-out

Almost everything runs IN PROCESS: the handler parses, reads/writes the
database or a file, and prints. The exceptions, each with the external
binary it names:

- `gateway start|stop|restart|status` and `service *` shell out to
  `systemctl` (and `launchctl` on macOS); `service logs` shells out to
  `tail`; `gateway stop` / `install` also call `kill` and `systemctl`.
  `cmd_service.rs` and `cmd_gateway.rs` are the only handlers with a
  meaningful number of spawn sites (9 and 13 respectively).
- `config edit` and `post_setup.rs` open `$EDITOR` (falling back to
  `nano`).
- `doctor` probes external binaries: `git`, `docker`, `ssh`, `npm`,
  `systemctl`, `pgrep`, `gh`, `codex`, `loginctl`
  (`cmd_doctor/checks_tools.rs`, `cmd_doctor/checks_config.rs`). Every
  probe has a timeout because `std::process::Command` has none.
- `plugins install <git url>` shells out to `git`
  (`plugins_install.rs`).
- The Herdr integration shells out to the `herdr` binary
  (`integrations/herdr_plugin.rs`).
- `autonomous` shells out to git and to the configured test command
  (`autonomous.rs`).
- The gateway's WhatsApp arm shells out to `python3`
  (`gateway_runner.rs`).

## The support modules behind the commands

These are the CLI's own machinery — the parts that are neither a command
nor a handler, and which no other library page names.

| Module | What it is |
|---|---|
| `commands.rs` | The slash-command registry: a static `COMMAND_REGISTRY` slice of `CommandDef` (name, description, aliases, args hint, `cli_only` / `gateway_only` gating) plus a runtime handler map. **84 slash commands**, derived from `CommandDef::new(` in the registry. This is what a session resolves; the CLI subcommands above are the shell surface, and the two are deliberately separate registries. |
| `gateway_api.rs` | The daemon's read-only session API (`GET /api/sessions`, `/api/sessions/:id/messages`, `/api/subagents`) in one axum route group, token-gated with the dashboard's discipline, loopback-bound. Read-only on purpose: the daemon already holds the single-writer flock. |
| `gateway_runner/allocator.rs` | Arena cap + debounced `malloc_trim` for the long-lived daemon (vendored from jcode). Without it glibc's one-arena-per-core retained ~94 MB on an idle connection. |
| `gateway_runner/enrichment.rs` | Telegram photo / voice / document arms of the turn handler, split out of `gateway_runner.rs` so that file does one job. |
| `skill_roots.rs` | The ONE list of vendor skill roots (installed content integrations plus plugin-contributed dirs), so the SkillManager, the in-loop tools, the TUI overlay, the CLI listing and the gateway's `/skills` cannot disagree about which skills exist. |
| `prompt_helpers.rs` | The shared interactive prompt primitives for the setup wizard (text / password / confirm / select / multiselect / fuzzy), all following "show the current value, Enter keeps it". |
| `hardening.rs` | `RLIMIT_CORE = 0` + `PR_SET_DUMPABLE = 0`, run first. Every step returns `Result` and the caller logs and continues. |
| `post_setup.rs` | The post-wizard summary and configuration menu loop. |
| `memory_provider_tools.rs` | Adapter that registers the ACTIVE memory provider's own tool schemas (`memory_smart_search`, `memory_save`, …) into the registry, so the memory surface exists without the optional MCP server. |
| `integrations/herdr_plugin.rs` | Install / inspect / repair the Herdr plugin (`herdr plugin link <dir>`), plus its doctor action. |
| `integrations/omarchy_agent.rs` | Register operant as an Omarchy selectable default agent. NOT a config edit: Omarchy's agent list is a `case` inside a package-owned script, so the fix is a `~/.local/bin` wrapper that precedes `/usr/bin` on PATH. |

## Surfaces — the slash commands inside a session

Type `/` in the TUI or a channel and `commands.rs` resolves it. 84
commands are registered, spanning session control (`/new`, `/branch`,
`/compress`, `/undo`, `/retry`), model and behaviour (`/model`,
`/provider`, `/reasoning`, `/yolo`, `/verbose`, `/skin`), management
(`/skills`, `/mcp`, `/plugins`, `/tools`, `/cron`, `/kanban`, `/curator`,
`/memory`), ops (`/doctor`, `/status`, `/usage`, `/insights`, `/debug`,
`/reload`, `/restart`), and a `ShellCommand` escape hatch for the few that
spawn an interactive subprocess. A registered command can never fall
through to the model — that invariant is pinned both statically
(`no_registered_tui_command_is_unwired`) and by the `command_sweep.py`
corpus.

## Failure modes — what the operator sees

1. **A channel token passed as `--api-key`.** `cmd_channel` restores the
   file's original `client.api_key`, `client.base_url` and `agent.model`
   before it persists, because channel commands write the config back.
2. **`gateway run` losing its logs.** A daemon forces stderr (or the
   configured `log_file`) regardless of `[tui] rich_output`, which would
   otherwise route logs to `io::sink()`.
3. **A refused hardening step.** A warning on stderr naming the step; the
   process continues and may dump core on a crash. It never refuses to
   start.
4. **A config that will not parse.** Every settings struct carries
   `deny_unknown_fields`, so the loader takes the upgrade-parking path
   (`config_heal::split_unknown`), heals what it can, names every change,
   and persists once. An unknown key from a NEWER binary parks rather than
   killing the run.
5. **A `cli` grant that does nothing.** `[cli] enabled` defaults to false
   and `cli_exec` has not landed, so `context cli add` records and costs a
   grant it cannot yet invoke. The command says so.

## Intention

The CLI is the operator's whole relationship with the harness: 52
subcommands, each one thin, each one delegating to the library that owns
the mechanism, so a fix to a mechanism lands once and every command that
touches it gets it. The two registries (shell subcommands and slash
commands) are separate because their audiences are: one is a script or a
human at a prompt, the other is a human or an agent inside a live session.
The support modules exist because the alternative was worse: `gateway_api`
so five surfaces could share ONE session record instead of five readers of
one database, `skill_roots` so five surfaces could not disagree about
which skills exist, `allocator` so the daemon's RSS is a number someone
chose rather than an accident of glibc. And `hardening.rs` runs first
because a core dump written while loading `.env` makes the `0600` on that
file worth nothing.
