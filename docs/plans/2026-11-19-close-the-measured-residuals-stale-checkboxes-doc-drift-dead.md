# Close the measured residuals: stale checkboxes, doc-drift, dead-code citation, and the clippy gate

Dated-frozen: published `2026-11-19`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Why this plan exists

The Hydrogen review campaign (iter-2671 → iter-2692) closed the four runtime
defects it found (F1/F2 iter-2679, F3 iter-2680, F6 iter-2688) and refuted
one (F5). A peer's tranche plan
(`2026-11-19-close-the-last-nine-ledger-rows-and-the-five-open-hydrogen-f.md`)
tracked the remaining rows and was executed through iter-2692. What is left
is not a new feature: it is **measurement drift** — checkboxes that are
stale, library docs that cite deleted files, a crate-graph claim in
OUTLINE.md that the Cargo manifests contradict, dead code the tree still
ships, and a clippy gate that reports 42 NEW warnings because the allowlist
is empty. Each item below is a measured defect with a file:line falsifier,
not a proposal.

This plan is the **owner** of the residual-cleanup question. It supersedes
the scheduling half of the three predecessor plans (all dated-frozen) and
records which of their rows are already closed so no one re-does them.

## The measured landscape at `66533e7f`

```text
git fetch origin
grep -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md   # 5
grep -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md   # 654
python3 scripts/audit-ledger-index.py               # 4 FAILs (RT-555 x2, RT-553, RT-554)
./scripts/genome lint                               # 16 errors, 6 warnings
bash scripts/check-doc-claims.sh                    # 1 DRIFT (TUI modules)
bash scripts/clippy-warning-gate.sh                 # seen=42 allowlist=0 new=42
```

### The five open ledger rows (all owner-gated or already-closed-in-code)

| # | row | true state at `66533e7f` | disposition |
|---|---|---|---|
| 1 | **RT-136** | `[pk]`/`[socialization]` parsed, surfaced by `unused_setting_warnings`, doc-commented "Nothing reads this table". Confirmed-not-a-defect iter-1176. | **Stale checkbox.** Flip `[x]`, record the iter-1176 disposition. Doc-only. |
| 2 | **RT-190** | memory-bank topology. RULED option (c) iter-1373: documented boundary, not unified. Row stays open as a future tranche. | **Owner-gated.** Do not touch. The (b) bridge is a future tranche. |
| 3 | **RT-280** | `suspended`/`resume_pending` read on every access, written by nothing. Doc comments now say so. Needs a product decision (max_age for resume, /stop semantics). | **Owner-gated.** Do not touch. |
| 4 | **RT-285** | **All three surfaces closed in code:** `discard` is durable + `#[must_use]` (iter-2681, `Result<usize>` + `delete_session` cascade + `discard_is_durable` 3 tests); `Autocompactor` deleted (grep: 0 hits); restart signals deleted iter-2665 (only tombstone comments remain). | **Stale checkbox.** Flip `[x]`, record iter-2681/iter-2665. Doc-only. |
| 5 | **RT-543** | socket shipped (iter-2654+), ACP disposition decision-gated. | **Owner-gated.** Do not touch until Q6/RT-543 ruling. |

### The Hydrogen findings — all dispositioned

| id | status | evidence |
|---|---|---|
| F1 | FIXED iter-2679 | `PrimaryBreaker` at `fallback.rs:236`, 5 breaker tests |
| F2 | FIXED iter-2679 | `clamp_max_tokens_to_model` at `fallback.rs:363`, called at :484/:536/:637 |
| F3 | FIXED iter-2680 | `gateway_turn_ceiling_secs` applied at both slot build sites |
| F4 | FIXED iter-2686/2687 | doctor TTS wording corrected |
| F5 | REFUTED | doc-only, corrected iter-2672 |
| F6 | FIXED iter-2688 | `persisted_key` param, 2 regression tests |
| F7 | PREMISE FALSE iter-2691 | no second config system; CLI adapter only |
| F8 | CONFIRMED doc-drift | OUTLINE.md:80-82 claims `operant-core/src/plugins/` reaches `operant-plugins`, but `operant-core/Cargo.toml` has no `operant-plugins` dep; `plugins-wasm` lives solely on `operant-cli`. **T1 below.** |
| F9 | CONFIRMED doc-drift | OUTLINE.md:47 credits `operant-core` with "MCP client + server"; the server is `crates/operant-cli/src/mcp_serve.rs`, the client is `crates/operant-core/src/mcp.rs`. **T2 below.** |
| F10 | CONFIRMED dead code | `gateway/admin.rs::handle_admin_command` is `pub use admin::*`'d at `gateway/mod.rs:67` and called only from `#[cfg(test)]` blocks at `mod.rs:2402-2495` and `admin.rs:149,166`. Zero production callers. **T3 below.** |
| F11 | CONFIRMED doc-drift (residual) | `slack.rs::handle_update` is gone (no `fn handle_update` in slack.rs); AGENTS.md:661 still says "Its `handle_update` parser is dead code (no caller feeds it)". The parser was removed; the doc claim is stale. **T4 below.** |
| F12 | FIXED iter-2672 | `historian_key_is_parked` + doctor warning at `checks_config.rs:289` |

### The genome lint failures (16 errors, 6 warnings)

| code | count | what | tranche |
|---|---|---|---|
| E303 | 5 | library docs cite deleted files: `security.md:24` → `write_approval.rs` (deleted iter-2665); `vision.md:14,123` → `schema/media.rs` (deleted iter-2680); `voice.md:14` → `tts_provider.rs` (deleted) | **T5** |
| E311 | 5 | files named by no library page: `config/docker_terminal.rs`, `config/ssh_terminal.rs`, `config/stt.rs`, `config/vision.rs`, `text_slice.rs` | **T6** |
| W006 | 2 | three plans answer the same question (the predecessor tranche plans) | **T7** |
| W010 | 3 | audit/library docs >300 lines with no TOC | out of scope (style) |
| W014 | 1 | Q4 open 16 days (terminal enhanced-keys) | out of scope (owner) |

### The clippy gate failure (42 NEW warnings, allowlist empty)

The allowlist (`.ci/clippy-allowlist.txt`) was emptied at iter-1171 and
never repopulated. The gate now reports 42 NEW warnings — all in
`operant-cli` (TUI dead_code, `gateway_runner.rs` unused_variables,
`await_holding_lock`, etc.). These are **peer WIP**, not a regression from
this campaign. The gate's own protocol says: fix warnings or annotate with
`#[expect]`, never `--update` to hide. **T8 below** triages them.

### The doc-claims drift (1 DRIFT)

`AGENTS.md:873` says "73 TUI modules (350 .rs files)"; the tree has 74
`pub mod` declarations in `tui/mod.rs` and 351 `.rs` files. **T9 below.**

## Tranches

### T1 — F8: OUTLINE.md's crate-graph claim is wrong

`docs/OUTLINE.md:80-82`:

> `operant-harness` and `operant-plugins` are reached through `operant-core`'s
> plugin host (`crates/operant-core/src/plugins/`), never the reverse.

**Falsifier:** `operant-core/Cargo.toml` has `operant-harness` (line 55) but
NO `operant-plugins` dependency. `plugins-wasm` is solely on
`operant-cli/Cargo.toml:106`. `operant-core/src/plugins/mod.rs` is a
Telegram-command registry (`PluginRegistry` for `/`-prefixed gateway
commands), not an Extism host. A non-CLI embedder of `operant-core` gets no
WASM plugins.

**Fix:** rewrite OUTLINE.md:80-82 to say what is true: `operant-harness` is
reached through `operant-core`'s plugin handle; `operant-plugins` (the
Extism host) is reached through `operant-cli` behind the `plugins-wasm`
feature, and a non-CLI embedder of `operant-core` does not get WASM plugins.

**Test:** none — doc-only. Mutation-prove by reverting the sentence.

**Files:** `docs/OUTLINE.md` only.

### T2 — F9: OUTLINE.md credits operant-core with the MCP server

`docs/OUTLINE.md:47`:

> `operant-core` | 244,960 | The product: the agent loop (`agent/`), the tool
> registry and every builtin (`tools/`), memory, **MCP client + server**,
> context management, ...

**Falsifier:** the MCP server is `crates/operant-cli/src/mcp_serve.rs`
(`run_mcp_serve` at :538, called from `cmd_mcp.rs`). The MCP client is
`crates/operant-core/src/mcp.rs` (`McpClient` at :108). The server lives one
crate up.

**Fix:** change "MCP client + server" to "MCP client" in the OUTLINE.md
crate table, and add a sentence naming `operant-cli/src/mcp_serve.rs` as
the server's home.

**Test:** none — doc-only.

**Files:** `docs/OUTLINE.md` only. (Can land in the same commit as T1.)

### T3 — F10: `gateway/admin.rs::handle_admin_command` is dead production code

`crates/operant-core/src/gateway/admin.rs:13` defines
`pub async fn handle_admin_command`. It is `pub use admin::*`'d at
`gateway/mod.rs:67`. Every caller is inside a `#[cfg(test)]` block:

- `admin.rs:149,166` — its own `#[cfg(test)]` mod
- `mod.rs:2402,2420,2441,2459,2477,2495` — inside `#[cfg(test)]` at :1069

Zero production callers. The gateway dispatch loop never calls it. It is
dead code that ships in the library.

**Fix:** delete `gateway/admin.rs` and the `pub use admin::*;` line, and
remove the test blocks that call it (`mod.rs:2395-2510`, `admin.rs` tests).
Keep the `admin` mod declaration only if something else uses it; if not,
remove `mod admin;` too.

**Test:** the existing admin tests are deleted with the code. A guard test
`admin_command_is_gone` that asserts the symbol does not resolve is NOT
needed — the compiler is the guard (deleting the `pub use` breaks any
external caller, and there are none).

**Files:** `crates/operant-core/src/gateway/admin.rs` (delete),
`crates/operant-core/src/gateway/mod.rs` (remove `mod admin;` + `pub use` +
test block).

**Risk:** low. The code is `pub`-exported but has zero non-test callers
anywhere in the workspace (grep confirmed). If a future Holon run needs
admin commands, it can resurrect from git history.

### T4 — F11 residual: AGENTS.md claims slack.rs has a dead `handle_update`

`AGENTS.md:661`:

> Its `handle_update` parser is dead code (no caller feeds it).

**Falsifier:** `crates/operant-core/src/gateway/slack.rs` has no
`fn handle_update` (grep: 0 hits). The parser was removed. The doc claim is
stale — it describes a parser that no longer exists.

**Fix:** delete the "Its `handle_update` parser is dead code" sentence from
AGENTS.md:661. The surrounding text ("Slack is OUTBOUND-ONLY... `start()`
logs and returns") is accurate and stays.

**Test:** none — doc-only.

**Files:** `AGENTS.md` only. (Can land with T1/T2 in one docs commit.)

### T5 — E303: library docs cite deleted files

`./scripts/genome lint` E303 (5 hits):

| doc:line | cites | reality |
|---|---|---|
| `docs/library/security.md:24` | `crates/operant-core/src/write_approval.rs` | deleted iter-2665 |
| `docs/library/security.md:341,348` | `write_approval.rs` and `write_origin.rs` | `write_approval.rs` deleted; `write_origin.rs` exists |
| `docs/library/vision.md:14,123` | `crates/operant-config/src/schema/media.rs` | deleted iter-2680 (RT-275) |
| `docs/library/voice.md:14` | `crates/operant-core/src/tools/tts_provider.rs` | deleted (provider trait moved) |

**Fix:** for each citation, either (a) repoint to the surviving file (e.g.
`write_origin.rs` still exists; `schema/media.rs`'s types moved to
`operant-config/src/schema/` — find where), or (b) remove the dead citation
and replace with a prose description of the mechanism. The security.md
:341-348 section on `write_approval.rs` must be rewritten to describe the
current write-approval mechanism (or deleted if the mechanism is gone).

**Test:** `./scripts/genome lint` E303 clean for these 5 sites.

**Files:** `docs/library/security.md`, `docs/library/vision.md`,
`docs/library/voice.md`.

### T6 — E311: five files named by no library page

`./scripts/genome lint` E311 (5 hits):

- `crates/operant-core/src/config/docker_terminal.rs`
- `crates/operant-core/src/config/ssh_terminal.rs`
- `crates/operant-core/src/config/stt.rs`
- `crates/operant-core/src/config/vision.rs`
- `crates/operant-core/src/text_slice.rs`

Each is a real source file that no `docs/library/` page cites. The genome
rule is "every source file is named by a library page or exempted with a
reason."

**Fix (per file):** either cite it in the relevant library page (e.g.
`stt.rs`/`vision.rs`/`docker_terminal.rs`/`ssh_terminal.rs` in
`docs/library/config.md` or their feature pages; `text_slice.rs` in
`docs/library/context-management.md` or `tools.md`), or add it to
`genome.toml`'s `ops.coverage_exempt` with a one-line reason.

**Test:** `./scripts/genome lint` E311 clean for these 5 sites.

**Files:** the library pages above, or `genome.toml`.

### T7 — W006: three plans answer the same question

`./scripts/genome lint` W006 (2 hits):

- `2026-10-27-remaining-implementation-gaps-outline-execution-plan-for-the.md`
- `2026-11-19-close-the-last-nine-open-ledger-rows-the-dead-surfaces-go-th.md`
- `2026-11-19-close-the-last-nine-ledger-rows-and-the-five-open-hydrogen-f.md`

Three dated-frozen plans for the same ledger rows. The genome rule is "one
owner per question." The third plan (iter-2671, mine) is the owner; the
first two are superseded predecessors.

**Fix:** add a tombstone header to the two superseded plans naming the
owner (this plan / the iter-2671 plan), per the genome archive discipline.
Do NOT delete them (dated-frozen, append-only).

**Test:** `./scripts/genome lint` W006 clean.

**Files:** the two superseded plan files (header only).

### T8 — Clippy gate: 42 NEW warnings, allowlist empty

`bash scripts/clippy-warning-gate.sh`: `seen=42 allowlist=0 new=42`. The
allowlist was emptied iter-1171 and never repopulated. The 42 warnings are
all in `operant-cli`:

- `dead_code` ×12 — `tui/operant_ui/ui_*.rs` (11 files) + `typeahead.rs`
- `unused_variables` ×4 — `gateway_runner.rs`, `tui/settings_screen/tests.rs`,
  `tui/theme_colors.rs`, `tui/app/tests.rs`
- `unused_imports` ×2 — `model_scope_nav.rs`, `surface_widget.rs`
- `await_holding_lock` ×2 — `gateway_runner.rs` + its tests
- `field_reassign_with_default` ×1 — `gateway_commands.rs`
- `non_snake_case` ×1 — `tui/settings_screen/tests.rs`
- `unfulfilled_lint_expectations` ×1 — `tui/mod.rs`
- `unused_must_use` ×1 — `cmd_skills.rs`
- `unused_mut` ×1 — `tui/app/tests.rs`

**Triage rule (from the gate's own protocol):**
1. `dead_code` in `ui_*.rs` — these are peer WIP (the TUI visual layer, not
   this campaign's code). Annotate with `#[allow(dead_code)]` + a reason
   comment, or fix if the code is genuinely unreachable. Do NOT delete peer
   WIP.
2. `unused_variables`/`unused_imports`/`unused_mut` — fix by prefixing `_`
   or removing. Low risk.
3. `await_holding_lock` in `gateway_runner.rs` — this is a real defect (a
   `std::sync::Mutex` guard held across `.await`). Fix by switching to
   `tokio::sync::Mutex` or dropping the guard before the await. Medium risk.
4. `field_reassign_with_default` — fix with struct update syntax.
5. `unfulfilled_lint_expectations` — remove the stale `#[expect]`.

**Test:** `bash scripts/clippy-warning-gate.sh` passes (new=0). The
allowlist stays EMPTY — the fix is in the code, not the allowlist.

**Files:** the `operant-cli` source files above.

**Risk:** medium. The `await_holding_lock` fix touches the gateway dispatch
path. The dead_code annotations are zero-risk. Do this tranche LAST (after
T1-T7) so the peer WIP has settled.

### T9 — Doc-claims drift: AGENTS.md says 73 TUI modules, tree has 74

`AGENTS.md:873`:

> `tui/` # 73 TUI modules (350 .rs files)

**Falsifier:** `grep -c 'pub mod' crates/operant-cli/src/tui/mod.rs` = 74;
`find crates/operant-cli/src/tui -name '*.rs' | wc -l` = 351.

**Fix:** update AGENTS.md:873 to "74 TUI modules (351 .rs files)".

**Test:** `bash scripts/check-doc-claims.sh` reports `OK TUI modules: 74`.

**Files:** `AGENTS.md` only. (Can land with T1/T2/T4 in one docs commit.)

## Order and dependency

**T1 + T2 + T4 + T9 (docs, one commit) → T5 (E303) → T6 (E311) → T7 (W006)
→ T8 (clippy, last, after peer WIP settles).**

T1/T2/T4/T9 are doc-only and share files (OUTLINE.md, AGENTS.md). T5/T6 are
library-doc fixes. T7 is a plan header. T8 is the code cleanup and goes last.

## Explicitly NOT in scope

- **RT-190, RT-280, RT-543** — owner-gated, documented as not scheduled.
- **RT-136, RT-285** — stale checkboxes, closed by code already (iter-1176,
  iter-2681). Flipping them is T10 (a BUGS.md-only docs iteration) — included
  here as a checkbox but not a tranche, because it is one `edit` call.
- **W010** (audit docs without TOC) — style, not a defect.
- **W014** (Q4 open 16 days) — owner-gated.
- **RT-555/RT-553/RT-554** (ledger-index FAILs) — peer state, out of scope.
- **The 28 pre-existing lib-test failures** (config/checkpoint/kernel/gateway)
  — proven pre-existing at `d250e538`, not this campaign's.
- **F7 convergence** (moving YAML keys onto AppConfig) — the finding's
  premise was false (iter-2691); no second system exists.

## Claim rule

Never take a number from a header. `git fetch origin`, read the top label,
add one. This plan owns the residual-cleanup question; the three predecessor
plans are dated-frozen and superseded.

## Cost

| tranche | files | ~LOC | new deps | risk |
|---|---|---|---|---|
| T1 F8 | `docs/OUTLINE.md` | ~5 | none | none (doc) |
| T2 F9 | `docs/OUTLINE.md` | ~3 | none | none (doc) |
| T3 F10 | `gateway/admin.rs` (delete), `gateway/mod.rs` | ~170 removed | none | low (dead code) |
| T4 F11 | `AGENTS.md` | ~1 | none | none (doc) |
| T5 E303 | `docs/library/{security,vision,voice}.md` | ~20 | none | none (doc) |
| T6 E311 | library pages or `genome.toml` | ~10 | none | none (doc) |
| T7 W006 | two plan files | ~5 | none | none (doc) |
| T8 clippy | ~15 `operant-cli` files | ~50 | none | medium (await_holding_lock) |
| T9 doc-claims | `AGENTS.md` | ~1 | none | none (doc) |
| T10 checkboxes | `BUGS.md` | ~4 | none | none (doc) |

Nine tranches. Zero new dependencies. Zero schema changes.