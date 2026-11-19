#!/usr/bin/env python3
"""Fail when the lib test suite has a failure nobody has accounted for.

THE PROBLEM THIS SOLVES. `cargo test -p operant-core --lib` was red at tip
for months (MEASURED at iter-2711: 27 failed). A suite that is always red
carries no information, so every audit had to run each candidate test three
ways -- in parallel, serially, and in isolation -- to tell a real defect from
pollution. That tax was paid repeatedly, and it is why a genuine config bug
(iter-2699) and a genuine cwd-deletion bug (iter-2711) both hid inside a
field of 27 identical-looking failures.

The fix is NOT "make the suite green" (iter-2711 did that: 27 -> 5). The fix
is that a NEW failure must be VISIBLE. So this script names the failures that
are expected in a bare checkout, and fails on any failure that is not on that
list.

THE BASELINE. `KNOWN_ENVIRONMENTAL` holds the failures a clean worktree
produces for reasons that are NOT defects: the kernel-sidecar tests need
`kernel-sidecar/.venv`, which is untracked provisioning (AGENTS.md fleet rule
6), so a throwaway worktree lacks it and those tests fail while passing in
the main tree. Each entry names WHY, so a reader can check the reason instead
of trusting the list. `--check` exits non-zero only for a failure that is NOT
in the baseline -- i.e. exactly the case nobody has looked at.

This is the same shape as `scripts/audit-config-surface.py`'s ACCOUNTED_FOR
baseline, and for the same stated reason: a check that is always red carries
no information.

Usage:
    python3 scripts/test-pollution-canary.py            # report only
    python3 scripts/test-pollution-canary.py --check     # gate

Exit: 0 (report only) unless --check is given and an UNACCOUNTED failure
exists, or the suite failed in a way the baseline does not describe.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Failures a clean checkout produces for a reason that is NOT a defect.
#
# Keyed by the test's full path as `cargo test` prints it. Each value is the
# reason, so a reader can verify it rather than trust it. Adding an entry here
# is a DECISION that a failure is expected; it is not a way to make a red
# suite pass. If you add one, say why in the commit.
KNOWN_ENVIRONMENTAL = {
    "tools::kernel::tests::ping_roundtrip": (
        "kernel-sidecar: needs kernel-sidecar/.venv, which is untracked "
        "provisioning (AGENTS fleet rule 6). Passes in the main tree, which "
        "has it -- VERIFIED iter-2711: 12 passed, 0 failed."
    ),
    "tools::kernel::tests::harness_apply_and_rollback_roundtrip": (
        "kernel-sidecar: same missing .venv as ping_roundtrip."
    ),
    "tools::kernel::tests::kernel_state_lists_the_requested_kind": (
        "kernel-sidecar: same missing .venv as ping_roundtrip."
    ),
    "tools::kernel::tests::a_harness_upsert_invalidates_the_injection_cache": (
        "kernel-sidecar: same missing .venv as ping_roundtrip."
    ),
    "tools::kernel::tests::a_holon_run_gates_the_continual_harness_before_its_lessons_r10": (
        "kernel-sidecar: same missing .venv as ping_roundtrip."
    ),
}

# `cargo test` prints the failing names in a trailing block, not on the
# `test <name> ... FAILED` lines (those go to the terminal only with
# `--nocapture`, and even then they interleave with test output). The block is:
#
#   failures:
#
#       agent::tests::foo
#       tools::kernel::tests::bar
#
#   test result: FAILED. 3650 passed; 5 failed; ...
#
# Reading THAT is reliable across cargo versions; reading the progress lines is
# not, and an earlier version of this script did exactly that and reported
# "no parseable failure line" on a suite with 5 real failures.
RESULT_RE = re.compile(r"^test result: (\w+)\. (\d+) passed; (\d+) failed")


def parse_failures(output: str) -> list[str]:
    """Names from the trailing `failures:` block.

    The block is indented four spaces and ends at the first non-indented,
    non-blank line. `---- <name> stdout ----` headers appear EARLIER and must
    not be mistaken for the block, hence the `startswith("----")` guard.
    """
    names: list[str] = []
    in_block = False
    for line in output.splitlines():
        if not in_block:
            if line.strip() == "failures:" and not line.startswith("----"):
                in_block = True
            continue
        if line.startswith("    ") and line.strip():
            names.append(line.strip())
        elif line.strip():
            in_block = False
    return names


def run_suite() -> tuple[int, list[str], str]:
    """Run the lib suite and return (exit_code, failed_names, summary)."""
    proc = subprocess.run(
        [
            "cargo",
            "test",
            "-p",
            "operant-core",
            "--lib",
            "--",
            "--test-threads=1",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    out = proc.stdout + proc.stderr
    names = parse_failures(out)
    summary = ""
    for line in out.splitlines():
        if RESULT_RE.match(line):
            summary = line
    return proc.returncode, names, summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero when an UNACCOUNTED failure exists",
    )
    args = parser.parse_args()

    code, failed, summary = run_suite()

    unaccounted = [n for n in failed if n not in KNOWN_ENVIRONMENTAL]
    accounted = [n for n in failed if n in KNOWN_ENVIRONMENTAL]

    print(summary or "(no test result line found)")

    if accounted:
        print(f"\naccounted for ({len(accounted)}), each a known environment gap:")
        for name in accounted:
            print(f"  {name}")
            print(f"      {KNOWN_ENVIRONMENTAL[name]}")

    if unaccounted:
        print(f"\nUNACCOUNTED FAILURE(S) ({len(unaccounted)}):")
        for name in unaccounted:
            print(f"  {name}")
        print(
            "\nThese are not in the baseline. Either fix them, or -- if the\n"
            "failure is genuinely environmental -- add it to KNOWN_ENVIRONMENTAL\n"
            "in this file WITH the reason. Do not add one to silence a real bug."
        )
        return 1

    if code != 0 and not failed:
        print("\nthe suite failed without printing a parseable failure line")
        return 1

    print(f"\ncanary: PASS ({len(failed)} failure(s), all accounted for)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
