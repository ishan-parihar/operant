# Skill directory configuration: global, project and manual roots for every skill surface

Dated-frozen: published `2026-11-14`. A plan that changes is a new plan (or declare it
in `genome.toml` `core.living_plans` and keep a `Resume here` section).

**Contents**

1. [The operator's question, answered](#the-operators-question-and-the-answer-in-one-line-each)
2. [The problem, measured on this host](#the-problem-measured-on-this-host-2026-11-14)
3. [How `@folder:` does it, and which half to copy](#how-folder-does-it--and-which-half-to-copy)
4. [How jcode does the same job](#how-jcode-does-the-same-job-the-second-reference)
5. [Mechanism](#mechanism)
6. [Gates / success criteria](#gates--success-criteria)
7. [Cost](#cost) · [Risks](#risks) · [Not doing](#not-doing)
8. [How each number was derived](#how-each-number-was-derived) · [Related](#related)

## The operator's question, and the answer in one line each

| asked | answer today |
|---|---|
| How does the agent fetch skills? | `SkillManager::load_all` (`crates/operant-core/src/skills.rs:219`) does one `read_dir` per root and keeps each immediate child that carries a `SKILL.md`. Resolution is a DIFFERENT walk: `find_skill_in_roots` (`:123`) recurses. |
| Is there a skills-directory setting in the config? | ONE key: `[skills] root_dir` (`crates/operant-core/src/config.rs:1599`). There is no list, and `#[serde(default, deny_unknown_fields)]` (`:1597`) means a second key is a parse error, not a silent no-op. |
| Is there a manual-location setting? | **No.** Nothing in `SkillsSettings` names a directory outside `root_dir`. The only other roots are the vendor roots (`crates/operant-core/src/skills.rs:109`) — the ponytail integration plus plugin-contributed dirs — decided by the HOST, not by the operator. |
| Is there global (user-level) discovery? | `root_dir` is it, and it defaults to `platform::operant_skills_dir()` (`:86`, `~/.operant/skills`). Nothing else is consulted. |
| Is there project-level discovery? | **No.** Nothing in the skills path resolves a directory from the workspace: `grep -rn current_dir` over `skills.rs`, `skill_mentions.rs` and `crates/operant-cli/src/skill_roots.rs` finds only the recursive-search helper's parameter name. The repo's own `skills/` library (100 skills) is only ever an install SOURCE (`bundled_skills_dir`, `cmd_skills.rs:206`) or a COPY target (`seed_bundled_skills`, `:255`). |
| Why does `$name` report "not found" about skills that work? | The `$` popup and `/skill <Tab>` read `loadable_skills(root, vendor_roots)` (`crates/operant-core/src/skill_mentions.rs:231`), whose `root` is the single `root_dir`. Anything outside that root and the vendor roots is invisible to the popup — and to `skills_list`, `/skills`, the gateway picker and the invocation expansion alike, because all of them take the same two arguments. |

## The problem, measured on this host (2026-11-14)

Everything below is a command's output, not a recollection. Re-derive before trusting it
(RG-011); the commands are in "How each number was derived" at the end.

| pool | `SKILL.md` files | leaf names | reachable from a skill surface? |
|---|---|---|---|
| `~/.operant/skills` (the operator pool, `root_dir`) | 105 (99 at the top level, 6 in category dirs) | 105 | YES — this is the root |
| `~/.agents/skills` (where 76 of the pool's symlinks point) | 286 | 285 | **NO** — 209 of its leaves are in no reachable pool |
| `<repo>/skills` (the project library, 16 category dirs) | 100 (98 at `<category>/<skill>/SKILL.md`) | 100 | **NO** — 41 of the 100 are in no other reachable pool |
| the vendor roots (ponytail `v5.3.3` + `herdr`) | 6 | 6 | YES — extra roots |

Four facts inside those numbers decide the design:

1. **The operator's pool is mostly symlinks, and six of them are dead.**
   `ls -la ~/.operant/skills` holds 112 entries: 83 symlinks, 27 plain dirs, 2 files. Six
   symlinks point at directories that no longer exist — `aportal`, `facebook-mcp`,
   `omarchy-bar-management` and `wealth-sovereign` into `~/.claude/skills` (6 entries left),
   `doc` into `~/.agents/skills`, and `storage-management` into `~/.jcode/skills`.
   `Path::is_dir()` follows symlinks, so the loader skips them silently: the CLI says
   "105 skills" and the operator's `$aportal` says "not found". **A dead link is a
   configuration defect nothing reports.**
2. **The listing and the resolution disagree about depth, and the pool already holds the
   disagreement.** `load_root` (`skills.rs:244`) reads a root's immediate children only;
   `find_skill_recursive` (`:143`) recurses, skipping dot-entries, `node_modules`,
   `.archive` and `.hub`. The live pool has SIX skills one level down —
   `debugging/source-grounded-decision-probe`, `operant/identity-warden`,
   `operant/operant-session-forensics`, `operations/live-scheduler-audit`,
   `testing/operant-cli-test-battery`, `testing/test-tool-verification` — and those are
   exactly the six that `operant skills list --json` (105 rows) does NOT return while
   `find_skill_in_roots` WOULD resolve. Findable but not listable, today.
3. **Four surfaces walk a directory tree, and they use three different skip rules.**
   `collect_skills_recursive` (`crates/operant-core/src/tools/skills_tool.rs:455`) skips
   dot-entries and `node_modules` only; `is_skill_dir_skipped` (`:104`) also skips
   `_`-prefixed names and the resource dirs (`ALLOWED_SUBDIRS`, `:77`); the seeder's
   `collect_skill_dirs` (`crates/operant-cli/src/cmd_skills.rs:326`) skips NOTHING — it
   recurses into `.archive`, `.curator` and `node_modules` alike. One of the three is
   already wrong on the live pool: `.curator` (holding `state.json`, `usage.json` and
   `usage.json.lock`) sits inside `root_dir`.
4. **Advertising is budgeted, so a bigger pool is not automatically a better one.**
   `<available_skills>` rides the frozen prefix (the system message, never evicted) and is
   capped at `ADVERTISED_SKILLS_TOKEN_BUDGET = 4_000` tokens
   (`context_management.rs:1691`, overridable by `[agent] advertised_skills_token_budget`,
   `config.rs:821`). Measured with the emitter's own line shape
   (`  <skill name="…">…</skill>\n`, `events.rs:1238`) and its own estimator
   (`estimate_tokens` = `chars/4` ceil, `context_management.rs:41`) over the descriptions
   the CLI actually reports (the block advertises `list_available()`, `events.rs:1218`,
   not `list()`):

   | pool set | skills | ≈tokens | advertised at 4 000 | dropped |
   |---|---|---|---|---|
   | pool + vendor (today) | 105 | 7 534 | 55 | 50 |
   | + `~/.agents/skills` | 314 | 19 727 | 55 | 259 |
   | + the repo library | 355 | 20 784 | 55 | 300 |

   **The advertised count is already saturated today** (55 of 105). Adding roots does not
   make the model see more; it widens what the model can REACH (invocation, listing, the
   tools) at the same prompt cost, and it grows the drop count. That is the honest framing
   of this plan's benefit. A root list that ships without an answer to "what is the model
   told" would be shipping a regression.

## How `@folder:` does it — and which half to copy

`crates/operant-core/src/context_references.rs` is the house's existing answer to "a
directory the user named". Its rules, from the module doc and the code:

* **canonicalize, then contain.** `resolve_path` (`:346`) expands `~`, joins a relative
  target against `cwd`, `canonicalize()`s, and refuses anything outside `allowed_root`
  (`:368`). `@folder:../external-repos/jcode/` — the operator's own example — is therefore
  **refused** by that containment rule: `allowed_root` is the canonicalized `cwd` (`:721-722`) and the referenced
  directory is outside it. That is correct FOR `@folder:` because it attaches file CONTENT
  to a prompt, and the gateway feeds untrusted remote text into the same path.
* **fail-closed sensitive-path deny-list.** `is_sensitive_path` (`:298`) refuses `~/.ssh`,
  `~/.aws`, `~/.gnupg`, `~/.kube`, `~/.docker`, `~/.azure`, `~/.config/gh`, the rc-files,
  `.netrc`, `.pgpass`, `.npmrc`, `.pypirc`, and the credential-store suffixes (`auth.json`,
  `.env`, `.git-credentials`, `operant.toml`, …). No `HOME` means refuse.
* **a bounded visibility walk.** `iter_visible_entries` (`:496`) skips dot-entries,
  `__pycache__`, `node_modules` and `target`, sorts, and caps the entry count.
* **a size cap with a stated refusal.** `MAX_AUTO_READ_TEXT_BYTES = 5 MiB` (`:436`).

**What a skill root copies:** the canonicalize, the fail-closed deny-list, the bounded walk
with a stated skip set.
**What a skill root must NOT copy:** the cwd containment. A skill root legitimately lives
outside the workspace — `~/.operant/skills` already does, and the ask is to point at
`external-repos/jcode/skills`. The boundary that matters for a root is different: the root's
own canonicalized path IS the boundary, the walk stays inside it, and the root is
operator-configured — never derived from message text.

## How jcode does the same job (the second reference)

`external-repos/jcode/crates/jcode-base/src/skill.rs` separates GLOBAL from PROJECT and
composes them at read time. That is the shape this plan adopts:

| jcode | what it does |
|---|---|
| `SkillRegistry::load_global()` (`skill.rs:260`) | plugins first (`~/.claude/plugins` via `installed_plugins.json`, bounded at `PLUGIN_SCAN_MAX_DEPTH = 5`, `:54`), then `~/.agents/skills`, then `~/.jcode/skills`; **later inserts win**, so explicit pools shadow plugin skills |
| `load_project_overlay(working_dir)` (`skill.rs:297`) | `./.claude/skills`, `./.agents/skills`, `./.jcode/skills` under the workspace, loaded FRESH from disk on access — edits visible without a restart, and two sessions in different repos never see each other's project skills |
| `effective_for_working_dir(base, working_dir)` (`skill.rs:314`) | the shared global registry + the session's overlay; the overlay merges LAST and `insert_loaded_skill` (`:674`) is an unconditional `HashMap::insert`, so an overlay skill wins a name collision |
| `shared_registry()`'s doc (`skill.rs:74`) | project skills are deliberately kept OUT of the process-wide registry, because the daemon's startup cwd must never decide another session's skills |

Two rules are worth importing verbatim: **a project pool is an overlay, not a copy**, and
**the project root resolves per session, never from a daemon's startup cwd.**

## Mechanism

### 1. One resolver owns the list (`operant_core::skills`)

```rust
// crates/operant-core/src/skills.rs — the ONE owner of "where does a skill live"
pub struct SkillRoots {
    /// Precedence order; the first root that yields `name` wins.
    pub roots: Vec<PathBuf>,
    /// Roots that were configured but are missing, unreadable or denied —
    /// for the doctor and the listing's footer. Never silently dropped.
    pub rejected: Vec<(PathBuf, String)>,
    /// True when a walk hit its cap and the set is incomplete (stated, never silent).
    pub truncated: bool,
}

impl SkillRoots {
    /// The precedence order:
    ///   primary pool → the operator's extra roots → the project root → vendor roots.
    pub fn resolve(
        config: &SkillsSettings,
        host_roots: &[PathBuf],
        workspace: Option<&Path>,
        holon_override: Option<&Path>,
    ) -> SkillRoots;
}
```

Rules, each of which is a test:

* **Precedence.** The primary pool first, so an operator skill shadows a same-named project
  or vendor skill — what `load_all`'s comment already promises and
  `find_skill_in_roots` already applies by search order.
* **Canonicalize and dedupe by canonical key.** `~/.agents/skills` plus an 83-entry symlink
  farm pointing into it must not become 83 duplicate entries.
* **Each root passes the SAME fail-closed deny-list `@folder:` uses.** `is_sensitive_path`
  becomes a `pub(crate)` seam (one list, two callers) and a denied root lands in `rejected`
  with its reason — dropped, named, never silent.
* **A missing root is kept in `rejected`, not hidden.** `load_all` already skips a
  non-directory extra root (`skills.rs:234`); the doctor prints the list.
* **A Holon run resolves to the brief's library only.** `vendor_skill_roots` already returns
  empty on a Holon run (`skills.rs:110`) and `main.rs:1780` already swaps the primary to the
  path `brief.json` names. The resolver takes that as `holon_override` rather than calling
  `is_holon_run()` itself, so that decision stays where it already lives.

### 2. The config surface (the smallest that works)

```toml
[skills]
root_dir = "$HOME/.operant/skills"          # unchanged: the primary pool

# NEW — manual locations, in precedence order. `~` / `$HOME` expanded like root_dir
# (RT-197); each canonicalized; each checked against the sensitive-path deny-list,
# fail-closed; a missing or denied one is reported by `operant doctor` and skipped.
extra_roots = [
  "$HOME/.agents/skills",
  "$HOME/Documents/github/my-projects/agentic-harness/operant/skills",
]

# NEW — project-level discovery: "auto" (default) | "off" | an explicit path.
# "auto" = <workspace>/skills, the workspace being the root @folder: resolves
# against (the process cwd, walked to the git root).
project_root = "auto"
```

* `extra_roots: Vec<PathBuf>` — the direct answer to "can we also set manual locations for
  the skills directory": today no such key exists, and `deny_unknown_fields` makes adding
  one a parse-time contract rather than a silent no-op.
* `project_root` — `"auto"` / `"off"` / a path. `auto` resolves `<workspace>/skills`, and a
  workspace without one is a normal state (a gateway chat has no repo), not a warning.
* Both keys are expanded in `SkillsSettings::expand_paths` (`config.rs:1637`), which already
  owns `~` for `root_dir` and `memory_dir`, and both ship in `operant.example.toml` in the
  same commit — the example TOML is the discoverability surface.
* **`memory_dir` and the curator do not move.** The curator's telemetry is
  `<primary>/.curator/usage.json` (`skill_preprocessing.rs:385`) and the primary stays
  `root_dir`, so the operator's existing history (39 entries on this host) is never orphaned.
  A root list that moved the primary would do exactly that, which is why it does not.

### 3. The loader becomes recursive, bounded, and origin-aware

`load_root` (`skills.rs:244`) walks each root the way `iter_visible_entries` walks a folder:
skip dot-entries, `_`-prefixed names, `__pycache__`, `node_modules`, `target`; sort; cap the
walk at a stated constant and set `truncated` when it truncates. **The walk lives in
`operant_core::skills` as ONE function and the seeder calls it**, so the "three skip rules"
fact above becomes one rule.

A nested skill is indexed under its **leaf name** and its **category-qualified path**
(`software-development/axi`), because `find_skill_in_roots` already accepts `category/skill`
and `is_plain_relative_path` (`skills.rs:172`) already keeps a `category/skill` name inside
the root — it refuses an absolute path and any `..`/`.` component. Leaf first: the leaf is
what the operator types.

`SkillOrigin` gains a third variant, `Project`. The mutators stay pool-only:
`create`/`delete` write the primary pool, and a project skill is refused with the reason — a
repo's `skills/` is version-controlled, so an in-place edit is a change to someone else's
repository. `skill_manage` already refuses integration skills the same way, so the pattern is
established rather than invented.

### 4. The `$` grammar learns the category separator

`is_mention_name_char` (`skill_mentions.rs:25`) gains `/`. `$software-development/axi`
becomes ONE token, resolvable by the existing `find_skill_in_roots`. The shell-safety
argument is unaffected and pinned by the existing cases: `/` cannot begin a shell variable
name, and the word-boundary rule plus `is_shell_spelling` (`:35`) still reject `a$b`,
`$HOME`, `$PATH`, `$1`, `${VAR}` and `$(cmd)`. **Two new rejections need their own cases:**
`$foo/` and `$/foo` (an empty segment) must not be mentions, because
`is_plain_relative_path` refuses them at resolve time and the popup would offer a name the
expander cannot run.

### 5. The surfaces take the list, not the pair

Every surface takes `(root, vendor_roots)` today and rebuilds the same vector by hand. The
signature becomes `&SkillRoots` (or `&[PathBuf]`):

| surface | today | after |
|---|---|---|
| the `$` popup + `/skill <Tab>` | `register_invocable_skills(root, vendor_roots, bundles)` (`tui/prompt_input/typeahead.rs:483`) → `loadable_skills` | `register_invocable_skills(&SkillRoots, bundles)` |
| the `/skills` overlay | `skills_view.open(skills_dir, vendor_roots)` (`tui/skills_view.rs:62`, called at `tui/app/commands.rs:592`, `:675`) | `open(&SkillRoots)` |
| the invocation expansion (TUI) | `expand_skill_mentions_in(root, …, vendor_roots)` (`tui/adapter_types/tui_app.rs:812`) | `(roots, …)` |
| the invocation expansion (gateway) | same call at `gateway_runner.rs:1033` | same |
| the gateway picker | `loadable_skills` / `search_skills(root, vendor_roots, …)` (`gateway_runner.rs:888`, `:890`) | `(roots, …)` |
| `skills_list` / `skill_view` | `SkillsTool::with_vendor_roots` + `visible_skills` / `find_visible_skill(pool, name, vendor_roots)` (`tools/skills_tool.rs:431`, `:449`) | one `SkillRoots` field |
| the agent's `SkillManager` | built at `main.rs:1791` from `root_dir` + vendor roots | built from `SkillRoots` |
| the CLI listing / search / inspect | 13 `SkillManager::new` sites in `cmd_skills.rs` | one helper taking `&AppConfig` + the workspace |
| the gateway's `/reload-skills` | rebuilds a manager per slot (`gateway_runner.rs:1234`) | the same helper |

The seam that makes this safe: **one function produces the list, the surfaces consume it.**
iter-2159 and iter-2198 each hand-threaded a list and each found a surface that disagreed
with the rest — that is the evidence the threading needs an owner, not a third instance.

### 6. Advertising: state the cost, then choose

The table above says the advertised block saturates at 73 skills whether the pool is 105 or
357. So the plan does not silently widen the pool and let the prompt absorb it:

* the drop is ALREADY stated in the frozen prefix (`events.rs:1255`, "N skill(s) advertised,
  M more installed but not advertised… Use the skills tool to list them"), so a wider pool
  stays honest without new prose;
* the operator gets the number: `operant doctor` gains one row — roots resolved, rejected
  roots with reasons, skills loadable, skills advertised, tokens used of the budget — so
  "why can't the model see my skill" is answered by a command instead of by this document.

A per-root advertisement policy (advertise the project pool, list-only for `~/.agents`) is
deliberately NOT here: it is a prompt-design decision, it needs a measurement against a real
provider, and the drop statement covers the interim.

## Gates / success criteria

(fixed before the run — each is a command or a test that fails without the change)

1. **A manual location resolves.** With `extra_roots = ["<tmp>/manual"]` holding a skill that
   exists ONLY there, `$name` completes in the popup and submitting the message expands the
   skill's body. The test drives `compute_skill_mention_suggestions` +
   `expand_skill_mentions_in` against a temp pool — no network, no live agent.
2. **A project root resolves without a copy.** With `project_root = "auto"` and a workspace
   holding `<ws>/skills/cat/leaf/SKILL.md`, `skills_list` reports `leaf` and `$cat/leaf`
   expands it; the primary pool's entry count is UNCHANGED afterwards (the regression guard
   against the copy the seeder would otherwise make).
3. **The surfaces agree.** One test collects the names from the popup, `/skill <Tab>`, the
   `/skills` overlay's loader, `skills_list`, `skill_view` and the gateway's picker, and
   asserts the sets are EQUAL for a fixture carrying an operator skill, a manual-root skill, a
   project skill and a vendor skill.
4. **A denied root is refused and NAMES the reason.** `extra_roots = ["$HOME/.ssh"]` is
   absent from `SkillRoots::roots`, present in `rejected` with the sensitive-path reason, and
   printed by `operant doctor`.
5. **A dead symlink is reported, not swallowed.** A pool entry whose target does not exist is
   listed by `operant doctor` with the link and its target (the six on this host are the
   fixture).
6. **The grammar still refuses shell syntax.** `$HOME`, `$PATH`, `$1`, `${VAR}`, `$(cmd)`,
   `a$b`, `$foo/`, `$/foo` are not mentions — the existing core tests stay green and two
   cases are added for the empty segment.
7. **The walk is bounded and says so.** A fixture above the cap yields `truncated = true` and
   the surfaces say "N more not listed" rather than dropping skills silently.
8. **The advertised number is unchanged or explained.** Measured before/after with the
   emitter's own estimate; if the advertised count drops, the CHANGELOG entry answers it.
9. **Holon is unaffected.** A Holon fixture resolves to `brief.json`'s library only — no
   operator pool, no project root, no vendor root.
10. `./scripts/check.sh check -p operant-core --lib`,
    `./scripts/check.sh check -p operant-cli --bin operant`, the touched crates' tests, and
    `bash scripts/clippy-warning-gate.sh` with no NEW warnings.

## Cost

* 2 new keys on `SkillsSettings` (`extra_roots`, `project_root`) + `operant.example.toml` +
  `expand_paths` handling. `deny_unknown_fields` makes it a parse-time contract, not a silent
  field.
* 1 resolver + 1 recursive walk in `operant_core::skills`; `collect_skill_dirs` moves from the
  CLI crate and the seeder calls it.
* 1 character in the `$` grammar, plus two rejection cases.
* ~25 call sites re-pointed from `(root, vendor_roots)` to the list. Mechanical, and the
  compiler finds every one — which is the point of changing the signature rather than adding a
  parallel path.
* `SkillOrigin::Project` + the mutator refusal for it.
* One `operant doctor` row.
* No new dependencies.

## Risks

* **A root can be huge.** `~/.agents/skills` is 285 skills / 3 000+ files. The walk is capped
  and states its truncation; the cap is a constant with a measured rationale, not a number
  copied from a test.
* **Name collisions across roots.** First root wins; the loser is invisible, not merged. The
  listing already marks vendor rows (`[integration]`); a `project` marker follows in the same
  spirit.
* **A project root makes a repo's `skills/` live.** That is the ask. The deny-list still
  applies, the primary pool still wins collisions, and `project_root = "off"` is the escape
  hatch. A workspace whose `skills/` symlinks into a credential store is refused by the same
  fail-closed check.
* **The advertised block saturates.** Named above with measurements; the mitigation is the
  stated drop count, and this plan refuses to widen the pool without saying what the model is
  told.
* **A Holon body must not gain a personal pool.** Pinned by gate 9 and by the existing
  `is_holon_run` early return in `vendor_skill_roots`.

## Not doing

* **Making `@folder:` reach outside the workspace.** A different permission surface: it
  attaches file CONTENT to a prompt. The honest change there is an explicit
  `[context_references] extra_allowed_roots` with the deny-list applied — a separate plan.
* **Auto-discovering `~/.agents/skills`.** It becomes a LISTED root (commented in the example
  TOML with what it is), not a magic one. Auto-discovery of a hidden directory is how a
  location becomes unauditable, and the pool's 83 symlinks into it are exactly the indirection
  this plan makes visible.
* **Migrating the pool or the curator telemetry.** The primary stays `root_dir`;
  `seed_bundled_skills` stays for operators who want copies.
* **A marketplace/hub for extra roots.** Out of scope; the listing and the doctor row are the
  whole observability surface.
* **Per-root advertisement policy.** A prompt-design decision that needs a measurement.

## How each number was derived

```bash
# pool / agents / repo leaf counts and the overlap sets
python3 - <<'PY'
import os,glob
def leaves(root):
    return {os.path.basename(os.path.dirname(p))
            for p in glob.glob(os.path.join(root,'**','SKILL.md'),recursive=True)}
pool=leaves(os.path.expanduser('~/.operant/skills'))
agents=leaves(os.path.expanduser('~/.agents/skills'))
repo=leaves('skills')
print(len(pool),len(agents),len(repo),len(repo-pool),len(agents-pool))
PY

# the six dead symlinks in the pool
cd ~/.operant/skills && for d in */; do n=${d%/}; [ -L "$n" ] && [ ! -e "$n" ] \
  && echo "DEAD $n -> $(readlink "$n")"; done

# advertised cost of a pool set (the emitter's own line shape, chars/4)
operant skills list --json | python3 -c 'import json,sys; …'   # table in §"measured"

# the grammar's current character class
rg -n 'fn is_mention_name_char' -A 3 crates/operant-core/src/skill_mentions.rs
```

## Related

* The earlier plan on the same subject, from the measurement side:
  `docs/plans/2026-11-14-skill-roots-one-resolver-n-locations.md` (iter-2208). It
  measured the invisibility and proposed the
  resolver, `extra_roots` and `project_root`. This plan is the same mechanism taken to
  execution: the config surface, the per-surface threading table, the advertisement budget
  answered with a measurement, the three-skip-rule defect, and dead-symlink reporting. Both
  stay on the record; the code follows this one.
* `docs/library/skills.md` — the mechanism library's owner for the skills system.
* Workflow board: `docs/kb/workflows/skill-directory-roots.md`.
