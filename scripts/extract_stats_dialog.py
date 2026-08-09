#!/usr/bin/env python3
"""Decompose stats_dialog.rs (1,084 lines) into sub-modules.

Section layout (1-indexed, verified):
  hub        1-181   imports, data types, StatsTab, StatsDialogState struct
  state    182-271   impl StatsDialogState + impl Default
  helpers  273-368   build_model_breakdown, compute_streaks, consecutive_dates,
                     date_to_days_since_epoch
  render   374-879   render_stats_dialog + tab renderers
  tests    880-1084  #[cfg(test)] mod tests
"""
import os

SRC = 'crates/operant-cli/src/tui/stats_dialog.rs'
DST = 'crates/operant-cli/src/tui/stats_dialog/'

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


head = get(1, 181, 'hub (stays)')
state = get(182, 271, 'state impl')
helpers = get(273, 368, 'helpers')
render = get(374, 879, 'render fns')
tests_raw = get(880, total, 'tests (raw)')

for label, blk in (('state', state), ('helpers', helpers), ('render', render)):
    o = ''.join(blk).count('{')
    c = ''.join(blk).count('}')
    if o != c:
        raise SystemExit(f'{label} UNBALANCED: {{ {o} vs }} {c}')

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
for f in ('state.rs', 'helpers.rs', 'render.rs', 'tests.rs'):
    p = DST + f
    if os.path.exists(p):
        os.remove(p)

STATE_HEADER = """// stats_dialog/state.rs — StatsDialogState methods + Default.
//
// Extracted from the stats_dialog.rs monolith.

use super::*;

"""

HELPERS_HEADER = """// stats_dialog/helpers.rs — Stats aggregation helpers.
//
// Extracted from the stats_dialog.rs monolith. Model breakdown building,
// streak computation, and date helpers.

use super::*;

"""

RENDER_HEADER = """// stats_dialog/render.rs — Stats dialog rendering (4 tabs).
//
// Extracted from the stats_dialog.rs monolith.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

"""

TESTS_HEADER = """// stats_dialog/tests.rs — Unit tests for stats aggregation and rendering.
//
// Extracted from the stats_dialog.rs monolith.

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
write('helpers.rs', HELPERS_HEADER, helpers)
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

print('\nDone. Rewrite stats_dialog.rs as mod.rs hub next.')
