---
name: lyr-cli-operations
description: 'Use when testing/fixing *-lyr social CLI tools.'
version: 1.0.0
platforms: [linux, macos, windows]
metadata:
  operant:
    tags: [social-cli, obscura, linkedin, instagram, twitter, browser]
---

# lyr Social-CLI Operations

## Purpose

The lyr family (linkedin-lyr, instagram-lyr, reddit-lyr, twitter-lyr, facebook-lyr) is a set of MCP/CLI tools for social platforms, owned by the operator. They share an architecture: each has its **own independent browser/cookie stack** and does NOT depend on any shared browser tier on the VPS. This skill covers operating, testing, and fixing them.

## When to Use

- Testing or debugging any `*-lyr` CLI (linkedin-lyr, instagram-lyr, reddit-lyr, twitter-lyr, facebook-lyr) after upgrades, cookie loss, or auth failures
- Auditing cookie/session state or restoring sessions from `invalid-state-*` backups
- Any decision about browser backends for social CLIs on the VPS (Obscura vs Chromium) — read the Golden Rule BEFORE acting
- Pushing fixes to the `*-lyr` upstream repos (direct-to-main)

## Golden Rule: NEVER Install System Browsers (HI-RG-028)

**Never add a system browser to the VPS** — not via the package manager (apt) and not via patchright/playwright browser-install subcommands. This is a hard prohibition (HI-RG-028, Aug 12 2026). The lyr tools use **Obscura** as their browser backend via CDP:

- Obscura binary: `/tmp/obscura` (symlink → `~/.config/igs-mcp/bin/obscura`), also `~/.local/bin/obscura`
- linkedin-lyr: `ObscuraBrowserManager` starts `obscura serve` as a CDP server (port 9224, storage dir, stealth flags), then Playwright connects via `connect_over_cdp` on the local WebSocket endpoint
- `patchright` / `playwright` in pyproject.toml are **API-only control libraries** (Page objects, connectOverCDP) — they are NOT browser binaries and do not need Chromium added for them
- instagram-lyr uses `obscura_core` plugin integration (`obscura_daemon_integration.py`), plus `instagrapi` + `curl-cffi` — no patchright at all

**If a lyr tool seems to need a browser, the fix is never "add a browser".** Check: (1) cookies/session state, (2) Obscura binary presence (`ls -la /tmp/obscura`), (3) whether the tool has a curl_cffi / Voyager API path that bypasses the browser entirely (linkedin-lyr's `profile_edit_*` tools do).

## Cookie / Session Three-Component Model

A lyr tool's `--status` reports `session: valid` ONLY when all three components exist under `~/.<tool>-lyr/`:

1. `cookies.json` — portable cookie export (root of the `.lyr` dir)
2. `source-state.json` — source session metadata (`{"version":1, "source_runtime_id":..., "login_generation":..., "profile_path":..., "cookies_path":...}`)
3. `profile/` dir — source profile directory (may contain `cookies.json` + `.cookie-sync-marker`)

`--status` checks ALL three (`if not source_state or not profile_exists(...) or not cookies_path.exists()`). Restoring only `cookies.json` yields `error: No valid source session` — you must restore all three.

## Restore From invalid-state Backups

When sessions fail, the tool moves artifacts to `~/.<tool>-lyr/invalid-state-<ISO-timestamp>/`. Restore:

```bash
BK=~/.linkedin-lyr/invalid-state-2026-08-11T22-30-40Z-f1a42ab3   # newest backup
mkdir -p ~/.linkedin-lyr/profile
cp "$BK/cookies.json" ~/.linkedin-lyr/cookies.json
cp "$BK/cookies.json" ~/.linkedin-lyr/profile/cookies.json
cp "$BK/source-state.json" ~/.linkedin-lyr/source-state.json
# profile dir may carry extra files (.cookie-sync-marker) — copy them too
instagram-lyr --status   # verify: expect 'status: valid'
```

Validate cookie integrity: `li_at` (LinkedIn) / `sessionid`+`csrftoken` (Instagram) must be present. If the JSON is a Playwright-style list vs flat dict, the loader may reject it — check the tool's `FileCookieStorage._load_cookies` expectations.

## End-to-End Testing Pattern (MCP Server + curl)

1. Start server in background: `<tool>-lyr --transport streamable-http --port 8091 --log-level ERROR --no-auto-import`
2. Initialize the MCP session with a POST to the server's `/mcp` endpoint (JSON-RPC `initialize`), capture the `Mcp-Session-Id` response header.
3. Call a tool with a second POST (`tools/call`) carrying that session id, e.g. `get_user_profile` with `{"username":"natgeo","sections":"about"}`.

**Pitfall — tool argument names differ per tool.** Instagram's tool param is `username`, NOT `instagram_username`. Passing `instagram_username` returns a pydantic validation error (`Missing required argument: username` / `Unexpected keyword argument: instagram_username`). Verify arg names from the tool's registration source or `--list-tools` before calling.

**Pitfall — "Session expired. A login browser window has been opened" is NORMAL behavior.** That response proves the server, auth check, tool routing, and error handling all work; the cookies are just stale server-side. It is not a tool bug. `--status` "valid" means local files exist, not that the remote session is live.

## Auth-Reality Expectations

- **linkedin-lyr**: LinkedIn HTTP 429 rate-limits the VPS IP — that is an IP-reputation issue, not a tool bug. Fix = proxy/IP rotation or refresh cookies from a real login.
- **linkedin-lyr Voyager path**: `profile_edit_*` tools use curl_cffi + cookies (no browser). Known upstream bug (fixed commit `92acbcc`): `profile_edit_status` passed `public_id=` to `VoyagerProfileEditClient.__init__()` which doesn't accept it — instantiate with no args (`_profile_id()` is hardcoded to `"me"`).
- **Cookies lost on upgrade**: `git stash` + `pull` upgrades can move/delete cookies.json; always check the `invalid-state-*` backups before re-logging-in.

## Upstream Repo Hygiene

- **NO branches — push directly to main.** User directive, no exceptions. Cherry-pick the fix onto main, delete any feature branch (local + remote), push main.
- Verify installs: `uv tool install --force` from local clones; test each tool after upgrade with its real command (`--status`, browse, search) before declaring it functional.

## Pitfalls Checklist

- [ ] Never add a browser to this VPS via apt or patchright/playwright subcommands
- [ ] Never run a browser-install subcommand even if a repo's dev docs list it
- [ ] Restore ALL THREE session components from backups, not just cookies.json
- [ ] Verify tool param names from source before curl calls
- [ ] Treat network auth failures (429, "Session expired") as infra reality, not tool bugs
- [ ] Push corrections to lyr upstream repos directly to main, no branches
- [ ] After `git pull` on a lyr repo, check cookies weren't moved to invalid-state