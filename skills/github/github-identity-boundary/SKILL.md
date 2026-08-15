---
name: github-identity-boundary
description: "Multi-account GitHub workflows: manage repos across personal + brand identities."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  operant:
    tags: [github, multi-account, identity, dual-identity, repo-organization]
    related_skills: [github-auth, github-repo-management]
---

# GitHub Identity Boundary

Pattern for maintaining two GitHub identities: a personal account and a
brand/agent account. Covers auth awareness, repo namespace discipline, and the
operational habit of checking which account you are building under.

## When This Skill Applies

Use when:
- User says "these repos should be under the agent's account, not mine"
- You need to create or push repos and must pick the right owner
- You are planning a repo migration between accounts
- You discover repos in the wrong namespace and need a cleanup plan

## 1. Always Check Active Identity

Before any repo-creating operation, verify which user `gh` is authenticated as:

```bash
gh auth status 2>&1 | grep "Logged in"
```

The active account determines namespace. Listing another user's repos may work
via token scopes, but creating/pushing requires that user's credentials.

Common failure mode: `gh` is authed as personal account — agent repos
unintentionally end up in the personal namespace.

## 2. Identity Inventory

Map what belongs where:

```bash
# Personal account
gh repo list PERSONAL_USER -L 50 --json name

# Brand/agent account  
gh repo list BRAND_USER -L 50 --json name
```

Cross-reference against the boundary convention (Section 5).

## 3. Repo Transfer Options

### Using the Transfer API

Moves entire repo (issues, stars, history) to another user/organization.

```bash
gh api -X POST /repos/SOURCE_OWNER/REPO_NAME/transfer \
  -f new_owner=DESTINATION_USER
```

**Pitfall — 422 "Repository has already been taken":**
This error can be misleading. It may mean:
- Repo with same name exists on destination
- Soft-deleted repo not yet garbage-collected
- Your token lacks admin on destination (error message is confusing)

**Action:** Confirm manually via `gh repo list DESTINATION`, then fall back
to mirror push if the name is available but transfer still fails.

### Mirror Push (fallback when cross-account admin unavailable)

When your token has admin only on the source side:

1. Create an empty repo on destination (needs destination credentials)
2. Mirror clone the source locally
3. Push everything to the destination repo
4. Update the local working copy's remote URL

After migration, update local remotes:

```bash
git remote set-url origin https://github.com/NEW_OWNER/REPO_NAME.git
```

## 4. Post-Migration Updates

After moving a repo, audit all references:

- [ ] Local working copy remote URLs
- [ ] CI/CD configs (Actions, deployment hooks)
- [ ] Webhooks pointing to old URL
- [ ] README badges and install links
- [ ] MCP server configs that reference the repo
- [ ] Cron jobs or scripts that clone/pull from old location

## 5. Repository Boundary Convention

Maintain clear namespace separation:

```
Personal Account:
  personal-site, side-projects, shared-mcp-servers

Brand/Agent Account:
  tdg, trading-engine, storefront, agent-mcp-services, website
```

**Rule:** Agent initiatives (autonomous, revenue-generating) go under brand.
Personal experiments and shared infra stay personal.

## 6. Identity Boundary Beyond GitHub

The personal-vs-brand distinction extends to all infrastructure:

- **Vercel:** Deploy under brand account, not personal
- **Cloudflare:** Brand domain (ishanparihar.com) under brand credentials
- **Razorpay:** Payments under brand
- **Gumroad/Substack:** Brand stores under brand email
- **Twitter:** Brand handle for agent presence

Rule of thumb: if it generates revenue or represents the agent, use brand
identity. Shared infra or personal work stays personal.

## 7. Memory Curation

The identity boundary depends on current memory. Periodically audit memory for
stale entries that conflict with current boundaries:

- Remove binary-version-specific entries and session-specific debug traces
- Replace outdated identity descriptions with current boundary rules
- Archive migration details into skill reference files, not memory

Memory holds durable identity facts; skills carry procedures.

## 8. Verification Checklist

- [ ] Active `gh` identity matches target for the operation
- [ ] Destination repo shows correct owner
- [ ] Old location returns 404 (after transfer)
- [ ] Local remotes point to new URL
- [ ] Deployment/CI configs updated for new namespace
