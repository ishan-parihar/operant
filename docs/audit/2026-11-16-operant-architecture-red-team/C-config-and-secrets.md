# Lane C — Configuration, Secrets and Identity

| field | value |
|---|---|
| lane | **C** (red-team + ponytail audit of `operant`) |
| seam | configuration, secrets, identity, profile/home resolution, env passthrough |
| pinned rev | `25b86ae5` (`feat(iter-2472): retract the /bar status-line half-port…`, 2026-11-16T08:00:16+05:30) |
| audited bytes | **WORKING TREE at `71af1dcf`** = `25b86ae5` + one commit (`iter-2473`) |
| peer-dirty? | **NO staged WIP.** `git status --porcelain` shows only ` M docs/kb/feed.md` and untracked paths. `crates/operant-core/src/config.rs`, `config_registry.rs`, `operant.example.toml` and `tui/` are all clean against the index. The warned-about peer WIP is **not present**. |

## Pinned-rev deviation (DECLARED, material)

`git rev-parse --short HEAD` = **`71af1dcf`**, one commit AHEAD of the pinned `25b86ae5`.
`git rev-list --count 25b86ae5..HEAD` = 1.

The single commit between them (`iter-2473`) touches my seam: it **deletes**
`tui.bar_style` from `crates/operant-core/src/config.rs:1494-1519`,
`config_registry.rs` (the `CONFIG_KEYS` row, the `SETTING_ROWS` row, and the
`KNOWN_KEYS` entry) and `operant.example.toml:460-472`.

Consequences, stated rather than hidden:

* Findings about a key that **still exists** are unaffected.
* The coordinator's confirmed fact "SETTING_ROWS has 386 entries / KNOWN_KEYS
  386" was true at `25b86ae5` and is **385** at the audited bytes. I re-derived
  it below rather than re-reporting it as a finding.
* Line numbers below cite the **audited working tree**. Where a line is
  pin-specific I say so.

No `.rs` was edited. No cargo run. No mutating subcommand run.

---

## Files read (exact)

_pending — filled as read._

## Files NOT read

_pending._

---

## Concept counts

| concept | count | how derived |
|---|---|---|
| `KNOWN_KEYS` | _tbd_ | `rg -c` on config_registry.rs |
| `SETTING_ROWS` | _tbd_ | |
| `CONFIG_KEYS` | _tbd_ | |
| `ALLOWED` entries in `one_home_resolver.rs` | _tbd_ | |
| `operant_home()`-shaped resolvers found | _tbd_ | |
| secret-bearing config fields | _tbd_ | |

### Classification table

_pending._

---

## Findings

_pending._

---

## Ponytail cut list

_pending._

---

## Rust discipline table

_pending._