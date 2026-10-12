# Google Services Protocol

**MANDATORY**: Always use `gog` CLI for ALL Google services. Never use Social Forge MCP for Google Calendar, Gmail, Drive, YouTube, or any Google Workspace API.

## Why

- Social Forge MCP does NOT have Google Calendar/Gmail connected (not even listed as an available provider)
- `gog` CLI is purpose-built for Google Workspace with full OAuth support
- Social Forge's Google tools (`mcp_social_forge_goog_*`) will fail with "Google not connected"
- `gog` binary is installed at `gog` (v0.13.0-dev)

## Authenticated Accounts

```bash
gog auth list
# $GOG_ACCOUNT  (primary)
# $GOG_ACCOUNT
# $GOG_ACCOUNT
```

## Common Operations

### Calendar Events
```bash
# List events for a date range
gog calendar events --account=$GOG_ACCOUNT \
  --from="2026-07-11T00:00:00+05:30" \
  --to="2026-07-11T23:59:59+05:30" \
  --max=20 -j

# Search events
gog calendar events --account=$GOG_ACCOUNT \
  --query="Facilitator Seminar" --max=10 -j
```

### Gmail Messages
```bash
# Search messages
gog gmail messages search "from:law-of-one-europe@proton.me" \
  --account=$GOG_ACCOUNT --max=5 -j

# Get full message with attachments
gog gmail get <messageId> --account=$GOG_ACCOUNT -j
```

### Key Flags
- `--account=EMAIL` — required for all operations (or set `GOG_ACCOUNT`)
- `-j` / `--json` — JSON output for parsing
- `--max=N` — limit results
- `-j` is essential for agent consumption

## Protocol Enforcement

When user asks for Google Calendar, Gmail, Drive, YouTube, Docs, Sheets, or any Google service:
1. **Use `gog` CLI via `terminal()` tool**
2. **Never call `mcp_social_forge_goog_*` tools**
3. **Never use `mcp_igs_web_search` for Google services**

This is a hard rule — Social Forge simply doesn't have Google connected on this system.