---
ID: "AD-005"
Title: "Operant plugins extend through one TOML manifest; permissions are enforced and tool and LLM hooks ship before a guest SDK"
Status: "Proposed"
Date: "2026-10-19"
Description: "Plugin ecosystem architecture: one TOML manifest, enforced permissions, tool-call and LLM-request hooks, config-owned enable state, and a git-first install path"
---

# AD-005: Operant plugins extend through one TOML manifest; permissions are enforced and tool and LLM hooks ship before a guest SDK

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

**What operant has, read from the code on 2026-10-19.**

- WASM plugin runtime: `crates/operant-plugins`, on Extism. Manifest `manifest.toml` (`PluginManifest`, `lib.rs:21-50`). Five capabilities (`PluginCapability`, `lib.rs:52-63`): Tool, Channel, Memory, Observer, Skill. Six permissions (`PluginPermission`, `lib.rs:68-83`): HttpClient, FileRead, FileWrite, EnvRead, MemoryRead, MemoryWrite.
- Guest ABI: the guest exports `tool_metadata` and `execute` (`runtime.rs:205, 216`). Host functions are `zc_http_request` (`runtime.rs:186`) and `zc_env_read` (`runtime.rs:194`). Only HttpClient and EnvRead are checked against the manifest (KB-10).
- Signing: Ed25519 through `ring` (`signature.rs`). The default mode is Disabled (`helpers.rs:558`).
- Shipping: the `plugins-wasm` feature is not in the `operant-cli` default features (`default = ["agent-runtime"]`, `Cargo.toml:94-96`), and `scripts/build.sh` passes no features. The released binary has no plugin loader.
- Hooks: `HookEvent` (`gateway_pipeline.rs:65`) has nine variants. None fires before or after a tool call, and none fires at an LLM request. The `DynamicHook` trait (`harness_slots.rs:161`) has one implementation, the test `CountingHook` (line 475). The `[hooks]` config table is not read (RT-14, BUGS.md).
- Enable state: `operant plugins enable` and `disable`, and the TUI plugins hub, write `<name>.enabled` marker files (`cmd_plugins.rs`, `tui/plugins_hub.rs`). The load path does not read them (KB-20).
- Plugin-path gaps from the knowledge-base red-team: declared permissions are not enforced (KB-10), a plugin tool can replace a built-in tool (KB-11), and the plugin HTTP host function accepts any address (KB-17). All three are reachable only with `plugins-wasm`, which the shipped binary does not include.

**What Hermes does** (`/home/ishanp/.hermes/hermes-agent`, read 2026-10-19).

- Loader `hermes_cli/plugins.py`: `discover_and_load` (:1298). Sources are the bundled `plugins/`, `~/.hermes/plugins/`, a project `./.hermes/plugins/` gated by `HERMES_ENABLE_PROJECT_PLUGINS` (:1376), and pip entry points `hermes_agent.plugins` (:217, :1677).
- Manifest `plugin.yaml`, parsed at :1582–1671 into `PluginManifest` (:281–315).
- 23 hook names in `VALID_HOOKS` (:135–215). The ones that matter here: `pre_tool_call` (can block or approve), `post_tool_call`, `transform_tool_result`, `pre_llm_call` (context goes into the user message, never the system prompt), `post_llm_call`, `on_session_start` and `on_session_end`, `pre_gateway_dispatch`.
- Registration: tools, commands, CLI commands, context engine, image, video, web-search, browser, TTS and transcription providers, platforms, hooks, middleware, skills.
- Enable state: a `plugins.enabled` allow list and a `plugins.disabled` deny list (:227–270), and per-plugin `plugins.entries.<id>` (`plugins_cmd.py:829`). The override gate is `allow_tool_override` (:438–491).
- Security: plugins run in-process with no sandbox and no signing.

**What Claude Code does** (`https://code.claude.com/docs/en/plugins/create` and linked pages, read 2026-10-19).

- Manifest `.claude-plugin/plugin.json`. Only `name` is required. Components: `skills/`, `commands/`, `agents/`, `hooks/hooks.json`, `.mcp.json`, `.lsp.json`, `output-styles/`, `themes/`, `monitors/`, `bin/` (added to the Bash PATH), `settings.json`.
- 33 hook events, `PreToolUse` among them. Exit code 2 blocks on blockable events. Handler types: `command`, `http`, `mcp_tool`, `prompt`, `agent` (experimental).
- Variables: `${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, `CLAUDE_PLUGIN_OPTION_<KEY>`. Install scopes are `user`, `project`, and `local`. Marketplace file `.claude-plugin/marketplace.json`, with `github`, `url`, `git-subdir`, `npm`, `archive`, and `command` sources.
- Security: plugins run with the user's privileges. Project-scope plugins need workspace trust. Managed settings can allowlist.

**Limit of this record.** The hook input schemas past roughly 120,000 characters of the Claude Code pages, and the tail of the components page, were read through a summary, not verbatim. Re-read those before implementing any part that depends on them.

## Decision

Proposed. The owner ratifies it; until then the plan does not start the items marked **[owner]**.

1. **One manifest, in TOML.** Keep `manifest.toml`. Do not adopt `.claude-plugin/plugin.json` or Hermes's `plugin.yaml`. Operant's config is TOML-first, and a second manifest format adds parsing and documentation with no gain. Claude-style skills and commands are not imported by this record.

2. **Tool-call and LLM-request events come first.** Add `PreToolCall`, `PostToolCall`, `PreLlmCall`, and `PostLlmCall` to `HookEvent`. A `PreToolCall` hook can block the call; the reason goes back to the model as the tool result. A `PreLlmCall` hook can add context to the user message, never to the system prompt. Recommended: a `PreToolCall` hook that errors or times out fails closed (the call is refused). The other events fail open and are logged.

3. **Plugin hooks are a capability backed by `DynamicHook`.** A WASM plugin declares `[[hooks]]` entries in its manifest, each with an event, an export, and a fail mode. The host creates a fresh instance per call, which is the model the runtime already uses (`runtime.rs`). No state is shared between calls.

4. **Permissions are enforced at the host boundary, before new surface ships.** A host function checks the manifest's declared permission before it runs (KB-10). A plugin tool whose name matches a built-in is rejected at registration (KB-11). The HTTP host function denies loopback, private, and link-local addresses (KB-17). Hooks let a plugin observe every turn, so these three land first.

5. **Registration stays at the five capabilities that exist.** Tool, Channel, Memory, Observer, and Skill. Do not add image, video, TTS, web-search, or platform registration until a concrete plugin needs one. Operant has no consumer for them today.

6. **[owner] One enable source of truth, in config.** `[plugins]` gets an `enabled` allow list and a `disabled` deny list, and the load path honors them. The `.enabled` marker files are retired in the same change that moves `operant plugins enable` and `disable` and the TUI hub onto the config. Recommended over keeping markers, because config is the one place an operator audits and the signing policy already lives there.

7. **Per-plugin configuration** goes in `[plugins.entries.<id>]`. Secrets are not stored in TOML. The plugin names the environment variables it reads, and the host passes them through `zc_env_read` under the EnvRead permission. This follows Hermes for the table shape and Claude Code for the option-as-env pattern.

8. **[owner] Project-scope plugins need workspace trust.** A plugin under a workspace's own plugin directory is loaded only after the operator trusts that workspace. Default: not loaded. Hermes uses an environment flag for this, and Claude Code uses workspace trust.

9. **Guest SDK after hooks.** A crate, `operant-plugin-sdk`, wraps the guest ABI (`tool_metadata`, `execute`, and the hook export) and the JSON envelope, and it carries an ABI version field in the manifest. It is built after point 3, so the SDK exposes hooks from its first release.

10. **[owner] Signing default.** Recommended: Strict when `plugins-wasm` is compiled in, and Disabled only in development builds. Today the default is Disabled everywhere (`helpers.rs:558`).

11. **[owner] Ship `plugins-wasm` in release builds, after points 4 and 10 land.** Today no released binary can load a plugin. The cost is the Extism dependency footprint in the release binary, which the owner should weigh.

12. **Distribution is git first.** `operant plugins install <git-url>[@ref]`, pinned to a commit in a lock file. A marketplace index comes later. Python pip entry points are not adopted.

## Consequences

- Plugins gain pre-tool, post-tool, pre-LLM, and post-LLM hooks, which are the mechanisms Hermes and Claude Code use for guardrails and context injection.
- Points 4 and 10 are preconditions. The hook capability is not enabled in any release until both land, because hooks let a plugin see every turn of every session.
- Config change: `[plugins]` allow and deny lists, and the retirement of the marker files (point 6). `operant.example.toml` and README must be updated when each step lands, as CLAUDE.md requires.
- Operant plugins will not load Claude Code or Hermes plugin packages unmodified. Interoperability is not a goal of this record.
- Nothing here touches memory-wire or the Holon seam.

## Orthogonality

- The plugin path is compiled out of the shipped binary today. Any statement about what ships must say so, and must name the `plugins-wasm` build.
- AD-001 (trajectory attribution) and AD-004 (roster check) are unaffected.

## Revisions

- 2026-10-19: created as Proposed, from the Hermes and Claude Code reads and the operant code reads recorded above.
