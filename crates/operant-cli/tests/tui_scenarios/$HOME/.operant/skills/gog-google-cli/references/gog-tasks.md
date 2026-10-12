# gog tasks — Google Tasks Reference

Manage task lists and tasks with due dates, repetition, and hierarchy.

## Commands

### lists list — List task lists
```bash
gog tasks lists list --account $GOG_ACCOUNT -j
```

### lists create — Create a task list
```bash
gog tasks lists create "My Tasks" --account $GOG_ACCOUNT
```

### list (ls) — List tasks in a list
```bash
gog tasks list "@default" --account $GOG_ACCOUNT --max 50 --show-completed
```
| Flag | Description |
|------|-------------|
| `--max=20` | Max results (max 100) |
| `--all` | Fetch all pages |
| `--show-completed` | Include completed tasks |
| `--show-deleted` | Include deleted tasks |
| `--show-hidden` | Include hidden tasks |
| `--show-assigned` | Include tasks assigned to current user |
| `--due-min=STRING` | Lower bound for due date (RFC3339) |
| `--due-max=STRING` | Upper bound for due date (RFC3339) |
| `--completed-min=STRING` | Lower bound for completion date |
| `--completed-max=STRING` | Upper bound for completion date |
| `--updated-min=STRING` | Lower bound for updated time |

### get (info, show) — Get a task
```bash
gog tasks get "@default" "task123" --account $GOG_ACCOUNT
```

### add (create) — Add a task
```bash
# Simple task
gog tasks add "@default" --title "Buy groceries" --account $GOG_ACCOUNT

# Task with due date and notes
gog tasks add "@default" --title "Submit report" --due "2026-07-01"   --notes "Q2 financial report" --account $GOG_ACCOUNT

# Subtask
gog tasks add "@default" --title "Subtask" --parent "parentTaskId" --account $GOG_ACCOUNT

# Recurring task
gog tasks add "@default" --title "Weekly standup" --recur weekly --repeat-count 12   --account $GOG_ACCOUNT
```
| Flag | Description |
|------|-------------|
| `--title=STRING` | Task title (required) |
| `--notes=STRING` | Task notes |
| `--due=STRING` | Due date (RFC3339 or YYYY-MM-DD) |
| `--parent=STRING` | Parent task ID (create as subtask) |
| `--previous=STRING` | Previous sibling (controls ordering) |
| `--repeat=STRING` | Repeat cadence: daily, weekly, monthly, yearly |
| `--recur=STRING` | Alias for --repeat |
| `--recur-rrule=STRING` | Repeat via RRULE (FREQ + optional INTERVAL) |
| `--repeat-count=INT` | Number of occurrences |
| `--repeat-until=STRING` | Repeat until date (RFC3339 or YYYY-MM-DD) |

### update (edit, set) — Update a task
```bash
gog tasks update "@default" "task123" --title "Updated title" --status completed   --account $GOG_ACCOUNT
```
| Flag | Description |
|------|-------------|
| `--title=STRING` | New title |
| `--notes=STRING` | New notes |
| `--due=STRING` | New due date |
| `--status=STRING` | needsAction or completed |

### done (complete) — Mark task completed
```bash
gog tasks done "@default" "task123" --account $GOG_ACCOUNT
```

### undo (uncomplete, undone) — Mark task needs action
```bash
gog tasks undo "@default" "task123" --account $GOG_ACCOUNT
```

### delete (rm, del, remove) — Delete a task
```bash
gog tasks delete "@default" "task123" --account $GOG_ACCOUNT
```

### clear — Clear completed tasks
```bash
gog tasks clear "@default" --account $GOG_ACCOUNT
```

### raw — Dump raw API response
```bash
gog tasks raw "@default" "task123" --account $GOG_ACCOUNT --pretty
```
