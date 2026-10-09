# STRUCTURE: Section Sequencing, Layout Families, and Section Mapping

| Turns TOKENS and DIRECTION into the SECTION MAP artifact: an ordered list of named sections, each with a layout family, component recipe path, and explicit content slots. Enforces section sequencing by page kind, layout repetition caps, bento cell counts, zigzag caps, eyebrow restraint, hero padding limits, and explicit mobile collapse rules.

- Input: `TOKENS.css` produced by [system](system.md) (spacing scale, typography tokens, surface variables) and `DIRECTION.md` produced by [direction](direction.md) (page kind, style family, DENSITY dial, MOTION budget).
- Output: `SECTION-MAP.md` (or a `SECTION MAP` code block in markdown) written to the project root.
- Downstream consumers: [components](components.md) and component recipes under [components](components.md). They implement the mapped sections directly and never introduce unmapped layout wrappers.

Information architecture determines visual rhythm. No layout markup is written before the SECTION MAP passes its sequencing, repetition, and constraint checks.

## 1. The SECTION MAP Artifact

Every web property requires a deterministic section map before component generation starts. The SECTION MAP binds design direction and design tokens to concrete component recipes.

Mandatory columns for every SECTION MAP table:

| Column | Purpose | Example |
|---|---|---|
| # | Sequential execution index | 1, 2, 3... |
| Section Name | Functional purpose of the section | Hero, Social Proof, Core Bento |
| Layout Family | One of the 6 canonical structural families | split, bento, full-width, centered |
| Component Recipe Path | Exact path under [components](components.md) | [heroes](components/heroes.md) |
| Content Slots | Exact numerical constraints for text and assets | headline 6w max, sub 18w max, 2 CTAs, 1 asset |
| Mobile Collapse Spec | Explicit stack behavior under 768px viewport | single col, text then asset, gap var(--space-6) |

Rules for content slots:
1. Slot counts are strict integers, never subjective descriptions. Write `3 cards`, `4 counters`, `headline max 8w`, not `several features`.
2. Recipe paths must match real files in [components](components.md) ([heroes](components/heroes.md), [feature-sections](components/feature-sections.md), [social-proof](components/social-proof.md), [conversion](components/conversion.md), [navigation](components/navigation.md), [forms](components/forms.md)).
3. Mobile collapse behavior must be stated per row. No implicit fallback assumptions.

## 2. Section Sequencing by Page Kind

Sequence sections to guide the visitor from orientation to comprehension to action. Pick the matching page kind and follow its default order.

### 2.1 SaaS and Product Landing Pages
1. Hero: One strong headline, concise subtext, primary CTA, secondary CTA, single product visual.
2. Social Proof: Uncaptioned logo strip or metrics ribbon directly under the hero.
3. Problem or Value Contrast: Side-by-side or split layout highlighting pain vs solution.
4. Primary Feature Showcase: Asymmetric bento or deep split layout demonstrating core workflow.
5. Secondary Capabilities: Structured feature grid, tabbed pane, or modular cards.
6. Process / How It Works: Stepped rail (3-4 steps max, verb-noun labels, numbered only if strictly linear).
7. Social Proof / Testimonials: 1-3 direct customer quotes with verified names, roles, and avatar assets.
8. Pricing or Conversion Tier: Transparent tier cards with feature lists and primary conversion trigger.
9. FAQ Accordion: Single-column accordion handling critical objections (security, billing, migration).
10. Final CTA Band and Global Footer: High-contrast closing call to action with minimal navigation links.

### 2.2 Portfolio and Agency Pages
1. Hero: Work sample or project preview as the focal anchor, not an abstract slogan.
2. Selected Work / Case Studies: Varied visual grid (3-5 projects max) with outcome metrics.
3. Capabilities / Services: Dense typographic grid or grouped spec sheet, never identical cards.
4. Process / Philosophy: Short narrative split with real workshop or production imagery.
5. Client Roster / Endorsements: Monochrome logo matrix with contextual testimonial pull-quotes.
6. Contact and Inquiry CTA: Direct conversation initiation block with structured inquiry fields.

### 2.3 Editorial and Publication Pages
1. Hero Feature: Prominent lead story with headline typography, metadata line, and primary artwork.
2. Curated Top Stories: 3-column asymmetric layout with varying image heights and summary lengths.
3. Category Showcase: Multi-track list or tabbed section organized by editorial taxonomy.
4. Deep Dive Feature: Wide reading column (max-width 65ch) with pull quotes and inline figures.
5. Newsletter Subscription: High-intent inline signup module with privacy commitment.
6. Archive Navigation and Footer: Dense category directory and masthead colophon.

### 2.4 Ecommerce and Storefront Pages
1. Hero: Flagship product hero with contextual lifestyle photography and direct buy action.
2. Category Grid: Visual collection gateways with asymmetric highlight tiles for top collections.
3. Curated Product Grid: 4-item product showcase with pricing, colorway badges, and quick-add actions.
4. Trust and Service Guarantees: 3-item horizontal service ribbon (shipping, returns, lifetime warranty).
5. Customer Reviews and Visual UGC: Media-rich review grid with star ratings and verified buyer tags.
6. Newsletter and Footer: Incentive-driven newsletter capture with customer service footer.

### 2.5 Dashboard and Web Application Shells
1. Top Utility Bar: Workspace switcher, global search command palette, notifications, user profile.
2. Primary Navigation: Collapsible left sidebar with icon plus text labels and active indicators.
3. KPI Stat Strip: 3-4 metric cards using tabular numerals (`font-variant-numeric: tabular-nums`).
4. Main Canvas: Primary operational interface (data table, kanban canvas, or interactive visualization).
5. Contextual Inspector / Secondary Panel: Right-side details drawer or activity feed.

## 3. The Six Layout Families

Every section on a page belongs to exactly one of the six layout families. Alternating layout families creates visual rhythm and eliminates monotonous stacked card patterns.

| Layout Family | Geometry and Composition | Primary Usage | Desktop Token Spec |
|---|---|---|---|
| centered | Single column, centered or left-aligned text, max 65ch width | Hero, FAQ, Final CTA, Process Steps | `max-width: var(--container-sm); margin-inline: auto; text-align: center;` |
| split | 2 columns (50/50, 60/40, or 40/60), text on one side, asset on other | Value prop, Hero split, Deep feature | `display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-8);` |
| bento | Asymmetric multi-cell grid with mixed row and column spans | Feature sets, capability overviews | `display: grid; grid-template-columns: repeat(6, 1fr); gap: var(--space-5);` |
| zigzag | Alternating 2-column split rows (L/R then R/L) | Multi-step feature walkthroughs | 2 rows max, `grid-template-columns: 1fr 1fr; gap: var(--space-8);` |
| full-width | Edge-to-edge container bleed, colored or tinted background | Stat ribbons, quote bands, product panoramas | `width: 100vw; margin-left: calc(50% - 50vw); padding-block: var(--space-9);` |
| marquee | Ambient continuous horizontal ticker or logo strip | Customer logos, partner networks | `display: flex; overflow: hidden; white-space: nowrap; mask-image: linear-gradient(...);` |

### 3.1 Family Layout Specifications

Each family has fixed layout geometry that components must respect.

#### Centered
- Container: Constrained to `var(--container-sm)` or 65ch to maintain optimal line lengths.
- Text alignment: Centered for short headlines and CTAs; left-aligned for long-form explanatory copy.
- Spacing: Top and bottom padding driven by `var(--space-8)` or `var(--space-9)`.
- Anti-pattern: Stacking three centered text sections in succession produces a dead, static cadence.

#### Split
- Structure: Two columns with explicit proportions (`1.2fr 1fr`, `1fr 1fr`, or `1fr 1.2fr`).
- Text column: Headline (max 2 lines), body paragraph (max 25 words), optional bullet cluster or CTA.
- Visual column: Real photography, high-fidelity UI preview, or interactive component. Never a fake div mock.
- Anti-pattern: Split header with a headline on the left and a floating orphan paragraph on the right.

#### Bento
- Grid base: 6-column or 12-column CSS Grid enabling diverse asymmetric spans.
- Hierarchy: Exactly one hero cell that commands primary visual focus.
- Diversity: Minimum 2 non-standard backgrounds (one real image, one token gradient, or one tinted fill).
- Anti-pattern: Equal-sized square tiles arranged in a uniform 3x2 matrix. That is a card grid, not a bento.

#### Zigzag
- Structure: Exactly two alternating rows. Row 1: text left, image right. Row 2: image left, text right.
- Row spacing: Separated by `var(--space-8)` or `var(--space-9)` to establish clear visual breathing room.
- Image assets: Aspect ratio matched across rows (typically 16:10 or 4:3) with subtle token borders.
- Anti-pattern: A third consecutive zigzag row. Three alternating splits in a row trigger fatigue.

#### Full-Width
- Container: Full viewport bleed (`width: 100vw`) breaking out of standard layout containers.
- Content containment: Inner content re-constrained to `var(--container)` with horizontal safety gutters.
- Contrast: Background distinct from default `--bg` via `--surface`, subtle brand tint, or full dark fill.
- Anti-pattern: Full-width text with unconstrained line lengths exceeding 100 characters.

#### Marquee
- Mechanism: Continuous CSS keyframe translation with linear timing function.
- Requirement: Only permitted when DIRECTION specifies MOTION dial of 4 or higher.
- Edge fading: CSS `mask-image` with horizontal linear gradient fades on both left and right edges.
- Anti-pattern: Marquees containing interactive links with tight hover targets, or running on mobile screens.

## 4. Structural Hard Rules and Repetition Caps

These layout rules are absolute. Failing any rule produces templated, low-craft output.

### 4.1 Layout Family Repetition Cap (Max Once Per Page)
Once a layout family is used for a section, that exact family configuration can appear at most ONCE on the page. If Section 3 uses a 3-column card grid, Section 5 cannot use a 3-column card grid. A landing page with 8 sections must incorporate at least 4 distinct layout families.

### 4.2 Bento Cell Count Rule
A bento grid must have EXACTLY as many cells as there are real content items.
- 3 content items = 3 cells (1 large + 2 stacked, or asymmetric trio).
- 4 content items = 4 cells (2x2 with one wide span, or 1 hero column + 3 stacked).
- 5 content items = 5 cells (6-column base grid: 4-span hero + 2-span card, 2-span + 2-span + 2-span row).
Never generate empty cells, ghost placeholders, or decorative filler tiles to complete a geometric grid. If items do not fill the grid, redesign the grid areas.
Background diversity rule: In any grid of 4 or more cells, at least 2 cells must have visual treatments other than plain `var(--surface)` (such as a real image, token gradient, or distinct tint).

### 4.3 Zigzag Alternation Cap (Max 2 Consecutive)
Alternating left-image/right-text and left-text/right-image rows quickly feels repetitive.
- Maximum 2 consecutive split rows on the entire page.
- A 3rd consecutive image and text split row is an immediate build failure.
- Break the zigzag cadence with a full-width band, a bento grid, a stat strip, or a centered module before introducing another split section.

### 4.4 Eyebrow Restraint Rule (Max 1 Per 3 Sections)
An eyebrow is a small uppercase tracking label positioned above a section headline (such as `PLATFORM ARCHITECTURE` or `SECURITY FIRST`).
- Hard cap: Maximum 1 eyebrow per 3 sections across the entire page (the hero counts as 1).
- Formula: `Total Eyebrows <= Math.ceil(Total Sections / 3)`.
- If Section 1 has an eyebrow, Sections 2 and 3 must not have an eyebrow.
- When in doubt, omit the eyebrow entirely. Let the headline carry the section identity.

### 4.5 Hero Viewport and Padding Limits
- Hero Top Padding Cap: Maximum `pt-24` (6rem or 96px) on desktop. Excessive top padding pushes the hero content down and wastes the initial viewport.
- Viewport fit: The entire hero block (headline, subtext, CTAs, and primary visual) must fit within the initial 100vh viewport on standard desktop displays without scrolling.
- Hero Stack Discipline (max 4 text elements):
  1. Eyebrow or brand indicator (optional, pick zero or one).
  2. Headline (maximum 2 lines on desktop).
  3. Subtext (maximum 20 words and maximum 3-4 lines).
  4. CTA cluster (1 primary button + maximum 1 secondary link or button).
- Banned in hero: Logo walls, feature lists, pricing snippets, and customer quote carousels. Place logo walls directly under the hero in a dedicated social proof section.

### 4.6 Split-Header Ban
The pattern of placing a large headline on the left and a small explanatory paragraph floating in the right column is banned as default. Sections must deliver one unified message. Place the headline on top and stack the explanatory text directly below it, constrained to `max-width: 65ch`.

### 4.7 Numbered Markers Encode Information (not decoration)
Visual structure is information. Numbered markers (`01 / 02 / 03`, `Step 01:`, `Phase 02`) are permitted ONLY when content is a genuine sequence: stepped process, timeline, ranked order, or how-it-works with linear dependency. Use [anti-slop](quality/anti-slop.md) check 09 to detect decorative numbers; before adding them, verify: does the content truly order by sequence? If not, use the headline alone, or a plain list, or a card grid. A 3-feature marketing section with `01/02/03` is decorative and banned; a 4-step onboarding flow with `Step 01-04` is information and allowed with the sequence named in the section's purpose.

## 5. Explicit Mobile Collapse Architecture

Responsive behavior must be defined in the SECTION MAP before code generation. Every multi-column layout requires an explicit mobile collapse plan for viewports below 768px.

| Desktop Family | Mobile Collapse Rule (under 768px) | Layout Integrity Safeguard |
|---|---|---|
| split (text + asset) | Stack vertically: headline + copy + CTAs first, asset directly below | Prevents image from pushing primary CTA below the mobile fold |
| zigzag (multi-row) | Stack vertically: asset on top, copy underneath for every row | Maintains consistent scanning pattern across alternating items |
| bento (mixed spans) | Collapse to 1 column: `grid-template-columns: 1fr; grid-template-areas: none;` | Assign minimum heights (`min-height: 220px`) to image and media cells |
| grid (3-4 columns) | Collapse to 1 column (or 2 columns for compact metric badges) | Keep card padding at `var(--space-4)` or `var(--space-5)` to prevent overflow |
| full-width band | Retain edge-to-edge fill, adjust horizontal padding to `var(--space-4)` | Ensure typography scales down via fluid clamp formulas |
| marquee | Freeze CSS animation; display as flex-wrap row with soft gap | Prevents CPU battery drain and erratic touch scrolling on mobile |

### 5.1 Mobile Collapse Implementation Reference

Standard CSS implementation patterns for collapsing multi-column structures:

```css
/* Split section collapse */
.split-layout {
  display: grid;
  grid-template-columns: 1.2fr 1fr;
  gap: var(--space-8);
  align-items: center;
}
@media (max-width: 767px) {
  .split-layout {
    grid-template-columns: 1fr;
    gap: var(--space-6);
  }
}

/* Bento grid collapse */
.bento-layout {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: var(--space-5);
}
@media (max-width: 767px) {
  .bento-layout {
    grid-template-columns: 1fr;
  }
  .bento-layout .cell {
    grid-column: 1 / -1 !important;
    grid-row: auto !important;
  }
}

/* Marquee mobile freeze */
@media (max-width: 767px) {
  .marquee-track {
    animation: none;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-4);
  }
}
```

## 6. Worked Example: SaaS Landing Page (RelayDB)

Context: High-performance distributed database product, Dark Theme, DENSITY 8 (technical/instrument), MOTION 4.

### 6.1 SECTION MAP for RelayDB

| # | Section Name | Layout Family | Recipe Path | Content Slots | Mobile Collapse Spec |
|---|---|---|---|---|---|
| 1 | Hero | split | [heroes](components/heroes.md) | eyebrow 2w, headline 7w, subtext 16w, 2 CTAs, 1 interactive query terminal asset | Stack copy first, CTAs visible, terminal below with 280px min-height |
| 2 | Social Proof | marquee | [social-proof](components/social-proof.md) | 8 partner SVG logos, 0 captions, opacity 70% | Freeze motion, wrap into 2 rows of 4 centered logos |
| 3 | Core Engine | bento | [feature-sections](components/feature-sections.md) | 5 items: 1 large architecture visual cell, 1 latency benchmark cell, 3 spec cards | Single column stack, architecture visual cell rendered first |
| 4 | Protocol Walkthrough | zigzag | [feature-sections](components/feature-sections.md) | 2 rows max: row 1 (Raft consensus + graphic), row 2 (graphic + storage engine) | Image top, copy bottom for both rows, gap var(--space-6) |
| 5 | Global Metrics | full-width | [social-proof](components/social-proof.md) | 4 KPI stat counters with mono tabular numbers, 1 section caption (no eyebrow) | 2x2 grid below 768px, font-size clamp active |
| 6 | Comparison Matrix | centered | [feature-sections](components/feature-sections.md) | 1 headline, 1 subtext 14w, 2-column Us vs Legacy table with 5 attribute rows | 1-column comparison cards with sticky column headers |
| 7 | Developer FAQ | centered | [feature-sections](components/feature-sections.md) | 5 question-answer disclosure items, max 50 words per answer, single column | Single column, max-width 100%, accordion touch targets 48px |
| 8 | Conversion CTA | full-width | [conversion](components/conversion.md) | headline 5w, subtext 12w, 1 primary action button, 1 terminal install string | Centered stack, install string full width with horizontal scroll |

### 6.2 Rhythm and Constraint Verification for RelayDB
- Section count: 8 sections.
- Distinct layout families used: split, marquee, bento, zigzag, full-width, centered (6 distinct families).
- Family repetition check: No layout family repeats consecutively or within any 3-section window.
- Eyebrow check: Eyebrows present in Section 1 and Section 3 only (2 total eyebrows). `2 <= Math.ceil(8 / 3) = 3` (PASS).
- Zigzag check: Section 4 contains exactly 2 split rows, preceded by bento and followed by full-width band (PASS).
- Bento check: Section 3 contains 5 content items mapped to 5 distinct grid areas (PASS).
- Hero padding check: Desktop top padding specified as `var(--space-8)` (64px, well under the 96px cap) (PASS).

### 6.3 Semantic Document Skeleton for RelayDB

The SECTION MAP maps directly to this top-level markup skeleton:

```html
<main id="content">
  <!-- 1. Hero: split layout -->
  <section class="section hero split-layout" aria-label="Product introduction">
    <div class="hero-copy"><!-- eyebrow, headline, subtext, CTAs --></div>
    <div class="hero-visual"><!-- query terminal asset --></div>
  </section>

  <!-- 2. Social Proof: marquee layout -->
  <section class="section social marquee-layout" aria-label="Customer trust">
    <div class="marquee-track"><!-- 8 partner SVG logos --></div>
  </section>

  <!-- 3. Core Engine: bento layout -->
  <section class="section core bento-layout" aria-label="Core architecture">
    <!-- 5 cells: architecture visual, benchmark, 3 spec cards -->
  </section>

  <!-- 4. Protocol Walkthrough: zigzag layout -->
  <section class="section protocol zigzag-layout" aria-label="Protocol walkthrough">
    <!-- row 1: text left, image right -->
    <!-- row 2: image left, text right -->
  </section>

  <!-- 5. Global Metrics: full-width layout -->
  <section class="section metrics full-width-layout" aria-label="Key performance metrics">
    <div class="metrics-grid"><!-- 4 KPI stat counters --></div>
  </section>

  <!-- 6. Comparison Matrix: centered layout -->
  <section class="section comparison centered-layout" aria-label="Feature comparison">
    <div class="matrix-table"><!-- Us vs Legacy attribute table --></div>
  </section>

  <!-- 7. Developer FAQ: centered layout -->
  <section class="section faq centered-layout" aria-label="Frequently asked questions">
    <!-- 5 accordion items -->
  </section>

  <!-- 8. Conversion CTA: full-width layout -->
  <section class="section conversion full-width-layout" aria-label="Get started">
    <div class="cta-card"><!-- headline, install command, action button --></div>
  </section>
</main>
```

## 7. The Seven Pre-Flight Checks

Before emitting the SECTION MAP or initiating component coding, verify these seven gating rules.

1. Page Kind Sequence Alignment: The sequence of sections follows the established flow for the target page kind without omitting essential conversion or credibility steps.
2. Layout Family Diversity: Every layout family appears at most once on the page, and the total number of distinct layout families is at least `Math.ceil(total_sections / 2)`.
3. Zigzag Alternation Boundary: No sequence contains more than 2 consecutive image and text split rows anywhere on the page.
4. Bento Exact Cell Count: Every bento grid has exactly as many cells as content items (no blank filler tiles, no missing cells), with at least 2 distinct background styles in grids of 4+ cells.
5. Eyebrow Restraint Gate: Total uppercase tracking eyebrow labels across the entire page does not exceed `Math.ceil(total_sections / 3)`. No two adjacent sections both carry eyebrows.
6. Hero Viewport and Padding Compliance: Hero top padding does not exceed 96px (`pt-24`), headline does not exceed 2 lines, subtext does not exceed 20 words, and CTAs fit above the fold.
7. Explicit Mobile Definitions: Every row in the SECTION MAP specifies an unambiguous mobile collapse rule for viewports under 768px.
