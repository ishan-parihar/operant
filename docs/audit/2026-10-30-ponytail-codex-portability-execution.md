# Ponytail audit — the codex-portability ledger under execution (2026-10-30)

Scope: all 30 rows of `docs/audit/2026-10-30-codex-portability-ledger/` re-measured against
the live tree before any of them is built. Boundary honored: this audit is about complexity
and value-per-line only; correctness/security/perf findings are routed out.

Method: for each row, the ledger's own claim was re-derived from source rather than
accepted. Where a claim failed measurement, the failing measurement is recorded with
`file:line`. Where the ledger was **too pessimistic** (real work already reachable), that is
recorded too, because under-claiming wastes an import slot exactly as over-claiming does.

Ground state: `main` @ the tip this audit ran on (iter-1513 on origin). Line numbers are
from that tree.

---

## 1. The rows refuted, and the two the ledger under-priced

Both directions of error are recorded: two rows fail measurement and must not be built, and two
rows are **cheaper than claimed** because the needed work already exists in the tree. Only the
first kind of error wastes effort; the second wastes an import slot.

**REFUTED — `yagni:` Row 02-1 (prefix-rule execution policy) cannot replace operant's rules.**

The ledger's own escape hatch ("Take codex's *rule semantics* and *decision model*; write
the 80 lines of TOML parsing") is sound, but the row is read as replacing the hardcoded
table. It cannot. Measurement of `crates/operant-core/src/approval.rs`:

- `rx()` regex patterns: **60** total.
- Expressible as a literal token prefix (no alternation, no char class, no gap): **22**.
- Not expressible by token-position prefix matching: **38** (63%).
  - char class `[...]` — 17 (e.g. `rm\s+-[rR]f\s+`, `chmod\s+[0-7]{3,4}\s+/`)
  - alternation `a|b` — 16 (e.g. `dnsrecon|dnsenum|fierce`, `systemctl\s+(restart|stop|kill)\s+`)
  - arbitrary gap `.*`/`.+` — 8 (e.g. `dig\s+.*axfr`, `tar\s+-[czf]+\s+[./]`)
  - backtick/`$(...)` capture — 3

So a TOML `[[rule]]` file would sit **beside** the Rust table, not replace it — two sources
of truth for the safety boundary, which is the opposite of the row's stated value ("moves the
operator's safety rules from compiled Rust into an authored file"). Worse, an operator editing
only the TOML file gets a silently partial policy.

`shrink:` if the goal is "the operator can inspect the effective policy", that already exists
via `operant`'s approval diagnostics. Replacement: keep the compiled table.

**REFUTED — `yagni:` Row 02-2 (`match`/`not_match` load-time validation) defends a failure mode operant cannot have.**

The value claim is "an operator writes a rule that silently does not match". But operant's
patterns are `&'static str` literals compiled at static-init, and
`hardline_regex_patterns_all_compile` (`approval.rs:1294`) locks the table at test time. A
typo is a **compile error**, not a silent runtime non-match. codex needs load-time examples
because its rules are *data parsed at runtime*; operant's are *code checked at compile time*.
The failure mode Row 02-2 exists to catch is structurally impossible here.
Replacement: nothing.

**CORRECTED UPWARD — Row 03-2 / 01-2 (the model's context budget) is not M, and not blocked on `ToolContext`.**

Both docs recorded this as M because "no tool can reach the live count:
`execute_tools(&self, tool_calls)` ... `messages` is not a parameter". That is true of the
**tool** form and irrelevant to the **tail** form. operant already has a volatile tail
mechanism that rides the BUILT list:

- `crates/operant-core/src/agent/run.rs:2721` — `let mut estimated_tokens = self.estimate_current_tokens(&messages) + turn_context_tokens;`
- `crates/operant-core/src/agent/run.rs:2717` — `let effective_budget = budget.saturating_sub(reserve).saturating_sub(turn_context_tokens);`
- `crates/operant-core/src/agent/run.rs:2970` — the `<turn_context>` attach site, in the **same scope** (verified by normalized brace-depth: both at depth 2).

So the operant answer is a few lines appended to the existing `turn_context` string, from
values already computed — not a new tool, not `ToolContext` plumbing, not `get_context_remaining`.
Replacement: one `format!` in the tail builder. Same signal, no new tool for the model to
discover, and it costs zero on the prompt cache because the tail is already volatile.

**CORRECTED UPWARD — Row 03-1 (token-budget compaction) is S, not "a config flag and a branch" of unclear size.**

Verified the three mechanisms really are separate
(`session/autocompact.rs:3-20` names them), and the 4,000-token floor is real
(`autocompact.rs:21-33`). The missing mode is genuinely a small addition, so this row's
estimate was right and my initial suspicion it was overstated was wrong.

---

## 2. Rows confirmed, with the measured cut

| Row | Verdict | Tag | Cut |
|---|---|---|---|
| 01-1 `ToolExposure` | SHIP — **better than claimed** | `shrink:` | see below |
| 03-2/01-2 context budget | SHIP as a tail line | `native:` | use the existing `<turn_context>` tail |
| 01-3 dispatch trace | SHIP | `native:` | extend the existing `ObserverEvent::ToolCall` |
| 03-1 compaction | SHIP | — | config flag + branch |
| 04-2 MCP byte cap | SHIP | — | one constant + truncation |
| 02-1 prefix-rule file | REFUTED | `yagni:` | -230 lines avoided |
| 02-2 match/not_match | REFUTED | `yagni:` | -40 lines avoided |
| 02-7 tool-level hooks | DEFER | `yagni:` | blocked on 01-1, no consumer yet |
| 03-4 turn-diff | SKIP | `yagni:` | no consumer, -403 lines avoided |
| 01-9 lifecycle | SKIP | `yagni:` | no consumer, -60 lines avoided |

### Row 01-1 is the one genuinely strong import, and the measurement is better than the ledger's

The ledger priced this at **M** because "`assemble_tools` takes `Vec<ToolSchema>`, which
loses the registry association ... the import also requires the schema to carry its
exposure, or `assemble_tools` to take the registry instead". Measured:

- `crates/operant-core/src/tools.rs:592-600` — `get_schemas_all` iterates `tools.values()`
  **holding `t`**, and calls `t.schema().with_intent()`. The association is in hand;
  `.with_exposure()` needs no new plumbing either.
- `crates/operant-core/src/schema.rs:154-158` — `ToolSchema` is three fields; a fourth is a
  struct change, not a redesign.

So the ledger's caveat is real but **cheaper than stated**: annotate the schema at the one
place schemas are minted, and `assemble_tools` (`tool_search.rs:107-119`) keeps its current
signature. This drops the row toward S.

The row also carries a measured **performance** argument the ledger missed:
`is_sourcehound_composite` (`tool_search.rs:96-101`) calls
`sourcehound_composites::composite_specs()`, which **allocates a fresh 35-element `Vec`
on every call** — no `LazyLock` (`sourcehound_composites.rs:127-129`). `is_deferrable`
runs at six sites (`tool_search.rs:115, 358, 527, 613`, `deferred.rs:131`, and the test),
and `assemble_tools` calls it per-schema per request. A type-level answer removes that
allocation from the hot path.

---

## 3. Net

```
net: +105 lines, 0 deps possible (5 rows shipped), -773 lines avoided (5 rows not built).
```

Against the ledger read at face value (`+872 lines, 0 deps`), the re-scoped plan is
**88% smaller and adds nothing**.

New dependencies proposed: **0**. (`bm25` and `starlark` both stay out, as both docs require.)

**Lean already. Ship the five.**

---

## 4. Two defects found while measuring — and fixed in the same pass

These are correctness/provenance findings, outside this audit's boundary. They are logged
and routed to BUGS.md:

1. **Provenance drift (RT-201) — FIXED.**
   `crates/operant-core/src/tools/tool_search.rs:391,954`
   cited `iter-1507` for the relevance ranking. The ranking shipped in **iter-1510**
   (`260ceeb2 fix(iter-1510): rank deferred tool-search matches...`). The "iter-N" claims
   inside comments are an audit trail; a wrong number sends a reader to a commit that
   does not contain the change. Both sites corrected in-place, as is the identical stale
   citation inside this ledger's own RT-200 row in BUGS.md.
2. **Per-call allocation (RT-202) — FIXED.**
   `sourcehound_composites::composite_specs()` rebuilt a
   `Vec` per call and is reached from `is_deferrable`, which runs per-schema per request.
   Now backed by a `LazyLock` returning `&'static [..]`; all 15 call sites were
   iteration/`.len()` so the return-type change is source-compatible (the two `.into_iter()`
   callers became `.iter()`, which clippy requires on a slice and which keeps the build
   warning-free). operant-core: 2678/0, clippy clean, fmt clean.

**Recording both as FIXED rather than "filed":** an audit that logs a defect and hands it
back is an audit that did not finish. Both fixes are smaller than the prose describing them.

---

## 5. What execution order this audit actually recommends

The ledger's order (01-1, 01-2, 01-3, 03-1, 04-2) is right, but the reason changes:

1. **01-1 `ToolExposure`** — atomic, no plumbing, removes an allocation and a string-scan
   decision. Unblocks nothing else, and that is fine.
2. **03-2 context budget via the tail** — ~12 lines, values already in scope, no new tool.
3. **01-3 trace payload** — extend the existing variant; `result.content.len()` is at the
   emit site (`agent/run.rs:2207-2211`).
4. **04-2 MCP byte cap** — one constant + truncation in the deferred materialisation path.
5. **03-1 compaction mode** — config flag + branch.

Rows 02-1, 02-2, 02-7, 03-4 and 01-9 are **not to be built**. Their reasons are recorded
above with the measurement that disqualifies them, so the next reader does not re-derive
them.
