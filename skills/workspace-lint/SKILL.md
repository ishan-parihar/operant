---
name: workspace-lint
description: 'COMPATIBILITY ALIAS. Working-tree structure enforcement now lives in `project-genome`'s `ops` module — layout rules (allowed root files, allowed suffixes, required dir files), orphaned files, tracked bulk, dead links, escaping symlinks. Triggers: "organize files", "clean up directory", "where should I put this", "structure the workspace", "audit workspace", "orphaned files at the root", "stray build artifacts". Use `project-genome` and read its references/workspace-lint.md; this file exists only so old paths and links do not break.'
metadata:
  operant: {}
---

# workspace-lint → project-genome (`ops`)

This skill was absorbed into **`project-genome`** on 2026-11-02, by the
meta-skill discipline: one skill per class of work. The class is *"is this
working tree still shaped the way I declared it"*, and the kernel already
owned it as the `ops` module with a lint catalogue behind it.

## What to do when this skill triggers

1. Load `project-genome`.
2. Read `project-genome/references/workspace-lint.md` — it carries the full
   mapping from this skill's old config keys to the `ops` gates, the
   configuration block to write, and the two things that genuinely changed.
3. Run `genome lint` in the repo. It reports every structural violation by
   code (E301/E303/E305/E306/E307) and names the command that fixes each.
4. Move the project's `workspace-lint.yaml` contents into `genome.toml`'s
   `[ops]` section, one repo at a time. The config stays per-project; it
   moved home, it did not become global.

## What changed, and what did not

| Was | Now |
|---|---|
| `workspace-lint.yaml` | `[ops]` in `genome.toml` — `root_files`, `dir_suffixes`, `dir_requires`, `bulk_allow`, `owner_globs` |
| `scripts/workspace_lint.py` | `genome lint` (stdlib, no PyYAML) |
| `references/workspace-lint-patterns.md` | `project-genome/references/ops.md` + `references/lint-rules.md` |
| `--fix` (moved files for you) | **removed, deliberately** — every gate fails closed and names the command; the operator applies it |

The `--fix` loss is the one behavioural change worth flagging. Auto-moving
files breaks paths other agents mid-run are holding, so the gates here
report and never mutate.

The bundled `scripts/workspace_lint.py` still works and still lives in this
repo; nothing was deleted from operant's tree. It is superseded, not removed.
