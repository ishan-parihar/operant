# Preflight: Final Quality Gate

Final quality gate before shipping a website-design page. Runs upstream anti-slop plus seven own checks (accessibility, performance, SEO meta, Open Graph, favicon, 404/sitemap, render fallback), enforces ordering, fixes findings in one batch, re-audits until clean.

This leaf is the FINAL gate in the [quality](../quality.md) lane. It runs immediately before
handoff, after anti-slop, audit, accessibility, performance, and production
leaves have already been consulted during build. Preflight does not redo their
deep work. It runs a fast, ordered sweep of eight checks that catch anything
that slipped through, fixes everything in one batch, then re-audits until
every check passes. Only then may the page ship.

## Ordering

Checks run in this exact order and the order matters.

1. Anti-slop (upstream). Violations here change palette, layout, and copy, so
   they must surface before any downstream check examines markup or assets.
2. Accessibility (own). Contrast, focus visibility, ARIA roles. Structure
   changes here can alter Lighthouse scores and rendered output, so it runs
   before performance.
3. Performance (own). Lighthouse LCP, CLS, INP thresholds. Layout and media
   fixes land before SEO so meta tags see final markup.
4. SEO meta (own). Title, description, canonical, lang, viewport.
5. Open Graph and social cards (own). Depends on SEO meta being correct
   first; og:title, og:description derive from it.
6. Favicon and app icons (own). Asset presence and link tags.
7. 404 page and sitemap.xml (own). Routes and crawl surface.
8. Render fallback (own). Verify the page still renders with JavaScript
   disabled. Content must be reachable without hydration.

Do not reorder. Do not parallelize. Each check assumes the previous checks
have already passed on the current build.

## Scope and Inputs

Preflight operates on one page build at a time: the URL of the running dev
server or static dist, the source tree under the page root, and the
production domain when known. It assumes anti-slop, accessibility,
performance, and production leaves were consulted during build; preflight
is the verification pass, not the design pass. It runs the same eight
checks every time, regardless of which leaf raised the most findings
upstream, because pre-flight is the point where regressions from unrelated
fixes are caught.

Caller supplies three paths: the page URL, the source root, and an
optional production base URL used to validate canonical and og:url
targets. If the production base URL is unknown at preflight time, use a
placeholder domain and flag the difference in the final report rather than
skipping the check.

## Output

The deliverable is a one-line PASS or FAIL per check plus a fix list, not
a long-form report. If the caller asked for a written summary the audit
leaf produced one already; preflight confirms it on the final bits.

## Tools

- Run all checks against the local dev server or a static build. Default URL
  is `http://localhost:3000`. Override with the actual port in use.
- Checks 1 through 7 use the squirrel audit CLI:
  `squirrel audit <url> --format llm --quick 25 --max-score 100`.
  The `--format llm` flag returns a compact, token-lean report.
  After any fix batch, re-run with `--refresh` to re-fetch the page and
  `--diff <prior-id>` to compare against the previous audit.
- Check 8 uses render fallback: fetch the page HTML with JavaScript disabled
  (`curl -sL <url>` or a headless browser with JavaScript off) and confirm
  primary content is present in the initial HTML payload.

## The Eight Checks

Each check is binary. PASS means move on, FAIL means record the finding and
continue to the next check (do not stop mid-sweep). All failures are fixed
together in one batch after the sweep completes.

### Check 1: Anti-Slop (upstream)

Run `scripts/detect.sh` from [anti-slop](anti-slop.md) against the page source
tree. All 17 checks must pass.

```bash
./quality/anti-slop/scripts/detect.sh ./src --quiet
```

Common failures: banned warm-beige backgrounds, Inter on slate, AI purple
glow, triple equal cards, eyebrow spam, generic SaaS names.

If any detect.sh check fails, record the check number and matched files.
Do not continue to Check 2 with unfixed slop still in the diff. Record the
findings, finish the sweep, then fix everything in one batch.

PASS condition: `detect.sh` exits 0 with `0 failed`.

### Check 2: Accessibility

Verify three items using the squirrel audit output:

1. Contrast ratio at least 4.5:1 for body text, 3:1 for headings and large
   text at 24px or larger, against the actual computed background color.
2. Every focusable element (links, buttons, form controls) has a visible
   focus style. Check for `outline` or `ring` styles on `:focus` and
   `:focus-visible`. Flag any `outline: none` or `outline-none` without a
   replacement ring class.
3. Semantic ARIA usage: landmark regions (`header`, `nav`, `main`,
   `footer`), `aria-label` on icon-only buttons, `alt` text on every image
   that conveys content, `aria-hidden="true"` on decorative elements.

Run:

```bash
squirrel audit http://localhost:3000 --format llm --quick 25 --category accessibility
```

PASS condition: zero FAIL findings in the accessibility category.

### Check 3: Performance

Run Lighthouse via squirrel audit and enforce these budgets:

- LCP (Largest Contentful Paint): under 2.5 seconds.
- CLS (Cumulative Layout Shift): under 0.1.
- INP (Interaction to Next Paint): under 200 milliseconds.

```bash
squirrel audit http://localhost:3000 --format llm --quick 25 \
  --category performance --max-score 100
```

If any metric exceeds its budget, record the offending element selectors
from the report. Common culprits: unoptimized hero images (missing
`width`/`height`, no `loading="eager"` on LCP image, no `loading="lazy"`
below the fold), web fonts without `display=swap`, layout-shifting banners.

PASS condition: all three metrics under budget on the target device
profile (mobile by default).

### Check 4: SEO Meta

Verify presence and sanity of primary meta tags on every page:

- `<title>` present, unique per page, 30 to 65 characters.
- `<meta name="description">` present, 70 to 160 characters, unique.
- `<html lang="...">` set to the page language.
- `<meta name="viewport" content="width=device-width, initial-scale=1">`.
- `<link rel="canonical">` pointing at the production URL.

```bash
squirrel audit http://localhost:3000 --format llm --quick 25 --category seo
```

PASS condition: zero FAIL findings for missing or duplicated core meta.

### Check 5: Open Graph and Social Cards

Verify social sharing tags:

- `og:title`, `og:description`, `og:type`, `og:url`, `og:image` all present.
- `og:image` is an absolute URL, at least 1200x630 pixels, under 8 MB.
- `twitter:card` set to `summary_large_image` when an image exists.
- `og:title` falls back to the page `<title>` if omitted; flag if both
  missing.

PASS condition: every page renders a card preview without placeholder
images or localhost URLs.

### Check 6: Favicon and Icons

Verify icon assets resolve:

- `<link rel="icon">` pointing to an existing file (favicon.ico or SVG).
- Apple touch icon at 180x180 if the target audience includes iOS.
- No 404s in the network log for icon requests.
- SVG favicon preferred over ICO when both exist; keep ICO as fallback.

PASS condition: icon tab renders in a headless browser tab without a
broken image and every declared icon URL returns HTTP 200.

### Check 7: 404 Page and Sitemap

Verify error handling and crawlability:

- A custom `/404` route or `404.html` exists, includes site navigation and
  a link back home, and is styled consistently with the rest of the site.
- Requesting a bogus URL returns HTTP 404, not 200.
- `sitemap.xml` exists at the site root and lists every public route.
- `robots.txt` exists and references the sitemap.

```bash
curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/does-not-exist
curl -s http://localhost:3000/sitemap.xml | head
```

PASS condition: bogus URL returns 404 with a styled page, sitemap parses
as valid XML with at least one URL entry.

### Check 8: Render Fallback

Verify the page renders without JavaScript. Fetch raw HTML and confirm the
primary content (headline, hero copy, primary call to action, footer) is
present in the initial payload, not injected after hydration.

```bash
curl -sL http://localhost:3000 | grep -i "primary headline text"
```

If the page is a client-rendered SPA with empty initial HTML, flag it and
either enable SSR/SSG or add `<noscript>` with essential content.

PASS condition: primary content appears in the raw HTML response.

## Fix-in-Batch Loop

After the sweep completes, collect every FAIL finding from all eight checks
into one ordered fix list. Apply all fixes in a single batch, addressing
them in check order so upstream fixes flow downstream. Do not fix one check
and re-audit immediately. Batch everything first.

After the batch lands, re-audit once:

```bash
squirrel audit http://localhost:3000 --format llm --quick 25 --refresh \
  --diff <previous-audit-id> --max-score 100
```

The `--refresh` flag forces a re-fetch of the page. The `--diff` flag
compares findings against the prior audit and reports only items that
changed. Use the report to verify every prior FAIL now reads PASS and no
new FAILs appeared.

Repeat the loop, fix batch then re-audit, until every check passes. Seven
or fewer iterations is normal. If more than ten iterations pass without
convergence, stop and report the blocking finding to the caller instead of
looping.

If the repeat loop exceeds two iterations, look for a systemic cause (a
shared layout partial, a global stylesheet) rather than continuing to
patch individual pages.

## Acceptance Criteria

- All 8 checks pass on the final build.
- Final squirrel audit reports `--max-score 100` achieved, or 100 minus
  documented exceptions the caller has explicitly accepted.
- anti-slop `detect.sh` exits 0.
- No em-dash or en-dash characters anywhere in shipped markup or copy.
- No emoji in shipped markup or copy unless explicitly requested in the
  client brief.
- Render fallback confirmed; primary content reachable without JavaScript.

## Worked Example

Given a landing page under `src/pages/index.astro` ready to ship:

1. `detect.sh ./src --quiet` fails on check 5 (AI purple glow in `hero.
   astro` line 42) and check 12 (filler verb "seamlessly integrate" in
   `features.astro` line 18). Record both.
2. Accessibility audit passes.
3. Performance audit fails on LCP at 3.4s: hero image missing
   `loading="eager"` and fetched at 2400px wide for a 1200px slot.
4. SEO meta fails: `<title>` is "Home" (generic), no description.
5. OG check fails: no `og:image` defined.
6. Favicon passes.
7. 404 check fails: bogus URL returns 200 with empty body.
8. Render fallback passes.

Fix batch: swap the purple glow for the palette neutral, rewrite the
filler line with concrete copy, add `loading="eager"` and resize the hero
source image, write a real title and 90-character description, add
`og:image` pointing to `/og/cover.png`, add a styled 404 route returning
status 404.

Re-audit:

```bash
squirrel audit http://localhost:3000 --format llm --quick 25 --refresh \
  --diff audit-2026-09-15-01 --max-score 100
```

All findings report PASS, score 100. Ship.

## Anti-Patterns

- Do not skip Check 1. Slop bleeds into every downstream check.
- Do not fix findings one at a time. Batch, then re-audit once.
- Do not re-audit without `--refresh`; cached pages hide fixes.
- Do not omit `--diff`; without it you cannot confirm prior FAILs
  resolved.
- Do not treat a Lighthouse score below budget as acceptable "for now."
  Adjust images, fonts, or layout until budgets hold.
- Do not ship without the render-fallback check passing. An empty body for
  no-JS clients is a FAIL even if every other check passes.
