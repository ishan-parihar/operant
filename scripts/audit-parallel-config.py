#!/usr/bin/env python3
"""Report the config surface that no live config reads.

`operant-core/src/config.rs` holds the LIVE config (`AppConfig`): the
struct the CLI parses the operator's `operant.toml` into and every runtime
path reads.

`operant-config/src/schema/` holds the TYPES the live config composes
(the section structs `AppConfig` embeds) plus a set of section structs no
live config carries. The AGGREGATE that used to sit on top of them
(`operant_config::schema::Config`, with its own loader and migration) was
retired in iter-1138 — it was never loaded in production (RT-141).

This script re-derives what is left: for every settings struct in the
schema directory, it counts production references OUTSIDE the
`operant-config` crate (test code stripped by brace matching, never by
truncation). A struct with zero outside readers is dormant surface.

It is a heuristic (identifier occurrence counting, not type-aware): a field
read only through a macro or reflection is missed, and a name shared with
an unrelated symbol is a false hit. The script prints candidates for review,
not a verdict.

Usage: python3 scripts/audit-parallel-config.py [--check]
Exit: 0 (report only) unless --check is given and a dormant struct appears.
"""
import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent / "crates"
SCHEMA_DIR = ROOT / "operant-config" / "src" / "schema"
CRATE = ROOT / "operant-config"


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
    structs: dict[str, list[str]] = {}
    for path in sorted(SCHEMA_DIR.rglob("*.rs")):
        text = strip_test_items(path.read_text(encoding="utf-8", errors="replace"))
        for m in re.finditer(r"pub struct (\w+)\s*\{(.*?)\n\}", text, re.S):
            fields = re.findall(r"^\s*pub (\w+)\s*:", m.group(2), re.M)
            structs[m.group(1)] = fields

    # Production text OUTSIDE the operant-config crate.
    outside = []
    for path in ROOT.rglob("*.rs"):
        if CRATE in path.parents or "/tests/" in str(path):
            continue
        outside.append(
            strip_test_items(path.read_text(encoding="utf-8", errors="replace"))
        )
    blob = "\n".join(outside)
    outside_idents = collections.Counter(re.findall(r"\b\w+\b", blob))

    dormant = []
    for name, fields in sorted(structs.items()):
        readers = outside_idents.get(name, 0)
        if readers:
            continue
        dormant.append((name, len(fields)))

    print(f"schema structs found: {len(structs)}")
    print(f"with no production reader outside operant-config: {len(dormant)}")
    total_fields = sum(count for _, count in dormant)
    print(f"fields in those structs: {total_fields}\n")
    for name, count in dormant:
        print(f"    {name} ({count} fields)")
    if "--check" in sys.argv and dormant:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
