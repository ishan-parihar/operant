# gog chat — Google Chat Reference

Spaces, messages, threads, DMs, and reactions.

## Commands

### spaces list (ls) — List spaces
```bash
gog chat spaces list --account $GOG_ACCOUNT --max 100
```

### spaces find (search, query) — Find spaces by name
```bash
gog chat spaces find "Project X" --account $GOG_ACCOUNT --exact
```
| Flag | Description |
|------|-------------|
| `--max=100` | Max results |
| `--exact` | Exact name match |

### spaces create (add, new) — Create a space
```bash
gog chat spaces create "New Space" --account $GOG_ACCOUNT   --member "user1@example.com,user2@example.com"
```
| Flag | Description |
|------|-------------|
| `--member=MEMBER,...` | Members (email or users/...; repeatable) |

### messages list (ls) — List messages in a space
```bash
gog chat messages list "spaces/abc123" --account $GOG_ACCOUNT --max 50
```
| Flag | Description |
|------|-------------|
| `--max=50` | Max results |
| `--thread=STRING` | Filter by thread |
| `--unread` | Unread only |
| `--order=STRING` | Sort order |

### messages send (create, post) — Send a message
```bash
gog chat messages send "spaces/abc123" --account $GOG_ACCOUNT   --text "Hello team!" --thread "threads/xyz"
```
| Flag | Description |
|------|-------------|
| `--text=STRING` | Message text (required unless --attach) |
| `--thread=STRING` | Thread to post in |
| `--attach=ATTACH,...` | Attachments |

### messages react — Add emoji reaction
```bash
gog chat messages react "spaces/abc123/messages/msg1" "👍" --account $GOG_ACCOUNT
```

### messages reactions — Manage reactions
- `gog chat messages reactions list <message>` — List reactions
- `gog chat messages reactions create <message> <emoji>` — Add reaction
- `gog chat messages reactions delete <reaction>` — Remove reaction

### threads list — List threads in a space
```bash
gog chat threads list "spaces/abc123" --account $GOG_ACCOUNT
```

### dm send (create, post) — Send a direct message
```bash
gog chat dm send "user@example.com" --account $GOG_ACCOUNT   --text "Hey, quick question"
```

### dm space (find, setup) — Find or create DM space
```bash
gog chat dm space "user@example.com" --account $GOG_ACCOUNT
```
