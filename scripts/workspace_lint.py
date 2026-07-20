#!/usr/bin/env python3
"""
workspace_lint.py — Validate project directory structure against workspace-lint.yaml
"""
import os
import sys
import yaml
import fnmatch
from pathlib import Path
from typing import List, Dict, Any, Optional, Set
from dataclasses import dataclass
from enum import Enum

class Severity(Enum):
    ERROR = "error"
    WARNING = "warning"
    INFO = "info"

@dataclass
class Violation:
    severity: Severity
    rule: str
    path: str
    message: str
    fix_action: Optional[str] = None

class WorkspaceLinter:
    def __init__(self, config_path: str, project_root: str):
        self.project_root = Path(project_root).resolve()
        with open(config_path, 'r') as f:
            self.config = yaml.safe_load(f)
        self.violations: List[Violation] = []

    def lint(self) -> List[Violation]:
        self._check_root_files()
        self._check_root_dirs()
        self._check_crate_structure()
        self._check_placement()
        self._check_naming()
        self._check_forbidden()
        self._check_gitignore()
        return self.violations

    def _rel_path(self, path: Path) -> str:
        try:
            return str(path.relative_to(self.project_root))
        except ValueError:
            return str(path)

    def _should_skip(self, path: Path) -> bool:
        """Check if path should be skipped (target, .git, etc.)"""
        rel = self._rel_path(path)
        skip_patterns = [
            "target/**",
            ".git/**",
            "node_modules/**",
            ".superpowers/**",
            "skills/**",
            ".memsearch/**",
            ".contexty/**",
            ".sisyphus/**",
            ".hive/**",
            ".cortexkit/**",
            "graphify-out/**",
            "local/**",
            "__pycache__/**",
            "*.pyc",
        ]
        for pat in skip_patterns:
            if self._match_path(rel, pat):
                return True
        return False

    def _check_root_files(self):
        """Check files at project root against allow list"""
        root_cfg = self.config.get('root', {})
        allowed_patterns = root_cfg.get('allow', [])

        for item in self.project_root.iterdir():
            if item.name.startswith('.'):
                continue
            if self._should_skip(item):
                continue
            if item.is_file():
                rel = self._rel_path(item)
                if not self._matches_any(rel, allowed_patterns):
                    self.violations.append(Violation(
                        severity=Severity.ERROR,
                        rule="root_file_not_allowed",
                        path=rel,
                        message=f"File '{rel}' not in root allow list",
                        fix_action="Move to appropriate directory or add to workspace-lint.yaml"
                    ))

    def _check_root_dirs(self):
        """Check required/allowed directories at root"""
        root_cfg = self.config.get('root', {})
        required = root_cfg.get('required_dirs', [])
        allowed = root_cfg.get('allow_dirs', [])

        for req in required:
            if not (self.project_root / req).exists():
                self.violations.append(Violation(
                    severity=Severity.ERROR,
                    rule="required_dir_missing",
                    path=req,
                    message=f"Required directory '{req}' missing at root"
                ))

        for item in self.project_root.iterdir():
            if item.is_dir() and not item.name.startswith('.'):
                if self._should_skip(item):
                    continue
                if item.name not in allowed:
                    self.violations.append(Violation(
                        severity=Severity.WARNING,
                        rule="unexpected_root_dir",
                        path=item.name,
                        message=f"Directory '{item.name}' at root not in allow list"
                    ))

    def _check_crate_structure(self):
        """Validate crate structure"""
        crates_cfg = self.config.get('crates', {})
        required_members = crates_cfg.get('required_members', [])
        crate_types = crates_cfg.get('crate_types', {})

        crates_dir = self.project_root / 'crates'
        if not crates_dir.exists():
            return

        for member in required_members:
            crate_path = crates_dir / member
            if not crate_path.exists():
                self.violations.append(Violation(
                    severity=Severity.ERROR,
                    rule="required_crate_missing",
                    path=f"crates/{member}",
                    message=f"Required workspace member crate '{member}' not found"
                ))
                continue

            crate_type = crate_types.get(member, "lib")
            
            if crate_type == "lib":
                if not (crate_path / "src" / "lib.rs").exists():
                    self.violations.append(Violation(
                        severity=Severity.ERROR,
                        rule="required_crate_file_missing",
                        path=f"crates/{member}/src/lib.rs",
                        message=f"Required file 'src/lib.rs' missing in crate '{member}'"
                    ))
            elif crate_type == "bin":
                if not (crate_path / "src" / "main.rs").exists():
                    self.violations.append(Violation(
                        severity=Severity.ERROR,
                        rule="required_crate_file_missing",
                        path=f"crates/{member}/src/main.rs",
                        message=f"Required file 'src/main.rs' missing in crate '{member}'"
                    ))

            # Check Cargo.toml
            if not (crate_path / "Cargo.toml").exists():
                self.violations.append(Violation(
                    severity=Severity.ERROR,
                    rule="required_crate_file_missing",
                    path=f"crates/{member}/Cargo.toml",
                    message=f"Required file 'Cargo.toml' missing in crate '{member}'"
                ))

    def _check_placement(self):
        """Check file placement rules"""
        placement_rules = self.config.get('placement', [])
        
        # Get exclude_dirs from first rule that has it
        exclude_dirs = []
        for rule in placement_rules:
            if isinstance(rule, dict) and 'exclude_dirs' in rule:
                exclude_dirs = rule.get('exclude_dirs', [])
                break

        for rule in placement_rules:
            if not isinstance(rule, dict):
                continue
            pattern = rule.get('pattern', '')
            allowed_dirs = rule.get('allowed_dirs', [])
            exceptions = rule.get('exceptions', [])
            action = rule.get('action', '')

            if not pattern:
                continue

            # Find all files matching pattern
            for file_path in self.project_root.rglob('*'):
                if not file_path.is_file():
                    continue
                if file_path.name.startswith('.') and file_path.name != '.gitignore':
                    continue
                if self._should_skip(file_path):
                    continue

                rel = self._rel_path(file_path)

                # Check if in excluded directory
                in_excluded = False
                for exc in exclude_dirs:
                    if self._match_path(rel, exc):
                        in_excluded = True
                        break
                if in_excluded:
                    continue

                # Check if matches pattern
                if not self._match_pattern(file_path.name, pattern):
                    continue

                # Check exceptions
                if rel in exceptions:
                    continue

                # Check if in allowed directory
                in_allowed = False
                for allowed in allowed_dirs:
                    if self._path_in_dir(rel, allowed):
                        in_allowed = True
                        break

                if not in_allowed:
                    self.violations.append(Violation(
                        severity=Severity.ERROR,
                        rule="wrong_placement",
                        path=rel,
                        message=f"File '{rel}' matches pattern '{pattern}' but is not in allowed directories: {allowed_dirs}",
                        fix_action=action
                    ))

    def _check_naming(self):
        """Check naming conventions"""
        naming_cfg = self.config.get('naming', {})

        # Check crate directories: kebab-case
        crates_dir = self.project_root / 'crates'
        if crates_dir.exists():
            for crate in crates_dir.iterdir():
                if crate.is_dir() and not crate.name.startswith('.'):
                    if not self._is_kebab_case(crate.name):
                        self.violations.append(Violation(
                            severity=Severity.WARNING,
                            rule="naming_convention",
                            path=f"crates/{crate.name}",
                            message=f"Crate directory '{crate.name}' should be kebab-case"
                        ))

    def _check_forbidden(self):
        """Check for forbidden patterns"""
        forbidden = self.config.get('forbidden', [])
        for pattern in forbidden:
            for match in self.project_root.rglob('*'):
                if match.name.startswith('.'):
                    continue
                if self._should_skip(match):
                    continue
                # Skip node_modules entirely
                rel = self._rel_path(match)
                if 'node_modules/' in rel or rel.startswith('node_modules/'):
                    continue
                if self._match_path(rel, pattern):
                    self.violations.append(Violation(
                        severity=Severity.ERROR,
                        rule="forbidden_pattern",
                        path=rel,
                        message=f"Forbidden pattern '{pattern}' matched: {rel}"
                    ))

    def _check_gitignore(self):
        """Validate .gitignore has required patterns"""
        gitignore_path = self.project_root / '.gitignore'
        if not gitignore_path.exists():
            self.violations.append(Violation(
                severity=Severity.WARNING,
                rule="gitignore_missing",
                path=".gitignore",
                message=".gitignore not found at project root"
            ))
            return

        content = gitignore_path.read_text()
        required = self.config.get('gitignore', {}).get('required_patterns', [])

        for pattern in required:
            if pattern not in content:
                self.violations.append(Violation(
                    severity=Severity.ERROR,
                    rule="gitignore_missing_pattern",
                    path=".gitignore",
                    message=f"Required pattern '{pattern}' missing from .gitignore"
                ))

    def _matches_any(self, path: str, patterns: List[str]) -> bool:
        for pat in patterns:
            if self._match_path(path, pat):
                return True
        return False

    def _match_path(self, path: str, pattern: str) -> bool:
        """Match path against glob pattern with ** support"""
        if '**' in pattern:
            # Convert to regex-like matching
            parts = pattern.split('**')
            if len(parts) == 2:
                prefix, suffix = parts
                return path.startswith(prefix) and path.endswith(suffix)
        return fnmatch.fnmatch(path, pattern)

    def _match_pattern(self, name: str, pattern: str) -> bool:
        """Match filename against pattern"""
        return fnmatch.fnmatch(name, pattern)

    def _path_in_dir(self, path: str, dir_pattern: str) -> bool:
        """Check if path is within a directory pattern"""
        if dir_pattern == '.':
            return True
        if dir_pattern.endswith('/'):
            dir_pattern = dir_pattern[:-1]
        return path.startswith(dir_pattern + '/') or path == dir_pattern

    def _is_kebab_case(self, s: str) -> bool:
        return s.islower() and '_' not in s and s == '-'.join(filter(None, s.split('-')))


def main():
    if len(sys.argv) < 2:
        print("Usage: python workspace_lint.py <project_root> [--config <config_path>]")
        sys.exit(1)

    project_root = sys.argv[1]
    config_path = sys.argv[3] if len(sys.argv) > 3 and sys.argv[2] == '--config' else \
                  os.path.join(project_root, 'workspace-lint.yaml')

    if not os.path.exists(config_path):
        print(f"Config not found: {config_path}")
        sys.exit(2)

    linter = WorkspaceLinter(config_path, project_root)
    violations = linter.lint()

    # Group by severity
    errors = [v for v in violations if v.severity == Severity.ERROR]
    warnings = [v for v in violations if v.severity == Severity.WARNING]
    infos = [v for v in violations if v.severity == Severity.INFO]

    # Print results
    if errors:
        print(f"\n❌ ERRORS ({len(errors)}):")
        for v in errors:
            print(f"  [{v.rule}] {v.path}: {v.message}")
            if v.fix_action:
                print(f"    → Fix: {v.fix_action}")

    if warnings:
        print(f"\n⚠️  WARNINGS ({len(warnings)}):")
        for v in warnings:
            print(f"  [{v.rule}] {v.path}: {v.message}")

    if infos:
        print(f"\nℹ️  INFO ({len(infos)}):")
        for v in infos:
            print(f"  [{v.rule}] {v.path}: {v.message}")

    print(f"\n{'='*50}")
    print(f"Total: {len(errors)} errors, {len(warnings)} warnings, {len(infos)} info")

    if errors:
        sys.exit(1)
    else:
        sys.exit(0)


if __name__ == '__main__':
    main()