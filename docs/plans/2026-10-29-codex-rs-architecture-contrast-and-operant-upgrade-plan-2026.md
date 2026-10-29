# codex-rs architecture contrast and operant upgrade plan (2026-10-29)

Dated-frozen: published `2026-10-29`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

---
## Executive summary
codex-rs is a 122-crate, ~2.2M LOC Rust implementation (OpenAI Codex reference) that
provides a rich typed tool-spec layer, execpolicy DSL, code-mode (V8/JIT + schema→TypeScript),
remote process isolation via exec-server, thread-store / rollout feature-flag engine, app-server
protocol tier, Noise-encrypted remote environment registry, BM25 deferred tool discovery, and
a multi-agent fleet architecture. operant is a 7-crate, ~409K LOC Rust agent harness that
currently lacks most of these subsystems. This plan documents the contrast, lists every
gap, and proposes incremental upgrades.

---
## Codex subsystems operant lacks (with evidence)
1. **Typed tool spec layer** — `ToolSpec` enum in `tools/src/tool_spec.rs` with serde JSON
   serialization, `From<LoadableToolSpec>`, `ToolSpec::name()`, and
   `create_tools_json_for_responses_api()`. Operant has `OperantTool` trait (name/description/schema/execute)
   but no typed JSON schema generation or model-visible declaration serialization.

2. **Execpolicy DSL** — `execpolicy/` crate (policy.rs, parser.rs, decision.rs) with a
   persisted rule file format, 10+ rule types (PrefixRule, NetworkRule, PatternToken, etc.),
   and evaluation model. Operant has `approval.rs` with 47 hardcoded regex patterns and a
   static blocklist — no external policy file, no DSL, no configurable rule engine.

3. **Code-mode** — `code-mode-protocol/` with `CODE_MODE_PRAGMA_PREFIX`, `build_exec_tool_description`,
   `render_code_mode_sample`, schema→TypeScript, and `tools.tool_search` with BM25 deferred discovery.
   Operant has a `tool_search` bridge (hermes parity) but no code-mode, no V8 runtime,
   no pragma-prefixed inline exec, no schema→TypeScript.

4. **Thread-store + rollout engine** — `thread-store/` (in_memory.rs, live_thread.rs,
   queue_store.rs, projects.rs, attachments) and `rollout/` (compression, policy,
   persistence_metrics, recorder). Operant has no persistent per-thread store, no
   feature-flag/rollout system, no model_context middleware.

5. **App-server protocol tier** — `app-server/` + `app-server-protocol/` + `app-server-daemon/`
   + `app-server-transport/` with full client/server wire protocol, daemon process, long-lived
   session, capability negotiation. Operant is single-process binary; CLI talks directly to
   `operant-core` in-process; no protocol tier, no daemon, no separate server process.

6. **Sandbox / process isolation** — `exec-server/` with `SandboxType` enum, `seccomp.rs`,
   `launcher.rs`, `bwrap.rs`, landlock support. `blast_radius.rs:26` cites
   `operant-runtime`'s sandbox backends but that crate was deleted in iter-892. No active
   sandbox layer exists; only the approval guard's regex blocklist.

7. **Noise-encrypted remote environment registry** — `exec-server/src/environment_registry.rs`
   with NoiseChannelPublicKey-encrypted requests/responses carrying `environment_id`, `url`,
   `security_profile`. Operant has no remote registry; all state is SQLite-local.

8. **BM25 deferred tool discovery** — `tools.tool_search({query, limit})` returning
   BM25-ranked results with `external_web_access`/`indexed_web_access` flags. Operant's
   bridge uses threshold-pct/catalog-budget model (5% of context, 4000 tokens) with a
   cheaper char-per-token estimate, not BM25.

9. **Persistent session with per-turn compaction + retain_doc hooks** — codex has
   `memory_reflect`/`memory_retain` with `created_at`/`document_id`/`update_mode` and
   four lifecycle hooks. Operant has tiered eviction + decay curve + auto-compression on
   overflow but no `retain_doc` hooks.

10. **Skills market / BM25 skill search** — `skills/` crate with `ToolSpec`-like definitions,
    `model.rs`, `selection.rs`, BM25 skill search in `tools/user_messaging.rs`. Operant has
    `skills.rs`, `skill_marketplace.rs`, `skills_guard.rs` but no typed skill-spec layer,
    no BM25 skill search, no deferred skill disclosure.

11. **MCP SSE transport** — `McpSseClient` with background reader task + oneshot response
    routing (iter-68). Operant has MCP client over stdio only; no SSE endpoint, no background
    reader, no oneshot routing.

12. **Dynamic tool loading / namespace discovery** — codex's `tool_search` returns
    BM25-ranked `{tools: [{name, description}]}` with callable names and full TypeScript
    declarations. Operant's bridge tools surface deferred MCP tools but use threshold-pct
    model, not BM25 ranking.

13. **Hook system: 9 events + wildcard** — codex `HookRegistry` with 9 events
    (`agent_start`, `agent_end`, `turn_start`, `turn_end`, `tool_pre_execute`,
    `tool_post_execute`, `tool_fail`, `session_start`, `session_end`) plus wildcard `*`.
    Operant has 4 events via `HookRegistry`; no tool-level hooks, no wildcard.

14. **Agent-per-seat runtime** — fleet notes (iter-631) document `create_runtime_agent`,
    per-seat agent isolation. Operant has single-agent-in-process model; no
    `create_runtime_agent`, no seat isolation, no per-seat agent stacks.

15. **MCP capability negotiation** — `protocol/src/capabilities.rs` with capabilities JSON,
    `dynamic_tools.rs`, capabilities handshake on MCP connect. Operant has no capability
    negotiation surface.

---
## operant upgrade recommendations (phased, evidence-cited)

### Phase 1 — Bridge the immediate gaps (iter-1490 through iter-1494)

1. **Add typed tool spec serialization** — Implement a `ToolSpec`-like enum in
   `operant-core/src/tools/` that can serialize tool declarations to JSON for the model
   and drive `toolChoice: "auto"`. Model after codex `tools/src/tool_spec.rs` +
   `tools/src/tool_definition.rs`. Cite: `crates/operant-core/src/tools/builtin.rs:270`
   (OperantTool trait) as the starting point; add `render_json_schema_to_typescript`
   analog. Acceptance: `cargo check -p operant-core --lib` green; `operant tools list`
   shows schema for each tool.

2. **Add execpolicy DSL skeleton** — Add a minimal policy file format parser and
   evaluator (1 rule type: `PrefixRule` + `NetworkRule`). Model after codex
   `execpolicy/` but ship only the DSL parser, not the full runtime. Cite:
   `crates/operant-core/src/approval.rs:60` (ApprovalGuard) as the existing safety
   layer; the DSL would sit above it. Acceptance: `cargo check` green; a
   `config.policy_file` path loads without error.

3. **Add code-mode pragma support** — Add `CODE_MODE_PRAGMA_PREFIX` analog and
   `build_exec_tool_description` analog that surfaces exec-capable tools only when
   `// @exec:` pragma is present in the user prompt. Cite: `crates/operant-cli/src/main.rs:1172`
   (tool_search wiring). Acceptance: toggling a config flag enables/disables the
   pragma-aware tool filter.

4. **Add BM25-inspired tool search** — Replace the threshold-pct/catalog-budget model
   in `tool_search.rs` with a cheaper BM25-style ranking (embedder + scorer). The
   existing bridge infrastructure (3 bridge tools + tiered disclosure) can stay;
   just change the search algorithm. Cite: `crates/operant-core/src/tools/tool_search.rs:1`
   (current design). Acceptance: `operant tools list` still shows bridge tools;
   `tool_search` returns ranked results instead of hidden-below-budget.

5. **Add 4 extra hook events** — Extend `HookRegistry` from 4 to 9 events
   (add `tool_pre_execute`, `tool_post_execute`, `tool_fail`, `session_start`,
   `session_end`) and the wildcard `*`. Model after codex. Cite: existing
   `crates/operant-core/src/hooks.rs` (if it exists) or the harness bridge
   (iter-61/62). Acceptance: `operant hooks` command lists all 9 events.

### Phase 2 — Subsystem parity (iter-1495 through iter-1504)

6. **Add app-server protocol stub** — Add a minimal protocol wire types crate
   (`operant-core/src/protocol/`) with item/event definitions, without the full
   daemon process. Cite: codex `protocol/` crate shapes. Acceptance: `cargo check`
   green; the new module compiles.

7. **Add thread-store analog** — Add a per-session key-value store (in-memory HashMap
   + optional persistence) for thread-local state. Cite: codex `thread-store/` crate
   shapes. Acceptance: `operant session save/get` commands persist/retrieve values.

8. **Add rollout / feature-flag engine** — Add a config-driven feature-flag system
   (percentage rollout, model_context middleware). Cite: codex `rollout/` crate.
   Acceptance: `config.features` TOML keys toggle feature gates.

9. **Add sandbox-aware blast radius** — Replace the dangling `operant-runtime` citation
   in `blast_radius.rs` with a no-op or `operant-os` landlock reference, or remove
   the sandbox lineage entirely since the approval guard is the actual safety surface.
   Cite: `crates/operant-core/src/blast_radius.rs:26`. Acceptance: the comment
   no longer references a deleted crate.

10. **Fix the one_home_resolver test gap** — Audit all non-test `.rs` files for
    `.operant` path construction; add any genuine cases to the ALLOWED list in
    `operant-core/tests/one_home_resolver.rs`. Cite: `crates/operant-core/tests/one_home_resolver.rs`.
    Acceptance: `cargo test -p operant-core --test one_home_resolver` passes.

11. **Add skills BM25 search** — Add a skill search tool analogous to
    `tools.user_messaging.rs` in codex, but for operant skills. Model after
    `crates/operant-core/src/skills.rs`. Acceptance: `operant skills search
    <query>` returns ranked skill matches.

12. **Add capability negotiation surface** — Add a minimal `operant_core::protocol::capabilities`
    module with capabilities JSON and handshake support. Cite: codex `protocol/src/capabilities.rs`.
    Acceptance: `operant doctor --capabilities` prints the capability block.

### Phase 3 — Cleanup and stability (iter-1505 through iter-1510)

13. **Remove dead code** — Delete `cli_help_parser.rs` and `cli_discovery.rs` (both
    never called from production). Cite: grep proof in operant-side.md. Acceptance:
    `cargo check -p operant-core --lib` unchanged; `git status` no longer shows them.

14. **Consolidate tool-state paths** — Ensure tool-state reads from DB only (not the
    filesystem `tool_state.json` mirror). Cite: redundancies in operant-side.md.
    Acceptance: `operant tools disable` writes to DB; `operant tools list` reads from DB;
    no filesystem JSON involved.

15. **Final provenance gate** — After all changes, build release, deploy, and verify
    `operant --version` prints the new rev; `operant doctor` prints
    `memory-wire active (the only memory engine — in-process, no API key)`; and the
    provenance gate check (`ls --time-style=full-iso target/release/operant; git log -1
    --format=%cI`) shows the artifact mtime postdates the commit.

---
## Success criteria (all must pass before merge)
- `cargo check -p operant-core --lib` green with 0 warnings (clippy gate)
- `cargo check -p operant-cli --bin operant` green
- `cargo test -p operant-core --lib` all lib tests pass
- `cargo test -p operant-cli --bin operant` all CLI tests pass
- `bash scripts/clippy-warning-gate.sh` passes (no new warnings)
- `operant doctor` prints `memory-wire active` with no provider API-key probes
- `operant --version` prints the new rev after deploy
- The provenance gate artifact mtime postdates the commit

---
## Non-goals (do not attempt in this plan)
- Re-adding `operant-runtime` crate — deleted intentionally; equivalents live elsewhere
- Adding OS-level sandboxing (landlock/bwrap) — not a production requirement
- Switching memory provider from memory-wire — canonical in-process provider
- Changing TUI rendering invariants — tested and stable
- Removing the `tool_search` bridge — hermes parity feature the operator depends on