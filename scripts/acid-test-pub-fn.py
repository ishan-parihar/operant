#!/usr/bin/env python3
"""The acid test from scripts/audit-pub-fn-surface.py.

Drop the `pub` from one function, compile `operant-core --lib`, restore. A
clean compile is the ONLY proof a pub fn is dead -- `rg` is not.

Restoration is verified per candidate: the original bytes are held in memory,
rewritten in the `finally`, and the file's hash is compared against the saved
hash before a verdict is reported. A shell version of this lost two mutations
when a `trap`-based restore did not fire under a backgrounded shell; holding
the text in ONE python process removes that whole failure class.

Usage: python3 .acid.py <file>::<fn> [<file>::<fn> ...]

Two candidates in the SAME file are refused: two mutations in one compile
cannot be attributed to either one independently, and the whole point is
per-function attribution.
"""

import hashlib
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ENV = {
    "LOCAL_DIR": str(ROOT / "local"),
    "LIBCLANG_PATH": str(ROOT / "local/libclang_extract/usr/lib/x86_64-linux-gnu"),
    "PKG_CONFIG_PATH": str(ROOT / "local/pkgconfig"),
    "LD_LIBRARY_PATH": str(ROOT / "local/lib"),
    "RUSTFLAGS": f"-L native={ROOT / 'local/lib'}",
    "CARGO_INCREMENTAL": "0",
    "BINDGEN_EXTRA_CLANG_ARGS": (
        "-I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include"
    ),
    "PATH": "/home/ishanp/.cargo/bin:/usr/local/bin:/usr/bin:/bin",
}


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def main() -> int:
    specs = sys.argv[1:]
    if not specs:
        print(
            "usage: acid-test-pub-fn.py <file>::<fn> [...]",
            file=sys.stderr,
        )
        return 2

    files = [s.rsplit("::", 1)[0] for s in specs]
    dupes = {f for f in files if files.count(f) > 1}
    if dupes:
        print(f"TWO CANDIDATES IN ONE FILE (refused): {sorted(dupes)}", file=sys.stderr)
        return 2

    verdicts = []
    # Set when a restore fails. Checked AFTER the loop rather than returning
    # from inside `finally`: a `return` there aborts the whole run and discards
    # every verdict already collected, which is the opposite of what a failed
    # restore needs (you want the results you DID get, plus the alarm).
    restore_failed = False
    for spec in specs:
        if "::" not in spec:
            print(f"BAD SPEC {spec} (want <file>::<fn>)", file=sys.stderr)
            return 2
        rel, fn = spec.rsplit("::", 1)
        path = ROOT / rel
        if not path.is_file():
            print(f"MISSING {rel}", file=sys.stderr)
            return 2

        original = path.read_bytes()
        digest = sha(original)
        text = original.decode()

        # Match the DEFINITION, not a mention. Two defects made the naive
        # `f"pub fn {fn}"` substring unsafe (both hit in iter-2757):
        #   1. PREFIX COLLISION. `with_holon` is a prefix of the LIVE
        #      `with_holon_opt`, so replacing the first occurrence mutated the
        #      wrong function and reported a verdict about a different symbol.
        #      The needle now requires a `(` immediately after the name.
        #   2. DOC-COMMENT GAP. A doc comment may sit between an attribute and
        #      `pub fn`, so a definition anchored on the attribute is missed and
        #      the candidate is reported SKIP, which reads as "not a candidate".
        #      The regex therefore matches on the `fn` line alone, not on the
        #      attribute above it.
        #
        # Still a definition-shape match: it must be `pub fn <name>(` at the
        # start of a line (any indent), which a call site or a doc reference
        # cannot satisfy.
        pattern = re.compile(rf"(?m)^[ \t]*pub fn {re.escape(fn)}[ \t]*\(")
        m = pattern.search(text)
        if m is None:
            print(
                f"SKIP  {fn}  (no `pub fn {fn}(` definition in {rel})",
                file=sys.stderr,
            )
            return 2
        if len(pattern.findall(text)) > 1:
            print(
                f"AMBIGUOUS  {fn}  ({len(pattern.findall(text))} definitions in {rel})",
                file=sys.stderr,
            )
            return 2

        mutated = pattern.sub(f"fn {fn}(", text, count=1)
        try:
            path.write_text(mutated)
            proc = subprocess.run(
                ["cargo", "check", "-p", "operant-core", "--lib"],
                cwd=ROOT,
                env=ENV,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=900,
            )
            verdicts.append(("DEAD" if proc.returncode == 0 else "LIVE", fn, rel))
        finally:
            path.write_bytes(original)
            if sha(path.read_bytes()) != digest:
                print(f"RESTORE FAILED for {rel}", file=sys.stderr)
                restore_failed = True

    for verdict, fn, rel in verdicts:
        print(f"{verdict}  {fn}  ({rel})")

    if restore_failed:
        print("ABORTED: a restore failed; treat every verdict above as unproven.", file=sys.stderr)
        return 3

    dirty = subprocess.run(
        ["git", "status", "--porcelain"], cwd=ROOT, capture_output=True, text=True
    ).stdout
    tracked = [ln for ln in dirty.splitlines() if not ln.startswith("??")]
    print("tree: " + ("(clean)" if not tracked else "; ".join(tracked)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())