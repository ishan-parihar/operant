---
name: meta-skill-creator
description: How to design, build, grow, and validate meta-skills — one skill over a nested references infrastructure, where a domain decomposes into an unlimited-depth tree of plain markdown reference files loaded progressively, never routed between skills. Use this skill whenever the user wants to create a meta-skill, a skill suite, a skill with sub-skills, a "domain pack" of skills, multi-specialist agent capabilities, or wants to convert a flat/oversized skill into a hierarchy, restructure an existing skill tree, add sub-skills to an existing meta-skill, or generate/validate a skill registry. Also use when a single skill has grown past ~500 lines and needs decomposition. Ships with its own writing base, templates, and validator — no other skill required.
---

# Meta-Skill Creator

Build **meta-skills**: one registered skill whose instructions live in a **nested
references tree** — a complex domain (or several) decomposed into plain markdown
nodes at unlimited depth, loaded only as the current task needs them. The agent
operates as a routed team of specialists while paying context only for the sliver
of instruction in play.

## The core idea: one skill, files all the way down

A meta-skill is not a special format. It is an ordinary skill whose `references/`
directory is a tree.

```
NODE := a plain .md file under references/, grouped by directories
  <root>/SKILL.md     the skill — frontmatter (name, description) + body:
                      routing table, navigation protocol, conventions.
                      The ONLY frontmatter and the ONLY registered skill.
  references/         the nested node tree — depth unlimited by construction
    <node>.md         a node: H1 + one-line summary + procedure
    <dir>.md          index file for the sibling <dir>/ group (its router)
    <dir>/<node>.md   deeper nodes — same shape, recursively
    <dir>/<node>/     a branch dense enough to need its own directory
      frameworks/*.md alternative methods a node selects between
    _map.md           generated jump table (sharded per branch)
  scripts/ assets/    executable helpers / files used in output
  _build/             working state — underscore-prefixed, invisible to the validator
```

There is no "sub-skill format" and no second tier of skill identities: children
are **files**, not skills. Growth is local — a leaf sprouts a directory, an index
gains a line — nothing above the touched node restructures. Tiered designs
(meta → sub → framework, hard-coded) break the first time a branch gets complex;
documents linking to documents cannot break that way.

## The references contract

Only the **root** `SKILL.md` sits in the agent's always-loaded skill list. The
hierarchy works only if every node honors this contract:

1. **One skill per meta-skill.** Only the root has `SKILL.md` and frontmatter.
   A `SKILL.md` below root is an error — a regression to the routing model, and
   the validator rejects it.
2. **A node is a file.** `references/**/*.md`, nested to unlimited depth. No
   frontmatter below root: the `H1` is the name; the first line after it is a
   one-line summary — this absorbs what a child description used to do.
3. **Pointers are markdown links.** A parent routes by linking:
   `[label — descend when X](relative/path.md)` (file-relative, with the skill
   root as fallback). The validator resolves pointers **both ways**: a node
   unreachable from root is an error; a link to a missing file is an error.
   Backticked root-relative paths (with an extension) also count as pointers —
   handy for `scripts/` and `assets/`. Prose alone does not: an unlinked file
   is invisible.
4. **Every directory group gets an index file.** `<dir>.md` beside `<dir>/`
   holds that branch's routing table (pipeline order, selection guide, or facet
   list — name the pattern in the file). This is where an intermediate router's
   decision text lives now; there is no intermediate SKILL.md to put it in.
5. **Navigation surface = the root routing table + the generated `_map.md`.**
   Walk = O(depth) short reads; jump = one map read then the file. Load
   ceiling: root + one index chain + one node + at most one framework file.
   Needing more is a delegation or re-routing signal, not a reason to load the
   tree.
6. **No cross-skill routing for capability.** Everything needed to build a node
   — writing rules, description standards, eval loop — is vendored in
   [`references/writing-guide.md`](references/writing-guide.md). Mentions of
   other skills are discovery only, never a required step. The skill ships
   complete in its own directory.
7. **Every tree self-validates.** `python scripts/registry.py <root> --check`
   exits 0 on the tree it belongs to — including this skill's own directory.

## The scaling invariants

Node count is unbounded; what must stay bounded is **any single thing an agent
loads**:

1. **Bounded fan-out.** Keep every routing table at 5–12 entries. Fewer wastes a
   hop; more overloads one decision. Past 12, add a grouping directory with its
   own index — how dense domains become deep trees instead of wide messes.
2. **Sharded maps.** `_map.md` recurses like the tree: a branch bigger than the
   shard threshold gets its own map; the parent lists one pointer line. Every
   map stays one bounded read at any tree size.
3. **Delegation boundaries.** Every subtree is self-contained (shared resources
   at the lowest common ancestor), so any `references/<branch>/` path can be
   handed to a subagent with its slice of the task. The tree is an instruction
   hierarchy *and* a delegation map.

Under these invariants a task costs: one map read + one file (jump), or O(depth)
short reads (walk) — constant-bounded loads at any tree size.

## Runtime navigation protocol

Bake this **Navigation** section into the root (template in
[`references/templates.md`](references/templates.md)):

- **Jump** when you know what you need: read `_map.md`, go straight to the file.
- **Walk** when you don't: descend index files, reading only decision guides.
- **Announce** which file you are operating under, so switching is deliberate.
- **Re-route on task shift**: return to the map and route fresh — don't
  improvise from whatever file happens to be in context.
- **Delegate** branch-shaped subtasks: one subagent per branch, given the
  branch path plus its slice of the task.
- **Load ceiling**: root + one index chain + one node + one framework file.

## Build process

The full procedure lives in
[`references/build-process.md`](references/build-process.md) — interview,
architecture, parallel leaf building, wiring, validation, testing, growth. The
phases, one line each:

0. **Load the base** — read [`references/writing-guide.md`](references/writing-guide.md);
   nothing external is required.
1. **Decompose** — capability map confirmed with the user, persisted to `_build/plan.md`.
2. **Architect** — group into files, pick a routing pattern per index, get
   sign-off, write the conventions block.
3. **Build leaves first** — one file per capability; subagents past ~a dozen;
   frameworks pattern; shared resources at the lowest common ancestor.
4. **Wire indexes bottom-up** — routing tables describe reality; the root gains
   Navigation + conventions.
5. **Registry** — copy and run `scripts/registry.py` after Phase 4; fix every
   error, review warnings.
6. **Test navigation, then leaves** — check which files the agent actually
   Reads; fix one-line summaries first.
7. **Grow** — split/merge/add/restructure; local change plus one registry run.

## Multi-domain meta-skills

Treat each domain as a top-level facet under the root. Cross-domain connections
are where multi-meta-skills earn their keep — make them explicit but keep
single ownership: one file owns a piece of content; others link to it with one
line of "when to jump" context. Genuinely cross-domain workflows (pricing needs
both `business/` and `psychology/`) get a short "cross-domain playbooks"
section in the root that sequences the files — never a duplicate hybrid file.

## Reference files

- [`references/build-process.md`](references/build-process.md) — the full
  Phase 0–7 procedure.
- [`references/writing-guide.md`](references/writing-guide.md) — the vendored
  writing base: intent interview, description rules, progressive disclosure,
  writing style, eval loop (distilled from skill-creator; attribution inside).
- [`references/templates.md`](references/templates.md) — copy-paste templates:
  root SKILL.md, index file, leaf file, framework file, `_build/plan.md`, plus
  a worked 3-level mini-example.
- [`scripts/registry.py`](scripts/registry.py) — one walk generates
  `_registry.yaml` + sharded `_map.md` jump tables and validates the tree.
  Copy it into each meta-skill you build.
