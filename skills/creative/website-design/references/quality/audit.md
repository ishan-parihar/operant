# Audit: Squirrel 260-Rule Scan and Fix Loop

Run the squirrel audit against any URL: 260+ rules across SEO, performance, security, technical, content, and accessibility, then drive the iterative fix loop until the score clears its target. Run as the first step of redesigns (feeds briefing/), as the numeric gate inside quality/preflight/, or whenever the user says audit this site, check the score, or fix the findings.

The `squirrel audit <url> --format llm` report is the single source of truth for
site health. It is numeric (health score 0-100), grouped (issues by category,
affected URLs), and rerunnable, so every claim about the site's quality is a
before/after score pair instead of a vibe. Other quality leaves (accessibility,
performance, anti-slop, production) do the deep manual work inside their lanes;
this leaf owns the CLI report, the fix loop, and the score targets.

Requires the `squirrel` CLI (from squirrelscan.com). Check once with
`squirrel --version`. CLI setup, login, and publishing are out of scope here;
use the companion `squirrelscan` skill for those.

## Where this leaf runs in the pipeline

| Trigger | Who calls this leaf | What it contributes |
|---|---|---|
| Site already exists (preserve / overhaul) | `references/../briefing.md` step 2 | audit feeds the dials and foundation before design work starts |
| Numeric gate before delivery | [preflight](preflight.md) | the health score is the numeric gate value |
| Review-only request ("audit this site") | user directly | full scan + fix loop, no design work |
| After a fix batch | this leaf's own loop step 5 | re-audit with `--refresh`, compare with `--diff` |

Cross-links: [accessibility](accessibility.md) for the manual WCAG lanes behind the
accessibility findings, [performance](performance.md) for Core Web Vitals depth,
[anti-slop](anti-slop.md) for AI-tell issues the crawler can flag, and
[production](production.md) for meta, OG, favicon, sitemap, and 404 rules.

## Procedure

1. Get the target URL. If the user did not name one, audit the running dev
   server (the only URL that reflects code changes; never audit production
   to verify uncommitted work). Local servers audit as
   `squirrel audit http://localhost:3000 --format llm`.
2. Run the first scan in `quick` mode (25 pages). Read the report.
3. If any category is flagged, extend coverage to `surface` (100 pages) or
   `full` (500 pages) using the mode table below.
4. Propose the fix list: group the report's issues, ordered by severity.
5. Map each issue to its source file: rule docs URL (table below) gives the
   category, the affected URL gives the page, the page template gives the file.
6. Fix in batches after the user approves. Group same-file fixes so one edit
   clears many rules.
7. Re-audit with `--refresh` (cache bust, re-fetch) and compare against the
   baseline with `report --diff`.
8. Repeat steps 4-7 until the score clears the target from the table below or
   the remaining issues are explicitly out of scope.

Do not continue past a failing score without reporting the block. The loop
stops when the target is reached, not when the agent runs out of patience.

Prefer auditing the live site over a static dump: the crawler renders pages,
follows redirects, and measures performance, so a dev server or deployed
preview is the right target. A `file://` path or raw HTML misses rendering,
performance, and redirect rules.

Template-level sign-off needs a scan mode that covers every route in the
template family, not just the one page the user linked to.

## Scan modes and progression

| Mode | Pages | Use when |
|---|---|---|
| `quick` | 25 | First look, CI checks, verifying one fix batch |
| `surface` | 100 | Template-level coverage (e.g. every `/blog/{slug}` pattern) |
| `full` | 500 | Pre-launch sign-off, deep audits, sitemap-complete runs |

Progression rule: start `quick`. Escalate to `surface` when a template family
appears broken (same rule flagged on many URLs of one pattern). Escalate to
`full` only for a launch gate. Re-verify a single fix batch at `quick`, never
at the level the fix was found at.

Useful flags: `--refresh` (bypass cache and re-fetch, required for re-audits
after code changes), `--resume` (continue an interrupted crawl), `-m <n>`
(page limit override), `--verbose` (crawl progress in output).

Blocked crawls: some platforms (Shopify, Cloudflare) block unknown crawlers.
Retry with Web Bot Auth headers (`-H "Name: value"`, see
docs.squirrelscan.com/guides/web-bot-auth). Never put secrets in header flags;
the CLI redacts them in output, but the shell history keeps them.

## Rule documentation URL

Every rule in the report maps to docs at:

`https://docs.squirrelscan.com/rules/{category}/{id}`

For example, a `links/external-links` finding is documented at
https://docs.squirrelscan.com/rules/links/external-links. Use the docs page to
understand the rule before fixing it; guessing at the rule's intent produces
fixes that fail the re-audit.

## What the 260+ rules cover

The scanner runs 260+ rules in 15+ categories. Map each category to its fix
owner so findings land in the right quality leaf:

| Category | Typical findings | Deep-work owner |
|---|---|---|
| seo | missing meta description, duplicate titles, missing canonical, heading structure | [production](production.md) |
| links | broken internal/external links, redirect chains, orphan pages | this leaf (fix list, re-audit) |
| performance | unoptimized images, render-blocking resources, font loading, cache headers | [performance](performance.md) |
| accessibility | contrast, missing alt text, form labels, focus states | [accessibility](accessibility.md) |
| content | thin pages, duplicate content, missing language declarations | `references/../content.md` |
| technical | invalid HTML, mixed content, sitemap/robots conflicts | this leaf plus [production](production.md) |
| security | leaked secrets (96 patterns: OpenAI, AWS, Stripe), missing headers | this leaf, fix immediately, rotate the secret |
| mobile | viewport issues, tap targets, horizontal overflow | [accessibility](accessibility.md) touch lanes |
| social | missing OG/Twitter meta | [production](production.md) |
| best-practices | console errors, deprecated APIs, image aspect ratios | [performance](performance.md) |

Secret rule: a leaked secret is the one finding that never waits for a batch.
Fix and rotate the credential before anything else in the loop.

## The LLM report

Request reports with `--format llm`. The output is a compact text hybrid
optimized for agents: health score, grade, issues grouped by category with
severity, affected URLs per issue, broken links, and prioritized
recommendations. Full spec: `references/OUTPUT-FORMAT.md` in the squirrelscan
skill.

Reading order inside a report:

1. Health score and grade first: is the site passing at all?
2. Issues by severity within category: what breaks the score most?
3. Broken links: cheap to fix, count-heavy in the report.
4. The recommendation list: what to fix in what order.

Map recommendations, not raw findings, to the fix list when the two differ;
the recommendations are severity-ranked by the scanner and usually correct.

Output formats beyond `llm` (text, json, markdown, html) exist for humans and
CI; agents always pass `--format llm`.

## The fix loop

| Step | Action | Evidence kept |
|---|---|---|
| 1 | Read score, grade, issues by severity | baseline audit ID |
| 2 | Propose the fix list to the user | issue-to-file map |
| 3 | Map issues to source files via rule docs + affected URL | fix list |
| 4 | Apply approved fixes in batches | commit per batch |
| 5 | Re-audit `--refresh` | new audit ID |
| 6 | Compare `report --diff <baseline-id>` | delta report |
| 7 | Report the delta, propose the next batch | loop continues |

Batch rule: one commit per fix batch, with the rule IDs fixed in the commit
message. Never mix unrelated fixes into one batch; the diff comparison must be
able to attribute a score change to a batch.

Severity triage when the report has many issues:

| Severity | Action | Why |
|---|---|---|
| Critical / error | Fix first, before any polish | blocks the score target and often blocks users |
| Warning | Fix in the same batch as same-file criticals | cheap score points when the file is already open |
| Info / suggestion | Fix only if the target row requires it | polishing info items before the target is met is wasted turns |

Stop condition: target reached, or the user explicitly scopes out the
remaining categories. Never stop because "most issues are fixed".

Fix-loop failure modes (flag these, do not paper over them):

| Symptom | Cause | Action |
|---|---|---|
| Score plateaus across 2 re-audits | False source-file map or stale docs | Re-read rule docs, re-map files |
| Score drops after a fix batch | Fix introduced a new violation | Roll back the batch, re-audit |
| Re-audit shows the same IDs | The fix never shipped or cache hit | Add `--refresh`, verify deploy |
| Fails 3+ consecutive target rows | Target row wrong for the codebase | Drop back one row, proceed |

## Score targets by starting score

| Starting score | Target | Expected work |
|---|---|---|
| under 50 (F) | 75+ (C) | Major structural fixes first |
| 50-70 (D) | 85+ (B) | Moderate fixes, some template rework |
| 70-85 (C) | 90+ (A-) | Polish: meta, contrast, link hygiene |
| 85+ (B+) | 95+ (A) | Fine-tuning, single-digit issue counts |

Pick the row from the first `quick` scan's score. Do not re-target mid-loop
except upward (user asks for a higher grade). A score that plateaus for two
consecutive re-audits means the remaining issues need a different fix
approach: re-read the rule docs and re-map the source files before the next
batch.

## Regression checks

Once a baseline exists, every later change is checked against it so fixed
issues stay fixed.

```
squirrel report --diff <baseline-audit-id> --format llm
squirrel report --regression-since example.com --format llm
```

Use `--diff` inside the fix loop (attribute the delta to the batch). Use
`--regression-since` before delivery to prove nothing that passed at baseline
now fails. A regression finding blocks delivery until fixed or explicitly
waived by the user.

Re-render any stored report without re-crawling when its content is no
longer in view: `squirrel report <audit-id> --format llm`.

Regression rule: an improvement is only real if the diff shows the specific
issue IDs moving from failed to passed. A score increase with the same issue
IDs still failing means the score moved for the wrong reason; investigate
before declaring victory.

Stale audit shelf life: an audit older than the last code change is not
evidence. Always re-run with `--refresh` after edits; never quote an old
score for new code.

## Worked example

User: "audit http://localhost:4321 and get it to at least 85."

1. Baseline: `squirrel audit http://localhost:4321 --format llm` (quick, 25
   pages). Score 72 (D), grade C. Findings: 6 broken links in `links/`,
   3 missing meta descriptions in `seo/`, 2 contrast warnings in
   `accessibility/`, 1 duplicate-title error in `seo/`.
2. Target row: starting 50-70 means target 85+ (B). The score is 72, still
   inside that row.
3. Fix list, ordered: duplicate-title error first, then the 6 broken links
   (one batch, one commit: `fix(audit): seo/title duplicate, 6 links`),
   then meta descriptions, then contrast (contrast pairs verified with
   `scripts/contrast.py` per [accessibility](accessibility.md)).
4. Re-audit: `squirrel audit http://localhost:4321 --format llm --refresh`.
   Score 88.
5. Diff: `squirrel report --diff <baseline-id> --format llm`. Confirms all 12
   issue IDs moved to passed.
6. Report: before 72, after 88, 1 batch, 0 remaining issues. Target 85
   cleared. Hand the score to [preflight](preflight.md).

If step 4 had returned 74 instead: two flat re-audits means re-read the rule
docs for the remaining findings and re-map source files before the next batch,
not another blind fix round.

## Handoff

When the loop finishes: report the before/after score pair, the batches
applied, and any remaining issues with their severity. If the user wants the
results shared or published, use the companion `squirrelscan` skill (published
reports). Feed the final score into [preflight](preflight.md) as the numeric
gate value; preflight fails below its threshold and this leaf is the source
of that number.

## Audit-first mode (redesign entry)

When called from [redesign](../redesign.md) (preserve or overhaul), the audit
runs BEFORE any design work and its output shapes the redesign:

| Audit output | How it feeds the redesign |
|---|---|
| Health score and grade | baseline to beat; the redesign must not drop it |
| Category breakdown | which categories are weak picks where modernisation effort goes |
| Regressions since last audit | what previous changes already broke, do not repeat |
| Broken-link list | content inventory gaps, feeds the content plan |
| Secret findings | rotate before any redesign work starts |

The redesign's own re-audit after build uses the same loop above, with the
original audit as baseline: `squirrel report --diff <pre-redesign-id>
--format llm` must show no category regressed and the target row met.

## Checks

Seven mechanical checks. Each verifies in one step.

1. Report exists: a `squirrel audit` run produced an audit ID and health
   score for the target URL. No report means the leaf did not run.
2. Coverage matches intent: final re-audit ran at `quick` for fix
   verification, `surface`+ for template claims, `full` for a launch gate. A
   launch claim from a 25-page scan fails.
3. Re-audit used `--refresh`: the post-fix audit ID differs from the baseline
   ID and the run command contains `--refresh`. Comparing without re-fetch
   fails.
4. Diff compared: `report --diff` output exists showing per-category deltas
   between baseline and final audit. No diff means the score change is
   unattributed.
5. Fixes are attributed: each fix batch has one commit naming the rule IDs
   it clears. An uncommitted fix or a mixed-purpose commit fails.
6. Target cleared or scoped out: final score meets the target row for the
   starting score, OR the remaining issues are listed with the user's
   explicit scope-out. A silent stop below target fails.
7. No regressions: `--regression-since` (or the final diff) shows no issue
   that passed at baseline failing in the final audit. One new failure fails
   the check.
