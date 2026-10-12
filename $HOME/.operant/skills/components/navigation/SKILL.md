---
name: navigation
description: "Build the header, primary nav, mobile menu, and footer. Use IMMEDIATELY when the SECTION MAP reaches the header or footer, or whenever a nav wraps to two lines, exceeds 80px, lacks an active state, or a footer carries version stamps or dead links."
metadata:
  operant:
    tags: [nav, header, footer, mobile-menu]
---

# Navigation

The header and footer frame every page: they must be consistent, one line at
desktop, keyboard reachable, and free of decoration fixtures. Build the header
first, the footer last, both from this leaf.

## Procedure

1. Read the `DESIGN BRIEF` (page kind, DENSITY) and the site map. List every
   top-level destination BEFORE writing markup.
2. Pick the nav pattern from the adaptive table below.
3. Build the header to the header spec, including the mobile menu, in one file.
4. Build the footer to the footer spec, treating it as the page's sitemap.
5. Run `## Checks`. Fix fails before continuing the pipeline.

Framework note: for Tailwind, React, Vue, or shadcn output, map these token
names and recipes via `references/stack-adapters.md`. The limits here do not
change per stack.

## Header spec

| Rule | Value | Why / override |
|---|---|---|
| Height, desktop | 64-72px default, 80px absolute max | taller bars eat viewport; no override |
| Lines at >=1024px | exactly 1 | two-line desktop nav is broken; condense labels, drop items, or go hamburger |
| Item budget | brand + 4-6 links + 1 CTA | more items go to an overflow menu or grouped dropdown, never crammed |
| Active state | current page marked with color or weight plus an indicator, and `aria-current="page"` | users must see where they are; applies on every page |
| CTA | one, reuses the hero's primary intent and label | duplicate CTA intents are a preflight fail |
| Sticky | `position: sticky; top: 0` with `background: var(--bg)` and a 1px `var(--border)` bottom line | plus `html { scroll-padding-top: <header height> }` so anchor targets are not hidden |
| Mobile trigger | `<button>` with `aria-expanded` and `aria-controls`, visible at <1024px | never a bare icon div |
| Mobile menu | full-screen overlay (marketing) or drawer (app); focus trapped while open; ESC closes; body scroll locked | one interactive pattern, not both |
| Consistency | same placement, order, and labels on every page | never restyle nav per page type |

## Adaptive pattern selection

| Context | Pattern |
|---|---|
| Marketing, landing, blog, portfolio | top bar (this leaf's spec) |
| Docs or dashboard at >=1024px | sidebar for section nav + slim top bar for global items |
| Any page kind at <1024px | top bar + drawer or full-screen menu |
| Hierarchy 3+ levels deep | add breadcrumbs directly under the header |

Additional rules, no exceptions:

- Primary nav and secondary nav stay separated. Settings, logout, and account
  live in a secondary cluster (avatar menu, drawer footer), never mixed into
  primary links.
- Core navigation stays reachable from deep pages. Do not hide the header
  inside sub-flows.
- Breadcrumbs: each ancestor is a link, current page is plain text with
  `aria-current="page"`, wrapped in `<nav aria-label="Breadcrumb">`.
- One pattern per hierarchy level. Never mix tabs + sidebar + bottom nav for
  the same level.

## Footer spec

The footer is the sitemap, not a decoration zone.

| Slot | Rule |
|---|---|
| Brand column | logo or wordmark + one line of description, nothing poetic |
| Link groups | 2-4 groups (Product, Company, Resources, Legal), 3-6 links each, every top-level page reachable |
| Legal row | copyright with current year, privacy, terms; smallest text on the page but still >= 4.5:1 contrast |
| Contact | one email and, if the brief has a physical venue, one address line; atmospheric locale, time, or weather strips are banned, see `quality/anti-slop/SKILL.md` |
| Social | real profile links or omit the row entirely |
| Newsletter (optional) | label above input, real submit handling or omit |

Banned fixtures (`quality/anti-slop/SKILL.md` holds the full list, link do not
copy): version footers (`v1.4.2`, `Build 0048`, `last sync 4s ago`), weather or
locale strips, decorative status dots. Every footer link resolves; `href="#"`
is a dead link, not a placeholder.

## Worked example: header + footer

Marketing site, top-bar pattern, DENSITY 4.

```html
<header class="site-header">
  <a class="brand" href="/">Kelpline</a>
  <nav class="primary-nav" id="primary-nav" aria-label="Primary">
    <ul>
      <li><a href="/product" aria-current="page">Product</a></li>
      <li><a href="/pricing">Pricing</a></li>
      <li><a href="/docs">Docs</a></li>
      <li><a href="/blog">Blog</a></li>
    </ul>
    <a class="btn-primary" href="/signup">Start free</a>
  </nav>
  <button class="menu-toggle" aria-expanded="false"
          aria-controls="primary-nav">Menu</button>
</header>
```

```css
html { scroll-padding-top: 72px; }         /* matches header height */
.site-header {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 72px;                            /* 64-72 default, 80 max */
  padding-inline: var(--space-6);
  background: var(--bg);
  border-bottom: 1px solid var(--border);
}
.brand { font-family: var(--font-display); font-weight: 700; color: var(--text); }
.primary-nav { display: flex; align-items: center; gap: var(--space-6); }
.primary-nav ul {
  display: flex;
  gap: var(--space-6);
  list-style: none;
  margin: 0;
  padding: 0;
  white-space: nowrap;                     /* single line at desktop */
}
.primary-nav a { color: var(--text-muted); text-decoration: none; }
.primary-nav a[aria-current="page"] {
  color: var(--text);
  font-weight: 600;
  border-bottom: 2px solid var(--accent);  /* visible active indicator */
}
.primary-nav a:focus-visible,
.menu-toggle:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.btn-primary {
  background: var(--accent);
  color: var(--accent-contrast);
  padding: var(--space-2) var(--space-5);
  border-radius: var(--radius);
}
.menu-toggle { display: none; }

/* Mobile collapse: full-screen menu, declared in the same component */
@media (max-width: 1023px) {
  .menu-toggle { display: inline-flex; }
  .primary-nav { display: none; }
  .primary-nav[data-open] {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-6);
    position: fixed;
    inset: 72px 0 0 0;
    padding: var(--space-6);
    background: var(--bg);
  }
  .primary-nav[data-open] ul { flex-direction: column; gap: var(--space-5); }
}
```

Toggle behavior (any stack, a few lines of JS or a framework state flag): the
button flips `aria-expanded` and sets `data-open` on the nav; while open, trap
focus inside the nav, lock body scroll, and close on ESC returning focus to
the button.

```html
<footer class="site-footer">
  <div class="footer-grid">
    <div class="footer-brand">
      <a class="brand" href="/">Kelpline</a>
      <p>Continuous deploy for monorepos.</p>
      <a href="mailto:hello@kelpline.dev">hello@kelpline.dev</a>
    </div>
    <nav aria-label="Footer">
      <div class="footer-group">
        <h2>Product</h2>
        <ul>
          <li><a href="/product">Overview</a></li>
          <li><a href="/pricing">Pricing</a></li>
          <li><a href="/changelog">Changelog</a></li>
        </ul>
      </div>
      <div class="footer-group">
        <h2>Company</h2>
        <ul>
          <li><a href="/about">About</a></li>
          <li><a href="/blog">Blog</a></li>
          <li><a href="/careers">Careers</a></li>
        </ul>
      </div>
      <div class="footer-group">
        <h2>Legal</h2>
        <ul>
          <li><a href="/privacy">Privacy</a></li>
          <li><a href="/terms">Terms</a></li>
        </ul>
      </div>
    </nav>
  </div>
  <div class="legal-row">
    <p>&copy; 2026 Kelpline, Inc.</p>
  </div>
</footer>
```

```css
.site-footer {
  background: var(--surface);
  border-top: 1px solid var(--border);
  padding: var(--space-8) var(--space-6) var(--space-6);
}
.footer-grid {
  display: grid;
  grid-template-columns: 2fr 3fr;
  gap: var(--space-8);
  max-width: var(--container);
  margin-inline: auto;
}
.site-footer nav { display: grid; grid-template-columns: repeat(3, 1fr); gap: var(--space-6); }
.footer-group h2 {
  font-size: 13px;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--text-muted);
  margin: 0 0 var(--space-3);
}
.footer-group ul { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--space-2); }
.footer-group a { color: var(--text); text-decoration: none; }
.footer-group a:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.legal-row {
  max-width: var(--container);
  margin: var(--space-6) auto 0;
  padding-top: var(--space-4);
  border-top: 1px solid var(--border);
  color: var(--text-muted);
  font-size: 14px;
}

@media (max-width: 767px) {
  .footer-grid { grid-template-columns: 1fr; gap: var(--space-6); }
  .site-footer nav { grid-template-columns: 1fr 1fr; }
}
```

## Checks

1. At 1280px the primary nav renders on exactly one line, no wrapped items.
2. Computed header height <= 80px at desktop.
3. Exactly one nav link per page has `aria-current="page"` and is visually
   distinct from the others.
4. Sticky header pairs with `scroll-padding-top` on `html` >= the header
   height.
5. Mobile toggle is a `<button>` with `aria-expanded` and `aria-controls`; the
   open menu traps focus and closes on ESC.
6. Grep the footer for `v\d`, `Build `, `last sync`, weather or locale strips:
   zero matches (full ban list in `quality/anti-slop/SKILL.md`).
7. Every header and footer link has a real href (no `href="#"`) and shows a
   visible `:focus-visible` outline when tabbed to.
8. Nav placement, order, and labels are identical across all pages, and pages
   3+ levels deep show breadcrumbs.
