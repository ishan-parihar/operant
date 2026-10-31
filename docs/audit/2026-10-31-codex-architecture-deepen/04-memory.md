# Family 04 — memory: deterministic store vs LLM extraction pipeline

**Question this file answers:** codex's memory subsystem is a **two-phase LLM pipeline that runs
autonomously at startup**. operant's is a **deterministic in-process store with no model call**.
Which properties does each have that the other lacks, and does any of it earn an import?

**Ground state:** operant `main` @ `f53907e8`. Reference `../external-repos/codex/codex-rs` @ `588a277b`.

---

## CODEX-SIDE

### What the mechanism is

codex's memory is **LLM-driven extraction in two phases**, launched automatically, reading the
session's own rollout transcript as evidence.

**Write path** (`memories/write/`, ~4,400 LOC of production code):

- `phase1.rs` (812 LOC) builds a prompt (`build_stage_one_input_message`), calls the model, and
  parses a **structured** result (`phase1_output::output_schema`, `StageOneOutput`). It carries its
  own metrics: `MEMORY_PHASE_ONE_E2E_MS`, `MEMORY_PHASE_ONE_JOBS`, `MEMORY_PHASE_ONE_OUTPUT`,
  `MEMORY_PHASE_ONE_TOKEN_USAGE` — i.e. **each phase's token spend is measured**, because it is a
  model call and a real cost.
- `phase2.rs` (632 LOC) is the second pass.
- `rollout_input.rs` (376 LOC) **reads the session's own rollout** as evidence:
  `MAX_ROLLOUT_BYTES = 8 MiB`, `MAX_RECORD_BYTES = 1 MiB` (`:26-27`), redaction through
  `codex_secrets::redact_secrets`, media replaced with placeholders, and a reverse JSONL scanner so
  it reads the most recent evidence first without loading the whole file.
- `runtime.rs` (475 LOC) is the startup context; `start.rs` (95) is the entry.
- `storage.rs` (242) writes the result; `workspace.rs` (233) scopes it; `control.rs` (117) and
  `extensions/prune.rs` (100) handle enable/disable and eviction.

**Read path** (`memories/read/`, ~250 LOC):

- `citations.rs` (85) parses a `MemoryCitation` out of the model's own output: it extracts
  `<citation_entries>` blocks and rollout-id blocks (`:7-40`). The **model emits the citation** and
  the code parses it back — there is no retrieval-time "which memory did this come from" join.
- `usage.rs` records which memories were actually used.

### The load-bearing units (file:line + LOC)

| unit | file:line | LOC | what it owns |
|---|---|---|---|
| phase 1 (extraction) | `memories/write/src/phase1.rs` | 812 | prompt + model call + schema parse + metrics |
| phase 2 | `memories/write/src/phase2.rs` | 632 | second extraction pass |
| rollout evidence reader | `memories/write/src/rollout_input.rs` | 376 | reverse-scan + redaction + budgets |
| runtime / startup | `memories/write/src/runtime.rs`, `start.rs` | 570 | the trigger context |
| storage / workspace | `memories/write/src/storage.rs`, `workspace.rs` | 475 | persistence + scoping |
| control / prune | `memories/write/src/control.rs`, `extensions/prune.rs` | 217 | enable + eviction |
| citation parser | `memories/read/src/citations.rs` | 85 | parse `<citation_entries>` from output |
| usage recorder | `memories/read/src/usage.rs` | — | which memories were used |

---

## OPERANT-SIDE

### What exists today

**The provider is a trait with a shipped in-process implementation and a zero-dep fallback.**
`crates/operant-core/src/memory_provider.rs` defines `MemoryProvider` with defaulted
`prefetch` (returns empty), `sync_turn` (no-op), `queue_prefetch`, `on_pre_compress`,
`on_memory_write` (`:277-305`). The tracing/metrics around it is deliberate: `runtime_metrics.rs`
counts `sync_turn` failures, dropped jobs, and memory-sync failures
(`runtime_metrics.rs:32`, `:131`), and `MemorySyncExecutor` (`memory_provider.rs:94-110`) drains a
**bounded channel** and drops overflow with a warning rather than blocking the turn.

`crates/operant-core/src/memory_wire.rs` (the default, cargo git-dependency on
`ishan-parihar/memory-wire`) implements it as **direct store calls with no model involvement**:

```rust
// memory_wire.rs:663-666
async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
    self.retain_turn(user, assistant);
    Ok(())
}
```

and `prefetch` (`memory_wire.rs:618-644`) drains a queued-query backlog into a fused recall:

```rust
let queued: Vec<String> = self.prefetch_queue.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
let mut text = self.recall_text(query, 5);
for q in queued { let extra = self.recall_text(&q, 3); /* append */ }
```

`recall_text` is `recall_with_weights` — the module doc (`memory_wire.rs:19`) names it as
"SHIPPED fusion". A **`catch_unwind` boundary** wraps the whole thing (`:33-35`): "degrades a
panic (or an `Err`) to a *memory miss* — an empty prefetch or a no-op sync", which is exactly the
repo's degrade-don't-die doctrine at a subsystem seam.

`BuiltinProvider` (file-backed MEMORY.md/USER.md) is the zero-dependency fallback, and unknown
provider names silently downgrade to it.

### The gap, stated precisely

| property | codex | operant |
|---|---|---|
| write trigger | automatic, at session start | per completed turn (`sync_turn`) |
| write mechanism | **two LLM calls** with a schema | **zero model calls**, direct retain |
| extraction quality | LLM-selected, summarized | verbatim turn text |
| read | model emits citations, code parses them | `recall_with_weights` fusion, `[memory]` block on the tail |
| cost | measured per phase (`MEMORY_PHASE_ONE_TOKEN_USAGE`) | none — this is the whole point |
| determinism | non-deterministic (LLM) | deterministic |
| evidence source | the session's own rollout transcript, reverse-scanned with 8 MiB/1 MiB caps | the turn's own two strings |
| "which memory was used" | the model asserts it | not tracked |
| failure posture | phase failure logged, `JobOutcome::Failed` | channel-full → drop + warn; panic → miss |

**The honest reading: these are two different systems that both answer "what does the agent
remember".**

- codex buys **curation** (a distilled memory, possibly better than raw turns) and pays
  **nondeterminism + token cost + startup latency**.
- operant buys **determinism, zero cost, and no startup model call** and pays with **raw, uncured
  retention** — every turn is stored as written.

Neither is obviously better. codex's design is the right one for a product whose value is a
long-lived curated personal memory. operant's is the right one for an operator who does not want
the agent spending tokens and minutes at startup deciding what to remember.

---

## VERDICT

| mechanism | codex unit + LOC | operant equivalent | gap severity | portable? | verdict |
|---|---|---|---|---|---|
| LLM extraction phase 1 | `phase1.rs` 812 | none, deliberate | — | no | **DO-NOT** — a deliberate design divergence; adopting it costs tokens and determinism |
| LLM extraction phase 2 | `phase2.rs` 632 | none, deliberate | — | no | **DO-NOT** (same) |
| Rollout evidence reader (reverse scan + redaction + caps) | `rollout_input.rs` 376 | none | low | no — coupled to codex's rollout format | **DO-NOT** — operant's evidence is the turn itself, and the reader would be a pure cost here |
| Citation parse (`<citation_entries>`) | `citations.rs` 85 | none | low | yes | **DO-NOT** — the parse is trivial; the value is in the model emitting it, which needs a prompt change |
| Per-phase token-usage metrics | `phase1.rs` metrics | none | none | n/a | **N/A** — there is no model call to measure |
| Deterministic retain + fusion recall | — | `memory_wire.rs:618-666`, `recall_with_weights` | operant ahead | — | **ALREADY-HAVE** |
| Degrade-don't-die at the seam | — | `memory_wire.rs:33-35` `catch_unwind`; `memory_provider.rs:94-110` bounded channel | operant ahead | — | **ALREADY-HAVE** |
| Zero-dependency fallback | — | `BuiltinProvider` | operant ahead | — | **ALREADY-HAVE** |

**The ponytail reading: zero imports from this family, and that is the finding.** The two designs
disagree on a *premise* — "the agent should decide what is worth remembering" — and premises are
not portable. Every row that would transfer is either trivially small and valueless without its
prompt change (citations), or coupled to a data format operant does not have (rollout reader), or
the thing that would actively make operant worse (LLM extraction).

This row belongs in the ledger precisely so that the next agent does not read "codex has a 6,548-LOC
memory crate" as a gap to close. **The 6,548 LOC is the price of a different choice, not the size
of a missing feature.**
