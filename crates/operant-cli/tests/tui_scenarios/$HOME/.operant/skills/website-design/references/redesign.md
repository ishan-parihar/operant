# Redesign: Change an Existing Site Without Breaking It

Redesign an existing site without breaking it: audit-first (screenshots, accessibility baseline, full inventory), dial rules per mode (preserve = match +1 MOTION, overhaul = +2 VARIANCE/MOTION), preservation rules, modernisation levers in priority order. Use whenever a site, repo, or URL already exists and the user asks to modernize, refresh, rebrand, reskin, or redo it. Run BEFORE direction/, before any file is edited, before any color or component changes.

Misclassifying the mode and skipping the audit are the two ways redesigns go
wrong. This leaf exists to prevent both. It runs between [briefing](briefing.md) mode
detection and [direction](direction.md): briefing detects that a site exists and routes
here; this leaf produces the audit and the dial readings; then the normal
pipeline continues with real constraints instead of defaults.

## Procedure

1. Confirm the mode from [briefing](briefing.md): greenfield, preserve, or overhaul. If
   intent is still unclear, this counts as the one allowed question: "Should
   this redesign preserve the existing brand, or are we starting visually from
   scratch?"
2. Run the audit in Section 1 in full, before editing any file. The audit is
   not optional even for a "small refresh".
3. Read the existing site's dials off the audit (Section 2), then apply the
   mode's dial rule (Section 3).
4. Apply preservation rules (Section 4) as hard constraints on every later
   phase. Write them into the DESIGN BRIEF block if briefing has not run yet.
5. Choose modernisation levers in priority order (Section 5), stopping when
   the brief is satisfied.
6. Route out per the decision tree (Section 6).
7. Commit the audit before touching code ([version-control](version-control.md)).

## 1. Audit first: six inventories, all mechanical

Document the current state IN WRITING before proposing anything. Output: one
`REDESIGN AUDIT` fenced block. Do not redesign from memory of the homepage.

### 1.1 How to run the audit (tools)

| Step | Command or tool | Deliverable |
|---|---|---|
| Capture desktop and mobile screenshots | headless browser (Playwright, Chromium `--screenshot`, or the project capture script) at 1440x900 and 375x812 | PNG files under `audit/` |
| Run the full automated audit | `squirrel audit <url> --format llm` via [audit](quality/audit.md) | LLM-format report file, stored next to screenshots |
| Measure contrast | `scripts/contrast.py <#hex-a> <#hex-b>` | ratio plus pass or fail per WCAG AA |
| Enumerate missing alt text and nav labels | `grep -E '<img(?!.*alt=)'` and the nav markup | items for the retire list |

Run the audit BEFORE any edit. The audit is a snapshot of a moving target;
re-auditing after you start editing contaminates the baseline.

### 1.2 Screenshots (evidence, not vibes)

Capture and keep as files under the site repo (e.g. `audit/`):

| Shot | How | Why |
|---|---|---|
| Every key page, desktop viewport | headless browser screenshot at 1440x900 | layout baseline you diff against after changes |
| Same pages, mobile viewport | 375x812 | mobile breakage is the most common redesign regression |
| Interactive states of the hero and nav | hover, open menus, scrolled position | these never appear in static review and always break |

If [audit](quality/audit.md) is available, run `squirrel audit <url> --format llm` now
and store the report next to the screenshots. It is the performance, SEO, and
accessibility source of truth; do not re-derive its findings by eye.

### 1.3 Accessibility baseline (record numbers, not impressions)

Never regress these. Before changing anything, record:

| Check | Target | Tool |
|---|---|---|
| Body text contrast on every section background | >= 4.5:1 | `scripts/contrast.py` |
| Large text and icon contrast | >= 3:1 | `scripts/contrast.py` |
| Focus indicator visible on every interactive element | manual keyboard pass (Tab through the page top to bottom) | keyboard |
| All images have useful `alt` | grep for `<img` missing `alt`, and for `alt=""` on non-decorative images | grep |
| Landmarks present | one `<main>`, one `<nav>`, heading levels never skip (h1 -> h2 -> h3) | DOM query |
| `prefers-reduced-motion` honored | all animation gated behind the media query | grep for the media query |

Full rules live in [accessibility](quality/accessibility.md); this table is the before
snapshot. Any number that gets WORSE after the redesign is a fail, even if the
page looks better.

### 1.4 Baseline inventory

Six lists. Be exhaustive; later phases read these, not the old code.

1. **Brand tokens:** primary and accent colors (exact hex), type stack (faces
   and roles), logo treatment, border radii, shadow character.
2. **Information architecture:** page tree, route slugs, anchor IDs, primary
   nav labels and order, key conversion paths (entry -> CTA -> thank-you).
3. **Content blocks:** every section on key pages, what exists, what is doing
   work, what is filler. Mark filler explicitly; it is the only safe delete.
4. **Patterns to preserve:** signature interactions, the recognisable hero,
   the copy voice, anything a returning user would notice missing.
5. **Patterns to retire:** AI-slop tells (list per
   [anti-slop](quality/anti-slop.md)), broken layouts, dead links, generic stock
   imagery, slow third-party scripts. Each retired pattern needs a reason.
6. **SEO baseline:** ranking pages, meta titles, structured data present, OG
   cards. SEO migration is the number one redesign risk; this list is the
   migration checklist later.

### 1.5 Output format

```
REDESIGN AUDIT
mode: <preserve | overhaul>
screenshots: <paths, desktop+mobile, N pages>
a11y baseline: contrast <min found>:1, focus <ok|broken>, alt <ok|N missing>, landmarks <ok|issues>, reduced-motion <ok|missing>
brand tokens: colors <hex list>, type <faces>, radii <values>
IA: <N routes, slugs listed>, nav <labels>, conversion path <steps>
preserve: <3-6 named patterns>
retire: <3-6 named patterns, each with reason>
seo: <ranking pages, meta titles present y/n, OG y/n>
dial reading: VARIANCE=<n> MOTION=<n> DENSITY=<n>
```

## 1.6 Audit traps: the regressions that never show in screenshots

Screenshots catch nothing here. These are checked by hand, on purpose:

| Trap | Why it happens | Guard |
|---|---|---|
| Slug or label change | looks cosmetic, breaks bookmarks and SEO | Section 4 rule 1 hard-gates it |
| Form field rename | breaks analytics events and password autofill | Section 4 rule 5; list renames for approval |
| Alt text dropped in a rebuild | new markup forgets what old markup carried | diff the alt inventory before vs after |
| Keyboard path broken | new component cancels the `focusin` navigation | replay the Tab pass from the baseline |
| Reduced-motion unset | new animation ignores the OS setting | gate all new animation behind the media query |
| Contrast drift on the accent | recolor nudges the brand accent under 4.5:1 on body use | rerun `scripts/contrast.py` on every token change |

## 2. Reading dials off an existing site

The site's current dials are the start position, never the greenfield
baseline table. Estimate by observation:

| Observation | Reading |
|---|---|
| Strict columns, equal cards, aligned everything | VARIANCE 2-4 |
| Some offset grids, mixed image ratios | VARIANCE 5-7 |
| Asymmetric, broken grid, art-directed sections | VARIANCE 8-10 |
| Hover states only | MOTION 1-3 |
| Transitions, staggered fade-ins | MOTION 4-7 |
| Scroll-driven animation, parallax, pinned sections | MOTION 8-10 |
| Section padding >= 128px, few items per screen | DENSITY 1-3 |
| Standard 64-96px rhythm | DENSITY 4-7 |
| Tight tables, borders-as-dividers, data everywhere | DENSITY 8-10 |

## 3. Dial rules per mode

| Mode | VARIANCE | MOTION | DENSITY |
|---|---|---|---|
| preserve | match existing | existing +1 | match existing |
| overhaul | existing +2 | existing +2 | match existing |
| greenfield (brand itself changing) | baseline tables in [briefing](briefing.md) | same | same |

Why: preserve modernises the feel, motion is the cheapest feel upgrade, so
MOTION rises one notch while layout stability is respected. Overhaul is a new
visual language on kept content, so both variance and motion step up. Density
never changes silently: information load is part of the product's contract
with its users.

Cap the results: quiet constraints (regulated, public-sector,
accessibility-first) still cap VARIANCE at 4 and MOTION at 3, whatever the
math above says.

## 4. Preservation rules (hard constraints)

Format: rule, reason, then the override condition. Any override needs an
explicit line in the delivered summary stating what changed.

1. **Do not change information architecture unless asked.** Reason: route
   slugs, anchor IDs, and primary nav labels carry SEO equity and user muscle
   memory; a URL change points existing links at 404s. Override only: the IA
   itself is a top audit finding (broken structure) and the user approved the
   new IA.
2. **Extract brand colors before applying any direction palette.** Reason: a
   brand that is already purple stays purple; recognition ships in the accent.
   Override only: mode is overhaul with an approved new brand palette.
3. **Preserve copy voice unless asked for a rewrite.** Reason: visual
   modernisation is not a content rewrite; changing good copy reads as a
   rebrand the user did not ask for. Override only: copy is on the retire list
   (broken, wrong tone, filler).
4. **Never regress the accessibility baseline from 1.3.** Reason: a redesign
   that loses contrast or focus states is a regression, not a redesign.
   Override: none. Equal or better on every row, always.
5. **Respect existing analytics and integrations.** Reason: renamed button
   text, form field names, or section IDs silently break tracking, marketing,
   and autofill. Override only: the rename is design-mandated, is listed in
   the audit, and the owner approved it explicitly.

## 5. Modernisation levers (priority order)

Apply in order. Stop when the brief is satisfied; most preserve jobs end at
lever 3 or 4. Full rules for each lever live in their own leaves.

| # | Lever | What it means | Feed from |
|---|---|---|---|
| 1 | Typography refresh | new pairing, correct scale and roles | [direction](direction.md) references/font-pairings.md |
| 2 | Spacing and rhythm | section padding to the audited DENSITY, consistent vertical rhythm | [system](system.md) |
| 3 | Color recalibration | desaturate, unify neutrals, keep the brand accent | [system](system.md) token recolor only |
| 4 | Motion layer | micro-interactions at the new MOTION value on EXISTING components | [motion](motion.md) |
| 5 | Hero and key-section recomposition | restructure top-of-funnel using component recipes | `components/*`, one section at a time |
| 6 | Full block replacement | only when the audited block is flagged unsalvageable | `components/*` |

Biggest lift per unit of risk is lever 1, typography. Do not jump to lever 5
because it feels more productive; levers 1-3 ship in a day and carry
~70 percent of the perceived redesign at a fraction of the regression risk.

### 5.1 Serialization: one lever, one change set

Never land two levers in the same unreviewed batch. If levers 1 and 2 regress
together, you cannot tell which one broke the page. Ship lever by lever:

1. Apply the lever to all affected tokens, styles, or markup.
2. Re-run [preflight](quality/preflight.md) checks on the touched pages.
3. Diff the page against its saved screenshot from the audit.
4. If the diff regressed any accessibility number, revert the lever, re-apply
   with the constraint fixed, then continue.
5. Commit, then move to the next lever.

This is slower per lever and much faster overall, because you never debug a
merge of three systemic changes at once.

## 6. Decision tree: evolution vs full redesign vs greenfield

| Audit finding | Route |
|---|---|
| IA, content, and SEO are sound | targeted evolution, levers 1-4, mode stays preserve |
| Visual debt is structural (broken IA, no design system, broken mobile) | full redesign in overhaul mode, strict content preservation, all six levers |
| The brand itself is changing (new name, new positioning, new palette mandated) | greenfield. Leave this leaf; restart pipeline at [briefing](briefing.md) with mode greenfield |

## 7. What never changes silently

Never modify without explicit user approval:

- URL structure and route slugs.
- Primary nav labels.
- Form field names or field order (breaks analytics and password autofill).
- Brand logo or wordmark.
- Existing legal, consent, and cookie copy.

## Worked example (preserve mode)

Site: marketing page for a regional bakery chain, built five years ago. User
said: "Please modernize the site, it feels dated. We love our brand." Mode:
preserve (existing site plus "modernize, keep our brand").

Audit: 14 screenshots taken (7 pages, desktop plus mobile). Accessibility
baseline: body contrast 5.8:1 (ok), one nav submenu unreachable by keyboard
(must FIX, not regress), 9 images missing alt, reduced-motion not honored by
the slideshow. Brand tokens: terracotta `#B4543B` accent, cream `#F7F2EA`
background, Fraunces display plus system body. IA: 7 routes, slugs stable,
nav Order/Menu/About/Visit/Careers. Preserve: the hand-written location cards,
the "baked at 5am" copy voice, the menu PDF link. Retire: autoplay slideshow
(slop plus accessibility), three equal stock photo cards on the home hero,
dead /specials route linked from footer. SEO: /menu and /visit rank 1-3;
meta titles present; no OG cards.

Dial reading: VARIANCE 3 (strict grid), MOTION 2 (slideshow only), DENSITY 5.
Preserve rule: VARIANCE 3, MOTION 3 (2+1), DENSITY 5.

Levers applied: 1 typography refresh (keep Fraunces, fix the scale, drop the
fourth font), 2 rhythm (padding unified to 96px sections), 3 color (unify the
four grays into two neutrals, terracotta untouched), 4 motion (replace the
slideshow with a crossfade gallery honoring reduced-motion, add subtle reveal
on location cards). Stopped at 4: the brief said "modernize", not
"restructure". Never-touch list respected: slugs, nav labels, and the online
order form field names were not modified.

Post-check against baseline: contrast now 6.9:1, submenu keyboard reachable,
alt on all images, reduced-motion honored. Zero accessibility regressions.

## 8. Verify against the baseline (before delivering)

The redesign is not done when the code compiles. It is done when the audit
measurements confirm improvement. Run all of these:

1. Re-capture the same screenshots (same pages, same two viewports) and lay
   them next to the audit set. Scan for missing sections, cropped CTAs, and
   mobile overflow.
2. Re-run `squirrel audit <url> --format llm --diff` per
   [audit](quality/audit.md). Every accessibility or SEO regression must be
   fixed; performance may only regress with a named, approved reason (new hero
   media, etc.).
3. Re-run the keyboard pass on the hero, nav, and every form.
4. Re-run `scripts/contrast.py` on every recolored foreground/background pair.
5. Walk Section 7's never-change list and confirm zero silent changes.
6. Cross-check the `retire` list: every retired pattern is actually gone, and
   nothing new from [anti-slop](quality/anti-slop.md) was introduced.

Then run [preflight](quality/preflight.md) for the final gate before delivery.

## Pipeline navigation

## Checks

1. A `REDESIGN AUDIT` block exists with all sections filled (screenshots, a11y baseline, brand tokens, IA, preserve, retire, SEO, dial reading) BEFORE any source file was edited.
2. Screenshot files exist for every key page at both desktop and mobile sizes.
3. The accessibility baseline records at least contrast values, focus behaviour, alt coverage, and reduced-motion handling with concrete values, not adjectives.
4. Dials match the mode rule exactly: preserve = VARIANCE match, MOTION existing+1, DENSITY match; overhaul = VARIANCE +2, MOTION +2, DENSITY match; results clamp to 1-10 and to the quiet-constraint caps.
5. Every "preserve" pattern named in the audit is still present (or its removal was explicitly approved by the user).
6. No item from Section 7 (slugs, nav labels, form field names, logo, legal copy) was modified without explicit user approval.
7. Post-redesign accessibility numbers are equal or better on every row of the 1.3 baseline table.

Output: the `REDESIGN AUDIT` block plus the mode-derived dials flow BACK into
the DESIGN BRIEF (briefing/), then direction/, system/, structure/, and the
rest of the pipeline run as normal: the audit is the only redesign-specific
phase. All BUILD phases still obey version-control/: commit after each lever,
push before you stop.
