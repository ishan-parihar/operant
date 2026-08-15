# gog schema — Machine-Readable Command Contracts

**THE primary reference for AI agents.** `gog schema --json` outputs the complete CLI schema — every command, flag, type, default, and required field. No prose parsing needed.

## Usage

```bash
# Full CLI schema (large — use selectively)
gog schema --json

# Specific command schema
gog schema gmail send --json
gog schema calendar create --json
gog schema drive upload --json

# Specific command schema with hidden flags
gog schema gmail send --json --include-hidden
```

## Output Structure

```json
{
  "commands": {
    "gmail": {
      "commands": {
        "send": {
          "description": "Send an email",
          "flags": {
            "to": {
              "type": "string",
              "description": "Recipients (comma-separated)",
              "required": false
            },
            "subject": {
              "type": "string",
              "description": "Subject",
              "required": true
            },
            "body": {
              "type": "string",
              "description": "Body (plain text)",
              "required": false
            }
          }
        }
      }
    }
  }
}
```

## Agent Integration Pattern

```bash
# Before executing any command, get its schema
SCHEMA=$(gog schema gmail send --json)

# Parse with jq to get required flags
echo "$SCHEMA" | jq '.commands.gmail.commands.send.flags | to_entries[] | select(.value.required == true) | .key'
# Output: subject

# Get all flags with types
echo "$SCHEMA" | jq '.commands.gmail.commands.send.flags | to_entries[] | "\(.key): \(.value.type)"'
```

## Why Use Schema Over --help

| Aspect | `--help` | `schema --json` |
|--------|----------|-----------------|
| Parseable by agents | ❌ Prose, needs LLM | ✅ Structured JSON |
| Types documented | ❌ Implicit | ✅ Explicit (string, int, bool) |
| Required fields | ❌ Hard to extract | ✅ `"required": true` |
| Defaults | ❌ Sometimes shown | ✅ Always present |
| Machine-readable | ❌ No | ✅ Yes |

## Practical Use

```bash
# Get all commands that accept --json
gog schema --json | jq '[paths(.commands) | select(ends_with(["flags","json"])) | .[-2]] | unique'

# Get all leaf commands
gog schema --json | jq '[paths(.commands) | select(length == 2) | .[-1]]'

# Get flag types for a specific command
gog schema calendar create --json | jq '.commands.calendar.commands.create.flags | to_entries[] | "\(.key): \(.value.type) (default: \(.value.default // "none"))"'
```
