# 2026-10-31 codex-rs architecture deep-dive — index

**Status:** dated-frozen measurement record. Supersedes nothing. Complements and deepens:

- `docs/audit/2026-10-30-codex-portability-ledger/` — which **tool-surface / safety / session /
  observability-protocol** mechanisms to import (30 rows, 4 families). Still current.
- `docs/audit/2026-10-30-codex-architectural-discipline-contrast-*.md` — enforcement, crate
  structure, test discipline. Still current.
- `docs/plans/2026-10-31-codex-portability-remaining-gaps-*.md` — the build list for that ledger's
  residue (**4 rows shipped, 2 refuted with measurement**).

**Ground state:** operant `main` @ `f53907e8` (2026-10-31).
**Reference tree:** `../external-repos/codex/codex-rs` @ `588a277b`, 122 crates, 2,202,563 Rust LOC.

**What THIS audit is:** the codex surface the earlier audits did **not** visit. The portability
ledger audited four families and closed them; this one opens the eight families nobody had measured
against operant, and reports both directions — including the four places where **operant is ahead
and importing would make it worse**.

## The bar (ponytail), stated once

A mechanism earns an import row only when:

1. it closes a gap operant **measurably** has, and
2. the portable unit is small enough that copying beats rebuilding, and
3. its dependencies are ones operant already carries (or zero).

"Codex has a 6,548-LOC crate" is not a gap. It is sometimes a different design decision, sometimes
a 1,498-LOC subsystem wearing a 10,215-LOC costume. Every row below is sized by `wc -l` on the
named unit, and every DO-NOT carries the measurement that disqualifies it.

## Files

| # | File | Family | Rows | Headline |
|---|---|---|---|---|
| 00 | this file | the ranked shortlist | — | — |
| 01 | [01-hooks.md](01-hooks.md) | hooks / extension | 12 | **operant hooks can log; codex hooks can decide.** 0 of 8 decision channels |
| 02 | [02-command-safety.md](02-command-safety.md) | shell safety | 6 | **the interpreter unwrap is the real hole**, not the literal patterns |
| 03 | [03-skills.md](03-skills.md) | skills | 7 | operant is **ahead** on provenance + guarding; one 15-LOC struct worth taking |
| 04 | [04-memory.md](04-memory.md) | memory / state | 8 | **zero imports** — a premise divergence, not a gap |
| 05 | [05-file-editing.md](05-file-editing.md) | file editing | 9 | seek_sequence + text_file + compute_replacements = ~350 LOC, zero deps |
| 06 | [06-terminal-diagram-rendering.md](06-terminal-diagram-rendering.md) | mermaid | 4 | **1,498 LOC, zero new deps** — and the W9 cut was of a 6.8× bigger thing |
| 07 | [07-process-hardening.md](07-process-hardening.md) | process hardening | 6 | core dumps carry the env, the env carries the API keys |
| 08 | [08-observability-attribution.md](08-observability-attribution.md) | turn attribution | 6 | exclusive attribution + the `Incomplete` state RT-199 needed |

---

## The shortlist — ranked by value-per-line

Ordered by what the operator gets for what it costs. Every LOC is the portable unit.

| # | Mechanism | Codex source | LOC | operant cost | Why it is here |
|---|---|---|---|---|---|
| 1 | **Hook output decision contract** | `hooks/src/engine/output_parser.rs:1` | 617 (use ~250) | ~250 | operant hooks can **only observe**. block / updated_input / additional_context / permission-answer are 0 of 8. This turns hooks from a logger into a policy surface. |
| 2 | **Hook stdin JSON + matcher** | `hooks/src/engine/matcher.rs:1` (49) + per-event input structs | 49 + ~150 | ~200 | a hook cannot decide about a tool it never hears the name of; and every hook fires on everything today |
| 3 | **Mermaid terminal renderer** | `mermaid/` (non-test) | **1,498** | ~1,600 + tests | `unicode-width` already a dep. The gate `native_image_protocol_available() -> false` is the only blocker; the branch sites exist |
| 4 | **Interpreter unwrap for command safety** | `shell-command/src/bash.rs:106-119` | **14** | ~20 | closes the one hole `blast_radius.rs:22` names and codex closes. `bash -c "rm -rf /etc"` rates `None` today |
| 5 | **Patch seek_sequence + text_file** | `apply-patch/src/seek_sequence.rs` (164) + `text_file.rs` (121) | 285 | ~285 | pure functions, **zero imports**, port as-is. Fixes 4-tier fuzzy match, EOF anchor, CRLF fidelity |
| 6 | **compute_replacements** | `apply-patch/src/file_update.rs:60-160` | ~120 core | ~120 | the native multi-chunk engine behind the patch tool, so the aft bridge dying does not collapse editing to one whole-file replace |
| 7 | **`RLIMIT_CORE=0` + `PR_SET_DUMPABLE=0`** | `process-hardening/src/lib.rs:65-75`, `:99-115` | **28** | ~45 | `libc` already a dep. A core dump is a credential file |
| 8 | **Exclusive turn attribution** | `utils/critical-path/src/lib.rs:1` + `turn_timing/report.rs:4-25` | 181 | ~60 | answers "why did this turn take 40s". The `Incomplete` state is worth more than the arithmetic (RT-199) |
| 9 | **Hook event vocabulary** | `hooks/src/lib.rs:29-44` | ~40 | ~40 | 4 of 14 events; the missing ten are all tool/prompt/compact/model events |
| 10 | **`sudo` / `env` wrapper unwrap** | `is_dangerous_command.rs:146-155` | ~20 | ~20 | pure string work, no deps |
| 11 | **Skill tool dependencies** | `skills/src/model.rs:81-95` | **15** | ~40 | one frontmatter field wired to an `is_available()` that already exists |
| 12 | **`hooks.json` discovery** | `hooks/src/engine/discovery.rs:390-451` | ~60 | ~60 | single source, skip the layer/plugin machinery |

**Total: ~2,900 LOC of operant change across 12 rows, zero new production dependencies.**

For contrast: the same 122-crate reference tree contains `core/` (483,438) and `tui/` (475,508),
and the naive reading of "adopt codex" would be 2.2M LOC. The shortlist above is **0.13% of the
reference tree**, chosen by measurement.

---

## Rows where operant is AHEAD — importing would make it worse

These are recorded with the same care as the imports, because a false gap is more expensive than no
finding. Each was verified, not assumed.

| operant advantage | codex's version | why operant's wins | source |
|---|---|---|---|
| **Atomic per-file write with mode preservation** | `write_file_with_missing_parent_retry` (`apply-patch/src/lib.rs:765-818`) — plain `fs::write`, no temp, no rename, no mode | a crash mid-write cannot corrupt or leak a partial file | `file_tools.rs:65-108` |
| **Skill provenance + guarding** | no origin concept | `SkillOrigin::{OperatorPool, Integration}` refuses to delete vendor skills; `skills_guard.rs` is 2,675 LOC of policy | `skills.rs:35-41` |
| **Deterministic, zero-cost memory** | two LLM calls per session, measured token spend | no model call, no startup latency, no nondeterminism, `catch_unwind` degrade | `memory_wire.rs:33-35`, `:663-666` |
| **Isolated checkpoint store** | baseline commits in the user's own repo | shadow git store via `GIT_DIR`/`GIT_WORK_TREE`, never touches the operator's repo state | `tools/checkpoint_tool.rs:230-240` |
| **Terminal image pinning** | text-only diagrams | `pinned_images.rs` re-emits graphics every frame | `tui/pinned_images.rs:1-40` |
| **Boundary invariants as failing tests** | lint-level discipline only | `one_home_resolver.rs`, `dead_modules_stay_dead.rs`, `no_retired_scaffold_gate.rs` | `crates/operant-core/tests/` |

---

## The four DO-NOT families, with the disqualifying measurement

| Family | Codex size | Disqualifier |
|---|---|---|
| **Memory extraction** | `memories/` 6,548 | adopts token cost + nondeterminism to buy curation operant does not want. A **premise divergence**, not a gap |
| **Realtime voice / WebRTC** | `realtime-webrtc` 2,105 + `voice-host` 6,184 | operant's interactive voice state machine was **deleted in iter-1420 with zero callers** (`docs/library/voice.md`). Building a consumer for it is a new product decision |
| **Connectors / app catalog** | `connectors/` 5,491 | an account-scoped remote-app catalog with its own runtime. Operant's MCP client (`mcp.rs`, 3 transports) covers the actual need |
| **tree-sitter shell analysis** | 3 deps (`tree-sitter`, `-bash`, `-powershell`) | three dependencies to close a boundary a 14-line `shlex`-free unwrap closes for the common cases. Refuse what cannot be unwrapped instead |
| **Code-mode / V8** | `code-mode-runtime` 12,321 | operant emits no TypeScript. (Carried from the prior ledger; unchanged.) |
| **App-server protocol tier** | 277,376 across 4 crates | operant is a single-process binary; there is no client to serve |

---

## Defects found while measuring (not feature gaps)

These are bugs or near-bugs the contrast surfaced. Each is a standalone row, none requires an
import.

| # | Defect | Where | Severity | Note |
|---|---|---|---|---|
| D1 | `content.replace()` rewrites **every** occurrence of `find`, with no count check or occurrence index, so a model asking for one edit can rewrite five | `tools/patch_tool.rs:172-186` | med | the `exact_count` is computed, then used only as a boolean |
| D2 | a no-op branch that does nothing | `tools/patch_tool.rs:95-99` | low | `if replace.is_empty() { if !result_lines.is_empty() \|\| i > 0 { /* already handled */ } }` |
| D3 | the mermaid gate is a permanently-false function, so the entire deferred/registration apparatus is dead code that still looks live | `markdown_mermaid_fallback.rs:41`, `mod.rs:167-171` | med | 4 `RenderResult::Error` sites unreachable, `pinned_images` has no producer |
| D4 | the hardline blocklist's `rm` rules are literal strings, so a flag permutation misses layer 1 | `approval.rs:209` | low | **already mitigated** by blast_radius's token classifier at `approval.rs:662` |
| D5 | `bash -c "…"` is rated `None` by the blast-radius classifier because the verb it sees is `bash` | `blast_radius.rs:22`, `:596-624` | **high** | codex unwraps and classifies the inner script; this is row 4 of the shortlist |

## Method and honesty notes

- LOC counts are `wc -l` on the **named unit in the codex tree at `588a277b`**, not the crate.
- Every operant-side claim was read in this tree at `f53907e8`. Four swarm research legs produced
  independent measurements (hooks, memory, file-editing, observability); each claim in `01`, `05`
  and `08` was re-verified by hand before being written down, and two first-draft claims were
  corrected by that re-verification.
- **One row in this audit corrected itself during measurement and the correction is recorded rather
  than deleted:** the `LD_*` env-strip process-hardening row was first written as IMPORT-SMALL.
  Measuring `ldd target/release/operant` showed `libonnxruntime.so.1` resolving out of
  `~/.local/lib/operant/`, which means `LD_LIBRARY_PATH` is load-bearing for this binary and
  stripping it would break the Kokoro TTS default. The row is now DO-NOT, and the reasoning is in
  [07-process-hardening.md](07-process-hardening.md).
- Two rows are marked conditional and deferred to the operator rather than ranked: mermaid
  (needs the "diagrams-in-transcript are wanted" question answered — no transcript count exists in
  this repo) and hook decisions (the largest of the twelve, and the one that changes what a hook
  can *do*, which is an architecture decision not a porting one).
- The prior ledger's refutations stand: the MCP schema byte cap (the spend guard is the binding
  bound, measured), token-budget compaction (the no-model path is a **signature**, not a flag),
  prefix-rule TOML policy (38 of 60 patterns are not prefix-expressible), and tool-level hooks
  (needs the exposure type first). None is re-litigated here.
