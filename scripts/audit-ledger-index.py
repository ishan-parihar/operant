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

RG-021 (iter-1819) added the second pair of checks the ledger needed: the
id space itself. The ledger is written by SEVERAL agents on a shared
branch, and two of them can pick the same id — which has happened EIGHT
times in this ledger's life (five in the index table, eight among the detail
sections), each collision recorded only where a reader happened to look. A
lookup table whose ids are ambiguous is not a lookup table, and the set
difference above cannot see it: a set has no duplicates. So:

  * every index row's id must be UNIQUE,
  * every detail section's id must be UNIQUE, and
  * every index row must HAVE a detail section (a row that states a
    finding's fate without carrying the finding is half a row).

The ids that already collide are named in DUPLICATE_IDS and
INDEX_ROWS_WITHOUT_DETAIL below, each with the reason it is there, so the
gate fails on the NEXT collision rather than on the published ones. An id
in either list is a ledger defect to repair, not a licence to collide
again.

Exit 1 when any of the checks fail. Run it before acting on any row in that
table; a hand-read is what missed the 2026-10-29 defects.

Usage: `audit-ledger-index.py [LEDGER_PATH]` — the optional path is what
`scripts/tests/audit_ledger_index_test.sh` plants the defects in.
"""

import collections
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
LEDGER = REPO / "docs" / "audit" / "2026-06-19-BUGS.md"

# Ids whose rows are `### <id>` headings, not `- [x]` checkboxes. The ledger
# says so in the paragraph under the table; they are listed here so the
# comparison does not read them as unexplained differences.
HEADING_ROWS = {"S2", "D-5b", "D-6"}

# Ids that two findings share, in the index table, in the detail sections,
# or in both. Every one of these was published by two agents on the shared
# branch; the git history is append-only, so the collision is RECORDED (an
# ID NOTE in the row) rather than rewritten. They are named here so this
# gate catches the NEXT one instead of failing on the published set forever.
DUPLICATE_IDS = {
    # index table + detail sections
    "RT-218": "two published rows share it (the scroll counters, iter-1754, and the Ctrl+Q catalogue, iter-1758)",
    "RT-234": "two published rows share it (the empty answer, iter-1783, and the /help filter, iter-1785)",
    "RT-239": "two published rows share it (the /help second catalogue, iter-1788, and the sessions-list panic, iter-1791)",
    "RT-258": "two published rows share it (the [proxy] table, iter-1813, and the click behind a panel, iter-1815)",
    "RT-259": "two published rows share it (the deferred bridge, iter-1816, and the wheel in the panels, iter-1818)",
    # detail sections only (the index table carries one row for each)
    "RT-136": "two detail sections share it (both from the 2026-10-29 correction)",
    "RT-175": "two detail sections share it (both from the 2026-10-29 correction)",
    "RT-225": "two detail sections share it (both from the 2026-10-29 correction)",
    "RT-226": "two detail sections share it (both from the 2026-10-29 correction)",
    "RT-227": "two detail sections share it (both from the 2026-10-29 correction)",
}

# Index rows with no detail section: the row states the finding's fate but
# the finding itself is not written down. Named here so the gate reports the
# NEXT one; the five published ones are the concurrent slice's rows from
# iter-1787 (their detail text lives in their commit body) and RT-180.
INDEX_ROWS_WITHOUT_DETAIL = {
    "RT-180": "published without a detail section (pre-dates this campaign's format)",
    "RT-235": "published by the concurrent slice's iter-1787 with an index row only",
    "RT-236": "published by the concurrent slice's iter-1787 with an index row only",
    "RT-237": "published by the concurrent slice's iter-1787 with an index row only",
    "RT-238": "published by the concurrent slice's iter-1787 with an index row only",
}

ID = r"(RT-\d+|S\d+[a-z]?|D-\d+[a-z]?)"


def duplicates(ids: list[str]) -> list[str]:
    """The ids that appear more than once, in numeric order."""
    counts = collections.Counter(ids)
    return sorted(
        (i for i, c in counts.items() if c > 1),
        key=lambda i: (int(re.sub(r"\D", "", i) or 0), i),
    )


def main() -> int:
    ledger = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else LEDGER
    if not ledger.exists():
        print(f"ledger not found: {ledger}", file=sys.stderr)
        return 2
    text = ledger.read_text()

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
    index_ids: list[str] = []
    for line in table.group(1).splitlines():
        m = re.match(rf"^\|\s*{ID}\s*\|", line)
        if not m:
            continue
        index_ids.append(m.group(1))
        all_rows.add(m.group(1))
        if "CLOSED" not in line:
            non_closed.add(m.group(1))

    missing_from_table = sorted(open_checkbox - all_rows)
    # Non-CLOSED rows that are neither an open checkbox nor a sanctioned
    # heading row: a row the table calls open that the checkboxes call closed.
    stale_open = sorted(non_closed - open_checkbox - HEADING_ROWS)

    # RG-021: the id space itself.
    detail_ids = re.findall(rf"^- \[[ x]\] \*\*{ID}", text, re.M)
    duplicate_index = [i for i in duplicates(index_ids) if i not in DUPLICATE_IDS]
    duplicate_detail = [i for i in duplicates(detail_ids) if i not in DUPLICATE_IDS]
    without_detail = [
        i
        for i in sorted(all_rows - set(detail_ids), key=lambda s: (int(re.sub(r"\D", "", s) or 0), s))
        if i not in INDEX_ROWS_WITHOUT_DETAIL and i not in HEADING_ROWS
    ]

    print(f"open checkboxes       : {', '.join(sorted(open_checkbox)) or '(none)'}")
    print(f"index rows            : {len(all_rows)}")
    print(f"index non-CLOSED rows : {', '.join(sorted(non_closed)) or '(none)'}")
    print(f"detail sections       : {len(detail_ids)}")

    if missing_from_table:
        print(f"\nFAIL: open row(s) with NO index entry: {', '.join(missing_from_table)}")
    if stale_open:
        print(f"\nFAIL: index row(s) not CLOSED but closed in the checkboxes: {', '.join(stale_open)}")
    if duplicate_index:
        print(
            "\nFAIL: id(s) used by TWO index rows: "
            f"{', '.join(duplicate_index)}\n"
            "  Two agents on the shared branch picked the same id. The later "
            "publisher renumbers forward and records an ID NOTE."
        )
    if duplicate_detail:
        print(
            "\nFAIL: id(s) used by TWO detail sections: "
            f"{', '.join(duplicate_detail)}\n"
            "  Two agents on the shared branch picked the same id. The later "
            "publisher renumbers forward and records an ID NOTE."
        )
    if without_detail:
        print(
            "\nFAIL: index row(s) with NO detail section: "
            f"{', '.join(without_detail)}\n"
            "  A row that states a finding's fate without carrying the "
            "finding is half a row."
        )

    if (
        missing_from_table
        or stale_open
        or duplicate_index
        or duplicate_detail
        or without_detail
    ):
        print(
            "\nThe checkboxes are the record of the fix; the table is a claim "
            "about them. Re-derive the table (RG-016), and keep the id "
            "space unique (RG-021).",
            file=sys.stderr,
        )
        return 1

    print("\nOK: the index table and the checkboxes agree (RG-016), and the id space is unique (RG-021)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
