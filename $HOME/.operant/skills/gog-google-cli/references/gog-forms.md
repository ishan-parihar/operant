# gog forms — Google Forms Reference

Create, edit, and manage Google Forms with questions, responses, and watches.

## Commands

### get (info, show) — Get a form
```bash
gog forms get "<formId>" --account $GOG_ACCOUNT
```

### create (new) — Create a form
```bash
gog forms create --account $GOG_ACCOUNT   --title "Feedback Survey" --description "Please share your thoughts"
```

### update (edit) — Update form settings
```bash
gog forms update "<formId>" --account $GOG_ACCOUNT   --title "Updated Title" --quiz true
```
| Flag | Description |
|------|-------------|
| `--title=STRING` | New title |
| `--description=STRING` | New description |
| `--quiz=STRING` | Enable/disable quiz (true/false) |

### publish — Publish or unpublish
```bash
gog forms publish "<formId>" --account $GOG_ACCOUNT
gog forms publish "<formId>" --unpublish --account $GOG_ACCOUNT
```

### questions add (create, new) — Add a question
```bash
# Text question
gog forms questions add "<formId>" --account $GOG_ACCOUNT   --title "What is your name?" --type text --required

# Multiple choice
gog forms questions add "<formId>" --account $GOG_ACCOUNT   --title "Preferred language?" --type radio --option "English" --option "Hindi" --required

# Scale question
gog forms questions add "<formId>" --account $GOG_ACCOUNT   --title "Rate your experience" --type scale --scale-low 1 --scale-high 5   --scale-low-label "Poor" --scale-high-label "Excellent"
```
| Flag | Description |
|------|-------------|
| `--title=STRING` | Question title (required) |
| `--type="text"` | text, paragraph, radio, checkbox, dropdown, scale, date, time |
| `--required` | Make required |
| `--option=OPTION,...` | Answer options (for radio/checkbox/dropdown) |
| `--index=-1` | Position (0-based, -1 = append) |
| `--correct=CORRECT,...` | Correct answers (quiz mode) |
| `--points=INT` | Points (quiz mode) |
| `--scale-low=1` | Scale low value |
| `--scale-high=5` | Scale high value |
| `--description=STRING` | Question description |

### questions delete (rm, remove) — Delete a question
```bash
gog forms questions delete "<formId>" 2 --account $GOG_ACCOUNT
```

### questions move — Move a question
```bash
gog forms questions move "<formId>" 0 3 --account $GOG_ACCOUNT
```

### responses list (ls) — List form responses
```bash
gog forms responses list "<formId>" --account $GOG_ACCOUNT --max 20
```
| Flag | Description |
|------|-------------|
| `--max=20` | Max results |
| `--filter=STRING` | Filter responses |

### responses get (info, show) — Get a response
```bash
gog forms responses get "<formId>" "<responseId>" --account $GOG_ACCOUNT
```

### watch — Manage response watches (Cloud Pub/Sub)
- `gog forms watch create <formId> --topic <topic>` — Create watch
- `gog forms watch list <formId>` — List watches
- `gog forms watch delete <formId> <watchId>` — Delete watch
- `gog forms watch renew <formId> <watchId>` — Renew watch (extends 7 days)

### raw — Dump raw API response
```bash
gog forms raw "<formId>" --account $GOG_ACCOUNT --pretty
```
