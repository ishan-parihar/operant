#!/usr/bin/env python3
"""Command sweep: drive every REGISTERED slash command through the headless
simulator and report which ones are actually intercepted (consumed) vs falling
through to the model as literal prompt text.

Method: `tui debug simulate --keys '/<cmd><enter>' --assert 'messages == 0'`.
A consumed command leaves the transcript empty; a fall-through submits the
command as a user prompt (messages >= 1). Commands that legitimately append
transcript rows as part of their action will report fall-through here and must
be annotated in EXPECTED below.

Usage: python3 command_sweep.py [--binary path] [--config path]
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent

# Commands whose handlers intentionally append rows / open flows that submit.
EXPECTED_FALLTHROUGH = set()

# Registry source of truth.
APP_RS = ROOT.parents[1] / "src" / "tui" / "operant_app" / "app.rs"


def registered_commands() -> list[str]:
    src = APP_RS.read_text(encoding="utf-8")
    return sorted(
        set(re.findall(r'RegisteredCommand::(?:public|hidden)\("(/[a-z-]+)"', src))
    )


def consumed(binary: str, config: str, cmd: str) -> bool:
    try:
        out = subprocess.run(
            [
                binary, "-c", config, "tui", "debug", "simulate",
                "--keys", f"{cmd}<enter>", "--size", "100x30",
                "--assert", "messages == 0",
            ],
            capture_output=True, text=True, timeout=60,
        )
        return out.returncode == 0
    except subprocess.TimeoutExpired:
        return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", default=str(ROOT.parents[3] / "target/debug/operant"))
    ap.add_argument("--config", default=str(ROOT.parents[3] / "operant.example.toml"))
    args = ap.parse_args()

    cmds = registered_commands()
    print(f"registry lists {len(cmds)} commands; sweeping consumption…")
    fell_through = []
    for c in cmds:
        if not consumed(args.binary, args.config, c):
            fell_through.append(c)

    known = set(fell_through) & EXPECTED_FALLTHROUGH
    broken = [c for c in fell_through if c not in EXPECTED_FALLTHROUGH]
    print(f"consumed:      {len(cmds) - len(fell_through)}/{len(cmds)}")
    print(f"expected ft:   {len(known)}")
    print(f"BROKEN (fall through to the model as prompt text):")
    for c in broken:
        print(f"  {c}")
    return 0 if not broken else 1


if __name__ == "__main__":
    sys.exit(main())
