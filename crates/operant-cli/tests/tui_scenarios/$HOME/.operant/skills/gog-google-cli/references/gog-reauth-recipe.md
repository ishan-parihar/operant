# gog Re-auth on Headless VPS — Manual Redirect URL Flow

## The Problem
Google OAuth refresh tokens expire after months of inactivity (`invalid_grant: Bad Request`). Running `gog auth add` normally requires a browser on the same machine — impossible on this headless VPS.

## The Solution: gog `--manual` flag (v0.15.0+)

The gog binary **v0.15.0** introduced `--manual` and `--remote` flags that let you complete OAuth from any device. No headless browser needed.

### How `--manual` Works

```bash
export GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD
gog auth add <email> --manual --force-consent
```

1. gog prints an OAuth URL and waits for input
2. User opens the URL on their phone/desktop in any browser
3. User signs in and authorizes the app ("Ishan Parihar Windmill")
4. Google redirects the browser to `http://127.0.0.1:PORT/oauth2/callback?code=...&state=...`
5. **The page fails to load** (user's browser can't reach VPS `127.0.0.1`) — this is expected
6. User **copies the ENTIRE failing URL from the browser address bar** (it contains the auth code)
7. User pastes the URL back into the gog prompt
8. gog extracts the code, exchanges it for tokens, saves to keyring

### Why URL-Based vs Code-Based

The OAuth client (`138669390481`, app "Ishan Parihar Windmill") has **only** `http://127.0.0.1:PORT/oauth2/callback` registered as a redirect URI. This means:

- ❌ `urn:ietf:wg:oauth:2.0:oob` (standard copy-paste code) — returns **400 invalid_request**
- ✅ `http://127.0.0.1:PORT/oauth2/callback` — works, but redirect fails and user must copy the URL

**Never generate OAuth URLs with `urn:ietf:wg:oauth:2.0:oob` for this client.** Always use the gog binary or generate URLs with `http://127.0.0.1:PORT/oauth2/callback`.

### Non-interactive Version: `--remote`

If you can't use interactive input (e.g., you're capturing the URL via Telegram), use two-step `--remote`:

**Step 1 — Generate URL:**
```bash
gog auth add $GOG_ACCOUNT --remote --step 1 --force-consent
# Output includes: auth_url  https://accounts.google.com/o/oauth2/auth?...
# Save the state string too — needed for step 2
```

**Step 2 — Exchange code** (after user copies the redirect URL back):
```bash
gog auth add $GOG_ACCOUNT --remote --step 2 \
  --auth-url "http://127.0.0.1:41145/oauth2/callback?code=4/0A...&state=..." \
  --services gmail,calendar,drive,docs,slides,contacts,tasks,sheets,forms,appscript,ads \
  --force-consent
```

The `--services` list must match what was requested in step 1. The state parameter must match.

## Alternative: Python google-auth-oauthlib Flow

The `~/google-oauth-console.py` script provides a complete console-based flow:
```bash
python3 ~/google-oauth-console.py $GOG_ACCOUNT
```
This generates a URL, waits for code paste, exchanges the code, saves to `~/.config/google_auth/tokens/`, and verifies with a Gmail API call.

## After Successful Auth

`gog auth add` saves tokens to the gog keyring. The Python fallback (`gog auth`) stores tokens separately at `~/.config/google_auth/tokens/`. After running gog's auth flow:

1. gog keyring gets the token (but can't read it back due to keyring decryption bugs)
2. **You must also save the token for the Python fallback** using:
   ```bash
   python3 ~/google-oauth-setup.py save <email> <AUTH_CODE>
   ```
   Or manually copy the refresh_token to the Python token file.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `invalid_grant: Bad Request` | Refresh token expired | Re-authorize with `--force-consent` |
| `400 invalid_request` | Wrong redirect_uri | Use `http://127.0.0.1:PORT/oauth2/callback`, NOT `urn:ietf:wg:oauth:2.0:oob` |
| `Access blocked: request is invalid` | Account not a test user for the OAuth app, or app not published | Try a different account (another authorized account usually works) |
| Keyring `integrity check failed` | Password mismatch | Both `$GOG_KEYRING_PASSWORD` and `$GOG_KEYRING_PASSWORD` tried — neither matches the encrypted files |
| Browser shows blank/error after auth | Normal — redirect to localhost:PORT always fails on user's device | Copy the URL from address bar anyway |

## Key Files
- `~/google-oauth-setup.py` — two-step script (step 1 = URL, step 2 = save code)
- `~/google-oauth-console.py` — complete interactive flow with Gmail verification
- `gog auth` — Python fallback for API calls (uses `~/.config/google_auth/tokens/`)
- `~/.config/google_auth/tokens/` — working token storage for Python fallback
- `~/.config/gogcli/keyring/` — gog binary token storage (corrupted/broken)
