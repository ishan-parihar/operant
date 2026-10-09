# Quality: Review + Gate Router

Review and gate router for website-design. Routes review, audit, accessibility, performance, SEO/production, and preflight checks to the six quality leaves. Use for any review-only task (no new build) and as the mandatory gate before shipping any page.

This node unifies two models of quality work and routes both to the six leaves
under [quality](quality.md). It contains no checks of its own. Every mechanical rule,
ban, threshold, and procedure lives in a leaf. Read this file only to decide
WHICH leaf to open, then descend.

## The two models

**Review** answers "is this page good?" for work that already exists. Entry
points: user asks for a review, an audit, an accessibility check, a speed
check, or a launch-readiness check on a built page or live URL. Review work
produces findings and fixes in place; it does not re-enter the build pipeline.

**Gate** answers "may this page ship?" at the end of the build pipeline. After
[content](content.md) and [motion](motion.md), every build MUST pass through [preflight](quality/preflight.md)
before [deploy](deploy.md). The gate is not optional and not skippable for "small"
pages; preflight is fast precisely because the leaves were consulted during
the build.

## Routing

Match the request to exactly one leaf and descend. If several match, run them
in the preflight ordering (anti-slop first, see [preflight](quality/preflight.md)).

| Trigger in the request | Descend to |
|---|---|
| Looks AI-generated, templated, generic; purple gradients, Inter-on-slate, three equal cards | [anti-slop](quality/anti-slop.md) |
| WCAG, contrast, keyboard, focus, screen reader, ARIA, a11y | [accessibility](quality/accessibility.md) |
| Slow, Lighthouse, Core Web Vitals, LCP, CLS, INP, image weight, font loading | [performance](quality/performance.md) |
| squirrel audit, SEO scan, 260+ rules, fullsite report, fix-and-re-audit loop | [audit](quality/audit.md) |
| Meta tags, Open Graph, favicon, 404, sitemap, analytics, launch checklist | [production](quality/production.md) |
| Final check, ready to ship, pre-launch, handoff, "are we done" | [preflight](quality/preflight.md) |

Ambiguous broad request ("check my site", "is this good") with no specific
axis: descend to [audit](quality/audit.md) for a measured baseline, or to
[preflight](quality/preflight.md) if the site is about to ship.

## Leaves

- [anti-slop](quality/anti-slop.md) owns ALL absolute bans and forbidden patterns for the
  entire website-design tree. It ships `scripts/detect.sh`, a runnable
  17-check detector. Siblings link to it; they never restate ban lists. If a
  rule feels absolute ("never X", "always Y"), it belongs here, not in
  another leaf.
- [accessibility](quality/accessibility.md) runs WCAG checks: contrast ratios, keyboard
  traversal, focus visibility, ARIA roles, screen-reader paths.
- [performance](quality/performance.md) enforces Core Web Vitals thresholds and the image,
  font, and animation budgets that keep them green.
- [audit](quality/audit.md) drives `squirrel audit <url> --format llm` (260+ rules,
  15+ categories): map each finding to a source file, fix in batches,
  re-audit with `--refresh`, compare with `--diff`, loop until clean.
- [production](quality/production.md) covers SEO meta, Open Graph cards, favicons, 404
  pages, sitemap.xml, analytics, and the launch checklist.
- [preflight](quality/preflight.md) is the final gate: one ordered sweep (anti-slop, then
  accessibility, performance, SEO meta, OG, favicon, 404/sitemap, render
  fallback), one batched fix pass, re-audit until every check passes. It
  sequences the other leaves; it does not duplicate their depth.

## Review-only tasks

A review-only task touches no pipeline artifacts (no DESIGN BRIEF, DIRECTION,
TOKENS, SECTION MAP is created or edited). Procedure:

1. Route by the table above and descend directly to the leaf.
2. Run the leaf's checks against the page or URL as-is.
3. Fix findings in the source files, in the leaf's own order.
4. Re-run the leaf's verification (detect.sh, squirrel, Lighthouse, or its
   `## Checks` list) until green.
5. Stop. Do not redesign, reskin, or re-plan. Scope creep beyond the routed
   leaf requires an explicit new request.

## Gate tasks (in-pipeline)

During a build, each leaf is consulted at its natural phase (anti-slop at
style decisions, accessibility and performance while building components,
production before deploy). Then [preflight](quality/preflight.md) runs once, last, before
[deploy](deploy.md). A page that has not passed preflight has not finished the
pipeline, regardless of how complete the code looks.

## Anti-duplication contract

The six leaves compose by reference, never by repetition:

- Absolute bans live ONLY in [anti-slop](quality/anti-slop.md).
- Ordered cross-leaf sequencing lives ONLY in [preflight](quality/preflight.md).
- Squirrel tooling commands live ONLY in [audit](quality/audit.md).

When two leaves seem to overlap (example: contrast appears in both
accessibility and audit output), the owning leaf's procedure runs and the
other consumes its result.

## Checks

1. Every review/audit/performance/accessibility/SEO/preflight request is
   routed to exactly one leaf before any check runs.
2. No ban, threshold, or rule is stated in this router; open the owning leaf.
3. Review-only tasks create no pipeline artifacts.
4. No build reaches [deploy](deploy.md) without [preflight](quality/preflight.md) passing.
5. Any new absolute ban is added to [anti-slop](quality/anti-slop.md), not to a sibling.
