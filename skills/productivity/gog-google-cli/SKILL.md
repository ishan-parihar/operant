---
name: gog-google-cli
description: "Google CLI (`gog` v0.31.1) — REFERENCE SKILL for advanced/deep usage of email, calendar, contacts, drive. Do NOT use this skill for normal operations. Load ONLY when the function-named skills (google-auth, email, calendar, contacts, drive-storage) don't cover a specific edge case. Use function skills first; this is the fallback for unusual scenarios, deep debugging, and reference material."
category: productivity
version: 5.0.0
triggers:
  - When the function-named skills (google-auth, email, calendar, contacts, drive-storage) don't cover a specific need
  - When needing deep reference material for an edge case
  - When debugging an unusual gog CLI issue not covered by the function skills
  - DO NOT use for normal email/calendar/contacts/drive operations — use the function-named skills
metadata:
  operant:
    tags: [gog, google-cli, gmail, calendar, drive, reference]
---

# gog — Google CLI (v0.31.1)

> **⚠️ REFERENCE SKILL ONLY.** For normal operations, use the function-named skills:
> - **`google-auth`** — OAuth, tokens, keyring, account management
> - **`email`** — Send/receive/thread/labels/inbox
> - **`calendar`** — Events/scheduling (with IST timezone handling)
> - **`contacts`** — Address book CRUD/dedupe/export
> - **`drive-storage`** — File upload/download/share
>
> This skill is a fallback for advanced/deep usage not covered by the function skills. It contains the full gog CLI command reference and detailed examples.

## References

### Core Services
- `references/gog-command-reference.md` — Complete command reference (gmail, calendar, drive + extras)
- `references/gog-gmail-search-syntax.md` — Gmail search query syntax and patterns
- `references/gog-gmail-advanced.md` — **Thread ops, messages, raw, attachment, reply, reply-all, track, settings** (filters, delegates, forwarding, send-as, vacation, watch)
- `references/gog-docs-deep.md` — **Write, insert, tables, cell ops, find-replace, sed, format, header/footer, comments**
- `references/gog-sheets-deep.md` — **Conditional formatting, charts, tables, named ranges, validation, batch-update, delete-dimension**
- `references/gog-slides-deep.md` — **Markdown import, templates, thumbnails, text manipulation, tables, locate, move/duplicate slides**

### New Services (v0.31.1)
- `references/gog-contacts.md` — Search, list, create, update, delete, export (.vcf), dedupe
- `references/gog-tasks.md` — Task lists, CRUD, subtasks, recurring tasks (RRULE)
- `references/gog-people.md` — Profile, directory search, relations
- `references/gog-chat.md` — Spaces, messages, threads, DMs, reactions
- `references/gog-forms.md` — Create forms, add questions, responses, watches
- `references/gog-keep.md` — Google Keep notes (Workspace only)
- `references/gog-new-services.md` — **YouTube, Maps, Meet, Sites, Zoom, Analytics, Search Console, Photos, AppScript, Backup, Batch, API, MCP, Config**

### Calendar-Specific Patterns
- `references/gog-calendar-timezone-patterns.md` — **Critical timezone handling patterns for calendar create/update (IST calendar default)**

### Agent & Auth
- `references/gog-auth.md` — **Full auth lifecycle** (setup, add, doctor, import, service-account, tokens, keyring, credentials)
- `references/gog-schema.md` — **Machine-readable command contracts** (gog schema --json for agent integration)
- `references/gog-reauth-recipe.md` — Headless OAuth re-auth
- `references/gog-email-management.md` — **Inbox cleanup, unsubscribe extraction, label automation, auto-filters**

## Quick Start

```bash
# Health check
gog auth health && echo "✅"

# List inbox
gog gmail search "is:inbox" -a $GOG_ACCOUNT --max 10

# Send email (dry-run first)
gog gmail send -a $GOG_ACCOUNT \
  --to recipient@example.com --subject "Subject" --body-file /tmp/body.txt --dry-run

# MCP server (for agent integration)
gog mcp -a $GOG_ACCOUNT --allow-tool "gmail.*,docs_get,sheets"

# Auth diagnostics
gog auth doctor --check -j --results-only
```

## Accounts

| Account | Status | Use For |
|---------|--------|---------|
| `$GOG_ACCOUNT` | ✅ Working | All operations (default) |
| `$GOG_ACCOUNT` | ⚠️ Blocked by Google | — |
| `$GOG_ACCOUNT` | ⚠️ Expired tokens | — |

## Tools

| Tool | Path | Capability |
|------|------|------------|
| **gog binary** | `gog` (v0.31.1) | Full: gmail, calendar, drive, docs, sheets, slides, contacts, tasks, people, chat, forms, keep, youtube, maps, meet, sites, analytics, searchconsole, photos, appscript, backup, batch, api, mcp |
| **Python fallback** | `gog auth` | Read-only: gmail, calendar, drive (backup) |

**Primary tool: gog binary.** Python fallback is backup only.

## AI Agent Execution Patterns

### Default Flags for Agent Context
```bash
# ALWAYS use these flags for agent operations:
-a $GOG_ACCOUNT   # Explicit account
-j --results-only                       # JSON output, primary result only
--no-input                             # Never prompt
```

### Read-Only Agent
```bash
# Use --readonly for any read-only agent context
gog gmail search "is:inbox" --readonly -a $GOG_ACCOUNT -j --results-only
gog calendar events --readonly -a $GOG_ACCOUNT -j --results-only
```

### Untrusted Content Handling
```bash
# Wrap fetched text in security markers
gog gmail get "<msgId>" --wrap-untrusted -a $GOG_ACCOUNT -j
```

### Machine-Readable Contracts
```bash
# Get command schema before executing (agent best practice)
gog schema gmail send --json | jq '.commands.gmail.commands.send.flags | keys'
# Returns: ["bcc", "body", "body-file", "body-html", "cc", "from", ...]

# Get required flags
gog schema gmail send --json | jq '[.commands.gmail.commands.send.flags | to_entries[] | select(.value.required == true) | .key]'
```

### Exit Code Handling
```bash
# 0=ok, 1=error, 2=usage, 3=empty, 4=auth, 5=not_found, 6=denied, 7=rate_limited, 8=retryable
gog gmail search "is:inbox" -j --results-only -a $GOG_ACCOUNT
EXIT_CODE=$?
case $EXIT_CODE in
  0) ;; # Success
  3) ;; # Empty (no results) — not an error
  4) echo "AUTH ERROR - run: gog auth doctor --check" ;;
  7) echo "RATE LIMITED - retry later" ;;
  8) echo "RETRYABLE - retry with backoff" ;;
  *) echo "ERROR: exit code $EXIT_CODE" ;;
esac
```

### Dry-Run Before Mutation
```bash
# Always dry-run first, then execute
gog gmail send --to X --subject Y --body Z --dry-run -a $GOG_ACCOUNT
# After approval:
gog gmail send --to X --subject Y --body Z --force -a $GOG_ACCOUNT
```

## Email Composing Workflow

1. **ALWAYS show the draft first.** Write to temp file, display full draft, wait for explicit user approval ("send it" or edited version). NEVER skip this step — even if the user says "handle it."
2. **Style rules:** Apply the `writing-style` skill. Direct, concise, no fluff. No em-dashes. Warm but not apologetic.
3. **Default send account:** `$GOG_ACCOUNT`
4. **Always dry-run first**, then send with `--force` after approval.

### Plain text email
```bash
gog gmail send -a $GOG_ACCOUNT \
  --to recipient@example.com \
  --subject "Subject" \
  --body-file /tmp/email_body.txt \
  --dry-run 2>&1    # show to user, get approval
  # then:
  --force 2>&1      # actually send
```

### HTML email (formatted: bold, links, structure)
```bash
cat > /tmp/email.html << 'EOF'
<html><body>
<p>Hi <b>Name</b>,</p>
<p>Email body with <a href="https://example.com">links</a>.</p>
<p>Best,<br>Ishan</p>
</body></html>
EOF
gog gmail send -a $GOG_ACCOUNT \
  --to recipient@example.com \
  --subject "Subject" \
  --body-html-file /tmp/email.html \
  --force
```

### Email with attachments
```bash
gog gmail send -a $GOG_ACCOUNT \
  --to recipient@example.com \
  --subject "Subject" \
  --body-file /tmp/body.txt \
  --attach /tmp/report.pdf --attach /tmp/data.xlsx \
  --force
```

### Email with CC / BCC
```bash
gog gmail send -a $GOG_ACCOUNT \
  --to primary@example.com \
  --cc colleague@example.com \
  --bcc manager@example.com \
  --subject "Subject" \
  --body-file /tmp/body.txt \
  --force
```

### Reply within an existing thread
```bash
# Step 1: Find the message ID to reply to
gog gmail search "from:sender@example.com" -a $GOG_ACCOUNT --max 5

# Step 2: Read the thread to get full context
gog gmail thread get "<threadId>" -a $GOG_ACCOUNT --full

# Step 3: Reply (auto-populates recipients from original)
gog gmail send -a $GOG_ACCOUNT \
  --reply-to-message-id "<messageId>" \
  --reply-all \
  --subject "Re: Original Subject" \
  --body-file /tmp/reply.txt \
  --force
```

### Reply within a thread using thread ID (uses latest message)
```bash
gog gmail send -a $GOG_ACCOUNT \
  --thread-id "<threadId>" \
  --reply-all \
  --subject "Re: Original Subject" \
  --body-file /tmp/reply.txt \
  --force
```

### Reply with quoted original
```bash
gog gmail send -a $GOG_ACCOUNT \
  --reply-to-message-id "<messageId>" \
  --reply-all \
  --quote \
  --subject "Re: Original Subject" \
  --body-file /tmp/reply.txt \
  --force
```

### Send with signature
```bash
gog gmail send -a $GOG_ACCOUNT \
  --to recipient@example.com \
  --subject "Subject" \
  --body-file /tmp/body.txt \
  --signature \
  --force
```

## Receiving / Reading Email

### List inbox (latest 20)
```bash
gog gmail search "is:inbox" -a $GOG_ACCOUNT --max 20
```

### Search for specific emails
```bash
# From a specific person
gog gmail search "from:aaron@totoh.org" -a $GOG_ACCOUNT --max 10

# By subject
gog gmail search "subject:archetypes is:unread" -a $GOG_ACCOUNT

# Unread only, last 7 days
gog gmail search "is:unread newer_than:7d" -a $GOG_ACCOUNT

# With attachments
gog gmail search "has:attachment newer_than:7d" -a $GOG_ACCOUNT

# Sent mail
gog gmail search "in:sent to:aaron@totoh.org" -a $GOG_ACCOUNT
```

### Read a specific message
```bash
# Sanitized (strips HTML, removes URLs)
gog gmail get "<messageId>" -a $GOG_ACCOUNT --sanitize-content

# Full content
gog gmail get "<messageId>" -a $GOG_ACCOUNT --full

# Raw API dump (for debugging / header inspection)
gog gmail raw "<messageId>" -a $GOG_ACCOUNT
```

### Read a full thread (all messages in conversation)
```bash
gog gmail thread get "<threadId>" -a $GOG_ACCOUNT --full
```

### Download attachments from a thread
```bash
# List attachments
gog gmail thread attachments "<threadId>" -a $GOG_ACCOUNT

# Download all attachments
gog gmail thread attachments "<threadId>" --download --out-dir /tmp/attachments -a $GOG_ACCOUNT

# Download a single attachment by ID
gog gmail attachment "<messageId>" "<attachmentId>" -a $GOG_ACCOUNT --out /tmp/file.pdf
```

## Inbox Management

### Labels
```bash
# List all labels
gog gmail labels list -a $GOG_ACCOUNT -j --results-only

# Create a label
gog gmail labels create "Project-Aaron" -a $GOG_ACCOUNT

# Add/remove labels on a thread
gog gmail labels modify "<threadId>" -a $GOG_ACCOUNT --add "IMPORTANT" --remove "UNREAD"

# Thread-level label modify
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT --add "STARRED" --remove "UNREAD"
```

### Mark as read/unread
```bash
# Single message
gog gmail messages modify "<messageId>" -a $GOG_ACCOUNT --remove "UNREAD"

# Thread
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT --remove "UNREAD"
```

### Star / important
```bash
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT --add "STARRED"
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT --add "IMPORTANT"
```

### Trash / delete
```bash
# Trash by query
gog gmail trash -q "from:spam@example.com" -a $GOG_ACCOUNT --max 100

# Batch delete (permanent)
gog gmail batch delete "<msgId1>" "<msgId2>" -a $GOG_ACCOUNT --force

# Batch modify (e.g., archive multiple)
gog gmail batch modify "<msgId1>" "<msgId2>" -a $GOG_ACCOUNT --add "TRASH" --remove "INBOX"
```

### Filters (automated sorting)
```bash
# List filters
gog gmail settings filters list -a $GOG_ACCOUNT

# Create auto-filter (label + archive)
gog gmail settings filters create -a $GOG_ACCOUNT \
  --from "newsletter@example.com" --add-label "Newsletters" --archive

# Delete filter
gog gmail settings filters delete "<filterId>" -a $GOG_ACCOUNT
```

### Vacation responder
```bash
gog gmail settings vacation get -a $GOG_ACCOUNT
gog gmail settings vacation update -a $GOG_ACCOUNT \
  --subject "Out of Office" --body "Away until July 5." --start 2026-06-28 --end 2026-07-05
```

## Gmail Web URL (for sharing)
```bash
# Get the Gmail web URL for a thread
gog gmail url "<threadId>" -a $GOG_ACCOUNT
# Output: https://mail.google.com/mail/u/0/#inbox/<threadId>
```

### Calendar
```bash
gog calendar events -a $GOG_ACCOUNT --days 7 --max 20
gog calendar create primary -a $GOG_ACCOUNT \
  --summary "Meeting" --from RFC3339 --to RFC3339 --with-meet
gog calendar changed -a $GOG_ACCOUNT --since 24h
```

### Drive / Docs / Sheets / Slides
```bash
gog drive ls -a $GOG_ACCOUNT
gog docs write <docId> -a $GOG_ACCOUNT --markdown --file /tmp/content.md
gog sheets get <sheetId> "A1:D10" -a $GOG_ACCOUNT
gog slides create-from-markdown "Deck" -a $GOG_ACCOUNT --content-file /tmp/slides.md
```

### Contacts / Tasks
```bash
gog contacts search "John" -a $GOG_ACCOUNT
gog contacts dedupe -a $GOG_ACCOUNT --apply
gog tasks add "@default" --title "Todo" -a $GOG_ACCOUNT
gog tasks list "@default" -a $GOG_ACCOUNT --show-completed
```

### MCP Server (for agent integration)
```bash
gog mcp --list-tools -a $GOG_ACCOUNT
gog mcp -a $GOG_ACCOUNT --allow-tool "gmail.*,docs_get,sheets"
gog mcp -a $GOG_ACCOUNT --allow-tool "all" --allow-write
```

### Auth Diagnostics
```bash
gog auth doctor --check -j --results-only
gog auth list -j --results-only
gog auth status
```

## Pitfalls

⚠️ **`config.json` must have `default_account` set.** If `config.json` only has `"keyring_backend": "file"` with no `default_account`, gog will not know which token to use and commands fail silently. Fix: `echo '{"keyring_backend":"file","default_account":"$GOG_ACCOUNT"}' > ~/.config/gogcli/config.json`. Always verify with `cat ~/.config/gogcli/config.json` before debugging auth issues.

gog is the sole email/calendar/workspace tool.

⚠️ **`--body-file` contains body text only.** Do NOT put headers in the body file.

⚠️ **`gog calendar create --rrule` is BROKEN** (confirmed through v0.31.1). Use individual events.

⚠️ **Only `$GOG_ACCOUNT` works** — other accounts are blocked or expired.

⚠️ **Keep requires Workspace** — needs service account with domain-wide delegation.

⚠️ **Zoom uses its own OAuth** — not Google credentials. Run `gog zoom auth setup` separately.

⚠️ **`--readonly` is global** — when set, ALL write operations fail.

⚠️ **`--wrap-untrusted` wraps text fields** — affects output parsing; only use when security matters.

⚠️ **`gog gmail reply` does NOT exist.** Use `gog gmail send --reply-to-message-id <msgId> --reply-all --subject "Re: ..." --body-file /tmp/body.txt --force`. Requires `--subject` and `--reply-all` (or `--to`). See the Email Composing Workflow section above for full reply patterns.

⚠️ **`gog gmail batch modify` — no backslash continuation.** Message IDs must all be on ONE line. `\\` breaks in bash — the shell interprets the next line as separate commands. Always write the full command on one line.

⚠️ **`gog gmail reply` / `gog gmail reply-all` subcommands do NOT exist.** Use `gog gmail send` with reply flags: `--reply-to-message-id <id> --reply-all --subject "Re: ..." --body-file <file> --force`. See references/gog-gmail-advanced.md for full flags.

⚠️ **HTML email formatting.** Use `--body-html-file /tmp/email.html` for HTML emails. Use `--body-file` for plain text. For rich formatting (bold, links, structured layout), write HTML to a temp file and send with `--body-html-file`. Example:
```bash
cat > /tmp/email.html << 'EOF'
<html><body>
<p>Hi <b>Name</b>,</p>
<p>Email body with <a href="https://example.com">links</a> and formatting.</p>
<p>Best,<br>Ishan</p>
</body></html>
EOF
gog gmail send -a $GOG_ACCOUNT \
  --to recipient@example.com --subject "Subject" \
  --body-html-file /tmp/email.html --force
```

⚠️ **Sending attachments.** Use `--attach /path/to/file` (repeatable for multiple files):
```bash
gog gmail send -a $GOG_ACCOUNT \
  --to recipient@example.com --subject "Subject" --body-file /tmp/body.txt \
  --attach /tmp/report.pdf --attach /tmp/data.xlsx --force
```

⚠️ **Threading / replying to a conversation.** To reply within an existing thread, use either `--reply-to-message-id <msgId>` (sets In-Reply-To/References headers + auto-threads) or `--thread-id <threadId>` (uses latest message in thread for headers). Always pair with `--reply-all` to auto-populate recipients, or `--to` explicitly. `--quote` includes the quoted original message in the reply body. Use `--subject "Re: ..."` (required even for replies, inherited with Re: prefix).

⚠️ **gog binary keyring password for subprocess.** When running gog from Python subprocess (not shell), set `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD` in the env dict. Shell has it via `~/.bashrc` but subprocess does NOT inherit it.

⚠️ **Social Forge MCP does NOT have Google Calendar/Gmail.** All Google Workspace operations MUST go through `gog` CLI. Do not use `mcp_social_forge_goog_*` tools — they will fail with "Google not connected".

## Token Management

### gog binary tokens
Stored in `~/.config/gogcli/keyring/` (PBES2-HS256+A128KW encrypted).

### Python fallback tokens
Stored in `~/.config/google_auth/tokens/` (plain JSON).
```bash
gog auth add <email> <refresh_token>
```

### OAuth re-auth
See `references/gog-reauth-recipe.md`.

## Key Files

| File | Purpose |
|------|---------|
| `gog` | gog binary (v0.31.1) |
| `gog.v0.15.0.bak` | Backup of previous version |
| `gog auth` | Python fallback (read-only) |
| `~/google-oauth-setup.py` | OAuth URL generator + code saver |
| `~/google-oauth-console.py` | Complete interactive OAuth flow |
| `~/.config/gogcli/config.json` | gog binary config |
| `~/.config/gogcli/credentials.json` | OAuth client credentials |
| `~/.config/google_auth/tokens/` | Python fallback token store |
