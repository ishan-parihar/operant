# Meta-skill-creator → nested-references rewrite (audit + plan, 2026-10-09)

> **Owner decisions (locked):**
> 1. **Full lockstep** — rewrite the skill *and* its Rust validator copy *and*
>    migrate the 7 already-built trees. One architecture everywhere.
> 2. **Repo copy is source of truth** — edit `skills/software-development/
>    meta-skill-creator/` (git-tracked, iteration protocol), then sync outward
>    to `~/.agents/skills/meta-skill-creator/` (the copy agents actually load).
>
> **Direction:** stop building meta-skills as trees of *skills that route to
> other skills*; build them as **one skill over a nested references
> infrastructure** — unlimited-depth `references/` trees of plain markdown
> nodes, loaded progressively. Root stays the only registered, frontmatter
> bearing, triggered skill.

---

## §1 Audit — meta-skill-creator

Locations audited: `skills/software-development/meta-skill-creator/` (repo,
canonical, 2026-08-15, 16 298 B) and `~/.agents/skills/meta-skill-creator/`
(live, 2026-07-11, 16 059 B). Files: `SKILL.md` (263 lines),
`references/templates.md` (151 lines), `scripts/registry.py` (260 lines).

### A. Architecture routes skills to skills (the defect being fixed)

| # | Finding | Evidence |
|---|---------|----------|
| A1 | The single recursive `NODE` type **is a skill**: directory + `SKILL.md` with frontmatter. Routers route the agent to *child skills* ("Read child SKILL.md files while following pointers"), sub-skills, sub-sub-skills — skill→skill routing at every level. | SKILL.md "The core idea" + "The routing contract" |
| A2 | Child frontmatter descriptions are a **second, unregistered trigger surface** ("Descriptions are the routing surface … exactly what a root description does for Claude's skill list"). Nothing discovers them; they duplicate skill-list machinery inside the tree. | SKILL.md contract item 3 |
| A3 | The skill **routes to another skill as its own foundation**: Phase 0 → `~/.claude/skills/skill-creator/SKILL.md` (live) / `skill_view(name="skill-creator")` (repo); frontmatter description ends with "Builds every individual node using the skill-creator skill as the foundational base." Phase 6 (eval machinery), Phase 6 (description optimization), Phase 3 (writing guide) route to skill-creator again — **4 hard cross-skill dependencies**. | SKILL.md lines 14, 126, 173, 222–232 |
| A4 | Those dependencies are **not self-contained**: if skill-creator is absent, renamed, or not in the pool at that exact path/name, meta-skill-creator loses its writing rules, eval loop, and description optimizer. Nested references have no external dependency — the whole skill ships in its own directory. | consequence of A3 |

### B. Concrete defects

| # | Finding | Evidence |
|---|---------|----------|
| B1 | **Two diverged copies, no sync.** SKILL.md differs (operant `metadata:` block + `skill_view` routing vs absolute `~/.claude/…` path); `templates.md` and `registry.py` are byte-identical. Last live edit Jul 11, repo Aug 15. | `diff -rq` of both trees |
| B2 | **Pointer validation is substring matching, both directions.** A child is "reachable" if its directory name appears *anywhere* in the parent body; a pointer to a *nonexistent* path is never checked (no dangling-pointer detection); unreferenced resources match by basename anywhere in the subtree. | registry.py `validate()` |
| B3 | registry.py counts only `.md` resources — `scripts/` and `assets/` are invisible to the dead-weight check. No staleness check: a stale `_map.md`/`_registry.yaml` passes `--check`. | registry.py `scan()` |
| B4 | Phase 6 is harness-specific ("Run claude-with-the-meta-skill"). | SKILL.md Phase 6 |
| B5 | Frontmatter parser is naive single-line `key: value` (multi-line quoted YAML folds wrongly) — becomes irrelevant below root once only the root carries frontmatter. | registry.py `parse_frontmatter` |
| B6 | *No defect (fairness):* the skill obeys its own size budget (263 < 500), both internal pointers (`references/templates.md`, `scripts/registry.py`) resolve, and `registry.py` exits non-zero only on errors. | — |

### C. Ecosystem check ("check all the skills")

| # | Finding | Evidence |
|---|---------|----------|
| C1 | **7 trees already built to this contract** — 269 nested child SKILL.md, ≈23 900 lines, all carrying `_build/`, `_map.md`, `_registry.yaml`: human-conversation **140**, mental-model-atlas **49**, website-design **26**, linkedin-marketing **24**, agent-interface **17**, cognitive-kernel **7**, perfect-github-readme **6**. | `find ~/.agents/skills -name SKILL.md` nested count |
| C2 | Repo `skills/` (634 tracked files, 107 SKILL.md) has one nested tree: `skills/creative/website-design/components` (6 nodes). | nested-count sweep |
| C3 | **The routing contract has a Rust lockstep copy**: `skills_tool.rs:35` "meta-skill-creator registry.py parity" budgets; `collect_tree_validation` / `validate_skill_tree` / `collect_skill_children` / `skill_manage generate_map` (30 refs); `operant skills audit` tree gate (`cmd_skills.rs`, 4 refs); the always-on agent prompt block (`agent/mod.rs:71` — "regenerate the map… unreachable children, name/dir mismatches…"); the node-routing hint `skill_view(name='<parent>/<child>')` (`skills_tool.rs:1097`). | code search |
| C4 | **Operant already natively supports the target model.** Discovery registers only top-level dirs (`skills.rs:74`); `skill_view(file_path=…)` loads supporting files under `ALLOWED_SUBDIRS = ["references","templates","scripts","assets"]` with nested paths allowed and **no frontmatter validation below root** (`skills_tool.rs:75`, `validate_file_path`). Child SKILL.md are therefore second-class: registered nowhere, loadable only via the parent/child name hack. | code read |
| C5 | ≈60 top-level skills contain cross-skill routing prose ("load the X skill" / `skill_view(name=…)`) — inventory captured, sweep deferred (§5). | rg sweep over `**/SKILL.md` |
| C6 | Hygiene: **92 dangling symlinks** in `~/.claude/skills` (incl. writing-skills, writing-great-skills, skill-consolidation, resume-skills, operant-skill-authoring), `.bak` debris (`meta-skill-creator.bak`, `cognitive-kernel.bak`, `context7-mcp.bak`, `browserclaw.bak`), `.codex-marketplace/` nesting inside linkedin-marketing. | `find -L ~/.claude/skills -type l` |

---

## §2 Target contract — the nested references infrastructure

**Before (skill → skill routing):**

```
trading-systems/SKILL.md        router  (frontmatter: name+description)
├── strategy-research/SKILL.md  "sub-skill"  ← routed to by parent/child name
│   └── backtesting/SKILL.md    "sub-sub-skill"  (…each with its own identity)
└── scripts/registry.py
```

**After (one skill, nested references):**

```
trading-systems/
├── SKILL.md                    THE skill — only frontmatter in the tree;
│                               top-level routing table + Navigation + conventions
├── references/                 nested reference tree — unlimited depth
│   ├── strategy-research.md    node = plain .md (H1 + one-line summary + body)
│   │   ├── backtesting.md      …or a directory group when a branch grows
│   │   │   ├── data-hygiene.md
│   │   │   └── walk-forward.md
│   │   └── signal-design.md
│   │       └── frameworks/     alternative methods (unchanged semantics)
│   │           └── trend-regime.md
│   └── voice-guide.md          shared resource at the lowest common ancestor
├── scripts/registry.py
└── _map.md                     generated jump table (sharded per branch)
```

**The contract (replaces "The routing contract") — 7 rules:**

1. **One skill per meta-skill.** Only the root has `SKILL.md` + frontmatter +
   registered identity. A `SKILL.md` below root is an **ERROR** — a regression
   to the routing model, caught by the validator.
2. **A node is a file.** `references/**/*.md`, nested by directories to
   unlimited depth. No frontmatter below root: `H1` = name, first line =
   one-line summary (this absorbs the per-node "routing description").
3. **Pointers are markdown links**: parent → child is
   `[label — descend when X](relative/path.md)`. The validator resolves links
   **both ways**: node not reachable from root = ERROR; link to a missing file
   = ERROR (fixes B2). Bare backticked paths are accepted only for
   `scripts/`/`assets/` resources.
4. **Navigation surface = root routing table + generated `_map.md`** (path +
   first sentence per node, sharded once a branch exceeds the map threshold).
   Walk = O(depth) short reads; jump = map read + one file. Load ceiling: root
   SKILL.md + one reference path (+ at most one framework file). Delegation:
   hand a subagent `references/<branch>/` + its slice of the task.
5. **Resources at the lowest common ancestor**, as today; `frameworks/`
   keeps its selection-guide semantics; `scripts/` run without loading.
6. **No cross-skill routing for capability.** Skill-creator's writing rules,
   description standards and eval loop are **vendored** into
   `references/writing-guide.md` (distillate + attribution line). Cross-skill
   mentions elsewhere are discovery-only ("if installed, skill-creator has the
   full eval harness"), never a required step. Kills all 4 dependencies in A3.
7. **Every tree self-validates**: `registry.py <root> --check` must exit 0 on
   the meta-skill-creator's own directory — the skill passes its own gate.

---

## §3 Fix plan — one iteration per row (commit + push per AGENTS.md)

| Iter | Scope | Files |
|---|---|---|
| **1** | This plan (docs only) | `docs/plan-2026-10-09-meta-skill-creator-nested-references.md` |
| **2** | **Rewrite meta-skill-creator** (canonical, repo) | `skills/software-development/meta-skill-creator/SKILL.md`, `references/templates.md`, new `references/writing-guide.md`, `scripts/registry.py` |
| **3** | **Rust lockstep** | `crates/operant-core/src/tools/skills_tool.rs`, `crates/operant-cli/src/cmd_skills.rs`, `crates/operant-core/src/agent/mod.rs` (prompt block), tests, `CHANGELOG.md` |
| **4** | **Migration script + pilot tree** | new `scripts/migrate_to_references.py`, `skills/creative/website-design/**` |
| **5** | **Migrate the 7 live trees** (269 nodes) | `~/.agents/skills/{human-conversation,mental-model-atlas,website-design,linkedin-marketing,agent-interface,cognitive-kernel,perfect-github-readme}/**` (script committed in iter 4) |
| **6** | Verification sweep + sync + tracker updates | sync copy → live, `BUGS.md` rows for §5, `CHANGELOG.md` if audit UX changed |

### Iter 2 detail — the rewrite itself

- **SKILL.md** (target ≈230–260 lines, root budget ≤200 lines is a WARN not a
  hard gate): rewrite "core idea" (file-nodes, not skill-nodes), replace the
  routing contract with §2's 7 rules, retitle sub-skills → reference nodes,
  rewrite the description frontmatter (drop the skill-creator dependency
  sentence; trigger words keep "meta-skill", "skill suite", "domain pack",
  "decompose an oversized skill"), Phase 0 → *read the nested
  `references/writing-guide.md`* (no external skill load), Phase 6 made
  harness-neutral ("run the agent with the skill loaded; check which files it
  actually Read"), Phases 4–5 keep registry/mapping roles with link semantics.
- **references/writing-guide.md** (new, ≈60–80 lines): vendored distillate of
  skill-creator — description rules (what+when, pushy triggers), progressive
  disclosure, writing style (imperative, why-not-what), eval/iterate loop,
  + one attribution line pointing at the full skill for the heavy tooling.
- **references/templates.md**: root SKILL.md template (routing table +
  Navigation + conventions), **nested reference-file template** (H1, summary
  line, body, `Next:` links), framework template (unchanged), `_build/plan.md`
  (updated paths/checkboxes), worked 3-level mini-example in the new shape —
  same `content-engine/` story, converted to file-nodes.
- **scripts/registry.py** rewrite (keep CLI: `root`, `--check`, `-o`,
  `--shard`):
  - walk = root `SKILL.md` + `references/**/*.md` + resource dirs at any depth;
  - extract markdown links + backticked paths → **exact-path reachability
    both ways** (unreachable node ERROR, dangling pointer ERROR — fixes B2);
  - `SKILL.md` below root = ERROR (rule 1); root name/dir mismatch, missing
    description, placeholder description = ERROR; root >200 lines, reference
    file >500 lines = WARN;
  - `.md` under resource dirs **and** `scripts/`/`assets/` counted for the
    unreferenced-resource WARN (fixes B3);
  - `--check` gains **staleness detection** (regenerate into a temp buffer,
    diff against on-disk `_map.md`/`_registry.yaml`);
  - outputs unchanged: `_registry.yaml` + sharded `_map.md`.

**Iter 2 acceptance:** `python3 skills/software-development/meta-skill-creator/scripts/registry.py skills/software-development/meta-skill-creator --check` → `0 errors`;
mutation-proven: reintroduce a broken link → exit 1; drop a stray
`references/SKILL.md` fixture → exit 1. Then `cp -r` repo copy →
`~/.agents/skills/meta-skill-creator/` (the `metadata.operant` block is
tolerated by foreign harnesses and skipped by registry.py) and `diff -r` clean.

### Iter 3 detail — Rust lockstep

- `skills_tool.rs`: rewrite `collect_map_nodes` / `collect_tree_validation` /
  `validate_skill_tree` / `collect_skill_children` / `generate_map` /
  `MapNode` for file-nodes: walk `references/**`, link-based reachability,
  nested-SKILL.md ERROR, budgets const (line 35) retargeted; per-node
  name/dir/description checks collapse to root-only; update the hint at line
  1097 (`skill_view(name='<parent>/<child>')` → `skill_view(name='<root>',
  file_path='references/…')` or plain Read) and error strings
  ("never be routed to" → "never reachable from root").
- `cmd_skills.rs` tree gate adapts via `validate_skill_tree`; audit output
  wording updated.
- `agent/mod.rs:71` prompt block: new failure vocabulary (unreachable
  reference file, broken pointer, nested-SKILL.md regression) so the
  always-on guidance matches the validator it tells the agent to run.
- Tests: update the ~30 in-file + 4 CLI refs; **every fix ships a regression
  test that fails without it** (unreachable node, dangling pointer, nested
  `SKILL.md` under `references/`).
- Verify scoped: `./scripts/check.sh check -p operant-core --lib` →
  `cargo test -p operant-core --lib skills` → `check -p operant-cli --bin operant`
  → `clippy -p <both> -- -D warnings` → final `check/test --workspace` before
  push. `CHANGELOG.md` row (user-visible `operant skills audit` messages).

### Iter 4–5 detail — migration

`scripts/migrate_to_references.py` (committed in iter 4), per child node:

1. read `<child>/SKILL.md`; strip frontmatter; fold its `description` into the
   parent's routing line if the parent lacks one;
2. write body → `references/<relpath>.md` (directory structure preserved);
3. rewrite parent pointers `child/SKILL.md` → `[label](child.md)` links;
4. delete the emptied child dirs; regenerate `_map.md` + `_registry.yaml`;
5. **content-preservation assertion**: body lines before == after (minus
   frontmatter + blank separator) — the migration must not rewrite prose.

Pilot on the repo tree (`skills/creative/website-design`, 6 nodes) in iter 4;
then the 7 live trees in iter 5 (human-conversation 140 → … → perfect-github-
readme 6), each validated **twice**: new `registry.py --check` (0 errors) and,
after iter 3, `operant skills audit <tree>` (0 errors). Per-tree before/after
node + line counts recorded in the commit body. linkedin-marketing's
`.codex-marketplace/` exclusion honored by the scanner (underscore/hidden-dir
skip already exists).

### Iter 6 detail — closeout

- Sync repo → live (already done per-iter, re-run + `diff -r`).
- Navigation smoke test: fresh-context agent, root → `_map.md` → correct
  reference file in ≤2 reads on a known prompt and on a sibling near-miss.
- Grep gate: zero `SKILL.md` pointers below any root across both stores.
- `BUGS.md` rows for §5 items; `CHANGELOG.md` if anything user-visible moved.

---

## §4 Verification matrix

| Gate | Command / check | Pass criterion |
|---|---|---|
| Self-validation | `registry.py <meta-skill-creator> --check` | 0 errors |
| Mutation (validator) | break one link / plant one nested SKILL.md | exit 1, named file in message |
| Rust scoped | `check/test/clippy` on `operant-core` + `operant-cli` | green, no new warnings |
| Rust regression | 3 new tests (unreachable / dangling / nested-SKILL) | fail without the fix |
| Migration content | before/after body-line assertion per node | exact match |
| Migration validation | `registry.py --check` + `operant skills audit` per tree | 0 errors × 7 trees |
| Sync | `diff -r` repo ↔ live meta-skill-creator | identical |
| Navigation | fresh-agent jump + near-miss routing | ≤2 reads, right leaf |
| Final | `./scripts/check.sh check --workspace` + `test --workspace` | green |

## §5 Deferred (separate iterations, owner-approved later)

1. **C5 cross-skill routing sweep** — ≈60 skills carry "load the X skill"
   prose; convert *hard* dependencies to nested references (or downgrade to
   discovery-only mentions), one family per iteration. Inventory is reproducible
   with the rg pattern in the audit.
2. **C6 hygiene** — 92 dangling `~/.claude/skills` symlinks (5 of them
   skill-management family members), `.bak` debris, `.codex-marketplace/`
   nesting. Manual review; outside the repo, no commit needed.
3. Optional: fold `registry.py --check` for in-repo trees into
   `scripts/self-test.sh` (local only — GitHub Actions runs remain banned).
