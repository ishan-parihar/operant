#!/usr/bin/env python3
"""Decompose keybindings.rs (1,292 lines) into sub-modules.

Section layout (1-indexed, verified):
  header/types  1-186   allow(dead_code), KeyAction, BindingContext, KeyBinding,
                       DefaultBinding, impl DefaultBinding, KeyBindingRegistry
                       struct (stays in mod.rs so sibling impls see private fields)
  registry    189-283   impl KeyBindingRegistry methods (new..remove)
  defaults    285-1152  fn add_defaults (huge data table)
  config      1153-1166 load_keybindings_from_config / save_keybindings_to_config
  tests       1167-1292 #[cfg(test)] mod tests
"""
import os
import re

SRC = 'crates/operant-cli/src/tui/keybindings.rs'
DST = 'crates/operant-cli/src/tui/keybindings/'

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


# --- registry methods: new .. remove (189-283, self-contained) ---
# NOTE: the original has ONE big impl KeyBindingRegistry { 187-1150 }.
# The registry methods 189-283 (incl. remove's closing brace) are extracted
# and wrapped in a fresh impl block; add_defaults (286-1149) goes elsewhere.
registry_raw = get(189, 283, 'registry methods (raw)')
opens = ''.join(registry_raw).count('{')
closes = ''.join(registry_raw).count('}')
if opens != closes:
    raise SystemExit(f'registry methods UNBALANCED: {{ {opens} vs }} {closes}')
registry_body = registry_raw

# --- defaults: fn add_defaults (286-1152) wrapped in impl block ---
defaults_raw = get(286, 1149, 'add_defaults (raw)')
opens = ''.join(defaults_raw).count('{')
closes = ''.join(defaults_raw).count('}')
if opens != closes:
    raise SystemExit(f'add_defaults UNBALANCED: {{ {opens} vs }} {closes}')

config = get(1153, 1166, 'config fns (stay in mod.rs)')
tests_raw = get(1167, total, 'tests (raw)')

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
for f in ('registry.rs', 'defaults.rs', 'tests.rs'):
    p = DST + f
    if os.path.exists(p):
        os.remove(p)

REGISTRY_HEADER = """// keybindings/registry.rs — Registry lookup/manipulation methods.
//
// Extracted from the keybindings.rs monolith. Construction, registration,
// lookup (with context fallback), matching, and removal.

use super::*;

impl KeyBindingRegistry {
"""

DEFAULTS_HEADER = """// keybindings/defaults.rs — Default keybinding table.
//
// Extracted from the keybindings.rs monolith. The add_defaults() data table
// defining every built-in keybinding per context.

use super::*;

impl KeyBindingRegistry {
"""

TESTS_HEADER = """// keybindings/tests.rs — Unit tests for the keybinding registry.
//
// Extracted from the keybindings.rs monolith.

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

"""

# registry.rs
content = REGISTRY_HEADER + ''.join(registry_body) + '\n}\n'
# add_defaults is private but called from with_defaults (registry.rs) -> pub(crate)
content = content.replace('    fn add_defaults', '    pub(crate) fn add_defaults', 1)
o = content.count('{'); c = content.count('}')
status = 'OK' if o == c else f'UNBALANCED (diff {o - c})'
with open(DST + 'registry.rs', 'w') as f:
    f.write(content)
print(f'  wrote registry.rs: {len(registry_body)} body lines, braces {status}')

# defaults.rs
content = DEFAULTS_HEADER + ''.join(defaults_raw) + '\n}\n'
o = content.count('{'); c = content.count('}')
status = 'OK' if o == c else f'UNBALANCED (diff {o - c})'
with open(DST + 'defaults.rs', 'w') as f:
    f.write(content)
print(f'  wrote defaults.rs: {len(defaults_raw)} body lines, braces {status}')

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
o = content.count('{'); c = content.count('}')
status = 'OK' if o == c else f'UNBALANCED (diff {o - c})'
with open(DST + 'tests.rs', 'w') as f:
    f.write(content)
print(f'  wrote tests.rs: {len(inner)} lines, braces {status}')

print('\nDone. Rewrite keybindings.rs as mod.rs hub next.')
