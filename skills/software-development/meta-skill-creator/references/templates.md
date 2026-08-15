# Meta-Skill Node Templates

Three templates — router, leaf, framework — plus a worked mini-example. Every node uses
the same frontmatter shape; only the body differs by role. Replace all `<angle-bracket>`
text. Keep descriptions single-line quoted strings so the registry parser and human
readers both stay happy.

## Router node — `SKILL.md`

```markdown
---
name: <dir-name>
description: "<What this subtree covers, then triggers: Use when the user wants to <capability 1>, <capability 2>, <capability 3>. Triggers on '<phrase>', '<phrase>'.>"
---

# <Title>

<One short paragraph: what this subtree is for and the mental model that unifies it.>

## Routing pattern: <Pipeline | Selection | Facets>

<Pipeline: show the phase order and the artifact handed between phases.
 Selection: give a decision guide — "if <situation> → <child>".
 Facets: one line per child on what aspect it owns.>

## Children

- `<child-a>/SKILL.md` — <what it does>. Descend when <condition>.
- `<child-b>/SKILL.md` — <what it does>. Descend when <condition>.

## Navigation (root router only)

- Know what you need? Read `_map.md` and jump straight to that leaf.
- Not sure? Walk: descend one router at a time, reading only decision guides.
- State which leaf you are operating under; when the task shifts, return to
  `_map.md` and re-route instead of improvising from a stale leaf.
- Task spans branches? Delegate: one subagent per branch, give it the branch
  path + its slice of the task; it loads only that subtree.
- Load ceiling: active leaf + its ancestor routers + at most one framework file.

## Conventions (root router only)

<Cross-cutting facts every subtree assumes: directory locations, data formats,
naming rules. Promote these from `_build/plan.md`. Delete on non-root routers.>
```

Router rules of thumb: under 200 lines; 5–12 children (past 12, add a grouping node);
no procedure text (that belongs in a leaf); every child directory named at least once
with its relative path.

## Leaf node — `SKILL.md`

```markdown
---
name: <dir-name>
description: "<What this leaf does, then triggers: Use when the user wants to <task>. Triggers on '<phrase>', '<phrase>'.>"
---

# <Title>

<The actual procedure. Imperative voice. Explain *why* steps matter, per
skill-creator's writing guide. Include an example of input → output.>

## Framework selection guide   <!-- only if frameworks/ exists -->

- **<Framework A>** (`frameworks/<a>.md`) — when <situation>.
- **<Framework B>** (`frameworks/<b>.md`) — when <situation>.

## Output

<The exact artifact this leaf produces and where it goes.>
```

## Framework file — `frameworks/<name>.md`

```markdown
# <Framework Name>

## When to Use
<Situation that calls for this method.>

## When NOT to Use
<The near-miss situations where a sibling framework is better — name it.>

## The Method
<Steps, scoring axes, formulas. Concrete enough to execute without other files.>

## Example
<A short worked example with realistic values.>

## Output
<What artifact results.>
```

## Build plan — `_build/plan.md`

The persistent build state. Underscore-prefixed directories are invisible to
`registry.py`, so this never pollutes validation. Every node-building subagent receives
the conventions block plus its own node's lines — never the whole conversation.

```markdown
# Build plan: <meta-skill-name>

## Conventions
<Terminology, artifact formats, directory rules, tone. Written in Phase 2,
BEFORE any leaf. This block is the spec that keeps parallel builders coherent.>

## Capability map
- [ ] <capability as a user request> → <node-path>
- [ ] <capability as a user request> → <node-path>

## Tree
- [ ] <node-path> (<router|leaf>) — <one-line description>
  - [ ] <node-path>/<child> (leaf) — <one-line description>

## Status log
<One line per session: date, what was completed, what's next.>
```

## Worked mini-example (3 levels)

`content-engine/` — a meta-skill for producing published content.

```
content-engine/
├── SKILL.md                    router (Pipeline: research → draft → distribute)
├── researching/
│   └── SKILL.md                leaf — gather sources, produce a research brief
├── drafting/
│   ├── SKILL.md                router (Selection: by content type)
│   ├── longform/
│   │   ├── SKILL.md            leaf (sub-sub-skill)
│   │   └── frameworks/
│   │       ├── essay-arc.md
│   │       └── tutorial-format.md
│   └── shortform/
│       └── SKILL.md            leaf (sub-sub-skill)
├── distributing/
│   └── SKILL.md                leaf — channel choice + scheduling
├── references/
│   └── voice-guide.md          shared by drafting/* leaves; it sits at the root only
│                               because distributing/ also cites it — otherwise its
│                               lowest common ancestor would be drafting/
└── scripts/
    └── registry.py
```

Routing walk-through for the prompt *"turn these notes into a tutorial post"*:
root SKILL.md (pipeline says drafting) → `drafting/SKILL.md` (selection guide says
longform) → `longform/SKILL.md` (framework guide says `tutorial-format.md`) → execute.
Three reads, ~150 loaded lines, out of a tree that could hold thousands.
