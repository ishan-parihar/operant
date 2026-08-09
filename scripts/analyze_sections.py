#!/usr/bin/env python3
"""Analyze section boundaries in overlays.rs to plan the extraction.

Prints every section separator comment with its line number, plus the
top-level item that follows, so we can verify boundaries before slicing.
"""
import sys
import re

fp = sys.argv[1] if len(sys.argv) > 1 else 'crates/operant-cli/src/tui/overlays.rs'
with open(fp) as f:
    lines = f.readlines()

print(f'Total lines: {len(lines)}')
print('--- Section separators + following item ---')

ITEM_RE = re.compile(
    r'^(pub\s+)?(struct|enum|fn|const|static|mod|impl)\s+[\w:]+'
)

for i, line in enumerate(lines):
    s = line.strip()
    # Section separators: // ===...=== or // ---...--- (length >= 12 dashes/equals)
    if s.startswith('//') and ('=' * 12 in s or '-' * 12 in s):
        # Look ahead for the next non-comment, non-blank line
        next_item = ''
        for j in range(i + 1, min(i + 8, len(lines))):
            t = lines[j].strip()
            if not t:
                continue
            if t.startswith('//') or t.startswith('///'):
                if t.startswith('//') and not t.startswith('///'):
                    next_item += t[:60] + ' | '
                continue
            next_item += t[:80]
            break
        print(f'{i+1:5d}  {s[:70]:70s}  →  {next_item[:100]}')
