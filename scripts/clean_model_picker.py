#!/usr/bin/env python3
"""Clean up unused imports left over from the model_picker.rs decomposition."""

import re


def patch(fp: str, old: str, new: str, label: str) -> None:
    with open(fp) as f:
        c = f.read()
    if old not in c:
        print(f"SKIP {label}: pattern not found in {fp}")
        return
    c = c.replace(old, new, 1)
    with open(fp, "w") as f:
        f.write(c)
    print(f"OK   {label}: patched {fp}")


# 1. Hub: remove unused ratatui imports (render.rs now has its own)
patch(
    "crates/operant-cli/src/tui/model_picker.rs",
    """use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

""",
    "",
    "hub ratatui imports",
)

# 2. effort.rs: drop unused `use super::*` (self-contained module)
patch(
    "crates/operant-cli/src/tui/model_picker/effort.rs",
    "use super::*;\n\n",
    "",
    "effort.rs super glob",
)

# 3. models.rs: drop unused `use super::*` (self-contained module)
patch(
    "crates/operant-cli/src/tui/model_picker/models.rs",
    "use super::*;\n\n",
    "",
    "models.rs super glob",
)
