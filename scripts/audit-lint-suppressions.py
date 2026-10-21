#!/usr/bin/env python3
"""Report lint suppressions (`#[allow(...)]`, `#[expect(...)]`) in production code.

rust-best-practices ch.2 ("Fix warnings, don't silence them") is explicit:
prefer `#[expect]` over `#[allow]`, because an `allow` never expires while an
`expect` reports itself as unfulfilled the moment the lint stops firing.

This script makes that measurable instead of a prose claim, because prose
numbers rot: `docs/RUST_BEST_PRACTICES_PLAN.md:25` still reports 242 suppressed
warnings from a 2026-08-02 measurement. The live figure after the config
retirement, the provider-surface retirement and the three-crate purge is 33.

KEYING (why not line numbers). A baseline keyed on `path:line` breaks the first
time anything above it moves, and a gate that breaks when the code it guards is
edited is a gate nobody keeps. Each suppression is therefore identified by
`<relative-path>::<lint>::<item>` — the attribute's lint plus the identifier of
the item it decorates (the first non-comment token on a following line). That
is stable across reordering and reindentation, and ambiguous only when two
suppressed items in one file share a name, which is itself worth reporting.

WHAT IT CHECKS:

  * `--check` exits non-zero when a suppression is NOT in the baseline. That is
    the gate: an unexplained suppression is a decision nobody made. (An
    always-red check carries no information — the exact defect
    `audit-config-surface.py`'s `--check` had before iter-1153.)
  * A baseline entry with no reason is reported. A suppression with no reason
    is the kind that silently outlives its subject.
  * `#[allow]` sites where `#[expect]` would work are listed as CONVERSION
    CANDIDATES: outstanding work, not a failure.

Usage: python3 scripts/audit-lint-suppressions.py [--check]
Exit: 0 (report only) unless --check is given and an unexplained or
unreasoned suppression exists.
"""

import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent / "crates"

TRACKED = {
    "dead_code",
    "unused_variables",
    "unused_mut",
    "unused_assignments",
}

RE_ATTR = re.compile(r"#\[\s*(?P<kind>allow|expect)\s*\(\s*(?P<lints>[^)]*?)\s*\)")

# ---------------------------------------------------------------------------
# THE BASELINE. Every suppression that is known, with the reason it is still
# there. Key: "<relative-path>::<lint>::<item>". Value: (expectable, reason),
# where `expectable` is "yes" (an `allow` that could be an `expect,` i.e.
# outstanding work) or "no" (`expect` is the WRONG choice here — a macro
# expansion, a cfg-conditional body, or a feature-conflicting type).
# ---------------------------------------------------------------------------
BASELINE = {
    # ---- #[expect] sites that are already correct -------------------------
    # Listed so a regression to `#[allow]` is visible as an UNEXPLAINED entry
    # rather than silently matching an old allow-shaped baseline.
    "operant-core/src/schema.rs::dead_code::TestParams": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"test-only schema struct\")]`",
    ),
    "operant-core/src/schema.rs::dead_code::CoercionParams": (
        "n/a",
        "already an `#[expect(dead_code, reason = ...)]`",
    ),
    "operant-core/src/schema.rs::dead_code::NumberParams": (
        "n/a",
        "already an `#[expect(dead_code, reason = ...)]`",
    ),
    "operant-core/src/tools.rs::dead_code::TestArgs": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"test-only argument struct\")]`",
    ),
    "operant-core/src/mcp_oauth.rs::dead_code::is_interactive": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"reserved for TTY-only OAuth flows\")]`",
    ),
    "operant-core/src/agent/clients/prompt_caching.rs::dead_code::as_str": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"reserved for TTL introspection/debugging\")]`",
    ),
    "operant-core/tests/seat_policy_run_path.rs::dead_code::prompt": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"schema-only: the stub never reads the prompt\")]`",
    ),
    # ---- conversions landed in iter-1163 -----------------------------------
    "operant-core/src/tools/skills_tool.rs::dead_code::USAGE_TELEMETRY_LOCK": (
        "n/a",
        "converted to `#[expect]` in iter-1163: this doc comment claimed an "
        "`#[expect]` while the attribute was an `#[allow]`, so the suppression "
        "never expired when the legacy `.usage.json` writer it covers came back",
    ),
    "operant-cli/src/config.rs::dead_code::Parse": (
        "n/a",
        "converted to `#[expect]` in iter-1163 — a public error variant with a "
        "Display arm and no constructor",
    ),
    "operant-plugins/src/host.rs::dead_code::verification": (
        "n/a",
        "converted to `#[expect]` in iter-1163 — a SECURITY VERDICT written and "
        "never read; the escape hatch expires when it is finally surfaced",
    ),
    "operant-providers/src/bedrock.rs::dead_code::role": (
        "yes",
        "the response field is an #[expect(dead_code)] carrying a role for "
        "payload parity with the wire shape",
    ),
    # ---- #[allow(dead_code)] that must stay ---------------------------------
    "operant-providers/src/gemini.rs::dead_code::InternalGenerateContentResponse": (
        "no",
        "feature-conflicting: used by the default-feature response path and "
        "unused under `--all-features`, so `#[expect]` is unfulfilled under "
        "one of the two feature sets. Its own comment says so and it is true.",
    ),
    "operant-core/src/agent/background_review.rs::dead_code::summarize_review_actions": (
        "no",
        "`#[cfg_attr(not(test), ...)]`: exercised only by unit tests today, so "
        "the lint fires only OUTSIDE `cfg(test)` and `expect` would be "
        "unfulfilled in exactly the build that reads it",
    ),
    "operant-cli/src/tui/operant_app/todo.rs::dead_code::as_str": (
        "no",
        "`semantic_state!` generates ONE impl for eleven enums whose consumers "
        "differ — `ConfidenceState` aggregates numerically through "
        "`legacy_score`. Verified by experiment (iter-1163): `#[expect]` "
        "produces two 'lint expectation is unfulfilled' warnings, because a "
        "macro expansion cannot express per-expansion expectations.",
    ),
    "operant-cli/src/tui/operant_app/todo.rs::dead_code::legacy_score": (
        "no",
        "the `legacy_score` arm of the same `semantic_state!` expansion",
    ),
    # ---- #[allow] sites that could be #[expect] -----------------------------
    "operant-core/src/tools/kernel/sidecar.rs::dead_code::bridge_rx": (
        "yes",
        "`SidecarHandle::bridge_rx` — bridged tool-call requests awaiting host "
        "execution; the drain loop is the phase 2.5 wiring. A concrete field, "
        "so `#[expect]` is available.",
    ),
    "operant-providers/src/bedrock.rs::dead_code::Other": (
        "yes",
        "an enum variant carrying a parsed payload for a response shape only "
        "some calls produce",
    ),
    "operant-harness/src/row.rs::dead_code::config": (
        "yes",
        "Phase-3 stub carriage — the host is meant to read `config`",
    ),
    "operant-harness/src/row.rs::dead_code::config (2)": (
        "yes",
        "the SECOND `config` field in row.rs (line 75) — a Phase-3 stub field "
        "the host is meant to read. Two sites in one file share the name; the "
        "item-only key cannot tell them apart, which is the ambiguity the "
        "script's docstring says is itself worth reporting.",
    ),
    "operant-harness/src/row.rs::dead_code::FakeFilePromptSection": (
        "yes",
        "Phase-3 stub struct (third site in row.rs)",
    ),
    "operant-harness/src/pool_provider.rs::dead_code::PoolBundlePayload": (
        "yes",
        "Phase-3 stub payload struct",
    ),
    "operant-core/examples/simple_agent.rs::dead_code::EchoArgs": (
        "yes",
        "example code — an argument struct set up for a later step of the "
        "walkthrough",
    ),
    "operant-core/examples/simple_agent.rs::dead_code::CalcArgs": (
        "yes",
        "example code (second struct in the same file)",
    ),
    "operant-harness/tests/host_boot.rs::dead_code::_ensure_noop_linked": (
        "yes",
        "test helper kept for a later test (leading underscore already says "
        "intentional)",
    ),
    "operant-harness/tests/builder_factories.rs::dead_code::_host_smoke": (
        "yes",
        "test helper kept for a later test",
    ),
    # ---- #[allow(unused_*)] ------------------------------------------------
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::centered": (
        "yes",
        "consumed by the gated inline-image block until W6 (3 sites: `centered`, "
        "`diagram_mode`, `pinned_diagrams`)",
    ),
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::diagram_mode": (
        "yes",
        "the same W6 inline-image block's second local",
    ),
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::pinned_diagrams": (
        "yes",
        "the same W6 inline-image block's third local",
    ),
    "operant-config/src/policy.rs::unused_mut::mut": (
        "yes",
        "the command vec is mutated only on some target OSes",
    ),
    "operant-core/src/agent/run.rs::unused_assignments::mut": (
        "yes",
        "`tool_names` is collected before the match and read after it, which "
        "the compiler cannot see across the match",
    ),
    "operant-core/src/mcp_oauth.rs::unused_variables::handle_401": (
        "yes",
        "the 401-recovery path keeps the parameter for the poisoning handler's "
        "signature",
    ),
}

# Files whose text mention of the attribute is a note about its REMOVAL, not
# an attribute. `tui/banner.rs` and `tui/rustle.rs` each carry a head comment
# saying the module-level `#![allow(dead_code)]` was dropped in iter-392.
MENTION_ONLY = {
    "crates/operant-cli/src/tui/banner.rs",
    "crates/operant-cli/src/tui/rustle.rs",
    # `session_sqlite.rs` documents why its `db_path` field was deleted by
    # naming the attribute that used to cover it (iter-1163).
    "crates/operant-infra/src/session_sqlite.rs",
    # `redraw_reason.rs`'s test asserts the absence of an allow.
    "crates/operant-cli/src/tui/app/redraw_reason.rs",
    # `operant_model/adapter.rs` and `tui/copy_targets.rs` each carry MODULE
    # docs that spell the attribute out while explaining a decision about it;
    # the text is prose, not an attribute (adapter.rs:44 says its variants are
    # never constructed, copy_targets.rs:26 points at an expect on the module
    # declaration in tui/mod.rs).
    "crates/operant-cli/src/tui/operant_model/adapter.rs",
    "crates/operant-cli/src/tui/copy_targets.rs",
}


# Keywords that may PRECEDE the identifier the attribute decorates.
# They are skipped, not matched, so `pub config:` yields `config` and an
# inline `#[allow(dead_code)] serde_json::Value` yields the line fallback.
LEADING_KEYWORDS = re.compile(
    r"^(?:pub\s+(?:\([^)]*\)\s*)?)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\S+\s+)?"
    r"(?:const\s+)?(?:fn|struct|enum|static|let|type|impl|mod|trait)\s+"
)
# `pub` is optional on every item, so it has to be skipped here too — the
# old version matched the KEYWORD as the item name and keyed `pub config:` as
# `pub`, which is how two suppressed `config` fields in row.rs collided.
FALLBACK_IDENT = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)")
FIELD = re.compile(r"^(?:pub\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:")


def _relkey(path: str) -> str:
    """The canonical form of a suppression site: no leading `crates/`.

    The baseline is written crate-relative (`operant-core/src/...`) because that
    reads better, while `scan()` produces repo-relative paths
    (`crates/operant-core/src/...`) because it walks `crates/`. Normalising in
    ONE place — rather than remembering which spelling a given key uses — is
    what keeps the two in step; an earlier version had the prefix on neither
    side and the gate reported every entry as unexplained.
    """
    return path[len("crates/") :] if path.startswith("crates/") else path


BASELINE = {_relkey(k): v for k, v in BASELINE.items()}


def item_after(lines: list[str], start: int) -> str | None:
    """The identifier of the item the attribute decorates.

    Walks forward from the attribute, skipping blank lines, comments and other
    attributes, and returns the first item name it can read. Handles:

      * declarations  — `pub fn foo`, `struct Bar`, `static BAZ: T = ...`
      * fields        — `pub config: serde_json::Value,`
      * enum variants — `Other(#[allow(...)] serde_json::Value)` (the
        attribute is INLINE, so the identifier is on the SAME line and to the
        LEFT — handled by `item_on_line`)

    Returns None when no name can be read, and the caller keys the site by its
    line number instead — which is safe as long as it is rare and reported.
    """
    for raw in lines[start : start + 4]:
        text = raw.strip()
        if not text or text.startswith("//") or text.startswith("#["):
            continue
        # A declaration: `pub struct Foo {`, `fn bar(`, `static BAZ:`.
        after = LEADING_KEYWORDS.sub("", text)
        if after is not text:
            m = FALLBACK_IDENT.match(after)
            if m:
                return m.group(1)
        # A field or a bare item: `pub config: T,` / `role: String,`.
        m = FIELD.match(text)
        if m:
            return m.group(1)
        # `let mut cmds = ...` / `prompt: String,` inside a struct body.
        m = re.match(r"(?:let\s+)?(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)", text)
        if m:
            return m.group(1)
    return None


def item_on_line(line: str) -> str | None:
    """The identifier an INLINE attribute decorates on its own line.

    `Other(#[allow(dead_code)] serde_json::Value)` — the variant name is to the
    left of the attribute, so read back from the `#\[`.
    """
    idx = line.find("#[")
    if idx <= 0:
        return None
    left = line[:idx].strip()
    m = re.search(r"([A-Za-z_][A-Za-z0-9_]*)\s*\($", left)
    if m:
        return m.group(1)
    m = re.search(r"([A-Za-z_][A-Za-z0-9_]*)", left)
    return m.group(1) if m else None



def scan() -> list[tuple[str, str, str, int]]:
    """Yield (relpath, lint, item, lineno) for every tracked suppression."""
    out = []
    for path in sorted(ROOT.rglob("*.rs")):
        rel = str(path.relative_to(ROOT.parent))
        if rel in MENTION_ONLY:
            continue
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError):
            continue
        for i, line in enumerate(lines):
            m = RE_ATTR.search(line)
            if not m:
                continue
            for lint in m.group("lints").split(","):
                if lint.strip() not in TRACKED:
                    continue
                # An INLINE attribute (`Other(#[allow(dead_code)] Value)`) names
                # its item on the same line, to the LEFT. A LEADING attribute
                # names the item on a following line. `m.start() > 0` is NOT a
                # test for inline — an indented `    #[expect(...)]` also
                # starts past 0, and every indented attribute in this repo
                # would be keyed as inline and read the wrong identifier. The
                # test is: is there NON-WHITESPACE before the `#[`?
                before = line[: m.start()]
                inline = bool(before.strip()) and not before.strip().startswith("//")
                item = item_on_line(line) if inline else (item_after(lines, i + 1) or f"line{i + 1}")
                out.append((rel, lint.strip(), item, i + 1))
    return out


def main() -> int:
    found = scan()
    print(f"lint suppressions: {len(found)} across {len({s[0] for s in found})} files")
    print(f"baseline entries:  {len(BASELINE)}\n")

    unbaselined, no_reason, conversion, already_expect = [], [], [], []

    for rel, lint, item, line in found:
        # The key must be unique; a collision means two suppressed items in one
        # file share a name and the baseline cannot tell them apart (which is
        # exactly the row.rs `config` case below).
        key = f"{_relkey(rel)}::{lint}::{item}"
        keys = [key] if key in BASELINE else [
            k for k in BASELINE if k.split("::", 1)[1] == f"{lint}::{item}" and k.startswith(f"{rel}::")
        ]
        if not keys:
            unbaselined.append(f"{rel}:{line}: {lint} on `{item}`")
            continue
        expectable, reason = BASELINE[keys[0]]
        if not reason:
            no_reason.append(f"{rel}:{line}: {lint} on `{item}`")
        if expectable == "n/a":
            already_expect.append(f"{rel}:{line}: {lint} on `{item}`")
        elif expectable == "yes":
            conversion.append(f"{rel}:{line}: {lint} on `{item}`")

    if no_reason:
        print("BASELINE ENTRIES WITH NO REASON (add one — a suppression with no")
        print("reason is the kind that silently outlives its subject):")
        for k in no_reason:
            print(f"  ? {k}")
        print()

    if conversion:
        print("CONVERSION CANDIDATES — an `allow` where `#[expect]` would work")
        print("(the lint fires on every build, so the escape hatch would expire")
        print("when it stops being needed). Outstanding work, not a failure:")
        for k in conversion:
            print(f"  ~ {k}")
        print()

    if unbaselined:
        print("UNEXPLAINED SUPPRESSIONS (not in the baseline — add an entry with")
        print("the reason, or remove the attribute):")
        for k in unbaselined:
            print(f"  ! {k}")
        print()

    print(f"correct `#[expect]` sites: {len(already_expect)}")
    print(f"conversion candidates:    {len(conversion)}")

    if "--check" in sys.argv[1:]:
        if unbaselined or no_reason:
            return 1
        print("audit-lint-suppressions: pass")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
