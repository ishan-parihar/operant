# Workspace Lint — Validation Patterns

## Common Violation Patterns

### 1. Build Artifacts (`artifacts.detected`)

**What**: `.pyc`, `.pyo`, `__pycache__`, `node_modules`, `.venv`, `*.egg-info`

**Fix**: `workspace_lint --fix` removes these automatically.

**Manual fix**: Delete the directories and add them to `.gitignore`.

---

### 2. Root Forbidden Files (`root.forbidden`)

**What**: Files at project root that belong elsewhere (logs, temp files, OS artifacts).

**Fix**: `workspace_lint --fix` moves them to the appropriate `preferred_dir`.

**Manual fix**: Move files to the correct directory.

---

### 3. Directory Naming (`dir命名`)

**What**: Directory names with spaces, leading numbers, or inconsistent casing.

**Fix**: Rename directories to match the `canonical` structure in your config.

**Manual fix**: `git mv "1. PHANTOM" "PHANTOM"`

---

### 4. File Placement (`file.placement`)

**What**: Files in the wrong directory (e.g., Python files at root instead of `src/`).

**Fix**: `workspace_lint --fix` moves files to `preferred_dir`.

**Manual fix**: Move files to the correct directory.

---

### 5. Whitespace Duplicates (`dir.duplicate`)

**What**: Two directories that differ only by whitespace (e.g., "1. PHANTOM" vs "1.PHANTOM").

**Fix**: Delete the duplicate.

**Manual fix**: `git rm -r "1. PHANTOM"`

---

## Pre-Commit Checklist

Before running `git commit`:

1. Run `workspace_lint --root .`
2. Check the summary for errors
3. Fix any errors (use `--fix` for safe auto-fixes)
4. Re-run to verify clean state
5. Add changed files to staging
6. Commit

---

## CI Integration

Add to your CI pipeline:

```yaml
- name: Lint workspace
  run: python scripts/workspace_lint.py --root .
```

The script exits with code 1 if any errors are found, failing the CI build.
