#!/usr/bin/env python3
"""Scan a Rust file for top-level items using string/comment-aware brace tracking.

Fixed: item starts are detected BEFORE processing the line's braces, so
single-line `pub struct X {` / `impl Y {` / `fn z() {` headers are captured.

Usage: python3 scan_top_level.py <file.rs>
"""
import sys


def scan_lines(lines):
    """Return list of (start_idx, end_idx, header) for top-level items (0-indexed, inclusive)."""
    items = []
    depth = 0
    i = 0
    n = len(lines)
    item_start = None
    header_lines = []
    pending_attrs = []  # #[derive(...)] etc. before an item
    in_block_comment = False
    in_raw_string = False
    raw_hashes = 0

    ITEM_START_PREFIXES = (
        'pub struct ', 'pub enum ', 'pub union ', 'struct ', 'enum ', 'union ',
        'impl ', 'pub fn ', 'pub(crate) fn ', 'pub(super) fn ', 'pub async fn ',
        'pub(crate) async fn ', 'async fn ', 'fn ', 'pub const ', 'const ',
        'pub static ', 'static ', 'mod ', 'pub mod ', 'type ', 'pub type ',
        'trait ', 'pub trait ', 'macro_rules! ',
    )

    def try_start_item(line, attrs):
        stripped = line.strip()
        if not stripped:
            return False
        if stripped.startswith('#[') or stripped.startswith('//') or stripped.startswith('///'):
            return False
        if stripped.startswith('//'):
            return False
        for p in ITEM_START_PREFIXES:
            if stripped.startswith(p):
                return True
        return False

    while i < n:
        line = lines[i]
        stripped = line.strip()

        # --- Attribute / doc lines: carry them as pending for the next item ---
        if item_start is None and not in_block_comment:
            if stripped.startswith('#['):
                pending_attrs.append(i)
                i += 1
                continue
            if stripped.startswith('///') or stripped.startswith('//!'):
                # doc comments - keep as context but not part of item header
                i += 1
                continue
            if stripped.startswith('// ===') or stripped.startswith('// ---'):
                i += 1
                continue
            if stripped.startswith('//'):
                i += 1
                continue
            if stripped == '':
                i += 1
                continue

        # --- Detect item start BEFORE brace processing ---
        if depth == 0 and item_start is None and not in_block_comment and not in_raw_string:
            if try_start_item(line, pending_attrs):
                item_start = i
                header_lines = list(pending_attrs) + [i]
                pending_attrs = []
                # If it's a one-liner ending with ';' and no '{' before ';', close now
                semi = stripped.find(';')
                brace = stripped.find('{')
                if semi >= 0 and (brace < 0 or brace > semi):
                    items.append((item_start, i, stripped[:90]))
                    item_start = None
                    header_lines = []
                    i += 1
                    continue

        # --- Tokenize the line ---
        j = 0
        jlen = len(line)
        while j < jlen:
            c = line[j]
            if in_block_comment:
                if line[j:j+2] == '*/':
                    in_block_comment = False
                    j += 2
                    continue
                j += 1
                continue
            if in_raw_string:
                close = '"' + '#' * raw_hashes
                k = line.find(close, j)
                if k >= 0:
                    in_raw_string = False
                    j = k + len(close)
                else:
                    j = jlen
                continue
            if c == '/' and j + 1 < jlen and line[j+1] == '/':
                j = jlen
                continue
            if c == '/' and j + 1 < jlen and line[j+1] == '*':
                in_block_comment = True
                j += 2
                continue
            if c == '"':
                h = 0
                k = j + 1
                while k < jlen and line[k] == '#':
                    h += 1
                    k += 1
                if k < jlen and line[k] == '"':
                    in_raw_string = True
                    raw_hashes = h
                    j = k + 1
                    continue
                k = j + 1
                while k < jlen:
                    if line[k] == '\\' and k + 1 < jlen:
                        k += 2
                        continue
                    if line[k] == '"':
                        break
                    k += 1
                j = k + 1
                continue
            if c == "'":
                k = j + 1
                if k < jlen and line[k] == '\\' and k + 1 < jlen:
                    k += 2
                    if k < jlen and line[k] == "'":
                        k += 1
                    j = k
                    continue
                if k < jlen and line[k] != "'":
                    k += 1
                    if k < jlen and line[k] == "'":
                        k += 1
                    j = k
                    continue
                j += 1
                continue
            if c == '{':
                depth += 1
                j += 1
                continue
            if c == '}':
                depth -= 1
                if depth == 0 and item_start is not None:
                    hdr = lines[header_lines[0]].strip()[:90] if header_lines else '?'
                    items.append((item_start, i, hdr))
                    item_start = None
                    header_lines = []
                j += 1
                continue
            j += 1

        # --- End of line: if an item started but never opened a brace (header spanning lines), keep waiting ---
        i += 1

    return items


def main():
    if len(sys.argv) < 2:
        print('usage: scan_top_level.py <file.rs>')
        return 1
    with open(sys.argv[1]) as f:
        lines = f.readlines()
    items = scan_lines(lines)
    total = len(lines)
    print(f'Total lines: {total}')
    print(f'Top-level items: {len(items)}')
    print('---')
    for s, e, header in items:
        span = e - s + 1
        print(f'{s+1:5d}-{e+1:5d} ({span:4d} lines) {header}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
