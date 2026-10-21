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

THE BASELINE. `--check` compares the candidates against `ACCOUNTED_FOR` — the
ones that are KNOWN and each one that is accounted for. It exits non-zero
only when a candidate is NOT in that baseline, i.e. when a setting has become
unread WITHOUT anyone deciding what to do about it. That is the whole point
of a gate: a check that is always red carries no information (this script's
`--check` was exactly that for the 24 candidates RT-136 accounted for — and
three more appeared when the config retirement of iter-1140 deleted the
schema files that had been their last mention).

Usage: python3 scripts/audit-config-surface.py [--check]
Exit: 0 (report only) unless --check is given and an UNACCOUNTED candidate
exists.
"""

import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent / "crates"
CONFIG = ROOT / "operant-core" / "src" / "config.rs"

# Every candidate that is known, and WHY it is still there. A new key must
# be added here with its reason (or wired up) — the check is what makes the
# list honest instead of a growing wall of yellow.
ACCOUNTED_FOR = {
    "AppConfig.pk": "the [pk] table line (RT-136): the live kernel surface is [tools.kernel]",
    "BylawsSettings.grant_ttl_days": "retired with the grant ledger (RT-136): warns when set",
    "SocializationSettings.turn_budget": "the phase-1 coordinator never landed (RT-136): warns when set",
    "BehaviorSettings.max_consecutive_tool_only": "the guard it names does not exist (RT-136): warns when set",
    "TuiSettings.show_iterations": "the render layer consults none of the six [tui] keys (RT-136): warns",
    "TuiSettings.landing_title": "the render layer consults none of the six [tui] keys (RT-136): warns",
    "TuiSettings.prompt_placeholder": "the render layer consults none of the six [tui] keys (RT-136): warns",
    "TuiSettings.refresh_rate_ms": "the render layer consults none of the six [tui] keys (RT-136): warns",
    "TuiSettings.compact_width": "the render layer consults none of the six [tui] keys (RT-136): warns",
    "TuiSettings.medium_width": "the render layer consults none of the six [tui] keys (RT-136): warns",
    "SkillsSettings.template_name": "the skill creator does not use it (RT-136): warns",
    "SkillsSettings.template_description": "the skill creator does not use it (RT-136): warns",
    "PkSettings.sidecar_script": "the whole [pk] table is vestigial (RT-136): warns",
    "PkSettings.ndjson_socket": "the whole [pk] table is vestigial (RT-136): warns",
    "PkSettings.exec_timeout_secs": "the whole [pk] table is vestigial (RT-136): warns",
    "ToolSettings.event_channel_size": "the agent event channels use fixed capacities (RT-136): warns",
    "ToolSettings.browser_binary_path": "the browser resolves its binary inside sourcehound (RT-136): warns",
    "ToolSearchSettings.search_default_limit": "hardcoded 10/25 at the tool (RT-136): warns",
    "ToolSearchSettings.max_search_limit": "hardcoded 10/25 at the tool (RT-136): warns",
    "MemorySettings.agentmemory_url": "retired with the agentmemory era (RT-136): warns",
    "MemorySettings.agentmemory_secret": "retired with the agentmemory era (RT-136): warns",
    "MemorySettings.audit_enabled": "retired with the memory-factory era: the legacy-key warning names it",
    "MemorySettings.response_cache_enabled": "retired with the memory-factory era: the legacy-key warning names it",
    "WebToolSettings.search_url": "plan 11C1: web search is sourcehound-only, warns when set",
    "WebToolSettings.tavily_api_key": "plan 11C1: web search is sourcehound-only, warns when set",
    "WebToolSettings.exa_api_key": "plan 11C1: web search is sourcehound-only, warns when set",
    "WebToolSettings.searxng_base_url": "plan 11C1: web search is sourcehound-only, warns when set",
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
    new = []
    for name, fields in structs.items():
        dead = [f for f in fields if uses[f] == 0]
        if not dead:
            continue
        total += len(dead)
        print(f"{name}: {len(dead)}/{len(fields)}")
        for field in dead:
            key = f"{name}.{field}"
            reason = ACCOUNTED_FOR.get(key)
            if reason is None:
                new.append(key)
                print(f"    {field}  <-- NOT ACCOUNTED FOR")
            else:
                print(f"    {field}  ({reason})")
    print(f"\n{total} candidates across {len(structs)} structs")
    print(f"{total - len(new)} accounted for, {len(new)} new")
    if "--check" in sys.argv and new:
        print("\nDRIFT a config setting is accepted, defaulted and read by nobody:")
        for key in new:
            print(f"  {key}")
        print(
            "\nWire it up, or accept it with a warning and add it to ACCOUNTED_FOR\n"
            "in scripts/audit-config-surface.py with the reason it is still there."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
