#!/usr/bin/env python3
"""Decompose app/mod.rs (2,205 lines) — extract turn-state methods + tests.

Section layout (1-indexed, verified):
  header/struct  1-489   mod decls, imports, pub struct App (stays)
  turn methods 491-630   current_agent_mode_snapshot ... push_assistant_message
  run loop     633-1142  pub fn run (stays — core event loop)
  tests       1143-2205  #[cfg(test)] mod tests -> tests.rs
"""
import os

SRC = 'crates/operant-cli/src/tui/app/mod.rs'
DST = 'crates/operant-cli/src/tui/app/'

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


# The `impl App {` opener at 490 is shared with the run loop (which stays in
# mod.rs), so we extract ONLY the method bodies 491-625 and wrap them in a
# fresh `impl App { ... }` block. Balance-checked: methods are self-contained.
turn_raw = get(491, 625, 'turn methods (raw)')

opens = ''.join(turn_raw).count('{')
closes = ''.join(turn_raw).count('}')
if opens != closes:
    raise SystemExit(f'turn methods UNBALANCED: {{ {opens} vs }} {closes}')

body = turn_raw

tests_raw = get(1143, total, 'tests (raw)')

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
for f in ('turn_state.rs', 'tests.rs'):
    p = DST + f
    if os.path.exists(p):
        os.remove(p)

TURN_HEADER = """// app/turn_state.rs — Per-turn snapshot and metadata sync helpers.
//
// Extracted from the app/mod.rs monolith. Turn lifecycle helpers: agent-mode
// snapshots, user-turn begin/complete, transcript metadata sync, rewind flow
// entry, and onboarding persistence.

use super::*;

impl App {
"""

TESTS_HEADER = """// app/tests.rs — Unit tests for the TUI app (turn state, key handling,
// command routing).
//
// Extracted from the app/mod.rs monolith.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

"""

# turn_state.rs
content = TURN_HEADER + ''.join(body) + '\n}\n'
opens = content.count('{')
closes = content.count('}')
status = 'OK' if opens == closes else f'UNBALANCED (diff {opens - closes})'
with open(DST + 'turn_state.rs', 'w') as f:
    f.write(content)
print(f'  wrote turn_state.rs: {len(body)} body lines, braces {status}')

# tests.rs: strip cfg(test) + mod tests { wrapper and inner use super::*
inner = tests_raw
while inner:
    s = inner[0].strip()
    if s.startswith('#[') or s == 'mod tests {' or s == '':
        inner = inner[1:]
        if s == 'mod tests {':
            break
    else:
        break
# drop the inner use super::*; + crossterm import (re-added in header)
stripped = []
for ln in inner:
    s = ln.strip()
    if s == 'use super::*;' or s.startswith('use crossterm::event::'):
        continue
    stripped.append(ln)
inner = stripped
while inner and inner[-1].strip() in ('', '}'):
    if inner[-1].strip() == '}':
        inner = inner[:-1]
        break
    inner = inner[:-1]
content = TESTS_HEADER + ''.join(inner)
opens = content.count('{')
closes = content.count('}')
status = 'OK' if opens == closes else f'UNBALANCED (diff {opens - closes})'
with open(DST + 'tests.rs', 'w') as f:
    f.write(content)
print(f'  wrote tests.rs: {len(inner)} lines, braces {status}')

print('\nDone. Now rewrite mod.rs as the hub (drop extracted sections).')
