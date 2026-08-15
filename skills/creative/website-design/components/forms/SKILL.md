---
name: forms
description: "Build form fields, validation, states, and submit feedback. Load BEFORE writing any input, contact form, checkout, settings panel, or multi-step flow. Covers label anatomy, semantic types, autocomplete, inline validation, error recovery, aria-live, and the full submit lifecycle."
metadata:
  operant:
    tags: [forms, inputs, validation, submit]
---

# Forms

Field anatomy, validation behavior, the six field states, and the submit
lifecycle. Default LLM output ships the static success state only; this leaf
forces the full cycle. Signup/waitlist one-field blocks: `components/conversion/SKILL.md`.

## Procedure

1. List every field: name, semantic type, required or optional, autocomplete
   token (table below).
2. Write the field anatomy for each (label above, helper and error below).
3. Wire validation events (blur, not keystroke) and focus management.
4. Style all six states from the states table, then the submit lifecycle.
5. Run `## Checks`.

Framework mappings (React Hook Form, shadcn Form, Vue): `references/stack-adapters.md`.
The anatomy and event rules here are the contract; libraries only automate them.

## Field anatomy

Top to bottom, gap `--space-2` (8px) inside a field block, `--space-4` (16px)
between blocks:

```html
<div class="field">
  <label for="email">Work email <span aria-hidden="true">*</span></label>
  <input id="email" name="email" type="email" required autocomplete="email"
         aria-describedby="email-help">
  <p class="helper" id="email-help">We only use this for the invoice.</p>
  <p class="error" id="email-err" role="alert" hidden>
    Enter a valid email like name@company.com.</p>
</div>
```

- Label ABOVE the input, always visible. Placeholder-as-label is banned; it
  vanishes on the first keystroke. Placeholder holds an example value only.
- Helper text sits below the input, persistent, in `--text-muted`.
- Error text sits below in `--destructive` and REPLACES the helper while
  visible (toggle `hidden`). Error copy states cause plus fix, never a bare
  "Invalid input".
- Required fields: `*` in the label plus the `required` attribute. If most
  fields are required, mark the optional ones "(optional)" instead.
- Group related fields with `<fieldset><legend>`.

## Semantic types and autocomplete

Why: the right `type` triggers the right mobile keyboard; `autocomplete` lets
the browser fill the field for the user.

| Data | type | autocomplete |
|---|---|---|
| Full name | text | name |
| Email | email | email |
| Phone | tel | tel |
| Password (login) | password | current-password |
| Password (signup) | password | new-password |
| One-time code | text + `inputmode="numeric"` | one-time-code |
| Street address | text | street-address |
| Postal code | text | postal-code |
| Card number | text + `inputmode="numeric"` | cc-number |
| Search box | search | off |

- Password fields get a show/hide toggle: a `type="button"` inside the field
  with `aria-pressed`, never a submit.
- Every input: `min-height: 44px` (touch target) and font-size at least 16px
  (prevents iOS focus zoom).

## Validation behavior

| Event | Do |
|---|---|
| While typing (pristine field) | Nothing. Never validate per keystroke. |
| Blur | Validate that field; show its inline error if invalid. |
| Input on an errored field | Re-validate; clear the error the moment it is fixed. |
| Submit with 1-2 errors | Show inline errors; move focus to the first invalid field. |
| Submit with 3+ errors | Also render an error summary at the top: heading plus anchor links (`href="#field-id"`) to each invalid field; move focus to the summary. |

- Error nodes carry `role="alert"` (or live inside an `aria-live="polite"`
  region) so screen readers announce them.
- Invalid inputs get `aria-invalid="true"` and `aria-describedby` pointing at
  the error id.
- Multi-step forms: show progress ("Step 2 of 4" or a bar), allow back
  navigation without losing entered data, validate one step at a time.
- Long forms (10+ fields or multi-step): autosave a draft (localStorage or
  server) so accidental dismissal loses nothing.
- Destructive submits (delete account): button uses `--destructive`, sits
  visually separated from primary actions, and requires a confirmation step.

## States table

Exact CSS per state. Error and success colors must pass 4.5:1 against `--bg`.

| State | Selector | CSS |
|---|---|---|
| Default | `.field input` | `background: var(--bg); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius);` |
| Focus | `input:focus-visible` | `outline: 2px solid var(--accent); outline-offset: 1px; border-color: var(--accent);` |
| Filled | `input:not(:placeholder-shown)` | no restyle; filled fields must not look "completed" |
| Error | `input[aria-invalid="true"]` | `border-color: var(--destructive);` focus outline flips to `var(--destructive)` |
| Disabled | `input:disabled` | `opacity: 0.45; cursor: not-allowed;` plus the `disabled` attribute in markup |
| Readonly | `input:read-only` | `background: var(--surface);` full `--text` contrast, normal cursor; must NOT look disabled |

## Submit lifecycle

idle -> loading -> success | error. Never skip loading; never leave loading
without reaching a terminal state.

| Phase | Button | Feedback |
|---|---|---|
| Idle | enabled, e.g. "Send message" | none |
| Loading | `disabled` immediately (blocks double submit), inline spinner appears beside the label; label text stays so width does not shift | none yet |
| Success | hidden or re-enabled | replace the form, or show an inline confirmation line with a checkmark in the `aria-live="polite"` status node; use `--text` plus a glyph, do not invent a success token |
| Error | re-enabled | inline errors or summary, message states cause plus retry path, all entered values preserved; a timeout shows "Request timed out" with a retry button |

## Worked example: contact form

```html
<form class="contact" action="/contact" method="post" novalidate>
  <div class="summary" role="alert" tabindex="-1" hidden>
    <h3>Fix 3 errors to send your message</h3>
    <ul>
      <li><a href="#c-name">Add your name</a></li>
      <li><a href="#c-email">Enter a valid email</a></li>
      <li><a href="#c-msg">Add a message</a></li>
    </ul>
  </div>

  <div class="field">
    <label for="c-name">Name <span aria-hidden="true">*</span></label>
    <input id="c-name" name="name" type="text" required autocomplete="name">
    <p class="error" id="c-name-err" role="alert" hidden>
      Add your name so we know who to reply to.</p>
  </div>

  <div class="field">
    <label for="c-email">Work email <span aria-hidden="true">*</span></label>
    <input id="c-email" name="email" type="email" required autocomplete="email"
           aria-describedby="c-email-help">
    <p class="helper" id="c-email-help">We reply within one business day.</p>
    <p class="error" id="c-email-err" role="alert" hidden>
      Enter a valid email like name@company.com.</p>
  </div>

  <div class="field">
    <label for="c-msg">Message <span aria-hidden="true">*</span></label>
    <textarea id="c-msg" name="message" rows="5" required></textarea>
    <p class="error" id="c-msg-err" role="alert" hidden>
      Tell us a sentence or two about your project.</p>
  </div>

  <button class="btn btn-primary" type="submit">
    <span class="spinner" aria-hidden="true" hidden></span>Send message
  </button>
  <p class="status" aria-live="polite"></p>
</form>
```

```css
.contact { display: grid; gap: var(--space-4); max-width: 480px; }
.field { display: grid; gap: var(--space-2); }
.field label { font-weight: 600; color: var(--text); }

.field input, .field textarea {
  min-height: 44px; padding: var(--space-2) var(--space-3);
  font: inherit; font-size: max(1rem, 16px);
  background: var(--bg); color: var(--text);
  border: 1px solid var(--border); border-radius: var(--radius);
}
.field :focus-visible {
  outline: 2px solid var(--accent); outline-offset: 1px;
  border-color: var(--accent);
}
.field input:disabled, .field textarea:disabled { opacity: 0.45; cursor: not-allowed; }
.field input:read-only { background: var(--surface); }

.helper { font-size: 0.875rem; color: var(--text-muted); }
.error  { font-size: 0.875rem; color: var(--destructive); }
/* error replaces helper, and flags the control */
.field:has(.error:not([hidden])) .helper { display: none; }
.field:has(.error:not([hidden])) input,
.field:has(.error:not([hidden])) textarea { border-color: var(--destructive); }
.field:has(.error:not([hidden])) :focus-visible { outline-color: var(--destructive); }

.summary {
  border: 1px solid var(--destructive); border-radius: var(--radius);
  padding: var(--space-4); display: grid; gap: var(--space-2);
}
.summary h3 { color: var(--destructive); font-size: 1rem; }

.spinner {
  width: 1em; height: 1em; margin-right: var(--space-2);
  border: 2px solid currentColor; border-right-color: transparent;
  border-radius: 50%; animation: spin 700ms linear infinite;
}
@keyframes spin { to { transform: rotate(1turn); } }
.status { font-size: 0.9375rem; color: var(--text); }
```

Behavior contract (any language):

```
on field blur:            validate that field, show its error if invalid
on input in errored field: re-validate, clear error when fixed
on submit:
  errors = validate all fields, set aria-invalid on failures
  if errors >= 3: unhide summary, focus summary; stop
  if errors >= 1: focus first invalid field; stop
  disable button, unhide spinner, send request
  on success: status = "Message sent. We reply within one business day."
  on failure or timeout: re-enable button, hide spinner,
    status = cause + retry path, keep all field values
```

## Checks

1. Every input, select, and textarea has a `<label for>` matching its id;
   zero fields rely on placeholder as their only label.
2. Each field's `type` and `autocomplete` match the semantic table (grep
   `autocomplete=`; count equals the number of fillable fields).
3. Every error message names the cause and the fix; grep finds no bare
   "Invalid input" or "Error" strings.
4. The stylesheet defines all of `:focus-visible`, `:disabled`, `:read-only`,
   and an error selector for inputs (4 distinct greppable selectors).
5. Validation fires on blur, not per keystroke; errored fields clear on input.
6. Submit disables the button and shows an in-button spinner with no layout
   shift, and both terminal states (success and error with retry) exist.
7. Error nodes and the status line carry `role="alert"` or `aria-live`;
   a 3+ error submit renders a summary whose links anchor to field ids.
8. Every input computes to at least 44px height and at least 16px font-size.
