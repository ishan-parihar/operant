# KB surface management — the operant survey and its long-horizon limitations

Date: 2026-10-27 · Surveyor: `agent-omarchy` · Frozen (dated audit — evidence snapshot, verdicts live in the ledger/records)

**Owns:** the measured state of the operant knowledge base on 2026-10-27 and the
long-horizon limitations that state exposes. **Not for:** the protocol that
answers them (that is the companion plan
`docs/plans/2026-10-27-kb-management-protocol-consolidation-lifecycle-compaction-an.md`).

All figures below were derived on 2026-10-27 from the tree at `fa4a7430`
(iter-1416), per RG-008: they are the survey's evidence, not numbers to copy
into other documents.

## What the KB is today

The genome (`genome.toml`, schema v2, modules core+records+agents+ops) maps
these KB roles:

| Role | Path | Lines | Last commit | Mutability class |
|---|---|---|---|---|
| feed | `docs/kb/feed.md` | 113 (102 entries) | 2026-10-27 | append-only |
| worklog | `docs/kb/WORKLOG.md` | 426 | 2026-10-27 | append-only |
| ledger (CLOSED) | `docs/kb/CLOSED.md` | 23 | 2026-10-20 | append-only |
| asks | `docs/kb/ASKS.md` | 21 | 2026-10-21 | append-only, rows amended by ruling |
| workflows | `docs/kb/workflows/` (6 files) | 83 total | 2026-10-27 | rows update, claims guarded |

Around the KB sit the transient surfaces the genome also governs:

| Surface | Files | State on 2026-10-27 |
|---|---|---|
| `docs/plans/` | 45 dated-frozen plans | 12 never touched after creation day; 4 mention consolidation anywhere; the rest frozen at `status:` lines inside their text |
| `docs/audit/` | 33 dated audits | several predate the repo's own genome adoption; the 2026-05-13 setup-flow trio is 5.5 months old |
| `docs/archive/` | 41 items across 8 generations | tombstoned correctly (W007 clean) |
| `docs/library/` | 30 mechanism pages | the consolidation TARGET — `Source:` headers, E301 ownership-enforced |

## The long-horizon limitations the survey found

Each limitation below is a failure mode that gets WORSE, not better, as the
project's horizon grows. That is the criterion for including it.

### L1 — Append-only files grow without bound, and nothing compacts them

`WORKLOG.md` is 426 lines after ~11 days of the current sprint (26 entries on
2026-10-27 alone; 34–57/day at the sprint peak). At that rate a year of
operant-pace development is a 10,000+ line single file. The genome's own
E015 knows this: it maintains a worklog INDEX zone (`genome emit`). But the
index solves navigation, not size — every waking agent still reads
entry-after-entry, and the append-only law forbids the one operation
long-horizon survival requires: moving aged entries OUT.

The feed has the same shape (33 entries on 2026-10-27) plus a demonstrated
race: two `[F0096]` labels exist because the auto-number reads the file tail
between two agents' read-and-write. Append-only + concurrent agents + a
monotonic counter in the file body = label races by construction.

### L2 — Frozen plans never reach a terminal state

Of 45 plans, 12 were never touched after their creation day, and only 4
mention consolidation. The genome's own status vocabulary (`open
in-progress done rejected blocked`) has no representation ON the plan itself:
dated-frozen means the FILE cannot change, so the only state a plan can
reach is... silence. Nothing forces the question "this plan shipped — where
did its knowledge GO?" The plan stays in `docs/plans/` forever, at `LIVE`
status in the ownership map's eyes, indistinguishable from an active plan.

Concrete case: `2026-10-26-two-repo-ponytail-cut-and-integration-repair.md`
shipped (iters 1306/1307/1311/1314) and its durable knowledge lives in
`docs/library/` — but the plan file itself carries no marker saying so. A
waking agent reading plans/ sees 45 equally-frozen documents.

### L3 — Audits age into "evidence nobody re-reads"

`docs/audit/` holds audits from 2026-05-13 to 2026-10-27. The genome's E001
checks that AGENTS.md points at live paths, and BUGS.md rows track fix state —
but the audit FILES themselves have no disposition. The 2026-05-13 setup-flow
trio is the canonical example: 394+337+342 lines of setup-wizard analysis of
a setup wizard that has since been red-teamed (2026-10-27 audit), fixed
(iters 1402-1404), and documented (`docs/library/operability.md`). Nothing in
the tree marks the old audits as superseded, and E301 does not cover
`docs/audit/` (not in `owner_globs`).

### L4 — Consolidation is doctrine without a forcing function

The genome's law 3 says "superseded → archive", and the ops reference says
"the other half [of one-owner-per-question] is judgement: consolidate them
and archive one". Judgement without a checkpoint is how 45 plans stay 45
plans. The one consolidation loop that DOES work in operant — knowledge
flowing into `docs/library/` — works because a human agent ran staleness
sweeps (iters 1405/1407) by choice. At fleet scale and long horizon, choice
is not a mechanism.

### L5 — Redundancy across kb/, plans/, audits/ is invisible

The same finding can legitimately live in four places: a feed note, a
BUGS.md row, a plan, and an audit. Today nothing detects the overlap, so the
"non-redundant" requirement is unenforced anywhere. W006 catches two PLANS
answering the same question — the only redundancy check in the genome — but
not plan-vs-audit, audit-vs-kb, or feed-vs-ledger.

### L6 — Scratch has no protocol

The user's directive names "general scratch files". The genome has `E304`
(unreachable code), `E305` (tracked bulk), and `lint_layout` — but nothing
governs an untracked-or-tracked scratch note that is not code, not a plan,
not an audit, and not a KB role. In operant, scratch lives in agent sessions
and dies there (acceptable), but any project adopting the genome gets no
guidance at all for `scratch/`, `notes/`, or ad-hoc `DRAFT-*.md`.

## What the survey does NOT found (limits of the evidence)

- WORKLOG/feed growth rates are from a 10-day sprint at operant's unusual
  pace (a solo operator + concurrent agents). A slower project hits the same
  walls later, at the same proportion.
- No claim here says the genome's CURRENT checks fail — they all pass at tip
  (`lint 0 errors, 3 warnings; doctor healthy`). The limitations are about
  what the checks do not even ask.

## The companion

The protocol that answers L1–L6 is the plan
`docs/plans/2026-10-27-kb-management-protocol-consolidation-lifecycle-compaction-an.md`
— proposed at the /project-genome level so every adopting project inherits
the enforcement, not just operant.
