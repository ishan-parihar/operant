# Lane G — skills, org/kanban orchestration, curator, advisor

**Lane:** G of the 2026-11-16 operant architecture red-team + ponytail audit.
**Seam:** skills · skill guard · org/kanban orchestration · curator · advisor.
**Pinned revision:** `25b86ae5` (`feat(iter-2472)`), confirmed with
`git rev-parse --short HEAD`. Tree clean except untracked peer scratch dirs
(`.w10a-tmp`, `.w10b-tmp`, `.w10d-tmp`, `.tmp-test`, `crates/operant-cli/src/lib.rs`,
`docs/kb/feed.md` modified) — none read by the build, ignored.

**Status: IN PROGRESS (skeleton written first, per lane instructions).**

---

## 1. THE PRIOR-5 STATUS TABLE (centrepiece)

Prior pass: `docs/plans/2026-11-16-hermes-scale-comparison-skill-infrastructure.md`
(iter-2467), board `docs/kb/workflows/skill-scale-redteam-2026-11-16.md` rows
001..005. All five were `open` at `f76ae548`. Three named commits landed after
it: iter-2466 (`f76ae548` — the plan's own base), iter-2468 (`1aa63cb5`),
iter-2469 (`e1b501ad`).

| id | finding | prior sev | status at 25b86ae5 | evidence file:line |
|---|---|---|---|---|
| F-1 | shadowed skill silently dropped (`skills.rs` `contains_key` → `continue`) | HIGH | **PENDING RE-DERIVATION** | — |
| F-2 | project-skill quarantine absent from load path | HIGH | **PENDING RE-DERIVATION** | — |
| F-3 | description limit is a clip, no write-time reject | MED | **PENDING RE-DERIVATION** | — |
| F-4 | no content-digest scan cache | MED | **PENDING RE-DERIVATION** | — |
| F-5 | search truncates by (tier, name), ignores root precedence | MED | **PENDING RE-DERIVATION** | — |

---

## 2. Concept counts

(to fill from the map)

---

## 3. Findings

(to fill; G1.. each VERIFIED|ASSERTED + severity + file:line + failing case)

---

## 4. Ponytail cut list

(to fill, with LOC)

---

## 5. Rust discipline table

(to fill)

---

## 6. Files read / NOT read

**Read:** (to fill)
**NOT read:** (to fill)