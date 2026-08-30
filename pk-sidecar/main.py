#!/usr/bin/env python3
"""Plan 015 / Phase 0 — pk-sidecar scaffold.

This is a STUB. It does not implement the SessionKernel, the NDJSON
server, or the supervisor (those land in Phase 1). Its only job today
is to prove the script exists, is executable, and answers a `ping`
JSON request — so a sidecar-binary gate (scripts/check-pk-sidecar.sh)
can assert the scaffold is in place before any Phase 1 work begins.

The Phase 0 contract:
  - argv 0 is the script path (caller can locate us).
  - on stdin, accept a single JSON line: {"op": "ping"}.
  - on stdout, emit a single JSON line: {"ok": true, "phase": 0, "pid": <int>}.
  - exit code 0 on a handled op, 1 on any parse error.

The 015 plan intentionally keeps Phase 0 small — the supervisor
(health, restart, idle, process-group teardown) is Phase 1 work and
belongs in this file once the Phase 0 scaffold lands + a green gate
proves it.

Once a Phase 1+ rollout flips `[pk].enabled = true` in operant.toml,
the gateway spawns this script (or a wrapped `uv run python3` venv)
and the NDJSON protocol replaces the JSON-line ping.
"""
from __future__ import annotations

import json
import os
import sys
from typing import Any


def _emit(payload: dict[str, Any]) -> None:
    """Write a single JSON line to stdout and flush."""
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def _read_request() -> dict[str, Any] | None:
    """Read one JSON line from stdin. Returns None on EOF."""
    line = sys.stdin.readline()
    if not line:
        return None
    try:
        return json.loads(line)
    except json.JSONDecodeError:
        return None


def main() -> int:
    op_payload = _read_request()
    if op_payload is None:
        _emit({"ok": False, "error": "no-op: empty or non-JSON stdin"})
        return 1

    op = op_payload.get("op", "")
    if op == "ping":
        _emit(
            {
                "ok": True,
                "phase": 0,
                "pid": os.getpid(),
                "version": "0.2.0",
                "plan": "015",
            }
        )
        return 0

    _emit({"ok": False, "error": f"unknown op: {op!r} (Phase 0 supports only 'ping')"})
    return 1


if __name__ == "__main__":
    sys.exit(main())
