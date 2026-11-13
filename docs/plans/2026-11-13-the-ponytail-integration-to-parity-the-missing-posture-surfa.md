# the ponytail integration to parity — the missing posture surface (upstream v5.3.3)

Dated-frozen: published `2026-11-13`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## The request

> Check https://github.com/DietrichGebert/ponytail to understand how to integrate it
> effectively, and create a plan to do that effectively.

## What upstream IS — the integration model (measured against v5.3.3, 2026-10-29)

Upstream's own README: *"Ponytail is one prompt: `skills/ponytail/SKILL.md`. The compact
version, for agents that read a rules file, is `AGENTS.md`. Everything else in this repo
loads that prompt into different agents."* There is NO binary and the releases carry NO
assets — the tag tarball IS the payload.

| upstream artifact | what it is | operant's equivalent today |
|---|---|---|
| `skills/ponytail/SKILL.md` | the ONE prompt (the ladder, the never-cut list, the three levels) | the `ponytail` skill + the `<ponytail>` block in the frozen prefix ✓ |
| `skills/ponytail-{review,audit,debt,gain,help}/SKILL.md` | five auxiliary skills | the same five, installed, invocable, and reachable as commands ✓ (iter-2159) |
| `commands/*.toml` | eight slash commands: the level switch plus the five skills | the same eight NAMES ✓ (iter-2159) — the level switch's SEMANTICS are missing (GAP 1, GAP 2) |
| `AGENTS.md`, `.agents/rules/`, `.cursor/rules/`, `.windsurf/rules/`, `.kiro/steering/` | the compact ruleset for instruction-only hosts | not needed: operant injects the full SKILL.md, whose body strictly contains the compact variant ✓ (8 of 9 paragraphs verbatim; the 9th is only the title) |
| `hooks/*.js`, `assets/v5/`, the band | Claude Code's node lifecycle hooks and its UI | out of scope by design (the 2026-10-26 integration plan's YAGNY list) |
| `i18n/README.{es,ja,ko,zh-CN}.md`, the other hosts' plugin manifests | other hosts' packaging | none — operant has its own plugin and packaging story |

Latest upstream release: **v5.3.3**. Operant has v5.3.3 installed
(`operant doctor` → `✓ ponytail 5.3.3 (up to date)`).

## What operant already does (measured)

- the six skills install from the TAG TARBALL, are community-guard-scanned, and live
  under `~/.operant/integrations/ponytail-vX.Y.Z/skills/` ✓ (never in the operator pool)
- a skill NAME is a command: `/ponytail-audit` runs the skill in the TUI and over a
  channel, in the hyphenated spelling and in Telegram's underscored one ✓ (iter-2159)
- the vendor skills are listed as commands in the TUI palette and typeahead, in the
  gateway's `/help`, and in the Telegram command menu ✓ (iter-2159)
- the posture rides the frozen prefix as a `<ponytail>` block when
  `[agent.ponytail] enabled = true` and `inject = "always_on"` ✓
- ONE root list decides what a skill is (the integration + the plugin roots) and ONE
  resolver decides where it lives ✓ (iter-2159)

## The gaps this plan closes

### GAP 1 — a bare `/ponytail` does not switch it ON

upstream (`README.md`, Commands): *"Set the intensity, or turn it off. **No argument
switches ponytail on at the default level if it is off**, and otherwise reports the
current level."*

operant today: the TUI's `/ponytail` with no argument only REPORTS — the notice reads
`ponytail: {level} (runtime override; config: enabled=…, inject=…)`
(`crates/operant-cli/src/tui/app/commands.rs:1884`). An operator who types `/ponytail`
expecting upstream's behaviour gets a status line and the mode stays off.

### GAP 2 — a channel user cannot switch the posture, and the switch is process-global

upstream's Hermes note (`after-install.md`): *"In shared gateways, restrict `/ponytail`
to trusted users with Hermes slash-command access controls; runtime mode is
process-local."* That is upstream WARNING about a process-local runtime mode in a shared
gateway.

operant today: there is NO `/ponytail` command in the gateway's registry
(`crates/operant-cli/src/gateway_commands.rs` — the registry has `skills` with the alias
`skill`, and nothing for the posture), and the runtime override is a process-global
static (`crates/operant-core/src/ponytail.rs:85`, `set_posture_override` at `:88`,
`posture_override` at `:95`). So a TUI switch flips the posture for EVERY chat in the
daemon, and a channel user can neither set nor clear it for their own chat.

### GAP 3 — the content refresh is gated on the posture switch

operant today: the startup kick is
`if config.agent.ponytail.enabled && !is_holon_run() { kick_update_check() }`
(`crates/operant-cli/src/main.rs:1806`), so with the shipped default
`enabled = false` the CONTENT never refreshes — the installed version drifts from
upstream while `operant doctor` keeps reporting the drift.

upstream: the skills update independently of the mode; the mode only decides whether the
prompt is injected. A content integration that stops updating when its posture is off
is a broken integration, not a quiet one.

## NOT gaps (verified — do not spend an iteration on them)

- the review/audit TARGET grammar (`uncommitted`, `staged`, `branch`, a PR link, a folder)
  is in the installed SKILL.md bodies ✓, and operant passes the command's argument as the
  skill's instruction ✓ — `/ponytail-review staged` already means "review the staged
  changes"
- the compact ruleset is a strict subset of the injected body ✓
- the band, the node hooks, the i18n READMEs and the other hosts' plugin manifests have no
  operant equivalent by design ✓

## The phases

### Phase 1 — the content refresh must not depend on the posture

Split the two decisions the one flag currently fuses: the POSTURE (`enabled`,
`default_level`, `inject`) and the CONTENT refresh (a separate gate, defaulting ON).
The smallest shape: `[agent.ponytail] update = true` (default true) gates
`kick_update_check()`; `enabled` keeps gating the INJECTION only. `operant doctor`'s
ponytail row already reports installed-vs-latest in both states ✓.

Acceptance: with `[agent.ponytail] enabled = false`, `operant doctor` still compares the
installed version with the latest tag, the startup kick still runs, and
`operant update integrations --only ponytail` still refreshes the content alone
(iter-2139/2159 shipped that flag).

### Phase 2 — a bare `/ponytail` switches ON at the configured default level

In the TUI arm (`tui/app/commands.rs:1884`): with no argument, if the effective posture
is OFF, set the runtime override to the configured `default_level` and say what changed;
if it is already on, report the level (upstream's second clause). `/ponytail off` clears
it; `/ponytail <level>` sets it. The notice text names the state it left behind, so the
operator is never guessing.

Acceptance: a test pins the matrix — (off + bare) → on at the default level; (on +
bare) → reports; (`off`) → off; (`ultra`) → ultra. The same matrix must be observable in
the TUI and over a channel (Phase 3 reuses it).

### Phase 3 — a per-chat `/ponytail` in the gateway

Two parts, and the second is the real one:

1. the command: add `ponytail` to the gateway's command registry with
   `args_hint = "[lite|full|ultra|off]"` and the upstream semantics from Phase 2; gate it
   to admins on a shared gateway (upstream's access-control note), and say so when it is
   refused.
2. the SCOPE: the override must stop being a process-global static. The gateway already
   scopes per-chat state through the session (the same mechanism `/effort` uses), so the
   posture belongs there: a per-session override map, with the process-global static kept
   as the TUI's (single-session) fallback. A chat's `/ponytail off` clears THAT chat.

Acceptance: a test drives two sessions in one gateway process — chat A sets
`/ponytail full`, chat B is unaffected; a non-admin is refused on a shared gateway; the
TUI's switch still applies to the TUI's own session.

### Phase 4 — the acceptance gate (live, operator-visible)

1. `operant doctor` → ponytail up to date WITH the posture off
2. TUI: `/ponytail` bare turns it on at the default level; `/ponytail` again reports;
   `/ponytail off` clears it
3. channel: `/ponytail full` changes THAT chat only; another chat is unaffected
4. `/ponytail-audit` and `/ponytail-review <target>` run the skill in both surfaces
5. the Telegram command menu lists the six skill commands (already true — the pin stays)

## Constraints that must survive every phase

- ONE root list and ONE resolver decide what a skill is and where it lives (iter-2159) —
  no second copy may appear
- a Holon run carries no vendor root and no posture
- the operator's pool always wins a name collision; the vendor skills stay read-only
- the prompt cache: the posture level and its override change the frozen prefix bytes
  (already documented); no phase may add per-turn bytes
- no new dependency, no node runtime, no band, no second packaging path for the same
  content (operant's own plugin system could host the same markdown — the content
  integration is the ONE path; a plugin install would be a redundant second one)

## Cost

- Phase 1: one config field + one gate + a doctor/config test (≈ 40 lines)
- Phase 2: one TUI arm + one test (≈ 30 lines)
- Phase 3: one registry entry, one per-session override map, the gateway arm, three tests
  (≈ 120 lines) — the only phase with a real design decision (where the session scope
  lives)
- Phase 4: no code; the live checks above

## Gates / success criteria

(fixed before the run)

- `cargo clippy -p operant-core --all-targets -- -D warnings` and
  `cargo clippy -p operant-cli --all-targets -- -D warnings` clean
- `cargo test -p operant-core --lib` and `cargo test -p operant-cli --bin operant` no
  NEW failures against the recorded baseline (the core's 14 environment failures and
  the CLI's flake class are the baseline, not this plan's debt)
- every acceptance item above observed once on the DEPLOYED binary, not only in a test
- the release build + deploy + gateway restart per AGENTS.md §7, with the provenance
  gate's mtime AND content checks (a fleet deploy can ship an older branch — re-check
  the surface after every deploy)

## Not checked

- the Discord and Slack command surfaces (they share the gateway's dispatcher, but
  their command menus are not Telegram's)
- the ACP daemon's command surface (new in iter-2118)
- the upstream's `hooks/*.js` and the band (out of scope by design)
