#!/usr/bin/env python3
"""Validate a meta-skill's nested-references tree; generate _registry.yaml + _map.md.

Model (see SKILL.md): ONE skill — the root SKILL.md, the only frontmatter in the
tree — over a nested references/ tree of plain .md nodes. Pointers are markdown
links [label](relative/path.md) (resolved relative to the containing file, then
relative to the skill root) and backticked root-relative paths with an extension
(e.g. `scripts/registry.py`). Validation is bidirectional: every node must be
reachable from the root by following pointers, and every pointer must resolve to
an existing file. A SKILL.md below root is an ERROR — a regression to the old
skill-routing model.

Usage:
  python registry.py <root>              # validate + write _registry.yaml + _map.md
  python registry.py <root> --check      # validate only, write nothing (also
                                         # reports stale generated outputs)
  python registry.py <root> -o out.yaml  # write registry elsewhere
  python registry.py <root> --shard 150  # max nodes per _map.md before a branch
                                         # gets its own (default 150)

<root> may be a single meta-skill (has its own SKILL.md) or a container
directory holding several top-level meta-skills.

Exit code 1 if any ERROR was found, else 0.
"""

import argparse
import json
import os
import re
import sys

MD_LINK = re.compile(r"\[([^]]*)\]\(([^)\s]+)\)")
BT_TOKEN = re.compile(r"`([^`\n]+)`")
PATHISH = re.compile(r"^[\w./-]+\.(md|py|sh|json|ya?ml|toml|txt|html?|css|js|ts|csv)$")
FENCE = re.compile(r"^\s*(?:```|~~~)")
PLACEHOLDER_MARKERS = ("todo", "tbd", "fixme", "placeholder", "description here", "lorem")


def parse_frontmatter(text):
    """Naive single-purpose YAML frontmatter parser: top-level `key: value`
    pairs with indented continuation lines folded in. {} if no valid block."""
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


def unfenced(text):
    """Drop fenced code blocks entirely — template/example content inside them
    is illustration, not pointers, and must not be validated."""
    out, in_fence = [], False
    for line in text.splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if not in_fence:
            out.append(line)
    return "\n".join(out)


def hidden_rel(rel):
    return any(seg.startswith((".", "_")) for seg in rel.split(os.sep) if seg)


def walk_files(root_dir, md_only=False):
    """Relative paths of files under root_dir, skipping hidden/_ segments."""
    found = []
    for dp, dns, fns in os.walk(root_dir):
        dns[:] = sorted(d for d in dns if not d.startswith((".", "_")))
        for fn in sorted(fns):
            if fn.startswith("."):
                continue
            if md_only and not fn.endswith(".md"):
                continue
            rel = os.path.relpath(os.path.join(dp, fn), root_dir)
            if not hidden_rel(rel):
                found.append(rel)
    return found


def resolve(target, base_dir, root, base="file"):
    """Resolve a pointer target to an absolute path, or None if external/absolute.
    Tries file-relative first (markdown semantics), then skill-root-relative —
    writers may use either form; the error is only when neither exists."""
    target = target.split("#", 1)[0].split("?", 1)[0].strip()
    if not target or "://" in target or target.startswith(("mailto:", "#", "<")):
        return None
    if os.path.isabs(target):
        return os.path.normpath(target)
    order = [(base_dir, root), (root, base_dir)] if base == "file" else [(root, base_dir), (base_dir, root)]
    first_inside = outside = None
    for first, _ in order:
        cand = os.path.normpath(os.path.join(first, target))
        if cand == root or cand.startswith(root + os.sep):
            # Existence-checked fallback (docstring contract; skills_tool.rs
            # resolve_pointer parity): a nonexistent file-relative candidate
            # must not shadow a valid root-relative one — writers mix both
            # styles, and returning the first inside-root path blindly turned
            # valid root-rel links from depth>0 nodes into false dangling
            # errors (observed in linkedin-marketing).
            if os.path.exists(cand):
                return cand
            if first_inside is None:
                first_inside = cand
        elif outside is None:
            outside = cand
    return first_inside if first_inside is not None else outside


def extract_pointers(text, base_dir, root):
    """[(kind, target, resolved_path_or_None)] for links + path-like backticks,
    outside fenced blocks. Inline code spans are illustrations for links (a
    backticked link example is not a pointer) but ARE the backtick pointers.
    kind: 'link' (authored pointer) | 'backtick'."""
    body = unfenced(text)
    no_inline_code = re.sub(r"`[^`\n]+`", "", body)
    out = []
    for m in MD_LINK.finditer(no_inline_code):
        r = resolve(m.group(2), base_dir, root, base="file")
        if r is not None:
            out.append(("link", m.group(2), r))
    for m in BT_TOKEN.finditer(body):
        t = m.group(1).strip()
        if "/" not in t or not PATHISH.match(t):
            continue  # bare filenames and prose are not pointers
        if any(seg.startswith("_") for seg in t.split("/")):
            continue  # generated (_map.md) / working state (_build/) — skip
        r = resolve(t, base_dir, root, base="root")
        if r is not None:
            out.append(("backtick", t, r))
    return out


def first_line_summary(path, cap=120):
    try:
        text = unfenced(open(path, encoding="utf-8").read())
    except OSError:
        return ""
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        s = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", s)  # link -> label text
        s = s.lstrip("> ").strip()
        return s if len(s) <= cap else s[: cap - 1].rstrip() + "…"
    return ""


def count_nodes(dir_path):
    return len(walk_files(dir_path, md_only=True))


def validate_top(root_dir, problems):
    """Validate one meta-skill rooted at root_dir. Returns (root_text, fm)."""
    where = os.path.basename(root_dir.rstrip(os.sep))
    skill_path = os.path.join(root_dir, "SKILL.md")
    root_text = open(skill_path, encoding="utf-8").read()
    fm = parse_frontmatter(root_text)

    # Rule 1: SKILL.md below root = regression to the routing model.
    for rel in walk_files(root_dir):
        if os.path.basename(rel) == "SKILL.md" and rel != "SKILL.md":
            problems.append(("ERROR", where,
                             f"stray SKILL.md at '{rel}' — only the root may be a "
                             "skill; nodes are plain .md files under references/"))

    # Root health: the only frontmatter in the tree.
    dirname = os.path.basename(root_dir.rstrip(os.sep))
    if fm.get("name", "") != dirname:
        problems.append(("ERROR", where,
                         f"frontmatter name '{fm.get('name', '')}' != directory name '{dirname}'"))
    desc = fm.get("description", "")
    if not desc:
        problems.append(("ERROR", where, "missing root description — the skill list cannot route without it"))
    else:
        if len(desc) < 60:
            problems.append(("WARN", where,
                             f"root description is only {len(desc)} chars — say what it does AND when to use it"))
        low = desc.lower()
        for marker in PLACEHOLDER_MARKERS:
            if marker in low:
                problems.append(("ERROR", where, f"root description reads like a placeholder ('{marker}')"))
                break
    root_lines = len(root_text.splitlines())
    if root_lines > 200:
        problems.append(("WARN", where,
                         f"root SKILL.md is {root_lines} lines (>200) — it pays rent on every "
                         "traversal; move procedure into reference files"))

    # Partition .md files: nodes under references/, auxiliary elsewhere.
    all_md = [rel for rel in walk_files(root_dir, md_only=True) if rel != "SKILL.md"]
    nodes = [rel for rel in all_md if rel.split(os.sep)[0] == "references"]
    aux = [rel for rel in all_md if rel.split(os.sep)[0] != "references"]

    # Bidirectional pointer walk: reachability from the root + dangling detection.
    reachable, dangling, queue = {"SKILL.md"}, [], ["SKILL.md"]
    while queue:
        cur = queue.pop()
        cur_abs = os.path.join(root_dir, cur)
        try:
            text = open(cur_abs, encoding="utf-8").read()
        except OSError as e:
            problems.append(("ERROR", where, f"cannot read '{cur}': {e}"))
            continue
        for kind, tgt, resolved in extract_pointers(text, os.path.dirname(cur_abs), root_dir):
            if not os.path.exists(resolved):
                dangling.append((cur, kind, tgt))
                continue
            if not (resolved == root_dir or resolved.startswith(root_dir + os.sep)):
                problems.append(("ERROR", where,
                                 f"pointer in '{cur}' escapes the skill: '{tgt}'"))
                continue
            rel = os.path.relpath(resolved, root_dir)
            if rel.endswith(".md") and not hidden_rel(rel) and rel not in reachable:
                reachable.add(rel)
                queue.append(rel)

    for cur, kind, tgt in sorted(dangling):
        level = "ERROR" if kind == "link" else "WARN"
        problems.append((level, where,
                         f"dangling {kind} in '{cur}': '{tgt}' does not exist "
                         "(links are validated strictly; backticked paths are hints)"))

    for rel in sorted(nodes):
        if rel not in reachable:
            problems.append(("ERROR", where,
                             f"unreachable node '{rel}' — no chain of links from SKILL.md "
                             "reaches it; add it to its index's routing table"))
    for rel in sorted(aux):
        if rel not in reachable:
            problems.append(("WARN", where,
                             f"'.md' outside references/ is unreferenced: '{rel}'"))

    # Node budgets.
    for rel in nodes:
        try:
            n = sum(1 for _ in open(os.path.join(root_dir, rel), encoding="utf-8"))
        except OSError:
            continue
        if n > 500:
            problems.append(("WARN", where,
                             f"'{rel}' is {n} lines (>500) — split it into a directory group "
                             "with an index file"))

    # Directory groups should carry an index file (contract rule 4).
    ref_dir = os.path.join(root_dir, "references")
    if os.path.isdir(ref_dir):
        for dp, dns, _ in os.walk(ref_dir):
            dns[:] = [d for d in dns if not d.startswith((".", "_"))]
            for d in dns:
                if d == "frameworks":
                    continue
                idx = os.path.relpath(os.path.join(dp, d + ".md"), root_dir)
                if not os.path.exists(os.path.join(root_dir, idx)):
                    problems.append(("WARN", where,
                                     f"directory group '{os.path.relpath(os.path.join(dp, d), root_dir)}/' "
                                     f"has no sibling index '{d}.md' — its routing table has nowhere to live"))

    # Unreferenced resources: non-.md files nobody mentions.
    joined = ""
    for rel in reachable:
        try:
            joined += open(os.path.join(root_dir, rel), encoding="utf-8").read() + "\n"
        except OSError:
            pass
    for rel in walk_files(root_dir):
        if rel.endswith(".md"):
            continue
        if os.path.basename(rel) not in joined:
            problems.append(("WARN", where,
                             f"resource '{rel}' is not referenced by any reachable file — "
                             "dead weight or missing pointer"))
    return root_text, fm, nodes


# ---------------------------------------------------------------- generation

def emit_tree(dir_path, skill_root, shard, maps_out, depth=0, lines=None):
    """Emit map lines for dir_path's contents; recurse, sharding oversized dirs.
    Paths are relative to skill_root so every map reads the same way."""
    if lines is None:
        lines = []
    for entry in sorted(os.listdir(dir_path)):
        if entry.startswith((".", "_")):
            continue
        p = os.path.join(dir_path, entry)
        rel = os.path.relpath(p, skill_root)
        if os.path.isdir(p):
            cnt = count_nodes(p)
            if cnt > shard:
                build_map(p, entry, skill_root, shard, maps_out)
                lines.append(f"{'  ' * depth}- `{rel}/_map.md` [{cnt} nodes, own map]")
            else:
                lines.append(f"{'  ' * depth}- `{rel}/`")
                emit_tree(p, skill_root, shard, maps_out, depth + 1, lines)
        else:
            s = first_line_summary(p)
            lines.append(f"{'  ' * depth}- `{rel}`" + (f" — {s}" if s else ""))
    return lines


def build_map(dir_path, title, skill_root, shard, maps_out, dest=None):
    """Build one _map.md covering dir_path's contents and queue it for writing."""
    lines = [f"# {title} — node map (generated by registry.py, do not hand-edit)", ""]
    emit_tree(dir_path, skill_root, shard, maps_out, depth=0, lines=lines)
    content = "\n".join(lines) + "\n"
    maps_out.append((dest or os.path.join(dir_path, "_map.md"), content))
    return content


def generate_maps(tops, shard):
    """[(path, content)] for every _map.md this run would write: one root map
    per skill (placed at the skill root) plus a sharded map per oversized dir."""
    out = []
    for top_abs, top_rel, _, fm, _ in tops:
        ref = os.path.join(top_abs, "references")
        if os.path.isdir(ref):
            build_map(ref, fm.get("name", top_rel) or top_rel, top_abs, shard, out,
                      dest=os.path.join(top_abs, "_map.md"))
    return out


def registry_text(tops, root_dir, totals, out_name):
    lines = [
        "# Generated by registry.py — do not hand-edit.",
        f"# Regenerate: python scripts/registry.py <root> -o {out_name}",
        "nodes:",
    ]

    def emit(node_rel, indent, name, role, description, sk_lines, resources, children):
        pad = " " * indent
        out = [
            f"{pad}- name: {json.dumps(name)}",
            f"{pad}  path: {json.dumps(node_rel)}",
            f"{pad}  role: {role}",
            f"{pad}  description: {json.dumps(description)}",
            f"{pad}  skill_lines: {sk_lines}",
            f"{pad}  resources: {resources}",
        ]
        if children:
            out.append(f"{pad}  children:")
            for c in children:
                out.extend(c)
        return out

    def node_entry(base, rel):
        abs_p = os.path.join(base, rel)
        name = os.path.splitext(os.path.basename(rel))[0]
        try:
            n = sum(1 for _ in open(abs_p, encoding="utf-8"))
        except OSError:
            n = 0
        return emit(rel, 6, name, "node", first_line_summary(abs_p), n, 0, [])

    def dir_entry(base, rel):
        abs_p = os.path.join(base, rel)
        children = []
        for entry in sorted(os.listdir(abs_p)):
            if entry.startswith((".", "_")):
                continue
            child_rel = os.path.join(rel, entry)
            if os.path.isdir(os.path.join(base, child_rel)):
                children.append(dir_entry(base, child_rel))
            elif entry.endswith(".md"):
                children.append(node_entry(base, child_rel))
        return emit(rel, 6, os.path.basename(rel), "group",
                    first_line_summary(os.path.join(base, rel + ".md")) if
                    os.path.exists(os.path.join(base, rel + ".md")) else "",
                    0, len(children), children)

    for top_abs, top_rel, root_text, fm, nodes in tops:
        children = []
        ref = os.path.join(top_abs, "references")
        if os.path.isdir(ref):
            for entry in sorted(os.listdir(ref)):
                if entry.startswith((".", "_")):
                    continue
                if os.path.isdir(os.path.join(ref, entry)):
                    children.append(dir_entry(top_abs, os.path.join("references", entry)))
                elif entry.endswith(".md"):
                    children.append(node_entry(top_abs, os.path.join("references", entry)))
        res = [f for f in walk_files(top_abs) if not f.endswith(".md")]
        lines.extend(emit(top_rel, 0, os.path.basename(top_abs), "root",
                          fm.get("description", ""), len(root_text.splitlines()),
                          len(res), children))
    lines.append("totals:")
    for k in ("roots", "nodes", "groups", "max_depth", "resources"):
        lines.append(f"  {k}: {totals[k]}")
    return "\n".join(lines) + "\n"


def totals_of(tops):
    t = {"roots": len(tops), "nodes": 0, "groups": 0, "max_depth": 0, "resources": 0}
    for top_abs, _, _, _, nodes in tops:
        t["nodes"] += len(nodes)
        t["resources"] += len([f for f in walk_files(top_abs) if not f.endswith(".md")])
        ref = os.path.join(top_abs, "references")
        if os.path.isdir(ref):
            for dp, dns, _ in os.walk(ref):
                dns[:] = [d for d in dns if not d.startswith((".", "_"))]
                t["groups"] += len(dns)
                d = os.path.relpath(dp, ref).count(os.sep) + 1
                t["max_depth"] = max(t["max_depth"], d)
    return t


def check_stale(path, expected, problems, where, label):
    if not os.path.exists(path):
        return
    try:
        on_disk = open(path, encoding="utf-8").read()
    except OSError:
        return
    if on_disk != expected:
        problems.append(("WARN", where,
                         f"generated {label} is stale — rerun registry.py to regenerate"))


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
        roots = [root]
    else:
        roots = [os.path.join(root, e) for e in sorted(os.listdir(root))
                 if os.path.isfile(os.path.join(root, e, "SKILL.md"))
                 and not e.startswith((".", "_"))]
    if not roots:
        sys.exit(f"error: no SKILL.md found under {root}")

    problems = []
    tops = []
    for r in roots:
        rel_top = os.path.basename(r.rstrip(os.sep))
        root_text, fm, nodes = validate_top(r, problems)
        tops.append((r, rel_top, root_text, fm, nodes))

    for level, where, msg in problems:
        print(f"{level:5s} {where}: {msg}")

    t = totals_of(tops)
    errors = sum(1 for p in problems if p[0] == "ERROR")
    print(f"\n{t['roots']} skill(s), {t['nodes']} reference nodes, {t['groups']} groups, "
          f"max depth {t['max_depth']}, {t['resources']} resources — "
          f"{errors} errors, {len(problems) - errors} warnings")

    if not args.check:
        out_path = args.output or os.path.join(root, "_registry.yaml")
        with open(out_path, "w", encoding="utf-8") as f:
            f.write(registry_text(tops, root, t, os.path.basename(out_path)))
        print(f"wrote {out_path}")
        for path, content in generate_maps(tops, args.shard):
            with open(path, "w", encoding="utf-8") as f:
                f.write(content)
            print(f"wrote {path}")
    else:
        # staleness: compare against exactly what a write run would produce
        out_path = args.output or os.path.join(root, "_registry.yaml")
        expected = registry_text(tops, root, t, os.path.basename(out_path))
        check_stale(out_path, expected, problems, "registry", "_registry.yaml")
        for path, content in generate_maps(tops, args.shard):
            check_stale(path, content, problems, "registry", "_map.md")
        for level, where, msg in problems:
            if msg.startswith("generated"):
                print(f"{level:5s} {where}: {msg}")
        errors = sum(1 for p in problems if p[0] == "ERROR")

    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
