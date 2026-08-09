#!/usr/bin/env python3
"""Decompose settings_screen.rs (1,057 lines) into sub-modules.

Section layout (1-indexed, verified):
  hub      1-76    imports, SettingKind, SettingsEntry, SettingsScreen struct
  state   77-504   impl SettingsScreen + impl Default + all_entries
  render 505-759   render_settings_screen + render_settings_list
  keys   760-972   handle_settings_key + update_scroll_offset_for_selection
                   + toggle_or_cycle_current
  tests  973-1057  #[cfg(test)] mod tests
"""
import os

SRC = 'crates/operant-cli/src/tui/settings_screen.rs'
DST = 'crates/operant-cli/src/tui/settings_screen/'

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


head = get(1, 76, 'hub (stays)')
state = get(77, 504, 'state impl')
render = get(505, 759, 'render fns')
keys = get(760, 972, 'key handling')
tests_raw = get(973, total, 'tests (raw)')

for label, blk in (('state', state), ('render', render), ('keys', keys)):
    o = ''.join(blk).count('{')
    c = ''.join(blk).count('}')
    if o != c:
        raise SystemExit(f'{label} UNBALANCED: {{ {o} vs }} {c}')

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
for f in ('state.rs', 'render.rs', 'keys.rs', 'tests.rs'):
    p = DST + f
    if os.path.exists(p):
        os.remove(p)

STATE_HEADER = """// settings_screen/state.rs — SettingsScreen methods, Default, and entry
// enumeration.
//
// Extracted from the settings_screen.rs monolith.

use super::*;

"""

RENDER_HEADER = """// settings_screen/render.rs — Settings screen rendering.
//
// Extracted from the settings_screen.rs monolith.

use super::*;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

"""

KEYS_HEADER = """// settings_screen/keys.rs — Settings screen key handling + edit helpers.
//
// Extracted from the settings_screen.rs monolith. handle_settings_key routes
// keys; scroll/toggle helpers update state.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

"""

TESTS_HEADER = """// settings_screen/tests.rs — Unit tests for the settings screen.
//
// Extracted from the settings_screen.rs monolith.

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
write('render.rs', RENDER_HEADER, render)
write('keys.rs', KEYS_HEADER, keys)

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

print('\nDone. Rewrite settings_screen.rs as mod.rs hub next.')
