#!/usr/bin/env python3
"""
workspace_lint.py — Audit project directory structure.

Reads a workspace-lint.yaml config file and validates the current project
directory against declared canonical structure, file placement rules,
and git hygiene.

Philosophy: .gitignore is the source of truth for exclusions. This linter
focuses on structural rules that .gitignore cannot express (file placement,
naming conventions, orphaned files, git hygiene).

Usage:
    python3 workspace_lint.py [--root PATH] [--config FILE] [--fix] [--summary] [--json]
"""

import argparse
import fnmatch
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Optional

try:
    import yaml
except ImportError:
    print("ERROR: PyYAML required. Install: pip install PyYAML", file=sys.stderr)
    sys.exit(2)


CONFIG_NAMES = [
    "workspace-lint.yaml",
    "workspace-lint.yml",
    ".workspace-lint.yaml",
    ".workspace-lint.yml",
    "wlint.yaml",
    "wlint.yml",
]


def load_config(config_path: Optional[str], root: Path) -> dict:
    """Load the lint config, resolving the path."""
    if config_path:
        p = Path(config_path)
        if not p.exists():
            print(f"ERROR: config not found: {p}", file=sys.stderr)
            sys.exit(2)
    else:
        for name in CONFIG_NAMES:
            p = root / name
            if p.exists():
                break
        else:
            print(
                f"ERROR: no config found. Looked for: {', '.join(CONFIG_NAMES)}\n"
                f"Create one or pass --config.",
                file=sys.stderr,
            )
            sys.exit(2)

    with open(p) as f:
        cfg = yaml.safe_load(f)

    if not isinstance(cfg, dict):
        print(f"ERROR: config is not a YAML dict: {p}", file=sys.stderr)
        sys.exit(2)

    return cfg


class Violation:
    def __init__(self, path: str, rule: str, message: str, severity: str, fixable: bool = False):
        self.path = path
        self.rule = rule
        self.message = message
        self.severity = severity
        self.fixable = fixable

    def __str__(self):
        return f"{self.path}:{self.rule}: {self.message} [{self.severity}]"

    def to_dict(self):
        return {
            "path": self.path,
            "rule": self.rule,
            "message": self.message,
            "severity": self.severity,
            "fixable": self.fixable,
        }


def _relpath(p: Path, root: Path) -> str:
    """Relative path string from root, forward slashes."""
    return str(p.relative_to(root)).replace("\\", "/")


def _collect_files(root: Path) -> list[Path]:
    """Collect files git knows about (tracked + untracked non-ignored).
    Falls back to filesystem walk with .gitignore parsing if not a git repo."""
    try:
        result = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=10,
        )
        if result.returncode == 0 and result.stdout.strip():
            return [root / line for line in result.stdout.splitlines() if line.strip()]
    except (subprocess.TimeoutExpired, FileNotFoundError):
        pass

    # Fallback: walk and skip .gitignore'd dirs
    ignored = _gitignore_dirs(root)
    files = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [
            d for d in dirnames
            if not d.startswith(".") and d not in ignored
        ]
        for fname in filenames:
            files.append(Path(dirpath) / fname)
    return files


def _gitignore_dirs(root: Path) -> set:
    """Parse .gitignore for top-level directory ignore patterns."""
    gitignore = root / ".gitignore"
    if not gitignore.exists():
        return set()
    dirs = set()
    with open(gitignore) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if line.endswith("/") and "/" not in line[:-1]:
                dirs.add(line.rstrip("/"))
    return dirs


# ── Checks ──────────────────────────────────────────────────────────


def check_root_forbidden(files: list[Path], root: Path, config: dict) -> list[Violation]:
    """Files at root that match forbidden patterns (unless explicitly allowed)."""
    violations = []
    forbidden = config.get("rules", {}).get("root", {}).get("forbidden_files", [])
    allowed = set(config.get("rules", {}).get("root", {}).get("allowed_root_files", []))

    for f in files:
        if f.parent != root:
            continue
        rel = _relpath(f, root)
        if rel in allowed:
            continue
        for pattern in forbidden:
            if fnmatch.fnmatch(rel, pattern):
                violations.append(Violation(
                    path=rel,
                    rule="root.forbidden_files",
                    message=f"'{rel}' matches forbidden pattern '{pattern}' at root",
                    severity="error",
                ))
    return violations


def check_orphaned_root(files: list[Path], root: Path, config: dict) -> list[Violation]:
    """Files at root that aren't in the allowed list and don't match any forbidden pattern.
    These are neither explicitly allowed nor explicitly forbidden — likely misplaced."""
    violations = []
    forbidden = config.get("rules", {}).get("root", {}).get("forbidden_files", [])
    allowed = set(config.get("rules", {}).get("root", {}).get("allowed_root_files", []))

    for f in files:
        if f.parent != root:
            continue
        rel = _relpath(f, root)
        if rel in allowed:
            continue
        # Check if it matches any forbidden pattern (already caught by check_root_forbidden)
        matches_forbidden = any(fnmatch.fnmatch(rel, p) for p in forbidden)
        if not matches_forbidden:
            violations.append(Violation(
                path=rel,
                rule="root.orphaned",
                message=f"'{rel}' is at root but not in allowed_root_files — consider moving or adding to config",
                severity="info",
            ))
    return violations


def check_dir_naming(root: Path, config: dict) -> list[Violation]:
    """Directory naming: no leading/trailing whitespace, no duplicates within parent."""
    violations = []
    patterns = config.get("rules", {}).get("directories", {}).get("forbidden_patterns", [])

    seen_dirs: dict[str, dict[str, str]] = {}
    for dirpath, dirnames, _ in os.walk(root):
        # Skip hidden dirs
        dirnames[:] = [d for d in dirnames if not d.startswith(".")]
        for d in dirnames:
            full = Path(dirpath) / d
            rel = _relpath(full, root)

            for pattern in patterns:
                try:
                    if re.search(pattern, d):
                        violations.append(Violation(
                            path=rel,
                            rule="dir.whitespace",
                            message=f"Directory '{d}' matches forbidden pattern '{pattern}'",
                            severity="error",
                        ))
                except re.error:
                    pass

            # Duplicate detection within parent
            parent_key = str(full.parent)
            if parent_key not in seen_dirs:
                seen_dirs[parent_key] = {}
            norm_name = d.strip().lower().replace(" ", "")
            if norm_name in seen_dirs[parent_key]:
                dup_rel = seen_dirs[parent_key][norm_name]
                violations.append(Violation(
                    path=rel,
                    rule="dir.duplicate",
                    message=f"Possible duplicate of '{dup_rel}' (normalized: {norm_name})",
                    severity="warn",
                ))
            seen_dirs[parent_key][norm_name] = rel

    return violations


def check_empty_dirs(root: Path, config: dict) -> list[Violation]:
    """Canonical directories must exist and not be empty."""
    violations = []
    for entry in config.get("structure", {}).get("canonical", []):
        path = root / entry.get("path", "")
        if not path.exists():
            violations.append(Violation(
                path=entry["path"],
                rule="structure.missing_canonical",
                message=f"Canonical directory '{entry['path']}' does not exist",
                severity="error",
            ))
            continue
        if path.is_dir():
            contents = [c for c in path.iterdir() if c.name != ".gitkeep"]
            if not contents:
                violations.append(Violation(
                    path=entry["path"],
                    rule="structure.empty_canonical",
                    message=f"Canonical directory '{entry['path']}' is empty (add .gitkeep or remove from config)",
                    severity="warn",
                ))
    return violations


def check_file_placement(files: list[Path], root: Path, config: dict) -> list[Violation]:
    """Files should be in their preferred directory per config rules."""
    violations = []
    file_rules = config.get("rules", {}).get("files", {})
    allowed_root = set(config.get("rules", {}).get("root", {}).get("allowed_root_files", []))

    for f in files:
        rel = _relpath(f, root)

        # Skip files at root that are in the allowed list
        if f.parent == root and Path(rel).name in allowed_root:
            continue

        for rule_pattern, rule_config in file_rules.items():
            if not fnmatch.fnmatch(rel, rule_pattern):
                continue

            exclude_dirs = rule_config.get("exclude_dirs", [])
            if exclude_dirs:
                parts = Path(rel).parts
                if any(d in parts for d in exclude_dirs):
                    continue

            preferred = rule_config.get("preferred_dir")
            if preferred:
                preferred_path = preferred.rstrip("/")
                if not rel.startswith(preferred_path + "/") and rel != preferred_path:
                    violations.append(Violation(
                        path=rel,
                        rule="files.preferred_dir",
                        message=f"'{rel}' should be in '{preferred}/' (pattern: {rule_pattern})",
                        severity="warn",
                    ))

            max_kb = rule_config.get("max_size_kb")
            if max_kb:
                try:
                    size_kb = f.stat().st_size / 1024
                    if size_kb > max_kb:
                        violations.append(Violation(
                            path=rel,
                            rule="files.max_size",
                            message=f"'{rel}' is {size_kb:.0f}KB (max: {max_kb}KB)",
                            severity="warn",
                        ))
                except OSError:
                    pass
    return violations


def check_git_hygiene(root: Path) -> list[Violation]:
    """Detect tracked files that match .gitignore patterns.
    These should be `git rm --cached` to prevent accidental commits."""
    violations = []
    try:
        result = subprocess.run(
            ["git", "ls-files", "-i", "-c", "--exclude-standard"],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=10,
        )
        if result.returncode == 0 and result.stdout.strip():
            for line in result.stdout.splitlines():
                line = line.strip()
                if line:
                    violations.append(Violation(
                        path=line,
                        rule="git.tracked_but_ignored",
                        message=f"'{line}' is tracked but matches .gitignore — run: git rm --cached '{line}'",
                        severity="error",
                        fixable=True,
                    ))
    except (subprocess.TimeoutExpired, FileNotFoundError):
        pass
    return violations


# ── Fix + Output ────────────────────────────────────────────────────


def apply_fixes(root: Path, violations: list[Violation]) -> int:
    """Apply auto-fixable violations. Returns count of fixes applied."""
    import shutil

    fixed = 0
    for v in violations:
        if not v.fixable:
            continue

        if v.rule == "git.tracked_but_ignored":
            target = root / v.path
            if target.exists():
                # git rm --cached (keeps file on disk, removes from index)
                subprocess.run(
                    ["git", "rm", "--cached", v.path],
                    cwd=root,
                    capture_output=True,
                    timeout=10,
                )
                print(f"  [fixed] Untracked: {v.path} (file kept on disk)")
                fixed += 1

    return fixed


def print_summary(violations: list[Violation]):
    errors = [v for v in violations if v.severity == "error"]
    warns = [v for v in violations if v.severity == "warn"]
    infos = [v for v in violations if v.severity == "info"]
    fixable = [v for v in violations if v.fixable]

    print()
    print("─" * 60)
    print("  Workspace Lint Summary")
    print("─" * 60)
    print(f"  Errors:   {len(errors)}")
    print(f"  Warnings: {len(warns)}")
    print(f"  Info:     {len(infos)}")
    print(f"  Fixable:  {len(fixable)}")
    print("─" * 60)
    print()


def run_all_checks(root: Path, config: dict, files: list[Path]) -> list[Violation]:
    """Run all checks and return combined violations."""
    violations = []
    violations.extend(check_dir_naming(root, config))
    violations.extend(check_root_forbidden(files, root, config))
    violations.extend(check_orphaned_root(files, root, config))
    violations.extend(check_empty_dirs(root, config))
    violations.extend(check_file_placement(files, root, config))
    violations.extend(check_git_hygiene(root))
    return violations


def main():
    parser = argparse.ArgumentParser(
        description="Lint project directory structure against workspace-lint.yaml"
    )
    parser.add_argument("--root", default=".", help="Project root (default: cwd)")
    parser.add_argument("--config", default=None, help="Config file path")
    parser.add_argument("--fix", action="store_true", help="Auto-fix safe violations")
    parser.add_argument("--summary", action="store_true", help="Show only summary")
    parser.add_argument("--json", action="store_true", help="Output violations as JSON")
    args = parser.parse_args()

    root = Path(args.root).resolve()
    if not root.is_dir():
        print(f"ERROR: root is not a directory: {root}", file=sys.stderr)
        sys.exit(2)

    config = load_config(args.config, root)
    files = _collect_files(root)
    violations = run_all_checks(root, config, files)

    if args.fix and violations:
        fixes = apply_fixes(root, violations)
        if fixes:
            print(f"\nApplied {fixes} fixes.")
            files = _collect_files(root)
            violations = run_all_checks(root, config, files)

    if args.json:
        import json
        print(json.dumps([v.to_dict() for v in violations], indent=2))
    elif not args.summary:
        for v in violations:
            print(f"  {v}")

    print_summary(violations)

    if any(v.severity == "error" for v in violations):
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
