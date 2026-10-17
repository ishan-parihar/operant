# 08 — One memory engine everywhere (finish 11C3)

**Size:** M. **Finding:** 10 in `01`.

Owner instruction: memory-wire is the in-process memory, with no alternative
memory infrastructure. Plan 11C3 (iter-846) switched the **agent** to
memory-wire only. The surfaces around the agent were not switched.

## Evidence

| Surface | Reads/writes | Where |
|---|---|---|
| `operant memory list/show/search/store/get/delete/stats/profile/import/export/prune/clear` | legacy `MemoryManager` over `~/.operant/MEMORY.md`/`USER.md` | `crates/operant-cli/src/cmd_memory.rs:100-115,305-330` |
| `operant doctor` memory lines | file existence + size of MEMORY.md/USER.md | `crates/operant-cli/src/cmd_doctor/checks_config.rs:356-383` |
| TUI `/whoami` | `MemoryStore::new(~/.operant/memory)` (a third path) | `crates/operant-cli/src/tui/app/commands.rs:1580-1625` |
| TUI `/journey` | `MemoryStore::new(memory_dir)` | `crates/operant-cli/src/tui/journey_view.rs:95` |
| `operant migrate` | copies MEMORY.md/USER.md from other runtimes into `~/.operant` | `crates/operant-cli/src/cmd_migrate.rs:37-97` |
| `[memory]` defaults | `provider = "agentmemory"`, `agentmemory_auto_spawn = Some(true)`, plus `agentmemory_url/secret/version` fields and `agentmemory_package_spec()`/`agentmemory_mcp_spec()` | `crates/operant-core/src/config.rs:1505-1592` |
| warning | `warn_legacy_keys` fires on the **defaults** (provider "agentmemory" isn't in the allowed list, auto_spawn defaults to true) | called at `config.rs:2093` |
| `operant_config::DEFAULT_AGENTMEMORY_VERSION` | still used in 7 places | `crates/operant-config/src/lib.rs:13`, `schema/config_impl.rs:477` |

Seen: the simulator run with the clean `operant.example.toml` printed
`warning: [memory] agentmemory_auto_spawn is removed …` on stderr. Every user
sees it on every start, and before the TUI takes the screen.

Concrete split: `operant memory store "prefers tabs"` writes MEMORY.md. The
agent recalls only from memory-wire, so it never sees the fact. The 11C3
one-time importer only renames on the first start after the upgrade, so later
writes are never imported. `/whoami` reports "I don't know much about you yet"
while memory-wire holds hundreds of rows.

## Fix

memory-wire's `MemoryService` already offers what the CLI needs
(`api.rs` in the memory-wire checkout: `retain`, `recall`, `list_memories`,
`get_memory`, `delete_memory`, `bank_stats`). operant's
`MemoryWireProvider` wraps the service (`crates/operant-core/src/memory_wire.rs:152-330`).
Expose thin pass-throughs on `MemoryWireProvider`. Add no new abstraction.

| `operant memory …` | becomes |
|---|---|
| `list` | `list_memories(bank)` (add `--bank`, default the personal bank, `--all-banks` for diagnostics) |
| `show`, `get <id>` | `get_memory` (merge into one subcommand; keep `show` as an alias) |
| `search <q>` | `recall` |
| `store <text>` | `retain` |
| `delete <id>` | `delete_memory` |
| `stats` | `bank_stats` per bank |
| `clear` | delete every memory in the bank (keep the confirmation) |
| `export` | `list_memories` → JSONL |
| `import <file>` | JSONL → `retain`; MEMORY.md/USER.md input goes through the existing `import_legacy_files` (`memory_wire.rs:298`) |
| `profile` | `recall("user profile preferences")`, or delete. Default: delete (it read USER.md) |
| `prune --older-than` | delete if memory-wire has no timestamp filter; don't build one |

- `doctor`: one line, "memory-wire: <path>, N memories in <bank>, …" from
  `bank_stats`. Delete the MEMORY.md/USER.md probes.
- `/whoami`: `recall("about the user")` top 10, rendered as now. Delete the
  `MemoryStore` read.
- `/journey`: the memory half switches to `list_memories`. If the journey view has
  no other value, delete the view (ask the owner. Default: keep the skills half).
- `operant migrate`: import into memory-wire via `import_legacy_files`, not
  file copy.
- Config: delete `agentmemory_url`, `agentmemory_secret`, `agentmemory_auto_spawn`,
  `agentmemory_version` and the two `*_spec()` fns. Default `provider` becomes
  `"memory-wire"`. **Keep accepting** the old keys on load so existing files
  still parse (`deny_unknown_fields` is on the struct). Do that with
  `#[serde(default, skip_serializing)]` fields, or a serde `alias`, and warn once
  only when the key is *present in the file*. That means the warning must look
  at the parsed TOML table, not at the defaulted struct.
- Delete `operant_config::DEFAULT_AGENTMEMORY_VERSION` with its last user.
- Delete `operant_core::memory::MemoryStore` / `MemoryManager` if nothing else
  uses them after this. Grep the whole tree, including tests and the agent's
  `load_repo_memory_manager` in `main.rs`. `main.rs:2305` says the import runs
  inside the factory, so check whether the manager is still needed for that.

## Tests first

- `MemorySettings::default()` + `warn_legacy_keys` → no warning (capture
  stderr, or return the warnings as `Vec<String>` and assert empty). RED today.
- `operant memory store X` then the agent's provider `recall(X)` → finds it.
  RED today.
- Load a TOML with `agentmemory_auto_spawn = true` → it parses, warns once, and the
  re-serialized config does **not** contain the key.

## Acceptance

- `rtk grep -rn "MEMORY.md\|USER.md" crates/operant-cli/src crates/operant-core/src`
  → only the importer and its tests.
- `rtk grep -rn "agentmemory" crates --include=*.rs` → only the migration
  warning and tests.
- `operant.example.toml` `[memory]` comment updated. README memory section says
  "memory-wire, banks: personal / emp:<seat>".
