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
  4. the child's resource subtrees (references/, frameworks/, scripts/, …)
     travel with the node: <child>/<res>/ -> references/<child>/<res>/, so
     the emptied child dir really empties and citations keep resolving;
  5. pointers are rewritten, prose never is — one pass, original markup
     kind preserved except node pointers which become links (plan rule 3):
       `x/SKILL.md` / [x](x/SKILL.md)  -> [x](new.md)   target migrated
       `x/SKILL.md` (absent target)    -> `references/x.md`  hint
       `x/` dir of a migrated node     -> [x](new.md)
       resource target (file or dir)   -> path re-emitted so it still
           resolves from the moved file (file-relative for links,
           root-relative for backticks — mirrors the validator's order)
     Fenced code blocks are untouched — the validator ignores them too;
  6. a parent routing line that is nothing but a bare link gets the child's
     description folded in ("fold description into the parent's routing line
     if the parent lacks one", plan §3 iter-4 step 1);
  7. emptied child dirs are removed; a dir left with real (non-hidden) files
     is reported, never silently deleted; other empty ancestors are pruned;
  8. content-preservation assertion: each node's and the root's prose must
     survive byte-for-byte modulo pointer spans — proven by masking every
     pointer and the inserted summary on both sides and requiring exact
     equality (fences included — they are never rewritten);
  9. _map.md + _registry.yaml are regenerated and re-validated via the
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
LINK = re.compile(r"\[([^\]]*)\]\(([^)\s]+)\)")
SKILL_PTR = re.compile(r"^([\w./-]+)/SKILL\.md$")
DIR_PTR = re.compile(r"^([\w./-]+)/$")
# bare, dir-qualified pointer outside any markup (table cells, descriptions):
# the validator never sees these, but they name directories this migration
# deletes — the iter-6 grep gate bans them below any root, so rewrite them.
BARE_SKILL = re.compile(r"[\w.-]+(?:/[\w.-]+)*/SKILL\.md")
PATHISH = re.compile(r"^[\w./-]+\.(md|py|sh|json|ya?ml|toml|txt|html?|css|js|ts|csv)$")
BARE_ROUTING = re.compile(r"^(\s*[-*]\s*)\[([^\]]+)\]\(([^)]+)\)\s*$")
MASK_LINK = re.compile(r"\[[^\]]*\]\([^)]*\)")
MASK_BT = re.compile(r"`[^`\n]+`")
MASK_BARE_SKILL = BARE_SKILL
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


def compute_moves(root, migs):
    """(file_moves, dir_moves): every resource file/dir that relocates when a
    child node moves into references/. Nested node dirs are skipped (they map
    via their own migs entry); hidden/_ working state stays put."""
    file_moves, dir_moves = dict(migs), {}
    for old_skill in migs:
        d = posixpath.dirname(old_skill)
        new_dir = migs[old_skill][:-3]  # references/<d>
        for dp, dns, fns in os.walk(os.path.join(root, d)):
            rel = os.path.relpath(dp, root).replace(os.sep, "/")
            keep = []
            for name in sorted(dns):
                sub = f"{rel}/{name}"
                if name.startswith((".", "_")):
                    continue
                if os.path.isfile(os.path.join(root, sub, "SKILL.md")):
                    continue  # nested node — mapped by its own migs entry
                keep.append(name)
                dir_moves[sub] = f"{new_dir}/{sub[len(d) + 1:]}"
            dns[:] = keep
            for fn in sorted(fns):
                if fn.startswith((".", "_")) or fn == "SKILL.md":
                    continue
                old_rel = f"{rel}/{fn}"
                file_moves[old_rel] = f"{new_dir}/{old_rel[len(d) + 1:]}"
    return file_moves, dir_moves


def resolve_old(target, old_dir, root, prefer):
    """Abs path of target as the ORIGINAL tree reads it, existence-checked.
    prefer picks candidate order — 'file' mirrors link resolution, 'root'
    mirrors backtick resolution (the validator tries both; we must resolve the
    way the ORIGINAL corpus meant it). None when nothing inside root exists."""
    cands = []
    root_s = str(root)
    file_c = os.path.normpath(os.path.join(root_s, old_dir, target))
    root_c = os.path.normpath(os.path.join(root_s, target))
    for c in (file_c, root_c) if prefer == "file" else (root_c, file_c):
        if (c == root_s or c.startswith(root_s + os.sep)) and c not in cands:
            cands.append(c)
    for c in cands:
        if os.path.exists(c):
            return c
    return None


def file_rel(new_rel, new_dir):
    return posixpath.relpath(new_rel, new_dir) if new_dir else new_rel


def rewrite_target(target, kind, label, old_dir, new_dir, root, file_moves, dir_moves, stats):
    """Final replacement for ONE whole pointer (markup included). kind 'link'
    keeps/uses the original label; kind 'bt' returns link markup for node
    pointers (plan rule 3) and backticked paths otherwise. Resource pointers
    keep their kind but get a path that still resolves from the moved file.
    Absent node targets degrade to a backticked hint — a dangling link would
    be an ERROR."""
    if "://" in target or target.startswith(("mailto:", "#", "<")):
        return f"[{label}]({target})" if kind == "link" else f"`{target}`"
    # File-relative first for BOTH kinds, falling back to root-relative: the
    # corpus mixes both styles, and where a child ships its own references/x.md
    # alongside a root-level one, the author meant the LOCAL file (root-relative
    # preference silently redirected citations to the root's copy and orphaned
    # the child's — observed in linkedin-marketing).
    abs_old = resolve_old(target, old_dir, root, "file")
    rel_old = os.path.relpath(abs_old, root).replace(os.sep, "/") if abs_old else None
    skill = SKILL_PTR.match(target)
    dirm = DIR_PTR.match(target)

    def as_link(path, lbl):
        return f"[{lbl}]({path})"

    def as_bt(path):
        return f"`{path}`"

    if rel_old and rel_old in file_moves:  # target migrates with/into references/
        new_rel = file_moves[rel_old]
        if skill:
            stats["links"] += 1
            lbl = label if kind == "link" else posixpath.basename(new_rel)[:-3]
            return as_link(file_rel(new_rel, new_dir), lbl)
        stats["res"] += 1
        if kind == "link":
            return as_link(file_rel(new_rel, new_dir), label)
        return as_bt(new_rel)
    if abs_old and rel_old and rel_old + "/SKILL.md" in file_moves:
        # dir whose SKILL.md migrates → point at the node (its group dir hangs
        # off the index file, so the node link reaches everything)
        new_rel = file_moves[rel_old + "/SKILL.md"]
        stats["links"] += 1
        lbl = label if kind == "link" else posixpath.basename(new_rel)[:-3]
        return as_link(file_rel(new_rel, new_dir), lbl)
    if abs_old and rel_old in dir_moves:  # resource dir travels with the node
        stats["res"] += 1
        new_dir_rel = dir_moves[rel_old]
        if kind == "link":
            return as_link(file_rel(new_dir_rel, new_dir) + "/", label)
        return as_bt(new_dir_rel + "/")
    if abs_old:  # exists and stays — re-emit so the moved file still finds it
        if skill:
            # covers root SKILL.md targets (`../SKILL.md`) and hidden-dir
            # SKILL.md that never move: must not keep the SKILL.md string
            # (leftover check) yet must keep resolving
            stats["links"] += 1
            if kind == "link":
                lbl = label
            elif rel_old == ROOT_SKILL:
                lbl = "root"
            else:
                lbl = posixpath.basename(rel_old)[:-3]
            return as_link(file_rel(rel_old, new_dir), lbl)
        if dirm:
            stats["left"] += 1
            return as_bt(rel_old + "/")
        stats["res"] += 1
        if kind == "link":
            return as_link(file_rel(rel_old, new_dir), label)
        return as_bt(rel_old)
    # unresolvable in the original tree
    if skill:
        stats["hints"] += 1
        return as_bt(f"references/{skill.group(1)}.md")
    stats["left"] += 1
    return f"[{label}]({target})" if kind == "link" else f"`{target}`"


def rewrite_line(line, old_dir, new_dir, root, file_moves, dir_moves, stats,
                  node_names=None):
    """Single pass: backtick spans and out-of-span links each rewritten once,
    so emitted text is never re-processed against the old tree."""
    node_names = node_names or {}
    spans = list(BT.finditer(line))
    out, pos = [], 0

    def rewrite_gap(gap):
        """Links first, bare dir-SKILL tokens in the segments between them —
        emitted markup never re-enters the old-tree resolution."""
        def bare(segment):
            return BARE_SKILL.sub(
                lambda bm: rewrite_target(bm.group(0), "bt", None, old_dir,
                                          new_dir, root, file_moves, dir_moves, stats),
                segment)
        out, pos = [], 0
        for m in LINK.finditer(gap):
            out.append(bare(gap[pos:m.start()]))
            out.append(rewrite_target(m.group(2), "link", m.group(1), old_dir,
                                      new_dir, root, file_moves, dir_moves, stats))
            pos = m.end()
        out.append(bare(gap[pos:]))
        return "".join(out)

    for m in spans:
        out.append(rewrite_gap(line[pos:m.start()]))
        body = m.group(1)
        if (SKILL_PTR.match(body) or DIR_PTR.match(body)
                or ("/" in body and PATHISH.match(body))):
            out.append(rewrite_target(body, "bt", None, old_dir, new_dir,
                                      root, file_moves, dir_moves, stats))
        elif body in node_names:
            # bare skill-name mention (`linkedin-post-writer`) — the only
            # routing form some roots use; without this the node is unreachable
            stats["links"] += 1
            out.append(f"[{body}]({file_rel(node_names[body], new_dir)})")
        else:
            out.append(m.group(0))
        pos = m.end()
    out.append(rewrite_gap(line[pos:]))
    return "".join(out)


def rewrite_pointers(lines, old_dir, new_dir, root, file_moves, dir_moves, stats,
                     node_names=None):
    out, in_fence = [], False
    for line in lines:
        if FENCE.match(line):
            in_fence = not in_fence
            out.append(line)
            continue
        out.append(line if in_fence else
                   rewrite_line(line, old_dir, new_dir, root, file_moves,
                                dir_moves, stats, node_names))
    return out


def masked(lines, inserted_at):
    """Pointer-free view of a node body, blank lines dropped: two bodies that
    mask equal contain the same non-blank prose lines byte-for-byte modulo
    pointer spans (blank-line structure is deliberately ignored — the summary
    insertion owns it, and rewrites never add or merge lines). The summary is
    removed by INDEX: its pointers are rewritten, so content no longer
    identifies it."""
    if inserted_at is not None and inserted_at < len(lines):
        lines = lines[:inserted_at] + lines[inserted_at + 1:]
    out = [MASK_LINK.sub("‹ptr›", MASK_BT.sub("‹ptr›", l)) for l in lines]
    out = [MASK_BARE_SKILL.sub("‹ptr›", l) for l in out]
    return [l for l in out if l.strip()]


def non_fence(lines):
    out, in_fence = [], False
    for l in lines:
        if FENCE.match(l):
            in_fence = not in_fence
            continue
        if not in_fence:
            out.append(l)
    return out


def build_node(old_rel, new_rel, text, root, file_moves, dir_moves, stats,
               node_names=None):
    """Returns (final_lines, inserted_summary). Raises AssertionError when
    prose did not survive the rewrite."""
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

    inserted_desc, inserted_at = None, None
    if desc:
        j = 1
        while j < len(lines) and not lines[j].strip():
            j += 1
        first = lines[j].strip() if j < len(lines) else ""
        if first != desc:
            lines = lines[:1] + ["", desc, ""] + lines[j:]
            inserted_desc, inserted_at = desc, 2  # [H1, '', desc, '']

    old_dir = posixpath.dirname(old_rel)
    new_dir = posixpath.dirname(new_rel)
    lines = rewrite_pointers(lines, old_dir, new_dir, root, file_moves, dir_moves,
                             stats, node_names)

    if masked(lines, inserted_at) != masked(original, None):
        raise AssertionError(f"prose changed while migrating {old_rel}")
    leftovers = [l for l in non_fence(lines) if BARE_SKILL.search(l)]
    if leftovers:
        raise AssertionError(
            f"{old_rel}: {len(leftovers)} dir-qualified SKILL.md pointer(s) survived "
            f"the rewrite: {repr(leftovers[0][:90])}"
        )
    return lines, inserted_desc


def fold_bare_routing(lines, descs):
    """Step 6: a routing line that is only a bare link gets the child's
    description appended (safety net for trees whose parent routing line is a
    stub — the pilot trees all carry descriptions already)."""
    out = []
    for l in lines:
        m = BARE_ROUTING.match(l)
        if m and m.group(3) in descs and descs[m.group(3)]:
            out.append(f"{m.group(1)}[{m.group(2)}]({m.group(3)}) — {descs[m.group(3)]}")
        else:
            out.append(l)
    return out


def perform_moves(root, file_moves, dir_moves, migs, stats, skip=()):
    """Physical relocation of resource files (nodes are written, not moved;
    `skip` holds sources whose rewritten content was already written out).
    Returns (moved, problems)."""
    moved, problems = 0, []
    skip = set(skip) | set(migs)
    for old_rel, new_rel in sorted(file_moves.items()):
        if old_rel in skip:
            continue  # node or rewritten .md: written to new_rel already
        src, dst = root / old_rel, root / new_rel
        if not src.is_file():
            problems.append(f"move source vanished: {old_rel}")
            continue
        if dst.exists():
            problems.append(f"move target occupied: {new_rel}")
            continue
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.move(str(src), str(dst))
        moved += 1
    # every moved file: source gone, destination present
    for old_rel, new_rel in file_moves.items():
        if old_rel in skip:
            continue
        if (root / old_rel).exists():
            problems.append(f"move source still present: {old_rel}")
        if not (root / new_rel).is_file():
            problems.append(f"move target missing: {new_rel}")
    stats["moved"] = moved
    return moved, problems


def rewrite_static(root, migs, file_moves, dir_moves, node_names, stats):
    """Rewrite pointers in every OTHER .md below root: files that travel
    (their outgoing pointers break when their directory depth changes) and
    files that stay (root-level references/*.md, README.md — they cite child
    paths this migration relocates). Node files and the root are handled
    separately. Returns (pending_writes, moved_sources, problems): content is
    resolved against the INTACT old tree, so this must run before any move."""
    pending, moved_sources, problems = {}, [], []
    for dp, dns, fns in os.walk(root):
        dns[:] = sorted(d for d in dns if not d.startswith((".", "_")))
        for fn in sorted(fns):
            if fn.startswith((".", "_")) or not fn.endswith(".md"):
                continue
            rel = os.path.relpath(os.path.join(dp, fn), root).replace(os.sep, "/")
            if rel == ROOT_SKILL or rel in migs or rel in migs.values():
                continue  # root handled separately; node sources/targets are not static
            text = (root / rel).read_text(encoding="utf-8")
            if rel in file_moves:
                new_rel = file_moves[rel]
                old_dir, new_dir = posixpath.dirname(rel), posixpath.dirname(new_rel)
            else:
                new_rel = rel
                old_dir = new_dir = posixpath.dirname(rel)
            lines = rewrite_pointers(text.splitlines(), old_dir, new_dir, root,
                                     file_moves, dir_moves, stats, node_names)
            if masked(lines, None) != masked(text.splitlines(), None):
                problems.append(f"prose changed while rewriting {rel}")
                continue
            left = [l for l in non_fence(lines) if BARE_SKILL.search(l)]
            if left:
                problems.append(
                    f"{rel}: dir-qualified SKILL.md pointer survived the rewrite: "
                    + repr(left[0][:90]))
                continue
            pending[new_rel] = "\n".join(lines) + "\n"
            if rel in file_moves:
                moved_sources.append(rel)
    return pending, moved_sources, problems


def prune_empty_dirs(root):
    """Bottom-up removal of dirs the migration emptied (incl. container dirs
    like skills/ that held only nodes). Emptiness is checked LIVE per dir:
    os.walk's captured listings go stale the moment a child is removed, so a
    parent would see its already-deleted children and keep itself. Hidden/_
    residue keeps its dir."""
    pruned = 0
    for dp, _, _ in os.walk(root, topdown=False):
        if os.path.abspath(dp) == os.path.abspath(root):
            continue
        try:
            with os.scandir(dp) as it:
                if not any(it):
                    os.rmdir(dp)
                    pruned += 1
        except OSError:
            pass
    return pruned


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
    file_moves, dir_moves = compute_moves(root, migs)
    node_names = {}
    for old_rel, new_rel in migs.items():
        name = posixpath.basename(posixpath.dirname(old_rel))
        node_names.setdefault(name, new_rel)
    stats = {"links": 0, "hints": 0, "res": 0, "left": 0, "moved": 0}
    problems = []

    # namespace collisions: a node path already occupied by a root resource
    for old_rel, new_rel in migs.items():
        if (root / new_rel).exists():
            problems.append(f"node target occupied by existing file: {new_rel}")

    # --- phase A: build every migrated node in memory, assert prose survival
    old_texts = {rel: (root / rel).read_text(encoding="utf-8") for rel in migs}
    built = {}
    for rel in sorted(migs):
        old_lines = len(old_texts[rel].splitlines())
        try:
            lines, inserted_desc = build_node(rel, migs[rel], old_texts[rel], root,
                                              file_moves, dir_moves, stats,
                                              node_names)
        except AssertionError as e:
            problems.append(str(e))
            continue
        built[rel] = (lines, inserted_desc)
        print(f"  {rel} -> {migs[rel]}  ({old_lines} -> {len(lines)} lines"
              f"{', +summary' if inserted_desc else ''})")

    # --- phase B: rewrite the root's own pointers + fold bare routing lines
    if root_skill.exists():
        root_text = root_skill.read_text(encoding="utf-8")
    else:  # dry-run: the orphan restore was skipped, so read the source itself
        root_text = Path(args.root_source).expanduser().resolve().read_text(encoding="utf-8")
    root_lines = rewrite_pointers(root_text.splitlines(), "", "", root,
                                  file_moves, dir_moves, stats, node_names)
    if masked(root_lines, None) != masked(root_text.splitlines(), None):
        problems.append("prose changed while rewriting root SKILL.md")
    root_leftover = [l for l in non_fence(root_lines) if BARE_SKILL.search(l)]
    if root_leftover:
        problems.append(f"dir-qualified SKILL.md pointer(s) survived in root SKILL.md: "
                        + repr(root_leftover[0][:90]))
    descs = {migs[rel]: ins or "" for rel, (_, ins) in built.items()}
    root_lines = fold_bare_routing(root_lines, descs)

    # every other .md below root: moving files' and staying files' pointers
    pending, static_moved, static_problems = rewrite_static(
        root, migs, file_moves, dir_moves, node_names, stats)
    problems.extend(static_problems)
    print(f"  static .md rewrites: {len(pending)} ({len(static_moved)} travel with their dir)")

    # Nothing may be written while any node failed its assertions: deleting an
    # old SKILL.md whose replacement never got written would lose the prose.
    if problems:
        print("\nFAILED (nothing written):")
        for p in problems:
            print(f"  - {p}")
        sys.exit(1)

    # --- phase C: write nodes + root, unlink old, move resources, prune
    if args.dry_run:
        print(f"dry-run: would write {len(built)} nodes + root + {len(pending)} static .md, "
              f"move {len(file_moves) - len(migs) - len(static_moved)} resource files, "
              f"map {len(dir_moves)} resource dirs, prune emptied dirs")
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
        # rewritten static .md: destinations first, then drop the moved sources
        for new_rel, content in sorted(pending.items()):
            dest = root / new_rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(content, encoding="utf-8")
            if dest.read_text(encoding="utf-8") != content:
                problems.append(f"write verification failed: {new_rel}")
        for rel in static_moved:
            (root / rel).unlink(missing_ok=True)
        moved, move_problems = perform_moves(root, file_moves, dir_moves, migs,
                                             stats, skip=static_moved)
        problems.extend(move_problems)
        stats["pruned"] = prune_empty_dirs(root)
        # a child dir that survived with real files = unhandled structure
        for rel in sorted(migs, key=lambda r: r.count("/"), reverse=True):
            d = root / posixpath.dirname(rel)
            if d.is_dir():
                real = [f.name for f in d.iterdir()
                        if not f.name.startswith((".", "_"))]
                if real:
                    problems.append(
                        f"child dir not emptied (left in place): "
                        f"{posixpath.dirname(rel)}/ ({', '.join(real[:4])})")

    print(f"pointers: {stats['links']} -> node links, {stats['res']} -> re-pathed resources, "
          f"{stats['hints']} -> absent-node hints, {stats['left']} left as-is"
          + (f"; files moved: {stats['moved']}, dirs pruned: {stats.get('pruned', 0)}"
             if not args.dry_run else ""))

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
