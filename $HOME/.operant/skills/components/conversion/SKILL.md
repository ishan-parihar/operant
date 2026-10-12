---
name: conversion
description: "Build pricing sections, CTA bands, and signup/waitlist blocks that convert. Load BEFORE writing any pricing table, call-to-action, email capture, or button markup, and when auditing a page for duplicate CTAs, wrapped labels, or unreadable buttons."
metadata:
  operant:
    tags: [cta, conversion, pricing, signup, waitlist]
---

# Conversion blocks

Pricing sections, CTA bands, signup/waitlist blocks, and the button system they
share. Every hard rule here is a gate in `quality/preflight/SKILL.md`.

## Procedure

1. Read the SECTION MAP. List every button and button-like link the page will
   contain (nav, hero, feature sections, pricing, final band, footer).
2. Deduplicate labels by intent (table below). Pick ONE label per intent and
   reuse it verbatim everywhere on the page.
3. Build the shared button system first (spec below), then the pricing, CTA,
   and signup sections that consume it.
4. Run `## Checks` before handing off.

Framework mappings (Tailwind, React, shadcn, Vue): `references/stack-adapters.md`.

## Button spec

One `.btn` base plus three variants. Hierarchy comes from tokens, never from
new hex values.

| Variant | Background | Text | Border | Use |
|---|---|---|---|---|
| Primary | `--accent` | `--accent-contrast` | none | The page's one primary intent |
| Secondary | transparent | `--text` | `1px solid var(--border)` | The alternate action beside a primary |
| Ghost | transparent | `--text` | none | Low-stakes actions ("Learn more", nav items) |

Hard rules. Why: default LLM output ships a static happy-path button that
fails accessibility.

- All four states defined for every variant: `:hover`, `:active`,
  `:focus-visible`, `:disabled`. No exceptions.
- `:active` gives tactile feedback: `transform: translateY(1px)` or `scale(0.98)`.
- `:focus-visible`: `outline: 2px solid var(--accent); outline-offset: 2px`.
- `:disabled`: `opacity: 0.45; cursor: not-allowed` AND the `disabled`
  attribute in markup, never a class alone.
- Touch target: `min-height: 44px` on every button and button-styled link.
- CONTRAST (WCAG AA, mandatory): button text vs button background at least
  4.5:1 (3:1 only for text 24px+, or 18px+ bold). White-on-white, accent text
  on accent bg, and borderless transparent buttons over photos are banned;
  give ghost buttons over imagery a scrim, backdrop, or border.
  Full ban list: `quality/anti-slop/SKILL.md`.
- WRAP BAN (mandatory): a primary CTA label is 3 words max and renders on one
  line at desktop. If it wraps, shorten the label or widen the button; never
  fix it by capping the button's `max-width`.

## CTA discipline (page-wide)

One primary intent per page, repeated with the SAME label in nav, hero, and
footer. Why: two labels for one action reads as two different actions and
splits the click.

| You wrote | Intent | Keep exactly one, e.g. |
|---|---|---|
| "Get in touch", "Contact us", "Let's talk", "Reach out" | contact | "Contact us" |
| "Try free", "Get started", "Sign up free", "Start now" | signup | "Get started" |
| "View work", "See projects", "Browse portfolio" | portfolio | "View work" |

Final CTA band (last section before the footer):

- Headline: 8 words max, restates the value prop. Not "Ready to get started?"
  boilerplate.
- Exactly one button: the primary intent, same label as nav and hero.
- Optional one reassurance line under the button that lowers perceived risk:
  "Free 14-day trial. No credit card." True claims only; rules in
  `content/SKILL.md`.
- The band may flip to an accent background with `--accent-contrast` text,
  but every element on it must still pass contrast (see worked example for
  the inverted button).

## Pricing section

| Rule | Value | Why |
|---|---|---|
| Tiers on a landing page | 2-3 max | 4+ is a comparison task; move it to a dedicated /pricing page |
| Highlighted tier | exactly 1 | mark with `2px solid var(--accent)` border, slight scale, or a "Recommended" badge; never by flipping the whole card to accent background (kills feature-list contrast) |
| Feature rows per tier | 8 max | longer lists go to a comparison table on the pricing page |
| Price typography | large number + small period label | amount in `--font-display` at 2.5-3.5rem; "/month" at 0.875-1rem in `--text-muted` |
| Numerals | `font-variant-numeric: tabular-nums` | billing-toggle swaps must not shift layout |
| CTA per tier | 1 button, all tiers same intent | featured tier gets primary variant, others secondary |
| Claims | no invented stats, no fake precision | copy rules in `content/SKILL.md` |

Annual/monthly toggle: a real radio group, not a styled div.

```html
<fieldset class="billing-toggle">
  <legend class="visually-hidden">Billing period</legend>
  <label><input type="radio" name="billing" value="monthly" checked> Monthly</label>
  <label><input type="radio" name="billing" value="annual"> Annual (save 20%)</label>
</fieldset>
```

Tier skeleton (one `article` per tier inside a `.tiers` grid):

```html
<article class="tier tier-featured">
  <p class="badge">Recommended</p>
  <h3>Pro</h3>
  <p class="price"><span class="amount">$29</span><span class="period">/month</span></p>
  <ul class="features"><!-- 8 li max --></ul>
  <a class="btn btn-primary" href="#signup">Get started</a>
</article>
```

## Signup / waitlist block

Single email field plus button on one row; stack below 480px. Full field and
validation behavior lives in `components/forms/SKILL.md`; this block only adds:

- Input: `type="email"`, `autocomplete="email"`, `required`. A label element
  must exist in markup; it may be visually hidden ONLY here because the block
  heading names the action. Placeholder holds an example value, never the label.
- Success: replace the form row inline with a confirmation line announced via
  `aria-live="polite"`. Do not navigate away, do not use a toast.
- Error: message below the field in `--destructive`, stating cause and fix
  ("Enter a valid email like name@company.com"). Keep the user's input.
- Button label: the page's signup-intent label if that is the primary intent,
  otherwise "Join waitlist".

## Worked example: final CTA band with signup capture

```html
<section class="cta-band" id="signup">
  <div class="cta-inner">
    <h2>Ship your docs site this afternoon</h2>
    <form class="waitlist" action="/subscribe" method="post" novalidate>
      <label class="visually-hidden" for="cta-email">Work email</label>
      <input id="cta-email" name="email" type="email" required
             autocomplete="email" placeholder="name@company.com">
      <button class="btn btn-primary" type="submit">Get started</button>
    </form>
    <p class="reassurance">Free 14-day trial. No credit card.</p>
    <p class="form-msg" aria-live="polite"></p>
  </div>
</section>
```

```css
.cta-band {
  background: var(--accent);
  color: var(--accent-contrast);
  padding: var(--space-9) var(--space-5); /* 96px 24px */
  text-align: center;
}
.cta-inner { max-width: 560px; margin-inline: auto; display: grid; gap: var(--space-4); }
.cta-band h2 { font-family: var(--font-display); font-size: clamp(1.75rem, 4vw, 2.5rem); }

.waitlist { display: flex; gap: var(--space-2); }
.waitlist input {
  flex: 1; min-height: 44px; padding: 0 var(--space-3);
  font: inherit; font-size: max(1rem, 16px);
  background: var(--bg); color: var(--text);
  border: 1px solid var(--border); border-radius: var(--radius);
}
.waitlist input:focus-visible { outline: 2px solid var(--accent-contrast); outline-offset: 2px; }
@media (max-width: 480px) { .waitlist { flex-direction: column; } }

.reassurance { font-size: 0.875rem; opacity: 0.85; }

/* Button system: base + all four states, every variant */
.btn {
  display: inline-flex; align-items: center; justify-content: center;
  min-height: 44px; padding: 0 var(--space-4);
  border: none; border-radius: var(--radius);
  font: 600 1rem var(--font-body); white-space: nowrap;
  cursor: pointer; text-decoration: none;
  transition: filter 120ms ease, transform 120ms ease, background 120ms ease;
}
.btn-primary { background: var(--accent); color: var(--accent-contrast); }
.btn-primary:hover { filter: brightness(1.08); }
.btn-secondary { background: transparent; color: var(--text); border: 1px solid var(--border); }
.btn-secondary:hover { background: var(--surface); }
.btn-ghost { background: transparent; color: var(--text); }
.btn-ghost:hover { background: var(--surface); }
.btn:active { transform: translateY(1px); }
.btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.btn:disabled { opacity: 0.45; cursor: not-allowed; transform: none; }

/* On an accent band the primary button inverts to stay visible */
.cta-band .btn-primary { background: var(--accent-contrast); color: var(--accent); }
.cta-band .btn:focus-visible { outline-color: var(--accent-contrast); }
```

## Checks

1. List every button/link-as-button label on the page; each intent (contact,
   signup, portfolio, ...) maps to exactly ONE label string. Two labels for one
   intent is a fail.
2. Every primary CTA label is 3 words or fewer and renders on one line at
   1280px viewport width.
3. Button CSS contains all of `:hover`, `:active`, `:focus-visible`, and
   `:disabled` (grep the stylesheet; 4 distinct selectors minimum).
4. Every button text/background pair passes WCAG AA (4.5:1, or 3:1 for 24px+
   or 18px+ bold text), including inverted buttons on accent bands.
5. Every button and the signup input compute to `min-height` at least 44px.
6. Landing-page pricing shows 2 or 3 tiers, and exactly one tier carries the
   featured treatment (border, scale, or badge).
7. No tier feature list exceeds 8 rows; price amounts use tabular numerals.
8. The final CTA band has exactly one button, a headline of 8 words or fewer,
   and its button label matches the nav and hero labels character for character.
