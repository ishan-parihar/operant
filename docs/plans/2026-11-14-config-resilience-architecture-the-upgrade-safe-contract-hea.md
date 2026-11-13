# Config resilience architecture: the upgrade-safe contract, heal tables, describe/notes, and the doctor sweep

Dated-frozen: published `2026-11-14`. A plan that changes is a new plan (or declare it in
genome.toml `core.living_plans` and keep a 'Resume here' section).

> **Amended `2026-11-14` (same day, before implementation began).** §5 — the touch-point map and the forward gate — was added at the operator's request: how can an agent developing a new config feature know everything it must update, and is that enforced at the architectural level? Everything above §5 is the original published plan, unchanged.


**Question this answers.** "How does the config survive upgrades in operant's own architecture —
a new feature added to a newer binary, an older config file deployed, a key renamed, a value
healed?" The survey that grounds it: `docs/audit/2026-11-14-the-config-surface-survey-tooling-healable-set-missing-manag.md`.

## Mechanism

One resilience contract, four layers. Every key, in every settings struct, obeys the same rules;
every heal is *named*, *logged*, and *persisted once*; every key is *describable* to an agent.

```
L1 schema   : typed, serde default, doc comment, range, healing note
L2 heal     : heal() -> Vec<Healed{key, old, new, why}>  (idempotent, total over the tree)
              load -> heal -> persist-once -> one warn! line naming every healed key
L3 describe : operant config describe | config notes <key>  (+ config_manage parity)
L4 doctor   : the upgrade sweep — unknown keys, deprecated keys, missing sections,
              the two-file split, the unparseable-file backup; report + heal the safe ones
```

### L2 — the heal tables (the core)

Every settings struct gets `fn heal(&mut self) -> Vec<Healed>`; `AppConfig::heal()` sums them.
`Healed { key: &'static str, old: String, new: String, why: &'static str }`. Rules:

- **Missing key** → filled from `Default`; healed row says `missing -> default`.
- **Out-of-range** → clamped; row says `120 -> 300 (min 1)`.
- **Conflicting pair** → resolved by the documented precedence; row names both.
- **Unknown key** (the upgrade case) → **parked**, not dropped: `[__unknown]` in the TOML (or a
  sidecar — see the open decision below), with a warning naming it. The load never fails on it.
- **Idempotent**: a second load heals nothing; the file is rewritten exactly once, atomically,
   and only when the heal changed something.

Every heal is a *named row*, which is what "the tooling must give the agents relevant context"
means made concrete: an agent reads *what was healed and why*, not just that something was.

### L3 — describe / notes (the agent-facing surface)

- `operant config describe` → every key: dotted path, default, the doc comment (the note), and
   `default | non-default`.
- `operant config notes <key>` → the full note for one key (why it exists, the range, the heal).
- `config_manage` parity: `describe` and `notes` actions, so an agent that only has the tool can
   read the same map. The note text is the *single source*: the doc comment, the example TOML,
   and the describe output are generated from it, never hand-copied.

### L4 — the doctor sweep (the healable set, named)

| Row | Fires when | Heals? |
|---|---|---|
| `config.unknown-key` | a key the current binary does not know | park + warn |
| `config.deprecated-key` | a key renamed/removed in this version | migrate 1:1 where possible, else warn |
| `config.missing-section` | a section new in this version is absent | fill defaults + note |
| `config.two-files` | both `config.toml` and `operant.toml` exist | warn, name both, never merge |
| `config.unparseable` | the file fails to parse | back it up (`operant.toml.bak-<ts>`), fall back to defaults, name the backup |
| `config.inert` | a known-but-unread key is SET (today's `unused_setting_warnings`) | report (existing) |
| `config.range` | a value out of range (today's ad-hoc clamps) | clamp + row |

The doctor is the natural home because it already runs on demand and already has the
`check_config_semantics` category; the sweep is a new `check_config_resilience` pass.

## Gates / success criteria (fixed before the run)

1. A config with a missing key loads, heals it, and the file on disk contains the healed value
   after one `load_app_config()` — asserted by reading the file back.
2. A config with an unknown key (from a "future" version) **loads** on this binary, the key
   survives a round-trip (parked), and the warning names it.
4. `heal()` is idempotent: a second call returns an empty `Vec<Healed>`.
5. `operant config describe` lists every field of `AppConfig` (source-grep contract: the set of
   keys in `describe` equals the set of fields in the structs).
5. `config notes <key>` returns the same text as the doc comment (generated, not hand-copied).
6. The doctor's sweep is silent on a default config and fires one named row per crafted defect.
7. The unparseable-file case leaves a backup and a named log line, never a silent default.

## Open decision (the one choice the operator must make)

**Where do unknown keys live?** `[__unknown]` in the operator's TOML (one file, but the file
carries the parked junk) **or** a sidecar `~/.operant/operant.unknown.toml` (the operator's file
stays clean, but the state is in two files). The plan is complete either way; the choice only
moves where the parking happens.

## Cost

One settings-wide refactor (a `heal()` per struct + the `Healed` type + the persist-once path),
two new CLI verbs (`describe`, `notes`), one doctor pass, and the tests above. Estimated at two
to three iterations; the retry policy (`docs/plans/2026-11-14-agentic-retry-...`) is the first
consumer of the heal table and lands as its own iteration.

---

## 5. The touch-point map — and the gate that makes it architectural

**The question.** "How does an agent adding a new config key know *everything* it must update?"
Today the honest answer is: it does not, unless it already knows the map. The compiler forces
exactly one thing (the struct field); three partial gates exist; everything else is convention.
This section names the map and the gate that makes it a build failure.

### The map — the twelve places a new key must reach

| # | Touch-point | Owner of the rule | Enforced today by |
|---|---|---|---|
| 1 | the settings struct field | `config.rs` | **the compiler** (a missing field in an exhaustive literal; `..Default::default()` hides it) |
| 4 | the `Default` impl value | `config.rs` | the compiler (exhaustive literals) — not the value |
| 4 | the note (the doc comment, the single source) | `config.rs` | **nothing** — convention only |
| 5 | the `heal()` entry | `config.rs` | **nothing** — the L2 contract, not yet built |
| 6 | `operant.example.toml` (the operator's menu) | the template | **reverse-only**: `tests/example_template_loads.rs` fails a template that advertises an UNKNOWN key; it does NOT fail a struct key the template omits |
| 7 | `scripts/audit-config-surface.py` `ACCOUNTED_FOR` | the surface gate | the gate itself (only when the field has NO reader) |
| 8 | the doctor's `check_config_semantics` | `doctor.rs` | **nothing** — convention only |
| 9 | `config describe` / `config notes` | the plan's L3 | **not built** |
| 10 | `docs/RUNBOOK.md` (env vars, exact commands) | the runbook | **nothing** — `scripts/check-doc-claims.sh` checks AGENTS.md, not the runbook |
| 11 | `docs/CHANGELOG.md` (release history) | the changelog | **nothing** — convention |
| 12 | `docs/library/*.md` (the mechanism doc) | `genome lint` (one owner per question) | the doc laws, not the key |

### What enforces it today — the honest table

- **The compiler**: the field and (only in exhaustive literals) the value. It does not know about
  the note, the template, the heal entry, the doctor, or any doc.
- **`tests/example_template_loads.rs`**: reverse-only. A key the template OMITS is invisible to
  every gate — the template can silently drift behind the struct.
- **`scripts/audit-config-surface.py --check`**: forward, but only for *unread* fields (the
  `ACCOUNTED_FOR` baseline). A new key that IS read passes it without touching any other surface.
- **`scripts/check-doc-claims.sh`**: AGENTS.md claims only.
- **`genome lint`**: the doc laws (one owner, mutability, no deletions, statuses, claims). Nothing
  about keys.

**Verdict: the touch-point set is NOT enforced at the architectural level.** An agent can add a
key that compiles, passes every gate, and is invisible to the operator (no template entry), to the
doctor (no row), to the heal contract (no entry), and to the describe surface (none). That is the
gap this section closes.

### The gate — one registry, one forward check

1. **The registry is the struct's key set**, generated, not hand-maintained: a
   `#[derive(...)]`-driven (or build-script-driven) dump of every settings field with its dotted
   path, its default, and its note. The note is the doc comment — the single source the L3
   describe/notes surface already requires, so the registry and the describe output cannot drift.
2. **`scripts/audit-config-keys.py --check`** — the forward gate. It fails when a key in the
   registry is missing from ANY required surface: the `heal()` entry, the template, the doctor
   row (or an explicit "no row needed" marker), and the describe output. It also fails the
   reverse (a template key not in the registry — which `example_template_loads.rs` already
   catches at test time; the gate makes it a lint failure too, so it fires without a full test
   run).
4. **Wiring**: `scripts/check.sh check` runs it (the same place `audit-config-surface.py` runs),
   and the pre-commit `docs-check` runs it, so an agent that adds a key without the note, the
   heal entry, the template line, and the doctor row **cannot commit**.
5. **Acceptance criterion** (this plan's gate): adding a key to `AppConfig` and its `Default`
   **without** the note, the `heal()` entry, the template line, and the doctor row fails
   `audit-config-keys.py --check` with one named row per missing surface. Adding all four passes.
