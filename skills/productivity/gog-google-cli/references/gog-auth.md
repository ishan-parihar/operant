# gog auth — Authentication & Credentials Reference

Complete auth lifecycle: setup, add, doctor, import, service-account, tokens, keyring.

## Commands

### status — Show auth configuration
```bash
gog auth status
# Output: config_path, config_exists, keyring_backend, keyring_backend_source
```

### list — List stored accounts
```bash
gog auth list -j --results-only
gog auth list --check --timeout 15s  # Verify tokens work
```

### doctor — Diagnose auth issues
```bash
gog auth doctor -j --results-only
gog auth doctor --check --timeout 15s  # Verify refresh tokens by exchanging
```

### setup — Guided setup (Google Cloud + OAuth + account)
```bash
gog auth setup user@example.com   --services "gmail,calendar,drive,docs,sheets,contacts"   --create-project --enable-apis --login --force-consent
```
| Flag | Description |
|------|-------------|
| `--gcloud-project=STRING` | Google Cloud project ID |
| `--services="gmail,calendar,drive,docs,sheets,contacts"` | Services to configure |
| `--credentials=STRING` | Downloaded Desktop OAuth client JSON |
| `--create-project` | Create project with gcloud |
| `--enable-apis` | Enable selected Google APIs |
| `--login` | Run browser OAuth after setup |
| `--force-consent` | Force consent screen |

### add — Authorize and store a refresh token
```bash
# Interactive browser flow
gog auth add user@example.com --services "gmail,calendar,drive" --force-consent

# Headless: manual flow (paste redirect URL)
gog auth add user@example.com --manual --force-consent

# Headless: remote two-step flow
gog auth add user@example.com --remote --step 1 --force-consent
# User authorizes, copies redirect URL...
gog auth add user@example.com --remote --step 2   --auth-url "http://127.0.0.1:PORT/oauth2/callback?code=...&state=..."   --services "gmail,calendar,drive" --force-consent

# Import existing refresh token
gog auth import --email user@example.com --refresh-token-file /tmp/token.txt

# Import from environment variable
gog auth import --email user@example.com --refresh-token-env GOG_REFRESH_TOKEN
```
| Flag | Description |
|------|-------------|
| `--manual` | Browserless auth flow (paste redirect URL) |
| `--remote` | Remote/server-friendly two-step flow |
| `--step=INT` | Remote auth step: 1=print URL, 2=exchange code |
| `--auth-url=STRING` | Redirect URL from browser (for --remote --step 2) |
| `--force-consent` | Force consent screen |
| `--services="user"` | Services: user, all-user, or comma-separated list |
| `--drive-scope="full"` | Drive scope: full, readonly, file |
| `--gmail-scope="full"` | Gmail scope: full, readonly |
| `--listen-addr=STRING` | Address to listen on for OAuth callback |
| `--redirect-host=STRING` | Hostname for OAuth callback |
| `--timeout=DURATION` | Authorization timeout (default 5m) |

### import — Import refresh token non-interactively
```bash
gog auth import --email user@example.com --refresh-token-file /tmp/token.txt
gog auth import --email user@example.com --refresh-token-stdin <<< "1//0..."
gog auth import --email user@example.com --refresh-token-env GOG_TOKEN   --access-token-env GOG_ACCESS --services "gmail,calendar"
```
| Flag | Description |
|------|-------------|
| `--email=STRING` | Account email (required) |
| `--refresh-token-stdin` | Read refresh token from stdin |
| `--refresh-token-file=STRING` | Read refresh token from file |
| `--refresh-token-env=STRING` | Read refresh token from env var |
| `--access-token-stdin` | Also read access token from stdin |
| `--access-token-file=STRING` | Also read access token from file |
| `--access-token-env=STRING` | Also read access token from env var |
| `--services=STRING` | Services to record (informational) |

### credentials — Manage OAuth client credentials
```bash
# List stored clients
gog auth credentials list -j

# Store a client from file
gog auth credentials add --file /path/to/client.json

# Store a named client
gog auth credentials add --name "my-project" --file /path/to/client.json
```

### tokens — Manage stored tokens
```bash
# List tokens
gog auth tokens list -j

# Import token
gog auth tokens import --email user@example.json --refresh-token "1//0..."

# Export token
gog auth tokens export user@example.com -j
```

### service-account — Configure service account (Workspace only)
```bash
gog auth service-account --key /path/to/service-account.json
gog auth service-account --key /path/to/sa.json --impersonate admin@domain.com
```

### keyring — Configure keyring backend
```bash
# Show current backend
gog auth keyring

# Switch to file backend
gog auth keyring file file

# Set password for file backend
export GOG_KEYRING_PASSWORD=your-password
```

### alias — Manage account aliases
```bash
gog auth alias set work user@company.com
gog auth alias list
gog auth alias remove work
```

### remove — Remove stored token
```bash
gog auth remove user@example.com
```

### manage (login) — Open interactive accounts manager
```bash
gog auth manage  # Opens browser
```

### keep — Configure service account for Keep (Workspace only)
```bash
gog auth keep --key /path/to/service-account.json user@domain.com
```

## Exit Codes
| Code | Meaning |
|------|---------|
| 0 | Success |
| 4 | Auth error (expired/invalid token) |
| 10 | Config error |

## Agent Pattern
```bash
# Diagnose before any operation
gog auth doctor --check -j --results-only || echo "AUTH FAILED"

# List accounts
gog auth list -j --results-only

# Verify specific account
gog auth list --check -j --results-only | grep user@example.com
```
