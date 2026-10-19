# Knowledge-base red-team (2026-10-19)

Scope: the tools and paths that let the agent write or read its own knowledge: `learning_manage`, `skill_manage` and `skill_view`, `memory_store`, `memory_search`, and `memory_recall`, the memory-wire provider with its per-turn sync, and the WASM plugin memory and tool path. Findings are recorded in BUGS.md as `KB-n` under "OPEN — Red-team: knowledge-base surface (2026-10-19)".

Method: a read-only sweep of the code, with file and line citations. The main agent re-read the code for KB-1, KB-2, KB-5, KB-7, KB-14, KB-19, and KB-20. KB-1 and KB-2 were fixed in iter-958 with regression tests. The others are cited but not re-run, and each row in BUGS.md says which kind it is.

## Reachability

| Component | Reachable in production | Who can write |
|---|---|---|
| `learning_manage` (`tools/learning_mutation_tool.rs`, `agent/learning_graph.rs`) | yes, registered unconditionally (`builtin.rs:177-180`) | the agent, including prompt-injected content; not permission-gated |
| `skill_manage` and `skill_view` (`tools/skills_tool.rs`) | yes (`builtin.rs:126-132`) | the agent, background review, and gateway sessions; not permission-gated |
| `memory_store`, `memory_search`, `memory_recall` (`operant-core` `memory_tools.rs`) | yes (`builtin.rs:106-108`) | the agent; every turn is also retained automatically (`run.rs:1539`) |
| memory-wire provider (`memory_wire.rs`) | yes (`main.rs:2351-2355`, `gateway_runner.rs:1266-1273`) | the agent, automatic per-turn sync, and the CLI |
| `operant-memory` knowledge graph and `knowledge_tool.rs`; `operant-tools` memory tools | no: constructed only in tests (`knowledge_tool.rs:433-434, 574-575`) | none in production |
| WASM plugin memory and plugin tools | only with `--features plugins-wasm`, which the default and `scripts/build.sh` builds do not pass | plugin WASM and the plugin directory |
| `write_approval::gate` (`write_approval.rs:69-92`) | no: no callers outside its file; default off (`:51-54`) | n/a |
| `skills_guard::scan_skill` (`skills_guard.rs:1304`) | only from CLI skill install (`cmd_skills.rs:687, 804`), not from `skill_manage` | n/a |

## Findings and where they are recorded

| ID | Sev | One line | Status |
|---|---|---|---|
| KB-1 | H | `learning_manage` delete and edit followed a `node_id` out of the skills dir | fixed iter-958 |
| KB-2 | H | `skill_manage` `find_skill` and `category` joined unchecked names | fixed iter-958 |
| KB-3 | H | knowledge writes have no approval gate | open, policy decision |
| KB-4 | H | the security scan on skill writes never runs | open |
| KB-5 | H | skill descriptions enter the system prompt unescaped | open |
| KB-6 | H | recalled memory is injected unescaped, and every turn is retained | open |
| KB-7 | H | a deferred sync can retain one seat's turn into another seat's bank | open, timing-dependent |
| KB-8 | M | unbound sessions share the personal bank | open, reachability not traced |
| KB-9 | M | the Holon redirect does not cover `learning_manage` | open |
| KB-10 | M | declared plugin permissions are not enforced (plugins-wasm) | open, Phase 1.1 of the plugin plan |
| KB-11 | M | a plugin tool can replace a built-in (plugins-wasm) | open, Phase 1.2 of the plugin plan |
| KB-12 | M | writes have no length caps, quotas, or rate limit | open |
| KB-13 | M | secrets are stored in memory and in skills | open |
| KB-14 | M | the memory store silently falls back to in-memory | open |
| KB-15 | M | non-atomic writes and lost updates on memory and skill files | open |
| KB-16 | L | symlinks inside a skill directory are followed | open |
| KB-17 | L | the plugin HTTP host function accepts internal addresses (plugins-wasm) | open, Phase 1.3 of the plugin plan |
| KB-18 | L | a WASM plugin is instantiated synchronously on every call | open |
| KB-19 | L | `memory delete <id>` has no confirmation | open |
| KB-20 | M | `operant plugins disable` has no effect on loading (plugins-wasm) | open, decision D1 |

## Checked and safe

- Memory bank isolation: the bank is an enum-derived key, not a path. The `set_scope` tests (`memory_wire.rs:536-580`) cover seat and personal partitions.
- The memory-wire calls are wrapped in `catch_unwind` (`memory_wire.rs:114-145`).
- The sync queue is bounded at 256, with `try_send` and a drop counter (`memory_provider.rs`).
- Memory node ids: the source is allowlisted to `MEMORY.md` and `USER.md` (`learning_graph.rs`), and the index is bounds-checked.
- `skill_manage` create validates the name with `validate_name` (segment regex, rejects `..`, `.`, `//`, and leading or trailing `/`).
- `validate_file_path` and lexical `..` rejection hold for supporting files inside a found skill.
- Size caps: 100k characters for `SKILL.md`, 1 MiB per supporting file.
- The background review guard blocks edits to bundled and hub skills and requires a read before an edit (`skills_tool.rs`). It applies only to background-review origin.
- Pinned skills and meta-skill routers are protected from delete.
- `cmd_memory clear` requires typed confirmation.
- Plugin runtime: HttpClient and EnvRead are checked per instance (`runtime.rs`), and each call gets a fresh instance with no shared state.

## Dropped or not verified

- The knowledge graph, `knowledge_tool.rs`, and the `operant-tools` memory tools have no production constructor, so they were not audited further. The missing confirmation on `memory_forget` and `memory_purge` is dropped as unreachable.
- memory-wire's SQLite locking lives in the external `memory-wire` crate, so it was not verified.
- The plugin WASI filesystem scope (`Plugin::new(..., true)`) was not verified.
- Panics on bad input: no panic path was found on the agent-facing paths read. The audit is not exhaustive.
- KB-8 reachability: whether `allowed_users` enforcement stops a non-owner from reaching an unbound session was not traced.

## Next

The plugin items (KB-10, KB-11, KB-17, KB-20) are sequenced in `docs/plans/plan-2026-10-19-plugin-ecosystem.md`, Phase 1. KB-3 and KB-4 need a policy decision on whether knowledge writes from background review and gateway sessions are prompted.
