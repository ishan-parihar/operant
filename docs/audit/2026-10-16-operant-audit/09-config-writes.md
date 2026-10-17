# 09 — Config writes: change one key, in the file that was loaded

**Size:** S. **Findings:** 7, 18 in `01`.

## Evidence

**The TUI rewrites the whole file.** `App::sync_model_to_toml`
(`crates/operant-cli/src/tui/app/providers.rs:140-166`), called from
`persist_provider_and_model` (`:131-138`) on every model pick and provider
activation:

```rust
let mut config = operant_core::config::runtime_config();   // after CLI flags + env overrides
config.agent.model = model.to_string();
if let Some(ref provider) = self.active_provider { … config.client.base_url = p.default_base_url … }
let config_path = ~/.operant/operant.toml;                 // ignores `-c <path>`
std::fs::write(&config_path, toml::to_string_pretty(&config))
```

Effects (each one is concrete):
1. **Comments and layout are gone.** The whole file becomes serde output. The
   user's `~/.operant/operant.toml` shows the signs: every default section is
   written out (`[pk]`, `[genome.budget]`, `[credential_pool.strategies]`, …), and
   `[memory] agentmemory_auto_spawn = true` sits there even though 11C3
   removed it. The defaults keep writing it back.
2. **Wrong file.** `operant -c ~/work.toml` → `/model` writes `~/.operant/operant.toml`.
3. **base_url clobber.** The provider is *guessed from the model name*
   (`05` A1 row 2). Picking `claude-…` sets `client.base_url = "https://api.anthropic.com"`,
   which replaces `http://localhost:20129/v1` (the omp gateway) on the next start.
4. **Secrets to disk.** `runtime_config()` holds the env- and flag-resolved
   values (`main.rs:2806-2845`: `cli_app_config.client.api_key`,
   telegram/discord/slack tokens, then `apply_cli_overrides`). `ClientSettings.api_key`
   has no `skip_serializing` (`crates/operant-core/src/config.rs:426-432`), so a
   key that came from `OPENAI_API_KEY` ends up in the TOML in plain text.
5. Errors are swallowed (`if let Ok(...)`, `let _ =`).

**The CLI setters persist nothing:**
- `operant model set <name>` (`crates/operant-cli/src/cmd_model.rs:129-150`)
  changes the in-memory `runtime_config` and prints "Model set to: …". Then the
  process exits.
- `operant config set <key> <value>` (`crates/operant-cli/src/cmd_config.rs:69-88`)
  does the same.
- The agent's config tool has a copy of the same `set_nested` code
  (`crates/operant-core/src/tools/config_tool.rs:178-198`). In-process that is fine,
  but it is a duplicate.

## Fix (one writer, used by all three)

`toml_edit 0.22` is already in `Cargo.lock` (used by `operant-config`,
`comment_writer.rs`, `schema/config_impl.rs:1940`). Add it to
`operant-core`'s deps, so there is no new crate.

```rust
// crates/operant-core/src/config.rs
/// Set one dotted key in the config file that was actually loaded, keeping
/// every comment and every other key byte-for-byte.
pub fn persist_config_key(path: &Path, dotted: &str, value: toml_edit::Item) -> Result<()>
```

- Read the file → `DocumentMut` → walk/create tables for the dotted path →
  set the leaf → write to `path.tmp` → `rename`. Keep the file mode (the user's file
  is 0600). Return errors; callers show them.
- The loaded path: store it once at load (`load_app_config` already knows it;
  `LoadedConfig` likely carries it, else add `source_path`). Expose
  `operant_core::config::loaded_config_path()`.
- Validation: after writing, re-load the file through the normal loader; on
  error restore the previous bytes. This guards `deny_unknown_fields` typos from
  `operant config set`.

Callers:
- TUI `persist_provider_and_model` → `persist_config_key(path, "agent.model", value(model))`.
  **Never** write `client.*` from the TUI. Delete `sync_model_to_toml`.
- `operant model set` → the same call, plus print the path written.
- `operant config set` → `persist_config_key(path, key, coerce(value))`. Keep the
  in-memory update for `operant config get` in the same process.
- The agent config tool: keep its in-memory semantics, but replace its `set_nested`
  with the one in `cmd_config.rs` moved to core (one copy). Make it persist if
  and only if the tool's contract says so (read its schema description; don't
  widen it silently).

## Tests first

- Temp config with a comment and `client.base_url = "http://localhost:20129/v1"`
  → `persist_config_key(..., "agent.model", "small-stack")` → the file contains the
  comment and the base_url unchanged, and `agent.model = "small-stack"`. RED today
  (`sync_model_to_toml` drops both).
- `OPENAI_API_KEY=sk-x` in the env, then a model pick → the file does not contain
  `sk-x`. RED today.
- `operant model set foo` then `operant model show` (a new process) → `foo`.
  RED today.
- `-c /tmp/x.toml` → the write lands in `/tmp/x.toml`.

## Acceptance

- `rtk grep -rn "to_string_pretty(&config)" crates/operant-cli/src` → 0 hits in
  write paths.
- `rtk grep -rn "fn set_nested" crates` → 1 hit.
- Tell the owner (in the CHANGELOG and the iteration summary): their current
  `~/.operant/operant.toml` holds serde-expanded defaults, plus an `api_key` and a
  `telegram_token`. Suggest they review the file. **Don't edit the user's
  file for them.**
