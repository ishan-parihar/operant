#!/usr/bin/env python3
"""Decompose diff_viewer.rs (1,262 lines) into sub-modules.

Section layout (1-indexed, verified):
  header+types  1-122   imports, SYNTAX_SET/THEME_SET statics, DiffHunk,
                       DiffLine, DiffLineKind, FileDiffStats, DiffType,
                       DiffPane, DiffViewerState struct (stays in mod.rs so
                       sibling impl blocks can access render_cache field)
  state       123-245   impl DiffViewerState + impl Default
  parse       246-405   load_git_diff, parse_unified_diff, parse_hunk_header
  render      407-954   render_diff_dialog .. build_diff_lines
  tests       955-1262  #[cfg(test)] mod tests
"""
import os

SRC = 'crates/operant-cli/src/tui/diff_viewer.rs'
DST = 'crates/operant-cli/src/tui/diff_viewer/'

with open(SRC) as f:
    lines = f.readlines()

total = len(lines)
print(f'Source: {total} lines')


def get(lo, hi, label):
    if lo < 1 or hi > total or lo > hi:
        raise SystemExit(f'BAD RANGE {label}: {lo}-{hi} (total {total})')
    block = lines[lo - 1:hi]
    print(f'{label:22s} {lo:5d}-{hi:5d}  ({hi - lo + 1:4d} lines)')
    return block


head = get(1, 122, 'header+types (stay)')
state = get(123, 245, 'state impl')
parse = get(246, 405, 'parse fns')
render = get(407, 954, 'render fns')
tests_raw = get(955, total, 'tests (raw)')

for label, blk in (('state', state), ('parse', parse), ('render', render)):
    o = ''.join(blk).count('{')
    c = ''.join(blk).count('}')
    if o != c:
        raise SystemExit(f'{label} UNBALANCED: {{ {o} vs }} {c}')

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
for f in ('state.rs', 'parse.rs', 'render.rs', 'tests.rs'):
    p = DST + f
    if os.path.exists(p):
        os.remove(p)

STATE_HEADER = """// diff_viewer/state.rs — DiffViewerState methods + Default.
//
// Extracted from the diff_viewer.rs monolith.

use super::*;

"""

PARSE_HEADER = """// diff_viewer/parse.rs — Unified-diff parsing and git diff loading.
//
// Extracted from the diff_viewer.rs monolith. load_git_diff shells out to
// `git diff HEAD`, parse_unified_diff turns raw text into FileDiffStats.

use super::*;

"""

RENDER_HEADER = """// diff_viewer/render.rs — Diff dialog rendering (file list + detail panes).
//
// Extracted from the diff_viewer.rs monolith. render_diff_dialog, pane
// renderers, and the inline word-level diff / syntax-highlight helpers.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

"""

TESTS_HEADER = """// diff_viewer/tests.rs — Unit tests for diff parsing and state.
//
// Extracted from the diff_viewer.rs monolith.

use super::*;

"""


def write(name, header, block):
    content = header + ''.join(block)
    o = content.count('{')
    c = content.count('}')
    status = 'OK' if o == c else f'UNBALANCED (diff {o - c})'
    with open(DST + name, 'w') as f:
        f.write(content)
    print(f'  wrote {name}: {len(block)} lines, braces {status}')


write('state.rs', STATE_HEADER, state)
write('parse.rs', PARSE_HEADER, parse)
write('render.rs', RENDER_HEADER, render)

# tests.rs: unwrap
inner = tests_raw
while inner:
    s = inner[0].strip()
    if s.startswith('#[') or s == 'mod tests {' or s == '':
        inner = inner[1:]
        if s == 'mod tests {':
            break
    else:
        break
stripped = [ln for ln in inner if ln.strip() not in ('use super::*;',)]
inner = stripped
while inner and inner[-1].strip() in ('', '}'):
    if inner[-1].strip() == '}':
        inner = inner[:-1]
        break
    inner = inner[:-1]
write('tests.rs', TESTS_HEADER, inner)

print('\nDone. Rewrite diff_viewer.rs as mod.rs hub next.')
