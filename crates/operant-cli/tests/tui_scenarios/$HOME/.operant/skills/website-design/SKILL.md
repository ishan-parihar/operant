---
name: website-design
description: "Build and deploy world-class websites with any agent: briefing, direction, tokens, sections, components, content, motion, quality gates. Triggers on build me a website, landing page, portfolio, redesign, audit this site, deploy to higgsfield."
---

# Website Design

A pipeline that turns any AI agent, including small or weak models, into one that ships world-class, aesthetic, feature-rich websites and deploys them. Every phase produces a named artifact the next phase re-reads, so no decision is ever re-made or lost. Trigger via skill-creator conventions; every description is a single-line quoted string.

Pipeline artifacts (the working memory): `DESIGN BRIEF` (audience, vibe, dials) → `DIRECTION` (style, palette, typography, allowed/banned) → `TOKENS` (canonical CSS custom properties, dark mode) → `SECTION MAP` (ordered sections with headline/body/asset/CTA slots) → `DEPLOY SPEC` (host, type, URL, rollback).

## Routing pattern: Pipeline + Facets

Pipeline (in order, never skip a phase):

1. [briefing](references/briefing.md) - extract the DESIGN BRIEF from user message, links, screenshots, repo.
2. [direction](references/direction.md) - choose style, palette, typography BEFORE any HTML or CSS.
3. [system](references/system.md) - turn DIRECTION into TOKENS and wire theme/dark mode.
4. [structure](references/structure.md) - turn TOKENS and DIRECTION into the SECTION MAP.
5. [components](references/components.md) - build each section from block recipes (children: navigation, heroes, feature-sections, social-proof, conversion, forms).
6. [content](references/content.md) - fill every SECTION MAP slot with real copy and real assets.
7. [motion](references/motion.md) - per-section animation specs from the MOTION dial.
8. [quality](references/quality.md) - review-only gates: anti-slop, accessibility, performance, audit, production, preflight (final gate).
9. [deploy](references/deploy.md) - pick host: [higgsfield](references/deploy/higgsfield.md) or [generic](references/deploy/generic.md).

Facets (entry points, not phases):

- Site already exists (URL, repo, files) → start at [redesign](references/redesign.md) BEFORE briefing, then re-enter the pipeline.
- `quality/*` leaves are review-only; load them when checking, not when building.
- [version-control](references/version-control.md) runs ALONGSIDE every phase: commit each artifact as it is produced. Working in a webdev-managed site repo (WEBSITES/<site>) makes this mandatory.

## Children

- [briefing](references/briefing.md) - always first for new builds; produces DESIGN BRIEF with VARIANCE/MOTION/DENSITY dials.
- [direction](references/direction.md) - style, palette, typography via ui-ux-pro-max verified search; produces DIRECTION.md.
- [system](references/system.md) - canonical tokens, theme, dark mode; produces TOKENS.
- [structure](references/structure.md) - ordered sections and slot counts; produces SECTION MAP.
- [components](references/components.md) - block recipes per section kind; loads one leaf per section.
- [content](references/content.md) - real copy, real images, no filler; produces CONTENT.
- [motion](references/motion.md) - animation specs, GSAP skeletons, reduced-motion paths.
- [preflight](references/quality/preflight.md) - final gate; consults every quality sibling, then hands to deploy.
- [deploy](references/deploy.md) - hosting router: higgsfield (website/app/game) or generic static host; produces DEPLOY SPEC.
- [redesign](references/redesign.md) - audit-first entry when a site already exists.
- [version-control](references/version-control.md) - git discipline across all phases; never leave work uncommitted.

## Navigation (root router only)

- Jump: know the phase? Load that child directly; never load the whole tree.
- Walk: only when the decision itself is unclear; read one router at a time.
- Announce the phase you are operating under; when it shifts, say so.
- Re-route: if the loaded leaf contradicts the task, return here and pick again; never improvise a leaf.
- Delegate wide branches to a subagent with the child's path and a sliced task; only that subtree loads.
- Ceiling: a leaf never exceeds its ancestor's rules; this file wins conflicts.

## Conventions

Promoted from `_build/plan-expanded.md`; binding on every node in the subtree.

- Weak-model proofing: decision tables, concrete numbers ("padding 96-128px", never "generous"), 3-8 pass/fail checks per leaf that are countable and greppable. Explain in one line so smarter models generalize; the mechanical rule stands alone.
- Subject-matter grounding: every aesthetic choice anchors to the concrete subject, audience, and primary job (industry, materials, vernacular). A toy store for girls 8-11 and a tool for financial analysts must diverge. REAL content throughout; when content is missing, propose concrete copy via [briefing](references/briefing.md) and confirm.
- Language-agnostic: principles live in semantic HTML + CSS in `references/stack-adapters.md`. Never depend on React, Tailwind, or any framework.
- Canonical tokens: defined in [system](references/system.md), used everywhere: `--bg --surface --text --text-muted --accent --accent-contrast --border --destructive`, `--radius --radius-lg`, `--font-display --font-body --font-mono`, `--space-1..--space-10` (4 8 12 16 24 32 48 64 96 128), `--container`, `--shadow-1 --shadow-2`.
- Dials from [briefing](references/briefing.md): integers 1-10, exact names `VARIANCE` (1 minimal, 10 asymmetric/art), `MOTION` (1 static, 10 cinematic), `DENSITY` (1 sparse, 10 packed). Never invent new dials.
- ui-ux-pro-max protocol: direction/system choices verified against the pack's style/palette/typography data; on empty results, fall back to built-in tables and say so.
- Five artifacts travel as fenced blocks, re-read by name, never from memory.
- Frontmatter: `name:` matches dir, `description:` single-line quoted trigger string.
- Style: no em-dash (use hyphen or period), no emojis, absolute bans live ONLY in [anti-slop](references/quality/anti-slop.md), cross-link siblings root-relative ([heroes](references/components/heroes.md)).

## Non-negotiables

- No em-dash anywhere in any node; the anti-slop leaf's detector greps for it.
- No fabricated logos, testimonials, stats, or brand imagery; [content](references/content.md) proposes, user confirms.
- No placeholder tokens; every section renders through the canonical token set.
- No mixing design systems: one style family, one palette, one type pairing per build.
- No uncommitted artifacts: [version-control](references/version-control.md) commits each phase output before the next phase starts.
- No deploy without [preflight](references/quality/preflight.md) passing.
- Charts explicitly out of scope for this skill. For data visualization (25 chart types in ui-ux-pro-max), query `python ~/.agents/skills/ui-ux-pro-max/scripts/search.py "<data shape>" --domain chart` rather than improvising grid + bars.

OMO_INTERNAL_INITIATOR
