# Production

Ship the production files every page needs but components never create: SEO meta tags, Open Graph and Twitter cards, 1200x630 social images, favicons, canonical URLs, sitemap.xml, robots.txt, a custom 404, and minimal analytics. Run AFTER content and components exist and BEFORE any deploy, domain hookup, or link sharing.

Components build the page. This leaf builds everything around the page that
search engines, social crawlers, and browsers need: meta tags, OG images,
favicons, sitemaps, robots rules, a 404 page, and analytics. None of these
appear in a visual pass, so they are the most commonly forgotten. Ship them as
a checklist, not as an afterthought.

Scope: static sites and server-rendered pages alike. Examples are plain HTML;
map `head` blocks into your framework's metadata API (Next.js `metadata`,
Nuxt `useHead`, Astro frontmatter) without changing the values.

## The production set

Every deploy ships all nine items. A missing one is a failed check, not a
style choice.

| Item | Purpose | Failure if missing |
|---|---|---|
| Title + meta description | Search snippet, tab label | Google rewrites your title;CTR collapses |
| Canonical URL | Dedupes www/https/trailing-slash copies | Ranking signals split across duplicates |
| Open Graph + Twitter card | Link previews in chat, feeds, iMessage | Bare URL previews, zero clicks |
| OG image 1200x630 | The visual in every shared link | Cropped logo or blank gray card |
| Favicon set | Tab icon, bookmark, home-screen icon | Default globe icon, broken PWA icon |
| sitemap.xml | Tells crawlers what exists and when it changed | Slow or partial indexing |
| robots.txt | Rules for crawlers, points at the sitemap | Admin/draft pages indexed |
| Custom 404 | Keeps lost visitors on site | Dead-end browser error page |
| Analytics | Measures what shipped | Flying blind; no conversion data |

## Meta block

One block per page, unique per route. Paste into `<head>` and edit values.
Titles run 30-60 characters, descriptions 70-160. Both describe this page,
not the whole site, and neither contains the words "Welcome to" or "Home page".

```html
<!-- Primary -->
<title>Cloud cost reports for small teams | Kelpline</title>
<meta name="description" content="Kelpline reads your AWS bill and sends one weekly email with the three changes that cut the most spend. Setup takes ten minutes.">
<link rel="canonical" href="https://kelpline.app/pricing">

<!-- Open Graph -->
<meta property="og:type" content="website">
<meta property="og:site_name" content="Kelpline">
<meta property="og:title" content="Cloud cost reports for small teams">
<meta property="og:description" content="One weekly email. Three fixes. Lower AWS bill.">
<meta property="og:url" content="https://kelpline.app/pricing">
<meta property="og:image" content="https://kelpline.app/og/pricing.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="Kelpline report showing a 31 percent spend cut">

<!-- Twitter -->
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="Cloud cost reports for small teams">
<meta name="twitter:description" content="One weekly email. Three fixes. Lower AWS bill.">
<meta name="twitter:image" content="https://kelpline.app/og/pricing.png">
```

Rules that catch real bugs:

- Every URL in this block is absolute, protocol included. Crawlers do not
  resolve relative paths in `og:image` or `canonical`.
- `og:image` must return HTTP 200 with an `image/*` content type and stay
  under 5 MB. Test it with `curl -sI <url>`, not by trusting the file exists.
- `og:title`, `og:description`, and title/meta description may differ from
  each other, but `og:url` and `canonical` must match exactly, character for
  character, including trailing slash policy. Pick one canonical form per
  route and 301 every alternative to it.
- Twitter falls back to `og:*` tags when its own are absent; ship both anyway
  so `summary_large_image` renders as a large card instead of a thumbnail.
- One `<title>`, one meta description, one canonical per page. Frameworks
  that merge metadata can emit duplicates; view source on the deployed page
  and count.

## OG images: 1200x630

One per shareable page, or per template with variables. 1200x630 PNG or JPEG
is the card size every platform honors; other aspect ratios get cropped to it
anyway. Keep the design inside a 40px safe margin, 60px on the right side
where some clients overlay a favicon.

Design rules: the image is a poster for the page, not a screenshot. Site
wordmark, one short headline, one supporting visual. Text at 60px or larger so
it survives a 300px-wide chat preview. Use brand colors from the token set;
a gray gradient with the logo centered reads as a placeholder even when it
was designed.

Generation ladders, laziest first:

1. A framework route handler (`opengraph-image.png` in Next.js, `satori`/`og`
   workers) that renders from site tokens. Maintains itself when copy changes.
2. A one-off render of an HTML page screenshotted at exactly 1200x630 during
   build. Hand it the same tokens as the site.
3. A static export from a design tool. Fine for a marketing site that rarely
   changes; date it in the filename.

Then verify with a real scraper before calling it done: paste the URL into
the X card validator, the LinkedIn post inspector, or `opengraph.xyz`. A
stale CDN cache on `og:image` is the number one reason shares show the old
image; version the filename (`pricing.v2.png`) instead of purging.

Deploy cover and marketplace metadata: when deploying through Higgsfield,
the `marketplace_cover_url` and `app-meta.json` fields decided in
`deploy/higgsfield` are separate from these SEO tags. The OG image here is
for off-platform sharing (chat, feeds, search social cards); the Higgsfield
cover is a 3:2 on-platform marketplace card. One does not substitute for the
other, and both get built before publish.

## Favicon set

Five files cover every consumer; no generator pipeline needed:

```
/favicon.ico            32x32 ICO, multi-resolution if trivial
/icon.svg               hand-authored, single color currentColor preferred
/apple-touch-icon.png   180x180, solid background, no transparency
/manifest.webmanifest   name, theme_color, icons pointing at 192 and 512 PNG
/icon-192.png, /icon-512.png
```

Reference them explicitly; do not rely on `/favicon.ico` auto-discovery
alone:

```html
<link rel="icon" href="/icon.svg" type="image/svg+xml">
<link rel="icon" href="/favicon.ico" sizes="32x32">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="manifest" href="/manifest.webmanifest">
```

The SVG is hand-authored from the wordmark glyph, never a raster traced into
SVG and never a base64 blob. Test dark mode: a dark glyph on a dark browser
theme disappears; give the SVG a contrasting background shape or use
`prefers-color-scheme` media inside the SVG to flip its fill.

## sitemap.xml and robots.txt

`sitemap.xml` lists every indexable route, one `<url>` each. Lastmod is a
real date the content changed, not the build timestamp; a always-today
lastmod teaches Google to ignore it.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>https://kelpline.app/</loc>
    <lastmod>2026-09-10</lastmod>
  </url>
  <url>
    <loc>https://kelpline.app/pricing</loc>
    <lastmod>2026-09-12</lastmod>
  </url>
</urlset>
```

Pages excluded from the sitemap: 404, thank-you/confirmation pages, filtered
views, anything behind auth, anything carrying `noindex`. The sitemap and
`noindex` headers must agree; a URL in both is a crawler contradiction.

`robots.txt` at the root, minimal:

```
User-agent: *
Allow: /
Disallow: /admin
Disallow: /api

Sitemap: https://kelpline.app/sitemap.xml
```

Robots disallow is crawl control, not security. Never list a secret path in
robots.txt to hide it; the file is public and the list is a map for exactly
the wrong reader. Sensitive routes use `noindex` meta plus authentication.

## Custom 404

A 404 page users land on through old links, typos, and truncated pastes.
Requirements: returns HTTP status 404 (not 200), matches the site's layout
and nav so the visitor knows where they are, offers two routes out (home and
the most popular page, or a search box), and says what happened in one
sentence without blaming the visitor. It must not appear in the sitemap,
must carry `noindex`, and should be as light as any other page since it
serves under error load.

Framework note: in static hosts the file is `404.html` at the root; on
Higgsfield and SPA hosts, wire the not-found route so unknown paths render
this page with a real 404 status, not a client-side redirect home. A silent
redirect home destroys analytics and confuses crawlers.

## Analytics

One tool, installed once, no tag manager for a marketing site. Pick the
privacy-respecting default (Plausible, Umami, or the platform's built-in
web analytics) unless the team already runs something. The whole install is
one script tag in the shared layout:

```html
<script defer data-domain="kelpline.app"
        src="https://plausible.io/js/script.js"></script>
```

Rules: load it `defer` so it never blocks first paint; exclude the 404 page
from goal tracking but not from page views (404 views are signal); never
drop a second analytics vendor in "temporarily"; and set up exactly one goal
(signup, purchase, or contact submit) at launch. A dashboard with traffic
and no goal is decoration.

Verify by opening the site with devtools network panel, confirming the
beacon fires once per navigation, and checking the dashboard within the
hour. "Installed" without a seen pageview is not verified.

## Worked example: Kelpline pricing page, full production head

```html
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Pricing: $29/mo flat, no per-seat | Kelpline</title>
  <meta name="description" content="One plan, $29 a month, unlimited reports and alerts. 14-day free trial, cancel in two clicks.">
  <link rel="canonical" href="https://kelpline.app/pricing">

  <meta property="og:type" content="website">
  <meta property="og:site_name" content="Kelpline">
  <meta property="og:title" content="Pricing: $29/mo flat, no per-seat">
  <meta property="og:description" content="One plan. Unlimited reports. 14-day free trial.">
  <meta property="og:url" content="https://kelpline.app/pricing">
  <meta property="og:image" content="https://kelpline.app/og/pricing.v2.png">
  <meta property="og:image:width" content="1200">
  <meta property="og:image:height" content="630">
  <meta name="twitter:card" content="summary_large_image">

  <link rel="icon" href="/icon.svg" type="image/svg+xml">
  <link rel="icon" href="/favicon.ico" sizes="32x32">
  <link rel="apple-touch-icon" href="/apple-touch-icon.png">
  <link rel="manifest" href="/manifest.webmanifest">

  <script defer data-domain="kelpline.app"
          src="https://plausible.io/js/script.js"></script>
</head>
```

Companion files ship in the same deploy: `/sitemap.xml` with `/`, `/pricing`,
`/docs` and real lastmod dates; `/robots.txt` allowing all and pointing at
the sitemap; `/404.html` with site nav, a "that page moved" line, and links
to home and pricing. The Higgsfield deploy separately carries
`app-meta.json` and the 3:2 marketplace cover per `deploy/higgsfield`; these
head tags cover everything off-platform.

## Checks

Seven binary checks, all must pass before deploy or domain hookup. Run each
against the deployed page source, not the local template.

1. Title and meta description exist, are unique to this route, and fit
   budget: title 30-60 characters, description 70-160. Neither is the site
   name alone.
2. Exactly one canonical per page, absolute URL, and `og:url` matches it
   character for character. Alternates (www, http, trailing slash) 301 to it.
3. `curl -sI` on every `og:image` returns 200 with an `image/*` content type,
   under 5 MB, at 1200x630, and the image passes one real scraper preview
   (X validator, LinkedIn inspector, or opengraph.xyz).
4. Favicon set serves: `/favicon.ico` and `/icon.svg` return 200, the SVG is
   viewBox-based (not base64 raster), and the icon renders visibly on both
   light and dark browser chrome.
5. `/sitemap.xml` is valid XML listing only indexable routes with real
   lastmod dates, `/robots.txt` allows crawling and points at the sitemap,
   and no URL appears in both the sitemap and a `noindex` header.
6. The 404 page returns real HTTP 404 (check with `curl -sI`), renders site
   nav with at least two escape links, and is absent from the sitemap.
7. Analytics beacon fires once per navigation on a live page and a pageview
   appears in the dashboard; at least one conversion goal is configured.

Reference each failing check by number in the fix commit.
