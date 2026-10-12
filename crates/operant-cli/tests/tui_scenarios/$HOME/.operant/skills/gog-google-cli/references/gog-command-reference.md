# gog v0.15.0 — Complete Command Reference

Auto-generated from `gog --help` output (v0.15.0 baseline). Core commands (gmail, calendar, drive) are stable across versions. For deep flags on docs/sheets/slides and new services, see the dedicated reference files (built from v0.31.1). Always verify with `gog <command> --help` — the command surface moves fast.

## Global Flags

| Flag | Description |
|------|-------------|
| `-a, --account=STRING` | Account email (required for most commands) |
| `-j, --json` | JSON output (best for scripting) |
| `-p, --plain` | Stable parseable text (TSV, no colors) |
| `-n, --dry-run` | Print intended actions, don't execute |
| `-y, --force` | Skip confirmations for destructive commands |
| `--no-input` | Never prompt; fail instead (CI/agent safety) |
| `--gmail-no-send` | Block Gmail send operations |
| `--results-only` | JSON mode: emit only primary result (drops envelope fields) |
| `--select=STRING` | JSON mode: select comma-separated fields |
| `--verbose` | Enable verbose logging |

## Top-Level Aliases

| Alias | Resolves To |
|-------|-------------|
| `gog send` | `gog gmail send` |
| `gog ls` | `gog drive ls` |
| `gog search` | `gog drive search` |
| `gog open` | `gog drive` web URL |
| `gog download` | `gog drive download` |
| `gog upload` | `gog drive upload` |
| `gog login` | `gog auth add` |
| `gog logout` | `gog auth remove` |
| `gog status` | `gog auth status` |
| `gog me` / `gog whoami` | `gog people me` |

---

## 1. Gmail (`gog gmail`)

### Read

#### `gog gmail search <query>` (aliases: find, query, ls, list)
Search threads using Gmail query syntax.

| Flag | Description |
|------|-------------|
| `--max=10` | Max results |
| `--page=STRING` | Page token |
| `--all` | Fetch all pages |
| `--fail-empty` | Exit code 3 if no results |
| `--oldest` | Show first message date instead of last |
| `--timezone=STRING` | Output timezone (IANA, e.g. America/New_York) |
| `--local` | Use local timezone |

#### `gog gmail get <messageId>` (aliases: info, show)
Get a message (full|metadata|raw).

| Flag | Description |
|------|-------------|
| `--format=full` | Message format: `full`, `metadata`, `raw` |
| `--headers=STRING` | Metadata headers (comma-separated; metadata format only) |
| `--sanitize-content` | Strip HTML, remove URLs, omit raw payloads |

#### `gog gmail raw <messageId>`
Dump raw Gmail API response as JSON (lossless, for scripting).

#### `gog gmail attachment <messageId> <attachmentId>`
Download a single attachment.

#### `gog gmail url <threadId>`
Print Gmail web URLs for threads.

#### `gog gmail history`
Gmail history.

### Organize

#### `gog gmail thread <command>`
Thread operations (get, modify).

#### `gog gmail labels <command>`
Label operations:
- `gog gmail labels list` — List labels
- `gog gmail labels get <labelIdOrName>` — Get label details (including counts)
- `gog gmail labels create <name>` — Create a new label
- `gog gmail labels rename <labelIdOrName> <newName>` — Rename a label
- `gog gmail labels style <labelIdOrName>` — Change label color/visibility
- `gog gmail labels modify <threadId> ...` — Modify labels on threads
- `gog gmail labels delete <labelIdOrName>` — Delete a label

#### `gog gmail batch <command>`
- `gog gmail batch delete <messageId> ...` — Permanently delete (needs broader scope; prefer `trash`)
- `gog gmail batch modify <messageId> ...` — Modify labels on multiple messages

#### `gog gmail archive [<messageId> ...]`
Archive messages (remove from inbox).

| Flag | Description |
|------|-------------|
| `--query=STRING` | Archive all matching messages |
| `--max=100` | Max messages (with --query) |

#### `gog gmail mark-read [<messageId> ...]`
Mark messages as read.

#### `gog gmail unread [<messageId> ...]`
Mark messages as unread.

#### `gog gmail trash [<messageId> ...]`
Move messages to trash.

| Flag | Description |
|------|-------------|
| `--query=STRING` | Trash all matching messages |
| `--max=100` | Max messages (with --query) |

### Write

#### `gog gmail send`
Send an email.

| Flag | Description |
|------|-------------|
| `--to=STRING` | Recipients (comma-separated; required unless --reply-all) |
| `--cc=STRING` | CC recipients |
| `--bcc=STRING` | BCC recipients |
| `--subject=STRING` | Subject (required) |
| `--body=STRING` | Body (plain text) |
| `--body-file=STRING` | Body file path (`-` for stdin) |
| `--body-html=STRING` | Body (HTML) |
| `--reply-to-message-id=STRING` | Reply to message ID (sets In-Reply-To/References) |
| `--thread-id=STRING` | Reply within a thread |
| `--reply-all` | Auto-populate recipients from original |
| `--reply-to=STRING` | Reply-To header address |
| `--attach=ATTACH,...` | Attachment file path (repeatable) |
| `--from=STRING` | Send from verified send-as alias |
| `--signature` | Append Gmail signature from active send-as |
| `--signature-from=STRING` | Append signature from specific send-as |
| `--signature-file=STRING` | Append local signature file |
| `--track` | Enable open tracking |
| `--track-split` | Send tracked messages separately per recipient |
| `--quote` | Include quoted original in reply |

#### `gog gmail forward --to=STRING <messageId>`
Forward a message to new recipients.

| Flag | Description |
|------|-------------|
| `--to=STRING` | Recipients (required) |
| `--cc=STRING` | CC recipients |
| `--bcc=STRING` | BCC recipients |
| `--note=STRING` | Introductory text above forwarded message |
| `--note-file=STRING` | Note file path (`-` for stdin) |
| `--from=STRING` | Send from verified send-as alias |
| `--skip-attachments` | Don't include original attachments |

#### `gog gmail autoreply <query> ...`
Reply once to matching messages.

| Flag | Description |
|------|-------------|
| `--max=20` | Max matching messages to inspect |
| `--subject=STRING` | Override reply subject |
| `--body=STRING` | Reply body (required unless --body-html) |
| `--body-file=STRING` | Reply body file path |
| `--body-html=STRING` | Reply body HTML |
| `--from=STRING` | Send from verified send-as alias |
| `--label="AutoReplied"` | Label to add after replying (dedupe) |
| `--archive` | Archive threads after auto-replying |
| `--mark-read` | Mark threads as read after auto-replying |
| `--skip-bulk` | Skip auto-generated/list mail |
| `--allow-self` | Allow replying to own messages |

#### `gog gmail drafts <command>`
Draft operations.

#### `gog gmail track <command>`
Email open tracking.

#### `gog gmail settings <command>`
Settings and admin.

---

## 2. Calendar (`gog calendar`)

### List & Query

#### `gog calendar calendars`
List calendars.

#### `gog calendar events [<calendarId> ...]` (aliases: list, ls)
List events from a calendar or all calendars.

| Flag | Description |
|------|-------------|
| `--cal=CAL,...` | Calendar ID or name (repeatable) |
| `--calendars=STRING` | Comma-separated calendar IDs/names/indices |
| `--from=STRING` | Start time (RFC3339, date, or relative: today, tomorrow, monday) |
| `--to=STRING` | End time |
| `--today` | Today only |
| `--tomorrow` | Tomorrow only |
| `--week` | This week |
| `--days=0` | Next N days |
| `--max=10` | Max results |
| `--all` | Fetch from all calendars |
| `--query=STRING` | Free text search |
| `--fail-empty` | Exit code 3 if no results |
| `--fields=STRING` | Comma-separated fields to return |
| `--weekday` | Include start/end day-of-week columns |

#### `gog calendar event <calendarId> <eventId>` (aliases: info, show)
Get event.

#### `gog calendar raw <calendarId> <eventId>`
Dump raw Google Calendar API response as JSON.

#### `gog calendar search <query>` (aliases: find, query)
Search events.

#### `gog calendar freebusy [<calendarIds>]`
Get free/busy.

#### `gog calendar conflicts`
Find conflicts.

### Create & Modify

#### `gog calendar create <calendarId>` (aliases: add, new)
Create an event.

| Flag | Description |
|------|-------------|
| `--summary=STRING` | Event title |
| `--from=STRING` | Start time (RFC3339) |
| `--to=STRING` | End time (RFC3339) |
| `--start-timezone=STRING` | IANA timezone for start |
| `--end-timezone=STRING` | IANA timezone for end |
| `--description=STRING` | Description |
| `--location=STRING` | Location |
| `--attendees=STRING` | Comma-separated attendee emails |
| `--all-day` | All-day event (use date-only in --from/--to) |
| `--rrule=RRULE` | Recurrence rules (**BROKEN** — see pitfalls) |
| `--reminder=REMINDER,...` | Custom reminders (e.g. popup:30m, email:1d). Repeatable, max 5 |
| `--event-color=STRING` | Event color ID (1-11) |
| `--visibility=STRING` | default, public, private, confidential |
| `--transparency=STRING` | busy or free |
| `--send-updates=STRING` | all, externalOnly, none (default) |
| `--guests-can-invite` | Allow guests to invite others |
| `--guests-can-modify` | Allow guests to modify |
| `--guests-can-see-others` | Allow guests to see other guests |
| `--with-meet` | Create Google Meet video conference |
| `--event-type=STRING` | default, focus-time, out-of-office, working-location |

#### `gog calendar update <calendarId> <eventId>` (aliases: edit, set)
Update an event.

#### `gog calendar move <calendarId> <eventId> <destinationCalendarId>`
Move event to another calendar.

#### `gog calendar delete <calendarId> <eventId>` (aliases: rm, del, remove)
Delete an event.

#### `gog calendar respond <calendarId> <eventId>` (aliases: rsvp, reply)
Respond to event invitation.

### Utilities

#### `gog calendar subscribe <calendarId>` (aliases: sub, add-calendar)
Add a calendar to your calendar list.

#### `gog calendar create-calendar <summary>` (aliases: new-calendar)
Create a new secondary calendar.

#### `gog calendar acl <calendarId>` (aliases: permissions, perms)
List calendar ACL.

#### `gog calendar colors`
Show calendar colors.

#### `gog calendar focus-time --from=STRING --to=STRING [<calendarId>]`
Create a Focus Time block.

#### `gog calendar out-of-office --from=STRING --to=STRING [<calendarId>]`
Create Out of Office event.

#### `gog calendar working-location --from=STRING --to=STRING --type=STRING [<calendarId>]`
Set working location (home/office/custom).

#### `gog calendar unsubscribe <calendarId>`
Remove a calendar from your calendar list.
```bash
gog calendar unsubscribe <calendarId> -a $GOG_ACCOUNT
```

#### `gog calendar delete-calendar <calendarId>`
Delete a secondary calendar.
```bash
gog calendar delete-calendar <calendarId> -a $GOG_ACCOUNT
```

#### `gog calendar time`
Show server time.
```bash
gog calendar time -a $GOG_ACCOUNT
```

#### `gog calendar users`
List workspace users (use their email as calendar ID).
```bash
gog calendar users -a $GOG_ACCOUNT
```

#### `gog calendar team <group-email>`
Show events for all members of a Google Group.
```bash
gog calendar team group@example.com -a $GOG_ACCOUNT
```

#### `gog calendar alias <command>`
Manage calendar aliases.

#### `gog calendar users`
List workspace users.

#### `gog calendar team <group-email>`
Show events for all members of a Google Group.

---

## 3. Drive (`gog drive`)

### Browse

#### `gog drive ls`
List files in a folder (default: root).

#### `gog drive search <query>`
Full-text search across Drive.

#### `gog drive tree`
Print a read-only folder tree.

#### `gog drive du`
Summarize Drive folder sizes.

#### `gog drive inventory`
Export a read-only Drive inventory.

### File Operations

#### `gog drive get <fileId>`
Get file metadata.

#### `gog drive download <fileId>`
Download a file (exports Google Docs formats).

| Flag | Description |
|------|-------------|
| `--out=STRING` | Output file path |
| `--format=STRING` | Export format: pdf, csv, xlsx, pptx, txt, png, docx, md |
| `--tab=STRING` | Export specific tab (Google Docs only) |

#### `gog drive copy <fileId> <name>`
Copy a file.

#### `gog drive upload <localPath>`
Upload a file.

| Flag | Description |
|------|-------------|
| `--name=STRING` | Override filename |
| `--parent=STRING` | Destination folder ID |
| `--replace=STRING` | Replace existing Drive file ID (preserves shared link) |
| `--mime-type=STRING` | Override MIME type |
| `--convert` | Auto-convert to native Google format |
| `--convert-to=STRING` | Convert to: doc, sheet, slides |
| `--keep-frontmatter` | Keep YAML frontmatter when converting Markdown to Doc |

#### `gog drive mkdir <name>`
Create a folder.

#### `gog drive delete <fileId>` (aliases: rm, del)
Move to trash (`--permanent` to delete forever).

#### `gog drive move <fileId>`
Move file to different folder.

#### `gog drive rename <fileId> <newName>`
Rename file or folder.

### Sharing

#### `gog drive share <fileId>`
Share a file or folder.

#### `gog drive unshare <fileId> <permissionId>`
Remove a permission.

#### `gog drive permissions <fileId>`
List permissions on a file.

### Utilities

#### `gog drive url <fileId>`
Print web URLs for files.

#### `gog drive comments <command>`
Manage comments on files.

#### `gog drive drives`
List shared drives (Team Drives).

#### `gog drive raw <fileId>`
Dump raw Google Drive API response as JSON.

### Drive Extras (v0.31.1)

#### `gog drive shortcut <command>`
Manage shortcuts to Drive files and folders.
- `gog drive shortcut create <fileId> --name "Link" --parent <folderId>`
- `gog drive shortcut list <fileId>`
- `gog drive shortcut delete <shortcutId>`

#### `gog drive url <fileId>`
Print Gmail web URLs for threads.
```bash
gog drive url <fileId> -a $GOG_ACCOUNT
```

#### `gog drive audit <command>`
Audit Drive sharing without mutation.
```bash
gog drive audit list -a $GOG_ACCOUNT
```

#### `gog drive bulk <command>`
Bulk Drive permission operations.
```bash
gog drive bulk add <fileId> --email user@example.com --role reader --type user -a $GOG_ACCOUNT
```

#### `gog drive labels <command>`
Read and modify Drive labels.
```bash
gog drive labels list <fileId> -a $GOG_ACCOUNT
```

---

## 4. Docs (`gog docs`)

| Command | Description |
|---------|-------------|
| `gog docs export <docId>` | Export (pdf, docx, txt, md, html) |
| `gog docs info <docId>` | Get metadata |
| `gog docs create <title>` | Create new doc |
| `gog docs copy <docId> <title>` | Copy doc |
| `gog docs cat <docId>` | Print as plain text |
| `gog docs write <docId>` | Write content |
| `gog docs insert <docId> [<content>]` | Insert text at position |
| `gog docs delete --start=INT --end=INT <docId>` | Delete text range |
| `gog docs find-replace <docId> <find> [<replace>]` | Find and replace |
| `gog docs edit <docId> <find> <replace>` | Find and replace |
| `gog docs sed <docId> [<expression>]` | Regex find/replace (s/pattern/replacement/g) |
| `gog docs format <docId>` | Apply formatting |
| `gog docs clear <docId>` | Clear all content |
| `gog docs structure <docId>` | Show document structure |
| `gog docs raw <docId>` | Dump raw API response |
| `gog docs add-tab <docId>` | Add a tab |
| `gog docs rename-tab <docId>` | Rename a tab |
| `gog docs delete-tab <docId>` | Delete a tab |
| `gog docs list-tabs <docId>` | List all tabs |
| `gog docs comments <command>` | Manage comments |

---

## 5. Sheets (`gog sheets`)

### Read/Write

| Command | Description |
|---------|-------------|
| `gog sheets get <spreadsheetId> <range>` | Get values from range |
| `gog sheets update <spreadsheetId> <range> [<values> ...]` | Update values in range |
| `gog sheets append <spreadsheetId> <range> [<values> ...]` | Append values |
| `gog sheets insert <spreadsheetId> <sheet> <dimension> <start>` | Insert rows/columns |
| `gog sheets clear <spreadsheetId> <range>` | Clear values |
| `gog sheets find-replace <spreadsheetId> <find> <replace>` | Find and replace |

### Formatting

| Command | Description |
|---------|-------------|
| `gog sheets format <spreadsheetId> <range>` | Apply cell formatting |
| `gog sheets conditional-format <spreadsheetId>` | Manage conditional formatting |
| `gog sheets banding <spreadsheetId>` | Manage alternating color banding |
| `gog sheets merge <spreadsheetId> <range>` | Merge cells |
| `gog sheets unmerge <spreadsheetId> <range>` | Unmerge cells |
| `gog sheets number-format <spreadsheetId> <range>` | Apply number format |
| `gog sheets freeze <spreadsheetId>` | Freeze rows/columns |
| `gog sheets resize-columns <spreadsheetId> <columns>` | Resize columns |
| `gog sheets resize-rows <spreadsheetId> <rows>` | Resize rows |
| `gog sheets read-format <spreadsheetId> <range>` | Read cell formatting |

### Metadata

| Command | Description |
|---------|-------------|
| `gog sheets notes <spreadsheetId> <range>` | Get cell notes |
| `gog sheets update-note <spreadsheetId> <range>` | Set/clear cell note |
| `gog sheets links <spreadsheetId> <range>` | Get cell hyperlinks |
| `gog sheets named-ranges <spreadsheetId>` | Manage named ranges |
| `gog sheets table <spreadsheetId>` | Manage tables |
| `gog sheets metadata <spreadsheetId>` | Get spreadsheet metadata |
| `gog sheets chart <spreadsheetId>` | Manage charts |

### Lifecycle

| Command | Description |
|---------|-------------|
| `gog sheets create <title>` | Create new spreadsheet |
| `gog sheets copy <spreadsheetId> <title>` | Copy spreadsheet |
| `gog sheets export <spreadsheetId>` | Export (pdf, xlsx, csv) |
| `gog sheets add-tab <spreadsheetId> <tabName>` | Add new tab |
| `gog sheets rename-tab <spreadsheetId> <oldName> <newName>` | Rename tab |
| `gog sheets delete-tab <spreadsheetId> <tabName>` | Delete tab |
| `gog sheets raw <spreadsheetId>` | Dump raw API response |

---

## 6. Slides (`gog slides`)

| Command | Description |
|---------|-------------|
| `gog slides export <presentationId>` | Export (pdf, pptx) |
| `gog slides info <presentationId>` | Get metadata |
| `gog slides create <title>` | Create new presentation |
| `gog slides create-from-markdown <title>` | Create from markdown |
| `gog slides create-from-template <templateId> <title>` | Create from template |
| `gog slides copy <presentationId> <title>` | Copy presentation |
| `gog slides add-slide <presentationId> <image>` | Add slide with image |
| `gog slides list-slides <presentationId>` | List all slides |
| `gog slides delete-slide <presentationId> <slideId>` | Delete slide |
| `gog slides read-slide <presentationId> <slideId>` | Read slide content |
| `gog slides thumbnail <presentationId> <slideId>` | Get/download thumbnail |
| `gog slides update-notes <presentationId> <slideId>` | Update speaker notes |
| `gog slides replace-slide <presentationId> <slideId> <image>` | Replace slide image |
| `gog slides insert-text <presentationId> <objectId> <text>` | Insert text into element |
| `gog slides replace-text <presentationId> <find> <replacement>` | Find-and-replace |
| `gog slides raw <presentationId>` | Dump raw API response |

---

## 7. Other Commands

### Auth (`gog auth`)
- `gog auth add <email>` (alias: `gog login`) — Authorize and store refresh token
- `gog auth remove <email>` (alias: `gog logout`) — Remove stored token
- `gog auth status` (alias: `gog status`) — Show auth/config status
- `gog auth list` — List accounts

### People (`gog people`)
- `gog people me` (aliases: `gog me`, `gog whoami`) — Show your profile

### Config (`gog config`)
Manage configuration.

### Agent (`gog agent`)
Agent-friendly helpers.

### Schema (`gog schema`)
Machine-readable command/flag schema.

### Backup (`gog backup`)
Encrypted Google account backups.

### Groups (`gog groups`)
Google Groups.

### Admin (`gog admin`)
Google Workspace Admin (Directory API) — requires domain-wide delegation.

---

## Common Workflows

### Search inbox for spam, then trash
```bash
# Find spam
gog gmail search "is:inbox category:promotions" --account $GOG_ACCOUNT --max 50 -j --results-only

# Trash by message ID
gog gmail trash <messageId1> <messageId2> --account $GOG_ACCOUNT

# Trash by query
gog gmail trash -q "from:spam@example.com" --account $GOG_ACCOUNT --max 100
```

### Send email with dry-run first
```bash
gog gmail send --account $GOG_ACCOUNT \
  --to recipient@example.com \
  --subject "Subject" \
  --body-file /tmp/body.txt \
  --dry-run 2>&1

# Then send for real:
gog gmail send --account $GOG_ACCOUNT \
  --to recipient@example.com \
  --subject "Subject" \
  --body-file /tmp/body.txt \
  --force 2>&1
```

### Read email content
```bash
# List recent inbox
gog gmail search "is:inbox" --account $GOG_ACCOUNT --max 10

# Get full message
gog gmail get <messageId> --account $GOG_ACCOUNT --format full

# Get sanitized (agent-friendly)
gog gmail get <messageId> --account $GOG_ACCOUNT --sanitize-content
```

### Calendar: check upcoming events
```bash
gog calendar events --account $GOG_ACCOUNT --days 7 --max 20

# Check specific day
gog calendar events --account $GOG_ACCOUNT --from 2026-06-28 --to 2026-06-29
```

### Drive: upload and share
```bash
gog drive upload /path/to/file.pdf --account $GOG_ACCOUNT --parent <folderId>
gog drive share <fileId> --account $GOG_ACCOUNT --role reader --type anyone
```
