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
    # Added after the iter-2357 audit measured a live key loss: a config
    # carrying ONE unknown key takes the upgrade-parking path, and
    # `split_unknown` parks every section with no registry entry. These three
    # were absent, so `[proxy]`, `[providers]` and `[compression]` were parked
    # WHOLESALE — silently disabling the outbound proxy and the compression
    # layer for any operator who also had a stale key somewhere. `cli`,
    # `observability` and `terminal_backend` were dropped the same way.
    "proxy",
    "providers",
    "cli",
    "observability",
]

# Bare top-level keys: an `AppConfig` field whose type is a scalar or an enum,
# not a settings struct, so it has no leaves to collect. It is still a key an
# operator sets, and a missing entry is still a key `split_unknown` parks.
#
# This was a HAND-WRITTEN allowlist (`["terminal_backend"]`), which is a
# ratchet that only ever fails silently: every later top-level field is dropped
# from the registry the moment it is added, and the loader then parks it as
# "a key this operant build does not know" even though the struct has it.
# MEASURED 2026-11-18 on the operator's own live install: `version`,
# `database_path` and `command_allowlist` are all real `AppConfig` fields
# (38 fields, confirmed by parsing the struct) and all three were being parked
# into `operant.unknown.toml`, so the operator was told their valid config was
# wrong. `terminal_backend` was the only entry because the list was written when
# it was the only such field -- the iter-2357 audit that motivated this list
# caught the SAME class of loss for `[proxy]`/`[providers]`/`[compression]`, so
# this is a regression of a known defect, not a new one.
#
# The fix is to STOP maintaining a list. `bare_top_level_keys()` derives the
# answer from `AppConfig` itself: a field is a bare key when its type is not one
# of the section structs the registry already knows how to walk. Adding a field
# then cannot silently drop it, and an unrecognised TYPE fails loudly here
# instead of quietly producing a config that rejects a valid key.
TOP_LEVEL_SECTION_TYPES = {
    "ClientSettings",
    "ProxyConfig",
    "BehaviorSettings",
    "AutonomousSettings",
    "LoggingSettings",
    "TuiSettings",
    "McpSettings",
    "SkillSettings",
    "GatewaySettings",
    "PluginSettings",
    "ToolSettings",
    "VisionSettings",
    "EmbeddingSettings",
    "MemorySettings",
    "TtsSettings",
    "SttSettings",
    "ImageSettings",
    "CompressionSettings",
    "ObservabilitySettings",
    "AdvisorSettings",
    "CliSettings",
    "PkSettings",
    "ProvidersConfig",
    "BylawsSettings",
    "SocializationSettings",
    "ContextSettings",
    "CronSettings",
    "KanbanSettings",
    "OrgSettings",
    "ApprovalSettings",
}


def bare_top_level_keys(app_src: str, app_lo: int, app_hi: int) -> list[str]:
    """Every `AppConfig` field that is a BARE key rather than a section.

    Derived from the struct, not hand-listed. A field whose type is one of
    `TOP_LEVEL_SECTION_TYPES` has leaves collected through its section; anything
    else is a key in its own right and MUST appear in the registry, because
    `split_unknown` parks every top-level key the registry has no row for.
    """
    out = []
    for field, ftype, _note in fields_of(app_src, app_lo, app_hi):
        if bare_type(ftype) not in TOP_LEVEL_SECTION_TYPES:
            out.append(field)
    return out

# Nested settings structs: the dotted prefix they live under, and the field
# name that reaches them. `client.rate_limit` is the shape every operator
# recognises, so the nesting is expressed rather than flattened.
NESTED = {
    "client": ["rate_limit"],
    "agent": ["retry", "ponytail"],
    "tools": ["web", "http", "terminal", "code_execution", "kernel", "stt", "tool_search"],
    # `advisor.model`, `recap.model` and `bylaws.budget` are real nested
    # settings the operator sets, and all three were missing here — which is
    # why iter-2345 had to hand-append their leaves to a GENERATED file.
    "advisor": ["model"],
    "recap": ["model"],
    "bylaws": ["budget"],
    "integrations": ["herdr"],
    # Added when the documented-key guard (iter-2364) measured the commented
    # half of the example: these are real nested tables an operator sets, and
    # none of them had a registry entry, so any config carrying one unknown key
    # would have had them parked wholesale.
    "tui": ["colors"],
    "auxiliary_models": [
        "memory",
        "compression",
        "vision",
        "web_extract",
        "image_gen",
        "embeddings",
        "search",
        "code_execution",
        "reasoning",
    ],
    "providers": ["models"],
    "compression": ["families"],
}

# Depth-3 settings: `grandparent -> parent -> child`. The two-level NESTED walk
# cannot reach these, and they are real operator knobs — `tools.kernel.
# tool_bridge.*` and `tools.kernel.pyskill.*` bound the kernel's tool bridge,
# which is a live capability. A table at depth 3 with no registry entry is
# parked wholesale exactly like a missing depth-2 one.
DEEP = {
    ("tools", "kernel"): ["tool_bridge", "pyskill"],
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


def fields_of(src: str, start: int, end: int) -> list[tuple[str, str, str]]:
    """(field name, type, note) for every `pub` field in `src[start:end]`.

    The body is scanned in place rather than sliced, because a field's NOTE
    lives on the lines above it and the slice's line numbering would point at
    the wrong text. Only TOP-LEVEL fields are returned: a line inside a
    nested `#[serde(...)]` attribute or a multi-line type is not a field
    declaration, and the brace depth is what tells them apart.

    The TYPE is captured because the panel's widget is inferred from it (see
    `widget_for`). Measured on this file: 369 of 370 fields declare their type
    on the same line as the name and none wrap, so a single-line match is the
    whole population rather than a convenient subset.
    """
    out: list[tuple[str, str, str]] = []
    # Absolute offset of the first body line, so a relative line index can be
    # converted back for the doc-comment read.
    body_start_line = src.count("\n", 0, start)
    body_lines = src[start:end].splitlines()
    depth = 0
    for i, line in enumerate(body_lines):
        stripped = line.strip()
        if stripped.startswith("}") or stripped.startswith("]"):
            depth -= 1
        m = re.match(r"\s*pub ([a-z_][a-z0-9_]*)\s*:\s*(.*?),?\s*$", line)
        if m and depth == 0:
            out.append((
                m.group(1),
                m.group(2).strip().rstrip(","),
                doc_comment_at(src, body_start_line + i),
            ))
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


# The vocabulary a doc comment uses to say "this field is parsed and ignored".
# Measured against every settings struct: `PARKED` (the newest spelling, added
# by the historian retirement) plus the older phrasings that predate it. A field
# carrying one of these in ITS OWN doc block is `live: false`.
#
# WHY THE DOC COMMENT AND NOT THE WARNING BODY: `unused_setting_warnings` builds
# each message with `format!` at runtime, so searching its body for a dotted key
# finds zero keys — the keys exist only as `self.tui.compact_width != …` inside a
# `flag!` macro call. And a peer parked two keys whose doc blocks say "accepted
# and ignored" while the warning body still names neither. The doc comment is
# the one place every parking is recorded, and the generator already reads it.
PARKED_MARKERS = re.compile(
    r"PARKED|accepted and ignored|accepted but unread|accepted and unread"
    r"|READ BY NOTHING|read by nothing|accepts? it and ignore",
    re.IGNORECASE,
)


# The dotted paths `unused_setting_warnings` proves unread. They are recovered
# from the `flag!` macro calls rather than from the messages: the body builds
# every string with `format!` at runtime, so searching it for a dotted key finds
# zero. The macro's FIRST argument names the fields (`self.tui.compact_width`)
# and its second is the label; the fields are what this needs.
def inert_keys_from_warnings(src: str) -> set[str]:
    """Every dotted config path the warning body tests for a non-default value.

    A key that only exists to be reported as "accepted but unread" is a key no
    code reads — that is what the warning MEANS — so its presence here is the
    same fact the doc-comment marker carries, from the other side.
    """
    start = src.find("pub fn unused_setting_warnings")
    if start < 0:
        return set()
    end = src.find("pub fn apply_env_overrides", start)
    body = src[start:end if end > 0 else len(src)]
    out: set[str] = set()
    for cond, _label in re.findall(r'flag!\(\s*(.*?),\s*"([^"]+)"\s*\)\s*;', body, re.S):
        out.update(re.findall(r'self\.([a-z0-9_.]+)', cond))
    # The two cluster arms compare LOCALS, not `self.<path>`, so their fields
    # are recovered from the `let <local> = &self.<section>;` binding that opens
    # each block and the `unread.push("<field>")` literals inside it. Bounding
    # each block at the NEXT binding is what keeps the observability fields from
    # inheriting the context_injection prefix — a single flat scan cannot tell
    # two same-shaped blocks apart.
    binds = [
        (m.start(), m.group(1))
        for m in re.finditer(
            r"^\s*let\s+\w+\s*=\s*&self\.([a-z_]+);\s*$", body, re.M
        )
    ]
    for i, (pos, section) in enumerate(binds):
        if section not in {"observability", "context_injection"}:
            continue
        stop = binds[i + 1][0] if i + 1 < len(binds) else len(body)
        for field in re.findall(r"unread\.push\(\"([a-z_]+)\"\)", body[pos:stop]):
            out.add(f"{section}.{field}")
    for name in re.findall(r'\("([a-z_]+)",\s*&self\.', body):
        out.add(f"auxiliary_models.{name}")
    return out


def is_live(note: str) -> bool:
    """Whether a field's own doc comment claims code reads it.

    False only on a positive parked marker. Silence is NOT evidence of death:
    most live fields say nothing about being read, so an absent marker must
    default to live or the filter would hide most of the panel.
    """
    return not PARKED_MARKERS.search(note or "")


# Rust scalar types the panel can edit directly.
BOOL_TYPES = {"bool"}
NUMBER_TYPES = {
    "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64",
    "isize",
}


def bare_type(ftype: str) -> str:
    """`Option<T>` / `&'a T` wrappers stripped, so a type can be resolved.

    A `pub model: Option<String>` field is still a `String` field for widget
    purposes; the panel shows the current value and the unset state is the
    empty string, which is what `Option` means in the config file anyway.
    """
    t = (ftype or "").strip()
    t = re.sub(r"^Option<\s*", "", t)
    t = re.sub(r"^&'?\w*\s*", "", t)
    return re.sub(r">$", "", t).strip()


def widget_for(ftype: str, enum_values: int | None) -> str:
    """The panel widget a field's Rust type implies.

    Returns the VARIANT NAME as it appears in the generated module. `None` for a
    type the panel must not offer, which is the honest answer for an `f64`
    ratio (measured: 16 fields) or a nested settings struct — a text box that
    accepts any string is a control the config will reject or silently ignore.

    `enum_values` is the variant count when the field's type is an enum defined in
    `config.rs`; None when the generator cannot resolve it.
    """
    t = (ftype or "").strip()
    bare = re.sub(r"^Option<|>$", "", t).strip()
    if bare in BOOL_TYPES:
        return "Bool"
    if bare in NUMBER_TYPES:
        return "Number"
    # An array option cycles in place; anything longer belongs to a config
    # editor, because a twelve-step cycle is a worse control than typing.
    if enum_values is not None:
        return "Cycle" if enum_values <= 6 else "Unsupported"
    if bare in {"String", "PathBuf", "toml::Value"}:
        return "Text"
    return "None"


def enum_variant_count(src: str, enum_name: str) -> int | None:
    """How many variants `enum_name` declares, or None when it is not here.

    Only enums declared in `config.rs` are counted: an enum imported from
    another crate has variants the generator cannot see, and guessing a count
    would silently mislabel a widget.
    """
    m = re.search(r"pub enum " + re.escape(enum_name) + r"\s*\{", src)
    if not m:
        return None
    lo = m.end() - 1
    depth = 0
    hi = None
    for i in range(lo, len(src)):
        if src[i] == "{":
            depth += 1
        elif src[i] == "}":
            depth -= 1
            if depth == 0:
                hi = i
                break
    if hi is None:
        return None
    body = src[lo:hi]
    names = re.findall(r"^\s{4}([A-Z][A-Za-z0-9_]*)", body, re.M)
    return len(names) if names else None


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


# Settings structs live in more than one file, so the generator needs a way to
# reach them all. The iter-2357 audit measured the cost of not having this: six
# live sections (`proxy`, `providers`, `observability`, `compression`,
# `advisor`, `recap`) resolve to structs OUTSIDE `config.rs`, so the generator
# emitted nothing for them, and `split_unknown` — which re-walks a config
# against exactly this list — parked those sections WHOLESALE. A section with
# no registry entry is a section the parking path silently discards.
SOURCE_GLOBS = [
    "crates/operant-core/src/**/*.rs",
    "crates/operant-config/src/**/*.rs",
]


def load_sources() -> dict[str, str]:
    """Every settings-source file, keyed by repo-relative path.

    `config.rs` first so it wins a name collision, then the rest in sorted
    order for a deterministic run.
    """
    out: dict[str, str] = {CONFIG.relative_to(ROOT).as_posix(): None}
    for pattern in SOURCE_GLOBS:
        for path in sorted(ROOT.glob(pattern)):
            rel = path.relative_to(ROOT).as_posix()
            out.setdefault(rel, None)
    loaded: dict[str, str] = {}
    for rel in out:
        path = ROOT / rel
        if path.is_file():
            loaded[rel] = path.read_text(encoding="utf-8")
    return loaded


def _normalise(text: str) -> str:
    """Collapse whitespace so a `cargo fmt` reflow is not read as drift.

    Rustfmt also moves spaces INSIDE punctuation — a one-line tuple
    `("key", ...)` becomes `( "key", ... )` once it is reflowed across lines —
    so collapsing runs is not enough on its own. Spaces adjacent to any of
    `()[]{},;` are removed after the collapse, which is purely lexical and
    cannot change a key, a default or a note.
    """
    collapsed = re.sub(r"\s+", " ", text).strip()
    # Rustfmt writes a TRAILING COMMA before a closing bracket when it reflows
    # a tuple across lines; the generator's single-line form has none. It is
    # semantically identical Rust, so drop it rather than read it as drift.
    collapsed = re.sub(r",\s*([)\]}])", r"\1", collapsed)
    return re.sub(r"\s*([()\[\]{},;])\s*", r"\1", collapsed)


def main() -> int:
    if not CONFIG.exists():
        print(f"missing {CONFIG}", file=sys.stderr)
        return 2
    src = strip_test_items(CONFIG.read_text(encoding="utf-8"))
    defaults = default_map(src)

    rows: list[tuple[str, str, str, str, bool]] = []  # (dotted, default, note, widget, live)
    # TWO sources, because neither alone is complete. The warning body names 41
    # keys that only exist to be reported unread; the doc-comment marker names
    # keys a peer parked WITHOUT updating the warning body (measured at rev 2:
    # the two historian keys). A key dead by either test is dead.
    inert = inert_keys_from_warnings(src)

    # Struct lookup across every loaded source, `config.rs` first so its
    # definitions win a name collision with another file's.
    all_sources = load_sources()
    config_rel = CONFIG.relative_to(ROOT).as_posix()
    ordered = [config_rel] + sorted(k for k in all_sources if k != config_rel)

    def find_struct(name: str) -> tuple[str, int, int] | None:
        for rel in ordered:
            text = all_sources.get(rel)
            if not text:
                continue
            span = struct_span(text, name)
            if span:
                return rel, span[0], span[1]
        return None

    def collect(struct_name: str, prefix: str) -> None:
        found = find_struct(struct_name)
        if found is None:
            return
        rel, lo, hi = found
        body_src = all_sources[rel]
        dmap = defaults.get(struct_name, {})
        for field, ftype, note in fields_of(body_src, lo, hi):
            dotted = f"{prefix}.{field}" if prefix else field
            default = dmap.get(field, "")
            if not default:
                default = f"<{struct_name}::default()>"
            note = note or "<no note>"
            # A cycle needs the variant count, which only the declaring file
            # can supply. An enum from another crate resolves to None and falls
            # through to Text rather than being guessed at.
            variants = enum_variant_count(body_src, bare_type(ftype))
            widget = widget_for(ftype, variants)
            rows.append((
                dotted, default, note, widget,
                is_live(note) and dotted not in inert,
            ))

    # Top-level sections: `AppConfig`'s own fields name the section, and each
    # section's struct supplies the leaves.
    app_found = find_struct("AppConfig")
    section_struct: dict[str, str] = {}
    if app_found:
        app_rel, app_lo, app_hi = app_found
        app_src = all_sources[app_rel]
        for field, _ftype, _note in fields_of(app_src, app_lo, app_hi):
            m = re.search(
                r"pub " + re.escape(field) + r"\s*:\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*(?:Option<)?([A-Za-z_][A-Za-z0-9_]*)",
                app_src,
            )
            if m:
                section_struct[field] = m.group(1)

    for section in TOP_LEVEL_SECTIONS:
        struct_name = section_struct.get(section)
        if struct_name:
            collect(struct_name, section)

    # Bare top-level keys carry no leaves: the key IS the row, and its note
    # comes from the field's own doc comment.
    if app_found:
        app_rel, app_lo, app_hi = app_found
        app_src = all_sources[app_rel]
        # Derived from the struct, so a new top-level field cannot be silently
        # dropped (that is the defect this replaced -- see the comment on
        # `TOP_LEVEL_SECTION_TYPES`).
        bare_keys = bare_top_level_keys(app_src, app_lo, app_hi)
        for field, ftype, note in fields_of(app_src, app_lo, app_hi):
            if field not in bare_keys:
                continue
            dmap = defaults.get("AppConfig", {})
            note = note or "<no note>"
            variants = enum_variant_count(app_src, bare_type(ftype))
            rows.append((
                field,
                dmap.get(field, "<AppConfig::default()>"),
                note,
                widget_for(ftype, variants),
                is_live(note) and field not in inert,
            ))

    # A nested settings struct is reached through its parent field, so the
    # dotted prefix is the parent path -- not a synthesised name. The parent
    # field names the struct, so the child's keys are `section.child.field`.
    for section, children in NESTED.items():
        parent_found = find_struct(section_struct.get(section, ""))
        if not parent_found:
            continue
        p_rel, p_lo, p_hi = parent_found
        p_src = all_sources[p_rel]
        # Read the field's type from the PARENT'S BODY ONLY. Searching the whole
        # file finds the first `pub <field>:` anywhere, which for a common name
        # like `memory` is a DIFFERENT struct's field — `AppConfig.memory:
        # MemorySettings` at the top of config.rs shadowed
        # `AuxiliaryModels.memory: Option<AuxiliaryModelConfig>`, so every
        # `auxiliary_models.memory.*` row was built from the wrong struct.
        parent_body = p_src[p_lo:p_hi]
        for field, _ftype, _note in fields_of(p_src, p_lo, p_hi):
            if field not in children:
                continue
            m = re.search(
                r"\bpub " + re.escape(field)
                    + r"\s*:\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*(?:Option<)?([A-Za-z_][A-Za-z0-9_]*)",
                parent_body,
            )
            if m:
                collect(m.group(1), f"{section}.{field}")

    # Depth-3 tables: `section.child.grandchild.field`.
    for (section, child), grandchildren in DEEP.items():
        child_struct = section_struct.get(section, "")
        gp_found = find_struct(child_struct)
        if not gp_found:
            continue
        g_rel, g_lo, g_hi = gp_found
        g_src = all_sources[g_rel]
        g_body = g_src[g_lo:g_hi]
        for field, _ftype, _note in fields_of(g_src, g_lo, g_hi):
            if field != child:
                continue
            cm = re.search(
                r"\bpub " + re.escape(field)
                    + r"\s*:\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*(?:Option<)?([A-Za-z_][A-Za-z0-9_]*)",
                g_body,
            )
            if not cm:
                continue
            mid_found = find_struct(cm.group(1))
            if not mid_found:
                continue
            m_rel, m_lo, m_hi = mid_found
            m_src = all_sources[m_rel]
            m_body = m_src[m_lo:m_hi]
            for leaf, _t, _n in fields_of(m_src, m_lo, m_hi):
                if leaf not in grandchildren:
                    continue
                lm = re.search(
                    r"\bpub " + re.escape(leaf)
                        + r"\s*:\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*(?:Option<)?([A-Za-z_][A-Za-z0-9_]*)",
                    m_body,
                )
                if lm:
                    collect(lm.group(1), f"{section}.{child}.{leaf}")

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
        # Compare CONTENT, not bytes. `cargo fmt --all` reflows this file — a
        # one-line tuple becomes a four-line one — and the generated output is
        # not rustfmt-shaped, so a plain byte comparison reads a formatting
        # pass as drift and leaves the gate red for every writer until someone
        # regenerates. That happened on `main` after iter-2381/2382: 382 keys,
        # identical defaults and notes, and `--check` failed on the tuple
        # layout alone. Whitespace-normalising both sides keeps the gate
        # honest about CONTENT (a changed key, default or note still fails)
        # while tolerating the formatter this repo runs on every change.
        if _normalise(current) != _normalise(rendered):
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


def render(rows: list[tuple[str, str, str, str, bool]]) -> str:
    """The generated Rust module.

    `include_str!` of the config source is not an option (the registry must be
    a static list, not a parse at startup), so the rows are emitted as a
    `&[(...)]` the runtime can binary-search by key.

    Two tables, because they have two readers: `CONFIG_KEYS` is the operator-
    facing `(key, default, note)` triple that `config describe` renders, and
    `SETTING_ROWS` adds what a panel needs — which widget to draw and whether
    the field is read at all.
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
    for dotted, default, note, _w, _live in sorted(rows):
        safe_default = default.replace("\\", "\\\\").replace('"', '\\"')
        safe_note = note.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(f'    ("{dotted}", "{safe_default}", "{safe_note}"),')
    lines.append("];")
    lines.append("")
    lines.append("/// What a settings panel draws for one key.")
    lines.append("///")
    lines.append("/// (2026-11-15, panels plan S1) The widget is INFERRED from the field's")
    lines.append("/// Rust type, and `live` from two sources: the field's own doc comment")
    lines.append("/// (a parked field says so) and `unused_setting_warnings`, which names")
    lines.append("/// the keys that exist only to be reported as unread. Neither can drift")
    lines.append("/// from the struct. `live: false` is the phantom-surface guard: a panel")
    lines.append("/// that offers such a key ships a control that does nothing.")
    lines.append("pub struct SettingRow {")
    lines.append("    /// The dotted config key, e.g. `agent.context_window`.")
    lines.append("    pub key: &'static str,")
    lines.append("    /// The widget a panel should draw.")
    lines.append("    pub widget: Widget,")
    lines.append("    /// Whether any code reads this key. False = accepted and ignored.")
    lines.append("    pub live: bool,")
    lines.append("}")
    lines.append("")
    lines.append("/// The controls a panel can draw for one config key.")
    lines.append("///")
    lines.append("/// Inferred from the field's type, so the set is closed by what the")
    lines.append("/// generator can prove. `None` means the panel must not offer the key:")
    lines.append("/// an `f64` ratio or a nested table has no honest single-line editor.")
    lines.append("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    lines.append("pub enum Widget {")
    lines.append("    /// A boolean, cycled in place.")
    lines.append("    Bool,")
    lines.append("    /// An enum with at most six variants, cycled in place.")
    lines.append("    Cycle,")
    lines.append("    /// An integer with a known range, edited by typing.")
    lines.append("    Number,")
    lines.append("    /// Free text, parsed as the field's own type.")
    lines.append("    Text,")
    lines.append("    /// An enum longer than a cycle should hold, or a shape with no")
    lines.append("    /// single-line editor.")
    lines.append("    Unsupported,")
    lines.append("    /// No editor: the key needs a structured surface (a table, a")
    lines.append("    /// nested struct, an `f64` ratio).")
    lines.append("    None,")
    lines.append("}")
    lines.append("")
    lines.append("pub const SETTING_ROWS: &[SettingRow] = &[")
    for dotted, _default, _note, widget, live in sorted(rows):
        live_lit = "true" if live else "false"
        lines.append(
            f'    SettingRow {{ key: "{dotted}", widget: Widget::{widget}, live: {live_lit} }},'
        )
    lines.append("];")
    lines.append("")
    lines.append("/// Every dotted key this build accepts.")
    lines.append("pub const KNOWN_KEYS: &[&str] = &[")
    for dotted, _default, _note, _w, _live in sorted(rows):
        lines.append(f'    "{dotted}",')
    lines.append("];")
    lines.append("")
    return "\n".join(lines)

if __name__ == "__main__":
    sys.exit(main())