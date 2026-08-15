---
name: components
description: "Block recipes for every website section: header/nav/footer, heroes, feature sections and bento grids, social proof, pricing and CTAs, forms. Use whenever building or fixing any visible section of a page; the SECTION MAP artifact names which child leaf implements each section."
metadata:
  operant:
    tags: [website, components, html, css]
---

# Components

Recipes for the visible blocks of a page. Every leaf assumes the pipeline artifacts
exist (DESIGN BRIEF, DIRECTION, TOKENS, SECTION MAP) and expresses its recipes as
semantic HTML + canonical CSS tokens, so the same recipe compiles to any stack via
`../references/stack-adapters.md`.

## Routing pattern: Selection

The SECTION MAP tags each section with the child leaf that implements it. If working
without a SECTION MAP (single-component request), pick by what is being built:

- Header, nav, mobile menu, footer, breadcrumbs -> `navigation/SKILL.md`
- Hero (any top-of-page section) -> `heroes/SKILL.md`
- Features, bento grid, zigzag, stats, steps, comparisons, FAQs/accordions, long
  lists, spec sheets -> `feature-sections/SKILL.md`
- Logo wall, testimonials, quotes, case-study teasers, reviews -> `social-proof/SKILL.md`
- Pricing, CTA bands, signup/waitlist blocks, buttons -> `conversion/SKILL.md`
- Any input, contact form, checkout, multi-step flow, validation -> `forms/SKILL.md`

## Children

- `navigation/SKILL.md` - header spec (height caps, single-line rule, active states,
  mobile menu) and footer-as-sitemap. Descend for any site chrome.
- `heroes/SKILL.md` - six hero paradigms gated by VARIANCE/MOTION plus the hard hero
  rules (element caps, font-scale bands, viewport fit). Descend for any hero.
- `feature-sections/SKILL.md` - grid specs by content shape, bento cell rules, zigzag
  caps, FAQ accordions, long-list alternatives. Descend for any mid-page content section.
- `social-proof/SKILL.md` - logo sources, testimonial layout by count, realistic
  attribution. Descend for any trust content.
- `conversion/SKILL.md` - pricing tiers, CTA intent dedup, full button state system.
  Descend for anything meant to convert.
- `forms/SKILL.md` - field anatomy, validation timing, states table, submit lifecycle.
  Descend for any user input.

## Shared rules

Content for every slot (copy, images, icons) comes from `../content/SKILL.md`.
Animation for any block comes from `../motion/SKILL.md`. Bans live in
`../quality/anti-slop/SKILL.md`. Each leaf ends with mechanical Checks; run them
before leaving the leaf.
