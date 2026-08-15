---
name: feature-sections
description: "Build feature sections: bento grids, zigzag splits, stats bands, how-it-works steps, comparisons, FAQ accordions, and long lists or spec sheets. Use whenever the SECTION MAP calls for features, capabilities, metrics, process, FAQs, or any content list, and ALWAYS before rendering more than 3 items in a grid."
metadata:
  operant:
    tags: [features, bento, sections, layout]
---

# Feature sections

Turn a SECTION MAP entry plus its content slots into one feature section. The layout is
chosen by the shape of the content, never by habit. The habit layout (3 equal cards) is banned.

## Procedure

1. Count the items and classify the content shape (features, steps, stats, comparison, spec list).
2. Pick the layout family from the selection table below. Respect the repetition caps:
   a layout family appears at most ONCE per page (see `structure/SKILL.md`).
3. Build with semantic HTML (`section > h2` + `article` per item) and the canonical tokens.
4. Declare the mobile collapse (below 768px) in the same stylesheet block. Never assume it.
5. Run `## Checks` before moving to the next section.

Section header rule: headline on top, optional body below it, body `max-width: 65ch`.
Never the split header (big headline left, small paragraph floating right). One message per section.

Icons inside feature cells: one icon family, one size, one stroke width, never emoji as icons.
Full icon guidance lives in `content/SKILL.md`.

Framework note: every recipe here is plain HTML + CSS custom properties.
Tailwind, React, Vue, and shadcn mappings live in `references/stack-adapters.md`.

## Selection table

| Content shape | Layout family | Grid spec | Gap |
|---|---|---|---|
| 3 features | Asymmetric trio (bento 1+2) | `grid-template-columns: 1.4fr 1fr; grid-template-areas: "a b" "a c"` | `var(--space-5)` |
| 4 features | 2x2 with one wide cell | `repeat(2, 1fr)`, one cell spans both columns via areas | `var(--space-5)` |
| 5-6 features | Bento, mixed spans | `repeat(6, 1fr)` + `grid-template-areas` (see worked example) | `var(--space-5)` |
| 1-2 deep features | Zigzag split (image + text) | `grid-template-columns: 1fr 1fr` per row, alternate order | `var(--space-8)` |
| Process steps | Steps rail | see Steps section | `var(--space-6)` |
| 3-4 stats | Stats band | `repeat(n, 1fr)`, n <= 4, full-width band | `var(--space-6)` |
| Comparison (us vs them) | 2-col split cards | `grid-template-columns: 1fr 1fr` | `var(--space-5)` |
| More than 5 list items | Long-list table below | pick by row | per pick |
| Spec sheet | Spec-sheet table below | pick by row | per pick |
| Question-answer pairs | FAQ accordion (section below) | single column, `max-width: 65ch` | `var(--space-2)` |

Why: matching layout to content count kills the two loudest AI tells, the padded 3-card row
and the bento with a blank tile.

**No 3 equal feature cards.** Three identical cards in a `repeat(3, 1fr)` row is banned as
default. Use the asymmetric trio, a 2-column zigzag, or a horizontal-scroll row instead.
Override: only when a design system in `preserve` mode already uses this pattern.

## Bento build rules

1. **CELL COUNT = ITEM COUNT.** 3 items means 3 cells, 5 items means 5 cells. An empty cell
   or a filler tile means the grid shape is wrong. Re-shape the areas, never pad.
2. **Mixed cell sizes.** At least one cell spans 2+ columns or 2+ rows. Equal tiles are a
   card grid, not a bento. Use `grid-template-areas` so spans are explicit and re-shapeable.
3. **Background diversity.** In any grid of 4+ cells, at least 2 cells carry real visual
   variation, chosen from:
   - a real image: `background-image: url(https://picsum.photos/seed/{descriptive-seed}/800/600)`
   - a token gradient: `linear-gradient(135deg, var(--accent), color-mix(in oklab, var(--accent) 35%, var(--bg)))`
   - a tint: `color-mix(in oklab, var(--accent) 8%, var(--surface))`
   Text-only cells sit on `var(--surface)` with `1px solid var(--border)`.
4. **Rhythm, not repetition.** Do not stack 6 identical image-left rows inside a bento.
   Vary spans and content type cell to cell.
5. VARIANCE dial: 1-3 keep spans mild (max 2-col span), 4-7 allow row spans and a hero cell,
   8-10 allow overlapping visuals and off-grid accents.

## Zigzag rules

- One row = image half + text half, `grid-template-columns: 1fr 1fr`, gap `var(--space-8)`,
  alternate which side the image is on per row.
- **Max 2 consecutive** image+text split sections on the page. The 3rd in a row is a fail.
  Break with a full-width band, a stats band, a bento, or a marquee, then you may return.
- Image half is a real image (generated or `picsum.photos/seed/...`), never a div-built
  fake screenshot (ban list: `quality/anti-slop/SKILL.md`).
- Mobile: stack image above text, image first, gap `var(--space-6)`.

## Stats band spec

Full-width band, 3-4 stats max, values from the brief or marked mock (`<!-- mock -->`).
Organic numbers, never fake-round ones (see `content/SKILL.md`).

```css
.stats { display: grid; grid-template-columns: repeat(4, 1fr); gap: var(--space-6);
         padding-block: var(--space-8); border-block: 1px solid var(--border); }
.stat-value { font-variant-numeric: tabular-nums; font-family: var(--font-display);
              font-size: clamp(2.25rem, 4.5vw, 3.5rem); line-height: 1.05; }
.stat-label { color: var(--text-muted); margin-top: var(--space-2); }
@media (max-width: 767px) { .stats { grid-template-columns: repeat(2, 1fr); } }
```

- `font-variant-numeric: tabular-nums` is mandatory on every stat value so digits align.
- At DENSITY >= 8, switch values to `font-family: var(--font-mono)` for the instrument look.
- Label under the number, in `var(--text-muted)`, max 4 words.

## Long-list decision table

A plain `<ul>` or divided-row list caps at 5 items. Above 5, pick by shape:

| List shape | Component |
|---|---|
| Items group into 2-4 categories | Tabs or accordion, one group open by default |
| Each item has an image or icon | Card grid, `repeat(auto-fill, minmax(240px, 1fr))`, gap `var(--space-5)` |
| Short glanceable labels | Horizontal scroll-snap pills |
| Breadth matters, not each item (logos, capabilities) | Carousel |
| Ambient breadth, no individual attention needed | Marquee |
| Pairs well as two themes | 2-column split with grouped items |

## Spec-sheet table

A 10-row table with a hairline under every row is the worst default. Banned. Pick:

| Situation | Component |
|---|---|
| Every spec deserves explanation | 2-col card grid: spec name + large value + one-line "why it matters", 1-col mobile |
| Specs cluster logically | Grouped chunks: 2-3 clusters, ONE soft divider and a heading per cluster |
| 3-4 specs matter, rest is reference | Featured-vs-rest: hero specs as large display tiles, rest behind a "View full specifications" disclosure |
| Specs are short scannable pairs | Scroll-snap horizontal pills |

## Steps / how-it-works

- Labels are **verb-noun**: "Connect repo", "Map fields", "Ship". Never "Step 1", "Stage 1",
  "Phase 01", "Pass One" (ban list: `quality/anti-slop/SKILL.md`). A numeral may appear as a
  small visual marker, but the label text is the action itself.
- Max 5 steps. More than 5 means the process needs grouping, not a longer rail.
- 2-3 steps: horizontal rail, `grid-template-columns: repeat(n, 1fr)`, thin connector line in
  `var(--border)`. 4-5 steps: vertical timeline, marker column `var(--space-6)` wide.
- Each step: label + max 20 words of body. No paragraph essays per step.
- Mobile: always vertical.

## FAQ / accordion

- Semantic base: one `<details><summary>Question</summary><p>Answer</p></details>` per
  item. When styling demands full control, use the button + panel disclosure pattern in
  `quality/accessibility/SKILL.md` (aria-expanded, aria-controls, hidden).
- 4-8 questions on a landing page; more belongs on a dedicated support page. The first
  item may start open; never all open.
- Question <= 12 words; answer <= 60 words in the page's copy register. Answers handle
  objections (pricing, security, cancellation), not documentation.
- Single column, `max-width: 65ch`. Never a 2-column FAQ grid; reading order breaks.
- Chevron rotates via `transform` 200ms. Do not animate panel height (layout thrash);
  instant reveal or opacity only.
- Launch note: visible FAQ sections get FAQPage JSON-LD, see
  `quality/production/SKILL.md` section 2.

## Worked example: 5-item bento

```html
<section class="features">
  <h2>Built for the whole pipeline</h2>
  <div class="bento">
    <article class="cell cell-a"><img src="https://picsum.photos/seed/deploy-console/800/520" alt="Deploy console">
      <h3>Preview every branch</h3><p>Each push gets a URL your whole team can open.</p></article>
    <article class="cell cell-b"><h3>Rollback in one click</h3><p>Every deploy is immutable. Restore any of the last 50.</p></article>
    <article class="cell cell-c"><h3>Edge caching</h3><p>Static assets served from 30 regions by default.</p></article>
    <article class="cell cell-d"><h3>Secrets sync</h3><p>Environment variables encrypted and versioned.</p></article>
    <article class="cell cell-e"><h3>Audit trail</h3><p>Who shipped what, when, and from which commit.</p></article>
  </div>
</section>
```

```css
.features { padding-block: var(--space-9); max-width: var(--container); margin-inline: auto; }
.features > h2 { font-family: var(--font-display); max-width: 22ch; margin-bottom: var(--space-7); }
.bento {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  grid-template-areas:
    "a a a a b b"
    "c c d d b b"
    "c c e e e e";
  gap: var(--space-5);
}
.cell { background: var(--surface); border: 1px solid var(--border);
        border-radius: var(--radius-lg); padding: var(--space-6); }
.cell-a { grid-area: a; padding: 0; overflow: hidden; }          /* image cell */
.cell-a img { width: 100%; height: 100%; object-fit: cover; }
.cell-b { grid-area: b;                                           /* gradient cell */
  background: linear-gradient(160deg, var(--accent),
              color-mix(in oklab, var(--accent) 35%, var(--bg)));
  color: var(--accent-contrast); border: none; }
.cell-c { grid-area: c;                                           /* tinted cell */
  background: color-mix(in oklab, var(--accent) 8%, var(--surface)); }
.cell-d { grid-area: d; }
.cell-e { grid-area: e; }
@media (max-width: 767px) {
  .bento { grid-template-columns: 1fr; grid-template-areas: "a" "b" "c" "d" "e"; }
  .cell-a { min-height: 220px; }
}
```

5 items, 5 cells, three spans differ, image + gradient + tint = 3 varied cells, explicit
single-column collapse. That is the template to imitate, with different areas per project.

## Checks

1. Every bento or feature grid has exactly as many cells as content items. Zero empty or filler cells.
2. No section renders 3 equal-width identical feature cards (grep `repeat(3, 1fr)` on feature sections; any hit with three same-shaped cards fails).
3. Count consecutive image+text split sections top to bottom: the count never exceeds 2.
4. In every grid of 4+ cells, at least 2 cells have a background other than plain `var(--surface)` (image, gradient, or tint).
5. Every multi-column grid has an explicit rule below 768px collapsing it (grep `@media` per grid; missing = fail).
6. Every stat value has `font-variant-numeric: tabular-nums`; if DENSITY >= 8 it also uses `var(--font-mono)`.
7. No step label matches `Step N`, `Stage N`, `Phase N`, or `Pass N` (grep is definitive).
8. No plain bulleted or divided-row list on the page has more than 5 items.
