# gog keep — Google Keep Reference

**⚠️ Workspace only** — requires service account with domain-wide delegation.

## Commands

### list — List notes
```bash
gog keep list --account $GOG_ACCOUNT --max 100
```
| Flag | Description |
|------|-------------|
| `--max=100` | Max results |
| `--filter=STRING` | Filter notes |
| `--all` | Fetch all pages |

### get — Get a note
```bash
gog keep get "notes/abc123" --account $GOG_ACCOUNT
```

### search — Search notes by text (client-side)
```bash
gog keep search "meeting notes" --account $GOG_ACCOUNT --max 500
```

### create — Create a note
```bash
# Simple note
gog keep create --account $GOG_ACCOUNT   --title "Quick Note" --text "Remember to call dentist"

# Checklist
gog keep create --account $GOG_ACCOUNT   --title "Shopping" --item "Milk" --item "Eggs" --item "Bread"
```
| Flag | Description |
|------|-------------|
| `--title=STRING` | Note title |
| `--text=STRING` | Note body text |
| `--item=ITEM,...` | Checklist items (repeatable) |

### delete — Delete a note
```bash
gog keep delete "notes/abc123" --account $GOG_ACCOUNT
```

### attachment — Download an attachment
```bash
gog keep attachment "notes/abc123/attachments/xyz789" --account $GOG_ACCOUNT   --out /tmp/attachment.bin
```
