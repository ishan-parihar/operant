# Meta-Skill Node Templates

Templates for the nested-references model: root `SKILL.md`, index file, leaf
file, framework file, `_build/plan.md`, plus a worked mini-example. Replace all
`<angle-bracket>` text. Everything below root is plain markdown — no frontmatter,
no name/description fields. Keep pointers as markdown links so the validator can
resolve them.

## Root `SKILL.md`

```markdown
---
name: <dir-name>
description: "<What the whole subtree does, then pushy triggers: Use when the
user wants <capability 1>, <capability 2>, <capability 3>. Triggers on
'<phrase>', '<phrase>'.>"
---

# <Title>

<One paragraph: domain overview + the mental model that unifies it.>

## Navigation

- Know what you need? Read `_map.md` and jump straight to that file.
- Not sure? Walk: descend index files, reading only decision guides.
- State which file you are operating under; when the task shifts, return to
  `_map.md` and re-route instead of improvising from a stale file.
- Task spans branches? Delegate: one subagent per branch — give it the branch
  path + its slice of the task; it loads only that subtree.
- Load ceiling: root + one index chain + one node + at most one framework file.

## Routing pattern: <Pipeline | Selection | Facets>

<Pipeline: phase order + the artifact handed between phases.
 Selection: decision guide — "if <situation> → [label](<file>.md)".
 Facets: one line per entry on what aspect it owns.>

## Routing table

- [<child-a> — descend when <condition>](<child-a>.md) — <what it covers>.
- [<child-b> — descend when <condition>](<child-b>/<index-or-leaf>.md) — <what it covers>.

## Conventions

<Cross-cutting facts every file assumes: directory locations, artifact formats,
naming rules, tone. Promoted from `_build/plan.md`. Delete from non-root files
only if truly nothing below needs them.>
```

Root rules of thumb: under 200 lines; 5–12 routing-table entries (past 12, add a
grouping directory with its own index); no procedure text (that belongs in files);
every file under `references/` reachable from this file by links.

## Index file — `references/<dir>.md`

```markdown
# <Title>

<One-line summary: what this branch covers and when to descend into it.>

## Routing pattern: <Pipeline | Selection | Facets>

<Decision content for the entries below — this is where an intermediate
router's guidance now lives.>

## Files

- [<file-a> — descend when <condition>](<dir>/<file-a>.md) — <what it covers>.
- [<file-b> — descend when <condition>](<dir>/<file-b>.md) — <what it covers>.

## Next

- Back to the [root routing table](../SKILL.md) when the task shifts.
```

## Leaf file — `references/<dir>/<name>.md`

```markdown
# <Title>

<One-line summary — the routing sentence. Written last, after the file works.>

<The actual procedure. Imperative voice. Explain *why* steps matter.
Include an Input → Output example.>

## Framework selection guide   <!-- only if frameworks/ exists -->

- **<Framework A>** ([frameworks/<a>.md](frameworks/<a>.md)) — when <situation>.
- **<Framework B>** ([frameworks/<b>.md](frameworks/<b>.md)) — when <situation>.

## Output

<The exact artifact this file produces and where it goes.>

## Next

- <Related file — jump when <condition>> ([../<sibling>.md](../<sibling>.md)).
```

## Framework file — `references/<dir>/<node>/frameworks/<name>.md`

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

The persistent build state. Underscore-prefixed paths are invisible to
`registry.py`, so this never pollutes validation. Every node-building subagent
receives the conventions block plus its own file's lines — never the whole
conversation.

```markdown
# Build plan: <meta-skill-name>

## Conventions
<Terminology, artifact formats, directory rules, tone. Written in Phase 2,
BEFORE any file. This block is the spec that keeps parallel builders coherent.>

## Capability map
- [ ] <capability as a user request> → <file-path>
- [ ] <capability as a user request> → <file-path>

## Tree
- [ ] SKILL.md (root) — <one-line description>
  - [ ] references/<index>.md (index) — <one-line description>
    - [ ] references/<index>/<leaf>.md (leaf) — <one-line description>

## Status log
<One line per session: date, what was completed, what's next.>
```

## Worked mini-example (3 levels)

`content-engine/` — one skill over a nested references tree.

```
content-engine/
├── SKILL.md                    root: Navigation + Pipeline routing table
│                               (research → draft → distribute)
├── references/
│   ├── researching.md          leaf — gather sources, produce a research brief
│   ├── drafting.md             index (Selection: by content type)
│   │   ├── longform.md         leaf + framework selection guide
│   │   │   └── frameworks/
│   │   │       ├── essay-arc.md
│   │   │       └── tutorial-format.md
│   │   └── shortform.md        leaf
│   ├── distributing.md         leaf — channel choice + scheduling
│   ├── voice-guide.md          shared: cited by drafting/* and distributing —
│   │                           sits at references/ (the lowest common ancestor)
│   └── _map.md                 generated jump table
├── scripts/
│   └── registry.py
└── _build/
    └── plan.md
```

Routing walk-through for the prompt *"turn these notes into a tutorial post"*:

```text
root SKILL.md (pipeline says draft) → references/drafting.md (selection says
longform) → references/drafting/longform.md (framework guide says
tutorial-format.md) → execute.
```

Four bounded reads, one skill loaded, out of a
tree that could hold thousands of files.
