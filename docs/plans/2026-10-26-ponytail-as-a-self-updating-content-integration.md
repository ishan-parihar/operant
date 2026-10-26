# ponytail as a self-updating content integration

Dated-frozen: published `2026-10-26`. A plan that changes is a new plan.

Goal: integrate <https://github.com/DietrichGebert/ponytail> into the operant
agentic core runtime so the agent *runs on* ponytail (lazy-senior-dev mode as
a first-class behavior surface) and the integration *stays current* with the
upstream repo, exactly like memory-wire / sourcehound / AFT do — each of those
keeps itself fresh by a mechanism matched to what it is:

| integration | what it is | freshness mechanism |
|---|---|---|
| memory-wire | in-process cargo git-dep | persistent-latest lock sync at every build (`scripts/build.sh`), locked rev baked into the binary, `operant doctor` compares vs upstream main |
| sourcehound | binary on PATH, driven over its MCP server | runtime `binary_updater` check (6 h throttle, once-guard), first-use kick, SHA256-verified release asset, atomic replace |
| AFT | versioned cache-dir of binaries | same `binary_updater`, `InstallKind::CacheDir`, prune keep=2 |
| **ponytail (this plan)** | **content package: markdown skills + command prompts, NO binary** | **new `InstallKind::Content` on the same `binary_updater`** + a pinned-tarball download + GuardScanner sweep + config-gated always-on prompt injection |

## What ponytail actually is (audit result)

Audited 2026-10-26 against `DietrichGebert/ponytail` @ `v5.3.2`
(tag commit `16352ac`, npm `@dietrichgebert/ponytail` 5.3.2). It is **not a
library and not a binary** — it is one prompt shipped in many wrappers:

- `skills/ponytail/SKILL.md` — the core prompt ("lazy senior dev", the
  option ladder, never-cut list, levels lite/full/ultra). YAML frontmatter
  (`name`, `description`, `argument-hint`, `license`). **Directly parseable
  by `operant_core::skills::SkillManager`** (it reads `name`, `description`,
  plus nested `metadata.operant.*`; the extra keys ride as plain body text —
  verified against `skills.rs` parse logic).
- `skills/ponytail-{review,audit,debt,gain,help}/SKILL.md` — five
  auxiliary skills (review/audit prompts, debt-ledger harvest, scoreboard,
  help). Same frontmatter shape.
- `commands/*.toml` — six slash-command prompts (host-specific prompt
  templates with `{{args}}` substitution). Operant already has the exact
  equivalents as SKILL.md bodies, so the commands are NOT ported as a
  separate surface: each command body rides inside its skill's
  `description`-adjacent content, invoked through the existing
  `/skill ponytail-review` path. (Rationale: a second command TOML surface
  would duplicate `skill_bundle`/`build_skill_invocation_message_in`; the
  skills path already names them and the TUI `/skill` typeahead already
  completes them.)
- `hooks/*.js` — node lifecycle hooks for Claude Code/Codex/Cursor/Qoder
  band UI, mode tracking, statusline. **Not ported**: operant has no node
  runtime in its loop, the band is host UI, and operant's mode persistence
  is natively simpler (config file + runtime override, below). Listed here
  so a future agent does not "rediscover" them.
- `AGENTS.md` — compact rules-only variant. **Not shipped into the prompt
  verbatim**: the always-on injection uses the full SKILL.md, which
  strictly contains the compact variant's content.
- Releases carry **no assets** (`assets: []`) — the tarball/zipball at the
  tag IS the payload. This is why `binary_updater` needs a new install kind:
  `pick_asset` on an empty asset list returns `None` → `Available` notice,
  never an install. The content path must fetch
  `https://api.github.com/repos/DietrichGebert/ponytail/tarball/vX.Y.Z`
  (public, no gh auth needed) instead of a release asset.

Fork/steal score: 176k stars, MIT license — upstream is healthy; integration
is a follower, not a fork. npm `@dietrichgebert/ponytail` exists but the
GitHub release tarball is the cleaner fetch (no node, no npm runtime).

## Where it lands in operant (surfaces mapped)

1. **Storage**: `~/.operant/integrations/ponytail-vX.Y.Z/skills/ponytail*/`
   — versioned cache-dir, same shape as AFT's `aft-vX.Y.Z/`
   (`platform::operant_home().join("integrations")`, prune keep=2). NOT
   `~/.operant/skills/`: that dir is the operator's skill pool
   (seeded, listed, `/skills` overlay) and an auto-updated vendor tree must
   not overwrite operator skills on any cadence; the frozen-prefix builder
   reads the versioned dir and a copy-once pointer (below) keeps the
   versioning honest.
2. **Freshness**: new `binary_updater::InstallKind::Content { dir, keep }`
   — same IntegrationSpec ("ponytail", repo "DietrichGebert/ponytail"), same
   `check`/`update`/`throttle`/`kick_session_update`, but install = fetch
   tarball → verify → extract skills/ → prune. Wired into
   `operant update --integrations` (`cmd_update.rs`) and `operant doctor`
   (integrations section, `checks_tools.rs` — the memory-wire row's
   pattern: installed vs latest with the "run scripts/build.sh"-style
   hint).
3. **Skill exposure**: the agent's `SkillManager` in `main.rs:1639` /
   `gateway_runner.rs:943` gains the ponytail skills as *additional* load
   roots (one manager, extra roots — no second manager, no second scan).
   `/skill ponytail`, `/skill ponytail-review` etc. work immediately; the
   `/skill` Tab-typeahead lists them; `operant skills list` shows them
   (marked `[integration]`). `skills_guard::scan_skill` runs on first
   download AND on every background re-download (upstream prompt text is
   third-party content; a Dangerous verdict refuses the update and keeps
   the previous version).
4. **Behavior surface (the actual point)**: a `[behavior] ponytail` config
   section (`config.rs` BehaviorSettings-adjacent, `operant.example.toml`
   in the same change):
   - `enabled` (default **false** — opt-in; the operator's design
     preferences stay intact, Holon runs must not silently change posture)
   - `default_level` = `full` | `lite` | `ultra` (README's three levels)
   - `inject` = `always_on` (frozen prefix) | `on_invoke` (skills only)
   - When `inject = always_on`: the active level's ruleset is appended to
     the frozen prefix AFTER `<available_skills>` (same place
     `SKILLS_GUIDANCE` rides, `agent/events.rs::build_frozen_prefix`) so
     cache discipline holds — byte-stable per level switch, level switches
     are rare config changes.
   - `/ponytail <level>` TUI command flips a *runtime* level override
     (session-scoped, mirrors the daemon-kill pattern of a process-local
     state cell + config default at boot) — **do not** wire this through
     `HookEvent`s; ponytail's level is prompt state, not a hook.
5. **Subagents/turn state**: nothing. Ponytail's own hooks push rules into
   subagents on other hosts; operant's background-review fork already
   inherits the frozen prefix, so an always-on ponytail reaches it for
   free. No new mechanism.
6. **Cron/gateway seats**: nothing new. Gateway agents construct the same
   SkillManager roots, so `/skill ponytail-review` works from any seat;
   always-on posture follows config, per-seat.

## Phases (each is one iteration, commit + push, deploy per R&D protocol)

### Phase 1 — `InstallKind::Content` in `binary_updater` (core, no UI)

- Add enum variant `InstallKind::Content { dir, keep }` + install logic:
  fetch tag tarball (public api.github.com tarball_url; no `gh` needed —
  add a public-release fetch path alongside the private `gh` path, or a
  tarball-URL hint in the spec), extract ONLY `*/skills/*/` entries into
  `<dir>/ponytail-vX.Y.Z/skills/` (strip the `DietrichGebert-ponytail-<sha>/`
  prefix GitHub tarballs carry), refuse `.exe`/`.dll`/ELF entries (README's
  own authenticity rule + guard parity), prune keep=2.
- Version source: the release tag (`v5.3.2`), parsed by the existing
  `parse_version`. No package.json read needed (tag is authoritative and
  simpler; note the npm package exists as an alternative source if GitHub
  tarballs ever 404).
- Unit tests: tarball-prefix strip, binary-entry refusal, prune, version
  compare, `pick_asset` empty-assets → Available (already-covered
  behavior), public-tarball URL shape. Fake-fetch test per the
  `check_resolves_versions_through_a_fake_gh` precedent (no network in
  tests).
- Acceptance: `cargo test -p operant-core --lib` green; clippy gate green;
  **no behavior change** to existing sourcehound/AFT paths (same IntegrationSpec
  struct, additive variant).
- Files: `crates/operant-core/src/binary_updater.rs` (+ its tests),
  possibly a tiny `ponytail_spec()` in a new
  `crates/operant-core/src/integrations/ponytail.rs` or inline in
  `aft_bridge.rs`-style module. Keep it ONE module.

### Phase 2 — CLI surface: `operant update --integrations` + doctor row

- `cmd_update.rs::cmd_integrations` adds the ponytail check/update
  alongside sourcehound/AFT (the `report` closure already handles
  Updated/Available/Failed/None).
- `cmd_doctor/checks_tools.rs` integrations section: ponytail row —
  installed version (highest cached `ponytail-v*` dir) vs latest tag, with
  the sourcehound-style "not installed — run operant update
  --integrations" hint when absent.
- **First-install kick**: where sourcehound's `kick_update_check` fires
  (first use per session), ponytail's kick fires at CLI startup when
  `[behavior].ponytail.enabled = true` — because content, unlike binaries,
  has no natural "first tool call" moment; startup is the natural gate.
  Throttle/once-guard/`OPERANT_NO_AUTO_UPDATE` all inherited.
- Acceptance: fresh box → `operant doctor` shows ponytail "not installed";
  `operant update --integrations` installs v5.3.2; doctor shows up-to-date;
  re-run is a no-op ("up to date"). Mutation-style test: version dir
  rename → doctor reports stale.
- Files: `crates/operant-cli/src/cmd_update.rs`,
  `crates/operant-cli/src/cmd_doctor/checks_tools.rs`,
  `crates/operant-core/src/integrations/ponytail.rs` (spec + kick).

### Phase 3 — skill exposure: extra SkillManager roots + guard on download

- `SkillManager::load_all` gains optional extra roots (additive field,
  default empty — existing behavior unchanged). Wire in `main.rs:1639` +
  `gateway_runner.rs:943`: append the ponytail versioned skills dir when
  it exists.
- `skills_guard::scan_skill` on the extracted skills at download time
  (Phase 1 install path calls it; Dangerous verdict → abort install,
  keep prior version, surface `UpdateNotice::Failed`).
- `operant skills list` marks them; TUI `/skill` typeahead includes them
  (it derives from skills_view, which re-scans — verify the existing flow
  covers extra roots after `reload-skills`).
- Acceptance: with ponytail installed, `/skill ponytail-review` injects
  the real upstream prompt (integration test: build invocation message,
  assert body contains "senior developer who will be paged" +
  frontmatter name parse). Unknown-command fall-through untouched
  (`no_registered_tui_command_is_unwired` stays green).
- Files: `crates/operant-core/src/skills.rs`,
  `crates/operant-core/src/integrations/ponytail.rs`,
  `crates/operant-cli/src/main.rs`, `crates/operant-cli/src/gateway_runner.rs`.

### Phase 4 — behavior surface: `[behavior] ponytail` config + frozen-prefix injection

- `config.rs`: `PonytailSettings { enabled: bool (default false),
  default_level: PonytailLevel (default Full), inject: PonytailInject
  (default OnInvoke) }` + `operant.example.toml` rows in the same change
  (AGENTS.md rule: config fields → example toml same change).
- `agent/events.rs::build_frozen_prefix`: when enabled AND
  `inject = always_on`, append `<ponytail>` block with the ACTIVE level's
  ruleset (full SKILL.md body for `full`; the ladder-restricted variant —
  upstream's lite/ultra deltas are two sentences each, carried as a level
  suffix line, e.g. "Level: lite — build what was asked; name the smaller
  option" / "Level: ultra — also question the request before building").
  **Level switches change the prefix bytes** → cache invalidation is a
  documented consequence, not a bug (same class as a config change).
- `/ponytail [lite|full|ultra|off]` TUI command: sets a process-local
  runtime override (the `config_shim` process-global cell pattern with its
  `reasoning_override_test_guard` discipline), prints the level, `off`
  clears injection. Registered in the command registry so the
  not-wired catch-all test stays green.
- Where NOT to inject: Holon runs (`is_holon_run()` — the org brief is the
  only guidance; personal postures must not leak into org bodies — same
  call `load_context_file_prompt` already makes for owner context). This
  is a design preference, DO NOT ship silently: call it out in the
  example toml comment.
- Acceptance: config-driven prefix test (enabled+always_on → prefix
  contains `<ponytail>`; disabled → byte-identical to today's prefix);
  level-override test; `cargo check -p operant-cli --bin operant`.
- Files: `crates/operant-core/src/config.rs`,
  `crates/operant-core/src/agent/events.rs`,
  `crates/operant-cli/src/tui/operant_app/config_shim.rs` (or a sibling
  cell), `crates/operant-cli/src/tui/app/commands.rs` (+ a row in the
  TUI command registry definition), `operant.example.toml`.

### Phase 5 — docs + ledger

- `docs/CHANGELOG.md` entry; `README.md` skills/tools section mentions
  ponytail; `AGENTS.md` "Design Preferences" gains a "Ponytail" row
  (default OFF, opt-in posture, never auto-enable — the operator's
  standing preference for that surface).
- `docs/kb/feed.md`: post completion. `TODO.md`: no change (autonomous
  ledger) unless behavior changes affect it.
- Acceptance: `./scripts/genome lint` green; doc gates green.

## Coordination (fleet rules)

- **Peer WIP conflict, Phase 3/4**: the RT-18x memory/sourcehound leg (F0056)
  is editing `agent/events.rs` + `agent/builders.rs` + `agent/mod.rs` +
  `gateway_runner.rs` in the shared tree right now. Phases 3–4 touch the
  same files. Do NOT start Phase 3/4 until that leg's changes are pushed
  or the overlap is coordinated in the feed (`genome post`); the frozen-prefix
  seam is exactly the kind of shared surface rule 3 exists for. Phases 1–2
  are conflict-free (`binary_updater.rs`, `cmd_update.rs`, doctor).
- Stage explicit paths only; never `git add -A` (the F0038/F0053 lesson).
- One iteration per phase, push after each, deploy release build per R&D
  protocol after code phases.
- `Cargo.lock` is derived; if a peer's lock churn appears, `git checkout --
  Cargo.lock` per fleet rule 4.

## What this plan deliberately does NOT do (YAGNI, ponytail-style)

- No npm/node runtime, no `hooks/*.js` port, no band UI, no statusline.
- No port of `commands/*.toml` as a separate command TOML surface — the
  skills cover them.
- No marketplace registry entry (`skill_marketplace.rs` stays as-is;
  ponytail is an integration, not a registry skill).
- No i18n, no examples dir, no benchmarks port.
- No `PONYTAIL_DEFAULT_MODE` env var in v1 (config covers it; add later
  if an operator asks).
- No `plugin.json`/`plugin.yaml` manifest parsing — the GitHub tag tarball
  is the version source and the skills/ dir is the payload; the manifests
  describe OTHER hosts' loading conventions.

## Open questions for the operator (non-blocking, defaults chosen)

1. Default `inject` mode: `on_invoke` chosen (skills only, zero prompt
   change) — always-on changes every prompt's bytes and is the operator's
   posture call. Flip `inject = "always_on"` in config to opt in.
2. Should `/ponytail` runtime overrides persist across restarts? Default:
   no (session-scoped, matching upstream's Hermes runtime mode note
   "runtime mode is process-local"). Config sets the durable default.
3. Purge cadence: keep=2 (matches AFT). Fine for content.
