---
name: workspace-lint
description: Maintain a pristine, project-specific directory structure by enforcing rules declared in a root config file (e.g. `workspace-lint.yaml`). Use this skill proactively whenever an AI agent creates new files, scripts, docs, reports, logs, or artifacts in any project — every placement decision (where to put a `.py`, `.md`, `.mq5`, `.csv`, log file, or analysis report) should be checked against this skill's config before the file is written. Trigger on phrases like "organize files", "clean up directory", "where should I put this", "structure the workspace", "new script", "draft report", "save analysis to", "audit workspace", or any time you notice orphaned files at the project root, duplicate directories (e.g. `1. PHANTOM` and `PHANTOM`), stray build artifacts (`__pycache__`, `.pyc`, `.log`), or a project that looks "messy". Run the bundled validator (`scripts/workspace_lint.py`) after each iteration to catch drift early.
metadata:
  operant: {}
---

# Workspace Lint

Keep any project's directory structure pristine. The skill enforces rules declared in a root-level config file (`workspace-lint.yaml` by default). After every iteration, run the validator to detect drift before it compounds.

**Design philosophy:** `.gitignore` is the source of truth for exclusions. This linter focuses on structural rules that `.gitignore` cannot express — file placement, naming conventions, orphaned files, and git hygiene.

## Two components work together

1. **Config** (`workspace-lint.yaml`) — declared in the project root. Defines the canonical structure, allowed files per directory, and forbidden patterns. Project-specific.
2. **Validator** (`scripts/workspace_lint.py`) — bundled with this skill. Audits the project against the config. Reports violations. Optionally fixes them with `--fix`.

Both must exist for the skill to function. The config is per-project; the validator is shared.

## Setup: Author the Config

Place `workspace-lint.yaml` in the project root. The minimum viable config:

```yaml
project:
  name: "MyProject"
  type: "game-typescript"

structure:
  canonical:
    - path: "src"
      purpose: "All source code"
    - path: "tests"
      purpose: "All test files"
    - path: "docs"
      purpose: "Documentation and reports"

rules:
  root:
    forbidden_files:
      - "*.py"
      - "*.log"
    allowed_root_files:
      - "README.md"
      - "AGENTS.md"
      - ".gitignore"
      - "workspace-lint.yaml"

  files:
    "*.py":
      preferred_dir: "src"
    "*.md":
      preferred_dir: "docs"

  directories:
    forbidden_patterns:
      - "^\\s"     # Leading whitespace
      - "\\s$"     # Trailing whitespace
```

## Run the Validator

```bash
# Default: lint the current directory
python3 scripts/workspace_lint.py --root .

# Lint with a non-default config
python3 scripts/workspace_lint.py --config my-lint.yaml

# Auto-fix safe violations (git rm --cached for tracked-but-ignored files)
python3 scripts/workspace_lint.py --fix

# Show only summary
python3 scripts/workspace_lint.py --summary

# Output as JSON (for programmatic use)
python3 scripts/workspace_lint.py --json
```

Exit codes:
- `0` — no violations
- `1` — violations found
- `2` — config missing or invalid

## What the Validator Checks

| Check | Severity | Description |
|---|---|---|
| `root.forbidden_files` | error | File at root matches a forbidden pattern |
| `root.orphaned` | info | File at root not in allowed list — likely misplaced |
| `dir.whitespace` | error | Directory has leading/trailing whitespace |
| `dir.duplicate` | warn | Possible duplicate directory (normalized name collision) |
| `structure.missing_canonical` | error | A canonical directory from config doesn't exist |
| `structure.empty_canonical` | warn | Canonical directory is empty |
| `files.preferred_dir` | warn | File not in its preferred directory |
| `files.max_size` | warn | File exceeds configured max size |
| `git.tracked_but_ignored` | error | File is tracked but matches .gitignore — should be `git rm --cached` |

## Interpret Output

```
<rule>: <message> [<severity>]
```

| Severity | Meaning |
|---|---|
| `error` | Hard violation — must fix before commit |
| `warn` | Soft violation — should fix but not blocking |
| `info` | Hint — could improve but not required |

## Bundled Resources

| Resource | Purpose |
|---|---|
| `scripts/workspace_lint.py` | The validator. Audit + optional fix. |

# Workspace Lint

Keep any project's directory structure pristine. The skill enforces rules declared in a root-level config file (`workspace-lint.yaml` by default). After every iteration, run the validator to detect drift before it compounds.

**Design philosophy:** `.gitignore` is the source of truth for exclusions. This linter focuses on structural rules that `.gitignore` cannot express — file placement, naming conventions, orphaned files, and git hygiene.

## When to use

- **Before creating any file**: Check the config to know where it belongs.
- **After each iteration**: Run the validator to catch misplaced files.
- **On any cleanup request**: Audit the workspace against the canonical layout.
- **When a project feels messy**: The validator will tell you exactly what violates rules.

## Two components work together

1. **Config** (`workspace-lint.yaml`) — declared in the project root. Defines the canonical structure, allowed files per directory, and forbidden patterns. Project-specific.
2. **Validator** (`scripts/workspace_lint.py`) — bundled with this skill. Audits the project against the config. Reports violations. Optionally fixes them with `--fix`.

Both must exist for the skill to function. The config is per-project; the validator is shared.

---

## 1. Setup: Author the Config

Place `workspace-lint.yaml` in the project root. The minimum viable config:

```yaml
project:
  name: "MyProject"
  type: "game-typescript"

structure:
  canonical:
    - path: "src"
      purpose: "All source code"
    - path: "tests"
      purpose: "All test files"
    - path: "docs"
      purpose: "Documentation and reports"

rules:
  root:
    forbidden_files:
      - "*.py"
      - "*.log"
    allowed_root_files:
      - "README.md"
      - "AGENTS.md"
      - ".gitignore"
      - "workspace-lint.yaml"

  files:
    "*.py":
      preferred_dir: "src"
    "*.md":
      preferred_dir: "docs"

  directories:
    forbidden_patterns:
      - "^\\s"     # Leading whitespace
      - "\\s$"     # Trailing whitespace
```

## 2. Run the Validator

```bash
# Default: lint the current directory
python3 scripts/workspace_lint.py --root .

# Lint with a non-default config
python3 scripts/workspace_lint.py --config my-lint.yaml

# Auto-fix safe violations (git rm --cached for tracked-but-ignored files)
python3 scripts/workspace_lint.py --fix

# Show only summary
python3 scripts/workspace_lint.py --summary

# Output as JSON (for programmatic use)
python3 scripts/workspace_lint.py --json
```

Exit codes:
- `0` — no violations
- `1` — violations found
- `2` — config missing or invalid

## 3. What the Validator Checks

| Check | Severity | Description |
|---|---|---|
| `root.forbidden_files` | error | File at root matches a forbidden pattern |
| `root.orphaned` | info | File at root not in allowed list — likely misplaced |
| `dir.whitespace` | error | Directory has leading/trailing whitespace |
| `dir.duplicate` | warn | Possible duplicate directory (normalized name collision) |
| `structure.missing_canonical` | error | A canonical directory from config doesn't exist |
| `structure.empty_canonical` | warn | Canonical directory is empty |
| `files.preferred_dir` | warn | File not in its preferred directory |
| `files.max_size` | warn | File exceeds configured max size |
| `git.tracked_but_ignored` | error | File is tracked but matches .gitignore — should be `git rm --cached` |

## 4. Interpret Output

```
<rule>: <message> [<severity>]
```

| Severity | Meaning |
|---|---|
| `error` | Hard violation — must fix before commit |
| `warn` | Soft violation — should fix but not blocking |
| `info` | Hint — could improve but not required |

## 5. Apply to Your Workflow

When you are about to write a file:

1. **Read the config first.** Check `workspace-lint.yaml` for placement rules.
2. **Match the file to a rule.** Most projects have rules like `*.py → src/` or `*.md → docs/`.
3. **If no rule matches:** Place the file in the closest canonical subdirectory.
4. **After writing**, run the validator. Fix violations before declaring done.
5. **Commit the config alongside the project.** Layout changes → config update in same commit.

## 6. Common Pitfalls

- **Config drift.** When you reorganize directories, update `workspace-lint.yaml` in the same commit.
- **Tracked-but-ignored files.** Run with `--fix` to untrack them, or `git rm --cached` manually.
- **Empty directories.** Add a `.gitkeep` or remove the directory from config.
- **Cross-platform paths.** Use forward slashes everywhere.

## 7. Bundled Resources

| Resource | Purpose |
|---|---|
| `scripts/workspace_lint.py` | The validator. Audit + optional fix. |
