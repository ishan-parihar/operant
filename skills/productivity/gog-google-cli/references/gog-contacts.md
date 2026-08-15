# gog contacts — Google Contacts Reference

Search, list, create, update, delete, export, and deduplicate contacts.

## Commands

### search — Search contacts by name/email/phone
```bash
gog contacts search "<query>" --account $GOG_ACCOUNT --max 50
```
| Flag | Description |
|------|-------------|
| `--max=50` | Max results |

### list (ls) — List contacts
```bash
gog contacts list --account $GOG_ACCOUNT --max 100 -j
```
| Flag | Description |
|------|-------------|
| `--max=100` | Max results |
| `--page=STRING` | Page token |

### get (info, show) — Get a contact
```bash
gog contacts get "people/abc123" --account $GOG_ACCOUNT
# Or by email:
gog contacts get "user@example.com" --account $GOG_ACCOUNT
```

### export — Export contacts as vCard (.vcf)
```bash
# Export by query
gog contacts export --query "John" --account $GOG_ACCOUNT --out /tmp/contacts.vcf

# Export all personal contacts
gog contacts export --all --account $GOG_ACCOUNT --out /tmp/all_contacts.vcf
```
| Flag | Description |
|------|-------------|
| `--query=STRING` | Search query (max 30 results) |
| `--all` | Export all personal contacts |
| `--out="-"` | Output path (.vcf), or - for stdout |
| `--max=30` | Max results for --query (1-30) |
| `--page-size=1000` | Page size for --all (1-1000) |

### dedupe — Find and optionally merge duplicate contacts
```bash
gog contacts dedupe --account $GOG_ACCOUNT --match "email,phone" --max 0
# Apply merges:
gog contacts dedupe --account $GOG_ACCOUNT --apply
```
| Flag | Description |
|------|-------------|
| `--match="email,phone"` | Fields to match on |
| `--max=0` | Max duplicates (0 = unlimited) |
| `--resource=RESOURCE,...` | Restrict to specific resource names |
| `--apply` | Actually merge (dry-run without) |
| `--fail-empty` | Exit code 3 if no duplicates found |

### create (add, new) — Create a contact
```bash
gog contacts create --account $GOG_ACCOUNT   --given "John" --family "Doe" --email "john@example.com" --phone "+1234567890"   --org "Acme" --title "Engineer" --note "Met at conference"
```
| Flag | Description |
|------|-------------|
| `--given=STRING` | First name (required) |
| `--family=STRING` | Last name |
| `--email=STRING` | Email address |
| `--phone=STRING` | Phone number |
| `--org=STRING` | Organization |
| `--title=STRING` | Job title |
| `--url=URL,...` | URLs (repeatable) |
| `--note=STRING` | Notes |
| `--address=ADDRESS;...` | Address |
| `--gender=STRING` | Gender |
| `--custom=CUSTOM,...` | Custom fields |
| `--relation=RELATION,...` | Relations |

### update (edit, set) — Update a contact
```bash
gog contacts update "people/abc123" --account $GOG_ACCOUNT   --title "Senior Engineer" --org "New Corp"
```
Same flags as create, plus `--from-file=STRING`, `--ignore-etag`, `--birthday=STRING`.

### delete (rm, del, remove) — Delete a contact
```bash
gog contacts delete "people/abc123" --account $GOG_ACCOUNT
```

### raw — Dump raw People API response
```bash
gog contacts raw "people/abc123" --account $GOG_ACCOUNT --person-fields "names,emailAddresses" --pretty
```

### directory — Workspace directory (requires Workspace account)
- `gog contacts directory list` — List Workspace directory people
- `gog contacts directory search "<query>"` — Search Workspace directory

### other — Other contacts (auto-collected)
- `gog contacts other list` — List other contacts
- `gog contacts other search "<query>"` — Search other contacts
