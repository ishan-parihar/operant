# Motion: Animation Specs per Section

Translate the SECTION MAP and MOTION dial into per-section animation specs. Use when adding motion, hover states, scroll reveals, sticky-stacks, or GSAP timelines to any website design.

Motion serves comprehension, not decoration. Every transition must communicate hierarchy, storytelling, state change, or tactile feedback. Aim for at most one memorable motion moment per page; keep the rest of the interface quiet, stable, and instant.

## Procedure

1. Read the `DESIGN BRIEF` (extract the `MOTION` dial, 1-10) and the `SECTION MAP`.
2. Determine the motion band from Section 1:
   - Band 1-3: Static and instant.
   - Band 4-7: CSS transitions, subtle hover, native reveals.
   - Band 8-10: GSAP ScrollTrigger, pinned sections, orchestrated timelines.
3. Audit the sections for the single memorable motion moment. If two sections compete for attention, demote one to subtle transitions.
4. Check marquee usage: allow at most one marquee across the entire page.
5. Apply the GPU-only constraint: only `transform` and `opacity` may be animated.
6. Verify the reduced-motion fallback plan for every animated element.
7. For GSAP patterns (Band 8-10), select from the canonical skeletons in `references/motion/references/gsap-skeletons.md`. Do not inline custom scroll engines.
8. Append the `MOTION SPEC` block to the design output and run `## Checks`.

## 1. Motion Bands (gated by MOTION dial)

| Band | Dial | Strategy | Typical stack | Scroll behavior |
|---|---|---|---|---|
| Static | 1-3 | Zero scroll motion. Instant state changes. | Pure CSS | Native browser scroll. No reveals. |
| Transitions | 4-7 | Subtle viewport reveals, micro-interactions, spring hovers. | CSS / Motion `whileInView` | Single-fire fade and rise on scroll. |
| Cinematic | 8-10 | Pinned storytelling, sticky stacks, scrubbed timelines. | GSAP + ScrollTrigger | Scrubbed transforms, pinned containers. |

### Band 1-3: Static (Trust-first, editorial, regulatory, low-power)

- No scroll-triggered animations of any kind. Elements render at final resting position.
- Interactive states use fast color and background transitions (`duration <= 150ms`, `ease-out`).
- Hover lift is banned or capped at `1px` via `transform: translateY(-1px)`.
- No marquees, no infinite loops, no parallax, no pinned viewports.
- If the brief asks for high motion but dial is set to 1-3, dial wins. Ship clean static markup.

### Band 4-7: CSS Transitions and Motion Primitives (Standard modern web)

- Entry reveals fire once when elements cross 15-20% into the viewport.
- Maximum entrance travel: `16px` to `24px` on the Y-axis. Never fly in from offscreen.
- Timing: duration `300ms` to `500ms`, easing `cubic-bezier(0.16, 1, 0.3, 1)` or spring (`stiffness: 100, damping: 20`).
- Micro-interactions acknowledge clicks with tactile compression (`scale(0.98)` or `translateY(1px)` on active).
- Staggered entrances: step delay `40ms` to `60ms` per item, total duration capped at `600ms`.
- Prefer native CSS `animation-timeline: view()` or Motion's lightweight `whileInView`. Avoid importing heavy libraries for basic reveals.

### Band 8-10: GSAP ScrollTrigger and Orchestrated Motion (Campaign, brand, agency)

- Reserved for explicit narrative moments: product breakdowns, sticky card stacks, horizontal pans.
- Requires `@gsap/react` (`useGSAP`) or strict lifecycle cleanup in vanilla JS.
- Pinned sections MUST start at `start: "top top"`. Never start pinning mid-viewport.
- Scrubbed timelines must use gentle dampening (`scrub: 1` or `scrub: 0.5`).
- Never pin more than two sections on the same page. Pinned scroll exhausts user attention.
- Load concrete implementations from `references/motion/references/gsap-skeletons.md`.

## Inputs and outputs

**Inputs:**

- `DESIGN BRIEF` block from [briefing](briefing.md) (source of the MOTION dial, 1-10).
- `SECTION MAP` from [structure](structure.md) (per-section name, layout family, content slots).
- Token contract from [system](system.md) (`--duration-fast`, `--duration-med`, z-index scale, spacing).

**Output (append to the working design document):**

```markdown
MOTION SPEC
dial: <1-10>
band: static | transitions | cinematic
memorable_moment: <section name> | none
marquee_count: <0 or 1>
reduced_motion: fallback strategy per animated section
per_section:
  <section name>: <trigger, properties, duration, easing, cleanup>
```

The spec is a contract: implementation either matches it or the spec is revised and the reason recorded. Never let the built animations drift from the spec silently.

## Downstream dependencies

- [accessibility](quality/accessibility.md) re-verifies reduced-motion coverage and focus behavior during the quality gate.
- [performance](quality/performance.md) audits rAF, listener, and GPU memory budgets after build.
- [anti-slop](quality/anti-slop.md) bans decorative animation tells (default fade-up on every card, floating blobs) and checks for them mechanically.

## 2. Restraint: The One Memorable Moment

Generic AI sites animate every card, heading, button, and image equally. Distinctive design selects one focal experience and keeps the surrounding sections disciplined.

### Valid motives for animation
- **Hierarchy:** Guiding user focus to the primary value proposition or CTA.
- **Storytelling:** Revealing hardware layers, product architecture, or chronological steps in sequence.
- **Feedback:** Acknowledging user input immediately (button press, form submit, toggle).
- **State transition:** Explaining where an element went when opened, collapsed, or deleted.

### Invalid motives (Banned)
- "It looked cool."
- "Filling empty whitespace."
- "Adding visual interest to weak copy."
- "Showing what GSAP can do."

If the purpose cannot be articulated in a single sentence describing user benefit, drop the animation.

## 3. Marquee Rules (Hard limits)

A horizontal scrolling strip (logos, phrases, ticker) is a high-cost visual device.

- **Maximum one marquee per page.** Two marquees on the same page is an immediate failure.
- **Must pause on hover and focus:** Include `@media (hover: hover)` pause and `:focus-within` pause.
- **Must stop under reduced motion:** Under `prefers-reduced-motion: reduce`, the marquee collapses to a wrapped, static flex row or an accessible scroll container.
- **Content density:** Do not use marquees for critical reading material. Logo walls and ambient badges only.

```css
.marquee-track {
  display: flex;
  gap: var(--space-6);
  animation: marquee-scroll 24s linear infinite;
}
.marquee-container:hover .marquee-track,
.marquee-container:focus-within .marquee-track {
  animation-play-state: paused;
}
@media (prefers-reduced-motion: reduce) {
  .marquee-track {
    animation: none;
    flex-wrap: wrap;
    justify-content: center;
  }
}
```

## 4. Forbidden Animation Patterns (Absolute Bans)

These patterns cause frame drops, memory leaks, and broken touch gestures. They are banned across all stacks.

1. **`window.addEventListener("scroll", ...)`:**
   Attaching raw scroll listeners triggers layout thrashing and bypasses compositor batching. Use `IntersectionObserver`, CSS scroll-driven animations, or `ScrollTrigger`.
2. **Scroll position stored in React state:**
   Writing `window.scrollY` or scroll percentages to `useState` causes the entire component tree to re-render on every frame. Use Motion values (`useScroll`, `useTransform`) or GSAP tweens.
3. **`requestAnimationFrame` loops touching UI state:**
   Never sync UI component re-renders to a rAF loop. Drive continuous transforms directly through DOM node refs or Motion values outside the render cycle.
4. **Animating geometry properties:**
   Never animate `top`, `left`, `right`, `bottom`, `width`, `height`, `margin`, or `padding`. Animate only `transform` and `opacity`.
5. **Continuous CPU filters on scrolling containers:**
   Never put `backdrop-filter: blur(...)` or SVG noise filters on long scrollable parents. Put noise overlays exclusively on a `fixed inset-0 pointer-events-none` layer.
6. **Perpetual unmotivated animations:**
   Do not put infinite pulsing, floating, or bouncing effects on plain informational cards. Keep cards stationary until interacted with.

## 5. Hardware Acceleration and Performance

To guarantee 60fps on mobile and low-power devices, animate only properties handled directly by the GPU compositor.

```
+-------------------------------------------------------------+
| SAFE (Compositor Thread):                                   |
| - transform: translate3d(), scale(), rotate(), skew()       |
| - opacity                                                   |
+-------------------------------------------------------------+
| BANNED (Layout & Paint Thrashing):                          |
| - top, left, right, bottom                                  |
| - width, height, min-width, max-height                      |
| - margin, padding, border-width                             |
+-------------------------------------------------------------+
```

### will-change usage
- Apply `will-change: transform` only to elements currently animating or about to animate on scroll.
- Remove `will-change` on completion or unmount to release GPU memory surfaces.
- Never place `will-change: all` anywhere in any stylesheet.

## 6. Z-Index Restraint

Arbitrary `z-index: 9999` or scattered `z-10`, `z-20`, `z-50` classes produce stacking context collisions with pinned headers and modals. Follow the standard systemic scale:

| Level | Value | Usage |
|---|---|---|
| Deep | `-1` | Canvas backdrops, subtle gradients |
| Base | `0` | Normal flow content, default section body |
| Card | `1` | Lifted cards, floating tiles |
| Pinned | `10` | Pinned panels in sticky-stack setups |
| Sticky header | `40` | Main site navigation bar |
| Dropdowns | `50` | Select menus, popovers |
| Dialog / Modal | `100` | Fullscreen overlays, mobile drawers |
| Toast / Alert | `120` | Transient system notifications |

Keep all animated element stacking within levels 0 through 10. Do not elevate animated items above sticky headers (`level 40`).

## 7. Reduced-Motion Collapse Protocol

Support for `prefers-reduced-motion` is mandatory across all projects. Any site where motion cannot be disabled is considered broken.

### Three-tiered fallback rule
1. **Entrance reveals:** Transition duration drops to `0s` or opacity snaps to `1` with zero translation offset.
2. **Pinned scroll and pans:** Sticky-stack panels collapse into regular vertically stacked sections in natural document flow. Horizontal pans convert to native touch-scroll containers (`overflow-x: auto`).
3. **Continuous loops:** Marquees and video loops pause or collapse to static wrapped grids.

### CSS implementation
```css
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}
```

### JS and React implementation
```tsx
import { useReducedMotion } from "motion/react";

export function SectionReveal({ children }: { children: React.ReactNode }) {
  const shouldReduce = useReducedMotion();
  return (
    <div
      style={{
        opacity: 1,
        transform: shouldReduce ? "none" : undefined,
      }}
    >
      {children}
    </div>
  );
}
```

## 8. Canonical GSAP Skeletons (Overview)

When `MOTION >= 8`, use the pre-tested templates in `references/motion/references/gsap-skeletons.md`:

1. **Sticky-Stack (`stickyStack`):**
   - Cards pin to the top of the viewport (`start: "top top"`, `pin: true`).
   - The subsequent card rises over the previous card, which scales down slightly (`scale: 0.92`, `opacity: 0.55`).
   - Pinned element cleanup is mandatory on component unmount.
2. **Horizontal-Pan (`horizontalPan`):**
   - Section pins at the top (`start: "top top"`).
   - Outer container stays pinned while the inner track slides horizontally via `xPercent` or negative scrollWidth.
   - Travel distance matches `scrollWidth - window.innerWidth`.
3. **Reveal-Stagger (`revealStagger`):**
   - Grid or card list reveals with small Y offset (`16-24px`) and sequential delay.
   - Fired once on viewport entry (`once: true`), never scrubbed back and forth.

Full code, CSS, and React adapters are documented in `references/motion/references/gsap-skeletons.md`.

## 9. Worked Example: Motion Spec

Below is an example of an audit pass translating a `SECTION MAP` into a production motion plan.

### Input
- `DESIGN BRIEF`: B2B SaaS landing page, `MOTION: 6`.
- `SECTION MAP`: Header, Hero (Asymmetric Split), Trust Wall, Bento Features, Testimonials, CTA, Footer.

### Produced MOTION SPEC
```markdown
### Motion Spec (MOTION: 6)

- Hero: One-time entrance reveal. Headline and visual fade and rise (y: 16px, duration: 400ms, ease: cubic-bezier(0.16, 1, 0.3, 1)). Subtext follows with 60ms delay. Reduced motion: immediate opacity 1, y: 0.
- Trust Wall: Static logo row. Dial is 6, so marquee is omitted to keep the page calm. Logos sit at opacity 0.7, hover to 1.0 (duration: 150ms).
- Bento Features: Entrance stagger using whileInView. 4 cards reveal sequentially (delay: 50ms per card, y: 20px, duration: 450ms). Hover card lift: translateY(-2px), shadow expansion. Reduced motion: static grid.
- Testimonials: One memorable moment. Quote cards use subtle spring hover physics on active pagination.
- CTA Section: Static presentation with tactile button feedback (active scale: 0.98).
- Stack: CSS custom properties + Motion whileInView. No GSAP needed (dial < 8).
```

## Timing and easing dictionary

Use these exact values. Do not tune ad hoc.

| Purpose | Duration | Easing | Notes |
|---|---|---|---|
| Hover color/background | 150-200ms | `ease-out` or `power1.out` | No travel, color only |
| Hover lift | 200-300ms | `power2.out` | max 2-4px |
| Press / tactile | 100-150ms | `ease-out` | `scale(0.98)` or `translateY(1px)` |
| Entrance reveal | 300-500ms | `cubic-bezier(0.16, 1, 0.3, 1)` | `y: 16-24px`, opacity 0 to 1 |
| Stagger step | 40-80ms per item | same as reveal | total cap 600ms |
| Marquee loop | 20-30s linear | `linear` | pause on hover, collapse on reduced motion |
| Scrubbed pin | scroll-linked | `none` / linear | `scrub: 0.5-1`, never springy |

Durations over 600ms for non-scrubbed UI motion are banned. If an element needs more than 600ms to feel right, the design is compensating, and the fix is better layout or copy, not slower motion.

## Checks

Run these 7 mechanical checks before delivering any motion spec or implementation:

1. **MOTION dial band match:** Dial 1-3 uses only static CSS. Dial 4-7 uses single-fire CSS reveals or Motion whileInView. Dial 8-10 is the only band permitted to use GSAP ScrollTrigger pinning.
2. **Zero forbidden scroll listeners:** Grep shows zero instances of `addEventListener("scroll"`, zero `window.scrollY` in React `useState`, and zero `requestAnimationFrame` updating component state.
3. **GPU-only properties:** All animations and transitions target only `transform` and `opacity`. Zero animations on `top`, `left`, `width`, `height`, `margin`, or `padding`.
4. **Reduced-motion coverage:** Every animation has an explicit fallback via `@media (prefers-reduced-motion: reduce)` or `useReducedMotion()`. Fallbacks render the element in its final readable state.
5. **Marquee count check:** The entire page contains at most 1 marquee component. If count is 0 or 1, pass. If count > 1, fail.
6. **GSAP pinning discipline:** When GSAP ScrollTrigger is used, `start` is set to `"top top"`, `pin: true` is configured, and cleanup (`revert()` or `kill()`) is wired to component unmount.
7. **Z-Index budget respected:** All animated elements use z-index values between 0 and 10. Sticky navigation sits at 40; zero animated cards overlap or exceed level 40.
