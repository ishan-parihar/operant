---
name: readme-quality-engineering
description: Upgrade READMEs to readme-craft bar via rubric + GH API.
category: software-development
metadata:
  operant:
    tags: [readme, documentation, quality]
---

# README Quality Engineering

**Class-level skill for auditing, upgrading, and maintaining GitHub READMEs to a defined quality bar using the readme-craft methodology.**

## Scope
- Audit existing READMEs against a measurable rubric
- Generate upgraded READMEs following readme-craft structure (Value→Proof→Mechanism→First Use→Detail)
- Deploy README-only updates via GitHub API (no repo clones, VPS-space-safe)
- Maintain consistency across a portfolio of repositories

## Quality Bar (8 Checks)

Every README must pass all 8 checks:

| Check | Criteria |
|-------|----------|
| **Value proposition** | First paragraph clearly states what the project does and for whom |
| **Proof before architecture** | Visual evidence (screenshots, diagrams, output) appears before or alongside architecture section |
| **Visual proof depth** | ≥3 images total: hero + visual proof gallery (tables of screenshots) |
| **Install-first structure** | Quick Start/Installation appears before Architecture/Advanced config |
| **Code examples** | ≥2 runnable code blocks (install, usage, MCP config, etc.) |
| **Structured tables** | Tables for features, tools, config, providers, comparisons |
| **Badges for signal** | 3-8 badges (language, license, protocol, domain-specific) |
| **Condensed length** | 100-170 lines, no bloat — detailed content moved to `DOCS.md`, `TOOLS.md`, `ARCHITECTURE.md` |

## Audit Rubric (Scoring)

```python
# Each check: +1 strength if pass, -1 issue if fail
# Target score: 8/8 (0 issues)
checks = [
    "value_prop_clear",
    "proof_before_arch",
    "visual_proof_3plus",
    "install_before_arch",
    "code_examples_2plus",
    "tables_for_structure",
    "badges_3_to_8",
    "length_100_170"
]
```

## Visual Proof Patterns

### Hero Section (first 500 chars)
```
# Project Name

**One-line promise — key differentiators, tech stack, outcome.**

![Hero visual](https://github.com/user/repo/raw/main/assets/readme/hero.png)
```

### Visual Proof Gallery (after hero, before architecture)
```
## Visual proof

| Category 1 | Category 2 | Category 3 |
|:---:|:---:|:---:|
| ![Screenshot 1](url) | ![Screenshot 2](url) | ![Screenshot 3](url) |
| Caption | Caption | Caption |

| Category 4 | Category 5 | Category 6 |
|:---:|:---:|:---:|
| ![Screenshot 4](url) | ![Screenshot 5](url) | ![Screenshot 6](url) |
| Caption | Caption | Caption |
```

**Image hosting:** Use `https://github.com/user/repo/raw/main/assets/readme/` — no external dependencies.

### Badge Template (5 badges standard)
```
![Language](https://img.shields.io/badge/Language-Version-color?logo=logo)
![License](https://img.shields.io/badge/License-MIT-green)
![Protocol](https://img.shields.io/badge/Protocol-Version-orange?logo=protocol)
![Domain1](https://img.shields.io/badge/Domain-Metric-color)
![Domain2](https://img.shields.io/badge/Domain-Metric-color)
```

## Structure Order (readme-craft)

```
1. Hero (title + promise + hero visual)
2. What it is (table: component → description)
3. Quick Start (install + first command + MCP config)
4. Visual Proof (gallery of 6-12 screenshots)
5. Architecture (diagram + component table)
6. Features/Tools (tables)
7. Configuration (table + YAML example)
8. Commands/CLI (table)
9. Requirements
10. License
```

## GitHub API Deployment Workflow

```bash
# 1. Fetch current README + SHA
gh api repos/OWNER/REPO/contents/README.md?ref=BRANCH

# 2. Encode new content
base64 -w0 new_readme.md

# 3. PUT with SHA
gh api --method PUT repos/OWNER/REPO/contents/README.md \
  --input <(jq -n --arg msg "msg" --arg content "b64" --arg sha "sha" \
  '{message: $msg, content: $content, sha: $sha, branch: "BRANCH"}')
```

**Branch detection:** Always query `gh api repos/OWNER/REPO --jq '.default_branch'` first — repos use `main` or `master`.

## VPS-Space-Safe Constraints

- **Only README files** — never clone repos
- Use `gh api repos/OWNER/REPO/contents/README.md` for read/write
- Temporary files in `/tmp/readme_upgrades/` only
- Clean up temp directory after use

## Pitfalls to Avoid

| Pitfall | Prevention |
|---------|------------|
| Architecture before proof | Enforce section order in generator |
| No visual proof | Require hero image + gallery in template |
| Too many badges (>10) | Cap at 8, prefer 5 |
| README bloat (>200 lines) | Move detail to separate docs, link from README |
| Wrong branch push | Always detect default branch first |
| Missing SHA on update | Always fetch current SHA before PUT |
| External image links rot | Use GitHub raw URLs in repo's assets/readme |

## References
- `references/audit-rubric.md` — Full scoring implementation
- `references/visual-proof-patterns.md` — Screenshot gallery templates
- `references/github-api-workflow.md` — Deployment scripts and patterns
- `references/readme-craft-mapping.md` — How each check maps to readme-craft principles