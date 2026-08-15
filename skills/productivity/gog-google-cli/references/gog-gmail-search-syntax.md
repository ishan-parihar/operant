# Gmail Search Syntax — Quick Reference

For use with `gog gmail search "<query>"`. Full syntax: https://support.google.com/mail/answer/7126229

## Basic Operators

| Operator | Example | Description |
|----------|---------|-------------|
| `from:` | `from:boss@gmail.com` | From specific sender |
| `to:` | `to:me@gmail.com` | Sent to specific recipient |
| `subject:` | `subject:meeting` | Subject contains word |
| `subject:` | `subject:(team meeting)` | Subject contains phrase |
| `is:` | `is:unread` | Unread messages |
| `is:` | `is:read` | Read messages |
| `is:` | `is:starred` | Starred messages |
| `is:` | `is:important` | Important messages |
| `is:` | `is:spam` | Spam messages |
| `is:` | `is:trash` | Trash messages |
| `label:` | `label:work` | Has specific label |
| `has:` | `has:attachment` | Has attachment |
| `category:` | `category:promotions` | In promotions category |
| `category:` | `category:social` | In social category |
| `category:` | `category:updates` | In updates category |
| `category:` | `category:forums` | In forums category |
| `category:` | `category:personal` | In personal category |

## Time Operators

| Operator | Example | Description |
|----------|---------|-------------|
| `newer_than:` | `newer_than:1d` | Received in last 1 day |
| `newer_than:` | `newer_than:7d` | Received in last 7 days |
| `newer_than:` | `newer_than:1m` | Received in last 1 month |
| `older_than:` | `older_than:1y` | Received more than 1 year ago |
| `before:` | `before:2026/01/01` | Before specific date |
| `after:` | `after:2026/06/01` | After specific date |

## Numeric Operators

| Operator | Example | Description |
|----------|---------|-------------|
| `larger:` | `larger:10M` | Larger than 10MB |
| `larger:` | `larger:5M` | Larger than 5MB |
| `smaller:` | `smaller:1M` | Smaller than 1MB |

## Boolean / Combinations

| Syntax | Example | Description |
|--------|---------|-------------|
| `OR` | `from:alice OR from:bob` | Either condition |
| `-` | `-from:newsletter` | NOT (exclude) |
| `()` | `(from:alice subject:meeting)` | Group conditions |
| `"` | `subject:"quarterly report"` | Exact phrase |

## Common Search Patterns

### Spam / Unsubscribe Candidates
```bash
# Promotions in inbox
gog gmail search "is:inbox category:promotions" -a $GOG_ACCOUNT

# Bulk mail (unsubscribe candidates)
gog gmail search "is:inbox category:updates larger:500k" -a $GOG_ACCOUNT

# Newsletter senders (common patterns)
gog gmail search "is:inbox (category:promotions OR category:updates) newer_than:30d" -a $GOG_ACCOUNT

# Mailing lists
gog gmail search "is:inbox list: unsubscribe -category:social" -a $GOG_ACCOUNT
```

### Finding Specific Emails
```bash
# From specific person
gog gmail search "from:boss@gmail.com newer_than:7d" -a $GOG_ACCOUNT

# With attachments
gog gmail search "is:inbox has:attachment larger:1M" -a $GOG_ACCOUNT

# Unread messages
gog gmail search "is:inbox is:unread" -a $GOG_ACCOUNT

# Specific subject
gog gmail search "subject:invoice is:unread" -a $GOG_ACCOUNT
```

### Date Ranges
```bash
# Last 24 hours
gog gmail search "newer_than:1d" -a $GOG_ACCOUNT

# Last week
gog gmail search "newer_than:7d" -a $GOG_ACCOUNT

# Specific month
gog gmail search "after:2026/06/01 before:2026/07/01" -a $GOG_ACCOUNT
```

### Cleanup
```bash
# Trash old promotions (dry-run first)
gog gmail search "category:promotions older_than:30d" -a $GOG_ACCOUNT --max 100

# Archive read updates
gog gmail search "category:updates is:read older_than:7d" -a $GOG_ACCOUNT --max 50
```
