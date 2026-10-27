# Outline-plan: executing the remaining implementation gaps identified in the memory-wire plan validation rounds (iters 1350-1380)

- Date: 2026-10-27
- Status: proposed
- Origin: deep-validation rounds over `plan-2026-10-26-memory-wire-lifecycle-hooks-tool-surface-consolidation.md` (RT-191/192/193 all closed and shipped in v0.3.0), plus the fleet/remaining-gaps board at tip `ac92db08`
- Scope: the gaps found VALIDATING that work, not the work itself. Every claim below was re-probed at the current tip on 2026-10-27, not carried forward from an earlier note.

## What this plan is for

The memory-wire plan is finished and released. What its validation rounds produced is a
second, smaller set of gaps — several in my own prior "closed" work — that are not yet
tracked as executable items. This is the outline for executing them, in an order that
does not let a release-blocked item block the rest.

## The gate that decides the order

**No release later than v0.3.0 exists** (verified `gh release list`, v0.3.0 Latest
2026-10-26T21:44:28Z). The alias-removal scope is therefore correct to keep blocked, and
the outline below deliberately front-loads everything that needs no release.

## Phase 0 — release-independent, highest value, do now

### 0.1 Shipped docs teach a memory surface that does not exist (peer-owned)

Owner is `feat(skills)` / the docs slice, not this plan's author — reported (F0077/F0079),
tracked here so it is not lost. All counts verified live at the current tip:

| file | still teaches the retired names | state |
|---|---|---|
| `skills/autonomous-ai-agents/operant/references/skills.md:27` | lists all five as the Memory toolset | **wrong for every user today** |
| `skills/autonomous-ai-agents/operant/references/configuration.md:9` | declares `agentmemory` the **default** provider, "auto-spawns via npx" | **wrong for every user today** |
| `skills/autonomous-ai-agents/operant/references/configuration.md:19-20` | names `memory_recall` under the `builtin` provider | **wrong for every user today** |
| `docs/library/memory.md:78,82` | `memory_save`/`memory_recall` backward-compat prose | correct **today**, must move with the removal |

The independent defect (needs no release, no alias decision): **the shipped configuration
doc tells the operator to set a provider that is a legacy name.** `MemorySettings::default()`
resolves `provider: "memory-wire"` (`config.rs:1803`) and `operant.example.toml:521` agrees;
the doc still names `agentmemory` as the default and describes an `npx @agentmemory/mcp`
server that the tree removed (memory_provider REST/npx/MCP gone — the name is *accepted*
and downgrades, it is not a hard error, which is exactly why it misleads silently).
Replacement text is already written out verbatim in F0079.

**Acceptance:** all three present-tense claims corrected against source in one change;
the two compat sentences in `memory.md` left until the removal phase below.

### 0.2 RT-194 landed — re-verify the deferral it was justified by

The peer shipped the 23 composites riding the deferred bridge
(`feat(iter-1379)` / `698b2e63`), which closes board row `remaining-gaps-1364-004` ("done").
Verified in code: `sourcehound_composites::composite_names()` is chained into
`builtin_tool_names` (`builtin.rs`, 1 occurrence) and the composites file is 850 lines.

**Acceptance:** the measured saving claim (~4781 tok/request in the board, ~760 tok on the
turn that researches) re-measured from a live `tools/list` on the deployed binary, because
the board number was estimated before the composites registered. This is a number to
*confirm or correct*, not to trust.

## Phase 1 — release-gated, fully scoped, execute when a release lands

### 1.1 The alias removal (RT-192's contract-ed drop)

Scope is complete **by symbol** and audited against **each of the alias map's three keys**
(`builtin_tool_aliases`, `tools.rs`, keys `memory_recall` → `memory_search`, `memory_save`
→ `memory_store`, `memory_smart_search` → `memory_search`):

- the alias machinery: `builtin_tool_aliases`, `resolve_alias` + its three call sites
  (`get`/`execute`/`is_available`), the `aliases` field and its `Clone`
- `OBSERVE_ALLOW`'s `memory_recall` entry (`operant-os/src/policy.rs` — names it explicitly;
  `native_tool_names()` provably cannot supply it, so the seat currently admits a name the
  registry resolves *only* through the alias map)
- the `operant-config` readonly test asserting `memory_recall` is readable
- flip `retired_memory_names_still_execute_through_the_alias_layer` to assert the retired
  names **no longer** resolve (contract flips, does not disappear)
- **two live provider code paths, not tests:** `memory_wire.rs::handle_tool_call`'s
  `memory_smart_search` arm (query+limit, default 5, clamped 1-20) **and** its
  `memory_save` arm (content required, concepts → tags)
- **one parser surface:** `parser.rs`'s `known_tools` bare-word list contains
  `memory_recall` — leaving it recreates the same policy-allowed-but-unresolvable class
  in the parser
- **one deliberate decision, not a break:** `turn_summary.rs`'s `VERB_GROUPS` lists
  `memory_save` for the "updated memory/memories" verb. A cosmetic classification; decide
  retain-or-delete explicitly so it is not mistaken for an oversight
- the two compat sentences in `docs/library/memory.md` (§0.1)

**Standing blind spot, recorded honestly:** a site naming none of the three bare literals
(a dynamically-built string, a config value, an indirect seat mapping) is invisible to
every search performed so far. So: re-grep each key at execution time, RED-first, and
re-read line numbers (the peer's RT-194 work shifted `tools.rs`/`builtin.rs` again).

## Phase 2 — the tracker rows that remain open (owned elsewhere, tracked so they are not lost)

Seven rows are open in `docs/audit/2026-06-19-BUGS.md` at this tip. None is this plan's
to close; they are listed so the outline covers the full remaining picture, with the
board's own status:

| row | what it is | board |
|---|---|---|
| RT-190 | memory bank topology — the store is a second disconnected bank family | `005` open, owner ruling (c) document-boundary now, (b) bridge later |
| RT-179 | TUI functional core has no channel twin (the peer just shipped `/effort` + `/plan` twins, `ac92db08` — **row likely partially stale, re-read before acting**) | `001` open |
| RT-195 | `aft_search`'s description promises a lexical fallback that does not exist; both index lanes unobserved | `002` in-progress (claimed by this author) |
| RT-196 | smoke report claims all tools exercised; 22 were never called | `003` open |
| RT-135 | `operant cron run <id>` marks triggered, never runs | accepted documented gap |
| RT-136 | `[pk]`/`[socialization]` parsed and reported, nothing reads them | confirmed-not-a-defect |
| S2 | ambiguous/wide glyph widths, WIDE (emoji) class still unproven | `006` open |

## Verification posture for every phase

- RED-first: a test that fails for the stated reason before the fix, cited in the row.
- Live-probe any number that gets quoted (the §0.2 measurement, the wire bytes).
- Deploy + md5-verify for code changes; docs-only skips deploy.
- Peer-owned paths are reported through the feed, never edited from this slice.
