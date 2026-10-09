#!/usr/bin/env python3
"""WCAG 2.1 contrast checker for website-design tokens.

Prints the contrast ratio between a foreground and a background color plus
PASS/FAIL for the four conformance thresholds. Exits 1 when AA normal-text
fails, so token pipelines can stop on the first failing pair.

Used by system/SKILL.md, direction/SKILL.md, quality/accessibility/SKILL.md
and references/palettes.md: run it on every foreground/background pair in
the TOKENS artifact before any section code is written.

Usage:
    contrast.py --fg '#1f2937' --bg '#f9fafb'
    contrast.py --fg '#1f2937' --bg '#f9fafb' --quiet
    contrast.py --selftest
"""
import argparse
import sys

GATE = 4.5  # AA normal text: the pipeline gate


def _channel(c):
    c /= 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def luminance(hex_color):
    """Relative luminance of a hex color, WCAG 2.1 formula."""
    h = hex_color.strip().lstrip("#")
    if len(h) == 3:
        h = "".join(ch * 2 for ch in h)
    if len(h) != 6 or any(c not in "0123456789abcdefABCDEF" for c in h):
        raise ValueError(f"invalid hex color: {hex_color!r}")
    r, g, b = (int(h[i:i + 2], 16) for i in (0, 2, 4))
    return 0.2126 * _channel(r) + 0.7152 * _channel(g) + 0.0722 * _channel(b)


def ratio(fg, bg):
    """Contrast ratio, 1.0 to 21.0."""
    lf, lb = luminance(fg), luminance(bg)
    hi, lo = max(lf, lb), min(lf, lb)
    return (hi + 0.05) / (lo + 0.05)


def selftest():
    assert abs(ratio("#000000", "#ffffff") - 21.0) < 0.01
    assert abs(ratio("#ffffff", "#ffffff") - 1.0) < 0.01
    assert ratio("#1f2937", "#f9fafb") > 4.5        # known AA pass
    assert ratio("#3b82f6", "#ffffff") < 4.5        # blue-500 on white, known fail
    assert luminance("#fff") == luminance("#ffffff")  # 3-digit shorthand
    print("contrast selftest OK")


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--fg", help="foreground hex color, e.g. '#1f2937'")
    ap.add_argument("--bg", help="background hex color, e.g. '#f9fafb'")
    ap.add_argument("--quiet", action="store_true", help="one-line output only")
    ap.add_argument("--selftest", action="store_true", help="run built-in checks and exit")
    a = ap.parse_args()
    if a.selftest:
        selftest()
        return
    if not a.fg or not a.bg:
        ap.error("--fg and --bg are required (or pass --selftest)")
    r = ratio(a.fg, a.bg)
    checks = [("AA normal", 4.5), ("AA large", 3.0),
              ("AAA normal", 7.0), ("AAA large", 4.5)]
    verdicts = ",  ".join(f"{name} {'PASS' if r >= limit else 'FAIL'}"
                          for name, limit in checks)
    if a.quiet:
        print(f"{r:.2f}:1  {verdicts}")
    else:
        print(f"ratio {r:.2f}:1  ({a.fg} on {a.bg})")
        print(verdicts)
    sys.exit(0 if r >= GATE else 1)


if __name__ == "__main__":
    main()
