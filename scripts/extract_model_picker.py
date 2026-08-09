#!/usr/bin/env python3
"""Decompose model_picker.rs (1,041 lines) into sub-modules.

Section layout (1-indexed, verified):
  hub      1-26    imports (stays)
  effort  27-110   EffortLevel enum + impl + model_supports_effort/max_effort
  models 111-263   ModelEntry + model_entry + provider model lists
  state  264-499   ModelPickerState struct + impl + Default
  render 500-750   render_model_picker
  tests  751-1041  #[cfg(test)] mod tests
"""
import os

SRC = 'crates/operant-cli/src/tui/model_picker.rs'
DST = 'crates/operant-cli/src/tui/model_picker/'

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


head = get(1, 26, 'hub (stays)')
effort = get(27, 110, 'effort level')
models = get(111, 263, 'model entries')
state = get(264, 499, 'state')
render = get(500, 750, 'render')
tests_raw = get(751, total, 'tests (raw)')

for label, blk in (('effort', effort), ('models', models), ('state', state), ('render', render)):
    o = ''.join(blk).count('{')
    c = ''.join(blk).count('}')
    if o != c:
        raise SystemExit(f'{label} UNBALANCED: {{ {o} vs }} {c}')

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
for f in ('effort.rs', 'models.rs', 'state.rs', 'render.rs', 'tests.rs'):
    p = DST + f
    if os.path.exists(p):
        os.remove(p)

EFFORT_HEADER = """// model_picker/effort.rs — Effort levels and capability checks.
//
// Extracted from the model_picker.rs monolith. EffortLevel enum, its label
// helpers, and model_supports_effort / model_supports_max_effort checks.

use super::*;

"""

MODELS_HEADER = """// model_picker/models.rs — Model entry definitions and provider model lists.
//
// Extracted from the model_picker.rs monolith. ModelEntry struct, registry
// lookups, default-model resolution, and provider-specific model lists.

use super::*;

"""

STATE_HEADER = """// model_picker/state.rs — ModelPickerState struct + methods + Default.
//
// Extracted from the model_picker.rs monolith.

use super::*;

"""

RENDER_HEADER = """// model_picker/render.rs — Model picker dialog rendering.
//
// Extracted from the model_picker.rs monolith.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

"""

TESTS_HEADER = """// model_picker/tests.rs — Unit tests for the model picker.
//
// Extracted from the model_picker.rs monolith.

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


write('effort.rs', EFFORT_HEADER, effort)
write('models.rs', MODELS_HEADER, models)
write('state.rs', STATE_HEADER, state)
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

print('\nDone. Rewrite model_picker.rs as mod.rs hub next.')
