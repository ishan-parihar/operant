# Memory — memory-wire, the one memory engine

Owns: operant's persistent memory mechanism — the memory-wire engine, its
retain/recall/sync_turn lifecycle, retrieval, import, and the
degrade-don't-die contract.
Not for: how memories are injected into a turn
([context-injection.md](context-injection.md)), session history
([sessions.md](sessions.md)), the LCM context DAG
([context-management.md](context-management.md)).

Source: `crates/operant-core/src/memory_provider.rs`,
`crates/operant-core/src/memory_wire.rs`,
`crates/operant-core/src/memory.rs`,
`crates/operant-core/src/memory_import.rs`,
`crates/operant-core/src/tools/memory_tools.rs`,
`crates/operant-cli/src/cmd_memory.rs` (CLI), `memory-wire` upstream at
<https://github.com/ishan-parihar/memory-wire> (cargo git-dependency,
advanced by `scripts/sync-vendors.sh`, rev baked into the binary).

## The one engine

`[memory] provider = "memory-wire"` — and that is the whole choice (plan
11C3). `build_memory_provider` (`memory_provider.rs`) accepts the legacy
names (`memory-wire` | `memory_wire` | `agentmemory` | `memory` | empty)
and warns once for anything else, because the alternatives are gone:

- The six legacy providers (TDG, Hindsight, RetainDb, Mem0, LocalVector,
  agentmemory-as-subprocess) were deleted.
- **`BuiltinProvider` is deleted too**: there is no file-backed fallback
  anymore. On init failure, memory degrades to *unavailable for the
  session* — the agent's degrade-don't-die contract turns it into empty
  prefetches, not a dead agent (`memory_provider.rs:375`).

The engine is **in-process**: `MemoryWireProvider` embeds the
`memory_wire` crate as a library (not a subprocess), fusing Hindsight-style
`retain`/`recall`/`reflect` with agentmemory-style hybrid retrieval (BM25
+ token-overlap, RRF-fused). Every call boundary is a `catch_unwind` — a
memory failure is a logged miss, never a dead turn.

## The lifecycle

| Phase | What happens | Where |
|---|---|---|
| **retain** | a durable fact is written, content + context redacted server-side before the write | `memory_wire.rs` → upstream `retain` |
| **prefetch / recall** | most relevant memories for the current turn are retrieved, weighted, formatted, and queued for the next turn's `<turn_context>` | `memory.rs` prefetch + memory-wire `n` (RRF fusion, SHIPPED weights) |
| **sync_turn** | after each completed turn the loop auto-writes the turn's durable content back to the store | `agent/turn_finalizer.rs` |
| **reflect** | answer a question from the bank citing its top hit | upstream `reflect` |

The store is a bank-isolated SQLite database under the operant home
(`~/.operant/memory_wire.sqlite`, personal bank) — resolved through the
one-home resolver (see [config.md](config.md)).

## Retrieval (memory-wire `n`, in-process)

Hybrid scoring lives in the memory-wire engine itself —
`svc.n(&bank, query, …, &FusionWeights::SHIPPED)` — fused BM25 +
token-overlap via RRF, dispatched through `memory_wire.rs`'s
`catch_unwind` seam. The historical defect the fusion exists for: the
only real recall path iterated `results.take(10)` in whatever order the
store returned them, and a relevant memory could lose to an arbitrary
row. A local `retrieval.rs` (1,068 lines) duplicated this with zero
qualified callers and was deleted in iter-1420
(`docs/audit/2026-10-27-ponytail-audit-memory-compat-and-dead-code.md`);
the engine path above is the only live retrieval.

## Agent-facing tools

The live wire pair (registered by `register_memory_tools` in
`tools/memory_tools.rs`; RT-192's consolidated toolset):

- **`memory_store`** — store a fact (`key` and `blockType` are kept as
  tags; write bounds refuse oversized writes, KB-12). The retired
  `memory_save` shape (content-only, no key) still stores through the
  registry alias, and its `concepts` argument arrives as tags rather
  than being dropped (pinned iter-1339/1345).
- **`memory_search`** — ranked hybrid recall. The retired
  `memory_recall`/`memory_smart_search` names resolve to it through the
  registry's alias layer (`tools.rs::builtin_tool_aliases` — zero
  schema cost; the wire carries neither). Compat fields are
  `#[schemars(skip)]`-hidden: accepted, never advertised (iter-1326).
  The alias window closed with v0.3.0; removal is deferred one release
  (RT-192 row — pinned prompts need a migration window).

`memory_not_a_thing` is the refusal tool name for tests.

## One-time import (`memory_import.rs`)

On first start after the upgrade, if `MEMORY.md` / `USER.md` exist in the
operant home, every block is parsed with the same §-format parser that
wrote them and retained into the default bank tagged
`[imported, <block_type>]`; the files are then renamed `*.imported` (never
renamed on a failed retain).

## Operator surface

`operant memory` (`cmd_memory.rs`) reads and writes the SAME engine the
agent recalls from (`MemoryWireProvider::new(operant_home())`), so the
operator view and the agent view cannot drift. `operant doctor`'s
memory check is honest to the one-engine world (iter-1385): it prints
"memory-wire active (the only memory engine — in-process, no API
key)", warns that `builtin`/`disabled` leave the tools answering "No
memory backend attached", and tells the operator to delete any
retired `[memory] provider` spelling. The old `MemoryManager`/
`MEMORY.md` store the CLI once wrote was a store the agent never read —
that split is the documented reason the CLI now opens the engine
directly.

## Intention

Memory exists to make the agent better *across sessions*, not inside one.
Everything about the design follows from three commitments: (1) one
engine, so operator and agent share one truth; (2) in-process with a panic
boundary, so memory can never kill a turn; (3) derived counts and honest
measurements — the wire-token budget work (RT-192) happened because the
consolidation claimed savings the live capture did not support. History:
the provider deletions and the plan-11C3 consolidation are recorded in
`docs/CHANGELOG.md` and the BUGS.md ledger.
