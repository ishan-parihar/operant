# Content: Fill the Slots

Fill every content slot in the SECTION MAP with real copy and a real asset decision. Use IMMEDIATELY after the SECTION MAP names sections and slot counts, before any component build. Produces the CONTENT fenced block downstream builds paste from. Triggers on empty headlines, lorem-style filler, fake screenshots, fake-precise stats, or mixed-voice pages.

The SECTION MAP says "hero: headline, sub, CTA x2, asset". This phase decides what that headline actually says and what that asset actually is. A component built with placeholder copy is built twice: once structurally wrong for the real copy length, once to be rebuilt. Copy first, always.

- Input: `DESIGN BRIEF` (vibe words, audience, dials) and `SECTION MAP` (section names, slot counts).
- Output: one `CONTENT` fenced block, one entry per section, every slot filled. Downstream skills copy from it verbatim.
- Hard rule: no component build starts while any slot reads "TBD", "lorem", or a placeholder.

## 1. Slot Grammar

Each section in the SECTION MAP has named slots. Fill all of them:

| Slot | What goes in | Hard limit |
|---|---|---|
| headline | One line of copy, sentence case | Renders on max 2 lines at 1280px |
| sub | Supporting sentence | 20 words max |
| body | Paragraph for text sections | 25 words max |
| cta | Button or link label, verb first | 3 words max |
| quote | Testimonial or pull quote | 3 rendered lines max |
| attribution | Quote source | name + role, optionally company |
| asset | What the image actually is | source decided per Section 3 |
| logos | Logo wall members | real company names only |
| list | Feature or spec list | item count matches SECTION MAP |

If a slot cannot be filled with real copy, the section is wrongly specced. Go back and change the SECTION MAP (merge, cut, or convert the section) rather than inventing filler to justify it.

## 2. Copy Rules

### Voice system (frontend-design writing intentionality)

Write from the end user's perspective: name things by what users understand in simple language ("manage notifications" not "webhook config"). Describe what something is or does in plain terms, not selling it. Specific and legible beats clever.

- Active voice as default. A CTA says exactly what happens: "Save changes" not "Submit". An action keeps the same name through the whole flow: button "Publish" produces toast "Published". Vocabulary is signposting; cohesion teaches navigation.
- Plain verbs, sentence case, no filler. Tone matches brand/audience. Each written element does exactly one job. Treat failure and emptiness as direction, not mood: explain what went wrong and how to fix it in the interface's voice (never vague apology); empty screens invite action ("Create your first project" with CTA).
- Conversational tone: plain verbs, no filler, matched to brand. One copy register per page (see below).

### Voice: one page, one voice

Pick one register from the vibe words and stay in it for every string on the page:

- **Technical-mono**: terse, numeric, lowercase ticks (`47 tasks · 0.6 ctx-switches/day`). For developer tools, infrastructure.
- **Marketing-punch**: short sentences, confident verbs, no hedging. For SaaS, consumer.
- **Editorial**: longer clauses, literary rhythm, proper nouns. For portfolios, studios, manifestos.
- **Service-plain**: government-flat, imperative, zero adjectives. For public sector, regulated.

Never mix two on one page. The hero and the footer speak the same language.

### CTA says exactly what happens

A CTA is a promise. "Start free trial" starts a free trial. "Save changes" saves changes. Banned labels: "Submit", "Get started", "Learn more", "Click here", "Discover". The verb names the outcome, the noun names the object, three words maximum. An action keeps the same name through the whole flow: a button that says "Publish" produces a toast that says "Published", not "Success".

### Density defaults

- Headline: 8 words or fewer.
- Sub / body: 25 words or fewer, one idea only.
- CTA: 3 words or fewer.
- Quote: 3 rendered lines at 1280px. Longer gets cut, not wrapped.

### Banned constructions

- No em-dashes anywhere on the page. Use a comma, a colon, or a full stop.
- No straight ASCII quotes in quotes or testimonials. Use real typographic marks (“ ” and ‘ ’).
- No cute-but-wrong wordplay, metaphors that do not track, or "elegant nothing" phrases ("Where design meets possibility", "Crafted with intention").
- No passive-aggressive humility or fake-craftsman labels ("honest software", "built with care"). If the product is good, say what it does.

### The self-audit (run before emitting CONTENT)

Re-read every visible string you wrote, out of context, and flag:

1. **Broken grammar or hallucination**: cute-but-wrong wordplay, a metaphor that does not survive a literal reading, anything that sounds thoughtful and means nothing.
2. **LLM-trying-to-sound-thoughtful**: passive-aggressive humility, mock-poetic micro-copy, fake-craftsman sign-offs.
3. **Fake precision**: invented numbers that look measured. `92% faster`, `4.1×`, `48k users`, `5.8 mm`, `13.4 lb` are all red flags unless they come from the user's brief, real analytics, or a labeled example.

Every flagged string gets replaced with something boring that means exactly what it says. Boring and true beats clever and fake, every time.

### Fake-precise numbers

Numbers read as measurements. Only three sources are legitimate:

1. **The brief**: the user gave the figure.
2. **Real data**: analytics, benchmarks, docs that actually exist.
3. **Labeled mock**: explicitly marked (`<!-- example stat -->`, "sample data", in a demo component).

Anything else is a lie that survives code review because it looks confident. Delete it or replace it with a true claim that needs no number ("used daily by teams at" + real logos beats "12,847 happy users").

## 3. Asset Strategy (Per Asset Slot)

Decide every asset slot in this order. Never skip ahead because a later rung is easier to type.

1. **Image-generation tool first.** If any image-gen tool exists in the environment (`generate_image`, an MCP image tool, an IDE-integrated generator), generate the asset. Prompt it section-specific: photography style, subject, mood from the vibe words. Generate per-section assets, not one generic hero.
2. **Real web photography second.** No gen tool: use real photography with a working URL:
   - `https://picsum.photos/seed/{descriptive-seed}/{w}/{h}` with a section-specific seed (`content-craft-hero`, `marrow-cookware-kitchen`).
   - Open-license sources (Unsplash source URL, Pexels direct link) when a specific subject is needed.
3. **Labeled placeholder third.** If neither is possible, a clearly labeled slot: `<!-- TODO: hero product photo, 1600x1200, replace before ship -->`. A visible placeholder beats a fake asset.

Never substitute: hand-rolled SVG illustrations, div-based fake screenshots, CSS blobs posing as photography. A hand-rolled SVG is allowed only as a simple geometric mark (circle, square, single-letter monogram) when a real logo does not exist.

### Hero always gets a visual

No hero ships text-only. Minimum acceptable: one real photograph or generated image. A text-only page is incomplete work, not minimalism. Even an editorial or Linear-style page needs 2-3 real images: hero, one product or lifestyle shot, one supporting image. DENSITY dial low does not mean image count zero.

### Product proof without a screenshot

Div-built fake dashboards, terminal windows, and code editors are banned. When the product needs proof, use exactly one of:

1. A real screenshot URL.
2. A generated screenshot via an image tool.
3. A mini version of the actual component rendered inside the page (real UI, not a lookalike).
4. Editorial photography instead of product UI at all.

### Logo walls: real marks only

A "Trusted by" strip is logos or nothing. Never plain-text wordmarks (`<span>Acme Co</span>` in a bold font) and never industry labels beneath a logo (`Vercel` + "hosting" under it - the logo carries the credibility or it does not).

- Real companies: Simple Icons CDN at `https://cdn.simpleicons.org/{slug}/{color}` (white `ffffff` for dark mode). For tech-stack logos, the `simple-icons` package or `@svgr/cli` exports.
- Invented companies (demo content): a single simple geometric mark or a one-letter monogram in a circle, inline SVG, in the page's style. Same treatment for every member of the wall.
- Logos must work in the active theme: white-on-dark, near-black-on-light, one color each.

## 4. Content Slot Format

The output of this phase is one fenced block. One entry per section, slots in SECTION MAP order, every slot filled. Downstream builds paste verbatim, so the entry names must match the SECTION MAP section names exactly.

```
CONTENT
hero:
  headline: <8 words max>
  sub: <20 words max>
  cta-primary: <verb + object, 3 words max>
  cta-secondary: <verb + object, 3 words max, or none>
  asset: <gen-tool prompt | picsum seed URL | TODO slot>
features:
  headline: <...>
  items: [ {title, body<25w>}, ... ]  # count = SECTION MAP count
proof:
  quote: <max 3 lines, typographic quotes, no em-dash>
  attribution: <name, role, company>
  logos: [simple-icons slugs or generated marks]
footer:
  tagline: <...>
  links: [...]
```

## 5. Worked Example (B2B SaaS Landing)

Brief: landing for a meeting-notes SaaS. Audience: engineering managers at B2B companies. Vibe: calm, premium, Linear-style. Dials VARIANCE=6 MOTION=4 DENSITY=3. Mode greenfield.

SECTION MAP (from structure/): hero (headline, sub, cta x2, asset), features (headline, 4 items), proof (quote, attribution, 5 logos), cta-final (headline, cta), footer (tagline, 6 links).

Step by step:

1. **Voice**: technical-confident marketing punch. Short sentences, no hedging, no jargon that a VP would not parse.
2. **Hero copy**: headline promises the outcome, not the feature. "Meeting notes that write themselves" - 5 words, sentence case, no em-dash. Sub under 20 words: "Record any call. Get decisions, action items, and a searchable transcript before you hang up." (18 words.) CTA primary: "Start free trial". Secondary: "Watch 2-min demo". Both verbs, both name the exact result.
3. **Hero asset**: image-gen prompt: "calm minimal photograph, over-shoulder view of a laptop on a wooden desk showing a clean notes interface, soft morning light, muted warm tones, Linear-style aesthetic." No gen tool: `https://picsum.photos/seed/scribe-hero-desk/1600/1200`.
4. **Features** (4 items, each body under 25 words): "Action items pulled out automatically", "Search every meeting you have ever had", "Syncs to Linear and Slack", "Works on Zoom, Meet, and Teams". Bodies are one plain sentence each. No "leverage", no "seamlessly".
5. **Quote**: “I stopped taking notes in week one. The action items are better than the ones I wrote myself.” - 2 lines. Typographic quotes, comma instead of em-dash, attribution "Priya Nair, Head of Engineering, Northwind".
6. **Logos**: five real-company slugs from simple-icons if the brief names them; otherwise five generated one-letter monogram circles, identical treatment, with an `<!-- example logos -->` marker.
7. **Self-audit pass**: "2-min demo" is real (the demo video exists in a real product; in mock the duration comes from the brief or gets cut to "Watch demo"). No fake percentages anywhere. No em-dashes. No "crafted", no "beautiful", no "powerful". Voice is consistent: every string is a short confident sentence.

Emitted block:

```text
CONTENT
hero:
  headline: Meeting notes that write themselves
  sub: Record any call. Get decisions, action items, and a searchable transcript before you hang up.
  cta-primary: Start free trial
  cta-secondary: Watch demo
  asset: gen://calm minimal photograph, laptop with clean notes UI on wooden desk, soft morning light, muted warm tones
    fallback: https://picsum.photos/seed/scribe-hero-desk/1600/1200
features:
  headline: Built for teams that ship
  items:
    - { title: Action items, pulled out, body: Every task mentioned on the call lands in a list with an owner. }
    - { title: Search everything, body: Full-text search across every meeting you have ever recorded. }
    - { title: Syncs to your stack, body: Push decisions to Linear and Slack without leaving the recap. }
    - { title: Works where you meet, body: Zoom, Meet, and Teams. One click to start capturing. }
proof:
  quote: “I stopped taking notes in week one. The action items are better than the ones I wrote myself.”
  attribution: Priya Nair, Head of Engineering, Northwind
  logos: [zoom, slack, linear, notion, github]   <!-- swap to client-approved list -->
cta-final:
  headline: Your next meeting, already written up
  cta: Start free trial
footer:
  tagline: Notes that keep up with the meeting.
  links: [Product, Pricing, Changelog, Security, Privacy, Terms]
```

## 6. Rules That Bite

1. Every slot in the SECTION MAP is filled in CONTENT before any component build starts.
2. Density: headline 8 words, sub 20 words, body 25 words, CTA 3 words, quote 3 rendered lines.
3. Zero em-dashes in any emitted string. Zero straight ASCII quotes in quotes or testimonials.
4. Zero fake-precise numbers without a brief source or an explicit mock label.
5. Every asset slot resolves to: generation prompt, real photography URL, or a labeled TODO. Never a hand-rolled illustration, never a div-based fake screenshot.
6. Logo walls carry real slugs (simple-icons) or uniform generated monograms. Never plain text wordmarks, never labels under logos.
7. One voice page-wide: every string passes the self-audit against the vibe words.

## 7. Checks

1. Every section named in the SECTION MAP has a CONTENT entry, and every slot in the SECTION MAP count is filled. No TBD, no lorem, no placeholder copy.
2. Every headline is 8 words or fewer, every sub 20 or fewer, every body 25 or fewer, every CTA 3 words or fewer and starts with a verb that names the outcome.
3. Search the emitted block for em-dash and straight-quote: zero hits.
4. Every quote is 3 rendered lines or fewer, with real typographic quotes and a name + role attribution.
5. Every numeric claim traces to the brief, real data, or carries an explicit mock/example label.
6. Every asset slot names a generation prompt, a real `https://` image URL, or a `TODO` comment. None is a hand-rolled SVG illustration or div-based fake screenshot.
7. Logo slots list simple-icons slugs or uniformly styled generated monograms, with no plain-text wordmarks and no labels beneath logos.
