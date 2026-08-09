#!/usr/bin/env python3
"""Decompose prompt_input/mod.rs (2,975 lines) into focused sub-modules.

Uses brace-aware function extraction for the interleaved impl block.

Sections (1-indexed lines, verified):
  header         1-35     module decls, re-exports, constants
  free fns       37-96    handle_paste, detect_pasted_path
  InputMode      97-105
  struct        106-168   PromptInputState
  impl          169-1008  impl PromptInputState (840 lines, interleaved)
  impl Default  1009-1028
  render fns    1029-1088 input_height, wrap_line
  render_prompt 1089-1475 render_prompt_input
  tests         1476-2975 #[cfg(test)] mod tests

Target layout:
  state.rs       InputMode, PromptInputState, constants + state mgmt methods
  editing.rs     insert/delete/cursor/word methods
  history.rs     history + paste/yank methods
  vim_ops.rs     vim undo/marks/macros/search methods
  suggestions.rs suggestion methods
  visual.rs      visual cursor methods
  render.rs      input_height, wrap_line, render_prompt_input
  tests.rs       unwrapped tests
"""
import os

SRC = 'crates/operant-cli/src/tui/prompt_input/mod.rs'
DST = 'crates/operant-cli/src/tui/prompt_input/'

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


# ---------------------------------------------------------------------------
# Brace-aware method splitter
# ---------------------------------------------------------------------------

def split_methods(block, label, fn_re):
    """Split an impl block into (name, lines) tuples by matching method starts.

    fn_re: regex matching a method-signature start line.
    Returns list of (name, block_lines) preserving order.
    """
    methods = []
    depth = 0
    cur = None
    for ln in block:
        stripped = ln.strip()
        if cur is None:
            # Looking for a method start (top of the block / between methods).
            m = fn_re.match(stripped)
            if m and depth == 0:
                name = m.group(1)
                cur = [ln]
                depth = ln.count('{') - ln.count('}')
                # A signature may span multiple lines; keep scanning until '{'.
                while depth == 0 and '{' not in ln:
                    methods.append((name, cur))
                    cur = None
                    break
                else:
                    continue
            continue
        # Inside a method body.
        cur.append(ln)
        depth += ln.count('{') - ln.count('}')
        if depth == 0:
            methods.append((cur and cur[0] and _name_of(cur) or 'unknown', cur))
            cur = None
    if cur is not None:
        raise SystemExit(f'{label}: unterminated method at EOF')
    return methods


def _name_of(method_lines):
    """Best-effort name extraction from the method's signature lines."""
    sig = ' '.join(ln.strip() for ln in method_lines[:4])
    sig = sig.split('{')[0]
    for kw in ('pub fn ', 'fn '):
        idx = sig.find(kw)
        if idx >= 0:
            rest = sig[idx + len(kw):].strip()
            return rest.split('(')[0].split('<')[0].strip()
    return 'unknown'


def find_free_fn(block, names):
    """Extract named free functions from a block; return dict name -> lines."""
    out = {}
    remaining = []
    i = 0
    while i < len(block):
        ln = block[i]
        stripped = ln.strip()
        m = None
        for n in names:
            if stripped.startswith(f'pub fn {n}(') or stripped.startswith(f'fn {n}('):
                m = n
                break
        if m:
            body = [ln]
            depth = ln.count('{') - ln.count('}')
            while depth > 0:
                i += 1
                body.append(block[i])
                depth += block[i].count('{') - block[i].count('}')
            out[m] = body
            i += 1
        else:
            remaining.append(ln)
            i += 1
    if remaining and any(l.strip() for l in remaining):
        raise SystemExit(f'find_free_fn: leftover lines: {remaining[:3]!r}')
    return out


# ---------------------------------------------------------------------------
# Extract top-level chunks
# ---------------------------------------------------------------------------

header = get(1, 35, 'header (stays in mod.rs)')
free_fns = get(37, 96, 'free fns (stay in mod.rs)')
input_mode = get(97, 105, 'InputMode (stays in mod.rs)')
struct = get(106, 168, 'struct (stays in mod.rs)')
impl_block = get(169, 1008, 'impl block')
impl_default = get(1009, 1028, 'impl Default (stays in mod.rs)')
render_fns = get(1029, 1088, 'render fns')
render_main = get(1089, 1475, 'render_prompt_input')
tests_raw = get(1476, total, 'tests (raw)')

# --- Split the impl block by method name ---
import re
METHOD_RE = re.compile(r'(?:pub\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)')

# The impl block starts with `impl PromptInputState {` and ends with `}`.
# Methods (incl. multi-line signatures), doc comments, and attributes (incl.
# multi-line #[expect(...)]) are captured with their attached doc/attrs.
methods = []
cur = None
cur_name = None
pending = []
in_attr = False
sig_open = False
body_depth = 0
for ln in impl_block:
    stripped = ln.strip()
    if cur is None:
        # --- between methods (top level of impl block) ---
        if stripped.startswith('impl '):
            continue
        if in_attr:
            pending.append(ln)
            if stripped.endswith(']'):
                in_attr = False
            continue
        if stripped.startswith('#['):
            pending.append(ln)
            if not stripped.endswith(']'):
                in_attr = True
            continue
        m = METHOD_RE.match(stripped)
        if m:
            cur_name = m.group(1)
            cur = pending + [ln]
            pending = []
            if '{' in ln:
                sig_open = True
                body_depth = ln.count('{') - ln.count('}')
            else:
                sig_open = False
                body_depth = 0
            continue
        if stripped.startswith('//') or stripped == '':
            pending.append(ln)
            continue
        if stripped == '}':
            continue  # impl block closer
        raise SystemExit(f'impl: unexpected top-level line: {stripped!r}')
    # --- inside a method ---
    cur.append(ln)
    if not sig_open:
        if '{' in ln:
            sig_open = True
            body_depth = ln.count('{') - ln.count('}')
        continue
    body_depth += ln.count('{') - ln.count('}')
    if body_depth == 0:
        methods.append((cur_name, cur))
        cur = None
        cur_name = None
        sig_open = False
if cur is not None:
    raise SystemExit(f'impl: unterminated method {cur_name}')

print(f'impl: {len(methods)} methods extracted')
for name, m in methods:
    print(f'  {name:28s} {len(m):4d} lines')

# --- Group methods ---
STATE_METHODS = {
    'new', 'add_image', 'clear_images', 'clear', 'take', 'normalize',
    'update_token_estimate', 'is_empty',
}
EDIT_METHODS = {
    'insert_char', 'insert_newline', 'backspace', 'delete', 'move_left',
    'move_right', 'kill_line_backward', 'kill_word_backward',
    'delete_word_backward', 'delete_word_forward', 'move_word_backward',
    'move_word_forward', 'delete_word_at_cursor',
}
HISTORY_METHODS = {'history_up', 'history_down', 'paste', 'yank', 'yank_pop',
                   'yank_to_register', 'paste_from_register'}
VIM_METHODS = {
    'push_undo', 'set_mark', 'jump_to_mark', 'start_macro_recording',
    'stop_macro_recording', 'replay_macro', 'execute_vim_cmdline',
    'vim_search_forward', 'vim_search_backward',
}
SUGGEST_METHODS = {
    'has_active_file_ref', 'update_suggestions', 'suggestion_next',
    'suggestion_prev', 'accept_suggestion_for_submit', 'accept_suggestion',
    'replace_text',
}
VISUAL_METHODS = {
    'cursor_visual_pos', 'move_visual_up', 'move_visual_down',
    'visual_row_count', 'set_cursor_at_visual',
}

groups = {
    'state.rs': (STATE_METHODS, []),
    'editing.rs': (EDIT_METHODS, []),
    'history.rs': (HISTORY_METHODS, []),
    'vim_ops.rs': (VIM_METHODS, []),
    'suggestions.rs': (SUGGEST_METHODS, []),
    'visual.rs': (VISUAL_METHODS, []),
}

for name, m in methods:
    placed = False
    for fname, (allowed, _) in groups.items():
        if name in allowed:
            groups[fname][1].append((name, m))
            placed = True
            break
    if not placed:
        raise SystemExit(f'impl: ungrouped method {name}')

missing = set()
for fname, (allowed, got) in groups.items():
    got_names = {n for n, _ in got}
    miss = allowed - got_names
    if miss:
        missing |= miss
if missing:
    raise SystemExit(f'impl: missing methods: {sorted(missing)}')

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
# don't wipe mod.rs / existing submodules — only remove files we regenerate
for f in ('state.rs', 'editing.rs', 'history.rs', 'vim_ops.rs',
          'suggestions.rs', 'visual.rs', 'render.rs', 'tests.rs'):
    p = DST + f
    if os.path.exists(p):
        os.remove(p)

# sanity: mod.rs keeps lines 1-168 + 1009-1028 (constants, InputMode, struct,
# free fns, impl Default). Its new size: ~200 lines.
kept = len(header) + len(free_fns) + len(input_mode) + len(struct) + len(impl_default)
print(f'mod.rs will keep ~{kept} lines (constants + types + free fns + impl Default)')

HEADER_TMPL = """// prompt_input/{fname} — {desc}
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;

"""

IMPL_HEADER = """impl PromptInputState {{
{body}}}
"""

RENDER_HEADER = """// prompt_input/render.rs — Input rendering (input_height, wrap_line,
// render_prompt_input).
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

"""

TESTS_HEADER = """// prompt_input/tests.rs — Unit tests for prompt input state + rendering.
//
// Extracted from the prompt_input/mod.rs monolith.

use super::*;
use super::typeahead::{compute_file_suggestions, compute_slash_suggestions};
use super::vim::{{
    VimOperator, motion_B, motion_E, motion_G, motion_W, motion_b, motion_e, motion_find_char,
    motion_first_nonblank, motion_gg, motion_w,
}};

"""


def join_methods(items):
    out = []
    for name, m in items:
        out.extend(m)
        out.append('\n')
    return out


def write_impl(fname, desc, items):
    body = ''.join(join_methods(items))
    content = HEADER_TMPL.format(fname=fname, desc=desc) + IMPL_HEADER.format(body=body)
    opens = content.count('{')
    closes = content.count('}')
    status = 'OK' if opens == closes else f'UNBALANCED (diff {opens - closes})'
    with open(DST + fname, 'w') as f:
        f.write(content)
    print(f'  wrote {fname}: {len(body.splitlines())} body lines, braces {status}')


descs = {
    'state.rs': 'Core state-management methods (new, clear, take, normalize).',
    'editing.rs': 'Character/word editing and cursor movement methods.',
    'history.rs': 'History navigation, paste, and kill-ring/yank methods.',
    'vim_ops.rs': 'Vim undo, marks, macros, and search methods.',
    'suggestions.rs': 'Suggestion computation and navigation methods.',
    'visual.rs': 'Visual cursor positioning and multi-line layout methods.',
}
for fname, (_, items) in groups.items():
    write_impl(fname, descs[fname], items)

# --- render.rs: input_height + wrap_line + render_prompt_input ---
render_fns_block = ''.join(render_fns).strip()
# render_fns contains two free fns; keep them as-is (they are self-contained)
render_body = render_fns_block + '\n\n' + ''.join(render_main)
content = RENDER_HEADER + render_body + '\n'
opens = content.count('{')
closes = content.count('}')
status = 'OK' if opens == closes else f'UNBALANCED (diff {opens - closes})'
with open(DST + 'render.rs', 'w') as f:
    f.write(content)
print(f'  wrote render.rs: {len(render_body.splitlines())} body lines, braces {status}')

# --- tests.rs: unwrap mod tests ---
inner = tests_raw
# strip leading #[cfg(test)] / #[allow(...)] / mod tests { lines
while inner:
    s = inner[0].strip()
    if s.startswith('#[') or s == 'mod tests {' or s == '':
        inner = inner[1:]
        if s == 'mod tests {':
            break
    else:
        break
# drop the inner `use super::*;`, `use super::typeahead::{...};` and
# `use super::vim::{...};` import blocks (re-added in the header). The vim
# import spans multiple lines and ends with `};`.
stripped_inner = []
skip_until_semi = False
for ln in inner:
    s = ln.strip()
    if skip_until_semi:
        if s.endswith('};'):
            skip_until_semi = False
        continue
    if s == 'use super::*;':
        continue
    if s.startswith('use super::typeahead::{'):
        if not s.endswith('};'):
            skip_until_semi = True
        continue
    if s.startswith('use super::vim::{'):
        if not s.endswith('};'):
            skip_until_semi = True
        continue
    stripped_inner.append(ln)
inner = stripped_inner
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

print('\nDone. Now rewrite mod.rs as the hub + compile.')
