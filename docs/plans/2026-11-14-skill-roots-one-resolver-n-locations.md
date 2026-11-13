# Skill roots — one resolver, N locations

Dated-frozen: published `2026-11-14`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## The problem, measured

The operator reports "a lot of missing skills" when searching with the `$name` mention.
Measured on this host (2026-11-14):

| pool | contents |
|---|---|
| `~/.operant/skills` (the operator pool) | **99** entries — 77 of them SYMLINKS into `~/.agents/skills`, 22 real dirs |
| `<repo>/skills` (the project library) | **100** skills across 15 category dirs (`software-development/axi`, `creative/website-design`, …) |
| the vendor roots (ponytail integration + plugin skill dirs) | 6 |

**72 of the project library's 100 skills are in NEITHER pool** — `ai-slop-cleaner`, `arxiv`,
`codex`, `opencode`, `operant`, `plan`, `skill-creator`, `test-driven-development`, … They are
invisible to EVERY skill surface: the `$` popup, `/skill <Tab>`, the `/skills` panel,
`skills_list` / `skill_view`, the gateway's picker.

**Why.** `crates/operant-cli/src/main.rs` seeds the bundled pool only when the operator pool is
EMPTY:

```rust
// First-run bootstrap: seed the bundled skill pool (ships with operant)
// into the user skills directory when it is empty or missing. No-op once
// the user has any skill installed; `operant skills seed --force` re-runs.
let empty = std::fs::read_dir(skills_dir).map(|mut e| e.next().is_none()).unwrap_or(true);
if empty && let Err(e) = cmd_skills::seed_bundled_skills(&loaded.config, None, false) { … }
```

The pool holds 99 entries → never empty → the project library is never seeded.
`operant skills seed --force` re-runs it, but that COPIES 100 skills into the pool — a second
copy that drifts from the repo the moment the operator edits one.

Four more shapes of the same gap:

1. **The listing is flat; the resolution is not.** `SkillManager::load_root` reads only the
   root's immediate children (`<root>/<name>/SKILL.md`), while `find_skill_in_roots` →
   `find_skill_recursive` resolves category-nested names AND `category/skill` paths. A nested
   skill is therefore FINDABLE but not LISTABLE — the model can invoke a skill the operator
   cannot see.
2. **The `$` grammar cannot spell a nested name.** `skill_mentions::is_mention_name_char`
   allows `[A-Za-z0-9_-:]` — no `/`. `find_skill_in_roots` supports `category/skill`;
   `$category/skill` cannot be typed.
3. **There is no configuration for more locations.** `SkillsSettings` (`operant-core/src/config.rs`)
   has `root_dir: PathBuf` — ONE path — and `#[serde(default, deny_unknown_fields)]`, so a list
   cannot even be spelled without a config change.
4. **A fourth location exists and is invisible AS a location.** The pool's 77 entries symlink
   into `~/.agents/skills`; nothing in the tree knows that directory exists. It works only
   because the loader follows the symlinks — one indirection the operator cannot configure,
   audit, or see in `/skills`.

## How `@folder:` does it — the model to copy

`crates/operant-core/src/context_references.rs` is the house's existing answer to "a directory
the user named". Its rules, verbatim from the module doc:

* References resolve **relative to the current working directory**, and cannot escape it
  (`allowed_root` defaults to cwd): `@file:../secret` and absolute paths outside the workspace
  are refused.
* A deny-list of sensitive home paths (`.ssh`, `.aws`, `.gnupg`, `.kube`, `.docker`, `.azure`,
  `.config/gh`, rc-files, `.netrc`, `.pgpass`, `.npmrc`, `.pypirc`, …) plus the canonical
  credential stores (`auth.json`, `.anthropic_oauth.json`, `mcp-tokens/`, `.env`) is enforced
  **fail-closed** — a path we cannot verify is refused, because the gateway feeds untrusted
  remote text into this path.
* The total injected context is budgeted against the model's context window: >50% is refused
  outright (`blocked`), >25% warns.

The mechanics, from the code:

```rust
// context_references.rs
fn resolve_path(cwd: &Path, target: &str, allowed_root: &Path) -> Result<PathBuf, String> {
    let expanded = if target.starts_with('~') { home.join(rest) } else { PathBuf::from(target) };
    let path = if expanded.is_absolute() { expanded } else { cwd.join(expanded) };
    let resolved = path.canonicalize().map_err(|_| "path does not exist".to_string())?;
    if !resolved.starts_with(allowed_root) {
        return Err("path is outside the allowed workspace".to_string());
    }
    if is_sensitive_path(&resolved) {
        return Err("path is a sensitive credential or internal path and cannot be attached".to_string());
    }
    Ok(resolved)
}

fn iter_visible_entries(path: &Path, limit: usize) -> Vec<PathBuf> { … }   // stack walk: skips
// dot-entries, __pycache__, node_modules, target; sorts; caps the count
```

So `@folder:../external-repos/jcode/` — which is outside the workspace — is **REFUSED** by the
containment rule, and the agent passes only `&cwd`, so `allowed_root` is not configurable today.

**What a skill root must copy:** the canonicalize, the fail-closed sensitive-path deny-list,
the bounded visibility walk, and the count cap.
**What a skill root must NOT copy:** the cwd containment. A skill root legitimately lives
OUTSIDE the workspace (`~/.operant/skills`, `~/external-repos/jcode/skills`). That rule exists
because a gateway feeds untrusted text into `@file:`; a skill root is operator-configured, and
its own canonicalized path IS its boundary. The boundary that matters for a root is: it must not
be a sensitive path, and the walk must stay inside it.

## Mechanism

### One resolver, N locations

`operant_core::skills` owns the list; every surface asks it.

```rust
// crates/operant-core/src/skills.rs — the ONE owner of "where does a skill live"
pub struct SkillRoots {
    /// The operator pool first, then the manual locations, then the project
    /// pool, then the vendor roots. First root wins a name collision.
    pub roots: Vec<PathBuf>,
}
impl SkillRoots {
    pub fn resolve(config: &SkillsSettings, workspace: Option<&Path>) -> SkillRoots { … }
}
```

Six surfaces ALREADY take the same list — `SkillManager`'s extra roots, the expansion's
`resolve_skill_dir`, `SkillsTool::with_vendor_roots`, `loadable_skills` (the `$` popup and the
gateway's picker), the TUI's `/skills` panel, and the gateway's `/skill` arm. The plan renames
the concept from "vendor roots" to "the roots" and makes one function produce it, so the NEXT
location is a config change, not a threading change. (iter-2159 and iter-2198 already had to
thread that list by hand twice — twice is the evidence that it needs an owner.)

### The config surface (the smallest that works)

```toml
[skills]
root_dir = "$HOME/.operant/skills"     # unchanged: the global pool
extra_roots = [                         # NEW: manual locations, in precedence order
  "$HOME/Documents/github/my-projects/agentic-harness/operant/skills",
  "$HOME/external-repos/jcode/skills",
]
project_root = "auto"                   # NEW: the workspace's own skills dir
```

* `extra_roots: Vec<PathBuf>` — the operator's manual locations. `~` / `$HOME` expanded exactly
  like `root_dir` (RT-197's rule); each canonicalized; each checked against the SAME
  sensitive-path deny-list `@folder:` uses (fail-closed); a missing or unreadable root is
  reported by `/doctor` and skipped — never fatal, never silent.
* `project_root: ProjectRoot` — `"auto"` (the default) discovers `<workspace>/skills`, where the
  workspace is the same root the agent resolves `@folder:` against; `false` disables it; an
  explicit path overrides it. The project pool becomes a LIVE root, not a copy: the repo's
  `skills/` is already canonical (`workspace-lint.yaml` lists it) and already discovered by
  `bundled_skills_dir()` — as an install SOURCE. Making it a root removes the copy AND the
  drift.

### The listing becomes recursive, and bounded

`SkillManager::load_root` walks its root the way `iter_visible_entries` walks a folder: skip
dot-entries / `__pycache__` / `node_modules` / `target`, sort, cap the count, and say so when
it truncates. The walk ALREADY EXISTS — `collect_skill_dirs` in `cmd_skills.rs` was written for
the seeder and does exactly this ("Recursively collect directories under `dir` that contain a
SKILL.md. A directory that itself carries SKILL.md is a skill root and is not descended into
further."). It moves to `operant_core::skills`; the seeder and the loader call the one
function.

A nested skill is listed under its leaf name AND under its category-qualified alias
(`creative/website-design`), so the listing and the resolution can never disagree. The leaf
name wins a collision — the root order already decides that.

### The `$` grammar learns the category separator

`is_mention_name_char` gains `/`, so `$creative/website-design` is ONE token;
`active_skill_token` and `find_skill_mentions` take the same rule. The shell-safety argument is
unchanged: `/` cannot begin a shell variable, and the word-boundary + all-caps rules still
reject `$HOME`, `$1`, `${VAR}` and `$(cmd)`.

### The startup seeding stays — as the fallback, not the only door

`main.rs`'s "seed only when empty" bootstrap remains for a fresh install, but it stops being the
ONLY way the repo's library reaches the agent: with `project_root = "auto"` the library IS a
root. `operant skills seed --force` remains for operators who want the copies.

## Gates / success criteria

(fixed before the run)

1. `$creative/website-design` in the TUI popup completes the skill, and submitting it expands the
   skill's body — a test that builds a categorized pool in a temp dir and drives
   `compute_typeahead` + `expand_skill_mentions_in`.
2. `skills_list` reports a categorized skill that lives ONLY in a project root — the listing and
   the resolution agree.
3. A manual location outside the workspace resolves (`extra_roots`); a location inside a
   sensitive path (`.ssh`) is REFUSED and the refusal NAMES the reason.
4. The six surfaces (the `$` popup, `/skill <Tab>`, the `/skills` panel, `skills_list` /
   `skill_view`, the gateway's picker, the invocation expansion) offer the SAME set — one test
   that collects the names from each and asserts equality.
5. The `$` grammar still rejects `$HOME`, `$1`, `${VAR}`, `$(cmd)`, `a$b` — the existing core
   tests stay green.
6. A project root contributes its skills WITHOUT copying them: after a run, the pool's entry
   count is unchanged (the regression guard on the copy the plan removes).

## Cost

* 2 new config fields on `SkillsSettings` (`extra_roots`, `project_root`). The struct is
  `deny_unknown_fields`, so this is a config CHANGE, not a silent no-op — and the example TOML
  ships the new keys.
* 1 new resolver in `operant_core::skills`; 6 call sites renamed from "vendor roots" to "roots"
  (no behaviour change for them).
* 1 recursive walk in `operant_core::skills` (`collect_skill_dirs` moves from the CLI crate).
* 1 character in the `$` grammar (`/`).
* The `operant skills seed` copy path stays, for compatibility.

## Risks

* **A root can be huge.** The recursive walk is capped and REPORTS the truncation (the same
  discipline `@folder:` uses for its listing).
* **Name collisions across roots.** First root wins — the operator's `extra_roots` order is the
  tiebreak — and `/skills` shows each skill's origin, so a shadowed skill is visible rather than
  silently replaced.
* **A project root changes what the agent can invoke.** That is the point of the plan, but it
  means a workspace's `skills/` becomes live. The sensitive-path deny-list still applies, and
  the existing `autoload` flag is the gate for it.
* **`~/.agents/skills` stays invisible as a LOCATION** until the operator lists it in
  `extra_roots` — the plan does not auto-discover it (auto-discovery of a hidden directory is
  how a location becomes unauditable). The 77 symlinks keep working meanwhile.

## Not doing

* Making `@folder:` reach outside the workspace. That is a different permission surface (it
  attaches file CONTENT to a prompt); a skills root only makes a skill invocable. If the
  operator wants to attach files from a manual location, the honest change is an explicit
  `[context_references] extra_allowed_roots` list — deliberately NOT in this plan.
* A skills marketplace / hub for the extra roots.
* Copying the project pool into the operator pool — the copy is what this plan removes.
