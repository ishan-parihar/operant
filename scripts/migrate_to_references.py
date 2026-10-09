#!/usr/bin/env python3
"""Migrate a skill-routing tree to the nested-references model.

Old model: every node was a directory holding its own SKILL.md — the skill
routed to child *skills*. New model (docs/plan-2026-10-09-meta-skill-
creator-nested-references.md §2): ONE root SKILL.md over a references/ tree
of plain .md nodes.

Per child SKILL.md:
  1. strip frontmatter (child frontmatter is a second, unregistered trigger
     surface — it dies with the migration);
  2. body -> references/<relpath>.md, directory structure preserved (a leaf
     dir collapses to one file; a router dir keeps its dir and its SKILL.md
     becomes the sibling index — the contract's <dir>.md convention);
  3. the frontmatter description becomes the node's one-line summary under
     its H1 (contract rule 2: the summary absorbs the routing description);
  4. pointers are rewritten, prose never is:
       `x/SKILL.md`  -> [x](relative.md)      target migrated into references/
       `x/SKILL.md`  -> `references/x.md`     target absent in this tree (hint)
       `x/`          -> [x](relative.md)      dir whose SKILL.md migrated
     Fenced code blocks are untouched — the validator ignores them too;
  5. a parent routing line that is nothing but a bare link gets the child's
     description folded in ("fold description into the parent's routing line
     if the parent lacks one", plan §3 iter-4 step 1);
  6. emptied child dirs are removed; a dir left with other files is reported,
     never silently deleted;
  7. content-preservation assertion: each node's prose must survive
     byte-for-byte modulo pointer spans — proven by masking every pointer and
     the inserted summary on both sides and requiring exact equality;
  8. _map.md + _registry.yaml are regenerated and re-validated via the
     meta-skill-creator's registry.py (the skill ships its own gate).

Usage:
  python3 scripts/migrate_to_references.py <root> [--root-source FILE]
                                               [--registry PATH] [--dry-run]

--root-source restores an orphaned root (copy SKILL.md from a source tree)
before migrating; it errors if the root already exists.

Exit 0 only when every assertion held and registry.py exits 0.
"""

import argparse
import os
import posixpath
import re
import shutil
import subprocess
import sys
from pathlib import Path

FM_KEY = re.compile(r"^([A-Za-z_][\w-]*):\s*(.*)$")
FENCE = re.compile(r"^\s*(?:```|~~~)")
BT = re.compile(r"`([^`\n]+)`")
LINK_TGT = re.compile(r"\]\(([^)\s]+)\)")
SKILL_PTR = re.compile(r"^([\w./-]+)/SKILL\.md$")
DIR_PTR = re.compile(r"^([\w./-]+)/$")
BARE_ROUTING = re.compile(r"^(\s*[-*]\s*)\[([^\]]+)\]\(([^)]+)\)\s*$")
MASK_LINK = re.compile(r"\[[^\]]*\]\([^)]*\)")
MASK_BT = re.compile(r"`[^`\n]+`")
ROOT_SKILL = "SKILL.md"

DEFAULT_REGISTRY = (
    Path(__file__).resolve().parent.parent
    / "skills" / "software-development" / "meta-skill-creator"
    / "scripts" / "registry.py"
)


def split_frontmatter(text):
    """(frontmatter dict, body text). Naive top-level `key: value` parser with
    indented continuation folding — same rules as registry.py, kept local so
    this script has no import coupling to a skill's scripts/ directory."""
    lines = text.splitlines(keepends=True)
    if not lines or lines[0].strip() != "---":
        return {}, text
    fm, key, end = {}, None, None
    for i, line in enumerate(lines[1:], start=1):
        if line.strip() == "---":
            end = i
            break
        m = FM_KEY.match(line.rstrip("\n"))
        if m:
            key = m.group(1)
            fm[key] = m.group(2).strip()
        elif key and line[:1] in (" ", "\t"):
            fm[key] += " " + line.strip()
    if end is None:
        return {}, text
    for k, v in list(fm.items()):
        if len(v) >= 2 and v[0] == v[-1] and v[0] in "\"'":
            fm[k] = v[1:-1].strip()
    return fm, "".join(lines[end + 1:])


def scan_children(root):
    """{old_rel: new_rel} for every SKILL.md below root; hidden/_ dirs skipped
    (working state like _build/ and .codex-marketplace/ stays invisible, which
    is also how registry.py's validator treats it)."""
    migs = {}
    for dp, dns, fns in os.walk(root):
        dns[:] = sorted(d for d in dns if not d.startswith((".", "_")))
        for fn in sorted(fns):
            if fn != "SKILL.md":
                continue
            rel = os.path.relpath(os.path.join(dp, fn), root).replace(os.sep, "/")
            if rel == ROOT_SKILL:
                continue
            p = rel[: -len("/SKILL.md")]
            migs[rel] = f"references/{p}.md"
    return migs


def old_candidates(target, old_dir):
    """Skill-root-relative readings of an old pointer: file-relative first
    (markdown semantics), then root-relative (the corpus writes root-relative
    pointers from depth >= 2). Paths escaping the root are dropped."""
    cands = []
    r1 = posixpath.normpath(posixpath.join(old_dir, target))
    if not r1.startswith(".."):
        cands.append(r1)
    r2 = posixpath.normpath(target)
    if not target.startswith("..") and not r2.startswith("..") and r2 not in cands:
        cands.append(r2)
    return cands


def choose(cands, migs):
    """The migrated node a pointer meant: in-map candidate wins (file-relative
    priority matches markdown resolution); when neither reading migrated,
    prefer the root-relative one — the corpus's cross-branch convention."""
    for c in cands:
        if c in migs:
            return c
    return cands[-1] if cands else None


def choose_dir(cands, migs):
    """Same for dir-form pointers (`briefing/`): resolved against the FILE the
    dir used to head."""
    for c in cands:
        if (c + "/SKILL.md") in migs:
            return c
    return None


def link_for(new_rel, from_dir):
    return posixpath.relpath(new_rel, from_dir or ".")


def rewrite_line(line, old_dir, new_dir, migs, stats):
    def bt_repl(m):
        span = m.group(1)
        mo = SKILL_PTR.match(span)
        if mo:
            chosen = choose(old_candidates(mo.group(1) + "/SKILL.md", old_dir), migs)
            if chosen is None or chosen == ROOT_SKILL:
                stats["left"] += 1
                return m.group(0)
            if chosen in migs:
                stats["links"] += 1
                label = posixpath.basename(migs[chosen])[:-3]
                return f"[{label}]({link_for(migs[chosen], new_dir)})"
            stats["hints"] += 1
            return f"`references/{chosen[:-len('/SKILL.md')]}.md`"
        mo2 = DIR_PTR.match(span)
        if mo2:
            chosen_dir = choose_dir(old_candidates(mo2.group(1), old_dir), migs)
            if chosen_dir is not None:
                stats["links"] += 1
                new_rel = migs[chosen_dir + "/SKILL.md"]
                label = posixpath.basename(new_rel)[:-3]
                return f"[{label}]({link_for(new_rel, new_dir)})"
            stats["left"] += 1
            return m.group(0)
        return m.group(0)

    def ln_repl(m):
        tgt = m.group(1)
        if not tgt.endswith("/SKILL.md"):
            return m.group(0)
        chosen = choose(old_candidates(tgt, old_dir), migs)
        if chosen is None or chosen == ROOT_SKILL:
            stats["left"] += 1
            return m.group(0)
        if chosen in migs:
            stats["links"] += 1
            return f"]({link_for(migs[chosen], new_dir)})"
        stats["hints"] += 1
        return f"](references/{chosen[:-len('/SKILL.md')]}.md)"

    line = BT.sub(bt_repl, line)
    return LINK_TGT.sub(ln_repl, line)


def rewrite_pointers(lines, old_dir, new_dir, migs, stats):
    out, in_fence = [], False
    for line in lines:
        if FENCE.match(line):
            in_fence = not in_fence
            out.append(line)
            continue
        out.append(line if in_fence else rewrite_line(line, old_dir, new_dir, migs, stats))
    return out


def masked(lines, inserted):
    """Pointer-free view of a node body, blank lines dropped: two bodies that
    mask equal contain the same non-blank prose lines byte-for-byte modulo
    pointer spans (blank-line structure is deliberately ignored — the summary
    insertion owns it, and rewrites never add or merge lines)."""
    if inserted is not None:
        for i, l in enumerate(lines):
            if l == inserted:
                lines = lines[:i] + lines[i + 1:]
                break
    out = [MASK_BT.sub("‹ptr›", MASK_LINK.sub("‹ptr›", l)) for l in lines]
    return [l for l in out if l.strip()]


def build_node(old_rel, new_rel, text, migs, stats):
    """Returns (final_lines, original_body_lines, inserted_summary). Raises
    AssertionError when prose did not survive the rewrite."""
    fm, body = split_frontmatter(text)
    desc = fm.get("description", "").strip()

    lines = body.splitlines()
    k = 0
    while k < len(lines) and not lines[k].strip():
        k += 1
    lines = lines[k:]
    if not lines or not lines[0].startswith("# "):
        title = fm.get("name") or posixpath.basename(posixpath.dirname(old_rel)) or "Untitled"
        lines = [f"# {title}", ""] + lines
    original = list(lines)

    inserted = None
    if desc:
        j = 1
        while j < len(lines) and not lines[j].strip():
            j += 1
        first = lines[j].strip() if j < len(lines) else ""
        if first != desc:
            lines = lines[:1] + ["", desc, ""] + lines[j:]
            inserted = desc

    old_dir = posixpath.dirname(old_rel)
    new_dir = posixpath.dirname(new_rel)
    lines = rewrite_pointers(lines, old_dir, new_dir, migs, stats)

    if masked(lines, inserted) != masked(original, None):
        raise AssertionError(f"prose changed while migrating {old_rel}")
    leftovers = [l for l in lines if "SKILL.md" in l]
    if leftovers:
        raise AssertionError(
            f"{old_rel}: {len(leftovers)} SKILL.md pointer(s) survived the rewrite: "
            + repr(leftovers[0][:90])
        )
    return lines, original, inserted


def fold_bare_routing(lines, descs):
    """Step 5: a routing line that is only a bare link gets the child's
    description appended (pilot trees all carry descriptions already, so this
    is the safety net for trees whose parent routing line is a stub)."""
    out = []
    for l in lines:
        m = BARE_ROUTING.match(l)
        if m and m.group(3) in descs and descs[m.group(3)]:
            out.append(f"{m.group(1)}[{m.group(2)}]({m.group(3)}) — {descs[m.group(3)]}")
        else:
            out.append(l)
    return out


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("root", help="skill root (must contain SKILL.md, or pass --root-source)")
    ap.add_argument("--root-source", default=None,
                    help="copy this SKILL.md to <root>/SKILL.md when missing (orphan restore)")
    ap.add_argument("--registry", default=str(DEFAULT_REGISTRY),
                    help="registry.py used to regenerate/validate (default: repo meta-skill-creator)")
    ap.add_argument("--dry-run", action="store_true", help="assert + report, write nothing")
    args = ap.parse_args()

    root = Path(args.root).resolve()
    if not root.is_dir():
        sys.exit(f"error: {root} is not a directory")
    root_skill = root / ROOT_SKILL
    if not root_skill.exists():
        if not args.root_source:
            sys.exit(f"error: {root_skill} missing and no --root-source given")
        src = Path(args.root_source).expanduser().resolve()
        fm, _ = split_frontmatter(src.read_text(encoding="utf-8"))
        if fm.get("name", "") != root.name:
            sys.exit(f"error: root-source name '{fm.get('name', '')}' != dir '{root.name}'")
        if not args.dry_run:
            shutil.copyfile(src, root_skill)
        print(f"restored orphan root: {src} -> {root_skill}")

    migs = scan_children(root)
    if not migs:
        print("nothing to migrate: no SKILL.md below root")
    stats = {"links": 0, "hints": 0, "left": 0}
    problems = []

    # --- phase A: build every migrated node in memory, assert prose survival
    old_texts = {rel: (root / rel).read_text(encoding="utf-8") for rel in migs}
    built = {}
    for rel in sorted(migs):
        old_lines = len(old_texts[rel].splitlines())
        try:
            lines, original, inserted = build_node(rel, migs[rel], old_texts[rel], migs, stats)
        except AssertionError as e:
            problems.append(str(e))
            continue
        built[rel] = (lines, inserted)
        print(f"  {rel} -> {migs[rel]}  ({old_lines} -> {len(lines)} lines"
              f"{', +summary' if inserted else ''})")

    # --- phase B: rewrite the root's own pointers + fold bare routing lines
    if root_skill.exists():
        root_text = root_skill.read_text(encoding="utf-8")
    else:  # dry-run: the orphan restore was skipped, so read the source itself
        root_text = Path(args.root_source).expanduser().resolve().read_text(encoding="utf-8")
    root_lines = rewrite_pointers(root_text.splitlines(), "", "", migs, stats)
    root_leftover = [l for l in root_lines if "SKILL.md" in l]
    if root_leftover:
        problems.append(f"SKILL.md pointer(s) survived in root SKILL.md: "
                        + repr(root_leftover[0][:90]))
    descs = {}
    for rel, (lines, inserted) in built.items():
        descs[migs[rel]] = inserted or ""
    root_lines = fold_bare_routing(root_lines, descs)

    # Nothing may be written while any node failed its assertions: deleting an
    # old SKILL.md whose replacement never got written would lose the prose.
    if problems:
        print("\nFAILED (nothing written):")
        for p in problems:
            print(f"  - {p}")
        sys.exit(1)

    # --- phase C: write nodes + root, remove old files, then emptied dirs
    emptied, stuck = [], []
    child_dirs = sorted({posixpath.dirname(rel) for rel in migs} - {""},
                        key=lambda p: p.count("/"), reverse=True)
    if args.dry_run:
        removed = set()  # simulate deepest-first rmdir ordering
        for p in child_dirs:
            d = root / p
            if not d.is_dir():
                continue
            extras = [f.name for f in d.iterdir()
                      if f.name != "SKILL.md"
                      and not (f.is_dir() and str(Path(p) / f.name) in removed)]
            (stuck if extras else emptied).append(p)
            if not extras:
                removed.add(p)
        print(f"dry-run: would write {len(built)} nodes + root, "
              f"remove {len(emptied)} dirs, leave {len(stuck)} non-empty")
    else:
        for rel, (lines, _) in built.items():
            dest = root / migs[rel]
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text("\n".join(lines) + "\n", encoding="utf-8")
            # read-back proof: what landed on disk is what was asserted
            if dest.read_text(encoding="utf-8") != "\n".join(lines) + "\n":
                problems.append(f"write verification failed: {migs[rel]}")
        root_skill.write_text("\n".join(root_lines) + "\n", encoding="utf-8")
        for rel in sorted(built):
            (root / rel).unlink(missing_ok=True)
        for p in child_dirs:
            d = root / p
            if not d.is_dir():
                continue
            try:
                d.rmdir()
                emptied.append(p)
            except OSError:
                stuck.append(p)
    for s in stuck:
        problems.append(f"child dir not emptied (left in place): {s}/")

    print(f"pointers: {stats['links']} -> links, {stats['hints']} -> absent-target hints, "
          f"{stats['left']} left as-is; dirs removed: {len(emptied)}, "
          f"left in place: {len(stuck)}")

    # --- phase D: regenerate + validate through the skill's own gate
    if not args.dry_run:
        registry = Path(args.registry).expanduser().resolve()
        if not registry.is_file():
            problems.append(f"registry.py not found: {registry}")
        else:
            for extra in ([], ["--check"]):
                r = subprocess.run([sys.executable, str(registry), str(root)] + extra,
                                   capture_output=True, text=True)
                sys.stdout.write(r.stdout)
                if r.returncode != 0:
                    problems.append(f"registry.py {'--check ' if extra else ''}exited "
                                    f"{r.returncode}: {r.stderr.strip()[:300]}")

    if problems:
        print("\nFAILED:")
        for p in problems:
            print(f"  - {p}")
        sys.exit(1)
    print("migration OK")


if __name__ == "__main__":
    main()
