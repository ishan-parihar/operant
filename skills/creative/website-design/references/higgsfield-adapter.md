# Higgsfield adapter: website-design artifacts to a TanStack Start Worker

Use this file when the `DEPLOY SPEC` targets Higgsfield (`--type website`,
`app`, or `game`). The five website-design artifacts map one-to-one onto the
Higgsfield project shape. Full build lifecycle, deploy gates, and CLI details
live in the `higgsfield-websites` skill (`references/website-flow.md`,
`app-flow.md`, `game-flow.md`); this file is only the artifact mapping plus
the hard rules website-design output must respect on that platform.

## Artifact-to-file map

| website-design artifact | source skill | Higgsfield destination |
|---|---|---|
| `DESIGN BRIEF` | [briefing](briefing.md) | `app/design-brief.md`, Phase 0 concept: design read, concept spine, delivery tier, animation mode |
| `DIRECTION` | [direction](direction.md) | brief spine + tier fields; type scale and pairing consumed by `tokens.css` |
| `TOKENS` | [system](system.md) | `app/src/styles/tokens.css` (`:root` + dark block, exact canonical names) |
| `SECTION MAP` | [structure](structure.md), `components/*` | TanStack Start routes + one section component per map row |
| `DEPLOY SPEC` | deploy leaves | `app/app.manifest.json`, `app/src/app-meta.json`, `wrangler.jsonc`, deploy + publish runs |

The two worlds agree by construction: website-design produces tokens as CSS
custom properties and sections as semantic HTML, and a TanStack Start Worker
consumes exactly that; `references/stack-adapters.md` section 3 covers the
React/Next.js side of the same stack (Server Components, `"use client"`
isolation, `motion/react`).

## Project shape

Everything lives in `app/` at the project root:

```
app/
  package.json            own dependencies and scripts
  src/
    app-meta.json         page metadata, machine-editable (gate below)
    styles/tokens.css     TOKENS artifact, imported once in the root route
    routes/               TanStack Start file routes, SSR by default
    components/
      scroll-scrub/       for animated websites (scroll-scrub.tsx + .css)
      sections/           one component per SECTION MAP row
  migrations/             D1 schema when the manifest declares db
  app.manifest.json       deploy input: bindings and type
  wrangler.jsonc          Worker config, generated, do not hand-tune
```

All `bun` / build / deploy commands run from `app/`.

## SSR rules (site-breaking if violated)

1. React 19 + TanStack Start renders on the server. A top-level `window` or
   `document` reference crashes the build: the single most common failure.
   Browser-only code sits inside client components behind `useEffect` or a
   mounted guard, exactly as in `references/stack-adapters.md` section 3.
2. Scroll-reveal media the user plays with scroll (Tier-1 hero, scroll-scrub)
   is built in the main build phase, never deferred to "later." A static page
   with a deferred camera journey fails the platform's own review gate.
3. No `opacity: 0` elements parked for IntersectionObserver reveals: the
   headless screenshot audit must show every section. Text builds fire on
   mount; scroll-linked motion animates transform only.
4. GSAP pinned sections need `pinSpacing: false` or the pin spacer reads as a
   blank band in the full-page screenshot.
5. Every animated path has a `prefers-reduced-motion` static fallback.
6. `h-screen` and 100vh heroes break mobile viewport behavior; use the sticky
   and snap patterns from `references/motion/references/gsap-skeletons.md`.

## app.manifest.json and app-meta.json

`app/app.manifest.json` declares capabilities: `"db"`, `"r2"`, `"kv"` bindings
and the product `type`. Fields the manifest supports come from the
`higgsfield-websites` reference docs; add only bindings the app uses.
Secrets go through the CLI (`higgsfield secrets <website_id> --name X
--value Y`), never into source.

`app/src/app-meta.json` carries the DEPLOY SPEC metadata: title, description,
`og_title`, `og_image_url` (the generated cover), favicon. Hard gate: neither
`og_title` nor `og_image_url` may be empty when the site goes live. The cover
is generated with the Higgsfield asset pipeline at build time, not patched in
at publish.

## DIALS to delivery tier

| website-design dials | Higgsfield delivery tier |
|---|---|
| MOTION 1-3 | `editorial`: typography and image led, micro-motion only |
| MOTION 4-7 | `cinema`: Lenis + GSAP, scroll chapters |
| MOTION 8-10 | `cinema` (animated website, scroll-scrub journey) default |
| MOTION 9-10 plus brief says awwwards/3D/immersive | `spectacle`: cinema + WebGL or custom cursor second beat |

`Animation mode` on the brief is authoritative. VARIANCE spikes do not change
the tier on their own; they change hero and layout boldness inside it.

## Build and deploy sequence

1. Translate artifacts with the table above; write `app/design-brief.md`
   first, before code.
2. `bun run typecheck` from `app/` once; fix everything it finds.
3. `higgsfield website deploy <website_id>` ships the live public site. There
   is no preview stage.
4. `higgsfield website status <website_id>` returns the live URL
   (`https://<subdomain>...`) to report.
5. `higgsfield website publish <website_id>` only when publish was opted in;
   it lists the site on the community feed using the cover + app-meta.json.
6. Never screenshot or visually probe the live URL after deploy; the
   pre-deploy mechanical gate is the verification.

## Pass/fail checks

1. `grep -rn "window\.\|document\." app/src/routes app/src/components` -> every
   hit sits inside a `"use client"` client component with an effect or mount
   guard; 0 top-level references in server-rendered files.
2. `app/src/app-meta.json` has non-empty `og_title` and `og_image_url`.
3. `app/app.manifest.json` declares exactly the bindings the code uses, no
   extra `db`/`r2`/`kv`.
4. `bun run typecheck` in `app/` exits clean before any deploy run.
5. Tokens exist in exactly one file (`app/src/styles/tokens.css`) imported
   once; `grep -rEn '#[0-9a-fA-F]{6}' app/src --exclude='tokens.css'`
   returns 0 matches.
