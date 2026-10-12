---
name: upstream-contribution
description: "Contribute fixes back to upstream repos: fork diff analysis, upgrade verification, and PR workflow."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  operant:
    tags: [GitHub, upstream, fork, contribution, PR]
    related_skills: [github-pr-workflow, github-identity-boundary]
---

# Upstream Contribution Workflow

Pattern for contributing fixes back to an upstream repo you've forked.
Adds a quality gate to the standard PR workflow: **verify the diff is
an upgrade, not a regression** before opening the PR.

## When This Skill Applies

- You found a bug in a dependency/project you forked
- You fixed it locally and want to contribute the fix upstream
- The user asked to "send the PR to [upstream]"

## Workflow

### 1. Fetch upstream and compare divergence

```bash
git fetch origin main   # origin = upstream
git log --oneline main..origin/main          # what upstream has that we don't
git log --oneline origin/main..main          # what we have that upstream doesn't
git diff main..origin/main --stat            # scope of divergence
```

### 2. Check for conflicts in your changed files

```bash
git diff --stat   # uncommitted local changes
# If upstream also touched your files → resolve before PR
```

### 3. Verify build passes

Critical for frontend/compiled projects. The dist/ output is often
gitignored — you must rebuild locally to verify, but the dist itself
is NOT committed.

```bash
npm run build --workspace=web  # or equivalent
# Confirm zero errors before proceeding
```

### 4. Evaluate: upgrade or regression?

- Does upstream have any of MY changes already? → Skip PR if yes
- Does my change conflict with upstream's direction? → Discuss first
- Is my change backward-compatible? → Document if no
- Did I introduce new type exports? → Document in PR body

### 5. Create branch, commit, push to fork

```bash
git checkout -b fix/description
git add <files>
git commit -m "fix(scope): description"
git push fork fix/description
```

### 6. Create PR targeting upstream

```bash
gh pr create \
  --repo UPSTREAM_OWNER/UPSTREAM_REPO \
  --head YOUR_FORK:fix/description \
  --base main \
  --title "fix(scope): description" \
  --body "..."
```

## Pitfalls

**Build artifacts in gitignore:** Many projects gitignore their build
output (dist/, build/, web_dist/). Your changes compile and the build
passes, but the dist diff won't show in `git diff`. This is correct —
upstream rebuilds on merge. Don't commit dist files.

**Fork remote naming:** Use named remotes to avoid pushing to the wrong
repo. Convention: `origin` = upstream, `fork` = your fork. Verify with
`git remote -v` before every push.

**Upstream may have merged your fix already:** Always check
`git log origin/main..main` before creating the PR. If upstream already
has equivalent changes, close the branch instead.

**Type union extensions:** Adding a new member to a TypeScript union type
(like `ConnectionState`) is backward-compatible at the value level but
may cause exhaustiveness-check warnings in consumers. Document this in
the PR body so reviewers know to check downstream pattern matches.

**PR body must explain the "why":** Upstream maintainers don't know
your context. Include: (1) what the bug is, (2) why it matters, (3) how
the fix works, (4) backward compatibility notes, (5) how to verify.
