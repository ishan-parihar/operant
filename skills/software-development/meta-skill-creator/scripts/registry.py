#!/usr/bin/env python3
"""Generate a recursive _registry.yaml for a meta-skill tree and validate its structure.

A node is any directory containing a SKILL.md. Child nodes are immediate
subdirectories that also contain a SKILL.md; subdirectories without one
(frameworks/, references/, scripts/, assets/, ...) are resource directories.

Usage:
  python registry.py <root>              # validate + write <root>/_registry.yaml
  python registry.py <root> --check      # validate only, write nothing
  python registry.py <root> -o out.yaml  # write registry elsewhere

<root> may be a single meta-skill (has its own SKILL.md) or a container
directory holding several top-level meta-skills.

Exit code 1 if any ERROR was found, else 0.
"""

import argparse
import json
import os
import re
import sys


def parse_frontmatter(text):
    """Naive single-purpose YAML frontmatter parser: top-level `key: value`
    pairs, with indented continuation lines folded in (how YAML dumpers wrap
    long descriptions). Returns {} if there is no valid frontmatter block."""
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return {}
    fm, key, closed = {}, None, False
    for line in lines[1:]:
        if line.strip() == "---":
            closed = True
            break
        m = re.match(r"^([A-Za-z_][\w-]*):\s*(.*)$", line)
        if m:
            key = m.group(1)
            fm[key] = m.group(2).strip()
        elif key and line[:1] in (" ", "\t"):
            fm[key] += " " + line.strip()
    if not closed:
        return {}
    for k, v in fm.items():
        if len(v) >= 2 and v[0] == v[-1] and v[0] in "\"'":
            v = v[1:-1].strip()
        fm[k] = v
    return fm


def scan(dir_path, rel):
    text = open(os.path.join(dir_path, "SKILL.md"), encoding="utf-8").read()
    fm = parse_frontmatter(text)
    node = {
        "name": fm.get("name", ""),
        "path": rel,
        "description": fm.get("description", ""),
        "skill_lines": len(text.splitlines()),
        "children": [],
        "_dir": dir_path,
        "_body": text,
        "_resources": [],
        "_orphans": [],
    }
    for entry in sorted(os.listdir(dir_path)):
        full = os.path.join(dir_path, entry)
        # _-prefixed dirs (_build/, ...) are working state, invisible to the tree
        if not os.path.isdir(full) or entry.startswith("_"):
            continue
        if os.path.isfile(os.path.join(full, "SKILL.md")):
            node["children"].append(scan(full, f"{rel}/{entry}"))
        else:  # resource dir: collect .md files, flag unreachable SKILL.md
            for dp, _, fns in os.walk(full):
                for fn in fns:
                    p = os.path.relpath(os.path.join(dp, fn), dir_path)
                    if fn == "SKILL.md":
                        node["_orphans"].append(p)
                    elif fn.endswith(".md"):
                        node["_resources"].append(p)
    node["role"] = "router" if node["children"] else "leaf"
    node["resources"] = len(node["_resources"])
    return node


def subtree_text(node):
    # ponytail: O(depth*n) when called per node; index bodies if trees get huge
    return node["_body"] + "".join(subtree_text(c) for c in node["children"])


def validate(node, problems):
    where = node["path"]
    dirname = os.path.basename(node["_dir"])
    if node["name"] != dirname:
        problems.append(("ERROR", where,
                         f"frontmatter name '{node['name']}' != directory name '{dirname}'"))
    desc = node["description"]
    if not desc:
        problems.append(("ERROR", where, "missing description in frontmatter"))
    elif len(desc) < 60:
        problems.append(("WARN", where,
                         f"description is only {len(desc)} chars — too thin to route on; "
                         "say what it does AND when to use it"))
    limit = 200 if node["role"] == "router" else 500
    if node["skill_lines"] > limit:
        problems.append(("WARN", where,
                         f"SKILL.md is {node['skill_lines']} lines (>{limit} for a "
                         f"{node['role']}) — consider splitting"))
    for child in node["children"]:
        child_dir = os.path.basename(child["_dir"])
        if child_dir not in node["_body"]:
            problems.append(("ERROR", where,
                             f"child '{child_dir}/' is never mentioned in SKILL.md — "
                             "no agent can route to it"))
        validate(child, problems)
    # a resource may be cited by any SKILL.md in this node's subtree
    # (shared resources live at the lowest common ancestor)
    cited_in = subtree_text(node)
    for res in node["_resources"]:
        if os.path.basename(res) not in cited_in:
            problems.append(("WARN", where,
                             f"resource '{res}' is not referenced by any SKILL.md "
                             "in this subtree — dead weight or missing pointer"))
    for orphan in node["_orphans"]:
        problems.append(("ERROR", where,
                         f"orphan SKILL.md at '{orphan}' — an ancestor directory has "
                         "no SKILL.md, so this node is unreachable"))


def totals(tops):
    t = {"nodes": 0, "routers": 0, "leaves": 0, "resource_files": 0, "max_depth": 0}

    def walk(n, d):
        t["nodes"] += 1
        t["routers" if n["role"] == "router" else "leaves"] += 1
        t["resource_files"] += n["resources"]
        t["max_depth"] = max(t["max_depth"], d)
        for c in n["children"]:
            walk(c, d + 1)

    for n in tops:
        walk(n, 1)
    return t


def subtree_count(node):
    return 1 + sum(subtree_count(c) for c in node["children"])


def first_sentence(desc, cap=120):
    m = re.match(r"(.{20,}?[.!?])\s", desc + " ")
    s = m.group(1) if m else desc
    return s if len(s) <= cap else s[:cap - 1].rstrip() + "…"


def write_map(dir_path, title, children, shard):
    """Write _map.md at dir_path covering `children`; branches larger than
    `shard` nodes get their own recursive _map.md and one pointer line here,
    so every map stays a single bounded read at any tree size."""
    lines = [f"# {title} — node map (generated by registry.py, do not hand-edit)", ""]
    written = []

    def emit(kids, depth):
        for c in kids:
            rel = os.path.relpath(c["_dir"], dir_path)
            cnt = subtree_count(c)
            if cnt > shard:
                written.extend(write_map(c["_dir"], c["name"], c["children"], shard))
                lines.append(f"{'  ' * depth}- `{rel}/_map.md` [{cnt} nodes, own map] — "
                             f"{first_sentence(c['description'])}")
            else:
                lines.append(f"{'  ' * depth}- `{rel}/SKILL.md` — "
                             f"{first_sentence(c['description'])}")
                emit(c["children"], depth + 1)

    emit(children, 0)
    path = os.path.join(dir_path, "_map.md")
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    return written + [path]


def emit_node(node, indent):
    pad = " " * indent
    out = [
        f"{pad}- name: {json.dumps(node['name'])}",
        f"{pad}  path: {json.dumps(node['path'])}",
        f"{pad}  role: {node['role']}",
        f"{pad}  description: {json.dumps(node['description'])}",
        f"{pad}  skill_lines: {node['skill_lines']}",
        f"{pad}  resources: {node['resources']}",
    ]
    if node["children"]:
        out.append(f"{pad}  children:")
        for c in node["children"]:
            out.extend(emit_node(c, indent + 2))
    return out


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("root", help="meta-skill root or container of meta-skills")
    ap.add_argument("--check", action="store_true", help="validate only, write nothing")
    ap.add_argument("-o", "--output", default=None,
                    help="registry output path (default: <root>/_registry.yaml)")
    ap.add_argument("--shard", type=int, default=150,
                    help="max nodes per _map.md before a branch gets its own (default 150)")
    args = ap.parse_args()

    root = os.path.abspath(args.root)
    if os.path.isfile(os.path.join(root, "SKILL.md")):
        tops = [scan(root, os.path.basename(root))]
    else:
        tops = [scan(os.path.join(root, e), e) for e in sorted(os.listdir(root))
                if os.path.isfile(os.path.join(root, e, "SKILL.md"))]
    if not tops:
        sys.exit(f"error: no SKILL.md found under {root}")

    problems = []
    for n in tops:
        validate(n, problems)
    for level, where, msg in problems:
        print(f"{level:5s} {where}: {msg}")

    t = totals(tops)
    errors = sum(1 for p in problems if p[0] == "ERROR")
    print(f"\n{t['nodes']} nodes ({t['routers']} routers, {t['leaves']} leaves), "
          f"max depth {t['max_depth']}, {t['resource_files']} resource files — "
          f"{errors} errors, {len(problems) - errors} warnings")

    if not args.check:
        out_path = args.output or os.path.join(root, "_registry.yaml")
        lines = [
            "# Generated by registry.py — do not hand-edit.",
            f"# Regenerate: python scripts/registry.py <root> -o {os.path.basename(out_path)}",
            "nodes:",
        ]
        for n in tops:
            lines.extend(emit_node(n, 0))
        lines.append("totals:")
        for k in ("nodes", "routers", "leaves", "max_depth", "resource_files"):
            lines.append(f"  {k}: {t[k]}")
        with open(out_path, "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
        print(f"wrote {out_path}")
        # jump tables: root map covers everything; oversized branches shard
        if len(tops) == 1 and tops[0]["_dir"] == root:
            maps = write_map(root, tops[0]["name"], tops[0]["children"], args.shard)
        else:
            maps = write_map(root, os.path.basename(root), tops, args.shard)
        for m in maps:
            print(f"wrote {m}")

    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
