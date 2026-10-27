# KB management protocol — consolidation lifecycle, compaction, and the /project-genome upgrade

Dated-frozen: planned `2026-10-27`. Evidence base: the companion survey
`docs/audit/2026-10-27-kb-surface-management-the-operant-survey-and-its-long-horizo.md`
(L1–L6). Status: **PROPOSED — awaiting operator ratification** (the operator
asked for this as an upgrade proposal at the /project-genome level).

**Owns:** the design of the KB management protocol — the lifecycle that makes
every KB file relevant, non-redundant, and operational; the archiving protocol
for transient artifacts; and the /project-genome skill upgrade that enforces
all of it for every adopting project. **Not for:** the operant-specific survey
numbers (the audit owns those), or the implementation itself (separate
iterations once ratified).

## Design laws (the load-bearing decisions)

1. **Status lives OUTSIDE the frozen file.** A dated-frozen plan cannot carry
   its own state — that is what frozen means. So the lifecycle state of every
   transient artifact lives in a REGISTER the genome owns and rewrites:
   exactly the mechanism TODO.md rows already use (`open in-progress done
   rejected blocked` + evidence). The frozen file stays byte-stable forever;
   the register row moves. This is the only design that respects both laws at
   once (append-only/frozen files + statuses that must move).
2. **`consolidating` is a first-class status.** The user's directive names
   it: a transient artifact whose implementation has LANDED and whose durable
   knowledge is being folded into the core-architecture files (in operant:
   `docs/library/`). It is the state between "shipped" and "archived" — the
   forcing function L4 lacks.
3. **Compaction is archive, not deletion.** Aging append-only content moves to
   dated generation directories under the archive (`docs/archive/kb/<date>/`)
   with a manifest, exactly like existing archive generations. Law 3 holds:
   nothing is deleted; the live file shrinks because history moved, not
   because it was lost.
4. **Every check is opt-in per project, on by default at adoption.** All new
   lint codes fire from `[ops]` config keys, so a project can tune thresholds
   — but `genome init` scaffolds them ON, because L1–L6 are failure modes of
   long-horizon development, not of operant specifically.

## The lifecycle (answers L2, L3, L4)

Every transient artifact (plan, audit, scratch note, red-team report) gets a
row in a new register the genome manages: `[paths].transients` →
`docs/kb/TRANSIENTS.md` (name tunable per project). The row:

| ID | Kind | File | Status | Consolidation target | Evidence |
|---|---|---|---|---|---|
| T-001 | plan | `docs/plans/2026-10-26-two-repo-ponytail...md` | consolidating | `docs/library/` `+` integration pages | → [2026-10-27] iters 1306/1307/1311/1314 shipped |

State machine (the register vocabulary extends, statuses move through the
genome's existing vocabulary law):

```
open ──▶ in-progress ──▶ done ──▶ consolidating ──▶ archived (terminal, evidence = archive path + consolidated-into list)
                │            │
                ▼            ▼
            rejected     blocked (needs operator ruling → ASKS.md)
```

- `done` means "the code shipped" — evidence is the iteration/commit.
- `consolidating` means "the durable knowledge is being folded into the
  core-architecture files" — evidence is the library-page edit (or an
  explicit `nothing-durable` ruling: not every shipped plan leaves knowledge
  behind; that ruling is itself the evidence).
- `archived` is terminal: the file moved under the archive with a tombstone,
  and the row names where the knowledge lives now. **A plan/audit row cannot
  reach `archived` without either a consolidation target or an explicit
  `nothing-durable` evidence cell** — that is the protocol's one hard rule,
  and it is exactly the forcing function L4 wants.

## The compaction protocol (answers L1)

Two mechanisms, both mechanical, both archive-not-delete:

1. **Worklog/feed rotation.** When `WORKLOG.md` (or the feed) exceeds
   `[ops].kb_compact_lines` (default 400), `genome compact kb` moves all
   entries older than `[ops].kb_compact_age_days` (default 14) into
   `docs/archive/kb/<YYYY-MM-DD>/WORKLOG.md` with a manifest, leaves a
   three-line pointer block at the top of the live file ("older entries:
   archive path + count"), and rewrites the E015 index. The append-only law
   is preserved the same way `genome archive` already preserves it: the
   archived copy is the append-only truth, the live file is the current
   window.
2. **Feed label race fix (rides along).** The auto-number reads the file
   tail; two agents posting between each other's read and write produce
   duplicate labels (two `[F0096]` exist). Fix: `genome post` takes the
   number under the lock file it already writes (`feed.md.genome-lock`) —
   read-tail and append inside one lock hold. (Observed, not speculative:
   F0098 recorded the race.)

## The scratch protocol (answers L6)

`lint_layout` gains a `scratch` convention: a `[paths].scratch` directory
(default `docs/kb/scratch/`, git-ignored by the genome's `never_commit`)
plus one law — **scratch is born with a death date.** `genome scratch new
<title>` stamps the file header with `expires: YYYY-MM-DD` (default
`[ops].scratch_ttl_days`, 7). Doctor lists expired scratch as W011 (warning
first, E-grade after `[ops].scratch_grace_days`): expired scratch must
either be consolidated (promoted into a library page / plan / audit — which
mints its TRANSIENTS row) or archived. No unmanaged third state exists.

## The redundancy checks (answers L5)

New lint, all under the existing `lint_ops` dispatcher:

- **E306 (plan↔audit overlap):** two dated files whose `genome ask` section
  headings answer the same question — W006 already does this for plans; the
  check extends to plan-vs-audit and is advisory-first (W012) for
  grandfathered pairs, error for new files.
- **E307 (feed↔ledger):** a feed note announcing a fix that has no
  corresponding CLOSED/BUGS.md row when the note's own text claims one —
  advisory (W013); the feed is conversation, the ledger is truth, but the
  mismatch is how claims drift.
- **E016 extension (worklog↔register):** an iteration claimed in a worklog
  row with no register/TRANSIENTS movement for the artifact it says it
  closed — the "did the knowledge GO anywhere" question, asked at the only
  moment the answer exists: when work claims completion.

## The /project-genome skill upgrade (the deliverable the operator ratifies)

Everything above is implemented as a genome-module upgrade, so every
project's KB inherits it — not an operant-only convention. Concretely:

| Piece | Where it lands | New |
|---|---|---|
| `TRANSIENTS` register + `consolidating` status | `scripts/genome`: `STATUS_VOCAB` extension + register machinery (the TODO.md code path, parameterized) | `genome tnew/tmove/tdone` aliases of existing register verbs, scoped to the transients register |
| `genome compact kb` | new subcommand next to `archive` | rotation + manifest + index rewrite under the lock |
| `genome scratch` | new subcommand | `new` (stamped), `list` (by expiry), `promote` (mints a TRANSIENTS row) |
| W011/W012/W013 + E306/E307/E016-ext | `lint_ops` + doctor | the redundancy/scratch/consolidation checks |
| `genome.toml [ops]` keys | schema v3: `kb_compact_lines`, `kb_compact_age_days`, `scratch_ttl_days`, `scratch_grace_days`, `transients` path, `owner_globs` defaults now including `docs/plans/*` `docs/audit/*` | all defaulted in `init` scaffolding |
| docs | `references/ops.md` + `references/kinds.md` + SKILL.md command map | the protocol written where adopters read |
| tests | `scripts/test_genome.py` | lifecycle transitions, compact idempotence, lock-held numbering, expiry grading |

**Sequencing (one skill release, three landings):**
1. Skill-side: the genome script upgrade + tests + docs (the skill repo's own
   iteration; `genome upgrade --apply` then carries it into projects).
2. Operant-side adoption: `genome.toml` keys on, TRANSIENTS.md minted with
   the 45 plans + 33 audits as `open` rows back-filled from git dates (a
   one-time `genome tadopt` command), the two known-consolidated cases
   (ponytail plan, setup audits) set straight to `consolidating`.
3. Operant-side compaction: the first `genome compact kb` run — WORKLOG at
   426 lines crosses the 400 threshold the day the keys land.

## What this protocol deliberately does NOT do

- **No auto-archival.** Every archive is agent-run with a manifest; the
  machine proposes (W-grades, doctor listing), the agent disposes. Law 5
  (claim before work) stays fully human/agent-judged.
- **No merge of KB roles.** feed, worklog, CLOSED, ASKS, workflows keep
  their separate mutability classes — the protocol adds a lifecycle AROUND
  them, it does not reshape them.
- **No renumbering of existing labels.** The duplicate `[F0096]` stays
  (append-only doctrine); the lock fix prevents the next one.
- **No retroactive E-grading.** The 45/33 grandfathered artifacts enter as
  `open` with their git dates; new-artifact rules fire from the upgrade
  commit forward.

## Acceptance (how the operator ratifies)

- [ ] Ratify the protocol as a whole, or name the parts to cut.
- [ ] Ratify `consolidating` joining the status vocabulary at the SKILL level
      (it becomes part of every project's genome, not just operant's).
- [ ] Choose thresholds: 400 lines / 14 days / 7-day scratch TTL — or amend.
- [ ] Decide the back-fill question: should the 45 plans + 33 audits be
      `open` rows on adoption (default) or triaged to `done`/`rejected`
      first (an operant-side sweep, ~one iteration per 15 artifacts)?
