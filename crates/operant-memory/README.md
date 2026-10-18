# operant-memory

Memory backends, embeddings, consolidation, and retrieval for Operant.

## Purpose

The memory subsystem (`src/lib.rs`). It includes backends (`sqlite.rs`, `postgres.rs`, `qdrant.rs`, `markdown.rs`, `lucid.rs`, `agentmemory.rs`, `none.rs`), embeddings (`embeddings.rs`), consolidation (`consolidation.rs`), decay (`decay.rs`), hygiene (`hygiene.rs`), a knowledge graph (`knowledge_graph.rs`), response caching (`response_cache.rs`), and retrieval (`retrieval.rs`).

## Configuration

- Cargo feature `memory-postgres` enables the `postgres` dependency and the Postgres backend.
- The `agentmemory` backend version is set with `[memory] agentmemory_version` or the `AGENTMEMORY_VERSION` environment variable (see `operant-config/src/lib.rs`).

## Extension points

- `EmbeddingProvider` trait in `src/embeddings.rs`.
- `Memory` trait from `operant-api` (`memory_traits.rs`) is the interface backends implement. See `src/backend.rs` for the backend layer.

## Limitations

- The Postgres backend is only built with `memory-postgres`.
- The crate docs give no capacity or performance figures.

## Model experience

Memories stored under the reserved key prefixes `assistant_resp` / `assistant_resp_*` and `user_msg` / `user_msg_*` are excluded from context assembly by all three context-building paths (`src/lib.rs`). Channel-scoped keys such as `telegram_user_msg_*` are not filtered. Do not use these prefixes for memories that should reach the agent's context. For the retrieval path, read `src/retrieval.rs`.
