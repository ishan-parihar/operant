# The config surface survey: tooling, healable set, missing management, and upgrade resilience

Dated-frozen: published `2026-11-14`. A survey is dated-frozen: a correction is a NEW audit.

> **Amended `2026-11-14` (same day).** §5 — the enforcement survey — was added at the operator's request, answering which gate (if any) forces an agent to touch the whole config surface. The survey above is the original published text, unchanged.

**Question this answers.** "What does the config surface actually have, what is missing, and is
the architecture resilient to upgrades in its own architecture?" Every claim below cites the
line that makes it true on `origin/main` at iter-2217 (`d3eb7a0c`), read on 2026-11-14.

---

## 1. The load path (what happens when the binary starts)

`config::load_app_config_impl` (`crates/operant-core/src/config.rs:2782`) is the single loader.
In order:

1. **Resolve the path** — an explicit `--config` wins; otherwise the first existing file of
   `default_config_paths()`. A MISSING file is not an error: it falls back to
   `AppConfig::default()` and records the config-dir path as the persistence target
   (`config.rs:2812-2829`). A **present but unparseable** file is a hard `?` error
   (`parse_config_file_without_recording(path)?`) — no fallback, no backup.
2. **Tighten permissions** — `fs_secrets::tighten_if_loose(&path)` (plan 002, `config.rs:2808`).
3. **Ad-hoc normalizations, scattered, none persisted:**
   - `[harness].enabled || [tools.kernel].enabled` alias OR (`config.rs:2838-2841`);
   - `[holon]` `~` expansion (`config.rs:2865`);
   - `[skills]` `~` / `$HOME` expansion (`config.rs:2868`);
   - `record_loaded_config_path(src)` so `persist_config_key` targets the right file
     (`config.rs:2883-2887`).
4. **The two-file guard** — `ignored_config_files(chosen)` warns once per process when a
   `config.toml` sits beside the loaded file (ENV-3, `config.rs:2843-2860`); the sibling is never
   read. A warning, not a guard.
5. **Dead-key reporting** — `unused_setting_warnings()` (`config.rs:3407`) prints one line per
   known-but-unread key that is SET (the `[tui]` block, the six phantom
   `[auxiliary_models.*]` slots). It is NOT an unknown-key detector — see §3.

**There is no `migrate()` on the load path.** The scattered normalizations above are all the
healing that happens on load, and none of it persists. `operant config migrate` (the CLI) is a
different thing: a **version-field** migration (`cmd_config.rs:368-428`, v0→v1→v2, the MCP
restructure) that stamps `version = 2` — not a key-level heal.

## 2. The tooling an agent has for the config

| Tool | Surface | Persists? | Notes / describe? |
|---|---|---|---|
| `config_manage` (agent tool) | `get` / `set` / `show` / `path` / `reload` | **no** — `set` mutates the live `RwLock` only; a restart loses it | no |
| `operant config set` (CLI) | validated + persisted write | yes (atomic) | no |
| `operant config show` / `path` / `check` / `edit` / `env-path` / `migrate` | the CLI subcommands (`cmd_config.rs:14-42`) | `migrate --apply` writes `version` only | no |
| `operant doctor` | `diagnose` → `check_config_semantics` (API key, default model, inert slots) | no | no heal rows, no unknown-key rows |
| `scripts/audit-config-surface.py` | a dev-time source gate (`ACCOUNTED_FOR` baseline) | n/a | not available to a running agent |

**The missing surface, named:** there is no `config describe` (every key, its default, whether
non-default), no `config notes <key>` (the doc comment for one key), and no heal visibility
(what the loader changed and why). The notes live in doc comments that only a source reader sees.

## 3. The upgrade cases, and what actually happens

| Case | Behaviour today | Resilient? |
|---|---|---|
| A key is missing (older config, newer binary) | serde `default` fills it; silent | yes, invisibly |
| A key is **unknown** (newer config, older binary) | `deny_unknown_fields` (58 occurrences across the settings structs) → **the load fails** | **no** — an older binary refuses a newer config |
| A value is out of range / conflicting | serde default, then the ad-hoc steps; `validate_context_window` (`config.rs:3094`) is one specific validator | partial |
| A whole section is new | defaults; no note | silent |
| A key was renamed/removed | becomes "unknown" → the load fails (same as above) | **no** |
| The file is unparseable | hard `?` error; the previous good copy is not kept | **no** |
| Two config files exist | `ignored_config_files` warns once per process; the sibling is never read | warning only |
| `set` from the agent tool | mutates the live config, never the file | the operator loses it on restart |

The measured trap that motivates all of this: the 2026-10-16 finding (recorded in
`resolved_config_path`'s history and the ENV-3 note) — a new `config.toml` beside the live
`operant.toml` silently split agent vs gateway settings until an agent deleted the sibling. The
warning exists; the guard does not.

## 4. What this survey says the plan must deliver

1. **Persist the heal.** The loader already computes a normalized config; write it back once,
   atomically, with a single log line naming every healed key. (Today: silent, in-memory, lost.)
2. **Tolerate unknown keys.** Park them (sidecar or `[__unknown]`), warn, never fail the load —
   the one change that makes an older binary survive a newer config.
4. **A describe/notes surface.** `config describe` + `config notes <key>`, and `config_manage`
   parity, so an agent can read the meaning and the default of every key without source.
5. **A doctor sweep.** Unknown keys, deprecated keys, missing sections, the two-file split, the
   unparseable-file backup — report, and heal the safe ones.

---

## 5. The enforcement survey — what makes an agent touch the whole map

The operator's question: "how can an agent developing a new config feature know everything it must
update, and is that enforced at the architectural level?" The grounded answer:

| Gate | File | What it forces | Direction |
|---|---|---|---|
| the compiler | `config.rs` | the struct field (and, in exhaustive literals, the default) | forward, partial |
| `tests/example_template_loads.rs` | the template test | every commented example names a key the loader ACCEPTS | **reverse only** — a struct key the template omits is invisible |
| `scripts/audit-config-surface.py --check` | the surface gate | a field with NO reader is in `ACCOUNTED_FOR` | forward, unread-fields only |
| `scripts/check-doc-claims.sh` | the docs-check | AGENTS.md's code-surface claims match the code | doc claims only |
| `genome lint` | the doc laws | one owner per question, mutability, no deletions, statuses | doc hygiene only |

**The gap, stated exactly:** no gate checks the FORWARD coverage of a new key across the note,
the heal entry, the template, the doctor row, the describe output, the RUNBOOK, and the CHANGELOG.
An agent can add a key that compiles, passes every gate, and is invisible to the operator. The
plan's §5 (`docs/plans/2026-11-14-config-resilience-architecture-...`) closes it with the generated
registry and `scripts/audit-config-keys.py --check`.
