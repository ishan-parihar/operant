# Deploy to Higgsfield (website / app / game)

Deploy a finished website-design build to Higgsfield: pick --type website/app/game (user decides), scaffold with the higgsfield CLI into the app/ dir (React 19 + TanStack Start SSR on one Cloudflare Worker), honor the Animated/Non-animated intake for websites, set a DNS-safe subdomain (>4 chars, lowercase/digits/hyphen only), complete app-meta.json (og_title, og_description, marketplace_cover_url) during BUILD not publish, use simple-app turn economy (write once complete, batch), deploy and publish. Trigger: 'deploy to higgsfield', 'higgsfield website', 'publish the site', og/cover metadata at deploy.

The website-design pipeline ends here. Upstream leaves produced the brief,
direction, tokens, section map, code, content, and audit passes. This leaf
turns that work into a live Higgsfield deployment: one repository holding one
React 19 + TanStack Start SSR application inside `app/`, built and deployed as a
single Cloudflare Worker behind `<subdomain>.<host>`.

Sibling for non-Higgsfield hosts: [generic](generic.md). Gate that must pass
first: [preflight](../quality/preflight.md). Production metadata companion:
[production](../quality/production.md). Token and section-to-Worker mapping lives in
`references/higgsfield-adapter.md`.

## Where in the pipeline

This leaf represents the terminal stage of the end-to-end website-design
pipeline:

briefing -> direction -> system -> structure -> components -> motion ->
content -> quality/* -> **deploy/higgsfield**

The [redesign](../redesign.md) and [version-control](../version-control.md) leaves operate alongside as orthogonal
concerns throughout every phase. If the user is deploying to a generic static
host rather than the Higgsfield Cloudflare Worker runtime, route to
[generic](generic.md) instead.

## Prerequisites
Before running any deployment commands, verify the environment has the
required tools installed:

1. **Higgsfield CLI**: [higgsfield](higgsfield.md) binary must be available on `$PATH`.
   If missing, install via the official script:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/higgsfield-ai/cli/main/install.sh | bash
   ```
2. **Git**: repository must be initialized and version-controlled.
   [version-control](../version-control.md) runs alongside every phase; never scaffold or
   deploy inside an unversioned directory.
3. **Bun**: required package manager and runtime for building the Worker
   bundle and running local checks (`bun --version`).

## Decision: which --type?

The product type is the user's choice, never inferred silently. If unclear
from the prompt or brief, ask the user exactly once before scaffolding.

| User says | --type | What it is |
|---|---|---|
| landing page, portfolio, marketing site, SaaS site, company page, no AI generation | `website` | Standalone site, no Higgsfield integration, no AI generation, no Higgsfield branding on page |
| generates images/video/audio, Higgsfield AI models, credits, user creation history | `app` | Sign in with Higgsfield + fnf SDK, Quanta design system, full D1 database contract |
| play a game, single-player or multiplayer | `game` | Realtime multiplayer rooms, six pure functions in `app/src/logic.js`, `--category` genre |

Hard rules by product type:

- `website`: no AI image, video, audio, or text generation. No fnf SDK
  packages, no `@higgsfield/quanta/*` component imports, no "Powered by
  Higgsfield" badges. Third-party APIs (payments, analytics, maps, email)
  using the user's own API keys are allowed.
- `app`: full Quanta design system; use layouts Quanta lacks, never a
  competing third-party UI library. Quanta layouts and tokens are app-only;
  never port them into a `website` build.
- `game`: same unified CLI pipeline (`higgsfield create/deploy`, identical
  infrastructure). Genre selected via `--category` (arcade, puzzle, shooter,
  board, action, strategy); single-player games set `minPlayers: 1`.

Deep implementation flows for each type live in the source skill references
(`higgsfield-websites/references/{website,app,game}-flow.md`). This leaf owns
the deploy-side contract: intake, scaffold shape, subdomain validation,
metadata completion, turn economy, and publish gating.

## Intake for --type website: Animated vs Non-animated (ALWAYS)

Before scaffolding any `website` build, ask the user exactly once:

> **Animated** (recommended): a scroll-driven cinematic website featuring a
> continuous seam-locked camera journey or multi-scene narrative chain.
> **Non-animated**: a well-crafted static-feeling site with lighter,
> restrained micro-motion.

Intake rules:

- The question is MANDATORY for `--type website`. It is NEVER asked for
  `--type app` or `--type game`.
- Animated is the default recommendation. If the user does not respond or
  already specified animated/motion in the brief, select Animated.
- The intake answer sets the brief's `Animation mode` line:
  `animated-website` or `non-animated - <user's reason>`.
- An `animated-website` mode obligates writing the journey block (4-7 named
  scenes, camera architecture, seam direction, mobile framing) into the brief
  before any generation or scaffolding begins.
- A `non-animated` mode must carry the user's explicit reason verbatim.
- Your own aesthetic preference (such as "minimal B2B" or "clean SaaS") is
  never a valid reason to choose Non-animated. Only the user's pick is.

## The intake bundle (ask in ONE message)

When asking the type or animation question, bundle the publish question in the
same message:

> "Should this site be published to the public Higgsfield community feed
> (marketplace) upon completion, or remain private to your direct URL?"

- Yes: build full metadata and run `higgsfield website publish` after deploy.
- No / Unanswered: build full metadata, run deploy, stop before publish.
- Never stall the build waiting for publish intent; default to deploy-only.

## Subdomain rules (set at `higgsfield create` time)

Every project receives a subdomain passed to `higgsfield create --subdomain`.
The live production URL becomes `https://<subdomain>.<host>`.

Validation rules (all mandatory):

1. **Length**: strictly greater than 4 characters (`slug.length > 4`). Short
   words (such as `app`, `web`, `site`, `test`) are rejected by the registry.
2. **DNS-safe character set**: lowercase letters (`a-z`), numbers (`0-9`),
   and hyphens (`-`) only.
3. **No edge hyphens**: never start or end the slug with a hyphen (invalid:
   `-myproject`, `myproject-`).
4. **No forbidden characters**: no uppercase letters, no underscores (`_`),
   no periods (`.`), no spaces or symbols.
5. **Semantic derivation**: derive the slug directly from the project name
   or product identity (for example: `nordmark-studio`, `kelpline-metrics`).

If the user's proposed slug violates any rule, normalize or re-prompt
immediately rather than attempting a failing create command.

## Scaffold: website-design artifacts into the app/ directory

The entire application lives inside `app/`. Every artifact generated by the
website-design pipeline maps directly onto the React 19 + TanStack Start SSR
Worker architecture:

| website-design artifact | Where it lands in app/ |
|---|---|
| [system](../system.md) canonical tokens (`--bg`, `--text`, `--accent`, `--space-*`, `--radius`, font definitions) | CSS custom properties on `:root` inside `app/src/styles.css` or entry CSS, loaded across all routes |
| [structure](../structure.md) SECTION MAP | TanStack Start routes and section components in `app/src/routes/`, one layout family per section with no consecutive layout repeats |
| `components/*` recipes (heroes, features, social proof, conversion, forms, navigation) | React components in `app/src/components/`, styled to the locked palette and type pairing |
| [motion](../motion.md) bands and scroll patterns | GSAP and Lenis scroll controllers; `animated-website` runs the scene-locked scrub chain |
| [content](../content.md) copy deck, realistic data | Copy placed directly in JSX; media assets placed in `app/public/assets/` |
| [production](../quality/production.md) metadata (title, description, canonical, OG, favicon) | `app/src/app-meta.json` plus route head elements |
| [anti-slop](../quality/anti-slop.md) rules | All quality bans enforced: no em-dashes, no fake brand marks, no raw hex values outside token definitions |

Detailed token-to-Worker mapping lives in `references/higgsfield-adapter.md`.

## app-meta.json is a BUILD step, not publish-only

Every deployment must produce `app/src/app-meta.json` during the build phase.
There is NO simple-app exception. Never defer metadata to a future step.

```json
{
  "og_title": "Nordmark Studio - Architectural Photography & Spatial Design",
  "og_description": "Documenting built environments and brutalist spaces with honest light and medium format craft.",
  "favicon_url": "/assets/favicon.svg",
  "og_image_url": "/assets/og-cover.png",
  "marketplace_cover_url": "/assets/marketplace-cover.png"
}
```

Metadata requirements:

- `og_title`: 30-60 characters, includes brand name and core value statement.
- `og_description`: 70-160 characters, written in the site's brand voice.
- `marketplace_cover_url`: required on ALL builds (even deploy-only builds),
  ensuring the project is immediately publishable later without code edits.
- Assets: all referenced image paths must resolve to valid files inside
  `app/public/assets/`.
- Consistency: values must match the head tags verified in
  [production](../quality/production.md).

## Turn economy (simple-app discipline)

Agent round-trips consume context budget. Deploy workflows must adhere to
strict efficiency constraints:

1. **Write ONCE, complete**: author each file in full in a single write call.
   Avoid write-then-patch cycles. Never re-read a file immediately after
   writing it.
2. **Batch operations**: group multi-file additions or edits into parallel or
   sequenced batches without intermediate conversational turns.
3. **No re-exploration**: rely on the adapter documentation and standard
   scaffold structure rather than repeatedly inspecting the filesystem.
4. **No asset re-download**: once a generated asset or image is placed in
   `app/public/assets/`, verify its path once and proceed.
5. **Pipelined execution**: prepare route components and styling while asset
   generation or build checks run.

## CLI lifecycle and commands

Scaffold, build, deploy, and publish using the standard CLI commands from the
repository root:

```bash
# 1. Scaffold project into app/ directory
higgsfield create --type website --subdomain nordmark-studio

# 2. Build the Worker bundle
higgsfield build

# 3. Deploy live Worker to Cloudflare edge
higgsfield website deploy

# 4. Verify live deployment status and URL
higgsfield status

# 5. Publish to marketplace (ONLY if user explicitly requested)
higgsfield website publish
```

Operational notes:

- Talking to the user: speak in product terms ("Setting up your site…", "Saving your changes…", "Your live preview is ready: <url>", "Publishing your site…"), not plumbing ("cloning repo", "committing", "pushing", "deploying the branch", "build passed"). Do plumbing silently and report outcomes, not mechanics. Exception: user explicitly asks about repo/branch/deploy internals or is clearly technical.
- `higgsfield website deploy` (or `higgsfield deploy <id>`) immediately
  provisions the live public URL `<subdomain>.<host>`.
- `higgsfield website publish` lists the project on the community feed. This
  step is executed ONLY when the user gave explicit affirmative consent.
- Never reveal internal API keys, raw deployment tokens, or sensitive
  credentials in user-facing status messages.
- Use clean status updates: "Scaffolding app", "Building worker",
  "Deployed: https://<subdomain>.<host>", "Published to community feed".

## Worked example: complete deploy sequence

User prompt: "Deploy my architectural studio portfolio on Higgsfield. Use
subdomain nordmark-studio and publish it to the community feed."

1. **Intake analysis**:
   - Product type: portfolio site -> `--type website`.
   - Subdomain: `nordmark-studio` (15 characters, lowercase, digits, hyphens
     only -> valid).
   - Publish intent: yes (user explicitly stated "publish it").
   - Website intake question: ask Animated vs Non-animated. User selects
     Animated.
   - Record `Animation mode: animated-website` and 5-scene journey in brief.
2. **Prerequisites check**: `higgsfield --version`, `bun --version`, `git status`.
3. **Scaffold command**:
   ```bash
   higgsfield create --type website --subdomain nordmark-studio
   ```
4. **Asset and code port**:
   - Tokens mapped to `:root` in `app/src/styles.css`.
   - Routes created for Hero, Gallery, Case Studies, About, Contact.
   - Copy placed from content deck; images placed in `app/public/assets/`.
5. **Metadata creation (BUILD step)**:
   Write `app/src/app-meta.json` with complete `og_title`, `og_description`,
   `favicon_url`, `og_image_url`, and `marketplace_cover_url`.
6. **Build and verification**:
   ```bash
   higgsfield build
   ```
7. **Deployment**:
   ```bash
   higgsfield website deploy
   higgsfield status
   ```
   Live URL confirmed: `https://nordmark-studio.higgsfield.app`.
8. **Publish**:
   ```bash
   higgsfield website publish
   ```
9. **Final report**: state live URL, confirmed subdomain, and marketplace
   published status in concise summary.

## Checks (7)

All seven checks must pass before claiming deployment completion.

1. **Type recorded**: `--type` is explicitly confirmed as `website`, `app`,
   or `game` based on user requirements, never silently guessed or defaulted.
2. **Animation intake recorded**: for `--type website`, the brief contains a
   valid `Animation mode: animated-website` or `Animation mode: non-animated -
   <reason>` line based on the user intake question.
3. **Subdomain validated**: the selected subdomain is strictly longer than 4
   characters, contains only lowercase letters, numbers, and hyphens, and has
   no leading or trailing hyphens.
4. **App directory structure**: `app/` contains the complete port with tokens
   on `:root`, all SECTION MAP entries represented in routes, and assets in
   `app/public/assets/`.
5. **app-meta.json complete**: `app/src/app-meta.json` exists and contains
   non-empty `og_title`, `og_description`, and `marketplace_cover_url` fields
   created during the build step.
6. **Deploy verified**: `higgsfield status` reports a successful deployment
   and returns the active `<subdomain>.<host>` live URL.
7. **Publish matches consent**: `higgsfield website publish` was executed
   only if the user gave explicit affirmative consent; otherwise the project
   remains in deployed-only state.
