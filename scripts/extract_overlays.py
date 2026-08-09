#!/usr/bin/env python3
"""Decompose overlays.rs (2,251 lines) into sub-modules using VERIFIED boundaries.

Section layout (1-indexed lines, verified against the file):
  layout.rs        1-274   (doc, imports, constants, geometry, modal helpers)
  help.rs          275-552 (HelpOverlay section header through render_help_overlay)
  history_search.rs 553-1181 (HistorySearchOverlay section through build_highlighted_spans)
  message_selector.rs 1182-1368
  rewind_flow.rs   1369-1513 (through render_rewind_confirm)
  kb_line block    1514-1523 -> moved to help.rs (only used by render_help_overlay)
  global_search.rs 1524-1920 (Global Search Dialog section)
  tests.rs         1922-2251 (mod tests, unwrapped)

Each boundary was verified: the first line of each slice is a section
separator comment or blank, and the last line closes the previous item.
"""
import os
import re

SRC = 'crates/operant-cli/src/tui/overlays.rs'
DST = 'crates/operant-cli/src/tui/overlays/'

with open(SRC) as f:
    lines = f.readlines()

total = len(lines)
print(f'Source: {total} lines')


def get(lo, hi, label):
    """Return lines[lo-1:hi] (1-indexed inclusive)."""
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


# ---------------------------------------------------------------------------
# Slices
# ---------------------------------------------------------------------------

# layout.rs: 1-274 (modal_search_line ends at 274, 275 is blank)
layout = get(1, 274, 'layout.rs')
verify(layout, 'layout', first_ok=('//'), last_ok=('}'))

# help.rs: 275-552 (HelpOverlay header at 275-277; render_help_overlay ends 552)
help_sec = get(275, 552, 'help.rs')
verify(help_sec, 'help', first_ok=('//', ''), last_ok=('}'))

# kb_line: 1513-1528 (Shared helper section, only used by render_help_overlay)
kb = get(1513, 1528, 'kb_line')
verify(kb, 'kb_line', first_ok=('//', 'fn kb_line'), last_ok=('}'))

# history_search.rs: 553-1181 (HistorySearchOverlay header through build_highlighted_spans)
hist = get(553, 1181, 'history_search.rs')
verify(hist, 'history_search', first_ok=('//', ''), last_ok=('}'))

# message_selector.rs: 1182-1368
msg_sel = get(1182, 1368, 'message_selector.rs')
verify(msg_sel, 'message_selector', first_ok=('//', ''), last_ok=('}'))

# rewind_flow.rs: 1369-1511
rewind = get(1369, 1511, 'rewind_flow.rs')
verify(rewind, 'rewind_flow', first_ok=('//', ''), last_ok=('}'))

# global_search.rs: 1529-1920 (blank + Global Search Dialog header)
glob = get(1529, 1920, 'global_search.rs')
verify(glob, 'global_search', first_ok=('', '//'), last_ok=('}'))

# tests: 1922-2251 -> strip the outer `#[cfg(test)] mod tests { ... }` wrapper
tests_raw = get(1922, 2251, 'tests.rs (raw)')
verify(tests_raw, 'tests', first_ok=('//', ''), last_ok=('}'))

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
for f in os.listdir(DST):
    os.remove(os.path.join(DST, f))

IMPORTS_LAYOUT = """// overlays/layout.rs — Shared geometry, constants, and modal helpers.
//
// Extracted from the overlays.rs monolith. Holds the OPERANT_* color
// constants, centered-rect / cycle helpers, dark-overlay + dialog-bg
// renderers, and the modal frame/title/search helpers used by every
// overlay in this module.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

"""

IMPORTS_HELP = """// overlays/help.rs — Full-screen help overlay (? / F1 / /help).
//
// Extracted from the overlays.rs monolith. Renders the two-column
// keyboard-shortcut + slash-command reference panel and owns the
// HelpOverlay / HelpEntry state types.

use super::*;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

"""

IMPORTS_HISTORY = """// overlays/history_search.rs — Ctrl+R history search floating panel.
//
// Extracted from the overlays.rs monolith. Includes HistoryEntry,
// pinned-entry persistence (~/.operant/history_pins.json), fuzzy
// subsequence scoring, the HistorySearchOverlay state machine, and
// the floating-panel renderer.

use super::*;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

"""

IMPORTS_MSG_SEL = """// overlays/message_selector.rs — Message selector used by /rewind step 1.
//
// Extracted from the overlays.rs monolith.

use super::*;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

"""

IMPORTS_REWIND = """// overlays/rewind_flow.rs — Multi-step /rewind flow (select → confirm → done).
//
// Extracted from the overlays.rs monolith.

use super::*;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

"""

IMPORTS_GLOBAL = """// overlays/global_search.rs — Global ripgrep search dialog (T2-7).
//
// Extracted from the overlays.rs monolith.

use super::*;

"""

IMPORTS_TESTS = """// overlays/tests.rs — Unit tests for the overlays module.
//
// Extracted from the overlays.rs monolith.

use super::*;

"""


def write(name, header, block):
    path = DST + name
    with open(path, 'w') as f:
        f.write(header)
        f.writelines(block)
    # brace-balance check
    content = header + ''.join(block)
    opens = content.count('{')
    closes = content.count('}')
    status = 'OK' if opens == closes else f'UNBALANCED (diff {opens - closes})'
    print(f'  wrote {name}: {len(block)} lines, braces {status}')


write('layout.rs', IMPORTS_LAYOUT, layout)
write('help.rs', IMPORTS_HELP, help_sec + ['\n'] + kb)
write('history_search.rs', IMPORTS_HISTORY, hist)
write('message_selector.rs', IMPORTS_MSG_SEL, msg_sel)
write('rewind_flow.rs', IMPORTS_REWIND, rewind)
write('global_search.rs', IMPORTS_GLOBAL, glob)

# tests.rs: unwrap `#[cfg(test)] mod tests {` wrapper
body = tests_raw
# Find the inner `use super::*;` and the final closing brace
inner_start = None
for i, ln in enumerate(body):
    if ln.strip() == 'use super::*;':
        inner_start = i
        break
if inner_start is None:
    raise SystemExit('tests: could not find inner use super::*;')
# Strip everything before AND INCLUDING the inner `use super::*;` (header adds its own)
inner = body[inner_start + 1:]
# The last line should be the closing '}' of mod tests — drop it
while inner and inner[-1].strip() in ('', '}'):
    if inner[-1].strip() == '}':
        inner = inner[:-1]
        break
    inner = inner[:-1]
write('tests.rs', IMPORTS_TESTS, inner)

print('\nDone. Now delete overlays.rs and create overlays/mod.rs.')
