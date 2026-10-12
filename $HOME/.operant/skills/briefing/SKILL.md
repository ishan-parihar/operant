---
name: briefing
description: "Read any brief and produce the DESIGN BRIEF artifact: page kind, audience, vibe words, VARIANCE/MOTION/DENSITY dials, mode, foundation. Phase 1 of website-design: ALWAYS run this before writing any design code, picking any color, or choosing any library. If a site already exists, this leaf detects it and routes to redesign/."
metadata:
  operant:
    tags: [website, design, brief, ux]
---

# Briefing: Read the Brief Before Anything Else

Most bad LLM design output happens because the model jumps to a default aesthetic
instead of reading the room. This phase costs zero code and prevents that. Output
is one artifact: the `DESIGN BRIEF` fenced block that every later phase re-reads.

## Procedure

1. Extract the seven signals (table below) from the user's message, linked URLs,
   screenshots, and any existing repo.
2. Detect the mode (greenfield | preserve | overhaul). If preserve or overhaul,
   read `redesign/SKILL.md` and run its audit BEFORE continuing here.
3. Declare a one-line design read. This happens before any code, always.
4. Set the three dials from the tables below.
5. Pick the foundation: an official design system package OR a named aesthetic family.
6. Emit the `DESIGN BRIEF` block in the exact format at the end of this file.
7. Continue to `direction/SKILL.md`.

If the design read genuinely diverges into two incompatible directions, ask exactly
ONE question (never a multi-question dump), e.g. "Should this feel closer to
Linear-clean or Awwwards-experimental?". If you can infer from context, do not ask.
Declare the read and proceed.

## 1. Signals to extract

| Signal | What to look for |
|---|---|
| Page kind | landing (SaaS / consumer / agency / event), portfolio (dev / designer / studio), editorial or blog, e-commerce, dashboard, docs |
| Product type | entertainment (social, video, music, gaming), tool (scanner, editor, converter), productivity (tasks, notes, calendar), commerce, or hybrid |
| Vibe words | user's own adjectives: "minimalist", "calm", "Linear-style", "Awwwards", "brutalist", "premium", "Apple-y", "playful", "serious B2B", "editorial", "glassy", "dark tech" |
| References | URLs linked, screenshots pasted, products named, competitors mentioned. These outrank your taste |
| Audience | who reads the page and where: B2B procurement panel, design-conscious consumer, recruiter scanning a portfolio, commuter on a phone. The audience picks the aesthetic, not you |
| Existing brand assets | logo, colors, type, photography already in the repo or brief. For redesigns these are starting material, not optional input |
| Quiet constraints | regulated industry, public sector, accessibility-first audience, trust-first commerce, kids' product. These OVERRIDE aesthetic preference |

## 2. The design read (one line, before any code)

State exactly one sentence in this shape:

> "Reading this as: \<page kind> for \<audience>, with a \<vibe> language, leaning toward \<design system or aesthetic family>."

Examples:
- "Reading this as: B2B SaaS landing for technical buyers, with a Linear-style minimalist language, leaning toward a hand-built minimal aesthetic with restrained motion."
- "Reading this as: solo designer portfolio for hiring managers, with an editorial kinetic-type language, leaning toward native CSS plus scroll-driven animation."
- "Reading this as: redesign of a public-sector service site, with a trust-first language, leaning toward govuk-frontend."

Anti-default rule: the read must be derived from the signals, never from the
LLM default aesthetic (purple gradient hero, three equal cards, Inter on slate).
The full ban list lives in `quality/anti-slop/SKILL.md`; do not restate it here,
just do not let a default become the read.

## 3. Mode detection

Misclassifying the mode is the single biggest source of bad redesign output.

| Evidence | Mode |
|---|---|
| No existing site or code, or user says "new site / from scratch" | greenfield |
| Existing site + "modernize, refresh, clean up, keep our brand" | preserve |
| Existing site + "new look, rebrand, start over visually" (content and IA stay) | overhaul |

If an existing site is present and intent is unclear, this counts as your one
allowed question: "Should this redesign preserve the existing brand, or are we
starting visually from scratch?"

For preserve or overhaul: read `redesign/SKILL.md` now. Its audit output feeds
the dials and foundation below. Greenfield continues directly.

## 4. Set the dials

Three integers 1-10, exact names `VARIANCE`, `MOTION`, `DENSITY`. Never invent
aliases like LAYOUT_VARIANCE or ANIM_LEVEL. Every later phase gates decisions
on these values.

### What each value means

**VARIANCE** (layout predictability):
- 1-3: symmetric 12-column grid, equal column widths, equal paddings, centered alignment.
- 4-7: offset layouts, negative-margin overlaps (around -32px), mixed image aspect ratios (4:3 next to 16:9), left-aligned headers over centered content.
- 8-10: asymmetric fractional grids (`grid-template-columns: 2fr 1fr 1fr`), masonry, deliberate large empty zones (left padding up to 20vw).
- Mobile override for 4-10: everything collapses to strict single column below 768px.

**MOTION** (animation intensity):
- 1-3: static. `:hover` and `:active` state changes only. Behave as if `prefers-reduced-motion` is always on.
- 4-7: CSS transitions, 0.3s `cubic-bezier(0.16, 1, 0.3, 1)`, staggered load-in delays, animate only `transform` and `opacity`.
- 8-10: scroll-triggered reveals, parallax, scroll-driven animation. Recipes and hard limits in `motion/SKILL.md`.

**DENSITY** (visual packing):
- 1-3: art gallery. Section vertical padding 128-192px. Expensive, airy.
- 4-7: daily app. Section vertical padding 64-96px.
- 8-10: cockpit. Tight padding, no card boxes, 1px `--border` lines separate data, all numbers set in `--font-mono`.

### Signal to dial values

| Signal in brief | VARIANCE | MOTION | DENSITY |
|---|---|---|---|
| "minimalist / clean / calm / editorial / Linear-style" | 5-6 | 3-4 | 2-3 |
| "premium consumer / Apple-y / luxury / brand" | 7-8 | 5-7 | 3-4 |
| "playful / wild / Dribbble / Awwwards / experimental / agency" | 9-10 | 8-10 | 3-4 |
| "landing page / portfolio / marketing site" (no other signal) | 7-9 | 6-8 | 3-5 |
| "trust-first / public sector / regulated / accessibility-critical" | 3-4 | 2-3 | 4-5 |
| redesign preserve | match existing | existing +1 | match existing |
| redesign overhaul | existing +2 | existing +2 | match existing |

### Use-case presets (pick one, then adjust with the signal table)

| Use case | VARIANCE | MOTION | DENSITY |
|---|---|---|---|
| Landing, SaaS mainstream | 7 | 6 | 4 |
| Landing, agency / creative | 9 | 8 | 3 |
| Landing, premium consumer | 7 | 6 | 3 |
| Portfolio, designer / studio | 8 | 7 | 3 |
| Portfolio, developer | 6 | 5 | 4 |
| Editorial / blog | 6 | 4 | 3 |
| E-commerce / trust-first commerce | 5 | 4 | 5 |
| Dashboard / data product | 3 | 3 | 7 |
| Public-sector service | 3 | 2 | 5 |

Quiet constraints cap the dials: regulated / public-sector / accessibility-first
briefs never exceed VARIANCE 4 or MOTION 3, whatever the vibe words say.

## 5. Pick the foundation

Two mutually exclusive kinds. Record exactly one in the brief.

### 5.A Brief reads as a real design system: use the official package

| Brief reads as | Use | Why |
|---|---|---|
| Microsoft / enterprise SaaS | `@fluentui/react-components` | Official Fluent, accessibility done |
| Google-ish / Material product | `@material/web` + Material 3 tokens | Official, theme-able |
| IBM-style B2B analytics | `@carbon/react` + `@carbon/styles` | Mature data-density patterns |
| Shopify app surfaces | Polaris (web components or React) | Required for Shopify admin |
| Atlassian / Jira-style product | `@atlaskit/*` + `@atlaskit/tokens` | Official Atlassian DS |
| GitHub-style devtool page | `@primer/css` or `@primer/react-brand` | Brand variant for marketing |
| UK public sector | `govuk-frontend` | Legally expected |
| US public sector / trust-first | `uswds` | Same |
| Fast local-business MVP | Bootstrap 5.3 | Boring, fast, works |
| Accessible React foundation | `@radix-ui/themes` | Primitives + polished theme |
| Modern SaaS, you own the code | shadcn/ui | Customisable; never ship default state |
| Indie / small-team marketing | Tailwind v4 utilities | Default for hand-built aesthetics |

Honesty rule: if the brief matches a row, install and use the OFFICIAL package.
Never recreate a system's CSS by hand, and never import a system's tokens then
override 90% of them. One system per project, never two mixed in one tree.

### 5.B Brief is an aesthetic, not a system

No official package exists for these. Name the family, record it as
`aesthetic:<family>`, and let `direction/SKILL.md` turn it into palette, type,
and effects: `minimal`, `editorial`, `brutalist`, `glassmorphism`, `bento`,
`dark-tech`, `aurora`, `kinetic-type`, `playful`, `luxury`. If the user names a
vendor-only effect (e.g. Apple Liquid Glass), record the nearest family and note
it is an approximation, there is no official web package for it.

## 6. Output format

Emit exactly this fenced block, all nine lines, then keep it in context for
every later phase:

```
DESIGN BRIEF
page kind: <landing-saas | landing-consumer | landing-agency | portfolio-designer | portfolio-dev | editorial | ecommerce | dashboard | docs | event>
audience: <who, plus one usage-context word>
vibe words: <2-4 adjectives, user's own where available>
design read: "Reading this as: <page kind> for <audience>, with a <vibe> language, leaning toward <foundation>."
dials: VARIANCE=<n> MOTION=<n> DENSITY=<n>
mode: <greenfield | preserve | overhaul>
foundation: <system:<package> | aesthetic:<family>>
constraints: <quiet constraints, or none>
```

## Worked example

Brief received: "Landing page for our AI meeting-notes SaaS. Should feel calm and
premium, like Linear. Buyers are engineering managers evaluating tools for their team."

Signals: page kind landing-saas; product type productivity tool; vibe words calm,
premium, Linear-style; reference Linear; audience technical B2B buyers; no existing
assets; no quiet constraints. Mode: no existing site, greenfield. Dials: preset
Landing SaaS (7/6/4) adjusted by the "calm / Linear-style" row down to 6/4/3.
Foundation: no system row matches; this is a minimal aesthetic on Tailwind v4.
No ambiguity, so no question asked.

```
DESIGN BRIEF
page kind: landing-saas
audience: engineering managers evaluating team tools, B2B
vibe words: calm, premium, Linear-style
design read: "Reading this as: B2B SaaS landing for engineering managers, with a calm Linear-style minimalist language, leaning toward a hand-built minimal aesthetic."
dials: VARIANCE=6 MOTION=4 DENSITY=3
mode: greenfield
foundation: aesthetic:minimal
constraints: none
```

Next: `direction/SKILL.md`.

## Checks

1. A fenced `DESIGN BRIEF` block exists with all nine lines, none blank.
2. The design read is one sentence in the exact "Reading this as:" shape and was stated before any code, tokens, or colors.
3. Dials line matches `VARIANCE=<int> MOTION=<int> DENSITY=<int>`, each value 1-10, no other dial names anywhere.
4. mode is exactly one of greenfield, preserve, overhaul; if preserve or overhaul, `redesign/SKILL.md` was read before the brief was emitted.
5. foundation is exactly one value: `system:<package>` from the 5.A table or `aesthetic:<family>` from the 5.B list, never both, never two systems.
6. If constraints include regulated, public-sector, or accessibility-first: VARIANCE <= 4 and MOTION <= 3.
7. Zero or one clarifying question was asked, never more.
8. This phase wrote no CSS, no hex values, no component code.
