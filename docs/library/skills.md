# Skills — the knowledge units

Owns: the skills system — discovery, the security guard, usage telemetry,
the marketplace, bundles, and SOPs.
Not for: how skills inject into the prompt
([context-injection.md](context-injection.md)), the background review
that writes them ([background-review.md](background-review.md)).

Source: `crates/operant-core/src/skills.rs` (SkillManager),
`crates/operant-core/src/skills_guard.rs` (the scanner),
`crates/operant-core/src/skill_usage.rs`,
`crates/operant-core/src/skill_marketplace.rs`,
`crates/operant-core/src/agent/skill_bundle.rs`,
`crates/operant-core/src/sops.rs`,
`crates/operant-core/src/tools/skills_tool.rs`,
`crates/operant-cli/src/cmd_skills.rs`.

## The unit

A skill is a directory with a `SKILL.md` carrying YAML front matter —
the same shape as upstream operant-agent. Pools, in lookup order:

1. the operator's own `~/.operant/skills/`,
2. **extra roots** — integration pools (ponytail installs under
   `~/.operant/integrations/ponytail-vX.Y.Z/`, never inside the
   operator's pool), with the operator's pool WINNING name collisions
   and delete refused for vendor-managed skills.

Progressive disclosure: tier 1 lists metadata only; tiers 2-3 load full
content + supporting files on view/invocation.

## The guard (`skills_guard.rs`)

Every skill downloaded from a registry passes the scanner before
installation: regex-based static analysis for known-bad patterns (data
exfiltration, prompt injection, destructive commands, persistence) plus
a trust-aware install policy — `Dangerous` refuses the install (keeping
the prior version), `Caution` installs with a warning. The guard is the
reason the ponytail content integration could ship: content packages
get the same scanning door binaries do.

## Usage telemetry (`skill_usage.rs`)

Per-skill usage metadata in a companion JSON sidecar (never frontmatter —
operational telemetry stays out of user-authored content), atomic
writes. Feeds the curator's stale-detection and the TUI's suggestion
ordering.

## The marketplace (`skill_marketplace.rs`)

Remote registry index fetch + search + install + version comparison —
the piece that was missing while the guard advertised "every skill
downloaded from a registry passes through this scanner" (gap B8,
closed): fetch, search, install, and version comparison against
installed skills.

## Bundles (`agent/skill_bundle.rs`)

A small YAML file in `~/.operant/skill-bundles/` names skills to load
together; `/<bundle-name>` loads every referenced skill's content into
one user message — aliases over sets.

## SOPs (`sops.rs`)

The operator's playbooks: TOML files in `~/.operant/sops/` (name,
description, steps). The same files the agent's `sop` tool reads
(iter-1126) — `operant sop list` shows exactly what the agent can
follow.

## Availability gating — and the one gap the codex contrast named

`SkillFrontmatter` (`crates/operant-core/src/skills.rs:18-27`) declares
`prerequisites_env` and `prerequisites_commands`; `is_available` (`:218-245`) refuses a
skill whose platform, env vars, or required commands are missing.

**There is no `prerequisites_tools` field.** A skill cannot declare "this needs
`web_search` to work". The 2026-10-31 codex contrast found the equivalent in the
codex tree's skills model (a 15-LOC struct) and recorded it as the one row in that
family worth importing — wired to the tool `is_available()` the registry already has
(`crates/operant-core/src/tools.rs:325`, called at `:640`, `:655`, `:686`, `:728`), so
no new mechanism is needed, only the declaration and the check.

Row: `docs/audit/2026-10-31-codex-architecture-deepen/03-skills.md` and
`docs/plans/2026-10-31-codex-portability-the-twelve-rows-the-deep-dive-adds.md` §7.

## Intention

Skills are how the agent gets *cheaper* at recurring work: a skill is
prompt-shaped knowledge that loads on demand instead of riding every
turn. The system's core tension is trust — skills are external content
that becomes instructions — and the guard + sidecar telemetry + refused
deletes are the three pieces that keep the operator in control of that
trust. The `sop` surface closes the loop the other way: the operator
writes the playbook, the agent follows it verbatim.
