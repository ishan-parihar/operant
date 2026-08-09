#!/usr/bin/env python3
"""Decompose messages/mod.rs (1,729 lines) into sub-modules.

Section layout (1-indexed lines, verified):
  mod.rs (kept)      1-66   doc, imports, mod decls, RenderContext, constants
  helpers.rs        68-167  shared rendering primitives
                       +1101-1117  truncate_user_prompt_text (used by render_user_text_with_ctx)
  transcript.rs    170-826  transcript renderers
                       +1119-1144  tool_result_text (used by transcript renderers at 505/679)
  tools.rs         828-1099 tool use/result/bash renderers
  commands.rs     1148-1403 system/command/goal renderers
  tests.rs        1410-1729 mod tests (unwrapped)
"""
import os
import sys

SRC = 'crates/operant-cli/src/tui/messages/mod.rs'
DST = 'crates/operant-cli/src/tui/messages/'

with open(SRC) as f:
    lines = f.readlines()

total = len(lines)
print(f'Source: {total} lines')


def get(lo, hi, label):
    if lo < 1 or hi > total or lo > hi:
        raise SystemExit(f'BAD RANGE {label}: {lo}-{hi} (total {total})')
    block = lines[lo - 1:hi]
    print(f'{label:24s} {lo:5d}-{hi:5d}  ({hi - lo + 1:4d} lines)')
    return block


def verify(block, label, first_ok=(), last_ok=()):
    first = block[0].strip()
    last = block[-1].strip()
    if first_ok and not any(first.startswith(p) for p in first_ok):
        raise SystemExit(f'BAD FIRST LINE in {label}: {first!r}')
    if last_ok and not any(last == p or last.startswith(p) for p in last_ok):
        raise SystemExit(f'BAD LAST LINE in {label}: {last!r}')


# helpers: 68-168 (render_user_text_with_ctx ... user_metadata_line)
helpers = get(68, 168, 'helpers.rs')
verify(helpers, 'helpers', first_ok=('fn render_user_text_with_ctx'), last_ok=('}'))

# truncate_user_prompt_text: 1101-1117 (append to helpers)
trunc = get(1101, 1117, 'truncate_user_prompt_text')
verify(trunc, 'trunc', first_ok=('fn truncate_user_prompt_text'), last_ok=('}'))

# transcript: 170-824 (render_transcript_assistant_meta ... render_transcript_assistant_message_tagged)
transcript = get(170, 824, 'transcript.rs')
verify(transcript, 'transcript', first_ok=('pub fn render_transcript_assistant_meta'), last_ok=('}'))

# tool_result_text: 1119-1144 (append to transcript)
tres = get(1119, 1144, 'tool_result_text')
verify(tres, 'tres', first_ok=('fn tool_result_text'), last_ok=('}'))

# tools: 826-1099 (title_case_word ... render_bash_output_block; 826-827 are doc comments)
tools = get(826, 1099, 'tools.rs')
verify(tools, 'tools', first_ok=('', '///', 'fn title_case_word'), last_ok=('}'))

# commands: 1148-1403 (render_system_api_error ... render_task_assignment)
commands = get(1148, 1403, 'commands.rs')
verify(commands, 'commands', first_ok=('pub fn render_system_api_error'), last_ok=('}'))

# tests: 1411-1729 (unwrapped mod tests; 1410 is `mod tests {`, 1411 is use super::*;)
tests_raw = get(1411, total, 'tests.rs (raw)')
verify(tests_raw, 'tests', first_ok=('use super::*'), last_ok=('}'))

# ---------------------------------------------------------------------------
# Build files
# ---------------------------------------------------------------------------

os.makedirs(DST, exist_ok=True)
# Do NOT delete existing files (markdown.rs, markdown_enhanced.rs, cache.rs stay)

SHARED_HELPERS_IMPORTS = """// messages/helpers.rs — Shared rendering primitives for message renderers.
//
// Extracted from messages/mod.rs. Low-level helpers used by the
// transcript, tool, and command renderers.

use super::*;
use crate::tui::app::TurnMetadata;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

"""

TRANSCRIPT_IMPORTS = """// messages/transcript.rs — Transcript (assistant / user / reasoning) renderers.
//
// Extracted from messages/mod.rs. Renders assistant metadata, live
// streaming text, user messages with file/attachment segments, thinking
// blocks, and the tagged assistant message.

use super::*;
use crate::tui::adapter_types::types::{ContentBlock, Message, ToolResultContent};
use crate::tui::app::TurnMetadata;
use crate::tui::transcript_turn::reasoning_heading;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

"""

TOOLS_IMPORTS = """// messages/tools.rs — Tool-use and tool-result renderers.
//
// Extracted from messages/mod.rs. Renders tool call summaries, file
// read/write results, generic success/error results, and bash I/O.

use super::*;
use crate::tui::adapter_types::types::{ContentBlock, ToolResultContent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

"""

COMMANDS_IMPORTS = """// messages/commands.rs — System, command, and goal-event renderers.
//
// Extracted from messages/mod.rs. Renders API errors, slash-command
// echoes, memory inputs, local command output, collapsed read/search,
// task assignments, and goal blocks.

use super::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

"""

TESTS_IMPORTS = """// messages/tests.rs — Unit tests for the messages module.
//
// Extracted from messages/mod.rs.

use super::*;

"""


def write(name, header, block):
    path = DST + name
    with open(path, 'w') as f:
        f.write(header)
        f.writelines(block)
    content = header + ''.join(block)
    opens = content.count('{')
    closes = content.count('}')
    status = 'OK' if opens == closes else f'UNBALANCED (diff {opens - closes})'
    print(f'  wrote {name}: {len(block)} lines, braces {status}')


write('helpers.rs', SHARED_HELPERS_IMPORTS, helpers + ['\n'] + trunc)
write('transcript.rs', TRANSCRIPT_IMPORTS, transcript + ['\n'] + tres)
write('tools.rs', TOOLS_IMPORTS, tools)
write('commands.rs', COMMANDS_IMPORTS, commands)

# tests.rs: unwrap mod tests body
inner_start = None
for i, ln in enumerate(tests_raw):
    if ln.strip() == 'use super::*;':
        inner_start = i
        break
if inner_start is None:
    raise SystemExit('tests: could not find inner use super::*;')
inner = tests_raw[inner_start + 1:]
while inner and inner[-1].strip() in ('', '}'):
    if inner[-1].strip() == '}':
        inner = inner[:-1]
        break
    inner = inner[:-1]
write('tests.rs', TESTS_IMPORTS, inner)

print('\nDone. Now rewrite mod.rs.')
