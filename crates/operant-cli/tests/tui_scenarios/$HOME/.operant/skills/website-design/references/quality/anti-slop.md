# Anti-Slop: Absolute Bans, Templated Tells, and Consistency Locks

Sole source of absolute bans, templated tells, and consistency locks for website-design. Run detect.sh to audit code against 17 automated patterns before shipping.

This leaf is the SOLE authority for absolute bans, forbidden design patterns,
and consistency locks across the entire website-design skill tree. Sibling leaves
must link here rather than duplicating ban lists.

LLMs default to predictable clichés: purple gradients, centered heroes, three equal
cards, Inter on slate, and warm-beige craft palettes. This leaf enforces mechanical
rejection of those tells.

## Procedure

1. Read the brief and foundation from [briefing](../briefing.md) and [direction](../direction.md).
2. Inspect draft markup, styles, and copy against the 12 templated tells below.
3. Verify compliance with the three mandatory consistency locks (Shape, Color, Button).
4. Run `references/quality/anti-slop/scripts/detect.sh` across the codebase to execute the 17 automated checks.
5. Fix any reported violation before passing output to [preflight](preflight.md).

## 1. Templated Tells and Bias Overrides

### 1.1 AI-Purple Glow
- Ban: Defaulting to purple or violet button glows, neon mesh gradients, or violet
  accent glow (`shadow-purple`, `from-purple-600 to-indigo-600`, `rgba(168,85,247,*)`).
- Why: It is the universal visual signature of unthinking LLM code generation.
- Override: Allowed only when the client brief explicitly names violet or purple as
  the primary corporate brand color. Even then, keep backgrounds neutral.

### 1.2 Centered Hero Default
- Ban: Centering every hero element (`text-center mx-auto`) when `VARIANCE > 4`.
- Why: Centered headlines over centered paragraphs create an identical silhouette.
- Override: Allowed only for short manifestos, single-input search tools, or launch
  announcements where text alone is the visual centerpiece. Otherwise use split (50/50),
  left-aligned headline with right-aligned asset, or asymmetric grid.

### 1.3 Three Equal Cards and the SaaS-Card Kit
- Ban: Three identical cards in a row (`grid-cols-1 md:grid-cols-3 gap-6`), each with
  an icon in a colored circle, bold title, two-line description, and arrow link.
- Why: The most overused marketing pattern on the web. It signals zero layout thought.
- Override: Use varied compositions: bento grids with mixed tile spans (2+1, 1+2),
  horizontal scroll-snap rows, zigzag feature pairings, or interactive tabs.

### 1.4 Glass Everywhere
- Ban: Applying `backdrop-blur` indiscriminately across cards, headers, badges, and
  sidebars without physical borders, inner highlights, or high-contrast fallbacks.
- Why: Generic blur without structure produces washed-out, illegible surfaces.
- Override: Glass is permitted for floating navigation or media overlays only when
  accompanied by a 1px border (`border-white/10`) and an inner highlight
  (`shadow-[inset_0_1px_0_rgba(255,255,255,0.1)]`), plus solid-color fallbacks.

### 1.5 Inter + Slate Default
- Ban: Defaulting to Inter sans paired with `slate-900` or `slate-950` backgrounds.
- Why: Default font stack of every boilerplate template since 2022.
- Override: Pick purposeful typography from [direction](../direction.md): Geist, Cabinet
  Grotesk, Satoshi, Outfit, or Plus Jakarta Sans. Reserve Inter strictly for neutral
  Linear-style tools or public-sector accessibility requirements.

### 1.5b Overdefaulted Serif Editorial Kit
- Ban: Reaching for Fraunces or Instrument Serif italic accents inside a modern sans
  headline as a default design move.
- Why: Mixed serif-sans headline styling became a signature LLM flourish for any brief
  that mentions editorial, fashion, or magazine.
- Override: Serif display type is allowed only as the primary brand typeface for the
  whole page, selected deliberately in [direction](../direction.md), not injected into one
  word of a sans headline. Use italic or weight shift inside the same family instead.

### 1.6 Premium Warm-Beige Trap
- Ban: Reaching for warm cream backgrounds, brass/clay accents, and espresso text
  whenever a brief mentions premium consumer, artisan, culinary, or luxury craft.
- Banned background hexes: `#f5f1ea`, `#f7f5f1`, `#fbf8f1`, `#efeae0`, `#ece6db`, `#faf7f1`, `#e8dfcb`.
- Banned accent hexes: `#b08947`, `#b6553a`, `#9a2436`, `#9c6e2a`, `#bc7c3a`, `#7d5621`.
- Banned dark text hexes: `#1a1714`, `#1a1814`, `#1b1814`.
- Why: The model uses this single palette for every DTC brand, erasing brand identity.
- Override: Rotate to cold luxury (silver and chrome), deep forest green with bone,
  sharp monochrome with a single saturated pop, or cobalt on cool neutral.

### 1.7 Tracked ALL-CAPS Eyebrow and Section Numbering
- Ban: Putting small uppercase wide-tracked labels (`text-xs uppercase tracking-widest`)
  above every single section header. Decorative section numbers like `01 / INDEX`,
  `001 · Capabilities`, `Stage 01: Install`, or `Phase 02` are banned when content is NOT a genuine sequence.
- Why: Repetitive mechanical rhythm that adds clutter instead of information.
- Rule: Maximum 1 eyebrow per 3 sections. Hero counts as 1. If section A has an eyebrow,
  sections B and C must not. Section numbers permitted ONLY when [structure](../structure.md) 4.7 confirms content is a genuine sequence (steps, timeline, ranked order); otherwise name the content directly without numbering.

### 1.8 Middle-Dot Meta and Decorative Dots
- Ban: Stringing phrases together with repetitive middle dots (`Item · Category · Date · Tag`).
  Ban decorative colored status dots before normal navigation links and list items.
- Why: Middle dots and colored circles mimic status dashboards on marketing sites.
- Rule: Maximum 1 middle dot per line in metadata strips. Use columns, line breaks, or
  subtle hairlines. Colored dots are permitted only for real semantic system state.

### 1.9 Spaced Em-Dash and Typographic Crutches
- Ban: The em-dash character (U+2014) and en-dash character (U+2013) are completely forbidden
  in headlines, eyebrows, pills, body copy, testimonials, and metadata.
- Why: The em-dash is the number one stylistic crutch of LLM text generation.
- Rule: Restructure sentences using periods, commas, colons, or parentheses. For
  number ranges or compound phrases, use the standard hyphen (`-`).

### 1.10 Tinted Near-Black and Pure Extreme Values
- Ban: Hardcoding pure `#000000` or `#ffffff` as large flat surface fills, and ban
  uncalibrated muddy near-black tints.
- Why: Pure black flattens depth; uncalibrated muddy tints clash with cool neutrals.
- Rule: Use semantic tokens from [system](../system.md) (`--bg`, `--surface`, `--text`).
  Calibrate dark mode to off-blacks (`zinc-950`, `gray-950`) with tuned contrast.

### 1.11 Monospace Data Labels and Fake-Precision Numbers
- Ban: Styling arbitrary text labels in monospace font to look technical. Ban fabricated
  hyper-precise metrics like `99.99% uptime`, `4.1x faster`, `48k stars`, or `13.4 lb`.
- Why: Monospace is for code and tabular figures, not decoration. Fake metrics destroy trust.
- Rule: Use `font-mono` only for real code snippets, tabular financial figures, or CLI output.
  If metrics are placeholders, label them clearly with mock data markers.

### 1.12 Arrow on Every Link
- Ban: Appending `→`, `->`, or arrow icons to every single hyperlink and button.
- Why: Creates visual noise and turns text links into repetitive pointer clutter.
- Rule: Reserve directional arrows for primary progression actions (next step, proceed
  to checkout). In-page navigation and secondary links rely on underline or weight.

## 2. Mechanical Bans and Grep Patterns

| Tell / Anti-Pattern | Grep Search Pattern | Required Remediation |
|---|---|---|
| Em-dash or En-dash | `[\x{2014}\x{2013}]\|&mdash;\|&ndash;` | Replace with period, comma, or standard hyphen |
| Warm-Beige BG Hexes | `(?i)#(f5f1ea\|f7f5f1\|fbf8f1\|efeae0\|ece6db)` | Use cool neutral, forest, or cold luxury tokens |
| Warm Craft Accent Hexes | `(?i)#(b08947\|b6553a\|9a2436\|9c6e2a\|bc7c3a)` | Select distinct brand accent from direction/ |
| Espresso Text Hexes | `(?i)#(1a1714\|1a1814\|1b1814)` | Use calibrated neutral `--text` token |
| Purple / Violet Glow | `(?i)(shadow-purple\|rgba\(\s*168,\s*85,\s*247)` | Replace with neutral border or brand accent |
| Inter + Slate Default | `(?i)(font-inter.*slate-900\|font-sans.*slate-900)` | Pick intentional typography from direction/ |
| Triple Equal Card Grid | `grid-cols-1 md:grid-cols-3 gap-[0-9]+` | Convert to bento, asymmetric split, or list |
| Tracked Eyebrow Spam | `(uppercase\s+tracking-(wider\|widest))` | Limit to max 1 per 3 sections; delete excess |
| Section Number Labels | `(?i)(?:0[0-9]\s*\/\|00[0-9]\s*·\|Step\s*0?[1-9]:)` | Remove numbering; use direct semantic title |
| Middle Dot Overuse | `·.*·.*·` | Limit to max 1 dot per line; use whitespace |
| Generic Names & Metrics | `(?i)\b(John Doe\|Jane Doe\|Acme\|99\.99%)\b` | Use realistic domain data and believable metrics |
| Marketing Filler Verbs | `(?i)\b(elevate your\|seamlessly\|unleash)\b` | Rewrite with direct, concrete action verbs |
| Fake Screenshot Divs | `(?i)(bg-gray-800.*rounded-t.*space-x-2\|mock-terminal)` | Use real screenshot or clean product wireframe |
| Arrow Link Spam | `(?i)(href=[^>]*>[^<]*[\x{2192}]\|href=[^>]*>.*->)` | Remove decorative arrow; style with underline |
| Glass Without Border | `backdrop-blur-(?:md\|lg)\s+(?!.*border-)` | Add 1px border and inner shadow highlight |
| Hardcoded Pure Black/White | `(?i)bg-\[#(?:000000\|ffffff)\]` | Use `--bg` and `--surface` semantic tokens |
| Duplicate CTA Intent | Synonymous labels in one page | Unify to single CTA text per user intent |

### 2.1 False Precision and Mock Data Discipline
- Fake-precise metrics (`92% faster`, `4.1x`, `48k users`) require one of two states:
  real supplied data from the brief, or a visible mock marker (`<!-- mock -->`).
- Names, testimonials, and avatars follow the same rule: either real or clearly
  labeled as placeholder. Invented names that read like plausible people are the
  worst option of the three.

### 2.2 Scope Boundary of This Leaf
- This leaf owns: char-level and markup-level bans, palette ban lists, eyebrow and
  meta density limits, title-case and box-radius hygiene in components, consistency
  locks, and the 17 automated detect.sh checks.
- This leaf does NOT own: type scale selection ([system](../system.md)), font pairing
  ([direction](../direction.md)), case study or hero composition (components/*), dialog or
  toast density counts, or the pass/fail preflight verdict (quality/preflight).
- Sibling leaves link here for bans rather than duplicating any row of the table.
  If two leaves contradict, the rule listed in this leaf wins.

## 3. Consistency Locks

### 3.1 Shape Consistency Lock
Pick exactly ONE corner-radius scale for the entire page and lock it:
- Sharp: `rounded-none` (0px) everywhere (cards, buttons, inputs, badges).
- Soft: `rounded-lg` to `rounded-xl` (8px to 12px) for cards, `rounded-md` (6px) for controls.
- Pill: `rounded-full` for all interactive buttons and tags; `rounded-2xl` (16px) for cards.
Mixing sharp cards with pill buttons or random rounded corners across sections is banned.
Every interactive component must follow the declared scale.

### 3.2 Color Consistency Lock
Once the primary accent color is declared in the DESIGN BRIEF, lock it across the entire page:
- Do not introduce a blue CTA on an emerald-accented landing page.
- Do not use amber warning badges as decorative accents in the footer.
- Secondary actions must use neutral ghost or outlined buttons, not alternate hues.
- Contrast verification: Text on accent background must pass WCAG AA (minimum 4.5:1
  for body, 3:1 for large text 18px+).

### 3.3 Button Consistency and Contrast Lock
- Single line desktop rule: Button text must fit on one line at desktop viewports.
  Text wrapping inside a button is an immediate failure. Keep CTA copy to 1-3 words.
- No duplicate CTA intent: Pick one label per conversion intent across the entire page.
  Do not combine "Get in touch", "Contact us", and "Let us talk". Pick ONE label and
  reuse it in header, hero, and footer.
- Ghost button legibility: Ghost buttons placed over photography or patterned backgrounds
  must include a tinted backdrop or explicit border to prevent contrast collapse.

### 3.4 Responsive and Density Fallback Lock
- Any multi-column layout (2+ columns) must name its mobile collapse explicitly.
  Two columns collapse to stacked at `md` (768px) by default; three or more collapse
  to single column with `w-full px-4 py-8`.
- DENSITY stays consistent across one page: a sparse hero (`py-32`) followed by a
  cramped 12-row hairline table is a locked violation. Section spacing rhythm is set
  once in the DESIGN BRIEF and repeated.
- Headline scale couples to copy length: a headline over 6 words may not exceed
  `text-4xl md:text-5xl` on desktop; only 3-word headlines may use `text-6xl`.

### 3.5 Spacing and Token Integrity Lock
- Raw hex codes outside token files are banned. Every color must resolve through
  a declared token (`--bg`, `--surface`, `--accent`, `--border`, `--text`).
- One spacing scale only: use 4px base tokens (`--space-1` through `--space-10`).
  Arbitrary margin and padding values (like `mt-[37px]` or `p-[19px]`) fail review.

## 4. Worked Example: Refactoring Slop to Clean Design

### Slop Input (Violates 6 Anti-Slop Rules)
```html
<section class="bg-[#f5f1ea] py-24 text-center">
  <span class="text-xs uppercase tracking-widest font-mono text-[#b08947]">01 / CAPABILITIES</span>
  <h2 class="text-4xl font-inter text-[#1a1714] mt-2">Elevate your workflow &#8212; seamlessly</h2>
  <p class="text-gray-600 max-w-xl mx-auto mt-4">Cloudly empowers modern teams to unleash productivity.</p>
  <div class="grid grid-cols-1 md:grid-cols-3 gap-6 max-w-5xl mx-auto mt-12">
    <div class="bg-white p-6 rounded-lg shadow-purple">
      <div class="w-10 h-10 rounded-full bg-purple-100 mb-4"></div>
      <h3 class="font-bold">Fast Setup</h3>
      <p class="text-sm text-gray-500 mt-2">99.99% automated config for teams.</p>
      <a href="#" class="text-purple-600 mt-4 inline-block">Learn more -></a>
    </div>
  </div>
</section>
```

### Refactored Output (Clean, Intentional, Slop-Free)
```html
<section class="bg-zinc-900 py-24 text-zinc-100 border-t border-zinc-800">
  <div class="max-w-6xl mx-auto px-6 grid grid-cols-1 lg:grid-cols-12 gap-12 items-start">
    <div class="lg:col-span-5">
      <h2 class="text-3xl lg:text-4xl font-semibold tracking-tight text-white">
        Built for continuous deployment
      </h2>
      <p class="text-zinc-400 mt-4 text-base leading-relaxed">
        Verify pull requests against production constraints before merging.
      </p>
      <a href="/docs/deploy" class="inline-flex items-center gap-2 text-emerald-400 font-medium mt-6 hover:underline">
        Read deployment guide
      </a>
    </div>
    <div class="lg:col-span-7 grid grid-cols-1 sm:grid-cols-2 gap-4">
      <div class="p-5 rounded-lg bg-zinc-800/60 border border-zinc-700/60">
        <h3 class="font-medium text-white text-base">Isolated sandbox</h3>
        <p class="text-sm text-zinc-400 mt-2">Runs ephemeral test clusters in under four seconds.</p>
      </div>
      <div class="p-5 rounded-lg bg-zinc-800/60 border border-zinc-700/60">
        <h3 class="font-medium text-white text-base">Branch routing</h3>
        <p class="text-sm text-zinc-400 mt-2">Direct staging traffic via custom domain headers.</p>
      </div>
    </div>
  </div>
</section>
```

## Checks

Execute these 7 mechanical checks using `references/quality/anti-slop/scripts/detect.sh` before finalizing any page.
A single failing check blocks preflight; no short list of exceptions ships through.
If a brief explicitly overrides a ban (brand color is violet, the studio asks for a
serif editorial kit), record the override in the DESIGN BRIEF, keep the rest of the
bans intact, and still run the remaining checks.

1. Typographic Dash Check:
   Run `scripts/detect.sh --check 1 <path>`
   Verify 0 em-dash (U+2014) and en-dash (U+2013) characters exist in markup and copy.
2. Banned Palette Check:
   Run `scripts/detect.sh --check 2 <path>`, `--check 3`, and `--check 4`
   Verify 0 instances of banned warm-beige backgrounds, brass accents, or espresso text.
3. Layout Repetition and Grid Check:
   Run `scripts/detect.sh --check 7 <path>`
   Verify no identical 3-column card rows exist without asymmetric cell spans.
4. Eyebrow and Metadata Restraint Check:
   Run `scripts/detect.sh --check 8 <path>`, `--check 9`, and `--check 10`
   Verify eyebrow density is at most 1 per 3 sections, with zero section-number prefixes
   and at most 1 middle dot per metadata line.
5. AI Glow and Palette Check:
   Run `scripts/detect.sh --check 5 <path>` and `--check 6`
   Verify no purple/violet glow tokens exist and Inter is not lazily paired with Slate-900.
6. Copy Realism and Buzzword Check:
   Run `scripts/detect.sh --check 11 <path>`, `--check 12`, `--check 13`, and `--check 14`
   Verify zero placeholder names (Jane Doe, Acme), fake metrics, filler verbs, or arrow link spam.
7. Consistency Locks Check:
   Run `scripts/detect.sh --check 17 <path>`
   Verify shape consistency (one radius scale), color consistency (single accent hue),
   and button intent discipline (no duplicate CTA labels for the same action).
