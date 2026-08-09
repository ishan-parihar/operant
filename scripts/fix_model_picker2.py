#!/usr/bin/env python3
"""Trim trailing render_model_picker doc comments from state.rs."""

fp = 'crates/operant-cli/src/tui/model_picker/state.rs'
with open(fp) as f:
    content = f.read()

marker = 'impl Default for ModelPickerState {'
idx = content.find(marker)
assert idx >= 0, 'Default impl not found'

# Find the matching close for the Default impl: it's the LAST `}` in the file
# (Default impl is the final top-level item).
last_close = content.rfind('}')
assert last_close > idx, 'no closing brace after Default impl'
content = content[:last_close + 1].rstrip() + '\n'

with open(fp, 'w') as f:
    f.write(content)
print(f'state.rs trimmed to {len(content.splitlines())} lines')
