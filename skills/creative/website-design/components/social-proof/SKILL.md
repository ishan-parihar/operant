---
name: social-proof
description: "Build social proof: logo walls, testimonials, pull quotes, case-study teasers, and review strips. Use whenever the SECTION MAP calls for trust, customers, testimonials, or reviews, and ALWAYS before placing any brand logo or quote on the page."
metadata:
  operant:
    tags: [testimonials, logos, social-proof, trust]
---

# Social proof

Credibility sections live or die on believability. Fake-looking logos, egg avatars, and
essay-length quotes destroy trust faster than having no social proof at all.

## Procedure

1. Read the SECTION MAP: which proof assets exist (logos, quotes, ratings, case studies)?
2. Place each asset by the placement table. Never stuff proof into the hero
   (hero rules: `components/heroes/SKILL.md`).
3. Build with the specs below, canonical tokens only.
4. Replace every generic name, avatar, and round number with realistic data
   (rules: `content/SKILL.md`).
5. Run `## Checks`.

Framework note: recipes are plain HTML + CSS custom properties.
Stack mappings live in `references/stack-adapters.md`.

## Placement table

| Asset | Placement | Why |
|---|---|---|
| Logo wall | Own section directly under the hero | Trust lands before claims do |
| Testimonials | Mid-page, after the main feature sections | Proof reads best right after the claims it backs |
| Review strip (rating + count) | Adjacent to pricing or the final CTA | Cuts decision anxiety at the commit point |
| Case-study teasers | After features, before pricing | Depth for evaluators who are almost convinced |

One proof section per slot. Two logo walls or two testimonial sections on one page is a fail.

## Logo wall spec

- **Real brands: real SVG logos.** Source: Simple Icons CDN,
  `https://cdn.simpleicons.org/{slug}/{hex}` (e.g. `https://cdn.simpleicons.org/stripe/8b8f98`).
  The hex has no `#`. Set it to the resolved value of `--text-muted`; a URL cannot read CSS
  custom properties, so this is the single allowed raw hex outside token definitions.
- **Invented brands: invent a mark too.** Never a plain text wordmark in a row. Monogram recipe:

```html
<svg viewBox="0 0 48 48" width="32" height="32" role="img" aria-label="Halvard">
  <circle cx="24" cy="24" r="22" fill="none" stroke="currentColor" stroke-width="2"/>
  <text x="24" y="31" text-anchor="middle" font-size="22"
        font-family="var(--font-display)" fill="currentColor">H</text>
</svg>
```

  Parent sets `color: var(--text-muted)`, so `currentColor` renders in both themes.
- **Single-color treatment.** All logos in one muted color at rest. Never a row of
  full-color logos by default. Optional at MOTION >= 4: hover restores brand color
  (`filter: grayscale(1) opacity(.7)` at rest, `filter: none` on hover, 150ms transition).
- **Both themes.** Logos must read on light and dark: use `currentColor` for inline SVGs,
  or match the CDN hex to the active theme's `--text-muted` value.
- **LOGO-ONLY rule.** A logo wall is logos and nothing else. No industry or category label
  under any logo (no "Stripe" + "payments", no "Vercel" + "hosting"). Brand name goes in
  `alt` text only. An optional link to the brand site is the ceiling.
- 5-8 logos, one row on desktop, uniform height 24-32px, gap `var(--space-7)`,
  centered with `align-items: center`. Overflow on mobile: wrap to 2 rows or marquee.
- Heading is optional and plain: "Trusted by teams at", "Customers include", or none.
  Cutesy phrasings are banned; the list lives in `quality/anti-slop/SKILL.md`, do not improvise.

## Testimonial spec

- **Quote body: max 3 lines** (about 30 words at body size). A landing-page quote is a
  snippet, not the review. Cut the original; never let it run to 6 lines.
- **Quote marks: real typographic quotes ( " " ) or none at all.** Never straight ASCII
  quotes as a design element. No em-dashes inside or around the quote, ever.
- **Attribution: name + role + company.** Never a bare name. Render in `var(--text-muted)`,
  one size below body.
- **Avatar: believable or initials.** Either a photo placeholder
  (`https://picsum.photos/seed/{person-seed}/96/96`, rendered 40-48px round) or an initials
  disc (`background: color-mix(in oklab, var(--accent) 15%, var(--surface))`, initials in
  `var(--text)`). Never an SVG egg, person glyph, or icon-library user icon.
- **Realistic data.** Locale-appropriate full names, real-sounding companies, organic
  numbers. Full rules in `content/SKILL.md`; do not ship "Jane Doe from Acme".

### Layout by count

| Quote count | Layout |
|---|---|
| 1 | Full-width pull quote, display size, centered, max-width 28ch |
| 2-3 | Asymmetric grid: one featured (larger cell and type), others smaller |
| 4+ | Carousel, or masonry columns with exactly one featured quote |

Never a symmetric row of 3 identical quote cards. Featured-vs-rest always beats equality.

## Review strip spec

- One line: rating value + source + count. Example: `4.8/5 on G2, 1,962 reviews`.
- Value uses `font-variant-numeric: tabular-nums`. Count is organic, not round
  (1,962, not 2,000). Mark mock data with `<!-- mock -->`.
- Stars, if drawn, come from the project icon family (icon rules: `content/SKILL.md`).
- Sits within `var(--space-6)` of the pricing table or final CTA, never as its own hero moment.

## Case-study teaser spec

- Max 3 teasers. Card = real image (`picsum.photos/seed/...` or generated), company name,
  one outcome metric ("34% fewer escalations"), link. No body paragraph.
- 2 teasers: `grid-template-columns: 1.4fr 1fr`. 3 teasers: one wide + two stacked,
  same asymmetric discipline as bento cells (`components/feature-sections/SKILL.md`).

## Worked example: 3-quote asymmetric grid

```html
<section class="testimonials">
  <h2>What teams say after switching</h2>
  <div class="quote-grid">
    <figure class="quote featured">
      <blockquote>“We cut deploy review from two days to forty minutes. Nobody on the team would go back.”</blockquote>
      <figcaption>
        <img src="https://picsum.photos/seed/priya-raghavan/96/96" alt="" width="44" height="44">
        <span><strong>Priya Raghavan</strong> Head of Platform, Northwind Freight</span>
      </figcaption>
    </figure>
    <figure class="quote">
      <blockquote>“The audit trail alone paid for the year.”</blockquote>
      <figcaption><span class="initials">MO</span>
        <span><strong>Marta Okafor</strong> Engineering Lead, Ferrum Labs</span></figcaption>
    </figure>
    <figure class="quote">
      <blockquote>“Setup took an afternoon, not a quarter.”</blockquote>
      <figcaption><span class="initials">JL</span>
        <span><strong>Jonas Lindqvist</strong> CTO, Kanal Analytics</span></figcaption>
    </figure>
  </div>
</section>
```

```css
.testimonials { padding-block: var(--space-9); max-width: var(--container); margin-inline: auto; }
.quote-grid { display: grid; grid-template-columns: 1.6fr 1fr;
              grid-template-areas: "feat q2" "feat q3"; gap: var(--space-5); }
.quote { background: var(--surface); border: 1px solid var(--border);
         border-radius: var(--radius-lg); padding: var(--space-6);
         display: flex; flex-direction: column; justify-content: space-between; }
.featured { grid-area: feat;
            background: color-mix(in oklab, var(--accent) 7%, var(--surface)); }
.featured blockquote { font-family: var(--font-display); font-size: 1.5rem; line-height: 1.35; }
.quote:nth-child(2) { grid-area: q2; }
.quote:nth-child(3) { grid-area: q3; }
figcaption { display: flex; align-items: center; gap: var(--space-3);
             margin-top: var(--space-5); color: var(--text-muted); }
figcaption img, .initials { border-radius: 50%; width: 44px; height: 44px; }
.initials { display: grid; place-items: center; font-family: var(--font-display);
            background: color-mix(in oklab, var(--accent) 15%, var(--surface));
            color: var(--text); }
figcaption strong { color: var(--text); display: block; }
@media (max-width: 767px) {
  .quote-grid { grid-template-columns: 1fr; grid-template-areas: "feat" "q2" "q3"; }
}
```

One featured quote, two supporting, typographic quotes, full attributions, initials discs
where no photo fits, explicit mobile collapse. Imitate the structure, not the copy.

## Checks

1. Every quote body is 30 words or fewer (renders within 3 lines at body size).
2. Every attribution contains all three parts: name, role, company.
3. Quotes use typographic marks ( " " ) or none; grep for straight `"` wrapping quote bodies, any hit fails.
4. The logo wall contains zero text nodes besides an optional plain heading; no category or industry label under any logo.
5. Every logo renders in the page theme: inline SVGs use `currentColor`, CDN URLs use the hex of `--text-muted`; no default full-color logo row.
6. No avatar is an SVG person glyph or icon-library user icon; each is a photo placeholder or an initials disc.
7. No name from the generic set (John Doe, Jane Doe, Sarah Chen, Jack Su) and no fake-round counts (2,000 reviews, 99.99%).
8. No social-proof heading matches a banned phrasing from `quality/anti-slop/SKILL.md`.
