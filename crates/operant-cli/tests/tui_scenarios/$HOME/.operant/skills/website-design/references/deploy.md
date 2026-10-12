# Deploy: Router

Deploy a finished website-design build: route to Higgsfield Worker (website/app/game) or generic static host (Vercel, Netlify, Cloudflare Pages, direct static). Triggers: deploy, publish, go live, ship site, higgsfield.

Gate: [preflight](quality/preflight.md) green. Never deploy a build that failed preflight or sits in an uncommitted working tree; commit discipline lives in [version-control](version-control.md).

This node routes only. It holds no deploy procedure itself; the child below matching the user's target does.

## Routing

One decision: where does the site run?

| Condition | Load |
|---|---|
| User said Higgsfield, or wants a Worker SSR app, or said `website`/`app`/`game` type | [higgsfield](deploy/higgsfield.md) |
| Vercel, Netlify, Cloudflare Pages, or a plain static directory on a server | [generic](deploy/generic.md) |
| Nothing stated | Read the design context: a Higgsfield account or an existing `app/` scaffold means higgsfield; otherwise ask one line, then route |

Never pick both, never invent a third host. If the user names a host not in the table, treat it as [generic](deploy/generic.md) unless it is Higgsfield.

## Children

- **higgsfield/** - three product types (`--type website|app|game`) on one Cloudflare Worker, React 19 + TanStack Start SSR inside `app/`, DNS-safe subdomain rules, `app-meta.json` (og_title, og_description, marketplace_cover_url) as a BUILD step not a publish step, animated vs non-animated mode, publish vs deploy-only intent.
- **generic/** - adapter-agnostic static deploy to Vercel, Netlify, Cloudflare Pages, or direct static rsync: host pick, one-time auth, build check, perf/secret sweeps, preview then production, rollback, via `references/stack-adapters.md`.

## Shared output: DEPLOY SPEC

Whichever child runs, the emitted artifact is one fenced `DEPLOY SPEC` block: host (with `--type` when Higgsfield), build command, output dir, preview URL, production URL, rollback action, status. Upstream phases ([briefing](briefing.md), [direction](direction.md), ... ) each emit a fenced block; deploy ends the chain with this one.

## Turn economy (Higgsfield only)

Higgsfield builds consume turn budget per round-trip. When the route is [higgsfield](deploy/higgsfield.md): ask the type, animation mode, subdomain, and publish intent ONCE in a single intake message, batch file edits parallel, never re-read files written this session, never re-download assets already in `app/public/assets/`. Generic deploys have no equivalent constraint; do not apply this discipline there.

## Checks

- [ ] Exactly one child loaded, matching the routing table (name it).
- [ ] Preflight re-run green immediately before deploy (pass/fail).
- [ ] DEPLOY SPEC block emitted with live production URL (present/absent).
- [ ] Higgsfield route: `app-meta.json` complete before any publish (file exists, three OG fields non-empty).
