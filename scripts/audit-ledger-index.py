#!/usr/bin/env python3
"""Check the audit ledger's index table against its own checkboxes (RG-016).

`docs/audit/2026-06-19-BUGS.md` keeps each row's state in THREE places:
the checkbox (`- [x]`), the "What is OPEN right now" index table's State
column, and the count prose. Closing a defect flips the checkbox, because
that is the line the fixing iteration edits. Nothing walks back to the other
two — so the table and the prose go stale in BOTH directions: a closed row
still reading "OPEN", and an open row missing from the table entirely.

That is RG-016. This script is its detection, and it is a set difference in
both directions, which is what a reader would otherwise have to do by eye:

  * every index row whose State does not begin "CLOSED" must name an id that
    IS an open checkbox, and
  * every open checkbox must HAVE an index row.

Two ids are sanctioned exceptions and are named here so a reader does not
have to re-derive why: `S2`, `D-5b` and `D-6` are HEADING rows (their rows
are `### D-5b` etc.) rather than checkboxes, which the ledger states in the
paragraph under the table. When a heading row is closed, mark it CLOSED in
the table or this reports it forever.

Exit 1 when the two sets disagree. Run it before acting on any row in that
table; a hand-read is what missed the 2026-10-29 defects.
"""

import pathlib
import re
import sys

LEDGER = pathlib.Path(__file__).resolve().parent.parent / "docs" / "audit" / "2026-06-19-BUGS.md"

# Ids whose rows are `### <id>` headings, not `- [x]` checkboxes. The ledger
# says so in the paragraph under the table; they are listed here so the
# comparison does not read them as unexplained differences.
HEADING_ROWS = {"S2", "D-5b", "D-6"}

ID = r"(RT-\d+|S\d|D-\d+[a-z]?)"


def main() -> int:
    if not LEDGER.exists():
        print(f"ledger not found: {LEDGER}", file=sys.stderr)
        return 2
    text = LEDGER.read_text()

    open_checkbox = set(re.findall(rf"^- \[ \] \*\*{ID}", text, re.M))

    table = re.search(
        r"\|\s*Item\s*\|\s*Section\s*\|\s*State\s*\|\n\|---\|---\|---\|\n(.*?)\n\n",
        text,
        re.S,
    )
    if not table:
        print("index table not found — has its header changed?", file=sys.stderr)
        return 2

    all_rows = set()
    non_closed = set()
    for line in table.group(1).splitlines():
        m = re.match(rf"^\|\s*{ID}\s*\|", line)
        if not m:
            continue
        all_rows.add(m.group(1))
        if "CLOSED" not in line:
            non_closed.add(m.group(1))

    missing_from_table = sorted(open_checkbox - all_rows)
    # Non-CLOSED rows that are neither an open checkbox nor a sanctioned
    # heading row: a row the table calls open that the checkboxes call closed.
    stale_open = sorted(non_closed - open_checkbox - HEADING_ROWS)

    print(f"open checkboxes       : {', '.join(sorted(open_checkbox)) or '(none)'}")
    print(f"index rows            : {len(all_rows)}")
    print(f"index non-CLOSED rows : {', '.join(sorted(non_closed)) or '(none)'}")

    if missing_from_table:
        print(f"\nFAIL: open row(s) with NO index entry: {', '.join(missing_from_table)}")
    if stale_open:
        print(f"\nFAIL: index row(s) not CLOSED but closed in the checkboxes: {', '.join(stale_open)}")

    if missing_from_table or stale_open:
        print(
            "\nThe checkboxes are the record of the fix; the table is a claim "
            "about them. Re-derive the table (RG-016).",
            file=sys.stderr,
        )
        return 1

    print("\nOK: the index table and the checkboxes agree (RG-016)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
