# gog v0.31.1 — New Services Reference

YouTube, Maps, Meet, Sites, Zoom, Analytics, Search Console, Photos, AppScript, Backup, Batch, API, MCP, Config.

## YouTube (`gog youtube`)
```bash
# Search videos
gog youtube search list --account $GOG_ACCOUNT --query "rust programming" --max 10

# List my videos
gog youtube videos list --account $GOG_ACCOUNT --mine

# List my playlists
gog youtube playlists list --account $GOG_ACCOUNT --mine

# List playlist items
gog youtube playlists items list --account $GOG_ACCOUNT --playlist-id "PLxxx"

# List subscriptions
gog youtube subscriptions list --account $GOG_ACCOUNT --mine

# List comments
gog youtube comments list --account $GOG_ACCOUNT --video-id "VIDEO_ID"

# Channel info
gog youtube channels list --account $GOG_ACCOUNT --mine
```

## Maps (`gog maps`)
```bash
# Geocode address
gog maps geocode --account $GOG_ACCOUNT --address "Noida, India"

# Reverse geocode
gog maps reverse-geocode --account $GOG_ACCOUNT --latlng "28.57,77.35"

# Directions
gog maps directions --account $GOG_ACCOUNT   --origin "Noida" --destination "Delhi"

# Distance matrix
gog maps distance --account $GOG_ACCOUNT   --origins "Noida" --destinations "Delhi,Mumbai"

# Place search
gog maps places search --account $GOG_ACCOUNT --query "restaurants near me"

# Place details
gog maps places details --account $GOG_ACCOUNT --place-id "ChIJxxx"
```

## Meet (`gog meet`)
```bash
# Create a meeting
gog meet create --account $GOG_ACCOUNT

# Get meeting info
gog meet get "<meetingId>" --account $GOG_ACCOUNT

# End a meeting
gog meet end "<meetingId>" --account $GOG_ACCOUNT

# Meeting history
gog meet history --account $GOG_ACCOUNT

# List participants
gog meet participants "<meetingId>" --account $GOG_ACCOUNT
```

## Sites (`gog sites`) — Drive-backed
```bash
gog sites list --account $GOG_ACCOUNT
gog sites search "My Site" --account $GOG_ACCOUNT
gog sites get "<siteId>" --account $GOG_ACCOUNT
gog sites url "<siteId>" --account $GOG_ACCOUNT
```

## Zoom (`gog zoom`)
```bash
# Setup (Server-to-Server OAuth — NOT Google credentials)
gog zoom auth setup --account-id "..." --client-id "..." --client-secret "..."

# Check auth status
gog zoom auth doctor
```

## Analytics (`gog analytics`)
```bash
# List GA4 accounts
gog analytics accounts --account $GOG_ACCOUNT

# Run report
gog analytics report --account $GOG_ACCOUNT   --property "<propertyId>"   --start-date 7daysAgo --end-date today   --metrics "sessions,pageViews"   --dimensions "pagePath,country"
```

## Search Console (`gog searchconsole`)
```bash
# List sites
gog searchconsole sites list --account $GOG_ACCOUNT

# Query search analytics
gog searchconsole query --account $GOG_ACCOUNT   --site-url "https://$GOG_DOMAIN"   --start-date 2026-06-01 --end-date 2026-06-27

# List sitemaps
gog searchconsole sitemaps list --account $GOG_ACCOUNT   --site-url "https://$GOG_DOMAIN"

# Submit sitemap
gog searchconsole sitemaps submit --account $GOG_ACCOUNT   --site-url "https://$GOG_DOMAIN" --sitemap-url "https://$GOG_DOMAIN/sitemap.xml"
```

## Photos (`gog photos`)
```bash
# List media
gog photos list --account $GOG_ACCOUNT

# Search media
gog photos search --account $GOG_ACCOUNT --query "2026"

# Download
gog photos download "<mediaId>" --account $GOG_ACCOUNT --out /tmp/photo.jpg

# Picker workflow (session-based)
gog photos picker create --account $GOG_ACCOUNT
gog photos picker wait "<token>" --account $GOG_ACCOUNT
gog photos picker list "<token>" --account $GOG_ACCOUNT
gog photos picker download "<token>" --account $GOG_ACCOUNT --out /tmp/
gog photos picker delete "<token>" --account $GOG_ACCOUNT
```

## AppScript (`gog appscript`)
```bash
# Get script info
gog appscript get "<scriptId>" --account $GOG_ACCOUNT

# Read script content
gog appscript content "<scriptId>" --account $GOG_ACCOUNT

# Run a function
gog appscript run "<scriptId>" --account $GOG_ACCOUNT   --function "myFunction" --params '["arg1","arg2"]' --dev-mode

# Create a new script
gog appscript create --account $GOG_ACCOUNT   --title "My Script" --parent-id "<driveFileId>"
```

## Backup (`gog backup`)
```bash
# Initialize backup
gog backup init --account $GOG_ACCOUNT

# Push (export services into encrypted shards)
gog backup push --account $GOG_ACCOUNT

# Status
gog backup status --account $GOG_ACCOUNT

# Verify shards
gog backup verify --account $GOG_ACCOUNT

# Read a shard
gog backup cat "<shard>" --account $GOG_ACCOUNT

# Plaintext export
gog backup export --account $GOG_ACCOUNT

# Gmail-specific backup
gog backup gmail --account $GOG_ACCOUNT
```

## Batch (`gog batch`) — Persisted Docs request batches
```bash
# Show batch
gog batch show "<batchId>" --account $GOG_ACCOUNT

# Submit batch
gog batch end "<batchId>" --account $GOG_ACCOUNT

# Delete batch
gog batch abort "<batchId>" --account $GOG_ACCOUNT

# Clean stale batches
gog batch prune --account $GOG_ACCOUNT
```

## API (`gog api`) — Generic Discovery API calls
```bash
# List available APIs
gog api list --account $GOG_ACCOUNT

# Describe an API
gog api describe drive v3 --account $GOG_ACCOUNT

# Call any API method
gog api call drive v3 files list --account $GOG_ACCOUNT   --query "mimeType='application/vnd.google-apps.folder'"
```

## MCP (`gog mcp`) — MCP Server
```bash
# List available MCP tools
gog mcp --list-tools --account $GOG_ACCOUNT

# Run MCP server (stdio transport)
gog mcp --account $GOG_ACCOUNT   --allow-tool "gmail.*,docs_get,sheets" --allow-write

# With write access
gog mcp --account $GOG_ACCOUNT --allow-tool "all" --allow-write
```
| Flag | Description |
|------|-------------|
| `--allow-tool=TOOL,...` | Tool/service allowlist (default: read-only) |
| `--allow-write` | Expose write tools |
| `--list-tools` | Print enabled tools as JSON |
| `--timeout-seconds=60` | Per-tool timeout |
| `--max-output-bytes=102400` | Max output per tool call |

## Config (`gog config`)
```bash
gog config list --account $GOG_ACCOUNT
gog config get "keyring.backend" --account $GOG_ACCOUNT
gog config set "keyring.backend" "file" --account $GOG_ACCOUNT
gog config unset "key" --account $GOG_ACCOUNT
gog config path --account $GOG_ACCOUNT

# Per-account no-send guard
gog config no-send set <account>
gog config no-send unset <account>
```

## New Global Flags (v0.31.1)
| Flag | Description |
|------|-------------|
| `--readonly` | Block all mutating API requests; auth requests read-only scopes |
| `--wrap-untrusted` | Wrap fetched text in untrusted-content markers (for agents) |
| `--enable-commands-exact` | Exact command enablement (no prefix matching) |
| `--home=STRING` | Override GOG_HOME |

## calendar changed — Recently modified events
```bash
gog calendar changed --account $GOG_ACCOUNT --since 24h
```
| Flag | Description |
|------|-------------|
| `--since=STRING` | Lower bound (RFC3339, date, or Go duration: 24h, 168h). Default: 720h (30 days) |
| `--cal=CAL,...` | Calendar IDs |
| `--max=10` | Max results |
| `--all` | All calendars |
| `--weekday` | Include day-of-week |
| `--location` | Include location |
