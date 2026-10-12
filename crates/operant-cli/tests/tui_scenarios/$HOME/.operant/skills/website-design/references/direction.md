# DIRECTION

| Translates a DESIGN BRIEF into a DIRECTION artifact: one concrete style, palette, typography pairing, and effects budget. Run BEFORE writing any HTML or CSS. Reads dials (VARIANCE, MOTION, DENSITY) and the ui-ux-pro-max data pack. Enforces the Lila Rule, bans premium-consumer defaults, and outputs WCAG pre-verified token values.

One page, one voice. You cannot emit a design direction that contradicts itself. Pick one pole per axis and stay there.

- Input: `DESIGN BRIEF` produced by [briefing](briefing.md). Read it fully before proceeding.
- Output: `DIRECTION.md` written to the project root. This file is the contract for every later phase.
- Downstream consumers: [system](system.md) (tokens), [structure](structure.md) (sections), [components](components.md) (recipes).

If the brief is missing or ambiguous, stop and re-run briefing. Do not guess intent and bake it into tokens.

## References

This leaf draws on three curated data packs. Read the relevant file before choosing when search is unavailable:
- `references/direction/references/styles.md` - style families, dial ranges, when-to-use (umbrella source: `ui-ux-pro-max` styles.csv)
- `references/direction/references/palettes.md` - WCAG pre-verified palettes mapped to canonical tokens (source: `colors.csv`)
- `references/direction/references/font-pairings.md` - pairings with serif discipline (source: `typography.csv`)

## Scope

Charts are explicitly out of scope for this skill. For data visualization, query `ui-ux-pro-max` via `search.py "<data shape>" --domain chart` instead of building ad-hoc grid + bar mocks. This file owns marketing/landing aesthetics, not dashboards.

## 1. Canonical Tokens (Non-Negotiable)

Every color, font, space, and elevation value you emit MUST map to these CSS custom properties.
Never invent new names. Never hardcode hex outside this map. Never create one-off utility classes that bypass it.

| Token                 | Semantics                                   | Constraint                                  |
|-----------------------|---------------------------------------------|---------------------------------------------|
| `--bg`                | Page background                             | WCAG vs `--text`: 4.5:1 minimum             |
| `--surface`           | Elevated components (cards, nav, panels)    | WCAG vs `--text`: 4.5:1 minimum             |
| `--text`              | Primary text                                | WCAG vs `--bg`: 4.5:1 minimum               |
| `--text-muted`        | Secondary text, captions, metadata          | WCAG vs `--bg`: 4.5:1 minimum               |
| `--accent`            | Primary action, links, brand highlight      | WCAG vs `--bg`: 3:1 minimum                 |
| `--accent-contrast`   | Text/icons placed ON accent                 | WCAG vs `--accent`: 4.5:1 minimum           |
| `--border`            | Hairlines, dividers, input strokes          | Visually detectable against `--bg`          |
| `--destructive`       | Delete, error, irreversible                 | 4.5:1 against its background                |
| `--radius`, `--radius-lg` | Corner system; pick ONE system          | See Section 7 for lock rules                |
| `--font-display`      | Headlines, hero, display numbers            | Distinct from body voice                    |
| `--font-body`         | Paragraphs, UI labels, controls             | Optimized for reading at 16px               |
| `--font-mono`         | Code, tabular numerals, data labels         | Same x-height feel as body                  |
| `--space-1 .. --space-10` | Spacing scale                            | 4, 8, 12, 16, 24, 32, 48, 64, 96, 128 px     |
| `--container`         | Max content width                           | Typical 1200-1440px; tighter for reading    |
| `--shadow-1`, `--shadow-2` | Elevation tokens                         | No `rgba(0,0,0,0.1)` generic soft grey      |
| `--duration-fast`, `--duration-med` | Motion timing tokens             | 150ms / 300ms; do not exceed 500ms          |

Verification: run `scripts/contrast.py` after choosing every color pair. It prints PASS/FAIL against WCAG AA. If it prints FAIL, adjust hex values and re-run before continuing. Do not "fix it in the browser later".

## 2. Palette Discipline

### 2.1 The Lila Rule

Never emit "AI Purple", cyan-to-magenta gradients, neon glows, or blurred color orbs as primary identity. These are the most common LLM visual shorthand and instantly read as template output.

Preferred recipe: neutral ground (Zinc, Slate, or Stone family) plus a single high-contrast accent (Emerald, Electric Blue, Deep Rose, Burnt Orange, Cobalt, etc.). The accent must be defensible from the subject matter in the brief. If you cannot explain the accent in one sentence tied to the product or industry, it is decoration; pick a different one or pick neutral.

### 2.2 Premium-Consumer Ban

If the brief describes cookware, wellness, artisan goods, luxury, heritage craft, DTC home goods, or any premium-consumer positioning, DO NOT output the warm-beige palette family. That combination is the default LLM failure mode for this segment.

Banned hex values (do not emit these exact values):
- Background/cream: `#f5f1ea`, `#f7f5f1`, `#fbf8f1`, `#efeae0`, `#ece6db`, `#faf7f1`, `#e8dfcb`
- Accent/brass-clay-oxblood: `#b08947`, `#b6553a`, `#9a2436`, `#9c6e2a`, `#bc7c3a`, `#7d5621`
- Text/espresso near-black: `#1a1714`, `#1a1814`, `#1b1814`

Allowed alternatives for premium-consumer (pick ONE; do not blend):
- Cold Luxury: silver-grey, chrome, smoke, ice blue accent
- Forest: deep green, bone, amber accent
- Black Tan: off-black, warm tan, high contrast, no beige
- Cobalt Cream: saturated blue foundation, not warm craft
- Terracotta + Olive: only if explicitly requested in brief
- Off-white + off-black: sharp, no warm cream

Defaulting to warm-craft because it "feels premium" is the second most common AI tell. Avoid it.

### 2.3 Single Accent Lock

One accent per page. Full stop. No accent cycling by section: rose footer, green hero, blue nav. No secondary accent "for variety". If the brief suggests multiple brand colors, pick the primary one for `--accent` and demote the rest to `--text-muted` usage or imagery.

### 2.4 Dark Mode

If the brief requests dark mode, emit complete token values for both themes. Test contrast in both. Do not reuse the same accent hex in both themes if it reduces contrast; adjust saturation/lightness and re-verify.

## 3. Typography Discipline

Pull evidence from `../ui-ux-pro-max/data/typography.csv` using the search protocol below. Apply its paired fonts exactly. Do not swap halves of a pairing.

### 3.1 Serif Restraint

Serif display faces are accepted ONLY when the brief explicitly calls for editorial, premium, luxury, or creative voice AND the paired sans-serif is neutral (not a "brand sans" with strong personality).

If the brief is SaaS, tool, docs, dashboard, B2B, or commercial product: default to strong sans pairing. Do not add a serif because it "feels elevated". That is the first design-taste regression tested in anti-slop review.

### 3.2 Emphasis Rule

To emphasize a word inside a headline, use italic or bold weight of the SAME font family. Never mix font families inside one headline. Mixed-family emphasis reads as amateur and breaks the typographic voice.

### 3.3 Banned Display Faces

Do not pick either of these two fonts for display use. They are the most common LLM picks and are banned:
- Fraunces
- Instrument_Serif

If you were about to choose one, stop. Pick from the alternative pool: PP Editorial New, GT Sectra Display, Cardinal Grotesque, Reckless Neue, Tiempos Headline, Recoleta, Cormorant Garamond, Playfair Display, EB Garamond, IvyPresto, Migra, Editorial Old, Saol Display, Söhne Breit Kursiv, Domaine Display, Canela, Schnyder, Tobias, NB Architekt, ITC Galliard.

### 3.4 Italic Descender Clearance

Italic lowercase descenders (p, q, y, g) clip when line-height is too tight. If you use italic, ensure `leading-[1.1]` or add `padding-bottom` to the element. Audit this before shipping.

### 3.5 Verified Typography Pairings (Quick Reference)

| Category | Display Font | Body Font | Mono Font | Best Use Case |
|----------|--------------|-----------|-----------|---------------|
| Sans / Technical | Geist | Geist | Geist Mono | Developer tools, modern SaaS, B2B dashboards |
| Sans / High-Contrast | Satoshi | Inter | JetBrains Mono | Product landing pages, high-energy startups |
| Sans / Neo-Grotesk | Cabinet Grotesk | Inter Tight | Space Mono | Marketing pages, creative agency sites |
| Sans / Clean Neutral | Outfit | Plus Jakarta Sans | Fira Code | Consumer applications, collaboration tools |
| Serif / Editorial | PP Editorial New | Inter | JetBrains Mono | Longform journalism, independent magazines |
| Serif / Bookish | Tiempos Headline | Public Sans | Roboto Mono | Publishing platforms, literary agencies |
| Serif / Luxury | Reckless Neue | ABC Diatype | Space Mono | High-end architecture, bespoke commerce |

## 4. Search Protocol (ui-ux-pro-max)

Do NOT invent style, palette, or typography from memory. Run the ui-ux-pro-max search pipeline first.

```bash
python3 /home/ishanp/.agents/skills/ui-ux-pro-max/scripts/search.py \
  "<product> <industry> <style keywords>" \
  --design-system --domain <domain>
```

Protocol:
1. If results return, use them preferentially over general guidance.
2. If zero results, retry once with `--stack html-tailwind` (or the detected project stack).
3. If still zero, fall back to this skill's built-in rules and state explicitly: "no verified pack match".
4. Use pairs from `typography.csv` exactly. Do not take display from one row and body from another.
5. For new projects, persist the system with `--persist` and `--output-dir` only if the caller asked for persistence.

If the search script errors or is unavailable, proceed with the built-in guidance but flag it in DIRECTION.md.

## 5. Effects and Motion Budget

Allowed effects (choose based on MOTION dial):
- Glassmorphism: subtle, low blur, no border glow
- Bento: asymmetric grid, max two columns at mobile base, no forced symmetry
- Brutalism: hairline rules, 0 radius, dense copy blocks
- Parallax depth: restrained, single axis, no nausea-inducing speeds
- Micro-interactions: hover lifts, active press states, focus rings

Banned effects:
- Ambient glow or neon bloom behind text
- Random gradient blobs that do not encode information
- Shimmer, shine, or animated gradient borders
- Scroll-jacked sections that prevent normal scrolling
- Marquee text loops
- "3D card tilt" that follows mouse unless the product is 3D

Motion budget mapping:
- MOTION 1-3: static, micro-interactions only
- MOTION 4-6: one scroll-linked reveal, one hover system
- MOTION 7-10: cinematic allowed, but still ban random glow

Density mapping:
- DENSITY 1-3: generous whitespace, fewer sections, large type
- DENSITY 4-6: standard spacing, two-column where useful
- DENSITY 7-10: compact spacing, dense grids, table-driven UI

## 6. Style Poles (Pick Exactly One)

Do not hybridize across poles. A page that is "minimal but also playful and editorial" is an unmade decision.

| Pole        | Markers                                    | Use when brief calls for            |
|-------------|--------------------------------------------|-------------------------------------|
| brutalist   | 0 radius, hairlines, dense, mono accents   | manifesto, technical, developer     |
| minimal     | whitespace, one type scale, no decoration  | docs, tool, utility                 |
| editorial   | serif display, asymmetric grid, thin rules | magazine, portfolio, brand story    |
| product     | neutral UI, clear hierarchy, icon support  | SaaS, dashboard, B2B                |
| luxury      | restrained color, high contrast, detail    | premium non-consumer, not warm-craft|
| playful     | soft shapes, brighter accent, motion       | consumer app, youth, entertainment  |

State the chosen pole and one sentence tying it to the subject matter. If you cannot write that sentence, the choice is arbitrary; reconsider.

## 7. Decision Process

1. Read DESIGN BRIEF. Extract dials: VARIANCE, MOTION, DENSITY (each 1-10). Extract subject matter, audience, and page kind.
2. Determine page kind: landing | app | docs | portfolio | editorial | commerce | other.
3. Run the ui-ux-pro-max search from Section 4. If it covers this domain, prefer its output.
4. Pick ONE style pole from Section 6. Do not blend.
5. Choose palette under Lila Rule + Premium-Consumer Ban. Lock single accent.
6. Choose typography. Check Fraunces/Instrument ban. Check serif discipline.
7. Generate ASCII wireframe showing primary layout and spacing proportions.
8. Emit DIRECTION.md using the token table. Run contrast checks.
9. Stop. Do not write HTML/CSS here. Hand off to [system](system.md).

## 8. Short Design Plan Wireframe (Mandatory in DIRECTION.md)

Before coding, create an ASCII block schematic of the primary viewport. It guarantees visual hierarchy and prevents standard card-stack repetition.

```
+-------------------------------------------------------------+
| LOGO                        NAV ITEMS                 [CTA] |
+-------------------------------------------------------------+
|                                                             |
|  [EYEBROW BADGE]                                            |
|  MAIN VALUE PROPOSITION HEADLINE                            |
|  Tight, max two lines desktop                               |
|                                                             |
|  Supporting descriptive copy under twenty words.            |
|  Explains clear value and who it is built for.              |
|                                                             |
|  [ PRIMARY CTA ]   [ SECONDARY LINK ]                       |
|                                                             |
|  +-------------------------------------------------------+  |
|  | HERO ASSET: Real interface view or live canvas         |  |
|  | No decorative abstract 3D floating geometries         |  |
|  +-------------------------------------------------------+  |
|                                                             |
+-------------------------------------------------------------+
```

Rules for the wireframe:
- Show exact placement of the four allowed hero text elements.
- Verify primary CTA is placed above the fold fold-line.
- Keep structural rhythm varied. Do not repeat identical card rows.

## 8.5 Two-pass review before code (frontend-design)

You have a plan (pole, palette, pairing, wireframe) but not yet code. Review for templating BEFORE any HTML/CSS:

1. Re-read your plan as if it were for *any* similar prompt (swap the subject for a neighbouring brand). If any part reads identically - the same palette family, the same hero framing, the same bento rhythm - it is a generic default, not a choice for this subject.
2. Revise that part and state what changed + why in `DIRECTION.md` (e.g. "Swapped premium-beige for Cold Luxury: logistics brand wants off-black + electric blue for scan contrast").
3. Only then emit `DIRECTION.md` and hand off to [system](system.md). One sentence of rationale per revised pole is the cost of avoiding templating.

## 9. Worked Example

Brief: B2B analytics dashboard for logistics operations. Professional, fast data scanning. Dark mode preferred. Dials: VARIANCE 3, MOTION 4, DENSITY 7.

DIRECTION.md content:

- Style pole: product
- Palette (dark):
  - `--bg`: `#0b0f14`
  - `--surface`: `#131a22`
  - `--text`: `#e5edf4`
  - `--text-muted`: `#94a3b8`
  - `--accent`: `#3b82f6` (electric blue, supports logistics-tech connotation)
  - `--accent-contrast`: `#ffffff`
  - `--border`: `#1f2937`
  - `--destructive`: `#ef4444`
- Typography:
  - `--font-display`: Geist
  - `--font-body`: Geist
  - `--font-mono`: Geist Mono
- Radius: 8px (`--radius`), 12px (`--radius-lg`), no pills
- Effects: row hover states, skeleton loaders, one scroll reveal for charts. No glow, no gradients.
- Container: 1360px

Wireframe:
```
+-------------------------------------------------------------+
| [LOGISTICS OS]        Fleet   Routes   Drivers      [Alert] |
+-------------------------------------------------------------+
| METRICS BAR: Active Trucks (1,420)   Delayed (3)   On-Time  |
| [=========================================================] |
|                                                             |
| +-------------------------+ +-----------------------------+ |
| | DISPATCH QUEUE          | | LIVE ROUTE TELEMETRY        | |
| | Row 1: Vehicle #402     | | Real-time map component     | |
| | Row 2: Vehicle #109     | | High-contrast vector paths  | |
| | Row 3: Vehicle #883     | |                             | |
| +-------------------------+ +-----------------------------+ |
+-------------------------------------------------------------+
```

Checks (all PASS):
1. contrast.py: all pairs 4.5:1 or better
2. Not a banned premium-consumer palette
3. Lila Rule OK: no purple, no cyan-magenta
4. Fonts not banned: Geist is allowed
5. Single accent: blue only
6. One radius system: consistent
7. MOTION 4 allows micro + one reveal: respected

## 10. Hard Bans (Do Not Emit)

- `font-family: Fraunces` or `font-family: "Instrument Serif"`
- Any hex from Section 2.2 banned lists when the brief is premium-consumer
- More than one `--accent` color
- Mixed radius systems on one page (sharp buttons + pill cards)
- A direction that contradicts DENSITY (low density with cramped spacing, or vice versa)
- `--font-display` and `--font-body` that are visually identical
- A DIRECTION.md that contains "lorem" or placeholder copy

## 11. Failure Modes and Recovery

- Search returns empty: state "no verified pack" and use built-in guidance. Do not invent a pack output.
- Brief asks for warm-craft but is premium-consumer: apply palette rotation to an allowed family from Section 2.2.
- Accent fails contrast: shift lightness/saturation, re-run `contrast.py`. Do not keep a broken accent.
- Two serif picks in one pairing: reject. Serif is display-only; body stays sans.
- Client demands purple/cyan glow: downgrade to subtle border color or remove. The Lila Rule is about avoiding the default LLM look, not forbidding purple ever.

## 12. Handoff Contract

The DIRECTION.md file is the contract. It contains:
- Complete token table with real values (hex, px, font names)
- Font import source (Google Fonts URL or `@font-face` path)
- One paragraph of subject-matter rationale
- ASCII block wireframe showing core spatial relationship
- Explicit allowed-effects list and banned-effects list
- One-line statement of style pole

## 13. Checks (Mandatory Verification)

Run these checks mechanically before outputting `DIRECTION.md`. If any check fails, do not proceed to [system](system.md).

1. Contrast check: run `scripts/contrast.py` on `--bg` vs `--text`, `--bg` vs `--accent`, and `--accent` vs `--accent-contrast`. All must return PASS.
2. Token completeness: all 16 canonical tokens defined with real values; zero placeholders, zero raw hex outside token map.
3. Lila Rule audit: no purple or blue ambient glows, no cyan-magenta gradients, accent saturation under 80% unless high-energy consumer.
4. Premium-consumer audit: if page is artisan, wellness, DTC, or culinary, zero hex values from the Section 2.2 banned list.
5. Typography verification: display font is not Fraunces or Instrument_Serif; serif is used only when editorial/luxury voice is documented.
6. Accent lock: exactly one accent color defined for the entire project.
7. Dial alignment: VARIANCE, MOTION, and DENSITY rules strictly respected; no complex motion at MOTION <= 3, no dense UI at DENSITY <= 3.
