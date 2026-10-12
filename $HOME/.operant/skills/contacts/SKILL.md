---
name: contacts
version: 1.0.0
category: productivity
description: Google Contacts operations: search, create, update, delete, dedupe, export. Use for managing address book entries, finding contact details, or cleaning up duplicates.
triggers:
  - When user asks about a person's contact info
  - When adding/updating/removing someone from address book
  - When searching for a contact by name, email, or phone
  - When deduplicating contacts
  - When exporting contacts to .vcf file
metadata:
  operant:
    tags: [contacts, address-book, dedupe, vcf]
---

# Contacts

Google Contacts operations via the `gog` CLI (binary at `gog`, v0.31.1).

**Account:** `$GOG_ACCOUNT`
**Auth diagnostics:** See `google-auth` skill if commands fail with exit code 4.

## Quick Start

```bash
# Health check
gog auth doctor --check -j --results-only

# Search for a contact
gog contacts search "Aaron" -a $GOG_ACCOUNT --max 5

# List all contacts
gog contacts list -a $GOG_ACCOUNT --max 20
```

## Default Flags

```bash
-a $GOG_ACCOUNT   # Explicit account
-j --results-only                    # JSON output
--no-input                           # Never prompt
```

**Subprocess env:** `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD`

## Search Contacts

```bash
# By name
gog contacts search "Aaron Maret" -a $GOG_ACCOUNT --max 5

# By email
gog contacts search "aaron@totoh.org" -a $GOG_ACCOUNT --max 5

# By phone number
gog contacts search "+91-9205112559" -a $GOG_ACCOUNT --max 5

# Free text (searches name, email, phone, org, note)
gog contacts search "TOTOH" -a $GOG_ACCOUNT --max 5
```

## List Contacts

```bash
# All contacts (latest 20)
gog contacts list -a $GOG_ACCOUNT --max 20

# With JSON output for parsing
gog contacts list -a $GOG_ACCOUNT -j --results-only --max 50
```

## Get Contact

Resource name format: `people/{id}` (e.g., `people/c1234567890`)

```bash
gog contacts get "people/c1234567890" -a $GOG_ACCOUNT -j --results-only
```

## Create Contact

```bash
gog contacts create   --name "Aaron Maret"   --email "aaron@totoh.org"   --phone "+1-555-123-4567"   --org "Temple of the Open Heart"   --title "Architect"   --note "Law of One community contact"   -a $GOG_ACCOUNT   --force
```

## Update Contact

```bash
# Update single field
gog contacts update "people/c1234567890"   --title "Senior Architect"   -a $GOG_ACCOUNT   --force

# Update multiple fields
gog contacts update "people/c1234567890"   --email "newemail@example.com"   --phone "+1-555-999-8888"   --note "Updated note"   -a $GOG_ACCOUNT   --force
```

## Delete Contact

```bash
gog contacts delete "people/c1234567890" -a $GOG_ACCOUNT --force
```

⚠️ **Delete is permanent.** There's no trash for contacts. Make sure you have the right resourceName first.

## Dedupe (find and merge duplicates)

```bash
# Dry-run first to see what would be merged
gog contacts dedupe -a $GOG_ACCOUNT --dry-run

# Actually apply dedupe
gog contacts dedupe -a $GOG_ACCOUNT --apply --force
```

## Export Contacts

```bash
# Export all contacts to .vcf
gog contacts export all -a $GOG_ACCOUNT --out /tmp/contacts.vcf

# Export specific group (if you have contact groups)
gog contacts export "group-name" -a $GOG_ACCOUNT --out /tmp/group-contacts.vcf
```

## Directory Search (Workspace only)

```bash
# Search workspace directory for people
gog contacts directory search "John Doe" -a $GOG_ACCOUNT

# Get directory person details
gog contacts directory get "people/d1234567890" -a $GOG_ACCOUNT
```

⚠️ **Directory search requires Workspace.** Will fail on regular Gmail accounts.

## Raw API Dump (for debugging)

```bash
gog contacts raw "people/c1234567890" -a $GOG_ACCOUNT
```

## Pitfalls

⚠️ **resourceName format is `people/{id}`** — NOT email address, NOT phone number. Get it from `get` or `search` output first.

⚠️ **`--email`, `--phone` may need repetition** for multiple values. Test with single value first; if multi-value is needed, repeat the flag: `--email "a@x.com" --email "b@y.com"`.

⚠️ **`--dedupe --apply` actually merges.** ALWAYS dry-run first: `--dedupe --dry-run`. Merged contacts cannot be easily unmerged.

⚠️ **Delete is PERMANENT** (no trash). Verify resourceName before deleting.

⚠️ **Export requires selector**: `all` for everything, or a group name. Just running `gog contacts export` without selector may error.

⚠️ **Directory search requires Workspace.** Regular Gmail accounts will get auth errors or empty results.

⚠️ **Exit codes:**
- 0 = success
- 3 = empty (no results)
- 4 = auth error (see google-auth skill)
- 5 = not_found

⚠️ **Always use `-a $GOG_ACCOUNT` explicitly.**

⚠️ **Subprocess env:** Set `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD` in env dict.

## Quick Reference

```bash
# Search
gog contacts search "<query>" -a $GOG_ACCOUNT --max 10

# Create
gog contacts create --name "Name" --email "email@example.com" -a $GOG_ACCOUNT --force

# Get details
gog contacts get "people/c1234567890" -a $GOG_ACCOUNT

# Update
gog contacts update "people/c1234567890" --field value -a $GOG_ACCOUNT --force

# Delete
gog contacts delete "people/c1234567890" -a $GOG_ACCOUNT --force

# Export
gog contacts export all -a $GOG_ACCOUNT --out /tmp/contacts.vcf
```
