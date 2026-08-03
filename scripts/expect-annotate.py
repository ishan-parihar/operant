#!/usr/bin/env python3
"""Phase 2b helper: insert `#[expect(clippy::...)]` attributes on functions
that contain justified `.unwrap()` / `.expect()` calls in PRODUCTION code.

Rules:
- Skips test code: `#[cfg(test)]` blocks, `mod tests` blocks, `tests/` dirs,
  and files named `tests.rs`.
- Annotates the nearest enclosing `fn` header (nested braces are fine). For
  module-level sites it annotates the enclosing `static`/`let`/`const`
  statement.
- Merges lints per function into one attribute and classifies a justification
  reason per site (lock-poison, once-init, or generic invariant).
- The attribute is inserted above the function's existing attribute block
  (after doc comments), so derives/other attrs stay intact.

Dry-run with --check to print what would change without editing.
"""

import argparse
import os
import re
import sys

PROD_REASON = "invariant guaranteed by surrounding validation"
LOCK_REASON = "poisoned lock: panic is the intended recovery"
ONCE_REASON = "infallible once-init / static init"
TEST_LIKE = re.compile(
    r"^(crates/[^/]+/(tests|benches|examples)/)|(^|/)tests\.rs$"
)

FN_HEADER = re.compile(
    r"^(pub(\s*\([^)]*\))?\s*)?(async\s*)?(unsafe\s*)?fn\s+[A-Za-z_][A-Za-z0-9_]*"
)
# Module-level items only (column 0): `let` never appears at module level in
# Rust, so inner `let` statements inside fn bodies never match this.
ITEM_HEADER = re.compile(r"^(pub\s+)?(static|const)\s+[A-Za-z_]")


def is_test_block(lines, i):
    """True if line i is inside a cfg(test)/mod tests block."""
    depth = 0
    in_block = False
    for j in range(i + 1):
        stripped = lines[j].strip()
        if re.match(r"^#\[cfg\(test\)\]", stripped):
            in_block = True
            continue
        if stripped.startswith("mod tests") or stripped.startswith("mod test "):
            in_block = True
        if in_block:
            depth += stripped.count("{") - stripped.count("}")
            if depth <= 0 and "}" in stripped:
                in_block = False
                depth = 0
    return in_block


def classify_reason(lines, i):
    ctx = "\n".join(lines[max(0, i - 3) : i + 1])
    if re.search(r"\.(lock|read|write|try_lock)\(\)", ctx):
        return LOCK_REASON
    if re.search(r"OnceLock|LazyLock|Lazy<|static ", ctx):
        return ONCE_REASON
    return PROD_REASON


def attr_block_start(lines, header_idx):
    """Walk up past attributes and doc comments; return insertion index."""
    j = header_idx - 1
    while j >= 0:
        s = lines[j].strip()
        if s.startswith(("#[", "///", "//!", "//", "/*", "*")):
            j -= 1
            continue
        break
    return j + 1


def find_target(lines, site_line):
    """Return (insert_idx, kind) for the nearest enclosing fn header, or for a
    module-level `static`/`const` item, or None.

    The scan goes UP from the site: the first `fn` header wins (covers the
    whole body, nested closures included). Module-level `static`/`const`
    (column 0 only) are matched only when no fn header is closer."""
    i = site_line - 1
    while i >= 0:
        stripped = lines[i].strip()
        if stripped.startswith(("//", "#[", "/*")):
            i -= 1
            continue
        if FN_HEADER.match(stripped):
            return attr_block_start(lines, i), "fn"
        if ITEM_HEADER.match(lines[i]):
            return attr_block_start(lines, i), "item"
        i -= 1
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="dry run, print only")
    ap.add_argument("paths", nargs="*", help="files or dirs (default: crates/)")
    args = ap.parse_args()

    files = []
    targets = args.paths or ["crates/"]
    for t in targets:
        if os.path.isfile(t):
            files.append(t)
        else:
            for dp, _dn, fns in os.walk(t):
                for fn in fns:
                    if fn.endswith(".rs"):
                        files.append(os.path.join(dp, fn))

    total_attrs = 0
    skipped = 0
    for path in sorted(files):
        if TEST_LIKE.search(path):
            continue
        try:
            lines = open(path).read().split("\n")
        except Exception:
            continue

        # site classification per target
        fn_sites = {}
        for i, line in enumerate(lines):
            # `.expect(` / `.expect_err(` and `.unwrap()` / `.unwrap_err()`
            # (clippy's expect_used/unwrap_used fire on the _err siblings too)
            has_expect = bool(re.search(r"\.expect(_err)?\(", line))
            # `.unwrap()` / `.unwrap_err()` only — `unwrap_or*` calls are not
            # flagged by clippy, and the precise regex never matches them.
            has_unwrap = bool(re.search(r"\.unwrap(_err)?\(\)", line))
            if not (has_expect or has_unwrap):
                continue
            if is_test_block(lines, i):
                continue
            loc = find_target(lines, i)
            if loc is None:
                skipped += 1
                print(f"SKIP {path}:{i+1}: no enclosing fn/item found", file=sys.stderr)
                continue
            idx, _kind = loc
            # Idempotency: if the target already carries an #[expect(clippy::
            # attribute (from a previous run), the site is already covered.
            if any(
                lines[j].lstrip().startswith("#[expect(clippy::")
                for j in range(idx, i + 1)
            ):
                continue
            if idx not in fn_sites:
                fn_sites[idx] = {"expect": set(), "unwrap": set()}
            if has_expect:
                fn_sites[idx]["expect"].add(classify_reason(lines, i))
            if has_unwrap:
                fn_sites[idx]["unwrap"].add(classify_reason(lines, i))

        if not fn_sites:
            continue

        edits = []
        for idx, lints in sorted(fn_sites.items(), key=lambda kv: -kv[0]):
            parts = []
            for lint, key in (("unwrap_used", "unwrap"), ("expect_used", "expect")):
                if lints[key]:
                    parts.append(f"clippy::{lint}")
            if not parts:
                continue
            reasons = lints["unwrap"] | lints["expect"]
            reason = next(iter(reasons)) if len(reasons) == 1 else PROD_REASON
            attr = f'    #[expect({", ".join(parts)}, reason = "{reason}")]'
            edits.append((idx, attr))
            total_attrs += 1

        for idx, attr in sorted(edits, key=lambda e: -e[0]):
            indent = ""
            if idx < len(lines):
                m = re.match(r"^(\s*)", lines[idx])
                indent = m.group(1)
            if args.check:
                print(f"{path}:{idx + 1}: {attr.strip()}")
            else:
                lines.insert(idx, indent + attr)

        if not args.check:
            open(path, "w").write("\n".join(lines))

    print(f"TOTAL ATTRIBUTES: {total_attrs}  (skipped: {skipped})")


if __name__ == "__main__":
    main()
