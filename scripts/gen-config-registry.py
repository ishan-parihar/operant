#!/usr/bin/env python3
"""Generate the config key registry from the settings structs' source.

    python3 scripts/gen-config-registry.py            # write the file
    python3 scripts/gen-config-registry.py --check    # fail if it is stale

WHY GENERATED AND NOT HAND-WRITTEN
==================================
The registry is the single list of every dotted config key this binary
accepts. Two consumers read it:

  * `crates/operant-core/src/config_registry.rs` — the runtime's "is this key
    known?" test, which decides whether an unknown key from a NEWER binary
    gets parked (load) or fails (refuse).
  * `scripts/audit-config-keys.py --check` — the forward gate that fails when
    a declared key is missing from any surface the operator or an agent reads.

A hand-maintained list drifts from the struct the moment someone adds a field,
and the drift is invisible: the struct compiles, the loader accepts the new
key, and the list quietly stops describing the binary. Generating it makes
the struct the only place a key is declared, so the registry cannot lag.

WHAT IT EXTRACTS
================
Every `pub` field of every settings struct in `config.rs`, with:

  * its dotted path (`section.field`, or `section.sub.field` for a nested
    settings struct reached through a named field),
  * its default, taken from the struct's `Default` impl when the file has one,
  * its NOTE — the doc comment on the field, verbatim.

The note is the single source the plan's L3 surface requires: `config
describe` and `config notes <key>` render THIS text, so the doc comment is
what an agent reads and there is no second copy to drift.

WHAT IT DELIBERATELY DOES NOT DO
================================
It does not evaluate Rust. A default that is a function call is recorded as
the call's source text (`ClientSettings::default()`), which is enough to name
the key in the describe surface and honest about where the value comes from.
A field whose type is a `HashMap` is recorded with the note that its entries
are the operator's own namespace.

Usage: python3 scripts/gen-config-registry.py [--check]
Exit: 0 always for --check unless the generated file is STALE.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CONFIG = ROOT / "crates" / "operant-core" / "src" / "config.rs"
OUT = ROOT / "crates" / "operant-core" / "src" / "config_registry.rs"

# Settings structs are the ones reachable as a TOP-LEVEL `[section]` table.
# A struct that only ever appears nested is reached through its parent field.
TOP_LEVEL_SECTIONS = [
    "client",
    "agent",
    "logging",
    "tui",
    "mcp",
    "skills",
    "gateway",
    "plugins",
    "tools",
    "tts",
    "memory",
    "browser",
    "vision",
    "credential_pool",
    "checkpoints",
    "harness",
    "bylaws",
    "context_injection",
    "socialization",
    "pk",
    "auxiliary_models",
    "moa",
    "advisor",
    "recap",
    "holon",
    "integrations",
    "compression",
    "autonomous",
]

# Nested settings structs: the dotted prefix they live under, and the field
# name that reaches them. `client.rate_limit` is the shape every operator
# recognises, so the nesting is expressed rather than flattened.
NESTED = {
    "client": ["rate_limit"],
    "agent": ["retry"],
    "tools": ["web", "http", "terminal", "code_execution", "kernel", "stt", "tool_search"],
}


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


def struct_span(src: str, name: str) -> tuple[int, int] | None:
    """The (start, end) offsets of `pub struct <name> { .. }`'s BODY."""
    m = re.search(r"pub struct " + re.escape(name) + r"\s*\{", src)
    if not m:
        return None
    start = m.end() - 1
    depth = 0
    i = start
    while i < len(src):
        if src[i] == "{":
            depth += 1
        elif src[i] == "}":
            depth -= 1
            if depth == 0:
                return (start + 1, i)
        i += 1
    return None


def fields_of(src: str, start: int, end: int) -> list[tuple[str, str]]:
    """(field name, note) for every `pub` field in `src[start:end]`.

    The body is scanned in place rather than sliced, because a field's NOTE
    lives on the lines above it and the slice's line numbering would point at
    the wrong text. Only TOP-LEVEL fields are returned: a line inside a
    nested `#[serde(...)]` attribute or a multi-line type is not a field
    declaration, and the brace depth is what tells them apart.
    """
    out: list[tuple[str, str]] = []
    # Absolute offset of the first body line, so a relative line index can be
    # converted back for the doc-comment read.
    body_start_line = src.count("\n", 0, start)
    body_lines = src[start:end].splitlines()
    depth = 0
    for i, line in enumerate(body_lines):
        stripped = line.strip()
        if stripped.startswith("}") or stripped.startswith("]"):
            depth -= 1
        m = re.match(r"\s*pub ([a-z_][a-z0-9_]*)\s*:", line)
        if m and depth == 0:
            out.append((m.group(1), doc_comment_at(src, body_start_line + i)))
        depth += line.count("{") - line.count("}")
    return out


def doc_comment_at(src: str, line_idx: int) -> str:
    """Collect the `///` block documenting the item at `line_idx`.

    The walk back skips ATTRIBUTE lines: a field is usually written

        /// The note.
        ///
        /// A second paragraph.
        #[serde(default)]
        pub field: T,

    and a doc block that stops at the first non-`///` line would see the
    attribute, not the note, and return nothing — which is how the first
    version of this generator produced 191 fields with no note at all.
    """
    lines = src.splitlines()
    out: list[str] = []
    i = line_idx - 1
    while i >= 0:
        stripped = lines[i].strip()
        if stripped.startswith("///"):
            out.append(stripped[3:].strip())
            i -= 1
            continue
        if stripped.startswith("#["):
            # An attribute between the doc block and its field. Keep walking.
            i -= 1
            continue
        break
    # Drop the empty paragraphs a `///` separator line leaves behind, so the
    # rendered note reads as prose rather than as `para  para`.
    return " ".join(part for part in reversed(out) if part).strip()


def default_map(src: str) -> dict[str, str]:
    """`Struct -> {field: rendered default}` from each `impl Default`."""
    maps: dict[str, dict[str, str]] = {}
    for m in re.finditer(r"impl Default for (\w+)\s*\{", src):
        name = m.group(1)
        # Walk to the matching close brace of the impl.
        start = m.end() - 1
        depth = 0
        i = start
        while i < len(src):
            if src[i] == "{":
                depth += 1
            elif src[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        body = src[start:i]
        fields = {}
        for fm in re.finditer(r"(\w+)\s*:\s*([^,\n][^\n]*?),?\s*(?:\n|//)", body):
            fields[fm.group(1)] = fm.group(2).strip().rstrip(",")
        maps[name] = fields
    return maps


def main() -> int:
    if not CONFIG.exists():
        print(f"missing {CONFIG}", file=sys.stderr)
        return 2
    src = strip_test_items(CONFIG.read_text(encoding="utf-8"))
    defaults = default_map(src)

    rows: list[tuple[str, str, str]] = []  # (dotted, default, note)

    def collect(struct_name: str, prefix: str) -> None:
        span = struct_span(src, struct_name)
        if span is None:
            return
        dmap = defaults.get(struct_name, {})
        for field, note in fields_of(src, span[0], span[1]):
            dotted = f"{prefix}.{field}" if prefix else field
            default = dmap.get(field, "")
            if not default:
                default = f"<{struct_name}::default()>"
            rows.append((dotted, default, note or "<no note>"))

    # Top-level sections: `AppConfig`'s own fields name the section, and each
    # section's struct supplies the leaves.
    app_span = struct_span(src, "AppConfig")
    section_struct: dict[str, str] = {}
    if app_span:
        for field, _note in fields_of(src, app_span[0], app_span[1]):
            m = re.search(
                r"pub " + re.escape(field) + r"\s*:\s*([A-Za-z_][A-Za-z0-9_]*)",
                src,
            )
            if m:
                section_struct[field] = m.group(1)

    for section in TOP_LEVEL_SECTIONS:
        struct_name = section_struct.get(section)
        if struct_name:
            collect(struct_name, section)

    # A nested settings struct is reached through its parent field, so the
    # dotted prefix is the parent path -- not a synthesised name. The parent
    # field names the struct, so the child's keys are `section.child.field`.
    for section, children in NESTED.items():
        parent_span = struct_span(src, section_struct.get(section, ""))
        if not parent_span:
            continue
        for field, _note in fields_of(src, parent_span[0], parent_span[1]):
            if field not in children:
                continue
            m = re.search(r"pub " + field + r"\s*:\s*([A-Za-z_][A-Za-z0-9_]*)", src)
            if m:
                collect(m.group(1), f"{section}.{field}")

    # Deduplicate while keeping declaration order: a nested struct collected
    # under a synthesised name and under its real path would otherwise appear
    # twice with different prefixes.
    seen: set[str] = set()
    uniq: list[tuple[str, str, str]] = []
    for row in rows:
        if row[0] in seen:
            continue
        seen.add(row[0])
        uniq.append(row)

    rendered = render(uniq)
    if "--check" in sys.argv:
        if not OUT.exists():
            print("DRIFT the registry does not exist: run scripts/gen-config-registry.py")
            return 1
        current = OUT.read_text(encoding="utf-8")
        if current != rendered:
            print(
                "DRIFT the config key registry is stale: the settings structs "
                "changed and the registry was not regenerated.\n"
                "  python3 scripts/gen-config-registry.py"
            )
            return 1
        print(f"registry is current: {len(uniq)} keys")
        return 0

    OUT.write_text(rendered, encoding="utf-8")
    print(f"wrote {OUT.relative_to(ROOT)}: {len(uniq)} keys")
    return 0


def render(rows: list[tuple[str, str, str]]) -> str:
    """The generated Rust module.

    `include_str!` of the config source is not an option (the registry must be
    a static list, not a parse at startup), so the rows are emitted as a
    `&[(&str, &str, &str)]` the runtime can binary-search by key.
    """
    lines = [
        "//! GENERATED FILE — do not edit.",
        "//!",
        "//! Source: `crates/operant-core/src/config.rs` settings structs.",
        "//! Generator: `scripts/gen-config-registry.py` (run it, or pass",
        "//! `--check` to fail when this file is stale).",
        "//!",
        "//! Every dotted config key this build accepts, with its default and",
        "//! the NOTE from the field's doc comment. The note is the single source",
        "//! `operant config describe` / `config notes` render, so the registry",
        "//! and the operator-facing surface cannot drift apart.",
        "",
        "/// `(dotted key, default, note)`, sorted by dotted key.",
        "pub const CONFIG_KEYS: &[(&str, &str, &str)] = &[",
    ]
    for dotted, default, note in sorted(rows):
        safe_default = default.replace("\\", "\\\\").replace('"', '\\"')
        safe_note = note.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(f'    ("{dotted}", "{safe_default}", "{safe_note}"),')
    lines.append("];")
    lines.append("")
    lines.append("/// Every dotted key this build accepts.")
    lines.append("pub const KNOWN_KEYS: &[&str] = &[")
    for dotted, _default, _note in sorted(rows):
        lines.append(f'    "{dotted}",')
    lines.append("];")
    lines.append("")
    return "\n".join(lines)


if __name__ == "__main__":
    sys.exit(main())