---
name: email
version: 1.0.0
category: productivity
description: Send, receive, search, thread, reply, attachments, labels, inbox management. The ONLY email tool. Use for any Gmail operation: composing drafts, sending mail, reading inbox, threading conversations, managing labels, filtering, downloading attachments.
triggers:
  - When user asks to send/read/check email
  - When drafting any email (ALWAYS show draft first)
  - When searching inbox by sender, subject, or date
  - When downloading email attachments
  - When managing labels, filters, or auto-sorting
  - When generating Gmail web URLs to share threads
  - When user says "email", "mail", "gmail", "inbox", "compose"
metadata:
  operant:
    tags: [email, gmail, send, inbox, labels, attachments]
---

# Email

All Gmail operations via the `gog` CLI (binary at `gog`, v0.31.1). This is the ONLY email tool. Never use anything else for email.

**Account:** `$GOG_ACCOUNT` (default)
**Auth diagnostics:** See `google-auth` skill if commands fail with exit code 4.

## Quick Start

```bash
# Health check (auth + working state)
gog auth doctor --check -j --results-only

# List inbox (latest 20)
gog gmail search "is:inbox" -a $GOG_ACCOUNT --max 20

# Send test (always dry-run first)
gog gmail send --to recipient@example.com --subject "Test" --body-file /tmp/body.txt -a $GOG_ACCOUNT --dry-run
```

## Default Flags (Always Use for Agent Context)

```bash
-a $GOG_ACCOUNT   # Explicit account (don't rely on default)
-j --results-only                    # JSON output, primary result only
--no-input                           # Never prompt; fail instead
```

**Subprocess env:** `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD` (must be set in env dict; shell has it via .bashrc but subprocess does NOT inherit)

## Draft-First Protocol (MANDATORY)

**ALWAYS show the draft first.** Write to temp file, display full draft, wait for explicit user approval ("send it" or edited version). NEVER skip this step, even if user says "handle it."

Always obtain explicit approval before sending. This protocol is not optional.

```bash
# 1. Write draft to temp file
cat > /tmp/draft.txt << 'EOF'
Hi Recipient,

Body of email here.

Best,
Ishan
EOF

# 2. Show full draft to user, wait for "send it" or edits

# 3. Dry-run to verify
gog gmail send -a $GOG_ACCOUNT   --to recipient@example.com --subject "Subject"   --body-file /tmp/draft.txt --dry-run

# 4. After approval, send for real
gog gmail send -a $GOG_ACCOUNT   --to recipient@example.com --subject "Subject"   --body-file /tmp/draft.txt --force
```

## Send Email

### Plain text
```bash
gog gmail send -a $GOG_ACCOUNT   --to recipient@example.com   --subject "Subject"   --body-file /tmp/body.txt   --force
```

### HTML formatted (bold, links, structure)
```bash
cat > /tmp/email.html << 'EOF'
<html><body>
<p>Hi <b>Name</b>,</p>
<p>Email body with <a href="https://example.com">links</a> and formatting.</p>
<p>Best,<br>Ishan</p>
</body></html>
EOF
gog gmail send -a $GOG_ACCOUNT   --to recipient@example.com --subject "Subject"   --body-html-file /tmp/email.html --force
```

### With attachments
```bash
gog gmail send -a $GOG_ACCOUNT   --to recipient@example.com --subject "Subject"   --body-file /tmp/body.txt   --attach /tmp/report.pdf --attach /tmp/data.xlsx   --force
```

### With CC / BCC
```bash
gog gmail send -a $GOG_ACCOUNT   --to primary@example.com   --cc colleague@example.com   --bcc manager@example.com   --subject "Subject"   --body-file /tmp/body.txt --force
```

### With signature (Gmail signature)
```bash
gog gmail send -a $GOG_ACCOUNT   --to recipient@example.com --subject "Subject"   --body-file /tmp/body.txt --signature --force
```

### With open tracking
```bash
gog gmail send -a $GOG_ACCOUNT   --to recipient@example.com --subject "Subject"   --body-file /tmp/body.txt --track --force
```

## Reply / Threading

⚠️ `gog gmail reply` and `gog gmail reply-all` subcommands do NOT exist. Use `gog gmail send` with reply flags.

### Reply by message ID (auto-threads + sets In-Reply-To)
```bash
# Find the message ID first
gog gmail search "from:sender@example.com" -a $GOG_ACCOUNT --max 5

# Reply (--reply-all auto-populates recipients from original)
gog gmail send -a $GOG_ACCOUNT   --reply-to-message-id "<messageId>"   --reply-all   --subject "Re: Original Subject"   --body-file /tmp/reply.txt   --force
```

### Reply by thread ID (uses latest message)
```bash
gog gmail send -a $GOG_ACCOUNT   --thread-id "<threadId>"   --reply-all   --subject "Re: Original Subject"   --body-file /tmp/reply.txt   --force
```

### Reply with quoted original
```bash
gog gmail send -a $GOG_ACCOUNT   --reply-to-message-id "<messageId>"   --reply-all --quote   --subject "Re: Original Subject"   --body-file /tmp/reply.txt --force
```

### Send from different alias (send-as must be verified)
```bash
gog gmail send --from alias@example.com   --to recipient@example.com --subject "Subject"   --body-file /tmp/body.txt --force
```

## Receive / Read Email

### Search inbox

**Search by sender:**
```bash
gog gmail search "from:aaron@totoh.org" -a $GOG_ACCOUNT --max 10
```

**Search by subject:**
```bash
gog gmail search "subject:archetypes is:unread" -a $GOG_ACCOUNT
```

**Search unread only:**
```bash
gog gmail search "is:unread newer_than:7d" -a $GOG_ACCOUNT
```

**Search with attachments:**
```bash
gog gmail search "has:attachment newer_than:7d" -a $GOG_ACCOUNT
```

**Search sent mail:**
```bash
gog gmail search "in:sent to:aaron@totoh.org" -a $GOG_ACCOUNT
```

### Gmail Search Syntax Reference

| Operator | Example | Description |
|----------|---------|-------------|
| `from:` | `from:boss@gmail.com` | From specific sender |
| `to:` | `to:me@gmail.com` | Sent to specific recipient |
| `subject:` | `subject:meeting` | Subject contains word |
| `subject:` | `subject:(team meeting)` | Subject contains phrase |
| `is:` | `is:unread` / `is:read` / `is:starred` / `is:important` | State |
| `label:` | `label:work` | Has specific label |
| `has:` | `has:attachment` | Has attachment |
| `category:` | `category:promotions` | In category |
| `newer_than:` | `newer_than:7d` | Received in last N days |
| `older_than:` | `older_than:1y` | Older than N years |
| `after:` / `before:` | `after:2026/06/01` | Date range |
| `larger:` / `smaller:` | `larger:10M` | Size filter |
| `OR` / `-` / `()` | `from:a OR from:b`, `-newsletter`, `(a b)` | Boolean |
| `"phrase"` | `subject:"exact match"` | Exact phrase |

Full reference: https://support.google.com/mail/answer/7126229

### Read a specific message

**Sanitized (strips HTML, removes URLs):**
```bash
gog gmail get "<messageId>" -a $GOG_ACCOUNT --sanitize-content
```

**Full content:**
```bash
gog gmail get "<messageId>" -a $GOG_ACCOUNT --full
```

**Raw API dump (for header inspection):**
```bash
gog gmail raw "<messageId>" -a $GOG_ACCOUNT
```

### Read full thread (all messages)

```bash
gog gmail thread get "<threadId>" -a $GOG_ACCOUNT --full
```

### Download attachments

```bash
# List attachments in thread
gog gmail thread attachments "<threadId>" -a $GOG_ACCOUNT

# Download all attachments from thread
gog gmail thread attachments "<threadId>" --download --out-dir /tmp/attachments -a $GOG_ACCOUNT

# Download single attachment by ID
gog gmail attachment "<messageId>" "<attachmentId>" -a $GOG_ACCOUNT --out /tmp/file.pdf
```

## Inbox Management

### Labels

```bash
# List all labels
gog gmail labels list -a $GOG_ACCOUNT -j --results-only

# Create a label
gog gmail labels create "Project-Aaron" -a $GOG_ACCOUNT

# Add/remove labels on thread
gog gmail labels modify "<threadId>" -a $GOG_ACCOUNT --add "IMPORTANT" --remove "UNREAD"

# Thread-level label modify
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT --add "STARRED" --remove "UNREAD"
```

### Mark read/unread, star, important

```bash
# Single message
gog gmail messages modify "<messageId>" -a $GOG_ACCOUNT --remove "UNREAD"

# Thread
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT --remove "UNREAD"
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT --add "STARRED"
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT --add "IMPORTANT"
```

### Trash / batch delete

```bash
# Trash by query
gog gmail trash -q "from:spam@example.com" -a $GOG_ACCOUNT --max 100

# Batch delete (PERMANENT)
gog gmail batch delete "<msgId1>" "<msgId2>" -a $GOG_ACCOUNT --force

# Batch archive (remove from inbox, keep in label)
gog gmail batch modify "<msgId1>" "<msgId2>" -a $GOG_ACCOUNT --remove "INBOX"
```

⚠️ **Batch modify: no backslash continuation.** All message IDs must be on ONE line. `\\` breaks in bash — the shell interprets the next line as separate commands.

### Filters (automated sorting)

```bash
# List filters
gog gmail settings filters list -a $GOG_ACCOUNT

# Create auto-filter (label + archive)
gog gmail settings filters create -a $GOG_ACCOUNT   --from "newsletter@example.com" --add-label "Newsletters" --archive

# Delete filter
gog gmail settings filters delete "<filterId>" -a $GOG_ACCOUNT
```

### Vacation responder

```bash
gog gmail settings vacation get -a $GOG_ACCOUNT
gog gmail settings vacation update -a $GOG_ACCOUNT   --subject "Out of Office" --body "Away until July 5." --start 2026-06-28 --end 2026-07-05
```

### Other settings

```bash
# Forwarding
gog gmail settings forwarding list -a $GOG_ACCOUNT
gog gmail settings forwarding create "fwd@example.com" -a $GOG_ACCOUNT

# Send-as aliases
gog gmail settings sendas list -a $GOG_ACCOUNT

# Delegates
gog gmail settings delegates list -a $GOG_ACCOUNT
```

## Gmail Web URL (for sharing)

```bash
# Get the Gmail web URL for a thread
gog gmail url "<threadId>" -a $GOG_ACCOUNT
# Output: https://mail.google.com/mail/u/0/#inbox/<threadId>
```

## Open Tracking

```bash
# List tracked emails
gog gmail track list -a $GOG_ACCOUNT

# Get tracking stats for a specific message
gog gmail track get "<messageId>" -a $GOG_ACCOUNT
```

## History (incremental sync)

```bash
# Full history since a given start ID
gog gmail history -a $GOG_ACCOUNT --start-history-id <id>
```

## Pitfalls

⚠️ **`gog gmail reply` / `gog gmail reply-all` subcommands do NOT exist.** Use `gog gmail send --reply-to-message-id <id> --reply-all --subject "Re: ..." --body-file <file> --force`.

⚠️ **`--subject` is required even for replies.** Without it, gog returns `required: --subject`. Use `"Re: Original Subject"` prefix.

⚠️ **`--reply-all` requires `--reply-to-message-id` or `--thread-id`.** Otherwise it errors out.

⚠️ **`--body-file` contains body text only.** Do NOT put headers (From/To/Subject) in the body file.

⚠️ **`--body-html-file` for HTML formatted email.** Pair with no `--body-file`.

⚠️ **`--attach` is repeatable** for multiple files: `--attach /tmp/a.pdf --attach /tmp/b.xlsx`.

⚠️ **`--gmail-no-send` flag blocks ALL Gmail send operations** (agent safety; useful for read-only contexts).

⚠️ **`--readonly` is global.** When set, ALL write operations fail (not just gmail).

⚠️ **Exit codes:**
- 0 = success
- 3 = empty (no results, not an error)
- 4 = auth error (see google-auth skill)
- 7 = rate limited (retry with backoff)
- 8 = retryable (retry with backoff)

⚠️ **Always use `-a $GOG_ACCOUNT` explicitly.** Don't rely on default_account; explicit is safer.

⚠️ **AI agent context:** Always use `-j --results-only --no-input`. For subprocess: also set `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD` in env.

⚠️ **Draft-first protocol is MANDATORY.** Show the full draft text, wait for explicit approval. Never skip, even if user says "handle it."

⚠️ **`--wrap-untrusted` wraps text fields** in security markers. Use when processing emails from unknown senders; affects output parsing.

⚠️ **Apply writing-style skill to all drafts.** Direct, concise, no em-dashes. See `writing-style` skill.
