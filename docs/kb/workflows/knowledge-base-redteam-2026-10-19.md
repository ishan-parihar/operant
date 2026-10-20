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

Reconciled 2026-10-20 (iter-1056) against `docs/audit/2026-06-19-BUGS.md`, which is the
owner of each row's fix state. Every row below was `open` when this board was
written; every row is now closed, and the table names the iteration. The
`Dedupe:` and `Limits:` notes in each BUGS.md row carry the residuals.

| ID | Sev | One line | Status |
|---|---|---|---|
| KB-1 | H | `learning_manage` delete and edit followed a `node_id` out of the skills dir | fixed iter-958 |
| KB-2 | H | `skill_manage` `find_skill` and `category` joined unchecked names | fixed iter-958 |
| KB-3 | H | knowledge writes have no approval gate | fixed iter-971 (AD-020) |
| KB-4 | H | the security scan on skill writes never runs | fixed iter-962 |
| KB-5 | H | skill descriptions enter the system prompt unescaped | fixed iter-962 |
| KB-6 | H | recalled memory is injected unescaped, and every turn is retained | fixed iter-971 (AD-019); retention for untrusted channels is the owner gate |
| KB-7 | H | a deferred sync can retain one seat's turn into another seat's bank | fixed iter-962 |
| KB-8 | M | unbound sessions share the personal bank | fixed iter-971 (AD-019) |
| KB-9 | M | the Holon redirect does not cover `learning_manage` | fixed iter-962 |
| KB-10 | M | declared plugin permissions are not enforced (plugins-wasm) | fixed iter-972 (AD-023); FileRead/FileWrite stay declarative |
| KB-11 | M | a plugin tool can replace a built-in (plugins-wasm) | fixed iter-972 (AD-023) |
| KB-12 | M | writes have no length caps, quotas, or rate limit | fixed iter-971 (AD-021) |
| KB-13 | M | secrets are stored in memory and in skills | fixed iter-971 (AD-021) |
| KB-14 | M | the memory store silently falls back to in-memory | fixed iter-971 (AD-022) |
| KB-15 | M | non-atomic writes and lost updates on memory and skill files | fixed iter-971 (AD-021) |
| KB-16 | L | symlinks inside a skill directory are followed | fixed iter-971 (AD-021) |
| KB-17 | L | the plugin HTTP host function accepts internal addresses (plugins-wasm) | fixed iter-972 (AD-023) |
| KB-18 | L | a WASM plugin is instantiated synchronously on every call | fixed iter-972 (AD-023) |
| KB-19 | L | `memory delete <id>` has no confirmation | fixed iter-971 (AD-022) |
| KB-20 | M | `operant plugins disable` has no effect on loading (plugins-wasm) | fixed iter-972 (AD-023) |

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
- KB-8 reachability: whether `allowed_users` enforcement stops a non-owner from reaching an unbound session was not traced. **Superseded by KB-8's fix (iter-971, AD-019):** the reachability question was about the gating mechanism, and the design that landed does not rely on `allowed_users` — an unbound sender who is not in `gateway.admins` runs in its own `MemoryScope::Sender`, so no two senders share a bank. The sender path is the gate now.

## Next

Nothing on this board is open. All 20 rows are closed, and the fix states above are read from `docs/audit/2026-06-19-BUGS.md`, which owns them.

Two residuals are worth naming rather than closing, because they are the rows a future session would otherwise re-open:

- KB-3 / KB-4, the policy question of whether knowledge writes from background review and gateway sessions are prompted — answered by AD-020 (iter-971). The staged-apply path was not built: the `write_approval` module still has no production caller.
- KB-10, FileRead and FileWrite stay declarative (no host function checks them; AD-023).
- The plugin items were sequenced in `docs/plans/plan-2026-10-19-plugin-ecosystem.md`, Phase 1, and are now closed in iter-972 (AD-023).
