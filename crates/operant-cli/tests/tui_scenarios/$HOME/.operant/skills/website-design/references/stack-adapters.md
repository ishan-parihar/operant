# Stack adapters: canonical tokens to concrete stacks

[system](system.md) defines the canonical token set and the language-agnostic
recipe (semantic HTML + CSS custom properties). That recipe is always the
source of truth. This file maps it mechanically onto the five stacks you will
actually be asked for. Pick exactly one adapter per build, apply it after the
`TOKENS` artifact exists, and never let the stack change the design decisions.

## Stack picker (3-line notes)

- **HTML + CSS custom properties.** Default. Nothing requested or unknown.
  No build step, no dependencies, works everywhere including partials other
  stacks import. When in doubt, ship this.
- **Tailwind v4.** Stack already uses Tailwind or the user asked. Tokens move
  into `@theme` in CSS; `tailwind.config.js` does not exist in v4.
- **React / Next.js.** Next.js App Router requested. Server Components by
  default; `"use client"` only on interactive leaves.
- **Vue 3.** Vue requested. SFCs with scoped styles; tokens stay in one global
  CSS file consumed via `var()`.
- **shadcn/ui.** Component library requested. Aliases its CSS variables onto
  the canonical tokens in one block, on top of the React adapter.

Decision rule: 1. Nothing named -> HTML + CSS. 2. Framework named -> that
recipe. 3. Component library named -> its alias recipe plus the framework it
sits on.

## 1. HTML + CSS custom properties (baseline)

The complete recipe. Replace the values with your `TOKENS` block, then write
sections as plain semantic HTML with a handful of classes.

```css
/* styles.css */
:root {
  --bg: #fafafa; --surface: #ffffff; --text: #1f2937;
  --text-muted: #6b7280; --accent: #1d4ed8; --accent-contrast: #ffffff;
  --border: #e5e7eb; --destructive: #b91c1c;   /* swap with TOKENS values */
  --radius: 6px; --radius-lg: 12px;
  --font-display: "Fraunces", serif; --font-body: "Inter", sans-serif;
  --font-mono: "JetBrains Mono", monospace;
  --space-1: 4px; --space-2: 8px; --space-3: 12px; --space-4: 16px;
  --space-5: 24px; --space-6: 32px; --space-7: 48px; --space-8: 64px;
  --space-9: 96px; --space-10: 128px;
  --container: 1200px; --shadow-1: 0 1px 3px rgba(23,21,15,0.08);
  --shadow-2: 0 8px 28px rgba(23,21,15,0.14);
}
@media (prefers-color-scheme: dark) { :root { /* duplicate ALL color tokens,
  re-derived per system/SKILL.md dark rules */ } }
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text);
  font-family: var(--font-body); font-size: 16px; }
.shell { max-width: var(--container); margin-inline: auto;
  padding-inline: var(--space-4); }
.card { background: var(--surface); border: 1px solid var(--border);
  border-radius: var(--radius-lg); box-shadow: var(--shadow-1); }
.btn { background: var(--accent); color: var(--accent-contrast);
  border-radius: var(--radius); }
```

Rules:

1. One `:root` block in one CSS file. Dark mode is one extra block that
   duplicates every color token, re-derived per [system](system.md).
2. No raw hex outside that file. Sections consume `var(--token)` only.
3. Section shell is `.shell`; reading columns use `max-width: 68ch`.
4. Verify every foreground/background pair with `scripts/contrast.py`
   before shipping tokens.

## 2. Tailwind v4 (`@tailwindcss/postcss`)

Install: `npm install tailwindcss @tailwindcss/postcss`. Configure in CSS only.

```css
/* styles.css */
@import "tailwindcss";
@theme {
  --color-bg: #fafafa; --color-surface: #ffffff; --color-text: #1f2937;
  --color-text-muted: #6b7280; --color-accent: #1d4ed8;
  --color-accent-contrast: #ffffff; --color-border: #e5e7eb;
  --color-destructive: #b91c1c;
  --radius-md: 6px; --radius-lg: 12px;
  --font-display: "Fraunces", serif; --font-body: "Inter", sans-serif;
  --font-mono: "JetBrains Mono", monospace;
  --spacing: 4px;            /* 1 unit = canonical space unit */
  --shadow-1: 0 1px 3px rgba(23,21,15,0.08);
  --shadow-2: 0 8px 28px rgba(23,21,15,0.14);
}
```

Each `--color-NAME` entry generates `bg-NAME`, `text-NAME`, `border-NAME`
utilities; `--font-*` makes `font-*`; `--shadow-*` makes `shadow-*`;
`--radius-*` makes `rounded-*`. Dark mode: duplicate color tokens under
`@media (prefers-color-scheme: dark) :root { ... }` in plain CSS below
`@theme`.

Spacing mapping (canonical token -> Tailwind step):

| token | px | utility |
|---|---|---|
| `--space-1` | 4 | `p-1` `m-1` |
| `--space-2` | 8 | `p-2` `m-2` |
| `--space-3` | 12 | `p-3` `m-3` |
| `--space-4` | 16 | `p-4` `m-4` |
| `--space-5` | 24 | `p-6` `m-6` |
| `--space-6` | 32 | `p-8` `m-8` |
| `--space-7` | 48 | `p-12` `m-12` |
| `--space-8` | 64 | `p-16` `m-16` |
| `--space-9` | 96 | `p-24` `m-24` |
| `--space-10` | 128 | `p-32` `m-32` |

Rules:

1. No `tailwind.config.js`, no `tailwind.config.ts`. v4 is configured in CSS.
2. Every color used is a `--color-*` entry made in `@theme` exactly once.
   Arbitrary color values (`bg-[#3b82f6]`) are banned: map the token instead.
3. `.shell` is a plain class for `--container` (Tailwind has no matching
   primitive): `max-width: var(--container); margin-inline: auto;
   padding-inline: var(--space-4);`
4. Eyebrow labels: `text-xs uppercase tracking-[0.08em] text-text-muted`.

## 3. React / Next.js (App Router)

Tokens live in one CSS file imported exactly once in the root layout.
Components are Server Components by default.

```tsx
// app/layout.tsx
import "./tokens.css";          // the :root blocks from recipe 1
export default function RootLayout({ children }) {
  return <html lang="en"><body>{children}</body></html>;
}
```

```tsx
// app/components/SectionHeading.tsx  (server, static)
export function SectionHeading({ eyebrow, title }) {
  return (
    <div className="shell">
      <p className="eyebrow">{eyebrow}</p>
      <h2 style={{ fontFamily: "var(--font-display)" }}>{title}</h2>
    </div>
  );
}
```

```tsx
// app/components/RevealOnScroll.tsx  (client, interactive leaf)
"use client";
import { motion, useScroll, useTransform } from "motion/react";

export function RevealOnScroll({ children }) {
  const { scrollYProgress } = useScroll();
  const y = useTransform(scrollYProgress, [0, 1], [0, -40]);
  return <motion.div style={{ y }}>{children}</motion.div>;
}
```

Rules:

1. `"use client"` on interactive leaves only: forms, carousels, motion
   components. Never on a route, page, or layout that could render on the
   server. RSC SAFETY: global state works ONLY in Client Components; wrap
   providers in a `"use client"` boundary.
2. INTERACTIVITY ISOLATION: any component using Motion, scroll listeners, or
   pointer physics MUST be an isolated leaf with `'use client'` at top. Server
   Components render static layouts only.
3. SSR safety: a top-level `window` or `document` reference crashes SSR. Put
   browser-only code inside `"use client"` components, behind `useEffect` or a
   mounted check.
4. State: local `useState`/`useReducer` for isolated UI. Global state ONLY for
   deep prop-drilling avoidance (Zustand, Jotai, or context). NEVER use
   `useState` for continuous values driven by user input (mouse position,
   scroll progress, pointer physics). Use Motion `useMotionValue`/`useTransform`/`useScroll`.
5. Fonts: always use `next/font` or self-host with `@font-face` + `font-display: swap`. Never link Google Fonts via `<link>` in production. Verify `package.json` before importing any third-party library; if missing, output install command first.
6. Icons: allowed libraries in priority order: `@phosphor-icons/react`, `hugeicons-react`, `@radix-ui/react-icons`, `@tabler/icons-react`. Discouraged: `lucide-react`. Never hand-roll SVG icons. One family per project; standardize `strokeWidth` globally (1.5 or 2.0). NEVER use emoji as icons; replace with icon glyphs. Allow emoji only when brief explicitly asks playful/chat vibe.
7. Responsiveness: standardize breakpoints `640/768/1024/1280/1536`; contain layouts with `max-w-[1400px] mx-auto` or `max-w-7xl`. VIEWPORT STABILITY: never use `h-screen` for hero; always `min-h-[100dvh]` (prevents iOS Safari jump). Layout: never use flexbox percentage math (`w-[calc(33%-1rem)]`); always CSS Grid (`grid grid-cols-1 md:grid-cols-3 gap-6`).
8. Motion: `motion/react` inside client components only. The scroll skeletons
   in `references/motion/references/gsap-skeletons.md` (vanilla ES modules) port
   directly; GSAP ScrollTrigger uses `useGSAP` or `useEffect` + register.
9. Screenshot-safe reveals: nothing waits at `opacity: 0` for an
   IntersectionObserver. Text builds fire on mount; scroll-linked effects
   animate transform only. See [motion](motion.md) for the full rule set.

## 4. Vue 3

Tokens stay in one global CSS file; SFC scoped styles consume them.

```vue
<!-- src/App.vue, once: import "./styles.css" -->
<script setup>
defineProps({ eyebrow: String, title: String });
</script>

<template>
  <header class="shell">
    <p class="eyebrow">{{ eyebrow }}</p>
    <h2 class="display">{{ title }}</h2>
  </header>
</template>

<style scoped>
.eyebrow { text-transform: uppercase; letter-spacing: 0.08em;
  font-size: 0.75rem; color: var(--text-muted); }
.display { font-family: var(--font-display); color: var(--text); }
</style>
```

Rules:

1. Never redeclare `:root` tokens inside a component. Scoped styles only read
   `var(--token)`.
2. Reusable behavior (scroll reveal, counter, carousel state) goes in a
   composable under `src/composables/`, not inline in templates.
3. Vue + Tailwind: compose recipe 2 as well; the stack note stays three
   lines: `@tailwindcss/vite` plugin, `@theme` tokens, no config file.

## 5. shadcn/ui (on top of React)

shadcn components read their own variables. Alias them onto the canonical
tokens once in `app/globals.css`, and the whole library picks up the design.

```css
:root {
  --background: var(--bg); --foreground: var(--text);
  --card: var(--surface); --card-foreground: var(--text);
  --primary: var(--accent); --primary-foreground: var(--accent-contrast);
  --border: var(--border); --destructive: var(--destructive);
  --muted-foreground: var(--text-muted); --radius: 6px;
}
```

Rules:

1. Canonical tokens are the source; shadcn variables are derived aliases, the
   mapping exists exactly once.
2. Never restyle a shadcn component with inline hex or one-off classes; change
   the canonical token instead.
3. shadcn assumes React: this adapter always rides on section 3.

## Native stack alternatives (non-React)

When the detected stack is not React/Next, do not force React patterns. Use the HTML+CSS, Vue, or Tailwind adapter above. For full 22-stack guidance (svelte, astro, nuxt-ui, angular, laravel, swiftui, react-native, flutter, jetpack-compose, threejs, etc.), query `ui-ux-pro-max` (`--stack <name> --domain ux`) before inventing.

Charts are explicitly out of scope for this skill. For data visualization (25 chart types), query `ui-ux-pro-max` via `"<data shape>" --domain chart` rather than improvising grid + bars.

## Motion cheatsheet across stacks

Skeletons and timing rules live in [motion](motion.md) and
`references/motion/references/gsap-skeletons.md`.

- HTML/CSS: vanilla ES modules from the skeletons; no IntersectionObserver
  opacity-0 patterns anywhere.
- React: `motion/react` or GSAP in client components; Lenis + GSAP bridge when
  MOTION is 8-10.
- Vue: same skeletons inside composables, `onMounted` only.

## Pass/fail checks

1. `grep -rn "tailwind.config" .` -> 0 matches outside lockfiles. v4 has no
   JS config.
2. `grep -rEn '#[0-9a-fA-F]{6}' app/ src/ --exclude='*tokens*' --exclude='*.css'`
   -> 0 raw hex in components or sections.
3. `"use client"` appears only in interactive leaf components, never in a
   layout, page, or static section.
4. Every `--space-N` used maps to a step in the spacing table of the active
   stack.
5. `scripts/contrast.py --fg '<text>' --bg '<bg>'` PASS for every pair from
   the `TOKENS` artifact before any component code is written.
