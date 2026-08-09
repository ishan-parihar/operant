#!/usr/bin/env python3
"""Fix extraction artifacts in model_picker sub-modules."""

# 1. effort.rs: restore derives above enum
fp = 'crates/operant-cli/src/tui/model_picker/effort.rs'
with open(fp) as f:
    c = f.read()
old = 'use super::*;\n\npub enum EffortLevel {'
new = (
    'use super::*;\n\n'
    '#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]\n'
    '#[serde(rename_all = "lowercase")]\n'
    '#[derive(Default)]\n'
    'pub enum EffortLevel {'
)
assert old in c, 'effort enum not found'
c = c.replace(old, new, 1)
with open(fp, 'w') as f:
    f.write(c)
print('effort.rs: restored derives')

# 2. models.rs: trim trailing doc comment for ModelPickerState
fp = 'crates/operant-cli/src/tui/model_picker/models.rs'
with open(fp) as f:
    c = f.read()
old = '/// State for the /model picker overlay.\n'
if old in c:
    c = c.replace(old, '', 1)
    print('models.rs: trimmed trailing doc comment')
with open(fp, 'w') as f:
    f.write(c)

# 3. render.rs: trim trailing Tests section header
fp = 'crates/operant-cli/src/tui/model_picker/render.rs'
with open(fp) as f:
    c = f.read()
old = '\n// ---------------------------------------------------------------------------\n// Tests\n// ---------------------------------------------------------------------------\n'
if old in c:
    c = c.replace(old, '\n', 1)
    print('render.rs: trimmed trailing Tests header')
with open(fp, 'w') as f:
    f.write(c)

print('Done.')
