# Close the E311 library-coverage error iter-2742 introduced and the two duplicate-owner plan pairs

**Date:** 2026-11-20
**Status:** PLANNED (frozen on mint)
**Predecessor:** `2026-11-20-close-the-three-residual-gaps-the-tests-lint-hole-the-rt-543-shari.md` (iters 2743-2745, T1 and T3 executed; T2 still open and NOT this plan's — see "What this plan does NOT do")

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan.

## Why this plan exists

I closed T6 (Hydrogen round-2 F4, the slash-command router) as iter-2742. That
commit added `crates/operant-core/src/command_router.rs` and the gate that owns
documentation coverage — `genome lint`, rule E311 — immediately went red on it:

```
E311 crates/operant-core/src/command_router.rs is named by no library page —
     cite its path or `command_router.rs`, or add it to ops.coverage_exempt with the reason
```

E311 is the *other* direction from E301: E301 asks whether every doc is listed
in the ownership map, E311 asks whether every code unit is NAMED by a doc. The
mechanism library's real failure is silent — a subsystem ships, nobody writes
its page, no gate says so. **I shipped the subsystem and left the gate red.**
That is mine to close, and it is the whole reason this plan exists.

Re-deriving the rest from the tree at `a6de67f8` (rather than trusting this
prose) found exactly one other actionable class: the two `W006` duplicate-owner
plan pairs. Everything else the gate battery reports is either a peer's owned
item or an operator ruling (both listed below so the next agent does not
re-derive them).

### The gate battery, measured at `a6de67f8`

| gate | exit | verdict |
|---|---|---|
| `bash scripts/check-doc-claims.sh` | 0 | green |
| `python3 scripts/audit-config-keys.py --check` | 0 | green |
| `python3 scripts/audit-ledger-index.py` | 0 | green (375 index rows, 3 open checkboxes — index and checkboxes agree, RG-016) |
| `bash scripts/supply-chain-gate.sh` | 0 | green |
| `bash scripts/clippy-warning-gate.sh` | 0 | **green** — the peer's iter-2744 (T1) fixed the `tests/` exemption hole |
| `./scripts/genome lint` | 2 | **1 error (E311, mine) + 6 warnings** |
| `python3 scripts/audit-pub-fn-surface.py --check` | 1 | **135 NEW** (131 after the peer's iter-2745 T3 batch) |

### The one open ledger row that is CODE, not a ruling

`RT-543`'s sharing half has no proof on the tree. Its own verdict says it is
"Pinned by: A test that the gateway and one attached client share one MCP child
set (the fake MCP server records connections in a file, so the count is on
disk)". Measured: that test does not exist. `rg -l 'session_socket|default_socket_path|run_session_socket' crates/*/tests/`
returns nothing, and no fake MCP server records connections to a file anywhere
under `crates/*/tests/`. RT-544's closure (iter-2662) leans on the socket-sharing
claim, so an absent proof there is not cosmetic.

**This is the peer's T2, still unstarted.** It is listed under "What this plan
does NOT do", not scheduled here — see the ownership note below.

## The items, in measured cost order

### T1 — E311: cite `command_router.rs` in the mechanism library (Severity 2, MINE)

**The claim.** `genome lint` exit 2 on one error, naming my own iter-2742 module.

**What is actually true.** The mechanism library has ONE page that owns the
slash-command surface: `docs/library/cli-surface.md`. Its support-module table
(§ "The support modules behind the commands", line 226) already carries a
`commands.rs` row describing the registry, and its § "Surfaces — the slash
commands inside a session" (line 238) describes resolution. The new module is
the *shared contract* both registries now build on, so the honest fix is a row
in that same table plus a correction to the two paragraphs that still describe
ONE registry where there are now TWO tables over ONE contract.

Three drifts in that page are exposed by the same reading, and all three are
stale prose rather than gate findings (E019 does not cover them — no
`ops.doc_claims` entry derives them):

1. **"84 slash commands"** (line 226, and again at line 240). Derived now:
   `rg -c 'CommandDef::new\(' crates/operant-cli/src/commands.rs` = **90** in
   the TUI registry. The gateway registry holds a further **62**
   (`crates/operant-cli/src/gateway_commands.rs`) — the page never mentions it.
2. **"the two are deliberately separate registries"** (line 226). Still true as
   *tables*, false as *contract*: since iter-2742 both are `CommandDef` from
   `operant_core::command_router` and both resolve through the same
   `resolve_command`. The sentence now understates the sharing it describes.
3. **The E311 gap itself** — the page names no `command_router.rs` at all.

**The fix.** In `docs/library/cli-surface.md`:
- add a `command_router.rs` row to the support-module table: what it is (the
  ONE shared `CommandDef`/`CommandResult`/`CommandRegistry`/`resolve_command`
  contract), why it exists (three surfaces carried three types and three
  classifiers; `/effort` resolved in the gateway but not the TUI registry), and
  which two tables build on it.
- correct the "84 slash commands" count to the derived pair (90 TUI + 62
  gateway) and say which is which.
- correct the "separate registries" sentence to "separate TABLES over one
  contract".

**Verify:** `./scripts/genome lint` exits 0 on E311 (the error disappears, not
suppressed — the citation is real); `genome lint` reports no new finding; the
counts in the page match the two `rg -c` derivations in the same commit.

### T2 — W006: retire the two superseded plan files (Severity 3)

**The claim.** Two `W006` pairs:

```
W006 plans `2026-10-27-remaining-implementation-gaps-outline-execution-plan-for-the.md`
     and `2026-11-19-close-the-last-nine-open-ledger-rows-the-dead-surfaces-go-th.md`
W006 plans `2026-11-19-close-the-last-nine-ledger-rows-and-the-five-open-hydrogen-f.md`
     and `2026-11-19-close-the-last-nine-open-ledger-rows-the-dead-surfaces-go-th.md`
```

**What is actually true.** The detector
(`scripts/genome`, § "duplicate-question plans (dated form)") compares the
word-sets of each plan's FIRST heading and fires at Jaccard ≥ 0.5, skipping any
file whose first 8 lines carry a `<!-- kb: tombstone target="..." -->` marker.
Measured: **none of the three files carries a tombstone marker** — all three
carry a prose `> **SUPERSEDED iter-2693.**` blockquote instead, which the
detector cannot see (its own docstring says why: a prose mention is not a
tombstone).

The doctrine for a dated-frozen plan is explicit in AGENTS.md: the class is
"dated-frozen", and "a plan that changes is a new plan". These three are
**already declared superseded by iter-2693** — by prose, in their own headers —
so retiring them is not a content decision, it is making the header's own claim
machine-visible. `genome archive <path> --reason "<why>"` leaves a tombstone at
the original path and an `docs/archive/INDEX.md` row, which is exactly the
"nothing is deleted" law.

**The fix.** Archive the two files the blockquotes name as superseded and that
no live question still routes through:

- `docs/plans/2026-10-27-remaining-implementation-gaps-outline-execution-plan-for-the.md`
  — its own header says "supersedes the scheduling half of this plan and the two
  that followed it", and the named owner
  (`2026-11-19-close-the-measured-residuals-stale-checkboxes-doc-drift-dead.md`)
  is the one that took the question.
- `docs/plans/2026-11-19-close-the-last-nine-open-ledger-rows-the-dead-surfaces-go-th.md`
  — same header, same named owner.

**Do NOT archive** `2026-11-19-close-the-last-nine-ledger-rows-and-the-five-open-hydrogen-f.md`.
It is the THIRD plan in that lineage and its header says the previous two
"closed most of what they scheduled" — it is the one that carried the live
question forward, and it is the file the two archived ones point at. Archiving
it would remove the owner the pair's own headers name.

**Verify:** `./scripts/genome lint` reports zero `W006`; `docs/archive/INDEX.md`
carries a row per archived file; the two tombstones resolve (a reader following
either original path lands on the owner, not on a dead end).

### T3 — W010: three long files with no table of contents (Severity 3)

**The claim.** `genome lint` names three files over the 300-line `toc_lines`
budget with no TOC:

```
docs/audit/2026-11-16-arch-redteam/A2-tools.md          341 lines
docs/audit/2026-11-16-arch-redteam/COORDINATOR-findings.md 330 lines
docs/library/voice.md                                   314 lines
```

**What is actually true.** The detector (`scripts/genome`, § "TOC on long
docs") accepts EITHER a hand-written `## Contents` / `](#` anchor form OR a
generated `<!-- genome:zone ledger-toc` zone. Two of the three are dated-frozen
audit artefacts; one is a live library page.

**The fix.** Two different treatments, because the mutability classes differ:

- The two `docs/audit/2026-11-16-arch-redteam/` files are **dated-frozen** — a
  finding report must not be edited. Add the TOC as a `<!-- genome:zone
  ledger-toc -->` generated zone if the genome supports it for audit dirs, or
  add `toc_lines` coverage exemption for that directory **with the reason**
  (frozen audit artefacts gain nothing from a TOC that cannot be kept current
  after the file is sealed). Decide by reading `genome.toml`'s `ops` section
  first — the exemption list is the sanctioned escape and must carry the reason.
- `docs/library/voice.md` is a **current** library page: add a real `## Contents`
  anchor list. It is the one that can be kept honest.

**Verify:** `./scripts/genome lint` reports zero `W010`; the frozen audits are
either exempted-with-reason in `genome.toml` or carry a generated zone; the
library page's TOC anchors resolve (every `](#...)` target is a real heading).

## What this plan does NOT do

- **RT-543's sharing proof (the peer's T2).** Measured absent, and the
  predecessor plan `2026-11-20-close-the-three-residual-gaps-...` schedules it
  as its own T2. **One owner per question** (genome law 1): it is that plan's
  item, not a second copy here. Do not implement it under this plan's label —
  if that plan is retired, mint a new one that owns it.
- **RT-190 (memory bank topology), RT-280 (resume-after-restart / auto-reset
  after `/stop`).** Both are **owner rulings**, not code. RT-190 was ruled
  option (c) and documented; option (b), the bank bridge, "stays the next
  tranche if cross-surface recall turns out to matter". RT-280 needs a product
  decision on "how recently active" a session must be, and its own row records
  that wiring it as-is would import a silent SQLite error (the state writers
  drop the error at :1156/:1181/:1206/:1781 while siblings propagate it).
- **RT-543's DISPOSITION half** (does `operant acp serve` still exist as a
  second daemon, or is it retired in favour of the session socket). That is the
  owner ruling the row itself names.
- **The 131 remaining `pub-fn` candidates.** Reported by
  `audit-pub-fn-surface.py --check`, and the predecessor plan explicitly
  declined them ("reported by the gate, inherited, not this change's to
  clear"). 9 of them are acid-test-dead (exactly one reference — their own
  definition — measured per name: `all_records_mut`, `approver_of`,
  `child_sessions`, `children_mut`, `determine_trust`, `format_terminal`,
  `from_app_config`, `from_str_key`, `enable_tool`) and are a clean future
  tranche; the other 122 have in-file tests. **Not this plan's** — minting a
  fourth plan for them would repeat the duplicate-owner defect T2 exists to
  retire.
- **W014 (`asks Q4`, open 17 days).** `genome lint` wants the operator
  re-asked or the ask withdrawn. It is a question about terminal enhanced-keys
  probing (~120 lines on the input path for one warning a user never sees,
  against fallbacks that are already shipped and live-verified). **An agent
  cannot answer it.** Surface it to the operator; do not withdraw it on the
  operator's behalf.
- **No design-preference change.** Phonon, memory-wire, sourcehound, the 7
  gateway platforms, tiered eviction — untouched.

## Order of work

```
1. T1  (E311 + the three drifts in the owning page)  → verify: genome lint exits 0, counts re-derived in-commit
2. T2  (archive the two superseded plans)            → verify: zero W006, tombstones resolve
3. T3  (three long files: 2 frozen, 1 live)          → verify: zero W010, anchors resolve
```

T1 first: it is the error I introduced, and it is the only ERROR in the battery
(the rest are warnings). T2 next because it is pure bookkeeping with a
machine-checkable outcome. T3 last because it is the only item with a judgement
call (exempt vs TOC) and benefits from the other two being green.

## Definition of done

- `./scripts/genome lint` exits 0 (was 2): zero errors, and the W006/W010
  warnings gone rather than allowlisted.
- `docs/library/cli-surface.md` cites `command_router.rs`, states the derived
  registry counts (90 TUI / 62 gateway), and describes the tables as separate
  over ONE contract.
- Two tombstones resolve to the owner iter-2693 named; `docs/archive/INDEX.md`
  has a row for each.
- Each item is its own iteration, committed and pushed, label re-derived after
  `git fetch origin` with a `git rev-list --left-right --count origin/main...HEAD`
  check before the push.

## Cost

Three iterations, all documentation or archive moves — **no code change**, so no
release build and no deploy. T1 is ~20 lines of prose in one page. T2 is two
`genome archive` invocations plus the index rows they generate. T3 is one
hand-written TOC and one `genome.toml` exemption decision.