# KB management protocol v2 — SQLite state, gates, one-active-per-agent, and the genome utilization audit

Dated-frozen: planned `2026-10-28` (session of 2026-10-27). Status:
**PROPOSED — the operator's four amendments incorporated; supersedes the
2026-10-27 v1 plan** (`docs/plans/2026-10-27-kb-management-protocol-consolidation-lifecycle-compaction-an.md`,
kept for the record; its survey base
`docs/audit/2026-10-27-kb-surface-management-the-operant-survey-and-its-long-horizo.md`
carries over unchanged).

**Owns:** the amended design — WORKLOG/BUGS/TODO/WORKFLOW as CLI-managed
SQLite state with a standardized schema; the new-file gate for plans/audits;
the one-active-file-per-agent rule; and the /ponytail-audit findings on the
genome architecture. **Not for:** the survey numbers (the audit owns them)
or implementation (separate iterations once ratified).

## The operator's four amendments, and what each changes in v1

1. **SQLite + CLI for WORKLOG/BUGS/TODO/WORKFLOW** — v1 kept markdown files
   as the state and added registers around them. The operator overrules:
   the four churn-surfaces become SQLite-backed with the markdown as a
   RENDERED VIEW (or the state moves entirely behind the CLI). v2 agrees
   and designs the schema below.
2. **New plan/audit files error while existing statuses are unapproved** —
   v1 had no admission gate at all. v2 adds one: minting is refused while
   any existing transient row lacks an approved status.
3. **One active file per agent in WORKFLOW/TODO** — v1 left boards to
   multiply; v2 makes concurrency legible: an agent's previous board must
   be done/archived before a new one can be claimed.
4. **First-class global genome install + ponytail-audit of the genome** —
   DONE this session, not proposed: install.sh hardened and pushed
   (project-genome `0151979`), `genome` now on PATH at
   `~/.local/bin/genome` (version `2026.10.22-17`), and the utilization
   audit ran (findings below).

## The SQLite schema (amendment 1)

One database, `.genome/kb.sqlite` (git-ignored via `never_commit`), owned
by the genome CLI. Markdown stays the READ surface (agents read files);
SQLite becomes the WRITE surface (agents run commands). Render-on-write
keeps every existing reader working with zero changes.

```sql
-- the four churn surfaces share one row grammar
CREATE TABLE entries (
  id          TEXT PRIMARY KEY,          -- 'W-0007', 'F0103', 'rg-1364-001', 'T-003'
  kind        TEXT NOT NULL,            -- worklog | feed | bugs | todo | workflow | transient
  agent       TEXT,                     -- claiming agent id, NULL when unclaimed
  status      TEXT NOT NULL,            -- the STATUS_VOCAB + 'consolidating'
  title       TEXT NOT NULL,
  body        TEXT,                     -- full text (feed note, bugs row, worklog entry)
  evidence    TEXT,                     -- terminal rows: '→ [date] path'
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE INDEX idx_kind_status ON entries(kind, status);
CREATE UNIQUE INDEX idx_active_per_agent
  ON entries(agent) WHERE status IN ('open','in-progress','consolidating')
  AND kind IN ('workflow','todo');                      -- amendment 3, machine-enforced
CREATE TABLE sync_state (                -- markdown ↔ sqlite reconciliation
  path TEXT PRIMARY KEY, mtime TEXT, sha TEXT
);
```

**The standardized schema** the operator asked for is this row grammar:
every surface gets id/kind/agent/status/title/body/evidence — one mental
model, four surfaces. The CLI verbs already exist conceptually in genome
(register/wf/log/post); v2 points them at the database and renders:

- `genome log --agent X` writes a worklog row → renders WORKLOG.md.
- `genome post "…"` writes a feed row under the lock → renders feed.md
  (fixes the F0096-class label race: the auto-number becomes
  `SELECT max(id)` inside the transaction, not a file-tail read).
- `genome bugs add/fix/close <id>` writes BUGS rows → renders BUGS.md.
- `genome claim/done` (existing) → the UNIQUE partial index makes the
  one-active rule a constraint, not a convention.
- `genome tnew/tmove/tdone` (v1's TRANSIENTS) → same table, `kind='transient'`.
- `genome compact kb` (v1) → `DELETE FROM entries WHERE kind='worklog' AND
  created_at < :cutoff RETURNING *` → archive render; the live render
  window is `SELECT … ORDER BY created_at DESC LIMIT :window`.

**Migration:** `genome migrate kb` reads the four markdown files, inserts
rows, and marks the files as rendered views. Idempotent; `--check` diffs
render-vs-database and fails on drift. Append-only doctrine holds because
the markdown render remains the published artifact — the database is the
write model, the file is the read model, and they reconcile on every write.

## The admission gate (amendment 2)

`genome plan` and `genome audit` mint dated-frozen files. v2 adds a
pre-mint check, **E017**:

> minting a new plan/audit fails when any existing transient row (v1's
> TRANSIENTS register, now `kind='transient'`) is `open`/`in-progress`/
> `consolidating` without an approved disposition, or when the minting
> agent already holds an active workflow/todo row.

The failure names the rows: "3 transient rows lack disposition (T-001
consolidating since 10-27, …); approve or close them first
(`genome tdone`), or pass `--force` with a written reason that lands in
the feed." `--force` exists because a genuinely urgent audit must not be
blocked by a sibling's stale row — but the forced reason is recorded.

## One active file per agent (amendment 3)

Enforced by the partial unique index above (machines beat conventions),
surfaced in three places:

- `genome claim` fails when the agent's `idx_active_per_agent` slot is
  taken, naming the current row.
- `genome wake` lists each agent's active row — the fleet state at a
  glance, which today requires reading every board.
- **The workflows directory shrinks to the truth**: this session's sweep
  archived five finished boards (holon-seat 9/9, org-tui 3/3, ponytail
  5/5, tui-redteam 9/9, kb-redteam prose→BUGS.md) leaving
  `remaining-gaps-1364` as the ONE active board — tombstones at the
  original paths, content in `docs/archive/`. That is the directory
  shape the rule produces: active state only.

## The /ponytail-audit findings on the genome (amendment 4)

Utilization was measured the only honest way available: corpus citation
counts across operant's AGENTS.md, docs/, scripts/ (the active dev loop),
cross-checked against the 45 top-level subcommands. Findings, ranked:

1. **`delete:` the vendored `scripts/genome` copy in every adopting repo.**
   Operant carries `scripts/genome` at version `2026.10.21-2` while the
   installed skill and upstream are at `2026.10.22-17` — 485 diff lines,
   two versions behind, already drifting (upstream moved again to
   `2026.10.27-3` with the outline module while this audit ran).
   Replacement: `genome` on PATH (installed this session) +
   `genome upgrade` per repo. The vendored copy is the classic
   fork-without-intent: nobody re-syncs it, and lint results silently
   differ between repos. [operant `scripts/genome`, `scripts/genome-outline.py`]
2. **`yagni:` 8 of 45 subcommands have ZERO corpus citations** — `set`,
   `secrets`, `resolve`, `outline`, `outline-check`, `modules`, `index`,
   `figure` (and `place`, `card`, `quarantine` at 1). Each is real
   machinery (~10-45 lines plus parser wiring plus help text plus test
   surface). Not "delete now" — several are adoption-path commands
   (`modules`, `index`) or new (`outline`) — but the list is the
   dead-weight watchlist; anything still at zero when the v2 work lands
   gets cut or documented into an adoption step.
3. **`shrink:` the research module ships for every KB but is off in
   operant** (`modules = core,records,agents,ops`). `lint_research` +
   `prereg`/`hyp`/`figure` machinery (~150+ lines) inactive here. The
   module system is doing its job — the finding is that
   `genome init --profile` should ask, not default-on.
4. **`native:` the feed's label race was hand-rolled file-tail
   numbering** — SQLite `max(id)` in a transaction is the native fix,
   and amendment 1 delivers it. [scripts/genome `cmd_post`]
5. **`delete:` 5 of 6 workflow boards were finished-and-still-listed** —
   swept this session (see amendment 3). The ponytail lesson: a board
   directory that never shrinks is dead weight wearing a live label.

net: `-1 vendored 5,893-line copy per adopting repo, -8 zero-use
subcommands (~250 lines) on ratification, -5 dead boards (done), plus
the SQLite label-race fix.`

## Sequencing (one skill release, then adoption)

1. **Skill-side (project-genome repo):** schema + `migrate kb` + rendered
   views + E017 + the partial unique index + `compact kb` + tests. The
   install.sh hardening is already pushed (`0151979`).
2. **Operant adoption:** delete the vendored `scripts/genome` (PATH
   install replaces it — AGENTS.md's `./scripts/genome` invocations
   become bare `genome`), run `genome migrate kb`, adopt the [ops] keys,
   back-fill TRANSIENTS rows from git dates.
3. **Ratification points for the operator** (carried from v1, still
   open): thresholds (400 lines / 14 days / 7-day scratch TTL), the
   `consolidating` vocabulary change at skill level, and the back-fill
   triage question (45 plans + 33 audits as `open` rows vs triaged
   first). Plus v2's new ones: approve E017's `--force` escape, and
   approve the vendored-copy deletion (it is a repo-root convention
   change for every adopting project).
