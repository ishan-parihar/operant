---
name: meta-skill-creator
description: How to design, build, grow, and validate meta-skills — hierarchical skill suites where one domain (or multiple domains) is decomposed into a tree of skills, sub-skills, sub-sub-skills, and reference frameworks at unlimited depth. Use this skill whenever the user wants to create a meta-skill, a skill suite, a skill with sub-skills, a "domain pack" of skills, multi-specialist agent capabilities, or wants to convert a flat/oversized skill into a hierarchy, restructure an existing skill tree, add sub-skills to an existing meta-skill, or generate/validate a skill registry. Also use when a single skill has grown past ~500 lines and needs decomposition. Builds every individual node using the skill-creator skill as the foundational base.
---
metadata:
  operant:
    tags: [meta-skills, skill-trees, routers, hierarchy, domain-packs]
    related_skills: [skill-creator, find-skills, holonic-skill-refinement]

# Meta-Skill Creator

Build **meta-skills**: trees of skills that let an agent operate as a routed team of
specialists over a complex domain (or several domains), while loading only the sliver of
instructions the current task actually needs.

This skill defines the architecture and the build process. For writing any *individual*
node — its description, its instructions, its evals — the foundational base is the
**skill-creator** skill (the pool skill `skill-creator` (view with `skill_view(name="skill-creator")`)). Read it before your
first build; everything it says about descriptions, progressive disclosure, writing style,
and the test/iterate loop applies at *every* node of a meta-skill, not just the root.

## The core idea: one recursive node type

A meta-skill is not a special format. It is an ordinary skill whose children are skills.

```
NODE := directory containing
  SKILL.md            required — frontmatter (name, description) + body
  <child-node>/       zero or more child NODEs (same shape, recursively)
  frameworks/         optional — alternative methods the node chooses between
  references/         optional — docs loaded into context as needed
  scripts/            optional — executable helpers (run without loading)
  assets/             optional — files used in output
```

That single rule is the whole hierarchy. There is no separate "meta-skill format",
"sub-skill format", or "sub-sub-skill format" — a node with child nodes is a **router**,
a node without them is a **leaf**, and any leaf can later sprout children without
restructuring anything above it. Depth is unlimited *by construction*, not by
enumeration. This is deliberate: tiered designs (meta → sub → framework, hard-coded)
break the first time a sub-skill gets complex enough to need its own sub-skills. The
recursive model never breaks, because growth is local.

```
trading-systems/                          root router (the only always-in-context description)
├── SKILL.md
├── strategy-research/                    router
│   ├── SKILL.md
│   ├── backtesting/                      router — a sub-skill that earned children
│   │   ├── SKILL.md
│   │   ├── data-hygiene/SKILL.md         leaf (a sub-sub-skill)
│   │   └── walk-forward/SKILL.md         leaf
│   └── signal-design/
│       ├── SKILL.md                      leaf
│       └── frameworks/                   4 alternative methods it selects between
├── execution/SKILL.md                    leaf
└── scripts/registry.py                   copied in from this skill
```

## The routing contract

Only the **root** node's description sits in the agent's always-loaded skill list.
Everything below is reached by the agent **Reading child SKILL.md files** while following
pointers. So the entire hierarchy works only if every router honors this contract:

1. **Route, don't do.** A router's body is a map: what each child covers, when to descend
   into it, and its relative path (`` `strategy-research/backtesting/SKILL.md` ``). Real
   procedure text lives in leaves. When a router accumulates its own how-to content,
   that content is invisible to prompts that route past it and bloats prompts that don't
   need it — move it into a leaf.
2. **Every child is reachable.** If a child directory isn't named in its parent's body,
   no agent will ever read it. The registry script checks this.
3. **Descriptions are the routing surface.** A child's frontmatter description does for
   the parent's routing table exactly what a root description does for Claude's skill
   list: it decides triggering. Write every node's description to skill-creator's
   standard — what it does AND when to use it, pushy about triggers — even at depth 4.
4. **Keep routers skimmable.** A router is read on *every* traversal through its
   subtree, so it pays rent constantly: aim well under 200 lines. Leaves get the normal
   <500-line budget.
5. **Every tree gets a map.** The registry script generates `_map.md` — a compact
   indented index (path + first sentence of each description) of the whole tree,
   automatically sharded per branch once a branch outgrows one comfortable read. The
   root router points to it. The map is the tree's *short-term memory surface*: an
   agent that knows what it needs reads the map and jumps straight to the leaf in two
   reads total; an agent that doesn't walks the routers. Depth then costs navigation
   nothing — it only organizes.

## The scaling invariants

Node count is unbounded; what must stay bounded is **any single thing an agent loads**.
Three invariants keep that true at any scale, which is why depth and branching can grow
without limit:

1. **Bounded fan-out.** Keep every router at 5–12 children. Fewer wastes a hop; more
   overloads one routing decision. Past 12, introduce an intermediate grouping node —
   that's how dense domains become deep trees instead of wide messes.
2. **Sharded maps.** `_map.md` files recurse the same way the tree does: when a branch
   exceeds the shard threshold it gets its own map and the parent map lists one pointer
   line instead of the expansion. Every map stays one bounded read, however many
   thousands of nodes the tree holds.
3. **Delegation boundaries.** Every subtree is self-contained (its resources live at or
   below it, per the lowest-common-ancestor rule), so any subtree path can be handed to
   a subagent along with a slice of the task — the subagent loads only that subtree.
   The tree is not just an instruction hierarchy; it is a **delegation map** for
   multi-agent work, at build time and at run time.

Under these invariants a task costs: one map read + one leaf (jump), or O(depth) short
router reads (walk) — constant-bounded loads regardless of total tree size.

## Runtime navigation protocol

Bake a short **Navigation** section into every root router (template in
`references/templates.md`) so the *using* agent moves through the tree without flooding
its context:

- **Jump** when you know what you need: read `_map.md`, go straight to the leaf.
- **Walk** when you don't: descend router by router, reading only decision guides.
- **Announce** which leaf you are operating under, so switching is deliberate.
- **Re-route on task shift**: when the task changes shape, return to the map and route
  fresh — don't improvise from whatever leaf happens to be in context.
- **Delegate** branch-shaped subtasks: one subagent per branch, given the branch path
  plus its slice of the task.
- **Load ceiling**: the active leaf, its ancestor routers, and at most one framework
  file. If the task seems to need more than that at once, that's a delegation or
  re-routing signal, not a reason to load the tree.

## Build process

### Phase 0 — Load the base
Read the pool skill `skill-creator` (view with `skill_view(name="skill-creator")`). Use its interview guidance, writing
patterns, description rules, and eval loop throughout. This skill only adds what is
specific to hierarchy: decomposition, routing, registry, and growth.

### Phase 1 — Decompose the domain
Interview the user and research the domain (existing docs, similar skills, the
conversation history) until you can write a **capability map**: a flat list of concrete
operations the meta-skill must support, each phrased as a user request ("plan a quarter",
"triage feedback signals", "price a retainer"). Aim for exhaustive-but-flat; do not
design the tree yet. For multi-domain meta-skills, tag each capability with its domain.
Get the user to confirm the map — it is the spec everything else derives from.

Persist it immediately to `_build/plan.md` inside the meta-skill root (template in
`references/templates.md`). A large build outlives any single context window; the plan
file is what lets you — or a fresh agent, or a fleet of subagents — resume without the
conversation. Underscore-prefixed directories are invisible to the registry scanner, so
`_build/` never pollutes validation.

### Phase 2 — Architect the tree
Group capabilities into nodes; nest groups only where the split rules (below) justify it.
Two forces trade off:
- **Deeper** = smaller, more focused contexts per task (cheaper, sharper leaves).
- **Shallower** = fewer routing hops, less risk of a mis-route (every level is a decision
  an agent can get wrong).

Practical defaults: start at 2 levels (root → leaves); add a level where a group breaks
the 5–12 fan-out invariant or a child is really a workflow of distinct steps. Choose
each router's **routing pattern** and say so in its body:
- **Pipeline** — children are ordered phases (gap-analyze → prioritize → define → …).
  State the order and what artifact hands off between phases.
- **Selection** — children are alternatives; give a decision guide ("if X, descend into
  A; if Y, B").
- **Facets** — children are independent aspects usable in any combination (typical for
  multi-domain roots); describe each and let the task pick.

Name nodes with gerunds or noun-phrases matching how users ask ("gap-analyzing",
"pricing-strategy"). Present the tree (as an indented outline with one-line descriptions)
for user sign-off before writing files.

Then, **before any leaf is written**, add two things to `_build/plan.md`:
- the approved tree outline, one line per node, each with a build status checkbox;
- the **conventions block**: shared terminology, artifact formats, directory rules,
  tone. Every node builder reads this block first — it is the only thing standing
  between a 100-leaf parallel build and 100 dialects of the same skill.

### Phase 3 — Build the leaves
Leaves are where the value lives, so build them first — routers are easy to wire once
leaf content is real. Each leaf is a normal skill: write it per skill-creator's guide.

**Scale the build with subagents.** Past roughly a dozen leaves, do not write them
inline — the builder's context becomes the bottleneck long before the tree does. Spawn
one subagent per leaf (or per small branch), each given only: the node's path, its
capability entries and the conventions block from `_build/plan.md`, and the leaf
template. Subagents run in parallel; the orchestrator holds nothing but the plan
checklist, marks nodes complete as they land, and spot-reviews samples against the
conventions. Because all state lives in `_build/plan.md`, an interrupted build resumes
from the checklist — by you or by any fresh agent.

Two leaf-specific patterns:
- **Frameworks** (`frameworks/*.md`): when a leaf offers several alternative methods,
  keep each method in its own file with `When to Use / When NOT to Use / The Method /
  Example / Output` sections, and put a *selection guide* in the leaf's SKILL.md. The
  agent then loads exactly one method, not all of them.
- **Shared resources live at the lowest common ancestor.** If two leaves need the same
  reference doc or script, place it in the nearest router's `references/` or `scripts/`
  and point both leaves at it by relative path. Never duplicate content across nodes —
  duplicated copies drift.

Use the templates in `references/templates.md` for router, leaf, and framework files.

### Phase 4 — Wire the routers (bottom-up)
Write each router's SKILL.md after its children exist, so the routing table describes
reality instead of intention. The root router additionally gets: the one-paragraph
domain overview, the **Navigation** section (jump/walk/delegate protocol — copy from the
root-router template), and the conventions block promoted from `_build/plan.md`.

### Phase 5 — Registry, maps, validation
Copy `scripts/registry.py` from this skill into the meta-skill's root `scripts/` and run
it **after Phase 4** (running mid-build reports the leaves as orphans, correctly — their
routers don't exist yet):

```bash
python scripts/registry.py .           # validate + write _registry.yaml + _map.md files
python scripts/registry.py . --check   # validate only, write nothing
```

One walk produces all three outputs: the recursive `_registry.yaml`, the `_map.md` jump
tables (root map, plus a sharded map inside any branch too big for one read), and the
validation report. All generated — never hand-edit them; hand-maintained indexes rot
into truncated, stale entries. Validation catches the failure modes that silently kill
hierarchies: unreachable children, missing/vague descriptions, name/dir mismatches,
oversized nodes, unreferenced resource files, orphan SKILL.md files nested under
resource directories. Fix every error; treat warnings as review prompts.

### Phase 6 — Test: routing first, then leaves
A meta-skill fails in a way flat skills can't: the right leaf exists but the agent never
reaches it. So test in two layers, using skill-creator's eval machinery for both:
1. **Routing evals.** Write realistic prompts whose correct answer is "which leaf should
   handle this", including near-misses that belong to a *sibling* leaf. Run
   claude-with-the-meta-skill and check which SKILL.md files it actually reads. A
   mis-route means a router's decision guide or a child's description needs sharpening —
   fix the description first; it is the cheaper lever.
2. **Leaf evals.** Standard skill-creator test cases against the leaves that matter
   most. No need to eval every leaf on day one — eval the ones on the critical paths,
   grow coverage as the meta-skill gets real use.

Finally, run skill-creator's **description optimization** on the *root* description only
(it is the only one Claude's triggering sees).

### Phase 7 — Grow
Growth is local; nothing above the touched node changes except its parent's routing
table and the regenerated registry + maps (one script run refreshes both).
- **Split** a leaf when it exceeds ~500 lines, covers more than one concern, or its
  framework files start clustering into groups — promote the clusters to child nodes.
- **Merge** a child back into its parent when it is thin (<~50 lines) and the parent has
  few children — a routing hop that saves no context is pure cost.
- **Add** a leaf: create the node, add one routing line to the parent, re-run registry.
- **Restructure an existing flat skill into a meta-skill**: its sections become the
  capability map (Phase 1); proceed normally, moving content into leaves rather than
  rewriting it.

## Multi-domain meta-skills

Treat each domain as a top-level facet under the root. Cross-domain connections are
where multi-domain meta-skills earn their keep, so make them explicit but keep single
ownership: one node owns a piece of content; other nodes point to it with a relative
path and one line of "when to jump" context. If genuinely cross-domain workflows emerge
(e.g. "pricing" needs both `business/` and `psychology/`), give the *root* router a
short "cross-domain playbooks" section that sequences the leaves — don't create a
duplicate hybrid leaf.

## Reference files

- `references/templates.md` — copy-paste templates: router SKILL.md (with Navigation
  section), leaf SKILL.md, framework file, `_build/plan.md`, plus a worked 3-level
  mini-example.
- `scripts/registry.py` — one walk generates `_registry.yaml` + sharded `_map.md` jump
  tables and validates the tree structure. Copy it into each meta-skill you build.
