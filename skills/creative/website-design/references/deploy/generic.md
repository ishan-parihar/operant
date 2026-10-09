# Deploy: Generic Static Hosting

Deploy the finished site to a generic static host: Vercel, Netlify, Cloudflare Pages, or a plain static directory on any server. Covers the build check, asset optimization, env vars, preview URL, production deploy, and rollback, adapter-agnostic and stack-agnostic. Semantic HTML plus CSS custom properties compile to any stack via references/stack-adapters.md. Run ONLY after quality/preflight passes. For Higgsfield hosting read [higgsfield](higgsfield.md) instead.

You are deploying a finished website RIGHT NOW. The site has passed
[preflight](../quality/preflight.md). This leaf ships it with zero framework
ceremony: every host in the tables below serves the same two things, static
files or a build that emits static files. Output is one artifact, the
`DEPLOY SPEC` fenced block, plus a live URL and a written rollback path.

Language-agnostic rule: nothing here depends on React, Tailwind, or any
framework. What you ship is semantic HTML plus CSS custom properties (the
canonical tokens from [system](../system.md)). If a framework is present, its
adapter entry in `references/stack-adapters.md` is the only source of truth
for the build command and the output dir. Never guess either.

## Procedure

0. Confirm the gate: preflight passed, working tree clean. Never deploy a
   site that failed preflight or has uncommitted changes. Commit discipline
   for the end of this phase comes from [version-control](../version-control.md).
1. Pick the target host (Section 1).
2. Authenticate once if needed (Section 2).
3. Detect the build shape, then run the build check (Section 3).
4. Optimize assets (Section 4).
5. Set env vars (Section 5).
6. Deploy to preview and verify the preview URL (Section 6).
7. Deploy to production and verify (Section 7).
8. Write the rollback path (Section 8).
9. Commit and emit the `DEPLOY SPEC` block (Section 9).

## 1. Pick the host

Pick in this order: the user named a host, the repo is already connected to
a host, otherwise the first row you can use. Never ask which host; pick and
say so in one line.

| Host | Pick when | CDN/caching |
|---|---|---|
| Vercel | user named it, or repo already connected | automatic |
| Netlify | user named it, or redirects/forms needed | automatic |
| Cloudflare Pages | user named it, or edge logic wanted | automatic |
| Direct static | no account, no login possible, an existing server, or "just give me the files" | you configure it (Section 4) |

Default rule: if no host is named and no account exists, deploy direct
static (rsync, or push to a branch the server serves). Why: zero signup,
zero tokens, works from any machine. Override: the user names any host and
that host wins.

Custom domains: the host dashboard sets the domain and prints the DNS
records. Never block a deploy on domain propagation; verify with the
temporary host URL and report the domain as pending.

## 2. One-time auth (skip if already logged in)

| Host | Login | Link the project | CLI missing |
|---|---|---|---|
| Vercel | `vercel login` | `vercel link` | `npx vercel ...` |
| Netlify | `netlify login` | `netlify init` | `npx netlify-cli ...` |
| Cloudflare Pages | `wrangler login` | none needed for direct deploy | `npx wrangler ...` |
| Direct static | SSH key or push access already in place | none | n/a |

Never install a CLI globally for a one-off deploy; run it through npx.
Repeat deploys skip this section entirely.

## 3. Detect the build shape, then run the build check

| What the repo contains | Build command | Output dir | Install first |
|---|---|---|---|
| Plain .html/.css/.js at the root | none | site root | no |
| package.json with Vite | `npm run build` | `dist/` | yes |
| package.json with Astro | `npm run build` | `dist/` | yes |
| Next.js with static export | `npm run build` | `out/` | yes |
| SvelteKit with the static adapter | `npm run build` | `build/` | yes |
| Anything else | the mapping in `references/stack-adapters.md` | as given there | as given there |

Build check, run BEFORE any deploy. A broken local build ships broken, and
the host will not save you.

1. If install is needed, run the install command. Fail: report and stop.
2. Run the build command. Fail: report and stop with the error, do not retry
   blindly.
3. Verify the output: `<output>/index.html` exists and is non-empty.
4. Scan the build output for missing-asset warnings (unresolved imports, 404
   asset paths). Any such warning: fix in source, rebuild. Never deploy a
   build that warned about missing files.

## 4. Asset optimization

Do this once per build, mechanically. Order: images, fonts, CSS/JS, cache.
Most rules below are already enforced upstream ([performance](../quality/performance.md),
[content](../content.md)); this section is the last chance to catch them before the files
leave the machine.

| Asset | Rule | Exact value |
|---|---|---|
| Images | modern format | WebP or AVIF; JPEG/PNG only where compat or transparency forces it |
| Images | lazy-load | `loading="lazy"` on every image below the fold; hero image excluded |
| Images | dimension hints | `width` and `height` attributes on every `<img>` to prevent layout shift |
| Fonts | preload only the display face | one `<link rel="preload">` for `--font-display`; never preload body or mono |
| Fonts | swap | `font-display: swap` in every `@font-face` |
| CSS and JS | minified | the build tool does it; a plain static site skips this (no transform step) |
| CSS and JS | hashed filenames | the build tool does it; never hand-rename cache-busting names |

Cache headers, direct static only (the hosted platforms set these for you):

| File pattern | Cache-Control |
|---|---|
| hashed assets (e.g. `app-a1b2c3.js`, `site-9f8e7d.css`) | `public, max-age=31536000, immutable` |
| `index.html` and all other HTML | `no-cache` (or `max-age=0, must-revalidate`) |
| `favicon.ico`, `robots.txt`, `sitemap.xml` | `public, max-age=86400` |

Apply them in the server config (nginx `location` blocks) or a `_headers`
file if the static host reads one. Why: immutable hashed assets make
rollbacks and repeat visits instant; un-cached HTML makes new deploys
visible immediately.

## 5. Env vars

Static sites have no server runtime. Anything inlined at build time is
public in the shipped files. A server-only secret does not exist here.

| Var kind | Rule | Where it lives |
|---|---|---|
| Public build-time values (API base URL, site URL) | use the stack's public prefix, inlined at build | host dashboard or CLI env command |
| Secrets (API keys, tokens) | NEVER referenced from static code; the build ships them publicly | nowhere; if a secret is needed the design is wrong, escalate |
| `.env` | never committed, never inside the output dir | repo only, gitignored |

Public prefixes per stack:

| Stack | Public prefix |
|---|---|
| Vite | `VITE_` |
| Next.js | `NEXT_PUBLIC_` |
| Astro | `PUBLIC_` |
| Plain static | none; pass values in a tiny `config.js` or inline at build |

Greppable gate before deploy: search the output dir for secret-shaped
strings, e.g. `grep -ri "sk_live\|api_key\|secret\|BEGIN PRIVATE" <output>/`.
It must return nothing. Any match: remove the secret from source, rebuild,
re-grep.

## 6. Preview URL

Deploy to a preview BEFORE production. The preview catches broken paths,
missing assets, and console errors without touching the live URL.

| Host | Preview command |
|---|---|
| Vercel | `vercel` (no `--prod`) |
| Netlify | `netlify deploy` (no `--prod`) |
| Cloudflare Pages | `wrangler pages deploy <output>` |
| Direct static | none; no preview stage on a bare server, go to Section 7 |

Verify the preview mechanically, not by eye:

1. `curl -s -o /dev/null -w "%{http_code}" <preview-url>/` returns `200`.
2. The fetched HTML contains the site title from the DESIGN BRIEF:
   `curl -s <preview-url>/ | grep -o "<title>[^<]*</title>"`.
3. No failed requests: if a render-check tool is available
   ([preflight](../quality/preflight.md) lists one), run it against the preview URL
   and read the console output. One console error about a 404 font or image
   means a path typo; fix in source, rebuild, redeploy the preview.

## 7. Production deploy

Run only after the preview verification passes.

| Host | Production command |
|---|---|
| Vercel | `vercel --prod` |
| Netlify | `netlify deploy --prod` |
| Cloudflare Pages | `wrangler pages deploy <output> --branch main` |
| Direct static | `rsync -avz --delete <output>/ user@host:/srv/www/<site>/releases/$(date +%Y%m%d-%H%M%S)/` then flip the symlink (Section 8) |

Verify production mechanically, same checks as preview:

1. Production URL returns `200`.
2. Fetched HTML contains the correct `<title>`.
3. Favicon returns 200: `curl -s -o /dev/null -w "%{http_code}" <prod-url>/favicon.ico`.
4. One hashed asset returns 200: copy its path out of the built HTML and
   curl it. A 404 here means the upload was partial or the cache headers
   are stale; re-upload before continuing.

404 page, robots.txt, sitemap.xml: launching them is owned by
[production](../quality/production.md). Deploy only verifies they shipped (a 404 on
`/404.html` is itself a deploy bug on direct static).

## 8. Rollback

Write the rollback path before you finish. If production breaks later, this
is the one-step recovery.

| Host | Rollback action |
|---|---|
| Vercel | dashboard: Deployments, pick the previous one, Promote; or `vercel rollback <deployment-url>` |
| Netlify | dashboard: Deploys, pick the previous one, Publish |
| Cloudflare Pages | dashboard: the deployment list, Rollback on the previous build |
| Direct static | point the symlink at the previous release dir |

Direct static, exact recipe (release dirs plus one symlink the server
serves):

```bash
# every deploy: new release dir, then flip the symlink
rsync -avz --delete <output>/ user@host:/srv/www/<site>/releases/<stamp>/
ssh user@host "ln -sfn /srv/www/<site>/releases/<stamp> /srv/www/<site>/current"

# rollback: flip the symlink back to the previous release
ssh user@host "ln -sfn /srv/www/<site>/releases/<prev-stamp> /srv/www/<site>/current"
```

Keep the last 3 release dirs (`ls -t .../releases | tail -n +4` are safe to
delete). Why 3: enough history to undo a bad deploy and the one before it,
small enough that disk never becomes the incident.

## 9. Commit and the DEPLOY SPEC

Commit at phase end per [version-control](../version-control.md)
(`web vc --site <site> commit --reason "deploy: <host>"`, raw git fallback
if the CLI is unavailable), then emit the block every later phase re-reads:

```
DEPLOY SPEC
host:        <vercel | netlify | cloudflare-pages | direct-static>
build:       <build command or "none">
output_dir:  <dir>
env_vars:    <count> public, 0 secrets
preview:     <preview URL or "skipped (direct static)">
production:  <live URL>
rollback:    <one host-specific action>
status:      live
```

## Worked example

Deploying a Vite-built landing page to Vercel, first time:

1. Host: user said "Vercel". Auth: `vercel login`, `vercel link` once.
2. Build shape: package.json with Vite, so `npm run build`, output `dist/`,
   install first. Build check: build succeeds, `dist/index.html` exists, no
   missing-asset warnings.
3. Assets: hero is WebP with width/height set, below-fold images have
   `loading="lazy"`, only `--font-display` is preloaded, `@font-face` rules
   use `font-display: swap`.
4. Env: one public var `VITE_API_URL` added via `vercel env add`. Secret
   grep on `dist/` returns nothing.
5. Preview: `vercel` prints `https://<site>-abc123.vercel.app`. Curl gives
   200, title matches the brief. Render check shows one console error, a
   404 on `/fonts/display.woff2`. Path typo in source; fix, rebuild,
   redeploy preview, console clean.
6. Production: `vercel --prod`. Curl 200, title correct, favicon 200, one
   hashed asset 200.
7. Rollback: previous deployment is promotable from the Vercel dashboard.
8. Commit: `web vc --site <site> commit --reason "deploy: vercel"`. DEPLOY
   SPEC emitted with the live URL.

## Checks

1. [ ] Build output exists: `<output>/index.html` is present and non-empty (one `ls`).
2. [ ] No secrets shipped: the secret grep on the output dir returns nothing (one grep).
3. [ ] Preview verified before production: preview URL returned 200, or "direct static" is stated (one curl).
4. [ ] Production verified: URL returns 200 and the fetched HTML contains the correct `<title>` (one curl plus grep).
5. [ ] Rollback path written: exactly one host-specific rollback action sits in the DEPLOY SPEC (countable).
6. [ ] Phase committed: `git status` is clean after this phase and the DEPLOY SPEC was emitted (one git status).
