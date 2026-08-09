#!/usr/bin/env python3
"""Decompose dialogs.rs (1,383 lines) into sub-modules.

Section layout (1-indexed lines, verified):
  permission.rs    18-668   PermissionDialogKind ... render_permission_dialog
  mcp_approval.rs 670-1010  MCP Server Approval Dialog ... truncate_str
  tests.rs       1017-1383  mod tests (unwrapped)
"""
import os

SRC = 'crates/operant-cli/src/tui/dialogs.rs'
DST = 'crates/operant-cli/src/tui/dialogs/'

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


def verify(block, label, first_ok=(), last_ok=()):
    first = block[0].strip()
    last = block[-1].strip()
    if first_ok and not any(first.startswith(p) for p in first_ok):
        raise SystemExit(f'BAD FIRST LINE in {label}: {first!r}')
    if last_ok and not any(last == p or last.startswith(p) for p in last_ok):
        raise SystemExit(f'BAD LAST LINE in {label}: {last!r}')


permission = get(18, 668, 'permission.rs')
verify(permission, 'permission', first_ok=('pub enum PermissionDialogKind'), last_ok=('}'))

mcp = get(670, 1010, 'mcp_approval.rs')
verify(mcp, 'mcp', first_ok=('// ---'), last_ok=('}'))

tests_raw = get(1017, total, 'tests.rs (raw)')
# tests section is wrapped in `mod tests { ... }` — verify and unwrap below
if tests_raw[0].strip() == 'mod tests {':
    tests_raw = tests_raw[1:]
verify(tests_raw, 'tests', first_ok=('use super::*'), last_ok=('}'))

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
for f in os.listdir(DST):
    os.remove(os.path.join(DST, f))

PERMISSION_HEADER = """// dialogs/permission.rs — Tool permission request dialogs.
//
// Extracted from dialogs.rs. Owns PermissionDialogKind, PermissionOption,
// PermissionRequest (with its constructors + key handling), and the
// render_permission_dialog renderer.

use super::*;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

"""

MCP_HEADER = """// dialogs/mcp_approval.rs — MCP server approval dialog.
//
// Extracted from dialogs.rs. Owns McpApprovalChoice, McpApprovalDialogState,
// the render_mcp_approval_dialog renderer, and handle_mcp_approval_key.

use super::*;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

"""

TESTS_HEADER = """// dialogs/tests.rs — Unit tests for the dialogs module.
//
// Extracted from dialogs.rs.

use super::*;

"""


def write(name, header, block):
    path = DST + name
    with open(path, 'w') as f:
        f.write(header)
        f.writelines(block)
    content = header + ''.join(block)
    opens = content.count('{')
    closes = content.count('}')
    status = 'OK' if opens == closes else f'UNBALANCED (diff {opens - closes})'
    print(f'  wrote {name}: {len(block)} lines, braces {status}')


write('permission.rs', PERMISSION_HEADER, permission)
write('mcp_approval.rs', MCP_HEADER, mcp)

# tests.rs: unwrap mod tests body (strip inner `use super::*;` and trailing `}`)
inner_start = None
for i, ln in enumerate(tests_raw):
    if ln.strip() == 'use super::*;':
        inner_start = i
        break
if inner_start is None:
    raise SystemExit('tests: could not find inner use super::*;')
inner = tests_raw[inner_start + 1:]
while inner and inner[-1].strip() in ('', '}'):
    if inner[-1].strip() == '}':
        inner = inner[:-1]
        break
    inner = inner[:-1]
write('tests.rs', TESTS_HEADER, inner)

print('\nDone. Now rewrite mod.rs + bump centered_rect/truncate_str to pub(crate).')
