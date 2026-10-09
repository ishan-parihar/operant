# Accessibility

Audit and fix WCAG 2.2 AA compliance: contrast ratios, focus states, screen-reader semantics, keyboard reachability, motion preferences, and dark-mode parity. Run BEFORE shipping any page, AFTER visual QA flags contrast or focus, or whenever a component ships without aria labels, visible focus, or a reduced-motion path.

Accessibility is a quality gate, not a polish pass. Every page must survive a
keyboard-only walk, a screen-reader pass, a 200% zoom, and a forced dark-mode
flip without breaking layout, hiding controls, or losing meaning. This leaf is
the audit: run it after components exist and before delivery, and re-run it
whenever a visual rule changes contrast, motion, or focus paths.

Scope: WCAG 2.2 level AA is the floor. AAA items are called out explicitly
where they are cheap. Anything not labeled AAA is AA and mandatory.

## Procedure

1. Inventory every interactive element on the page: links, buttons, inputs,
   selects, textareas, custom widgets, dialogs, menus, tooltips.
2. Run the seven audit lanes below against that inventory, in order. Each lane
   lists measurable pass conditions and a fix pattern.
3. For each component flagged by ui-ux-pro-max (icons, forms, toasts, charts,
   nav), apply the component-specific pattern in its lane.
4. Run `scripts/contrast.py` against the computed foreground/background pairs.
   Fix any pair under 4.5:1 (text) or 3.1 (non-text UI and its states).
5. Run `## Checks`. Fix fails before delivery.

## Lane 1: Contrast (AA)

| Subject | Minimum ratio | Notes |
|---|---|---|
| Body text (under 24px / 18.66px bold) | 4.5:1 against its background | measured on both light and dark themes |
| Large text (24px+ or 18.66px+ bold) | 3:1 | rare on marketing pages, still check |
| Nav links, form labels, button labels | 4.5:1 | even when styled small or muted |
| Placeholder text | 4.5:1 | placeholders are real content when they carry examples |
| Focus indicator | 3:1 vs adjacent color, plus visible shape change | never removed; see Lane 2 |
| Borders and icons carrying meaning | 3:1 | chart lines, toggles, icon buttons, error icons |
| Disabled controls | no contrast requirement | but they must look disabled and be unreachable |
| Logo text and brand marks | exempt | still prefer 3:1 so they read |

Dark mode rule: every contrast check runs twice, once per theme. A color pair
passing in light mode is not evidence it passes in dark mode. Tailwind `dark:`
variants and CSS custom-property themes both count; rerun against computed
colors, not source tokens.

Fix pattern: never dim body copy to get hierarchy. Raise hierarchy with weight,
size, or spacing. If a muted variant is unavoidable, measure it against 4.5:1
before use. Decorative-only text below 4.5:1 is banned; call it decorative in
code with a comment and remove it from a11y trees.

Icon buttons: the icon itself must hit 3:1 against its background, and the
button must have an accessible name (`aria-label` or visually-hidden text).
A visible label nearby is not enough; screen readers need the name bound to
the control.

## Lane 2: Focus and keyboard reachability

No default focus rings removed. The `:focus-visible` or `:focus` outline must
be visible on every interactive element. A subtle `outline: 2px solid
var(--accent)` with `outline-offset: 2px` is the floor. Replacing the ring
with a background tint alone fails the 3:1 adjacent-state contrast test unless
the tint hits 3:1 against the unfocused state.

Logical tab order matches visual order. The DOM is the keyboard; do not use
positive `tabindex` values to reorder. `tabindex="0"` is allowed to add
non-widget elements to the order; `tabindex="-1"` for programmatic focus
targets only (skip link target, dialog on open).

Skip link: first focusable element on the page, jumps to `#main` or the page's
`<main>` landmark. It must be visible on focus, not `display: none`. Standard
pattern: absolute-positioned offscreen until `:focus`, then rendered with
contrast-safe styling.

Focus never disappears behind chrome. Sticky headers, cookie banners, and
toasts must not cover the focused element. Use `scroll-padding-top` or
`scroll-margin-top` on focus targets equal to the sticky header height plus
padding (matches navigation leaf). WCAG 2.2 Focus Not Obscured (Minimum): AA.
Focus Not Obscured (Enhanced, full component visible): AAA, cheap when the
sticky region is bounded.

Dialogs: focus traps while open, returns focus to the trigger on close, ESC
closes. Annotate the trigger with a comment noting the trap owner so regressions
are greppable.

Touch targets: minimum 24 by 24 CSS px for pointer targets, 44 by 44 px for
native app conventions. Spacing alone does not count as size; size the actual
hit area (`min-width`, `min-height`, padding on the interactive element
itself).

## Lane 3: Semantics and accessible names

Use the native element. `<button>` over `<div onclick>`, `<a href>` over
`role="link"`, `<ul>` for nav lists, `<nav>` with `aria-label` for multiple
navs (`Primary`, `Breadcrumb`, `Footer`). Native roles, states, and keyboard
behavior come free with the right tag; re-creating them with ARIA is banned.

Headings nest in order: `<h1>` once per page, `<h2>` for sections, `<h3>`
inside sections, never skipping a level. Visual style is a class, not a tag;
a small heading in a card is still `<h2>` if it heads a section. Landmark
elements carry `aria-label` when there is more than one of the same kind
(`<nav aria-label="Footer">`).

Every form control has a programmatically associated label (`<label for>` or
`aria-label`). Placeholders are not labels. Buttons must name their action
(`Save changes` over `OK`); link text must make sense out of context (`Read
the pricing report` over `click here`); icon-only buttons get `aria-label="…"`.

Decorative icons and duplicated glyphs are removed from the accessibility
tree with `aria-hidden="true"`. Meaningful standalone icons carry
`role="img"` and `aria-label` or an inline `<title>`.

Images: informative images get descriptive `alt`; decorative images get
`alt=""` and `aria-hidden="true"`; figure captions use `<figure>` and
`<figcaption>`.

Live regions: status changes, toasts, form errors, and async content use
`aria-live="polite"` (default) or `aria-live="assertive"` only for critical
alerts. `role="status"` and `role="alert"` map to polite and assertive.
Errors render with `role="alert"`, and invalid inputs set
`aria-invalid="true"` plus `aria-describedby` pointing at the message.

Keyboard-only operation: every action has a non-pointer path. Drag and drop
gets a single-pointer alternative (WCAG 2.2 Dragging Movements, AA). Hover
tooltips are reachable by keyboard and dismissible with ESC (WCAG 1.4.13).
Carousels and auto-updating regions pause on hover and focus, and can be
paused manually.

## Lane 4: Motion and reduced-motion preference

`prefers-reduced-motion` is wired before any non-trivial motion ships. CSS
fallback: wrap animation and transition rules in
`@media (prefers-reduced-motion: no-preference) { … }` so reduced-motion
users get static layouts by default. The inverse (`reduce`) must kill parallax,
autoplay, scroll-triggered reveals, and velocity-based transforms.

JS motion respects the same signal. Use `window.matchMedia('(prefers-reduced-
motion: reduce)')` or framework hooks such as Motion's `useReducedMotion()`.
When reduced motion is on: skip GSAP ScrollTrigger pins, disable scrub
animations, drop parallax offsets, and snap to final states. Cross-section 6.B
of the motion spec requires this at every intensity level.

An `MOTION_INTENSITY` dial at 3 or higher still ships the reduced-motion gate;
the dial never overrides the user preference. Same for scroll hijacking,
autoplay video, and CSS scroll-driven animations: gate them.

Vestibular triggers: no flashing above 3 flashes per second (WCAG 2.3.1),
no rapid full-screen zooms, no smooth scrolling of whole-page anchors without
`prefers-reduced-motion` check. Anchor scroll uses `window.matchMedia` to
pick `behavior: 'instant'` when reduced motion is set.

## Lane 5: Dark mode parity

Dark mode is a first-class theme, not an afterthought. Rules:

- Respect `prefers-color-scheme` unless the user explicitly picked a theme.
- One strategy for the whole page: Tailwind `dark:` variant OR CSS custom
  properties with `data-theme` toggles. Do not mix.
- Every contrast check from Lane 1 is rerun against dark-palette values.
- Shadows, overlay scrims, and glass effects are re-earned in dark mode;
  nav blur borders and card borders often disappear.
- Focus outlines that pass against light backgrounds often fail against dark
  surfaces. Define a dark-safe focus color (`--focus-ring-dark`) and use it.
- Images with baked-in shadows or light backgrounds must be swapped for
  dark-safe assets or wrapped in contrasting containers.
- Never render white text on near-white images or logos on black.

Brand-preservation rule: dark mode keeps the brand's primary hue and contrast
relationships. Do not invert arbitrarily; map the design tokens and re-derive
surface scales (`--bg`, `--bg-raised`, `--text`, `--text-muted`,
`--border`) with the same lightness curve in both themes.

## Lane 6: Component-specific rules

Apply these on top of the generic lanes when a matching component exists.

| Component | Rule | Reference |
|---|---|---|
| Form inputs | Label above, helper text below; placeholder is an example only; `aria-describedby` binds helper and error | forms leaf |
| Error message | `role="alert"`, visible when text present, replaced not appended, 4.5:1 on destructive color | ui-ux-pro-max `aria-live-errors` |
| Submit feedback | `aria-live="polite"` region announcing success or failure; disabled button state is not enough | ui-ux-pro-max `toast-accessibility` |
| Icon-only button | `aria-label` with verb; focus ring preserved; touch target 24x24 minimum | quick-reference `aria-labels` |
| Decorative icon next to text | `aria-hidden="true"` on the icon, text carries meaning | quick-reference `icon-context` |
| Toast | `aria-live="polite"`, does not steal focus, dismissible with ESC | quick-reference `toast-accessibility` |
| Nav menu | `<nav aria-label="...">`, `aria-current="page"` on active item, ESC closes mobile menu, focus trapped | navigation leaf |
| Tooltip | Keyboard reachable, appears on focus as well as hover, ESC dismisses | quick-reference `tooltip-keyboard` |
| Modal dialog | Focus trap, `aria-modal="true"`, `aria-labelledby`/`aria-describedby`, returns focus on close | Lane 2 |
| Table data | `<table>` with `<th scope="col|row">`; use `aria-sort` on sortable headers | quick-reference `sortable-table` |
| Charts/dataviz | Provide `aria-label` summary and a text alternative (table or caption); do not rely on color alone | quick-reference `data-table`, `pattern-texture` |
| Live regions | `aria-live="polite"` default, `role="status"` for status updates, `role="alert"` for errors | Lane 3 |
| Skip link | Visible on focus, lands on `<main>` landmark with `tabindex="-1"` | Lane 2 |
| Pagination | `<nav aria-label="Pagination">`, current page with `aria-current="page"` | navigation leaf |
| Video | Controls keyboard-reachable, captions on, no autoplay with audio | Lane 4 |
| Autocomplete | `aria-expanded`, `aria-activedescendant` on the input, options in a `role="listbox"` | ui-ux-pro-max |

## Lane 7: WCAG 2.2 additions worth shipping

These are new in 2.2 and commonly missed. All AA unless noted.

- Focus Not Obscured (Minimum): sticky UI must not hide the focused element.
  Covered by Lane 2; restated here because it is the most-failed AA item on
  pages with sticky headers and consent banners.
- Focus Not Obscured (Enhanced), AAA: the entire focused component stays
  visible. Cheap to reach when the only sticky chrome is the header.
- Focus Appearance, AAA: focus indicator area is at least as large as a 2px
  perimeter of the component and hits 3:1 against the unfocused state. The
  default accent ring usually satisfies this; re-check after dark-theme
  override.
- Dragging Movements, AA: any author-supplied drag interaction (sliders with
  arrows, drag-to-reorder lists, kanban columns) needs a single-pointer
  alternative, typically buttons or a dropdown for reorder actions.
- Target Size (Minimum), AA: pointer targets at least 24 by 24 CSS px, with
  listed exceptions (inline links in sentences, browser defaults, essential
  small targets like map pins). Undersized targets may pass if spaced so no
  two targets overlap within a 24px circle, but size the control instead.
- Consistent Help, A: help mechanisms that repeat across pages (contact
  link, chat widget, help icon) keep the same relative order.
- Redundant Entry, A: do not ask for information the user already supplied in
  the same process. Pre-fill, select from previous entries, or accept in a
  different format.
- Accessible Authentication (Minimum), AA: never gate login on a cognitive
  test (memory puzzle, transcription of a code shown as an image) without a
  non-cognitive path. Password managers and paste must work in password
  fields. Enhanced version is AAA and only a topic when the site targets an
  AAA audit.

## Lane 8: Anti-patterns to flag on sight

- `outline: none` or `outline: 0` without a `:focus-visible` replacement
  in the same rule block.
- `tabindex` values above 0 anywhere in the document.
- `aria-hidden="true"` on an element that contains a focusable child.
- Icon-only button whose only name source is a `title` attribute. Tooltip is
  not an accessible name; add `aria-label`.
- A `div` or `span` with `onClick` and a `cursor: pointer` rule but no role
  and no keyboard handler. Convert to `<button>` rather than layering ARIA.
- Placeholder text acting as the only label for an input.
- Auto-playing video or carousel with no pause control and no reduced-motion
  branch.
- Color as the only signal: red-only error borders, green-only success text,
  color-coded charts without a pattern or label channel.
- Contrast tokens resolved through layers of variables that never get
  checked against the final computed value. Always audit computed colors.
- Focus moved on route change to nowhere, leaving keyboard users at the
  previous page's tab position. On client-side navigation, move focus to
  `<main>` or the new page's `<h1>` with `tabindex="-1"`.

## Lane 9: Verification scripts

Run the repo helper from the skill root:

```
uv run scripts/contrast.py --fg '#1f2937' --bg '#f9fafb'
```

It prints the contrast ratio and a PASS/FAIL line against the 4.5:1 text and
3:1 non-text thresholds. Feed it computed (after theme resolution) colors,
not token names. For a full sweep, pass the two CSS custom-property sets.

Checks you cannot script (keyboard trap, focus obscuring, semantic order)
are verified manually by walking the page with Tab, Shift-Tab, arrow keys,
and a screen reader (VoiceOver on macOS, NVDA/Orca on Linux). Log any fix
in the commit message with the rule ID (for example, `fix(a11y): contrast
4.2:1 -> 4.6:1 on --text-muted`).

## Checks

Seven binary checks, all must pass before delivery. Each check is a single
greppable or walkable assertion; do not batch them.

1. Interactive count matches keyboard-focused count: every button, link,
   input, select, textarea, and custom widget is reachable by Tab in a
   source-ordered path. Any item skipped fails.
2. Focus indicator present and non-trivial: every focused element shows a
   visible indicator with 3:1 contrast against its unfocused state and the
   surrounding background. No `outline: none` without an override.
3. Contrast ratios pass: every text pair at 4.5:1 or higher, meaning-bearing
   icons/borders at 3:1 or higher, and a separate run for the dark theme if
   one ships.
4. Accessible names bound: every icon button has `aria-label` or
   visually-hidden text; every form control has a `<label>` or `aria-label`;
   every nav landmark has a unique `aria-label` when more than one exists.
5. Reduced-motion path works: with `prefers-reduced-motion: reduce`, no
   parallax, autoplay, velocity, or scroll-driven animations run, and anchor
   jumps are instant.
6. Focus never obscured: with the sticky header and any banners present,
   tabbing to each control keeps the full component visible (or at minimum
   does not cover it). Sticky-header offset uses `scroll-padding-top` so a
   focused anchor is not hidden.
7. Lane-3 semantics hold: heading levels nest without skips, lists are lists,
   tables are tables, live regions use the right roles, and no `div`/`span`
   is styled to act as a button or link.

Reference each failing check by number in the fix commit.
