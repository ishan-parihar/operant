---
name: heroes
description: "Build the hero section. Use IMMEDIATELY when the SECTION MAP reaches the hero, or whenever a hero looks templated, overflows the viewport, or has no real visual. Picks the hero paradigm from VARIANCE, MOTION, and page kind, then enforces the hard hero build spec."
metadata:
  operant:
    tags: [hero, header, landing, above-the-fold]
---

# Heroes

The hero is one moment: value prop, one visual, one primary action. It is not a
feature list, a trust wall, or a pricing teaser. Build it to fit the first
viewport and pass the checks below before moving to the next section.

## Procedure

1. Read the `DESIGN BRIEF` (dials, page kind) and `DIRECTION` (style, effects).
2. Pick ONE paradigm from the table below. When two rows qualify, take the
   upper row. When unsure, take Asymmetric Split.
3. Write the hero copy FIRST: headline, subtext, CTA labels. Apply the copy
   limits in the build spec. Cutting copy now is cheaper than shrinking fonts later.
4. Secure the visual (rule below) BEFORE writing layout CSS, so font scale and
   asset size are planned together.
5. Build with semantic HTML + tokens, add the mobile collapse in the same file.
6. Run `## Checks`. Fix fails before continuing the pipeline.

Framework note: for Tailwind, React, Vue, or shadcn output, map these token
names and recipes via `references/stack-adapters.md`. The structure and limits
here do not change per stack.

## Paradigm selection

| Paradigm | Pick when | VARIANCE | MOTION | Page kind |
|---|---|---|---|---|
| Asymmetric Split | Real asset + copy, the default hero | 3-7 | any | SaaS, product, marketing, local business |
| Product Screenshot | The UI itself is the proof | 2-6 | 1-5 | app, dashboard, devtool |
| Media / Video | Photography or film carries the brand | any | 4+ for video, else image | travel, food, fashion, venue, event |
| Editorial Manifesto | Type-led statement, no hero asset | 5-9 | 1-4 | portfolio, studio, editorial, manifesto |
| Kinetic Type | Animated typography IS the visual | 7+ | 7+ | agency, experimental, launch teaser |
| Scroll-Pinned | Hero pins while story scrolls behind | 6+ | 8+ | narrative product reveal, campaign |

Hard gates on the table:

- MOTION <= 3 forbids Kinetic Type and Scroll-Pinned. Why: motion-led heroes at
  a static dial read as broken, not restrained.
- Editorial Manifesto still requires 2-3 real images elsewhere on the page. A
  text-only page is incomplete work, not minimalism.
- Product Screenshot requires a REAL screenshot, generated image, or a real
  working component preview. Div-built fake UI is banned, see
  `quality/anti-slop/SKILL.md`.
- Docs and content-first pages skip Scroll-Pinned. Why: it taxes the scroll
  budget readers need for content.

## Build spec (hard rules, failing any one is shipping broken work)

**Text stack: max 4 elements, in this order, nothing else.**

1. Eyebrow (small uppercase label) OR brand strip OR neither. Zero or one.
2. Headline. Max 2 rendered lines at desktop width.
3. Subtext. Max 20 words AND max 4 rendered lines. If the value prop does not
   fit in 20 words, the value prop is unclear, not the rule too tight.
4. CTA row. Exactly 1 primary + at most 1 secondary. Labels max 3 words, no wrap.

Everything else moves to its own section directly BELOW the hero: logo wall,
trust micro-strip, tagline under CTAs, pricing teaser, feature bullets,
avatar row. One small text element per hero, max: if you have an eyebrow AND
want a tagline, drop the tagline.

Hero-specific tells (no version-label eyebrows like `BETA` or `v2.0`, no
"Brand · No. 01" sub-eyebrows, no mono-caps decoration strip across the hero
bottom) are absolute bans listed in `quality/anti-slop/SKILL.md`. Check that
file, do not re-derive the list.

**Font scale is planned WITH the asset, not after it.**

| Headline length | Desktop font size | Why |
|---|---|---|
| 3-5 words | 60-72px | short line can carry poster scale next to a large asset |
| 6-9 words | 36-56px | longer line at poster scale wraps to 3+ lines |
| 10+ words | do not size it, cut the copy | a 4-line headline is always a copy error |

Use `clamp()` so the same rule holds down to tablet, e.g.
`font-size: clamp(36px, 5vw, 64px)`.

**Geometry.**

- Hero top padding at desktop: max 96px (`var(--space-9)`). More makes content
  float mid-viewport and reads as a bug. Need more presence? Increase font or
  asset scale, never top padding.
- Hero fits the initial viewport: headline, subtext, and CTAs all visible
  without scrolling at 1280x800 desktop and 390x844 mobile.
- Full-viewport heroes use `min-height: 100dvh`, NEVER `100vh`. Why: `100vh`
  overshoots under mobile browser chrome and pushes the CTA off screen. Capping
  is fine: `min-height: min(100dvh, 860px)`.

**Every hero gets a real visual.** One of, in priority order:

1. A generated image (use any available image tool, correct aspect ratio).
2. A real photo or real product screenshot URL.
3. A real component preview: an actual working mini-version of the UI.
4. A deliberate typographic composition (Editorial Manifesto or Kinetic Type
   paradigm chosen on purpose, not as a fallback for missing images).

Text plus a gradient blob is a placeholder, not a hero. Fake screenshots built
from styled divs are banned, see `quality/anti-slop/SKILL.md`. If no image
source exists, leave a labeled slot
(`<!-- TODO: hero product photo, 1600x1200 -->`) and tell the user.

## Worked example: Asymmetric Split hero

Brief: devtool landing page, VARIANCE 5, MOTION 3, DENSITY 4. Headline is 4
words, so the 60-72px band applies.

```html
<section class="hero">
  <div class="hero-copy">
    <p class="hero-eyebrow">Continuous deploy for monorepos</p>
    <h1>Ship every branch safely</h1>
    <p class="hero-sub">Preview environments and rollbacks for every pull
      request, with zero pipeline config.</p>
    <div class="hero-ctas">
      <a class="btn btn-primary" href="/signup">Start free</a>
      <a class="btn btn-secondary" href="/demo">Watch demo</a>
    </div>
  </div>
  <div class="hero-media">
    <img src="/img/deploy-dashboard.png" width="1240" height="930"
         alt="Deploy dashboard showing three preview environments" />
  </div>
</section>
```

```css
.hero {
  display: grid;
  grid-template-columns: 6fr 5fr;
  gap: var(--space-8);
  align-items: center;
  max-width: var(--container);
  margin-inline: auto;
  min-height: min(100dvh, 860px);           /* never 100vh */
  padding: var(--space-9) var(--space-6);   /* top capped at 96px */
}
.hero-eyebrow {
  font-family: var(--font-mono);
  font-size: 12px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--text-muted);
  margin: 0 0 var(--space-4);
}
.hero h1 {
  font-family: var(--font-display);
  font-size: clamp(40px, 5vw, 68px);        /* 4-word headline: 60-72px band */
  line-height: 1.05;
  letter-spacing: -0.02em;
  color: var(--text);
  margin: 0 0 var(--space-5);
  max-width: 14ch;                          /* holds the 2-line cap */
}
.hero-sub {
  font-family: var(--font-body);
  font-size: 18px;
  line-height: 1.55;
  color: var(--text-muted);
  max-width: 44ch;
  margin: 0 0 var(--space-6);
}
.hero-ctas { display: flex; gap: var(--space-4); flex-wrap: wrap; }
.btn {
  display: inline-flex;
  align-items: center;
  padding: var(--space-3) var(--space-6);
  border-radius: var(--radius);
  font-family: var(--font-body);
  font-weight: 600;
  white-space: nowrap;                      /* CTA labels never wrap */
}
.btn-primary { background: var(--accent); color: var(--accent-contrast); }
.btn-secondary { border: 1px solid var(--border); color: var(--text); }
.hero-media img {
  width: 100%;
  height: auto;
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-2);
}

/* Mobile collapse, declared in the same component */
@media (max-width: 767px) {
  .hero {
    grid-template-columns: 1fr;
    gap: var(--space-6);
    min-height: auto;
    padding: var(--space-7) var(--space-4) var(--space-8);
  }
  .hero h1 { font-size: clamp(32px, 9vw, 40px); }
  .hero-sub { font-size: 16px; }
}
```

The logo wall ("Trusted by ...") is the NEXT `<section>` below this one, never
inside `.hero`.

## Checks

1. Hero text elements counted in markup: at most 4 (eyebrow or brand strip,
   headline, subtext, CTA row). Zero items from the move-below list inside the
   hero (logo wall, trust strip, tagline under CTAs, pricing teaser, feature
   bullets, avatar row).
2. Headline renders on at most 2 lines at 1280px width.
3. Subtext word count <= 20.
4. CTA row: exactly 1 primary, 0 or 1 secondary, no label wraps at desktop.
5. Computed hero top padding at desktop <= 96px.
6. Grep hero CSS for `100vh`: zero matches. Full-viewport heights use `100dvh`.
7. Hero contains a real `<img>`, `<video>`, real component preview, or is a
   deliberately chosen typographic paradigm. No div-built fake UI and no hero
   tells from `quality/anti-slop/SKILL.md`.
8. Headline, subtext, and primary CTA all visible without scroll at 1280x800
   and 390x844.
