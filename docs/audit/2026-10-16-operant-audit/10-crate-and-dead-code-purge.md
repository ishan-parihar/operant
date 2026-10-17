# 10 — Crate and dead-code purge (plan 11D, now with evidence)

**Size:** L (mostly mechanical). **Findings:** 17, 18, 19 in `01`. **Do this last**,
after 02–09, so deletions don't race their dependents.

## Evidence: what the binary actually calls

Method: for each workspace crate, grep `<crate_name>::` and `use <crate_name>`
across every *other* crate's `.rs` files (tests included), and read each
`Cargo.toml` dep line. Line counts are `.rs` lines including tests.

| Crate | Lines | Who names it in code | Verdict |
|---|---|---|---|
| `operant-gateway` | 16,694 | **nobody** (the cli has a Cargo dep behind feature `gateway`, but no `operant_gateway::` anywhere) | **delete** |
| `operant-hardware` | 9,581 | only `operant-gateway` | **delete** (with gateway) |
| `operant-channels` | 60,810 | only `operant-gateway`. The cli dep is a "feature carrier" (`operant-cli/Cargo.toml:22-29`); the real channels run from `operant-core/src/gateway/{telegram,discord,slack,email,sms,webhook,whatsapp}.rs` | **delete** |
| `operant-runtime` | 85,530 | cli: **one** call, `operant_runtime::agent::reconciled::core_observer_from_config` (`operant-cli/src/gateway_runner.rs:4076`); the other two hits are comments (`cmd_gateway.rs:388`, `cmd_channel.rs:99`) | **delete** after moving that one function |
| `operant-tool-call-parser` | 2,996 | only `operant-runtime` | **delete** (with runtime) |
| `operant-tool-planning` | 386 | only `operant-runtime` | **delete** (with runtime) |
| `operant-tools` | 46,867 | `operant-core`: **one** call, `operant_tools::cli_discovery::discover_cli_tools` (`operant-core/src/doctor.rs:426`) | phase 2: move `cli_discovery` into core, then check whether anything else needs the crate |
| `operant-providers` | 37,916 | `operant-core`: **one** call, `operant_providers::create_provider` (`operant-core/src/doctor.rs:259`) | phase 2: same, or drop that doctor check |
| `operant-memory` | 13,593 | `operant-tools`, `-channels`, `-gateway`, `-runtime` (Postgres/Qdrant backends) | goes away with phase 2 if `operant-tools` goes; it is the "other memory infrastructure" the owner wants gone |
| `operant-config` | 30,541 | core: `DEFAULT_AGENTMEMORY_VERSION` (deleted by `08`), `schema::{Config, ModelProviderConfig, ObservabilityConfig}`, `providers::ProvidersConfig`, `scattered_types::ContextCompressionConfig` | phase 3: move the 4–5 types core uses into `operant-core/src/config.rs` (the one config home per CLAUDE.md), then delete |
| `operant-api` | 3,335 | many crates | re-check after phases 1–3; likely only used by the deleted ones |
| `operant-plugins` | 2,297 | cli `main.rs:1685-1727`, `plugin_memory.rs` | **keep** (wasm plugins are wired) |
| `operant-harness` | 7,865 | cli, core | keep |
| `operant-os` | 3,659 | cli | keep (Holon body) |
| `operant-infra`, `operant-macros` | 2,421 / 1,287 | config, runtime, … | re-check after phases 1–3 |

Phase 1 alone: gateway + hardware + channels + runtime + parser + planning
= **175,997 lines, 6 crates**.

## Phase 1 steps (one crate per commit, each proven)

1. **Move the observer factory.** The user config has `[observability] backend = "none"`.
   The lazy rung: in `gateway_runner.rs:4075-4078` use core's no-op
   observer (or `ConsoleObserver`, `operant-core/src/observer.rs:211`). If
   `backend != "none"`, log one warning: "observability backends were removed;
   the runtime crate is gone". **Ask the owner** whether they use
   otel or prometheus (`operant-runtime/src/observability/{otel,prometheus}.rs`).
   If yes, move just that one backend file into core. Default: no-op.
2. Remove `operant-runtime` from `operant-cli/Cargo.toml` and from the
   `agent-runtime` feature list. Build. Run the gateway smoke (below). Delete the crate
   dir and its workspace member line. Commit.
3. Remove the `gateway` feature (`gateway = ["dep:operant-gateway"]`) and every
   `channel-*` feature from `operant-cli/Cargo.toml` (they forward only to
   `operant-channels` / `operant-gateway`). **Check first**: grep
   `cfg(feature = "channel-` in `operant-core` and `operant-cli` .rs files. If
   the core gateway gates a platform on these, keep a core-local feature with the
   same name. Build, smoke, then delete `operant-gateway`, `operant-hardware` and
   `operant-channels`, one commit each.
4. Delete `operant-tool-call-parser` and `operant-tool-planning` (no users left).
5. `cmd_hardware.rs`, `cmd_peripheral.rs` and their `Commands::Hardware/Peripheral`
   arms (`main.rs:480-495,3117-3122`): check whether they used
   `operant-hardware`. They don't name it, so they are cli-local. **Ask the owner**
   (STM32/Arduino flashing in a coding agent). Default: delete.

**Per-deletion proof (paste into each commit body):**
- `scripts/build.sh --release` OK;
- `cargo test -p operant-cli --bin operant` + `-p operant-core` (gate green after `02`);
- gateway smoke: start the gateway with the user's config in a scratch
  `HERMES_HOME` copy; `operant gateway status` reports every enabled platform
  started; one Telegram (or webhook) round-trip if a test bot is configured;
- `operant --help` subcommand list unchanged (except deliberate removals);
- `du -sh target/release/operant` before → after.

## Phase 2–3 (separate iterations, re-measure first)

- Move `cli_discovery` (for doctor) into core, or drop that doctor check. Same
  for `create_provider`. Then `operant-tools` and `operant-providers` have no
  users. Before deleting, grep the 21 agent tools core registers to confirm
  none comes from `operant-tools` (core has its own `src/tools/`).
- `operant-config` → fold the handful of used types into core's `config.rs`.
- Write `docs/CRATES.md`: one line per remaining crate, saying why it exists.

## Dead code inside kept crates (same plan, small commits)

| What | Where | Action |
|---|---|---|
| Legacy twins of ported jcode pickers | `tui/model_picker/`, `tui/session_browser.rs`, `tui/effort_picker.rs`, `tui/session_branching.rs` | delete as `05` B replaces each |
| 113 `#[allow(dead_code)]` in `crates/operant-cli/src/tui` | `rtk grep -rn "allow(dead_code)" crates/operant-cli/src/tui` | remove each allow. Delete what the compiler then flags as unused, unless it is a `[port-decision]` the `05` B port will wire next |
| `#[cfg(any())]` disabled `/effort` row | `tui/operant_app/app.rs:266-268` | delete |
| `CommandRegistry::{len,is_empty,commands_by_category,format_help,all_commands}` marked "prepared for…" | `crates/operant-cli/src/commands.rs:~997-1100` | delete (`04` replaces the table) |
| `intercept_slash_command_with_args`, `intercept_slash_command` "used in tests" twins | `tui/app/commands.rs:6-26` | tests call `handle_tui_command`; delete the twins |
| `bash_command_allowed_by_prefix` test twin | `tui/app/dialog_routing.rs:248-252` | `03` §4 |
| `infer_provider_from_model` copy | `cmd_tui_debug.rs:780` | `05` A |
| Two Holon launcher resolvers | `operant-os/src/run.rs:43-50` (supervisor) vs `tools.rs:72-86` (body tools) | the bodies honour `$HOLON` (the body contract) and the supervisor doesn't. That is probably right, because the supervisor is not a body. Share only the fallback chain (`[holon] bin` → `$HOLON_BIN` → `holon`) as one fn. Not a bug |
| `set_nested` copy | `operant-core/src/tools/config_tool.rs` vs `operant-cli/src/cmd_config.rs` | `09` |
| Stale comments naming retired modules as live | `operant-core/src/agent/stream.rs:1083-1120` | `02` |
| `browser_provider.rs` retired-name shims | `operant-core/src/browser_provider.rs:234-240,465-470` | keep the load-time warning, delete the rest once a release has shipped with the warning |

## Not a finding (deliberate, keep)

- `// ponytail:` comments that name their ceiling, for example `/background`'s
  (`tui/app/commands.rs:1227-1230`). `04` deletes the command anyway.
- The 7 kept `org/` files. They follow the disposition.

## Acceptance

- `Cargo.toml` workspace members: 18 → 12 after phase 1.
- Each deletion commit carries the proof block above.
- `docs/CRATES.md` exists and matches `Cargo.toml`.
