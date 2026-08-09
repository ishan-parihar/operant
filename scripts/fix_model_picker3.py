#!/usr/bin/env python3
"""Fix model_picker hub + effort.rs artifacts."""

# 1. Hub: remove orphaned derives (lines 24-26) that belong to effort.rs
fp = 'crates/operant-cli/src/tui/model_picker.rs'
with open(fp) as f:
    c = f.read()
old = (
    '#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]\n'
    '#[serde(rename_all = "lowercase")]\n'
    '#[derive(Default)]\n'
)
assert old in c, 'hub derives not found'
c = c.replace(old, '', 1)
with open(fp, 'w') as f:
    f.write(c)
print('hub: removed orphaned derives')

# 2. effort.rs: trim trailing doc comment for ModelEntry
fp = 'crates/operant-cli/src/tui/model_picker/effort.rs'
with open(fp) as f:
    c = f.read()
old = '/// A single model entry shown in the picker.\n'
assert old in c, 'effort.rs trailing doc not found'
c = c.replace(old, '', 1)
c = c.rstrip() + '\n'
with open(fp, 'w') as f:
    f.write(c)
print('effort.rs: trimmed trailing doc comment')

print('Done.')
