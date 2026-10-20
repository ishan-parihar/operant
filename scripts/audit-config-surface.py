#!/usr/bin/env python3
"""Report config settings that nothing outside the loader reads.

Every `pub` field of every settings struct in
`crates/operant-core/src/config.rs` is matched against all NON-TEST code in
`crates/`. A field with zero other references is either dead surface (a key
the template advertises and the binary ignores) or a leftover shim that
warns when set.

Test code is removed by brace matching, not truncation: an earlier version
cut each file at its first `#[cfg(test)]` and reported fields as unused that
a test module happened to precede (`feed_seat_map`,
`session_summary_ttl_minutes` are read in `gateway_runner.rs`). Check a
candidate by hand before removing anything — the settings structs use
`deny_unknown_fields`, so a removed key breaks existing configs; the
sanctioned fix is accept + warn.

Usage: python3 scripts/audit-config-surface.py [--check]
Exit: 0 (report only) unless --check is given and candidates exist.
"""
import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent / "crates"
CONFIG = ROOT / "operant-core" / "src" / "config.rs"


def strip_test_items(text: str) -> str:
    """Remove every `#[cfg(test)]`-attributed item, braces included."""
    out = []
    i = 0
    while True:
        m = re.search(r"#\[cfg\(test\)\]", text[i:])
        if not m:
            out.append(text[i:])
            return "".join(out)
        start = i + m.start()
        out.append(text[i:start])
        brace = text.find("{", start)
        if brace == -1:
            i = start + len("#[cfg(test)]")
            continue
        depth = 0
        j = brace
        while j < len(text):
            if text[j] == "{":
                depth += 1
            elif text[j] == "}":
                depth -= 1
                if depth == 0:
                    j += 1
                    break
            j += 1
        i = j


def main() -> int:
    src = strip_test_items(CONFIG.read_text(encoding="utf-8"))
    structs = {}
    for m in re.finditer(r"pub struct (\w+)\s*\{(.*?)\n\}", src, re.S):
        structs[m.group(1)] = re.findall(r"^\s*pub (\w+)\s*:", m.group(2), re.M)

    uses = collections.Counter()
    for path in ROOT.rglob("*.rs"):
        if path == CONFIG or "/tests/" in str(path):
            continue
        text = strip_test_items(path.read_text(encoding="utf-8", errors="replace"))
        for ident in set(re.findall(r"\b[a-z_][a-z0-9_]{2,}\b", text)):
            uses[ident] += 1

    total = 0
    for name, fields in structs.items():
        dead = [f for f in fields if uses[f] == 0]
        if not dead:
            continue
        total += len(dead)
        print(f"{name}: {len(dead)}/{len(fields)}")
        for field in dead:
            print(f"    {field}")
    print(f"\n{total} candidates across {len(structs)} structs")
    if "--check" in sys.argv and total:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
