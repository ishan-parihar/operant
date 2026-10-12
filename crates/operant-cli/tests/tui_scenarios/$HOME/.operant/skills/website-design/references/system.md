# SYSTEM: Tokens, Scale, Dark Mode

| Turns the DIRECTION file into the TOKENS artifact: every CSS custom property the build uses, plus typography scale, density-gated spacing, dark-mode block, z-index scale, and elevation. No component code is written before these tokens exist and pass WCAG checks.

- Input: `DIRECTION.md` produced by [direction](direction.md) (style family, palette, font pairing, effects budget, DENSITY dial).
- Output: `TOKENS.css` (or `tokens.css`) written to the project root, containing light + dark values, plus a `TOKENS` fenced block echoed back in the reply.
- Downstream consumers: [structure](structure.md), [components](components.md), [motion](motion.md). They reference token names only, never raw hex or pixel values.

One page, one system. If a value is not in TOKENS, it does not exist in the CSS.

## 1. Canonical Token Set (Never Invent Names)

Emit exactly these custom properties on `:root`. No aliases, no renames, no project-specific one-offs.

| Token | Role | Constraint |
|---|---|---|
| `--bg` | Page background | 4.5:1 vs `--text` |
| `--surface` | Cards, nav, panels | 4.5:1 vs `--text`; never pure `#000` or `#fff` |
| `--text` | Primary text | WCAG AA vs `--bg` |
| `--text-muted` | Captions, metadata | 4.5:1 vs `--bg`; do not go below 60% of `--text` lightness distance |
| `--accent` | Primary action, links | 3:1 vs `--bg`; defensible from the brief |
| `--accent-contrast` | Text on accent fills | 4.5:1 vs `--accent` |
| `--border` | Hairlines, dividers | Visibly distinct from `--bg`; 1px use only |
| `--destructive` | Delete, error states | 4.5:1 vs its background |
| `--radius`, `--radius-lg` | Corner system | See Section 6 lock rules |
| `--font-display` | Headlines, hero, display numerals | From DIRECTION font pairing |
| `--font-body` | Paragraphs, UI labels | Readable at 16px |
| `--font-mono` | Code, tabular numerals, counters | Mandatory at DENSITY 8-10 for numbers |
| `--space-1 .. --space-10` | Spacing scale | 4, 8, 12, 16, 24, 32, 48, 64, 96, 128 px |
| `--container` | Max content width | 1200-1440px landing; 640-760px editorial reading column |
| `--shadow-1`, `--shadow-2` | Elevation | See Section 5; never `rgba(0,0,0,0.1)` generic grey |
| `--duration-fast`, `--duration-med` | Motion timing | 150ms / 300ms; nothing in the site exceeds 500ms except scroll choreography from [motion](motion.md) |

Hard rules:
1. All color, spacing, font, radius, and shadow values in component CSS reference these variables. Raw hex appears exactly twice in the project: once in the light `:root`, once in the dark block.
2. Verify every color pair with `scripts/contrast.py` before writing the file. FAIL means adjust hexes and re-run, not "check later in the browser".
3. Never emit pure `#000000` or `#ffffff`. Use off-black (near-950 zinc/warm-gray) and off-white. Pure values kill depth perception.

## 1.5 How Later Phases Consume TOKENS

Component recipes reference tokens by name and never restate values:

```css
.hero { padding-block: var(--space-8); background: var(--bg); }
.hero h1 { font-family: var(--font-display); color: var(--text); }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-lg); box-shadow: var(--shadow-1); }
.button-primary { background: var(--accent); color: var(--accent-contrast); border-radius: var(--radius); }
```

If a recipe needs a value that has no token, the correct move is to add one row to TOKENS and note why, not to inline a literal. This keeps the whole page restyleable from one file, which is what makes redesign and dark-mode work cheap.

## 2. Typography Scale

Derive a modular scale from the DIRECTION pairing. Marketing pages use `1.25` (major third); DENSITY 8-10 may use `1.2` (minor third). Start from `--font-size: 16px` base.

| Step | Class name | Formula at 1.25 | Use |
|---|---|---|---|
| `text-xs` | eyebrow, caption | 12-13px | Overline, fine print |
| `text-sm` | small | 14px | Secondary UI text |
| `text-base` | body | 16px | Paragraphs (never smaller) |
| `text-lg` | lead | 20px | Section intros |
| `text-xl` | h3 | 24-25px | Card titles |
| `text-2xl` | h2 | 31-32px | Section headings |
| `text-3xl` | h1 | 39-40px | Page-level heading |
| `text-display` | hero | 49-64px+ | Hero only; clamp with `clamp()` |

Rules:
1. Fluid sizes use `clamp(min, preferred, max)`. Example hero: `font-size: clamp(2.5rem, 5vw + 1rem, 4.5rem);`.
2. Line-height is 1.5 for body, 1.1-1.2 for display. Never default `1` or `2`.
3. Letter-spacing: negative (-0.01em to -0.03em) on display sizes only. Body and small text keep default spacing.
4. Load fonts with `next/font` or self-hosted `@font-face` + `font-display: swap`. Never `<link>` to Google Fonts in production HTML.

## 3. Density-Driven Spacing

Spacing sections by the DENSITY dial from the brief. This is the ui-ux-pro-max `--density` contract expressed in tokens.

| DENSITY | Feel | Section padding (`padding-block`) | Component gaps | Notes |
|---|---|---|---|---|
| 1-3 | Art gallery | 96px (`--space-9/-10`) up to `py-32..py-48` | 24-96px | Huge whitespace; narrow `--container` (max 1120px) |
| 4-7 | Daily app / standard marketing | 64-96px | 16-64px | Default band. Most landings live here |
| 8-10 | Cockpit / dashboard | 32-48px | 8-32px | 1px borders instead of card boxes; `font-mono` for all numerals |

Rules:
1. Pick the spacing column once, from the brief's DENSITY value. Never mix bands on one page.
2. Use only `--space-*` steps. If a gap "needs" 20px, the layout is off-grid; use 16 or 24.
3. Section vertical rhythm is `padding-block`, not `margin`, so background colors extend edge to edge.
4. `--container` is applied with `margin-inline: auto; padding-inline: var(--space-4);` at every breakpoint.

### 2.1 Type Fluency Rules

1. Only two size changes per view hierarchy level. Body plus one display family difference is enough for 90% of sections.
2. Eyebrow labels are `text-xs`, uppercase, `letter-spacing: 0.08em`, colored `--text-muted` or `--accent`, never both.
3. Long-form reading columns cap at `--container` 640-760px regardless of page `--container`; use a `.prose { max-width: 68ch; }` utility scoped to article content.
4. Numerals in stat blocks, dashboards, and pricing use `font-variant-numeric: tabular-nums` on top of `--font-mono` or `--font-display`, so columns never jitter.
5. Heading wrapping: enable `text-wrap: balance` on h1-h3, never on body paragraphs.

## 2.2 When the Pairing Is Wrong

If DIRECTION picked a display font that fails at small sizes (common with black weights and condensed faces), do not silently swap. Re-run [direction](direction.md) with the failure noted, because typography was decided there and every later recipe assumes its metrics.

## 3. Density-Driven Spacing

Every consumer-facing page ships dual-mode. One strategy per project: CSS variables here. (Tailwind `dark:` classes are the alternate; do not mix.)

```css
:root { color-scheme: light; /* light tokens */ }
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) { color-scheme: dark; /* dark tokens */ }
}
:root[data-theme="dark"] { color-scheme: dark; /* dark tokens */ }
```

Rules:
1. Duplicate ALL color tokens in the dark block: `--bg --surface --text --text-muted --accent --accent-contrast --border --destructive --shadow-1 --shadow-2`. Radii, fonts, spacing, and container do not change.
2. Dark-mode `--surface` is lighter than `--bg` (elevation by lightness), the reverse of light mode. Never darken a surface into the void.
3. Re-run `scripts/contrast.py` on dark pairs. Dark mode is the most common AA failure.
4. `--accent` may shift lightness in dark mode to hold contrast, but hue stays identical. Brand color stays recognizable in both modes.
5. A manual toggle (if the brief asks for one) sets `data-theme` on `<html>`. Default is `prefers-color-scheme`.
6. Never ship light-only or dark-only unless the brief explicitly says so, and never let one section flip theme mid-page (see [direction](direction.md) page theme lock).

### 3.1 Reading the Band Correctly

DENSITY comes from the DESIGN BRIEF and is restated in DIRECTION. If they disagree, DIRECTION wins. If the brief gives no number, default to 5 and say so in the TOKENS header comment.

Common failures to avoid:

1. Dashboard chrome built at DENSITY 4 spacing, which reads as marketing fluff to an operator audience. Dashboards live at 8-10.
2. Marketing landing at DENSITY 9, which reads as a settings page. Landings live at 4-7 unless the product is literally developer tooling for dense workers.
3. Using band padding but not band gaps. Both columns change together.

## 4. Dark Mode Block (Mandatory)

Two steps only, `--shadow-1` for resting cards, `--shadow-2` for popovers/overlays.

1. Shadows are tinted with the page background hue, not black. On a warm off-white page the shadow is a warm brown-grey alpha, not neutral grey. Derivation: take `--bg`, shift toward the accent hue by a few degrees, drop lightness, use as the shadow color at 4-12% alpha.
2. In dark mode, shadows carry almost no weight; use lighter `--surface` plus a subtle 1px `--border` highlight instead.
3. Banned: `rgba(0,0,0,0.1)` style grey smears, multi-colored glow shadows, huge diffuse `0 40px 80px` blobs on light pages. Hairline borders often replace shadows at DENSITY 4-7.

## 6. Radius Lock

Pick ONE corner system in the brief and lock it:

| System | `--radius` | `--radius-lg` | Fits |
|---|---|---|---|
| Sharp | 4px | 8px | Technical, data, B2B, brutalist |
| Standard | 8px | 12-16px | Default for most SaaS |
| Round | 12-16px | 20-24px | Consumer, playful, soft editorial |

Rules: buttons and inputs use `--radius`; cards and modals use `--radius-lg`. No per-element radius improvisation. Pills (`9999px`) are allowed for tags/chips only, never for cards.

### 5.1 Shadow in Practice

Use `--shadow-1` on resting interactive surfaces only (cards that lift on hover, dropdown panels). Static content blocks get border, not shadow. `--shadow-2` is reserved for layers that sit over other layers: popovers, drawers, toasts. If most cards have shadows, the page reads heavy; if none do, hierarchy comes from `--surface` contrast against `--bg` and from `--border` hairlines.

Hover lift, when MOTION allows, is `transform: translateY(-2px)` plus swapping `--shadow-1` to `--shadow-2`, both transitioned over `var(--duration-fast)`. Never animate box-shadow blur radius alone; it repaints constantly.

## 6. Radius Lock

Never emit `z-50`, `z-10`, or magic numbers. Define the whole stacking context once:

```css
:root {
  --z-base: 0;      /* page content */
  --z-sticky: 10;   /* sticky nav */
  --z-overlay: 20;  /* dropdowns, popovers */
  --z-fixed: 30;    /* fixed bars, floating CTAs */
  --z-modal: 40;    /* dialogs, drawers */
  --z-toast: 50;    /* notifications */
  --z-noise: 60;    /* grain overlay, pointer-events:none, fixed */
}
```

1. Grain/noise textures go on a single `position: fixed; inset: 0; pointer-events: none; z-index: var(--z-noise)` pseudo-element. Never on scrolling containers (continuous GPU repaint destroys mobile FPS).
2. Overlays only animate `transform` and `opacity`. Never `top`/`left`/`width`/`height`.
3. New layers require a new named rung, not `calc(var(--z-modal) + 1)` hacks.

### 7.1 Third-Party Widgets

Embeds (maps, chat bubbles, cookie banners) often inject their own z-index in the hundreds. Handle them by containment, not by escalating your scale: wrap the mount point and set `isolation: isolate` on the wrapper where possible, so 2147483647 inside the iframe or widget does not fight your `--z-modal`. Your own scale stays 0-60.

### 7.2 Defaults That Are Usually Wrong

Reject these when you catch yourself emitting them:

1. `--container: 1140px` from Bootstrap habit. Use 1200-1440px; 1140 is stale.
2. Ten spacing steps but only 3 used. Keep all ten in the file, but if a section invents `gap: 18px`, that is the bug.
3. `--accent: #3b82f6` (Tailwind blue-500) on a white page: luminance too close to white, fails 3:1 for large elements. Darken to a 600-700 value for light mode.
4. `--shadow-1` reused inside dark mode unchanged. Dark shadows must be re-derived, deeper and denser.
5. A z-index table with gaps at 100, 500, 1000 "just in case". Documented rungs, no empty slots.

## 8. Worked Example

### 8.1 Token Header Comment

Top of the file carries a comment block: DENSITY band, radius system name, typography scale ratio, and `contrast.py` pass date. One comment block; later phases grep for it.

Brief: B2B SaaS, DENSITY 7, VARIANCE 5, MOTION 4. Output artifact:

```css
:root {
  --bg: #fafafa; --surface: #ffffff;
  --text: #18181b; --text-muted: #52525b;
  --accent: #1663d9; --accent-contrast: #ffffff;
  --border: #e4e4e7; --destructive: #dc2626;
  --radius: 8px; --radius-lg: 16px;
  --font-display: "Space Grotesk", sans-serif;
  --font-body: "Inter", sans-serif;
  --font-mono: "JetBrains Mono", monospace;
  --space-1: 4px; --space-2: 8px; --space-3: 12px; --space-4: 16px;
  --space-5: 24px; --space-6: 32px; --space-7: 48px; --space-8: 64px;
  --space-9: 96px; --space-10: 128px;
  --container: 1280px;
  --shadow-1: 0 1px 2px rgba(24, 24, 39, 0.06), 0 1px 3px rgba(24, 24, 39, 0.08);
  --shadow-2: 0 8px 24px rgba(24, 24, 39, 0.12);
  --duration-fast: 150ms; --duration-med: 300ms;
  --z-base: 0; --z-sticky: 10; --z-overlay: 20; --z-fixed: 30;
  --z-modal: 40; --z-toast: 50; --z-noise: 60;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #101013; --surface: #1a1a1f;
    --text: #f4f4f5; --text-muted: #a1a1aa;
    --accent: #4d8dff; --accent-contrast: #0a0a0c;
    --border: #2c2c33; --destructive: #ef4444;
    --shadow-1: 0 1px 2px rgba(0, 0, 0, 0.4);
    --shadow-2: 0 8px 24px rgba(0, 0, 0, 0.5);
  }
}
```

## 9. Pass/Fail Checks (Run Before Moving On)

1. Every token in Section 1 exists in the file, light and dark. Count them: 25+ names.
2. `python3 ../scripts/contrast.py` (or the project copy) reports PASS for `--text`, `--text-muted`, `--accent`, `--accent-contrast`, `--destructive` against their backgrounds, in both modes.
3. Search the CSS for raw hex outside `:root` blocks: expect 0 hits outside comments.
4. Search for `z-50`, `z-10`, `z-index: 999`: expect 0 hits; every layer uses `--z-*`.
5. Section padding matches the DENSITY band table at every breakpoint.
6. `--container` is a single value applied once per layout; no competing max-widths.
7. Radius values on buttons/inputs/cards resolve to `--radius` or `--radius-lg` only.
8. Shadows contain no `rgba(0,0,0,0.1)`-style untinted grey on light mode.
9. Em-dash count in this file and in emitted copy: 0.

## 9.1 Self-Audit Before Hand-Off

Read the finished TOKENS file out loud as if reviewing someone else's work. Questions to answer:

1. Could a stranger restyle the whole site by editing only this file? If not, some value escaped into a component.
2. Does every accent usage pass on both themes?
3. Is the DENSITY band stated in a header comment, so structure can verify section spacing against it?
4. Would deleting `--radius-lg` break exactly the components that are cards or modals, and nothing else? If it breaks buttons too, the lock failed.

## TOOLS

- `scripts/contrast.py`: WCAG ratio checker. Required gate, not optional.
- [direction](direction.md): source of palette, fonts, dials. If DIRECTION.md is missing, stop and run direction first.
- [accessibility](quality/accessibility.md): deeper WCAG review after build.
- [performance](quality/performance.md): DOM cost and paint rules referenced in Section 7.

## MUST DO

- Write the TOKENS file before any component CSS.
- Set both light and dark values in the same edit.
- Reuse the exact token names above in every later phase.
- Gate numeric displays at DENSITY 8-10 behind `--font-mono`.
- Document any added rung in the z-index scale inline at its definition.

## MUST NOT DO

- Do not hardcode hex, px gaps, or z-index numbers in component code.
- Do not mix spacing bands across the density table.
- Do not let a single section switch themes mid-page.
- Do not use untinted grey shadows, pure black, or pure white.
- Do not copy "AI purple" gradients, generic blue-on-white SaaS sludge, or unverified accent pairs into tokens.
- Do not add tokens "just in case". A name with no consumer is deleted.
