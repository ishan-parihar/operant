# Performance

Hit Core Web Vitals on every page: LCP under 2.5s, INP under 200ms, CLS under 0.1. Covers image optimization (WebP/AVIF, next/image, reserved space), font loading (next/font, no raw @font-face), animation cost (grain on pseudo-elements only, lazy-loaded Motion/GSAP), main-thread budget, debounce/throttle, and a Lighthouse gate. Run after components exist and before delivery, or whenever a page feels slow, jumps on load, or drops Lighthouse score.

Performance is a quality gate, not a polish pass. A page that scores red on
Core Web Vitals is unfinished, no matter how it looks in a design review. This
leaf is the audit and fix loop: run it after components exist and before
delivery, and re-run it whenever a new hero asset, font, animation, or
third-party script lands.

The three gates, always:

| Metric | What it measures | Pass threshold | Failing feel |
|---|---|---|---|
| LCP (Largest Contentful Paint) | time until the hero/image/headline paints | under 2.5s on 4G | blank or half-painted hero |
| INP (Interaction to Next Paint) | delay between tap/click and visible response | under 200ms | taps feel dead, janky dropdowns |
| CLS (Cumulative Layout Shift) | how much the layout jumps while loading | under 0.1 | text shoves down as images/fonts pop in |

All three must pass on the real production build, measured on mobile. Desktop
simulation is a smoke test, not the gate.

## Procedure

1. Build the production bundle. Local dev builds lie about everything on this
   page: dev servers compile on demand and skip minification, so numbers from
   `next dev` or `vite dev` are meaningless. Use `next build && next start`,
   or the equivalent static export.
2. Run Lighthouse against the built page on mobile emulation with 4G
   throttling. Record LCP, INP (via TBT proxy in lab), CLS, total JS weight,
   and image weight.
3. Fix image cost first (Lane 1), fonts second (Lane 2), animations third
   (Lane 3), main-thread and scripts fourth (Lane 4). That order matches the
   size of typical wins on a marketing page.
4. Re-run Lighthouse after each lane. Stop when all three gates pass. Do not
   tune what already passes.
5. Run `## Checks`. Fix fails before delivery.

## Lane 1: Images

Images are the top LCP and CLS offender on almost every page this pipeline
ships. Fix order: format, sizing, loading.

**Format: WebP or AVIF, never raw PNG/JPEG for photos.** AVIF compresses best
but encodes slowly; WebP is the safe default. SVG only for icons, logos, and
flat illustrations, never exported raster-in-vector wrappers.

| Slot | Rule |
|---|---|
| Hero image / LCP candidate | `next/image` (or framework equivalent) with `priority` (Next) or `fetchpriority="high"` + `loading="eager"`. Preloaded automatically. Never `loading="lazy"` on the hero. |
| Below-the-fold images | `loading="lazy"` + `decoding="async"`. Frameworks do this by default for `next/image` below the fold; bare `<img>` tags need both attributes by hand. |
| Card avatars, thumbnails, icons | Sized to their render box, not the source file. A 96px avatar should not ship a 1024px file. |
| Responsive images | `sizes` attribute that matches the real layout, so the browser picks the right `srcset` candidate instead of the largest one. |

**Reserve space for every image.** Every `next/image` must have an explicit
`width`/`height` (or `fill` inside a container with a fixed aspect ratio). Raw
`<img>` needs `width` + `height` attributes or a CSS `aspect-ratio`. This is
the CLS gate: if the browser does not know the image's box before it loads,
the layout shifts when it lands, and CLS fails.

**Do not put text inside hero images.** It hides from screen readers, blurs
under compression, and cannot be selected or translated. Overlay real text on
top of the image instead.

**Stock/photo weight budget:** hero image under 150KB compressed if you can
get there, below-the-fold images under 100KB each. Anything larger needs a
reason in the commit message.

## Lane 2: Fonts

Fonts cause both invisible-text flashes and layout shifts. They also ship
weight silently: one unoptimized display font can weigh more than the entire
CSS bundle.

**Use `next/font` (or the framework's equivalent: `fontsource`,
`@nuxt/fonts`). Never write your own `@font-face` blocks.** Next/font
self-hosts the font, generates `font-display: swap`, adds preload hints, and
emits a size-adjusted fallback so the fallback font matches the real font's
metrics. Hand-rolled `@font-face` almost always misses one of the four.

| Rule | Why |
|---|---|
| 2 font families max on the page (display + body). A third (mono for code) only if it is actually rendered. | each family multiplies download, parsing, and layout-shift risk |
| Subset to the scripts you ship (`latin` usually, not `latin` + `cyrillic` + `greek` by default) | drops font payload by 50-80% |
| `font-display: swap` (or `optional` for non-critical fonts), never `block` | swap shows fallback immediately; block hides text (FOIT) |
| Preload only the font file used above the fold, in the weights you actually render | preloading everything delays the hero image, net loss |
| Reserve space: fallback font must be metrically close to the real one (next/font does this with `adjustFontFallback`) | a mismatched fallback causes a CLS spike when the real font swaps in |

If the design brief insists on a heavy variable font, load only the axis
ranges the design actually uses (`wght 400..700`, not the full 100..900
range), and ship `font-variation-settings` through one variable file instead
of five static weights.

## Lane 3: Animation cost

Animation is the most common INP offender in this pipeline's output because it
runs on the main thread and fights the user's input for frames.

**Grain and noise textures: CSS only, fixed position, pseudo-element.** Never
a container-level stack, never a canvas, never JS. The pattern:

```css
/* one element, fixed inset-0, pointer-events-none, z-60 or the top kinetic layer */
body::after {
  content: "";
  position: fixed;
  inset: -50%; /* oversize so translate does not reveal edges */
  z-index: 60;
  pointer-events: none;
  background-image: url("/noise.png"); /* small tile, under 10KB */
  opacity: 0.05;
  animation: grain 8s steps(10) infinite;
}
@keyframes grain {
  0%, 100% { transform: translate(0, 0); }
  10% { transform: translate(-5%, -10%); }
  30% { transform: translate(3%, -15%); }
  50% { transform: translate(12%, 9%); }
  70% { transform: translate(9%, 4%); }
  90% { transform: translate(-1%, 7%); }
}
```

Why this is the only legal pattern: one fixed pseudo-element composites to a
single GPU layer, transforms are free (no layout, no paint), and
`pointer-events: none` keeps it from intercepting clicks or hover. A grain
overlay applied per-section re-rasterizes that whole section every frame and
tanks scroll FPS.

**Lazy-load heavy motion libraries.** GSAP, Three.js, Framer-Motion-style
runtime animation, and Lottie players all ship large JS. Rules:

| Library | Rule |
|---|---|
| GSAP | dynamic `import("gsap")` inside the component that needs it, after first paint. Never a top-level import on a page that uses it for one scroll reveal. |
| Three.js / React Three Fiber | behind `next/dynamic` with `ssr: false`, loaded only when the 3D block enters the viewport (IntersectionObserver on a placeholder). |
| Framer Motion / Motion | fine top-level for small layouts; code-split the `AnimatePresence` orchestration blocks if they exceed ~30KB gzip. |
| Lottie | lazy-load the JSON + player, render only when visible, pause when off-screen. |

**Animate `transform` and `opacity` only.** Animating `width`, `height`,
`top`, `left`, `margin`, `padding`, or `box-shadow` forces layout or paint on
every frame. On a mid-range phone this drops you below 30fps and spikes INP.
Exceptions (rare): `clip-path` on small elements, `filter` on elements that
are promoted to their own layer.

**`prefers-reduced-motion` is mandatory.** Every non-trivial animation must
respect it. The leaf at [motion](../motion.md) defines the kill-switch pattern;
this leaf only enforces its existence and its cost.

## Lane 4: Main thread, scripts, and interactivity

The main thread is the resource INP actually measures. Every millisecond of
work you push onto it delays the response to the user's next tap.

**Per-frame budget: 16ms for 60fps.** Any synchronous work longer than ~50ms
is a "long task" and degrades INP. Breaks: `setTimeout`/`scheduler.postTask`
chucking, `requestIdleCallback` for low-priority work, Web Workers for true
compute (parsing, search, transforms).

**Debounce or throttle every high-frequency handler.** Scroll, resize, input,
mousemove, and drag handlers must not run unthrottled. Defaults: `scroll` ->
`requestAnimationFrame`-throttled or 100ms throttle, `resize` -> 150ms
debounce, `input` -> 200-300ms debounce for network calls, `mousemove` ->
rAF-throttled or pointer-events + transform-based tracking.

**Virtualize lists at 50+ items.** A long list of cards/feature rows/logos
re-renders everything on every scroll frame and inflates the DOM, which also
hurts CLS measurement (larger layout tree, slower style recalc). Use
`@tanstack/react-virtual` or the framework's equivalent. Under 50 items, plain
rendering is fine and a virtualization layer is over-engineering.

**Third-party scripts: `async` or `defer` by default, audit before adding.**
Analytics, chat widgets, A/B testers, cookie banners, and social embeds all
ship JS that runs on the main thread and steals INP budget. For each one ask:
is this load-bearing for the page's primary job? If no, defer it, lazy-load it
on interaction, or drop it. A chat widget should not boot until the user
clicks its launcher.

**Hydration and code splitting.** In Next.js/React Server Components: mark as
`"use client"` only the leaves that need interactivity. Route-level splitting
is the default; component-level `dynamic()` splits what ships per route. The
bundle shipped to the first paint decides LCP and INP on that page; anything
below the fold should arrive after it.

**Critical CSS and render-blocking resources.** Inline the CSS needed for the
first viewport (Tailwind already tree-shakes; the risk is hand-written global
CSS). Defer non-critical stylesheets. Avoid `@import` inside CSS files; it
serializes fetches and delays first paint.

## Lane 5: Measurement loop

Performance is verified by measurement, not by intent. The loop:

1. Build for production.
2. Run Lighthouse (CLI or DevTools) against the running build, mobile
   emulation, 4G throttle.
3. Record the three gates plus total JS transfer (KB gzip) and total image
   transfer (KB).
4. Fix the lowest-scoring lane first; re-measure after each change.
5. Repeat until all three gates pass and the page's JS+image weight stops
   dropping.

Budgets to sanity-check on every run:

| Budget | Healthy | Concerning |
|---|---|---|
| JS shipped to first paint (gzip) | under 150KB | over 300KB |
| Total page JS (gzip) | under 300KB | over 600KB |
| Hero image (compressed) | under 150KB | over 300KB |
| Number of webfonts | 2-3 files | 5+ files |
| Long tasks (>50ms) on load | 0-2 | 5+ |
| Lighthouse Performance score | 90+ | under 75 |

## Worked example

Page: `/pricing` on a Next.js App Router marketing site. Lighthouse mobile:
LCP 4.1s, INP 480ms, CLS 0.28, Perf score 62.

1. Images: hero is a 1.2MB PNG, `<img>` with no width/height. Convert to
   `next/image`, AVIF output, add `priority`, width/height set, quiet win.
   LCP drops to 2.6s, CLS drops to 0.05.
2. Fonts: hand-rolled `@font-face` for two display weights + body, no
   preloads, fallback is default sans with mismatched metrics. Swap to
   `next/font/google` with `latin` subset, `display: swap`, preload only the
   weight used in the hero. LCP drops to 2.3s, CLS to 0.02.
3. Animations: a 400KB Lottie animation autoplays above the fold on mount,
   blocking hydration. Lazy-load it below the fold only, replace the hero
   with a static SVG used as a poster while the anim loads off-screen. INP
   drops to 260ms.
4. Scripts: an Intercom widget + a Hotjar script both load synchronously in
   `<head>`. Gate both behind a `useEffect` with a 2s delay and an
   IntersectionObserver on the footer. INP drops to 180ms.
5. Re-run: LCP 2.3s, INP 180ms, CLS 0.02, Perf score 94. All three gates
   pass. Stop.

## Lane 6: Zero-cost defaults (decide in seconds)

Reach for these before any tuning work. They are the fastest legal option and
count as passing their lane on their own.

| Choice | Default | Only deviate when |
|---|---|---|
| System UI font stack (`ui-sans-serif, system-ui, sans-serif`) for body text | zero font bytes, zero CLS | the brief demands a branded body face |
| `next/font/google` over self-hosting by hand | one line, all four wins (preload, swap, fallback metrics, subset) | air-gapped / offline deploy |
| Static SVG/CSS illustration over animated canvas | no JS, no main-thread cost | animation is the product (portfolio piece) |
| `<details>/<summary>` or CSS `:target`/`:checked` accordions | no JS | controlled state needed (analytics, multi-open) |
| Native `loading="lazy"` over IntersectionObserver wrappers | no JS | framework already does it (next/image) |

If a zero-cost default solves the problem, stop there. Do not earn complexity
you do not need.

## Defer / out of scope

- Route-level caching, CDN headers, ISR/SSR strategy: belongs to the deploy
  leaf ([generic](../deploy/generic.md)) and the production gate
  ([production](production.md)).
- Absolute performance-vs-visual bans (like "no carousels") live in
  [anti-slop](anti-slop.md). This leaf ships the fix; anti-slop owns the
  ban list.
- Accessibility-side motion rules (vestibular safety): see
  [accessibility](accessibility.md). This leaf enforces cost, that leaf
  enforces harm.

## Checks

1. **LCP gate (measurable):** Lighthouse mobile run on the production build
   shows LCP under 2.5s. Save the report; a screenshot of an in-browser
   Lighthouse run counts.
2. **INP gate (measurable):** lab TBT under 200ms proxy OR field INP under
   200ms from Web-Vitals reporting. TBT over 200ms in lab is a fail signal
   even if the page "feels fine" on a fast dev machine.
3. **CLS gate (measurable):** Lighthouse mobile CLS under 0.1. Any layout
   shift trace pointing at images or fonts is a Lane 1/2 failure regardless
   of the final number.
4. **Image contract (greppable):** every raster image on the page renders
   through `next/image` (or framework equivalent) OR declares `width` +
   `height` + `loading="lazy"`/`fetchpriority`. Grep for `<img` in shipped
   components; any hit without all three attributes fails.
5. **Font contract (greppable):** no hand-written `@font-face` blocks in app
   code. Grep for `@font-face`; hits outside `next/font`-generated CSS or a
   documented one-off (self-hosted icon font) fail.
6. **Grain contract (greppable):** grain/noise overlays exist only on
   `position: fixed` + `pointer-events: none` pseudo-elements. Grep for
   `noise.png` / `grain` class usage; any usage on a layout container fails.
7. **Main-thread contract (greppable + measurable):** scroll/resize/input
   handlers are wrapped in debounce/throttle/rAF (grep each handler), and the
   production build reports 0-2 long tasks during initial load in the
   Lighthouse trace.
