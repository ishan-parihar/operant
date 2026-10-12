# GSAP Skeletons

Reference only. Load this file when the `SECTION MAP` MOTION column is 8-10. All snippets are language-agnostic ES-module JS; React/Vue/Svelte adapters live in `references/stack-adapters.md`.

Three canonical skeletons. Copy the skeleton that matches the pattern name in the `SECTION MAP` MOTION column, then rename classes to match the section. Do not mix skeletons.

## Rules that apply to every skeleton

- One GSAP import per page. If a sibling section already imported GSAP, reuse that instance.
- Register `ScrollTrigger` once, immediately after import. Never re-register.
- Guard every skeleton with `prefers-reduced-motion`; when reduced, call the `fallback` function shown in each skeleton.
- Scope every trigger to the section element; never use `body` or `document` as a trigger.
- Kill tweens on cleanup (`useGSAP` in React, `onDestroy` in Svelte, `onUnmounted` in Vue, or `cleanup()` in vanilla).
- Use only `transform` and `opacity` in tweens. Never `top`, `left`, `width`, or `height`.
- Set `will-change` on animated elements only, and remove it after the tween finishes (via `onComplete`).

## 1. Sticky-stack

Use when the `SECTION MAP` lists a section as `sticky-stack`. Cards or panels stack on top of each other while the user scrolls; the next card slides over the previous.

```html
<section class="stack-section" data-motion="sticky-stack">
  <div class="stack-pin">
    <div class="stack-card" data-index="0">...</div>
    <div class="stack-card" data-index="1">...</div>
    <div class="stack-card" data-index="2">...</div>
  </div>
</section>
```

```css
.stack-section { position: relative; height: calc(var(--stack-count, 3) * 100vh); }
.stack-pin { position: sticky; top: 0; height: 100dvh; overflow: hidden; }
.stack-card { position: absolute; inset: 0; display: grid; place-items: center;
  background: var(--surface); border: 1px solid var(--border);
  border-radius: var(--radius-lg); box-shadow: var(--shadow-2); }
```

```js
import gsap from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
gsap.registerPlugin(ScrollTrigger);

export function stickyStack(section, reduced) {
  const cards = gsap.utils.toArray('.stack-card', section);
  const count = cards.length;
  if (reduced || count === 0) {
    cards.forEach((card) => gsap.set(card, { clearProps: 'all' }));
    return () => {};
  }

  cards.forEach((card, i) => {
    gsap.set(card, { zIndex: i + 1, yPercent: i * 100 });
  });

  const tl = gsap.timeline({
    scrollTrigger: {
      trigger: section,
      start: 'top top',
      end: `+=${(count - 1) * 100}%`,
      scrub: 1,
      pin: '.stack-pin',
      pinSpacing: false,
      anticipatePin: 1,
    },
  });

  cards.forEach((card, i) => {
    if (i === 0) return;
    tl.fromTo(card, { yPercent: 100 }, { yPercent: 0, ease: 'none', duration: 1 }, i - 1);
  });

  return () => tl.scrollTrigger.kill();
}
```

Hard limits:
- `count` max 5; beyond 5 the scroll tax outweighs the effect.
- `zIndex` only rises; never animate `z-index`.
- `yPercent` step is 100; smaller steps make cards peek and read as a bug.

## 2. Horizontal-pan

Use when the `SECTION MAP` lists a section as `horizontal-pan`. Content pans horizontally while the page scrolls vertically.

```html
<section class="pan-section" data-motion="horizontal-pan">
  <div class="pan-pin">
    <div class="pan-track">
      <div class="pan-item">...</div>
      <div class="pan-item">...</div>
      <div class="pan-item">...</div>
      <div class="pan-item">...</div>
    </div>
  </div>
</section>
```

```css
.pan-section { position: relative; height: 300vh; }
.pan-pin { position: sticky; top: 0; height: 100dvh; overflow: hidden; }
.pan-track { display: flex; gap: var(--space-4); height: 100%;
  align-items: center; padding-inline: var(--space-4); will-change: transform; }
.pan-item { flex: 0 0 70vmin; height: 70vmin; background: var(--surface);
  border: 1px solid var(--border); border-radius: var(--radius-lg); }
```

```js
import gsap from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
gsap.registerPlugin(ScrollTrigger);

export function horizontalPan(section, reduced) {
  const track = section.querySelector('.pan-track');
  const items = gsap.utils.toArray('.pan-item', section);
  if (reduced || items.length === 0) {
    track.style.transform = 'none';
    track.style.overflowX = 'auto';
    return () => {};
  }

  const getScroll = () => track.scrollWidth - window.innerWidth;
  const tween = gsap.to(track, {
    x: () => -getScroll(),
    ease: 'none',
    scrollTrigger: {
      trigger: section,
      start: 'top top',
      end: () => `+=${getScroll()}`,
      scrub: 1,
      pin: '.pan-pin',
      pinSpacing: false,
      invalidateOnRefresh: true,
      anticipatePin: 1,
    },
  });

  return () => tween.scrollTrigger.kill();
}
```

Hard limits:
- `pan-item` count max 6; beyond 6 the horizontal distance reads as a carousel, not a reveal.
- Fallback is native `overflow-x: auto`: no pin, no JS-driven transform.
- Never pin on mobile without testing at 390px width; if the pan feels cramped at mobile, switch to vertical stack at MOTION 8-10.

## 3. Reveal-stagger

Use when the `SECTION MAP` lists a section as `reveal-stagger`. A grid or list fades and rises in sequence as it enters the viewport.

```html
<section class="reveal-section" data-motion="reveal-stagger">
  <h2 class="reveal-title">Section title</h2>
  <div class="reveal-grid">
    <div class="reveal-item">...</div>
    <div class="reveal-item">...</div>
    <div class="reveal-item">...</div>
    <div class="reveal-item">...</div>
    <div class="reveal-item">...</div>
    <div class="reveal-item">...</div>
  </div>
</section>
```

```css
.reveal-grid { display: grid; grid-template-columns: repeat(3, 1fr);
  gap: var(--space-4); }
.reveal-item { background: var(--surface); border: 1px solid var(--border);
  border-radius: var(--radius-lg); padding: var(--space-5); }
```

```js
import gsap from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
gsap.registerPlugin(ScrollTrigger);

export function revealStagger(section, reduced) {
  const title = section.querySelector('.reveal-title');
  const items = gsap.utils.toArray('.reveal-item', section);
  if (reduced || items.length === 0) {
    gsap.set([title, ...items], { clearProps: 'all' });
    return () => {};
  }

  const tl = gsap.timeline({
    scrollTrigger: {
      trigger: section,
      start: 'top 80%',
      toggleActions: 'play none none none',
      once: true,
    },
  });

  if (title) {
    tl.from(title, { opacity: 0, y: 16, duration: 0.4, ease: 'power2.out' });
  }
  tl.from(items, {
    opacity: 0,
    y: 24,
    duration: 0.5,
    ease: 'power2.out',
    stagger: 0.08,
  }, title ? '-=0.2' : 0);

  return () => tl.scrollTrigger.kill();
}
```

Hard limits:
- `stagger` max 0.08s; larger values lag behind fast scroll.
- `y` offset max 24px; larger reads as a jump, not a reveal.
- `toggleActions` is always `play none none none`; scrubbed staggers fight the eye.

## React adapter (all skeletons)

```jsx
import { useRef } from 'react';
import { useGSAP } from '@gsap/react';
import { stickyStack } from './motion/sticky-stack.js';

function StackSection() {
  const ref = useRef(null);
  useGSAP(() => {
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const cleanup = stickyStack(ref.current, reduced);
    return cleanup;
  }, { scope: ref });
  return (
    <section ref={ref} className="stack-section">...</section>
  );
}
```

Do not call `gsap.context()` manually; `useGSAP` handles scope and cleanup. In vanilla JS, wrap each skeleton call in a function that returns the cleanup and invoke it on page unload.

## Reduced-motion fallback

Every skeleton takes `reduced: boolean`. When true:

- `sticky-stack`: cards render in normal document flow, absolutely positioned styles removed.
- `horizontal-pan`: track becomes horizontally scrollable (`overflow-x: auto`), scroll-snap optional.
- `reveal-stagger`: all items visible immediately (`opacity: 1; transform: none`).

Never hide content behind JS opacity when reduced motion is active. The page must be fully readable with animations disabled.
