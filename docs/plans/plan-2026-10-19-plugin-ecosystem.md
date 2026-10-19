# Plan: plugin ecosystem (implements AD-005)

Date: 2026-10-19. Status: proposed, waiting on owner decisions D1 to D4.
Record: `docs/decisions/AD-005-plugin-ecosystem-one-manifest-hooks-before-sdk.md`.
Security findings referenced: BUGS.md `KB-10`, `KB-11`, `KB-17`, `KB-20`.

## Order, and why it differs from the first draft

The first draft put the hook events first. This plan puts the permission gates first. Hooks let a plugin observe every tool call and every LLM request, so a hook capability that runs before a plugin's declared permissions are enforced would widen a gap that exists today (KB-10: `MemoryRead` and `MemoryWrite` are never checked, so a memory plugin with `HttpClient` already receives every turn). Phase 1 closes that gap, and Phase 2 adds the events.

## Owner decisions (block the phases named)

| ID | Decision | Recommended | Blocks |
|---|---|---|---|
| D1 | Enable state source of truth: config allow and deny lists, or keep the marker files | config (AD-005 point 6) | Phase 1.4 |
| D2 | Project-scope plugin trust: not loaded by default | not loaded (AD-005 point 8) | Phase 1.5 |
| D3 | Signing default under `plugins-wasm` | Strict in release, Disabled in dev (AD-005 point 10) | Phase 5.2 |
| D4 | Ship `plugins-wasm` in release builds | yes, after Phase 1 and 5.2 (AD-005 point 11) | Phase 5.1 |

Phases 1.1–1.3, 2, 3, and 4 do not depend on a decision.

## Phase 1: close the plugin-path gaps before adding surface

Each step is verified by the test named in it. All run under `--features plugins-wasm` where the code is feature-gated.

- **1.1 Enforce declared permissions (KB-10).** Check `MemoryRead` before `prefetch` and `MemoryWrite` before `sync_turn` in `crates/operant-cli/src/plugin_memory.rs`. Check the permission in each host function in `crates/operant-plugins/src/runtime.rs`. FileRead and FileWrite have no host functions yet, so they stay declarative and the plan says so.
  Verify: a memory plugin without `MemoryRead` gets no `prefetch` call (new test in `plugin_memory.rs`); `cargo test -p operant-plugins --lib`.

- **1.2 Reject built-in name collisions (KB-11).** In `register_plugin_tools` (`crates/operant-cli/src/plugin_tools.rs`), refuse a plugin tool whose name is already registered as a built-in, instead of letting `ToolRegistry::register` overwrite it (`tools.rs:430-437`).
  Verify: a test plugin named `terminal` is rejected with an error, and the built-in `terminal` is still the registered tool.

- **1.3 HTTP host denies internal addresses (KB-17).** In `handle_http_request` (`runtime.rs:68`), refuse loopback, RFC 1918, link-local (including `169.254.169.254`), and unique-local addresses after DNS resolution.
  Verify: tests for `127.0.0.1`, `10.0.0.1`, and `169.254.169.254` are refused; a public address is allowed through the same check.
  Ceiling: DNS rebinding between the check and the request. Upgrade path: pin the resolved address into the request.

- **1.4 Enable state (KB-20, D1).** If D1 is config: add `[plugins].enabled` and `[plugins].disabled` to `operant-config`, have the load path in `plugin_tools.rs` and `plugin_memory.rs` honor them, and move `cmd_plugins.rs` and `tui/plugins_hub.rs` onto the config. Update `operant.example.toml` in the same change.
  Verify: a test with `disabled = ["x"]` registers no tools from `x`; `operant plugins disable x` followed by a restart leaves `x` out of `operant tools list`.

- **1.5 Project-scope trust (D2).** Load a plugin from the workspace's `plugins/` only when the workspace is trusted. Verify: an untrusted workspace registers no plugin tools; a trusted one does.

## Phase 2: tool-call and LLM-request events (AD-005 point 2)

- **2.1 New events.** Add `PreToolCall`, `PostToolCall`, `PreLlmCall`, and `PostLlmCall` to `HookEvent` (`crates/operant-core/src/gateway_pipeline.rs:65`). Emit them from `crates/operant-core/src/agent/run.rs` at the tool dispatch site and the provider request site. `PreToolCall` can return a block with a reason, which becomes the tool result. `PreLlmCall` can return context, which is appended to the user message.
  Verify: a test that a hook fires once per tool call and in order; a test that a block refuses the call and returns the reason; `cargo test -p operant-core --lib -- hook`.

- **2.2 Bridge to the CLI registry.** The `Hooks("*")` bridge in `crates/operant-cli/src/main.rs` (around 1471–1476) must carry the new events to `HookSlot::bridge_handler`.
  Verify: a registered hook receives a `PreToolCall` in the running CLI path (a test through the registry, not a mock).

- **2.3 Timeout.** Every hook call has a timeout. Verify: a hook that never returns is cut off and the call proceeds under that event's fail mode.
  Ceiling: a hook adds its latency to every tool call. Upgrade path: a per-event budget in config.

## Phase 3: plugin hooks (AD-005 point 3)

- **3.1 Manifest.** Add `[[hooks]]` to `PluginManifest` (`crates/operant-plugins/src/lib.rs:21-50`) with `event`, `export`, and `fail_mode`. Add the `Hook` capability to `PluginCapability`.
  Verify: manifest parse tests for valid and invalid events.

- **3.2 WASM `DynamicHook`.** Implement `DynamicHook` for a WASM plugin, creating an instance per call.
  Verify: a fixture plugin that blocks a named tool; an end-to-end test with `--features plugins-wasm` shows the tool call refused.

## Phase 4: guest SDK and example plugin (AD-005 point 9)

- **4.1 SDK crate.** `crates/operant-plugin-sdk`, for `wasm32-unknown-unknown`. Wraps `tool_metadata`, `execute`, the hook export, and the JSON envelope, and carries the ABI version.
  Verify: `cargo build --target wasm32-unknown-unknown -p operant-plugin-sdk`. The target must be installed (`rustup target add wasm32-unknown-unknown`).

- **4.2 Example plugin.** `examples/plugins/guard-shell`: a `PreToolCall` hook that blocks `rm -rf /`.
  Verify: an integration test loads it and sees the block.

## Phase 5: release build and signing (D3, D4)

- **5.1 Ship `plugins-wasm` in release (D4).** Enable the feature in the release build path (`scripts/build.sh`). Verify: the release binary's `operant plugins list` runs and lists an installed plugin; the provenance gate in AGENTS §7 passes.

- **5.2 Signing default (D3).** Verify: an unsigned plugin is refused under Strict; a signed one loads.

## Phase 6: distribution (AD-005 point 12)

- **6.1 Git install.** `operant plugins install <git-url>[@ref]`, pinned to a commit in a lock file.
  Verify: install from a local bare repository in a test; the lock records the commit.

- **6.2 Docs.** Each phase that changes config or user-visible behaviour updates `operant.example.toml`, `README.md`, and `docs/CHANGELOG.md`, as CLAUDE.md requires.

## Gates for every phase

1. `cargo fmt --all`
2. `cargo check --workspace --all-targets`
3. The phase's tests, named above
4. `bash scripts/clippy-warning-gate.sh` reports `new=0 stale=0`
5. `./scripts/genome lint` before any doc commit

## Not in this plan

- Interop with Claude Code or Hermes plugin packages.
- A pip entry-point loader.
- A marketplace index.
- Image, video, TTS, web-search, or platform registration.
- The knowledge-base findings KB-3 to KB-9 and KB-12 to KB-16, KB-18, and KB-19. They are tracked in BUGS.md and are not plugin-surface work.
