#!/usr/bin/env python3
"""Move ModelEntry doc comment + derives from effort.rs to models.rs."""

# 1. effort.rs: remove dangling derive + section header at tail
fp = 'crates/operant-cli/src/tui/model_picker/effort.rs'
with open(fp) as f:
    c = f.read()
old = (
    '\n// ---------------------------------------------------------------------------\n'
    '// Types\n'
    '// ---------------------------------------------------------------------------\n\n'
    '#[derive(Debug, Clone)]\n'
)
assert old in c, 'effort.rs dangling derive not found'
c = c.replace(old, '\n', 1)
c = c.rstrip() + '\n'
with open(fp, 'w') as f:
    f.write(c)
print('effort.rs: removed dangling ModelEntry derive')

# 2. models.rs: add doc + derives before pub struct ModelEntry
fp = 'crates/operant-cli/src/tui/model_picker/models.rs'
with open(fp) as f:
    c = f.read()
old = 'use super::*;\n\npub struct ModelEntry {'
new = (
    'use super::*;\n\n'
    '/// A single model entry shown in the picker.\n'
    '#[derive(Debug, Clone)]\n'
    'pub struct ModelEntry {'
)
assert old in c, 'models.rs ModelEntry not found'
c = c.replace(old, new, 1)
with open(fp, 'w') as f:
    f.write(c)
print('models.rs: added ModelEntry doc + derives')

print('Done.')
