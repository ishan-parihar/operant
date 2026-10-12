# gog people — Google People API Reference

Profile information, directory search, and relations.

## Commands

### me — Show your profile
```bash
gog people me --account $GOG_ACCOUNT -j
```

### get (info, show) — Get a user profile
```bash
gog people get "people/abc123" --account $GOG_ACCOUNT
```

### search (find, query) — Search Workspace directory
```bash
gog people search "John" --account $GOG_ACCOUNT --max 50
```
| Flag | Description |
|------|-------------|
| `--max=50` | Max results |
| `--all` | Fetch all pages |
| `--fail-empty` | Exit code 3 if no results |

### relations — Get user relations
```bash
gog people relations --account $GOG_ACCOUNT --type "coworker"
gog people relations "people/abc123" --account $GOG_ACCOUNT
```
| Flag | Description |
|------|-------------|
| `--type=STRING` | Relation type filter |

### raw — Dump raw People API response
```bash
gog people raw "people/abc123" --account $GOG_ACCOUNT --person-fields "names,emailAddresses" --pretty
```
