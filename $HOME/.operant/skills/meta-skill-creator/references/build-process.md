# The Build Process

The full phase-by-phase procedure for building a meta-skill: capability map →
architecture → leaves → indexes → registry → tests → growth.

## Phase 0 — Load the base

Read the nested [writing-guide.md](writing-guide.md) — this skill's vendored
distillate of skill-creator (intent interview, description rules, progressive
disclosure, writing style, eval loop). It is *inside* this skill: nothing
external is required. If skill-creator happens to be installed, its eval
harness is optional heavy machinery — never a dependency.

## Phase 1 — Decompose the domain

Interview the user and research the domain until you can write a **capability
map**: a flat list of concrete operations the meta-skill must support, each
phrased as a user request ("plan a quarter", "triage feedback signals"). Flat
and exhaustive; do not design the tree yet. Multi-domain? Tag each capability
with its domain. Get the user to confirm the map — everything else derives
from it.

Persist it immediately to `_build/plan.md` in the root (template in
[templates.md](templates.md)). A large build outlives any single context
window; the plan file lets you — or a fresh agent, or a fleet of subagents —
resume without the conversation. Underscore-prefixed paths are invisible to
the validator, so `_build/` never pollutes checks.

## Phase 2 — Architect the tree

Group capabilities into files; nest groups only where the split rules justify
it. **Deeper** = smaller, sharper contexts; **shallower** = fewer index hops,
less risk of a mis-route. Default to 2 levels (root → files); add a level where
a group breaks the 5–12 fan-out invariant or a child is really a workflow of
distinct steps. For each index file choose and state a **routing pattern**:

- **Pipeline** — ordered phases; state the order and the artifact handed off.
- **Selection** — alternatives; give a decision guide ("if X → file A; if Y → B").
- **Facets** — independent aspects usable in any combination.

Name files with gerunds or noun-phrases matching how users ask
("gap-analyzing", "pricing-strategy"). Present the tree as an indented outline
with one-line descriptions for user sign-off **before writing files**.

Then, before any node is written, add to `_build/plan.md`: the approved outline
(one line per file, with a build-status checkbox) and the **conventions
block** — shared terminology, artifact formats, directory rules, tone. Every
node builder reads it first; it is the only thing standing between a 100-file
parallel build and 100 dialects of the same skill.

## Phase 3 — Build the leaves

Leaves are where the value lives — build them first; indexes are easy to wire
once leaf content is real. Each leaf is one reference file written per the
[writing-guide](writing-guide.md) (writing rules, one-line summary, output
section).

**Scale the build with subagents.** Past roughly a dozen leaves, spawn one
subagent per leaf (or per branch), each given only: the file's path, its
capability entries and the conventions block from `_build/plan.md`, and the
leaf template. Subagents run in parallel; the orchestrator holds only the plan
checklist and spot-reviews samples against the conventions. All state lives in
`_build/plan.md`, so an interrupted build resumes from the checklist by any
fresh agent.

Two node-specific patterns:

- **Frameworks** (`<node>/frameworks/*.md`): one file per alternative method —
  `When to Use / When NOT to Use / The Method / Example / Output` — with a
  *selection guide* in the node linking exactly one at a time.
- **Shared resources live at the lowest common ancestor.** Two nodes needing
  the same doc link the one copy at the nearest common directory. Never
  duplicate content — duplicated copies drift.

Use the templates in [templates.md](templates.md).

## Phase 4 — Wire the index files (bottom-up)

Write each index file after its children exist, so its routing table describes
reality instead of intention. The root additionally gets: the one-paragraph
domain overview, the **Navigation** section, and the conventions block promoted
from `_build/plan.md`.

## Phase 5 — Registry, maps, validation

Copy [`scripts/registry.py`](../scripts/registry.py) into the meta-skill's
`scripts/` and run it **after Phase 4** (running mid-build reports unlinked
leaves as unreachable, correctly — their index files don't exist yet):

```bash
python scripts/registry.py .           # validate + write _registry.yaml + _map.md
python scripts/registry.py . --check   # validate only, write nothing
```

One walk produces all three outputs: the recursive `_registry.yaml`, the
sharded `_map.md` jump tables, and the validation report. All generated — never
hand-edit. Validation catches what silently kills hierarchies: unreachable
nodes, dangling links, stray `SKILL.md` below root, weak root description,
oversized files, unreferenced resources, stale maps. Fix every error; treat
warnings as review prompts.

## Phase 6 — Test: navigation first, then leaves

A meta-skill fails in a way flat skills can't: the right file exists but the
agent never reaches it. Test in two layers:

1. **Navigation tests.** Realistic prompts whose correct answer is "which file
   should handle this", including near-misses belonging to a sibling. Run the
   agent with the meta-skill loaded and check which files it actually Reads.
   A mis-route means an index decision guide or a one-line summary is weak —
   fix the summary/link text first; it is the cheaper lever.
2. **Leaf tests.** Per the eval loop in [writing-guide.md](writing-guide.md):
   spawn with-skill and baseline runs in the same turn, assert objectively
   verifiable outputs, iterate. Eval the critical-path leaves first, grow
   coverage with use.

Then run description optimization on the *root* description only — it is the
only one the skill list's triggering sees.

## Phase 7 — Grow

Growth is local; nothing above the touched node changes except its parent's
routing table (one link) and the regenerated registry + maps (one script run).

- **Split** a file past ~500 lines or whose frameworks cluster — promote each
  cluster to a directory group with an index.
- **Merge** a thin index (<~50 lines) whose branch has few files back into its
  parent — a hop that saves no context is pure cost.
- **Add** a file: create it, add one link to the index, re-run registry.
- **Restructure a flat skill**: its sections become the capability map
  (Phase 1); move content into files rather than rewriting it.
