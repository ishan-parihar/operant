# Skills — discovery, guarding, invocation

Owns: how skills are discovered (roots, walk, resolver), the skills guard
(ThreatPattern table, scan verdict, install policy), the in-loop skill tools
(`skills_list` / `skill_view` / `skill_manage`) and their write guards, the
curator lifecycle (state, stale report vs archive), skill bundles, skill
preprocessing, and the skill CLI (`operant skills`, `operant sop`).
Not for: the **content** scanning of skills' *files* is owned here; the
**gateway** / channel plumbing is in the gateway runner; the **Ponytail**
posture/injection surface lives on `docs/library/README.md`'s `ponytail.md`
(if it exists) and in `crates/operant-core/src/ponytail.rs`; the **guard stack**
behind the tools (blast radius, approval ladder, write origin) is in
[security.md](security.md); the **org/budget** governance of the seats that
run with skills is in [org.md](org.md).

Source: `crates/operant-core/src/skills.rs` (`SkillRoots`,
`SkillRoots::resolve`, `walk_skill_dirs`, `SkillManager`, `find_skill_in_roots`,
`SkillOrigin`), `crates/operant-core/src/skills_guard.rs` (`THREAT_PATTERNS`,
`SCAN_SKILL`, `install_policy_verdict`, `scan_skill`, `should_allow_install`),
`crates/operant-core/src/tools/skills_tool.rs` (`SkillsTool`, `SkillViewTool`,
`SkillManageTool`, `review_write_guard`, `REVIEW_MUTATING_ACTIONS`),
`crates/operant-core/src/curator/mod.rs` (`CuratorEngine`, `CuratorState`,
`CuratorEngine::maybe_run_curator`), `crates/operant-core/src/skill_usage.rs`
(`SkillUsageTracker`, `LifecycleState`), `crates/operant-core/src/agent/skill_bundle.rs`,
`crates/operant-core/src/agent/skill_preprocessing.rs`,
`crates/operant-core/src/sops.rs` + `crates/operant-core/src/tools/sop_tool.rs`,
`crates/operant-core/src/skill_marketplace.rs` (registry index fetch/install),
`crates/operant-core/src/skill_mentions.rs` (the inline `$skill` mention form),
`crates/operant-cli/src/cmd_skills.rs` (`operant skills`,
`seed_bundled_skills`), `crates/operant-cli/src/skill_roots.rs` (the one vendor
root list), `crates/operant-core/src/tools/builtin.rs` (where the three tools
are registered), `crates/operant-cli/src/main.rs` (the agent's skill wiring).

Counts below were read from the code on 2026-11-17 at `449c9dd2`.

## Contents

- [The shape in one sentence](#the-shape-in-one-sentence)
- [The roots — one list, one walk, one skip rule](#the-roots-one-list-one-walk-one-skip-rule)
- [The skill tools — three `OperantTool`s](#the-skill-tools-three-operanttools)
- [The guard (`skills_guard.rs`)](#the-guard-skills_guardrs)
- [The marketplace (`skill_marketplace.rs`)](#the-marketplace-skill_marketplacers)
- [Invocation and preprocessing](#invocation-and-preprocessing)
- [Bundles (`agent/skill_bundle.rs`)](#bundles-agentskill_bundlers)
- [SOPs (`sops.rs` + `tools/sop_tool.rs`)](#sops-sopsrs-toolssop_toolrs)
- [Curator (`curator/mod.rs` + `skill_usage.rs`)](#curator-curatormodrs-skill_usagers)
- [Configuration](#configuration)
- [Failure modes](#failure-modes)

## The shape in one sentence

A **skill** is a directory containing `SKILL.md` with YAML frontmatter; the
**manager** is a load-once HashMap index; the **roots** are one resolved list
(`SkillRoots`) every surface takes; the **tools** are three registered
`OperantTool`s; the **guard** is a separate scanner (`skills_guard`) the tools
and the installer call; and the **curator** is a background pass that archives
idle skills and reports stale ones without ever persisting a `stale` state.

## The roots — one list, one walk, one skip rule

### `SkillRoots::resolve` (`skills.rs:221`)

The ONE resolver, iter-2254. Inputs: the config's `SkillsSettings`, a list of
vendor roots, the session workspace (optional), and a Holon override (only
when the caller is in a Holon run).

```text
primary pool (config.root_dir) [or holon_override]
  → config.extra_roots (in order)
  → project root (resolved from `project_root` setting; default: `<workspace>/skills`)
  → vendor roots (ponytail + plugin skill dirs)
```

Precedence is order. The primary pool wins a name collision against every
extra root (`load_root` scans primary first; `skills` map insertion is
first-wins). `config.extra_roots` beats the project root; the project root
beats vendor. A Holon run stops after the primary (`skills.rs:236`): the
brief's library only.

Each root goes through `push_root` (`skills.rs:272`):
canonicalize → dedupe → `sensitive_path_reason` (refused: credential stores
like `~/.ssh/`, `~/.aws/`, `~/.config/gh/`) → metadata probe. A refused root
becomes a `RootIssue` (`reason: "sensitive_path: ..."`, `"missing"`,
`"unreadable"`, `"not_a_directory"`), NOT a silent drop. `RootIssue`s surface
on `operant doctor`.

The walk is `walk_skill_dirs(root, SKILL_ROOT_WALK_LIMIT)` (`skills.rs:329`),
depth-first, sorted, with `SKILL_ROOT_WALK_LIMIT = 20_000` (`skills.rs:209`).
Skip rule: `is_skipped_walk_name` (`skills.rs:412`) — names starting `.`
(hidden: `.git`, `.archive`, `.curator`, `.hub`) or `_`, or the literal
`node_modules` / `target` / `__pycache__`. A dead symlink becomes a
`RootIssue { reason: "dead_symlink" }` and is skipped; it never becomes a
load error. Truncation is a flag on the manager / `SkillRoots`, never silent.

`find_skill_in_roots(roots, name)` (`skills.rs:129`) is the OTHER way a skill
is found: no manager, no HashMap, first root wins, `find_skill_in_root`
(`skills.rs:418`) tries a direct root-joined name directory first, then recurses
(again honoring the skip rule), matching directory name OR frontmatter
`name:`. The name guard `is_plain_relative_path` (`skills.rs:473`) refuses
anything that is not a simple relative path.

### `SkillManager` (`skills.rs:481`)

Fields: `skills_dir: PathBuf`, `extra_roots: Vec<PathBuf>`, `skills:
HashMap<String, Skill>` (keyed by frontmatter name, directory name, AND
category-qualified alias), `issues: Vec<RootIssue>`, `truncated: bool`.

`load_all` (`skills.rs:548`) is the canonical loader: clears the map, walks
the primary root with `SkillOrigin::OperatorPool`, then each extra root with
`SkillOrigin::Integration` (and `Project` for the project root when it is
wired through a caller that passes it as an extra root). A missing integration
dir is a normal state and is skipped silently (`skills.rs:578`), because the
resolver has already recorded it if the operator named it. `load_skill`
parses `SKILL.md` (frontmatter or the file defaults); a parse error is logged
and the skill is skipped (`Err(_) => continue`, `skills.rs:654`).

Shadowing is by **skill name**, not by directory name (`skills.rs:637-651`): a
pool skill at `a/shared` named `alpha` and an integration skill at `b/shared`
named `beta` keep both; a same-named skill across roots keeps the earlier
root's. Aliases (the directory name and the `category/skill` qualified path)
are best-effort `or_insert_with` / first-wins — but only for root names that
exist, so a nested category dir still wins its own alias.

`SkillManager::list` / `list_available` (`skills.rs:677`, `:712`) — one row
per SKILL (not per alias — the map is keyed multiple times). `is_available`
(`skills.rs:734`) checks `platforms` (matches `current_platform()` if
non-empty), `prerequisites_env` (every env var must be set), and
`prerequisites_commands` (every command must be on PATH via `which::which`).
No caching — the check runs on every call because the process env and PATH
can change under a long-running gateway (RT-175).

### `SkillOrigin` (`skills.rs:35`)

`OperatorPool` | `Integration` | `Project`. The enum exists because
`create` / `delete` must only ever touch the operator's pool: the manager
passes it per root and the mutators (below) refuse `Integration` and
`Project` origins.

## The skill tools — three `OperantTool`s

Registered in `tools/builtin.rs:154-165` (order: `SkillsTool` →
`SkillViewTool` → `SkillManageTool` → `SopTool`). All three are registered
unconditionally when `skills_dir` is present — the manager is built by the
agent, the tools read it.

### `skills_list` (`SkillsTool`, `tools/skills_tool.rs:1398`)

Returns a `(name, description, category)` summary for every available skill,
with optional `tree=true` for a category-ordered view. It reads the
**manager's** list (the same one the frozen prefix advertises), so the tools
and the prompt cannot disagree. When the manager is empty the result is a
helpful "No skills found. Skills directory created." message, not an error.

### `skill_view` (`SkillViewTool`, `tools/skills_tool.rs:1564`)

Loads the full SKILL.md body (frontmatter stripped), plus `references/*`
supporting files when `file_path` names one. Not found → a tool error with
the directory it searched. Origin `background_review` triggers a
read-recording side effect (the review write guard below keys on it).

### `skill_manage` (`SkillManageTool`, `tools/skills_tool.rs:1805`)

Seven actions, dispatched at `:1851`:

| Action | Effect |
|---|---|
| `create` | new skill dir + SKILL.md (frontmatter validated: name matches dir name) |
| `edit` | full rewrite of SKILL.md |
| `patch` | find-replace on SKILL.md |
| `delete` | remove the skill dir |
| `write_file` | write a supporting file (e.g. a `references/` note) |
| `remove_file` | remove a supporting file |
| `generate_map` | write / regenerate `_map.md` (meta-skill routing index) |

`create` / `edit` / `patch` / `write_file` / `remove_file` all go through
`write_screened` → `scan_skill(dir, "agent-created")` → `should_allow_install`.
An unscannable result is refused; a `Dangerous` verdict on an agent-created
skill is `Ask`, and an unattended agent write has no one to ask, so it is
refused (the header doc at `tools/skills_tool.rs:340`). A failed rollback is
reported in the error, not discarded (RT-426, `tools/skills_tool.rs:353`).

Frontmatter validation on `create` / `edit`: the `name:` must equal the
directory name, the description must be ≥ `MIN_DESCRIPTION_CHARS = 60` and
not a placeholder substring; the root SKILL.md of a meta-skill (a skill with
a `references/` tree) is read on every traversal, so its budget is
`MAX_ROUTER_BODY_LINES = 200` (leaf skills get `MAX_LEAF_BODY_LINES = 500`).
Content size is capped at `MAX_SKILL_CONTENT_CHARS = 100_000` (~36k tokens);
a supporting file at `MAX_SKILL_FILE_BYTES = 1 MiB`.

`is_pinned` reads the curator usage.json through the tracker — the same file
the curator writes. `record_usage` (`tools/skills_tool.rs:2451`) bridges
every mutating action to `SkillUsageTracker::with_exclusive_lock`: `create` →
`record_created(name, is_bg)`, `delete` → `remove(name)`, `patch|edit|write_file|remove_file`
→ `bump_patch(name)`. `generate_map` writes the map and records nothing (it
does not change usage).

### The review write guard (`tools/skills_tool.rs:187`)

`REVIEW_MUTATING_ACTIONS` = `["edit", "patch", "delete", "write_file",
"remove_file", "create"]` (6 entries). `review_write_guard(name, skill_dir,
action)` runs only when `is_background_review()` is true (the
`TASK_ORIGIN` task-local from `write_origin.rs`, [security.md](security.md)):

1. `create` colliding with a protected name → `Blocked`. The prefixes are
   `["operant", "hermes-agent", "hermes-dev", "claw-dev"]`
   (`PROTECTED_SKILL_PREFIXES`, `:68`).
2. A hub-installed skill (`_hub` marker) → `Blocked`.
3. A mutation of an existing skill that was not first read via `skill_view`
   → `Blocked` (the read-before-modify rule; `create` is exempt because the
   skill does not exist yet).
4. `generate_map` is exempt from the action list but still subject to
   `is_protected_skill` via the generic name check.

## The guard (`skills_guard.rs`)

A pure static-analysis module: regex in, findings out, no network, no
execution.

### `skills_guard.rs` — `THREAT_PATTERNS`

`LazyLock<Vec<ThreatPattern>>`, **120** `tp!()` entries counted 2026-11-17.
Each `ThreatPattern` is `{regex, pattern_id, severity, category, description}`.
The histogram by category:

| Category | Count | Severity split |
|---|---|---|
| `exfiltration` | 24 | Critical 11, High 13 |
| `injection` | 19 | Critical 9, High 8, Medium 2 |
| `persistence` | 12 | Critical 4, High 1, Medium 7 |
| `obfuscation` | 14 | Critical 1, High 7, Medium 5, Low 1 |
| `supply_chain` | 10 | Critical 3, Medium 7 |
| `network` | 9 | Critical 3, High 4, Medium 2 |
| `destructive` | 8 | Critical 6, High 1, Medium 1 |
| `execution` | 6 | High 4, Medium 2 |
| `credential_exposure` | 6 | Critical 6 |
| `traversal` | 5 | Critical 1, High 2, Medium 2 |
| `privilege_escalation` | 5 | Critical 3, High 2 |
| `mining` | 2 | Critical 1, Medium 1 |

Overall severity: Critical **48**, High **42**, Medium **29**, Low **1**
(total **120**). The lone Low is the only one in the table; the
distribution is what the install policy keys on.

`INVISIBLE_CHARS` (`skills_guard.rs:1181`) is a separate `LazyLock<HashSet>` of
**17** Unicode code points: zero-width space/joiners, the bidi control
characters, and the isolate marks. `scan_file_inner` checks each line for
them and emits one `Medium` finding per line (`"invisible unicode character
… (possible text hiding/injection)"`).

### The scan (`skills_guard.rs:1342`)

Input: a path (directory or file) and a `source` string (e.g.
`"community"`, `"registry"`, `"agent-created"`, or a repo path). The source
is resolved through `resolve_trust_level` (`:1786`) — `"agent-created"` →
`AgentCreated`, `"official"` / `"official/..."` → `Builtin`, a
`skills-sh/` / `skills.sh/` / `skils-sh/` / `skils.sh/` prefix is stripped,
`TRUSTED_REPOS` (`"openai/skills"`, `"anthropics/skills"`) match as a path
prefix (RT-397: `openai/skills-evil` is NOT trusted), everything else is
`Community`.

The scan does three things in order:

1. **Structural check** (`check_structure`) — file count vs
   `MAX_FILE_COUNT = 50` (`:242`), total size vs `MAX_TOTAL_SIZE = 1 MiB`
   (`:244`), single-file size vs `MAX_SINGLE_FILE_SIZE = 256 KiB`
   (`:246`), a binary-extension check against
   `SUSPICIOUS_BINARY_EXTENSIONS` (12 entries: `.exe .dll .so .dylib .bin
   .dat .com .msi .dmg .app .deb .rpm`, `:266`), and a symlink check. A file
   whose extension is not in `SCANNABLE_EXTENSIONS` (23 entries: `.md .txt
   .py .sh .bash .js .ts .rb .yaml .yml .json .toml .cfg .ini .conf .html
   .css .xml .tex .r .jl .pl .php`, `:260`) is skipped for regex scanning
   (except `SKILL.md`, which is always scanned).
2. **Pattern scan** — every scannable file is read; `THREAT_PATTERNS` is
   matched line-by-line; each hit is a `SecurityFinding` with the pattern id,
   severity, category, file, line, and a truncated snippet.
3. **Invisible-char scan** — as above.

The result is `determine_verdict(&findings)` (`:1849`): **any Critical →
`Dangerous`**, else any finding → `Caution`, else `Safe`. There is no
`Info` verdict. A path that is neither a directory nor a regular file
(missing, dangling symlink, special file) yields a single `Medium`
`scan_gap` finding (`"unscannable_skill_path"`, RT-398) rather than a
false `Safe`.

### The install policy (`skills_guard.rs:225`)

A 4×3 matrix, `(trust_level, scan_verdict) → Verdict`:

| | Safe (0) | Caution (1) | Dangerous (2) |
|---|---|---|---|
| `Builtin` | Allow | Allow | Allow |
| `Trusted` | Allow | Allow | Block |
| `Community` | Allow | Block | Block |
| `AgentCreated` | Allow | Allow | Ask |

`should_allow_install(result, force)` (`:1409`) is the public wrapper. A
`Block` verdict for a `Community` / `Trusted` skill returns
`(Some(false), reason)` regardless of `force`; a `Community` `Dangerous` is
also non-overridable. `force` only upgrades `AgentCreated` `Dangerous`
from `Ask` to `Allow` (the rationale: an agent-created skill has a human in
the loop at creation, so `--force` is the operator overriding their own
agent's draft).

### Scan call sites (2026-11-17)

| Site | Trust source | On refusal |
|---|---|---|
| `operant skills install <name>` (`cmd_skills.rs:733, 850`) | `"community"` (or the tap name) | install aborted, exit non-zero |
| `skill_manage` writes (`tools/skills_tool.rs:348`) | `"agent-created"` | write rolled back, reason in tool error (RT-426) |
| marketplace install (`skill_marketplace.rs:376`) | `"registry"` | install aborted, partial file deleted |
| content-integration update (`binary_updater.rs:767`) | `guard_source` (e.g. `"community:DietrichGebert/ponytail"`) | update refused, previous version kept |

## The marketplace (`skill_marketplace.rs`)

The registry is a flat JSON array of `SkillRegistryEntry` at a configurable
URL (`DEFAULT_REGISTRY_URL` at `:52`,
`https://raw.githubusercontent.com/ishan-parihar/operant/main/skill-registry/index.json`; the
`OPERANT_SKILL_REGISTRY` env var overrides). The index is fetched lazily,
cached to `~/.operant/skill-registry.json` with `CACHE_TTL = 1 hour` (`:57`),
and re-fetched if expired or missing. Each entry carries `name`,
`description`, `version`, `author`, `license`, `tags`, `category`, `homepage`,
`download_url`, and an optional `checksum` (sha256). An entry with a
checksum is verified before install; a mismatch refuses the install.

The CLI surface is `MarketCommand` (`cmd_skills.rs`): `Search`, `Install
{ name, force }`, `List`, `Refresh`. `TapCommand` manages sources.
`seed_bundled_skills` (`cmd_skills.rs:257`) copies the bundled skill pool
(dev builds: repo `skills/`; production: the operator's packaged skills dir,
`OPERANT_BUNDLED_SKILLS_DIR` override) into the user skills directory,
recursively, honoring the walk-skip rule (RT-197: a bundled install must not
seed a `.git` directory the loader later skips).

## Invocation and preprocessing

A skill is invoked as a **user message**, not a system prompt. Three entry
points build that message:

- `build_skill_invocation_message` (`agent/skill_preprocessing.rs:280`) —
  the default, resolves against the platform home + vendor roots.
- `build_skill_invocation_message_in` (`:319`) — resolves against an explicit
  `skills_dir` (the operator's `config.skills.root_dir`), with vendor roots
  and the session id.
- The inline `$skill` mention form (`skill_mentions.rs`) — a message that
  names a skill mid-sentence ("apply `$ponytail` to this diff") expands into
  the SAME invocation message `/skill <name>` builds, so one code path serves
  the TUI composer, the gateway channels and the agent's own lane.

The body is the SKILL.md with frontmatter stripped, passed through
`preprocess_skill_content` (`skill_preprocessing.rs`):

1. **Template vars** (`substitute_template_vars`, `:95`) — `${OPERANT_SKILL_DIR}`
   and `${OPERANT_SESSION_ID}` are replaced when the caller knows them; an
   unknown token is left literal (so the author can spot it). Gated by
   `[skills] template_vars` (default `true`).
2. **Inline shell** (`` expand_inline_shell ``, `:130`) — `` !`cmd` ``
   snippets are executed at load time, CWD = the skill dir, stdout trimmed
   and capped at `INLINE_SHELL_MAX_OUTPUT = 4000` chars, killed at
   `inline_shell_timeout` seconds (default 10, 1-second floor). Gated by
   `[skills] inline_shell` (default `false` — off unless opted in). A failed
   snippet becomes an `[inline-shell error: …]` / `[inline-shell timeout …]`
   marker in the body, never a raised error (one bad snippet cannot wreck
   the whole turn; R47-3 fixed a discarded timeout that hung the agent).

The invocation message itself is a `[IMPORTANT: The user has invoked the
"<name>" skill …]` header plus the preprocessed body. The same path builds
the message for a slash command (`/skill <name>`), a `$name` mention in the
TUI / gateway, and a bundle expansion — one message shape, one curator
telemetry write.

## Bundles (`agent/skill_bundle.rs`)

A bundle is a YAML file in `~/.operant/skill-bundles/*.yaml` (env override
`OPERANT_BUNDLES_DIR` for tests). Shape: `name`, `slug`, `description`,
`skills: [name, …]`, `instruction`. Invoking `/bundle-name` loads every
referenced skill's body into ONE user message (the same path as a single
skill), with `instruction` prepended. `build_bundle_invocation_message`
(`:351`) returns `(message, loaded, missing)` — missing skills are reported,
not silently dropped. Bundle dispatch wins over a same-named skill
(`skill_bundle.rs:25`).

## SOPs (`sops.rs` + `tools/sop_tool.rs`)

An SOP is a TOML file in `~/.operant/sops/<name>.toml` with `name`,
`description`, `steps`. `discover_sops` (`sops.rs:38`) sorts by name and
keeps files that do not parse (with a placeholder description) so the
operator can see and fix them. The `sop` tool (`tools/sop_tool.rs`) is
registered with the builtins (`builtin.rs:168`): with no `name` it lists the
available SOPs; with a `name` it returns the file content. The CLI mirrors
it (`operant sop list | validate <name> | show <name>`). Before iter-1126 the
directory was read by the CLI alone; the agent could not see the operator's
playbooks.

## Curator (`curator/mod.rs` + `skill_usage.rs`)

The curator is the skill lifecycle manager. It is a background pass that
runs from the prompting loop (`agent/prompting.rs:877`, the
`maybe_run_curator` branch) and from `operant curator run`. Its state is
`state.json` inside the skills-dir `.curator/` directory; its usage
telemetry is `usage.json` in the same directory (`SkillUsageTracker`).

`CuratorState` (`curator/mod.rs:191`) defaults:

| Field | Default | Meaning |
|---|---|---|
| `enabled` | `true` | the curator runs at all |
| `paused` | `false` | an operator pause switch |
| `interval_hours` | `24` | minimum wall-clock between passes |
| `stale_after_days` | `14` | report a skill as stale after this many idle days |
| `archive_after_days` | `30` | archive an agent-created skill after this many idle days |

`LifecycleState` (`skill_usage.rs:27`) has **4** variants: `Active`
(default), `Deprecated`, `Archived`, `Retired`. There is **no `Stale`
variant** — "stale" is a REPORT classification the curator prints, never a
persisted state (the module doc says this explicitly,
`curator/mod.rs:9-14`: a skill is never marked stale, it is reported as
stale). The curator's `apply_automatic_transitions` (`:313`) walks the
tracker: an agent-created skill inactive for `≥ archive_after_days` is moved
to `.archive/` (a file move, `archiver.rs`); a skill inactive for
`≥ stale_after_days` but not yet at the archive threshold is added to the
`stale` report list. A skill referenced by a cron job is NEVER archived
(the cron DB is opened and read; an unreadable cron DB skips the pass rather
than running with an empty protection set, RT-267). Umbrella-skill
consolidation (the `consolidate` review pass) is LLM-driven and separate
from the deterministic archive/stale pass.

`SkillUsageTracker` (`skill_usage.rs`) is atomic (temp + rename),
migrates the legacy `.usage.json` to the curator's `usage.json` (covered by
tests at `skill_usage.rs:958-1080`), and exposes `record_usage`,
`record_created`, `bump_patch`, `remove`, `is_pinned`,
`with_exclusive_lock`.

## Configuration

| Key | Where | Default | What it changes |
|---|---|---|---|
| `root_dir` | `[skills]` | `$HOME/.operant/skills` (via `platform::operant_skills_dir`) | the primary pool |
| `extra_roots` | `[skills]` | `[]` | additional read-only roots, in precedence order |
| `project_root` | `[skills]` | `"auto"` | project discovery: `"auto"` resolves `<workspace>/skills`, `"off"` disables, a path is an explicit root |
| `template_vars` | `[skills]` | `true` | `${OPERANT_SKILL_DIR}` / `${OPERANT_SESSION_ID}` substitution |
| `inline_shell` | `[skills]` | `false` | `` !`cmd` `` expansion at load time |
| `inline_shell_timeout` | `[skills]` | `10` (seconds) | timeout per inline-shell snippet |
| `autoload` | `[skills]` | `true` | accepted by the schema, **no production reader found in this tree** — see Failure modes |
| `template_name` / `template_description` | `[skills]` | `"new-skill"` / `"Describe what this skill does."` | accepted by the schema, **read by nothing** (RT-136) |
| `[ponytail]` | top-level | see `ponytail.rs` | posture + content refresh for the ponytail integration |
| `OPERANT_SKILL_REGISTRY` | env | unset | marketplace registry URL override |
| `OPERANT_BUNDLES_DIR` | env | unset | bundle dir override (tests) |
| `OPERANT_BUNDLED_SKILLS_DIR` | env | unset | bundled skill pool override (seed) |

## Failure modes

| What breaks | What you see |
|---|---|
| A skill dir has a dead symlink | `RootIssue { reason: "dead_symlink" }` on `operant doctor`; the skill is skipped, not fatal |
| A root is a credential store (`~/.ssh/…`) | `RootIssue { reason: "sensitive_path: …" }`; the root is refused |
| A skill fails to parse | logged at load, skipped; `skills_list` does not list it |
| A skill has a placeholder description (< 60 chars or a substring match) | `skill_manage` `create`/`edit` returns a validation error |
| A skill's `name:` does not match its directory | `skill_manage` `create`/`edit` returns a validation error |
| An agent-created write trips a Critical pattern | write rolled back; tool error carries the guard reason (and a rollback-failure warning if the rollback itself failed, RT-426) |
| A marketplace install has a checksum mismatch | install refused, partial file deleted |
| A content-integration update trips a Dangerous verdict | update refused, previous version kept |
| The curator's cron DB is unreadable | auto-run SKIPPED (`warn!`), not run with an empty protection set (RT-267) |
| `[skills] autoload = false` | no production effect found in this tree; the schema comment says it should gate skill loading and tool registration, but no code reads it — treat as unimplemented |
| `[skills] template_name = "x"` | accepted, parsed, and ignored — the value never reaches a template (RT-136) |
| An inline-shell snippet times out | body contains `[inline-shell timeout …]`; the snippet is killed, the skill still loads |
| An inline-shell snippet fails | body contains `[inline-shell error: …]`; the snippet is skipped, the skill still loads |
| A bundle references a skill that does not exist | the message contains the loaded skills; `missing` names them; the turn continues with what loaded |
| `find_skill_in_roots` gets a non-relative name (`../x`, `/etc`) | returns `None` — the path guard refuses it |

## Intention

The page exists so a reader can answer four questions without reading code:
which directories are searched and in what order, which tools exist and what
they can do, which guard runs and what it refuses, and which knobs exist and
which do not. The two facts a reader should leave with: a skill is a
directory with `SKILL.md`, and the guard is a separate scanner the write path
calls — not a property of the manager. The third fact a reader should leave
with: `stale` is a report, not a state; nothing is ever persisted as stale.

If this page is wrong, the smallest places to check are
`SkillRoots::resolve`, `THREAT_PATTERNS`, `install_policy_verdict`,
`REVIEW_MUTATING_ACTIONS`, and the `[skills]` table in
`crates/operant-core/src/config.rs`. Each is a small constant or table that
the page cites by name.
