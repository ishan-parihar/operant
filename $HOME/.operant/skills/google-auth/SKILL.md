---
name: google-auth
version: 1.0.0
category: productivity
description: Google OAuth authentication, token management, and CLI configuration. Foundation skill for all Google Workspace operations (email, calendar, contacts, drive). Use when diagnosing auth issues, managing accounts, or re-authorizing Google access.
triggers:
  - When Google services fail with auth errors (exit code 4)
  - When managing Google accounts (add, remove, list)
  - When setting up OAuth for a new project
  - When diagnosing token/keyring issues
  - Before any email/calendar/contacts/drive operation that fails unexpectedly
  - When user asks about auth, login, sign-in, or "is Google connected"
metadata:
  operant:
    tags: [google, oauth, auth, tokens, keyring]
---

# Google Auth

OAuth authentication and CLI configuration for all Google Workspace operations. This is the foundation skill: when something fails, run diagnostics here FIRST.

## Quick Start

```bash
# Health check (run first when debugging)
gog auth doctor --check -j --results-only
# Exit 0 = healthy, 4 = auth error

# List all stored accounts
gog auth list -j --results-only

# Check specific account (verifies refresh token works)
gog auth list --check -j --results-only | grep $GOG_ACCOUNT

# Show config + keyring backend
gog auth status
```

## Account Configuration

**Working account:** `$GOG_ACCOUNT` (default for all operations)

**Blocked accounts (do NOT use):**
- `$GOG_ACCOUNT` — blocked by Google
- `$GOG_ACCOUNT` — tokens expired

**Config file:** `~/.config/gogcli/config.json`

```json
{
    "keyring_backend": "file",
    "default_account": "$GOG_ACCOUNT"
}
```

## Auth Diagnostics

```bash
# Full diagnostic with refresh token exchange test
gog auth doctor --check -j --results-only

# Just check stored accounts
gog auth list --check --timeout 15s

# Show raw config
gog auth status
```

**Exit codes:**
| Code | Meaning | Action |
|------|---------|--------|
| 0 | Healthy | proceed with operation |
| 4 | Auth error | re-auth required (see OAuth Re-auth below) |
| 10 | Config error | check config.json structure |

## Token Management

**Storage:** `~/.config/gogcli/keyring/` (PBES2-HS256+A128KW encrypted files)

**Keyring password:** `$GOG_KEYRING_PASSWORD` (stored in `~/.config/gogcli/.keyring_pass`)

**Critical for subprocess:** When running gog from Python subprocess (not shell), set `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD` in env dict. Shell has it via `~/.bashrc` but subprocess does NOT inherit it.

```python
import subprocess
env = {"GOG_KEYRING_PASSWORD": "$GOG_KEYRING_PASSWORD"}
r = subprocess.run(["gog", "auth", "list"], capture_output=True, env={**os.environ, **env})
```

**Python fallback tokens:** `~/.config/google_auth/tokens/` (plain JSON, read-only via `gog auth`)

### Import existing refresh tokens (PRIMARY METHOD for headless VPS)

If fallback tokens exist at `~/.config/google_auth/tokens/`, this is the fastest way to restore auth:

```bash
export GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD

# Check for existing fallback tokens
ls ~/.config/google_auth/tokens/
# Files like: $GOG_ACCOUNT_FILE

# Import each token
gog auth import --email $GOG_ACCOUNT \
  --refresh-token-file ~/.config/google_auth/tokens/$GOG_ACCOUNT_FILE \
  --services "gmail,calendar,contacts,drive"
```

To extract just the refresh_token from a fallback token file and import:

```python
import json, tempfile, subprocess, os
env = {**os.environ, "GOG_KEYRING_PASSWORD": "$GOG_KEYRING_PASSWORD"}
tokens_dir = os.path.expanduser("~/.config/google_auth/tokens")

accounts = [
    ("$GOG_ACCOUNT", "$GOG_ACCOUNT_FILE"),
    ("$GOG_ACCOUNT", "$GOG_ACCOUNT_FILE"),
    ("vyreagent@gmail.com", "vyreagent_at_gmail_dot_com.json"),
]

for email, token_file in accounts:
    tpath = os.path.join(tokens_dir, token_file)
    with open(tpath) as f:
        token = json.load(f)
    rt = token.get("refresh_token", "")
    with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as tf:
        tf.write(rt)
        tmp_path = tf.name
    subprocess.run(
        ['gog', 'auth', 'import', '--email', email,
         '--refresh-token-file', tmp_path,
         '--services', 'gmail,calendar,contacts,drive'],
        env=env, capture_output=True, timeout=30)
    os.unlink(tmp_path)
```

### Verify all accounts

```bash
gog auth list -j --results-only
gog auth doctor --check -j --results-only
```

### Headless OAuth flow (when no fallback tokens exist)

```bash
# Step 1: Get auth URL (visit in browser on local machine)
gog auth add user@example.com --manual --force-consent
# Outputs URL like: https://accounts.google.com/o/oauth2/auth?...

# Step 2: User authorizes in browser, gets redirected to localhost with auth code
# User copies the full redirect URL back

# Step 3: Complete auth
gog auth add user@example.com --manual --auth-url "http://127.0.0.1:PORT/oauth2/callback?code=...&state=..." --services "gmail,calendar,contacts,drive" --force-consent
```

### Remote two-step flow

```bash
# Step 1: print URL
gog auth add user@example.com --remote --step 1 --force-consent

# User authorizes, copies redirect URL...
# Step 2: exchange code
gog auth add user@example.com --remote --step 2 --auth-url "..." --services "gmail,calendar,drive" --force-consent
```

## Keyring Configuration

```bash
# Show current backend
gog auth keyring

# Switch to file backend
gog auth keyring file

# Password for file backend (must be in env for subprocess)
export GOG_KEYRING_PASSWORD=your-password
```

**Backends:**
- `file` — encrypted file in `~/.config/gogcli/keyring/` (default, password required)
- `os` — system keyring (interactive only, not for headless)
- `secret-service` — GNOME/KDE secret service

## Config Management

```bash
# Edit config
nano ~/.config/gogcli/config.json

# Verify structure
cat ~/.config/gogcli/config.json | python3 -m json.tool

# Verify default_account is set (most common fix)
grep default_account ~/.config/gogcli/config.json
```

**Required fields:**
- `keyring_backend` — "file" or "os"
- `default_account` — the email to use when --account not specified

## Account Removal

```bash
# Remove a stored account
gog auth remove user@example.com

# Manage aliases
gog auth alias set work user@company.com
gog auth alias list
gog auth alias remove work
```

## Service Account (Workspace only)

```bash
# Configure service account for domain-wide delegation
gog auth service-account --key /path/to/sa.json --impersonate admin@domain.com
```

## Pitfalls

⚠️ **config.json MUST have `default_account` set.** If `config.json` only has `"keyring_backend": "file"` with no `default_account`, gog fails silently. Fix:
```bash
echo '{"keyring_backend":"file","default_account":"$GOG_ACCOUNT"}' > ~/.config/gogcli/config.json
```
Always verify with `cat ~/.config/gogcli/config.json` before debugging auth issues.

⚠️ **GOG_KEYRING_PASSWORD must be set in env for subprocess calls.** Shell has it via `.bashrc` but Python subprocess does NOT inherit it. Set in env dict manually.

⚠️ **Only `$GOG_ACCOUNT` works.** Other accounts (`$GOG_ACCOUNT`, `$GOG_ACCOUNT`) are blocked or have expired tokens. Don't waste cycles debugging them.

⚠️ **Exit code 4 = auth error.** When you see exit code 4 from any gog command, run `gog auth doctor --check` immediately.

⚠️ **`--readonly` is global.** When set on any gog command, ALL write operations fail for that session.

⚠️ **Tokens expire after months of inactivity.** If a working account suddenly returns auth errors, re-auth using the manual flow above.

gog is the sole email/calendar/workspace tool.

## Key Files

| File | Purpose |
|------|---------|
| `gog` | gog binary (v0.31.1) |
| `~/.config/gogcli/config.json` | gog config (default_account, keyring_backend) |
| `~/.config/gogcli/credentials.json` | OAuth client credentials |
| `~/.config/gogcli/keyring/` | Encrypted tokens (PBES2) |
| `~/.config/gogcli/.keyring_pass` | Keyring password ($GOG_KEYRING_PASSWORD) |
| `gog auth` | Python fallback (read-only) |
| `~/google-oauth-setup.py` | OAuth URL generator + code saver |
| `~/google-oauth-console.py` | Interactive OAuth flow |
