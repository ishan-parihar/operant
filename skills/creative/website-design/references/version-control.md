# Version Control: commit as you go, never leave work on disk

Version-control discipline for every website build: git-init at scaffold, commit at every phase end, push before deploy. Run alongside every pipeline phase (briefing -> ... -> quality/preflight) when working inside a webdev-managed site repo. Triggered whenever you are building/editing a site that lives under WEBSITES/.

Sites are built on a headless VPS by autonomous agents. Work that is not
committed and pushed can be lost. This leaf is NOT optional: the webdev
quality gates (WD-AD-001 rule 3) refuse uncommitted work, and `web deploy`
pushes before building. The discipline is simple and mechanical.

## When this applies

- You are working inside any site repo under `~/Documents/GitHub/MY-PROJECTS/WEBSITES/<site>/`
- You just scaffolded, designed, wrote content, built components, or fixed bugs
- You are about to run `web quality` or `web deploy`

## Procedure

1. **Scaffold already git-inits** - `web scaffold` creates the repo + `.gitignore`
   + initial commit. Never skip it; never hand-roll a site outside the webdev CLI.
2. **Commit at every phase end.** After completing any meaningful chunk
   (design artifacts, essay/content seed, components, a bugfix), run:
   ```bash
   web vc --site <site> commit --reason "<phase>: <summary>"
   ```
   If the web CLI is unavailable, fall back to raw git:
   ```bash
   cd ~/Documents/GitHub/MY-PROJECTS/WEBSITES/<site>
   git add -A && git commit -m "<phase>: <summary>"
   ```
3. **Check status before gates.** Before `web quality`, confirm the tree is clean:
   ```bash
   web vc --site <site> status     # expect "clean=yes"
   ```
   If dirty, commit first. `web quality` FAILS on uncommitted changes by design.
4. **Push before you stop / before deploy.** `web deploy` commits and pushes
   automatically. If you are finishing a phase without deploying, push explicitly
   so the work is off the VPS disk:
   ```bash
   web vc --site <site> push       # creates ishan-parihar/<site>-web if missing (private)
   ```
5. **Never delete `.git` or commit `node_modules/`** - the scaffold `.gitignore`
   covers it; leave it alone.

## Checks

- [ ] Site repo exists (`.git` present) after scaffold
- [ ] Every phase ended with a commit (clean `git status`)
- [ ] `node_modules/`, `.svelte-kit/`, `.env` are ignored, not committed
- [ ] Work is pushed to `origin` before you finish a session (unless deploy does it)
- [ ] `web quality` passes its git-clean gate before any deploy attempt

## Why (one line)

Unversioned VPS disk work is one wipe away from loss - the webdev lifecycle
(enforced by `web quality` + `web deploy`) exists so this can never happen.
