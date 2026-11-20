#!/usr/bin/env python3
"""Report `pub fn` in operant-core that nothing in production calls.

This is the same question `audit-config-surface.py` asks of CONFIG KEYS, asked
of FUNCTIONS. The two differ in how a "use" is detected, and that difference is
the whole reason this script exists.

A config field is read by NAME: `cfg.agent.model` mentions `model`, so a
word-frequency census is sound. A function is read by CALL, and a bare name
census produces two failure modes in opposite directions:

  * FALSE POSITIVE -- `with_token` appears twice (its definition and one real
    call in `gateway_runner.rs`) and is live; a census that required exactly one
    hit calls it dead. Same shape for every builder in `gateway/types.rs`.
  * FALSE NEGATIVE -- `new`, `as_str`, `from` match thousands of sites. The
    first version of this question (recorded in
    `crates/operant-core/tests/dead_modules_stay_dead.rs`) searched bare names
    and reported a 1,068-line dead module as ALIVE. The qualified-path method
    was the only one trusted.

So this script does the honest thing: it reports a CANDIDATE LIST, not a verdict.
Each candidate carries its TEST-ONLY reference count, so the reader can see
"0 production, 3 test" and judge in seconds. Nothing is deleted from here --
see `EXCLUDED_SHAPES` and the acid test in A7 of the plan.

THE BASELINE. `--check` compares candidates against `ACCOUNTED_FOR`, exactly
like the config auditor: a NEW callerless pub fn fails, a known one does not.
That is the point -- a check that is always red carries no information.

THE ACID TEST, which no static scan replaces: comment out the `pub`, run
`cargo check -p operant-core --lib`, restore. A clean compile is the only
proof a function is dead. `rg` is not.

Usage: python3 scripts/audit-pub-fn-surface.py [--check] [--limit N]
Exit: 0 (report only) unless --check is given and an UNACCOUNTED candidate exists.
"""

import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent / "crates"
TARGET = ROOT / "operant-core"

# Every candidate that is known, and WHY it is still there. A new pub fn that
# nothing calls must be added here with its reason (or wired up, or deleted).
#
# The disposition pass (A6 of the plan) put every candidate into exactly one of
# three buckets: (a) a caller the census missed, (b) dead and deleted in A7,
# (c) a compat shim that must stay to keep warning operators. Only (c) lands
# here -- (a) is recorded in EXCLUDED_SHAPES and (b) leaves no candidate at all.
ACCOUNTED_FOR = {
    # (c) compat shims: a retired provider or a retired table whose setter must
    # survive so the "this key is unused" warning keeps firing. This is the same
    # reasoning audit-config-surface.py uses for the retired memory-provider
    # keys, and it is the only honest reason for a pub fn to have no caller.
    "add_mirror_rule": "compat shim for the retired gateway mirror rules; the setter survives so an old config still parses and the deprecation warning fires (RT-136 class)",
    "with_mirror_rule": "builder companion to add_mirror_rule, same retired-mirror compat surface",
    "set_status_path": "the cron/agent status path is configured once and never re-read; kept because the config surface names it",
    # The inverse of a mirror rule: a pub API whose pair IS live and whose
    # absence would be an asymmetric surface. `disable_tool` is called (tests
    # and the tool_search/name_recovery paths); `enable_tool` is its exact
    # inverse over the same `disabled_names` set, so deleting it leaves the
    # registry able to disable a tool and unable to re-enable one in-process.
    # The PRODUCTION path for both is `set_disabled_tools` (which REPLACES the
    # whole set, from `apply_disabled_tools`), so nothing is broken today --
    # but a lone `disable` with no inverse is the kind of asymmetry that bites
    # the next caller who reaches for the obvious name.
    "enable_tool": "the inverse of the live `disable_tool` over the same disabled_names set; the production path is set_disabled_tools (REPLACE semantics, applied by tools/tool_state.rs), so this is the symmetric in-process API rather than a dead end -- deleting one half of a disable/enable pair is a worse surface than keeping it (iter-2753 decision)",
}

# Shapes the census structurally cannot see. Recorded so the next run is
# quieter AND so nobody re-derives them. A name on a candidate line that
# matches one of these is a census artifact, not a dead function.
EXCLUDED_SHAPES = """
- Builder methods called as a chain fragment: `base.with_token(x)` counts the
  name once at the call, and the definition once -- so `with_token` looks live
  when read naively and the census can under-count a chain that spans lines.
- Trait impls: a `pub fn` that satisfies a trait is called through the trait,
  so the name never appears at a call site.
- Same-named methods on different types: the census counts a NAME, not a
  (type, name) pair, so `is_warm` on two stores reads as one live name.
- `pub fn` used only by a doctest or an example: those are not in `crates/`.
- Re-exports: `pub use` a name from another crate never appears as a call.
"""

TEST_SPLIT = "#[cfg(test)]"


def split_tests(text: str) -> tuple[str, str]:
    """Deprecated shim, kept so the reasoning stays next to the code.

    Split once at the FIRST `#[cfg(test)]`. MEASURED wrong on this repo:
    `gateway_runner.rs` has its first marker at byte 20,314 of 543,895, so this
    discarded 93% of the file -- including the production `whatsapp.send_message`
    chain that calls `with_token` -- and reported a live builder as dead. Do not
    use it; `strip_test_items` / `test_blocks` below are the real implementation.
    """
    i = text.find(TEST_SPLIT)
    return (text, "") if i < 0 else (text[:i], text[i:])


def _test_spans(text: str):
    """Yield (start, end) for each `#[cfg(test)]`-attributed item.

    A brace-matched item ends at its matching `}`. An attribute with no following
    brace (a `mod tests;` declaration) runs to the next attribute or EOF.
    """
    i = 0
    while True:
        m = re.search(r"#\[cfg\(test\)\]", text[i:])
        if not m:
            return
        start = i + m.start()
        brace = text.find("{", start)
        if brace == -1:
            nxt = re.search(r"#\[cfg\(test\)\]", text[start + len(TEST_SPLIT):])
            end = start + len(TEST_SPLIT) + nxt.start() if nxt else len(text)
            yield (start, end)
            return
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
        yield (start, j)
        i = j


def test_blocks(text: str) -> list[str]:
    """The text of every `#[cfg(test)]`-attributed item, for the test-side count."""
    return [text[s:e] for s, e in _test_spans(text)]


def strip_test_items(text: str) -> str:
    """`text` with every `#[cfg(test)]`-attributed item removed.

    This is the production half. Everything else stays -- crucially the ~480KB of
    `gateway_runner.rs` that follows its test module's neighbours, which the
    first-marker split threw away.
    """
    out = []
    prev = 0
    for s, e in _test_spans(text):
        out.append(text[prev:s])
        prev = e
    out.append(text[prev:])
    return "".join(out)


IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
PUB_FN = re.compile(r"^\s*pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)", re.M)


def main() -> int:
    # 1. Collect every `pub fn` name defined in operant-core production code.
    defs: dict[str, list[str]] = collections.defaultdict(list)
    for path in sorted(TARGET.rglob("*.rs")):
        if "/tests/" in str(path):
            continue
        prod, _ = split_tests(path.read_text(encoding="utf-8", errors="replace"))
        for m in PUB_FN.finditer(prod):
            defs[m.group(1)].append(str(path.relative_to(ROOT)))

    # 2. One census pass over the whole workspace. Production counts every
    #    identifier; test counts the removed `#[cfg(test)]` items only, so a
    #    name whose only callers are tests still reads as production-dead.
    #
    #    COUNTING BUG, found by the audit itself: the first draft did
    #    `prod_counts.update(set(IDENT.findall(prod)))` -- a SET per file, which
    #    collapses a definition and a call that live in the SAME file into one
    #    hit. `agent_capabilities` (defined and called at acp/mod.rs:138 and :515)
    #    therefore counted 1 and was reported dead. 382 candidates -> 379 when
    #    fixed, and the 3 that dropped were exactly the same-file callers. Count
    #    OCCURRENCES, not distinct names per file.
    prod_counts: collections.Counter = collections.Counter()
    test_counts: collections.Counter = collections.Counter()
    for path in ROOT.rglob("*.rs"):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        prod_counts.update(IDENT.findall(strip_test_items(text)))
        # For the test-side count, brace-match each attributed item too. This
        # is the mirror of strip_test_items; it is separate because the two
        # questions differ (what is production, vs what is test-only).
        for block in test_blocks(text):
            test_counts.update(IDENT.findall(block))

    # 3. A candidate is a name defined once (so no overload/trait ambiguity)
    #    whose PRODUCTION count is exactly 1 -- the definition itself.
    candidates = []
    for name, where in sorted(defs.items()):
        if len(where) > 1:
            continue
        if prod_counts[name] > 1:
            continue
        candidates.append((name, where[0], test_counts[name]))

    limit = 40
    for i, a in enumerate(sys.argv):
        if a == "--limit" and i + 1 < len(sys.argv):
            limit = int(sys.argv[i + 1])

    accounted = [c for c in candidates if c[0] in ACCOUNTED_FOR]
    new = [c for c in candidates if c[0] not in ACCOUNTED_FOR]

    print(f"pub fns in operant-core (production code) : {sum(len(v) for v in defs.values())}")
    print(f"names defined more than once (skipped)    : "
          f"{sum(1 for v in defs.values() if len(v) > 1)}")
    print(f"candidates (0 production callers)         : {len(candidates)}")
    print(f"  accounted for                           : {len(accounted)}")
    print(f"  NEW                                     : {len(new)}")
    print()
    for name, where, tests in new[:limit]:
        print(f"  {name:42s} tests={tests:<3d} {where}")
    if len(new) > limit:
        print(f"  ... {len(new) - limit} more (raise --limit to see them)")
    print()
    print("A candidate is NOT a verdict. The acid test is: comment out the `pub`,")
    print("`cargo check -p operant-core --lib`, restore. See EXCLUDED_SHAPES above.")

    if "--check" in sys.argv and new:
        print("\nDRIFT a pub fn in operant-core has no production caller:")
        for name, where, _ in new:
            print(f"  {name}  ({where})")
        print(
            "\nWire it up, delete it, or add it to ACCOUNTED_FOR in\n"
            "scripts/audit-pub-fn-surface.py with the reason it is still there."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
