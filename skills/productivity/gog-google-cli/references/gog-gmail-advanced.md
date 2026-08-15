# gog gmail advanced — Thread, Messages, Settings, Raw Reference

Advanced Gmail operations beyond basic search/send.

## Exit Codes
| Code | Meaning |
|------|---------|
| 0 | Success |
| 3 | Empty results |
| 4 | Auth error |
| 7 | Rate limited |
| 8 | Retryable error |

## Thread Operations

### thread get — Get thread with all messages
```bash
gog gmail thread get "<threadId>" -a $GOG_ACCOUNT --full
```
| Flag | Description |
|------|-------------|
| `--download` | Download attachments |
| `--full` | Full message content |
| `--sanitize-content` | Strip HTML, remove URLs |
| `--out-dir=STRING` | Directory for downloaded attachments |

### thread modify — Modify labels on all messages
```bash
gog gmail thread modify "<threadId>" -a $GOG_ACCOUNT   --add "IMPORTANT,STARRED" --remove "UNREAD"
```
| Flag | Description |
|------|-------------|
| `--add=STRING` | Labels to add (comma-separated) |
| `--remove=STRING` | Labels to remove (comma-separated) |

### thread attachments — List/download attachments
```bash
gog gmail thread attachments "<threadId>" -a $GOG_ACCOUNT
gog gmail thread attachments "<threadId>" --download --out-dir /tmp/attachments   -a $GOG_ACCOUNT
```

## Message Operations

### messages search — Search messages (not threads)
```bash
gog gmail messages search "is:unread" -a $GOG_ACCOUNT   --max 20 --include-body --body-format text
```
| Flag | Description |
|------|-------------|
| `--max=10` | Max results |
| `--all` | Fetch all pages |
| `--include-body` | Include message body |
| `--body-format="text"` | text or html |
| `--full` | Full message content |

### messages modify — Modify labels on single message
```bash
gog gmail messages modify "<messageId>" -a $GOG_ACCOUNT   --add "STARRED" --remove "UNREAD"
```

## Raw & Attachment Operations

### raw — Lossless API dump (for debugging/scripting)
```bash
gog gmail raw "<messageId>" -a $GOG_ACCOUNT
# Returns full Users.Messages.Get response as JSON
```

### attachment — Download single attachment
```bash
gog gmail attachment "<messageId>" "<attachmentId>" -a $GOG_ACCOUNT   --out /tmp/attachment.pdf
```

### url — Print Gmail web URLs
```bash
gog gmail url "<threadId>" -a $GOG_ACCOUNT
# Output: https://mail.google.com/mail/u/0/#inbox/<threadId>
```

### history — Gmail history
```bash
gog gmail history -a $GOG_ACCOUNT
```

## Reply Operations

### reply / reply-all — BROKEN in v0.13.0-dev

⚠️ **`gog gmail reply` and `gog gmail reply-all` subcommands do not exist in the installed version.** They return `unexpected argument reply`. The skill's SKILL.md and this reference document them, but the binary doesn't support them.

**Use `gog gmail send` with reply flags instead:**

```bash
# Reply to a specific message (sends to To + CC of original)
gog gmail send -a $GOG_ACCOUNT \
  --reply-to-message-id "<messageId>" \
  --reply-all \
  --subject "Re: Original Subject" \
  --body-file /tmp/reply.txt \
  --force

# Required flags for reply:
#   --reply-to-message-id  (sets In-Reply-To/References headers + thread)
#   --reply-all            (auto-populates recipients from original)
#   --subject              (required even for replies — use "Re: ..." prefix)
#   --body-file or --body  (the reply content)
```

| Flag | Description |
|------|-------------|
| `--reply-to-message-id=STRING` | Reply to Gmail message ID (sets In-Reply-To/References and thread) |
| `--thread-id=STRING` | Reply within a Gmail thread (uses latest message) |
| `--reply-all` | Auto-populate recipients from original message (requires --reply-to-message-id or --thread-id) |
| `--quote` | Include quoted original message in reply (requires --reply-to-message-id or --thread-id) |

**Pitfall:** `--subject` is required even for replies. Without it, gog returns `required: --subject`.

**Pitfall:** Without `--reply-all`, you must provide `--to` explicitly. Using `--reply-all` with `--reply-to-message-id` auto-fills recipients.

### track — Email open tracking
```bash
# List tracked emails
gog gmail track list -a $GOG_ACCOUNT

# Get tracking stats
gog gmail track get "<messageId>" -a $GOG_ACCOUNT
```

## Labels

### labels list — List all labels
```bash
gog gmail labels list -a $GOG_ACCOUNT -j --results-only
```

### labels create — Create a label
```bash
gog gmail labels create "Project-X" -a $GOG_ACCOUNT
```

### labels modify — Add/remove labels on thread
```bash
gog gmail labels modify "<threadId>" -a $GOG_ACCOUNT   --add "IMPORTANT" --remove "UNREAD"
```

### labels delete — Delete a label
```bash
gog gmail labels delete "Old-Label" -a $GOG_ACCOUNT
```

## Batch Operations

### batch delete — Permanently delete multiple messages
```bash
gog gmail batch delete "<msgId1>" "<msgId2>" -a $GOG_ACCOUNT
```

### batch modify — Modify labels on multiple messages
```bash
gog gmail batch modify "<msgId1>" "<msgId2>" -a $GOG_ACCOUNT   --add "TRASH" --remove "INBOX"
```

## Settings

### settings filters — Manage email filters
```bash
gog gmail settings filters list -a $GOG_ACCOUNT
gog gmail settings filters create -a $GOG_ACCOUNT   --from "newsletter@example.com" --add-label "Newsletters" --archive
gog gmail settings filters delete "<filterId>" -a $GOG_ACCOUNT
gog gmail settings filters export -a $GOG_ACCOUNT
```

### settings delegates — Manage mail delegates
```bash
gog gmail settings delegates list -a $GOG_ACCOUNT
gog gmail settings delegates add "delegate@example.com" -a $GOG_ACCOUNT
gog gmail settings delegates remove "delegate@example.com" -a $GOG_ACCOUNT
```

### settings forwarding — Manage forwarding
```bash
gog gmail settings forwarding list -a $GOG_ACCOUNT
gog gmail settings forwarding create "fwd@example.com" -a $GOG_ACCOUNT
gog gmail settings forwarding delete "fwd@example.com" -a $GOG_ACCOUNT
```

### settings sendas — Manage send-as aliases
```bash
gog gmail settings sendas list -a $GOG_ACCOUNT
gog gmail settings sendas create "alias@example.com" -a $GOG_ACCOUNT
gog gmail settings sendas verify "alias@example.com" -a $GOG_ACCOUNT
gog gmail settings sendas update "alias@example.com" -a $GOG_ACCOUNT
gog gmail settings sendas delete "alias@example.com" -a $GOG_ACCOUNT
```

### settings vacation — Vacation responder
```bash
gog gmail settings vacation get -a $GOG_ACCOUNT
gog gmail settings vacation update -a $GOG_ACCOUNT   --subject "Out of Office" --body "Away until July 5." --start 2026-06-28 --end 2026-07-05
```

### settings autoforward — Auto-forwarding
```bash
gog gmail settings autoforward get -a $GOG_ACCOUNT
gog gmail settings autoforward update -a $GOG_ACCOUNT   --email "fwd@example.com" --enable
```

### settings watch — Push notifications
```bash
gog gmail settings watch start -a $GOG_ACCOUNT
gog gmail settings watch status -a $GOG_ACCOUNT
gog gmail settings watch renew -a $GOG_ACCOUNT
gog gmail settings watch stop -a $GOG_ACCOUNT
gog gmail settings watch serve -a $GOG_ACCOUNT  # Long-poll
gog gmail settings watch pull -a $GOG_ACCOUNT   # Pull history
```

## Agent Safety Pattern

```bash
# Read-only agent: use --readonly flag
gog gmail search "is:inbox" --readonly -a $GOG_ACCOUNT -j --results-only

# Wrap untrusted content
gog gmail get "<msgId>" --wrap-untrusted -a $GOG_ACCOUNT -j

# Dry-run before any mutation
gog gmail send --to X --subject Y --body Z --dry-run -a $GOG_ACCOUNT
# Then:
gog gmail send --to X --subject Y --body Z --force -a $GOG_ACCOUNT
```
