# Family 03 — skills: declared dependencies and the invocation sigil

**Question this file answers:** codex skills declare their own tool dependencies and are invoked
with a `$sigil`. Operant skills declare prerequisites and are invoked with `/skill`. Is the gap a
real one, and is any of it worth taking?

**Ground state:** operant `main` @ `f53907e8`. Reference `../external-repos/codex/codex-rs` @ `588a277b`.

---

## CODEX-SIDE

### What the mechanism is

codex's skills crate (`skills/`, 2,840 LOC total) is a **selection and resolution** layer, not a
storage layer. Four things it owns:

1. **Declared tool dependencies.** `agents/openai.yaml` inside the skill directory can name the
   tools the skill needs (`skills/src/model.rs:81-95`):

   ```rust
   pub struct SkillDependencies { pub tools: Vec<SkillToolDependency> }

   #[serde(rename_all = "camelCase")]
   pub struct SkillToolDependency {
       pub r#type: String, pub value: String,
       pub description: Option<String>, pub transport: Option<String>,
       pub command: Option<String>, pub url: Option<String>,
       pub oauth_callback_port: Option<u16>,
   }
   ```
   `parse_skill_metadata` (`metadata.rs:26-35`) parses it and **drops invalid individual
   declarations** rather than failing the skill.

2. **Sigil invocation.** `skills/src/mentions.rs:26-30` defines `TOOL_MENTION_SIGIL: '$'` and four
   URI prefixes — `app://`, `mcp://`, `plugin://`, `skill://` (`mentions.rs:32-40`).
   `tool_kind_for_path` (`:42-55`) classifies any mentioned path. `extract_tool_mentions_with_sigil`
   scans the user's raw text for `$skill-name` tokens.

3. **Selection with documented complexity.** `collect_explicit_skill_mentions`
   (`selection.rs:39-…`) resolves structured `UserInput::Skill` selections by path first, then text
   mentions, with an explicit complexity note in its doc comment
   (`selection.rs:36-38`): "`O(T + (N_s + N_t) * S)` time, `O(S + M)` space". Plain names are used
   **only when the match is unambiguous** — `build_skill_name_counts` (`name_counts.rs`) rejects a
   plain name carried by more than one skill (`blocked_plain_names`).

4. **Loading with a snapshot cache.** `loading.rs` (`SkillRootLoader`, `SkillRootSnapshotCache`,
   `SkillRootLoadFuture`) caches per-root snapshots so a re-discover is cheap.

### The load-bearing units (file:line + wc -l)

| unit | file:line | LOC | what it owns |
|---|---|---|---|
| `SkillDependencies` + `SkillToolDependency` | `skills/src/model.rs:81-95` | 15 | the dependency declaration |
| `parse_skill_metadata` | `skills/src/metadata.rs:26-35` | 164 (file) | YAML parse, per-declaration drop-on-invalid |
| `tool_kind_for_path` + sigil consts | `skills/src/mentions.rs:26-55` | 232 (file) | the 4 URI prefixes + `$` sigil |
| `collect_explicit_skill_mentions` | `skills/src/selection.rs:39-…` | 204 (file) | path-first, then text, ambiguity-aware |
| `build_skill_name_counts` | `skills/src/name_counts.rs` | — | the ambiguity detector |
| `parser.rs` (`parse_skill_frontmatter_metadata`) | `skills/src/parser.rs` | 225 | SKILL.md frontmatter |
| `interface.rs` (`resolve_skill_interface`) | `skills/src/interface.rs` | 201 | interface asset policy |
| `loading.rs` | `skills/src/loading.rs` | — | root loader + snapshot cache |

---

## OPERANT-SIDE

### What exists today — and it is larger, in a different shape

**1. The store is 852 LOC, the assembled tool is 3,222 LOC.** `crates/operant-core/src/skills.rs`
(852) + `skill_usage.rs` (1,051) + `skills_guard.rs` (2,675) + `skill_marketplace.rs` (532) =
**5,110 lines**, against codex's whole 2,840-LOC skills crate. Operant's skill surface is not
smaller; it is **differently distributed** — more of the weight is in *guard* and *usage tracking*
than in *selection*.

**2. Declared prerequisites already exist, as a flat list.**
`crates/operant-core/src/skills.rs:18-27`:

```rust
struct SkillFrontmatter {
    name: Option<String>, description: Option<String>,
    version: Option<String>, author: Option<String>, license: Option<String>,
    platforms: Option<Vec<String>>, tags: Option<Vec<String>>, category: Option<String>,
    prerequisites_env: Option<Vec<String>>,
    prerequisites_commands: Option<Vec<String>>,
    metadata: Option<serde_json::Value>,
}
```

and `is_available` (`:218-245`) gates on platform, required env vars, and required commands
(`which::which(cmd)`). So the *"is this skill usable here"* question is answered, in the SKILL.md
frontmatter, without a second YAML file.

**3. Invocation is `/skill name`, not `$name`.** `build_skill_invocation_message`
(`agent/skill_preprocessing.rs:263`) reads the SKILL.md, runs `substitute_template_vars` and
`expand_inline_shell` (`:84`, `:204`) — **inline shell expansion inside skill content**, which
codex does not do — and returns the assembled message. `resolve_skill_dir` (`:276-…`) handles both
the flat and the categorized layout with a path-segment walk.

**4. Provenance is tracked; codex has no analogue.** `SkillOrigin::{OperatorPool, Integration}`
(`skills.rs:35-41`) tags where a skill came from so `create`/`delete` can refuse to touch
vendor-owned integration skills (the ponytail integration, iter-1306+). `skill_bundle.rs`
(`agent/skill_bundle.rs`, 513 LOC) adds a whole bundle-scanning layer with
`resolve_bundle_command_key` and `build_bundle_invocation_message`.

**5. A guard subsystem exists that codex has no counterpart for.** `skills_guard.rs` is 2,675 LOC.

### The gap, stated precisely

| capability | codex | operant | verdict |
|---|---|---|---|
| required env / commands | — | `prerequisites_env`, `prerequisites_commands`, `is_available` (`skills.rs:218-245`) | **operant ahead** |
| required *tools* | `SkillToolDependency` (`model.rs:87-95`) | none | **gap** |
| transport/command/url/oauth for a dep | `model.rs:88-95` | none | **gap, but see below** |
| ambiguous-name rejection | `build_skill_name_counts` + `blocked_plain_names` | `resolve_skill_dir` walks a path; ambiguity not modelled | **gap, low** |
| sigil invocation | `$skill` + `skill://` URI | `/skill name` | **different, both work** |
| inline shell in skill body | no | `expand_inline_shell` (`skill_preprocessing.rs:204`) | **operant ahead** |
| vendor-vs-operator provenance | no | `SkillOrigin` (`skills.rs:35-41`) | **operant ahead** |
| per-skill guard subsystem | no | `skills_guard.rs` 2,675 LOC | **operant ahead** |

### Why the tool-dependency gap is **not** the row it looks like

The `SkillToolDependency` struct carries `transport`, `command`, `url`, and
`oauth_callback_port`. Those fields exist because codex's dependency type can name a **remote
connector** — that is the `connectors/` crate (5,491 LOC, account-scoped app catalog with its own
runtime, metadata store, and MCP tool policy) that this ledger already records as DO-NOT.

The **narrow** version — "this skill needs `web_search` to be available" — maps onto something
operant already has and codex does not: `is_available()` on the *tool* registry
(`crates/operant-core/src/tools.rs:325`, called at `:640`, `:655`, `:686`, `:728`) filters the
toolset by availability, and `tools/tool_search.rs:621-622` gates on it too. So a skill that needs
a tool is checkable today with a registry lookup; what is missing is the **declaration in the
SKILL.md** and the wiring that reads it.

---

## VERDICT

| mechanism | codex unit + LOC | operant equivalent | gap severity | portable? | verdict |
|---|---|---|---|---|---|
| Declared tool dependencies (narrow: "needs tool X") | `model.rs:81-95` (15) | none in the skill file, but tool `is_available()` exists | low | yes, ~40 LOC | **IMPORT-SMALL** (frontmatter field + an availability check at invoke) |
| Dependency transport/command/url/oauth | `model.rs:87-95` | none — needs the connectors crate | none here | no | **DO-NOT** |
| `$skill` / `skill://` sigil | `mentions.rs:26-55` (30) | `/skill name` works, plus bundles | none | n/a | **DO-NOT** — two syntaxes for one action is a redundancy, not an upgrade |
| Ambiguity-aware plain-name resolution | `name_counts.rs` + `selection.rs` | path-walk resolution | low | ~30 LOC | DO-NOT unless a real collision is reported |
| Per-skill SKILL.md frontmatter parse | `parser.rs` 225 | `skills.rs:12-27` | none | — | **ALREADY-HAVE** |
| Vendor vs operator provenance | — | `SkillOrigin` (`skills.rs:35-41`) | operant ahead | — | ALREADY-HAVE |
| Guard subsystem | — | `skills_guard.rs` 2,675 | operant ahead | — | ALREADY-HAVE |

**The ponytail reading:** operant's skills surface is *bigger* than codex's and is ahead on the two
properties that actually protect an operator (provenance and guarding). The one thing worth taking
is the 15-LOC struct as a frontmatter field — "declare the tools this skill needs" — wired to an
`is_available()` check that already exists. Everything else in the family is either already built,
or gated behind the connectors crate this ledger has already rejected.
