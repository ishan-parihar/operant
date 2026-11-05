#!/usr/bin/env python3
"""Command sweep: drive every REGISTERED slash command through the headless
simulator and report which ones are actually intercepted (consumed) vs falling
through to the model as literal prompt text.

Method: `tui debug simulate --keys '/<cmd><enter>' --assert 'messages == 0'`.
A consumed command leaves the transcript empty; a fall-through submits the
command as a user prompt (messages >= 1). Commands that legitimately append
transcript rows as part of their action will report fall-through here and must
be annotated in EXPECTED below.

THREE OUTCOMES, not two (2026-11-05, iter-1748). The sweep used to read ANY
non-zero exit as a fall-through, so a simulator that could not start at all —
no writable HOME, a missing shared library, a crash before the first frame —
was reported as 98 broken commands. That is the RT-162 shape: a gate whose
failures are all one cause reports that cause once per command, and the next
agent reads "98 broken" as 98 defects. The sweep now separates:

  * CONSUMED         — the command was intercepted (the assertion held).
  * FELL THROUGH     — the command reached the model as prompt text (the
                       assertion failed with `messages != 0`). This is the
                       defect class, and only this class.
  * SIMULATOR ERROR  — the run did not complete, so the command's behaviour is
                       UNKNOWN. Reported separately, and still a non-zero exit:
                       a gate that cannot verify must not pass.

The run is also HERMETIC: a fresh HOME + HERMES_HOME per sweep, plus the pins
the corpus uses for colour depth and PATH. Without that, the answer depended on
the developer's machine — their sessions, skills and slash-usage store were in
scope, and a read-only HOME made every command look broken.

Usage: python3 command_sweep.py [--binary path] [--config path]
"""
import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent

# Commands whose handlers intentionally append rows / open flows that submit.
EXPECTED_FALLTHROUGH = set()

# Registry source of truth.
APP_RS = ROOT.parents[1] / "src" / "tui" / "operant_app" / "app.rs"

# The pins that make a run reproducible. `HOME`/`HERMES_HOME` are set per run
# (a fresh fixture), the rest come from here.
PINNED_ENV = {
    "OPERANT_COLOR_DEPTH": "truecolor",
    "TERM": "xterm-256color",
    "TZ": "UTC",
    "LANG": "C.UTF-8",
    "LC_ALL": "C.UTF-8",
    # The audio probe resolves a recorder on PATH; when it cannot, the TUI
    # shows a voice-mode notice that shifts the layout. The sweep asserts on
    # the transcript, not the layout, but a pinned PATH keeps the run's
    # FRAMES identical to the corpus's.
    "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
}

# The simulator's own failure message for a failed assertion
# (`cmd_tui_debug.rs::evaluate_assertions`). A fall-through shows this; a
# simulator that died earlier does not.
ASSERTION_FAILED = "Assertion failed: messages =="


def registered_commands() -> list[str]:
    src = APP_RS.read_text(encoding="utf-8")
    return sorted(
        set(re.findall(r'RegisteredCommand::(?:public|hidden)\("(/[a-z-]+)"', src))
    )


def classify(binary: str, config: str, cmd: str, env: dict[str, str]) -> str:
    """`consumed`, `fell-through`, or `error` for one command."""
    try:
        out = subprocess.run(
            [
                binary, "-c", config, "tui", "debug", "simulate",
                "--keys", f"{cmd}<enter>", "--size", "100x30",
                "--assert", "messages == 0",
            ],
            capture_output=True, text=True, timeout=60, env=env, cwd=env["HOME"],
        )
    except subprocess.TimeoutExpired:
        return "error"
    if out.returncode == 0:
        return "consumed"
    combined = f"{out.stdout}\n{out.stderr}"
    if ASSERTION_FAILED in combined:
        return "fell-through"
    return "error"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", default=str(ROOT.parents[3] / "target/debug/operant"))
    ap.add_argument("--config", default=str(ROOT.parents[3] / "operant.example.toml"))
    args = ap.parse_args()

    cmds = registered_commands()
    print(f"registry lists {len(cmds)} commands; sweeping consumption…")
    fell_through: list[str] = []
    errors: list[str] = []
    consumed = 0
    # One fixture for the whole sweep: the point of the fresh HOME is that the
    # OPERATOR's state is not in scope, not that each command gets its own
    # (a shared fixture is also what the corpus does per RENDER).
    with tempfile.TemporaryDirectory(prefix="operant_command_sweep_") as tmp:
        home = Path(tmp) / "home"
        (home / ".operant").mkdir(parents=True, exist_ok=True)
        env = dict(PINNED_ENV)
        env["HOME"] = str(home)
        env["HERMES_HOME"] = str(home / ".operant")
        env.pop("NO_COLOR", None)
        for c in cmds:
            outcome = classify(args.binary, args.config, c, env)
            if outcome == "consumed":
                consumed += 1
            elif outcome == "fell-through":
                fell_through.append(c)
            else:
                errors.append(c)

    known = set(fell_through) & EXPECTED_FALLTHROUGH
    broken = [c for c in fell_through if c not in EXPECTED_FALLTHROUGH]
    print(f"consumed:      {consumed}/{len(cmds)}")
    print(f"expected ft:   {len(known)}")
    if broken:
        print("BROKEN (fall through to the model as prompt text):")
        for c in broken:
            print(f"  {c}")
    if errors:
        print(
            "SIMULATOR ERROR (the run did not complete — the command's behaviour "
            "is UNKNOWN, not broken):"
        )
        for c in errors:
            print(f"  {c}")
        print(
            "  (a simulator that cannot start makes every command look broken; "
            "check HOME's writability, the binary's shared libraries, and the "
            "config path before reading anything into this list)"
        )
    return 1 if (broken or errors) else 0


if __name__ == "__main__":
    sys.exit(main())
