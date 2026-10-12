"""Resolve the operant home for standalone skill scripts.

Skill scripts may run outside the operant process (system Python, nix env,
CI) where ``hermes_constants`` is not importable.  This module provides the
same ``get_operant_home()`` contract without requiring it on ``sys.path``.

When ``hermes_constants`` IS available it is used directly so profile
resolution and any future enhancements are picked up automatically.
"""

from __future__ import annotations

import os
from pathlib import Path

try:
    from hermes_constants import get_operant_home as get_operant_home
except (ModuleNotFoundError, ImportError):

    def get_operant_home() -> Path:
        """Return the operant home directory (default: ``~/.operant``)."""
        val = os.environ.get("OPERANT_HOME", "").strip()
        return Path(val) if val else Path.home() / ".operant"
