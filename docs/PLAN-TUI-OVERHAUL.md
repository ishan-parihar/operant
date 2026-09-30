# Plan v2: TUI Overhaul → jcode parity (rev. 2)

> Supersedes rev. 1 (preserved at `docs/PLAN-TUI-OVERHAUL-v1.md`, commit `8580b453`).
> Adds: a liftability audit of jcode's 16 crates, Rust-practice corrections, and a
> fleet-deployment topology.
> **Decision:** Option B (hybrid lift). Accepted. See §0.

## Status

| Iteration | Landed |
|---|---|
| `iter-517` | Vendored 4 jcode crates (~9.6k LOC) as modules under `tui/vendor/`. 144 vendored tests green, 0 warnings. |
| `iter-518` | Phase 0 gate: `--baseline` drift detection, `--capture-frames`, `--capture-dir`, `scripts/tui-capture.sh`. Drift failure independently verified. |
| `iter-519` | Phase 1a/1b: both palettes bridged, `adapt_buffer_for_display` wired as the choke point, base chrome migrated, competing accents retired. Corpus green at 50 scenarios / 67 variant-runs. |

**Two constraints discovered during implementation — both change how the remaining work must be sequenced.**

**(1) The palette bridge has a collision rule that is not optional.**
`adapt_buffer_for_display` substitutes by *exact RGB match against each role's frozen default*, so seeding all 22 roles means any operant colour that happens to equal some role's default is silently repainted. Two real collisions exist: `dark.success` `(129,199,132)` **is** jcode's `Ai` default, and `dark`/`deuteranopia` `muted` `(120,120,120)` **is** jcode's `Tool` default. Seeding those roles from the semantically obvious fields would have turned every success-green cell red. `theme_colors.rs` now carries a documented collision rule and a table-driven test pinning it across all 8 themes. **Any future role addition must re-run that test** — it is the only thing standing between a palette change and an invisible, TUI-wide colour regression.

**(2) Goldens must be captured against a frozen render, so the primitive is built before the migration.**
The visual goldens are the regression net for all later surface work. Therefore: land the new primitive (`modal_frame`, spacing scale) with its tests **while changing no rendered output**, commit the goldens against the current stable render, and only then flip call sites — each flip verified by the goldens. Building the primitive first also means a wrong abstraction is caught by unit tests, which goldens cannot do.

---

## 0. The decision: three options, one verdict

The brief offered "copy jcode's files and discard operant's TUI." I tested that literally.
It is **~19,500 LOC liftable out of 259,323 — 7.5%.** The other 92.5% is welded.

| Option | Verdict | Why |
|---|---|---|
| **A. Hand-refactor everything** | Rejected | Fixing ~300 hardcoded colors across 39 files by hand is high-labour, low-leverage, and leaves the 4 genuinely-welded quality gaps (role substitution, one modal idiom, frame hygiene, narrow-terminal collapse) to be reinvented badly. |
| **B. Lift the foundation, keep the behaviour layer** ✅ | **Recommended** | The 4 things operant lacks all live in liftable crates. Keeps vim mode (which jcode has no equivalent of), keeps 349 tests, keeps operant-specific agent wiring. |
| **C. Discard operant's TUI, port jcode's** | Rejected | Would delete `prompt_input/vim.rs` (1,420 LOC) — **jcode has no modal editor at all** — plus 349 tests and all operant-specific wiring. Then require porting 133k LOC of `app/` welded to jcode-app-core (155,549 LOC) and jcode-base (133,293 LOC). |

### Why C is a trap, specifically

- **jcode has no vim mode.** `rg -i 'vim'` across the whole TUI = **3 files**, all readline key-repeat
  hints (`session_picker.rs:1154`, `inline_interactive.rs:3311,3336`). A repo-wide search for
  `NormalMode|InsertMode|VisualMode|CountPrefix` = **zero hits**. operant's 1,420-LOC modal editor is
  **unique to operant**. A port deletes a feature nothing can replace.
- **LaTeX is a wash, not a jcode win.** jcode's is 1,294 LOC of glue over a third-party
  `mdwright-latex 0.1.3` (`markdown_latex_image.rs:309` is the single integration point) vs
  operant's own 1,552-LOC `latex.rs`. jcode's version *adds* an unpinned 0.1.x dependency.
- **jcode's logic layer is a rewrite, not a port.** `src/tui/app/` is 175 files / 133,183 LOC = 61%
  of `jcode-tui`. 129 of 299 src files (43%) reference `App`. `app/commands.rs` alone hits
  `jcode_provider_core` 78 times. It reads jcode's own on-disk layout
  (`session_picker/loading.rs:213` → `storage::jcode_dir()/cache/…`).

### Two suspected blockers that turned out **not** to be blockers

- **No workspace inheritance.** jcode's root `Cargo.toml:8` has `[workspace]` with **only `members`** —
  no `[workspace.dependencies]`, no `[workspace.package]`. `rg 'workspace\s*=\s*true'` across all 16
  manifests = **zero hits**. A straight file copy resolves; only `path = "../jcode-*"` edges need
  repointing.
- **Licence is compatible.** jcode is MIT (Copyright © 2025 Jeremy Huang), single root `LICENSE`, no
  per-crate files. operant is MIT OR Apache-2.0. **Condition: the copyright + permission notice must
  travel with every lifted file.**

---

## 1. Lift manifest — what to copy, and how

**~19,500 LOC across 11 crates.** Ratatui 0.30 in both trees, so versions align.

| Priority | Crate | LOC | Modification needed |
|---|---|---|---|
| **1** | `jcode-tui-style` | 5,256 | **One line.** `src/lib.rs:34` calls `jcode_logging::warn` → repoint at operant's log facade or `eprintln!`. `jcode-logging` drags in jcode-core + jcode-storage + tokio — do **not** copy it. The 2 `jcode_app_core` mentions (`color.rs:45,72`) are doc comments; drop them. |
| **2** | `jcode-render-core` | 4,532 | **None.** Deps: `pulldown-cmark`, `serde`, `unicode-width`. Documented as ratatui-free. |
| **3** | `jcode-config-types` | 2,684 | **None.** Deps: `serde`, `serde_json`. Its 1 external ref is a doc comment (`lib.rs:1321`). |
| **4** | `jcode-tui-workspace` | 1,232 | **None.** Sole dep `ratatui 0.30`. **Best lift in the repo** — and a *capability operant has no equivalent for*. |
| **5** | `jcode-tui-account-picker` | 1,607 | **None.** Deps: anyhow, crossterm, ratatui, serde_json. 0 external refs. |
| **6** | `jcode-tui-anim` | 1,131 | **None — the only zero-dependency crate.** Replicate jcode's `opt-level = 3` profile pin or the idle-animation CPU rationale is lost. |
| **7** | `jcode-terminal-image` | 759 | **None.** Deps: base64, crossterm. Replaces operant's 873-LOC `pinned_images.rs` protocol half. |
| **8** | `jcode-tui-core` | 3,241 | 2 refs in one file (`graph_topology.rs:58,302`) → copy `jcode-memory-types` (4-dep leaf) or stub 5 types. |
| **9** | `jcode-tui-tool-display` | 257 | **None.** serde_json + unicode-width. |
| **10** | `jcode-tui-visual-debug` | 857 | 2 log calls (`lib.rs:284,478`). **Reconsider** — operant already has `cmd_tui_debug.rs`; overlap likely. |
| **11** | `jcode-tui-render` | 5,590 | Lifts with 1 import, **but 77% is a swarm/memory visualizer** (`swarm_gallery.rs` 3,099 + `swarm_tiles.rs` 611 + `memory_tiles.rs` 587 = 4,297). Only `lib.rs` 202 + `chrome.rs` 91 + `layout.rs` 64 = **357 LOC is reusable** — the `render_rounded_box` / `clear_area` / right-rail chrome we want. **Copy those 3 files, skip the swarm.** |

Tiny DTO leaves (`jcode-message-types`, `jcode-session-types`, `jcode-memory-types`,
`jcode-usage-types`) only if §4 picks up the pickers/usage overlay.

### Do NOT lift

| Crate | LOC | Why |
|---|---|---|
| `jcode-tui` | 218,416 | 116/302 files reference `App`; needs jcode-app-core + jcode-base. |
| `jcode-tui-mermaid` | 11,541 | Pinned git dep on one person's fork (`1jehuang/mermaid-rs-renderer` tag v0.3.1) + resvg/usvg/ratatui-image. Supply-chain risk. |
| `jcode-tui-messages` | 1,559 | Its types *are* jcode's session/message model. |
| `jcode-tui-permissions` | 866 | Welded to jcode-base's safety model. |
| `jcode-tui-markdown` | 9,847 | Needs syntect→`regex-onig` (C oniguruma) + third-party `mdwright-latex` 0.1.3. Borderline; defer. |

### The pattern worth taking even though the code isn't liftable

jcode's render layer **is** decoupled — every render fn takes `&dyn TuiState`, never `&App`. Two
implementors: `impl TuiState for App` (`app/tui_state.rs:568`, 2,501 LOC) and
`impl TuiState for TestState` (`ui_tests/mod.rs:157`, a hand-written double). The `ui_tests/` suite is
**19 files / 10,729 LOC** driving that double through the real render path.

**Take the shape, not the trait.** See §2(a).

---

## 2. Rust-practice corrections to rev. 1

Four places where blind porting of the exemplar would be *wrong*:

**(a) Do not port the 143-method `TuiState` trait.** It has ~143 methods and its signatures expose 12
jcode app-core types (`AuthStatus`, `ContextInfo`, `SidePanelSnapshot`, `BatchProgress`, …) resolving
through `pub use jcode_app_core::*` → 289k LOC. Lifting it means either faking 143 accessors or
copying a quarter-million lines to satisfy a signature. Per **ch.6** it is also the ❌ case: a `dyn
Trait` that fat is vtable indirection over a huge surface. Per **ch.1 §1.8** those 12 types are
*coincidental* similarity, not shared knowledge.

operant already has the ✅ pattern: `App::run<B: ratatui::backend::Backend>` (`app/mod.rs:776-782`)
is generic, and renderers take `&App`. **Keep it.** Take only the *testing* idea: a lightweight state
double so surfaces can be rendered without a full `App`.

**(b) A single `modal_frame()` is still correct — but that is not a licence to add a token layer.**
**ch.1 §1.8**: "duplication is far cheaper than the wrong abstraction." 16 modal formulas is one
knowledge (must change together), so extracting is right. But jcode has **no spacing scale at all**
(grep-confirmed zero `Margin::`/`Padding::`/`theme::space`), so rev. 1's spacing scale is a
*deliberate deviation*, not a port. Keep it minimal: a `space` const set and a `pad_v`. No type
scale, no token struct hierarchy.

**(c) The per-frame buffer substitution must be measured, not assumed.** **ch.3**: "Don't guess,
measure", always with `--release`. `adapt_buffer_for_display` walks every cell each frame — 4,800
cells at 120×40. Budget a micro-benchmark. Note the contrast-repair binary search is a **startup**
transform, not a per-frame one.

**(d) Two upstream patterns to reject as cargo-cult:**
- `FullFrameInvalidation::SoftRepaint` — fills the previous buffer with a `U+FDD0` sentinel to force a
  full re-emit without a clear escape (`run_shell.rs:189-222`). Clever, but it works around a flicker
  bug in jcode's terminal-image placeholders. **Only adopt if we hit that bug.**
- Oklab harmony scoring — 2,357 LOC to score palettes. **Defer.** Our problem is 300 hardcoded colors
  bypassing the palette, not an unmeasured palette.

---

## 3. Revised phase outline

Rev. 1's phases 0/2/3/4 stand. Changes: **Phase 1 becomes a lift**, a new **Phase 2.5** appears, and
two upstream patterns are explicitly rejected.

### Phase 0 — Make the ugliness visible *(0.2–0.6 landed iter-518; 0.1 in flight)*

| # | Task | Done when |
|---|------|-----------|
| 0.1 | `--dump-style`: serialise `symbol`+`fg`+`bg`+`modifier` per cell. Today `tui_app.rs:933` extracts `c.symbol()` only, so **a palette regression is structurally invisible** | a theme change produces a different dump |
| 0.2 | `--baseline <path>`: unified diff, non-zero exit. Baselines in `crates/operant-cli/tests/tui_baselines/` | changing one colour fails with a readable diff |
| 0.3 | `--capture-frames 0,3,7`: per-frame buffer capture. `FrameRendered` exists (`event_bus.rs:96-100`) but records timing only | a mid-stream frame is diffable |
| 0.4 | Scenario corpus, one per surface, all 43 `render_*` entry points + the 13-overlay pack | a new surface without a scenario fails the gate |
| 0.5 | `tmux` real-terminal capture (`capture-pane -p -e`) for interactive-only paths `TestBackend` bypasses: raw mode, alt-screen, mouse, real `FocusGained` | a real `operant chat` pane is diffed the same way |
| 0.6 | Fix the stale header at `cmd_tui_debug.rs:13-15` ("does NOT render anything" — false since `Simulate` exists) | docs match reality |

### Phase 1 — Lift the foundation *(sequential; replaces rev. 1 phases 1.1–1.7)*

> **1.1–1.7 landed** (iter-517, iter-519). One deviation from plan: the crates were vendored as
> **modules under `operant-cli/src/tui/`**, not as new workspace crates, because the repo-root
> `Cargo.toml` carried another agent's uncommitted work and adding `[workspace] members` would have
> meant staging a line inside their diff. Every crate `operant-cli` already depended on was
> present, so no manifest change was needed and the move stays reversible.

| # | Task | Source | Done when |
|---|------|--------|-----------|
| 1.1 | Vendor `jcode-tui-style` as new workspace crate `operant-tui-style`. Repoint the 1 log line. Strip `jcode_*` doc comments. Add the MIT attribution header. | 5,256 LOC | compiles; `cargo test -p operant-tui-style` green |
| 1.2 | Vendor `jcode-render-core` (ratatui-free markdown prep) | 4,532 LOC | compiles |
| 1.3 | Vendor `jcode-tui-render`'s **3 generic files only** — `lib.rs`, `chrome.rs`, `layout.rs`. **Skip the 4,297 LOC of swarm visualizer.** | 357 LOC | `render_rounded_box`, `clear_area`, `right_rail_border_style` available |
| 1.4 | Vendor `jcode-tui-workspace` — **new capability for operant** | 1,232 LOC | widget renders in a TestBackend |
| 1.5 | Vendor `jcode-tui-anim` + replicate the `opt-level=3` pin | 1,131 LOC | compiles |
| 1.6 | Wire `adapt_buffer_for_display(frame.buffer_mut())` into `render_app` after the draw — **the substitution choke point** | — | `/theme` repaints every surface |
| 1.7 | Retire the competing accents. `app.accent_color`/`ACCENT_BUILD` (`app/enums.rs:314`, `Rgb(255,191,0)`) is byte-identical to the theme's `emphasis` yet a separate literal, so **`/theme` cannot repaint it on 7 of 8 themes**. Plus hardcoded `Magenta` at `journey_view.rs:194` and `effort_picker.rs:72`. | — | grep finds exactly one accent source |
| 1.8 | One modal idiom `modal_frame(title, hint)` + a minimal `space` const set. Replaces 16 sizing formulas and 5 title idioms. | — | one function sizes every modal — **primitive landing now, call sites NOT yet migrated** (see constraint 2) |
| 1.9 | Frame hygiene: full clear + `BeginSynchronizedUpdate`. Drop hardcoded `Color::Black` base fill (`render/mod.rs:103`). | — | no stale cells, no flicker |
| 1.10 | **Benchmark 1.6** against the noise floor, `--release`, ≥3 repeats (ch.3) | — | recorded number, not an estimate |

### Phase 2 — Loop architecture *(sequential; mostly unchanged)*

`needs_redraw: bool` discipline · named redraw reasons · extract the 9 inline channel drains · event
coalescing · **make `dialog_priority()` the router** (currently computed and discarded at
`key_handling.rs:45-47`, under a comment admitting it's "for debugging") · reassert terminal modes on
`FocusGained`.

### Phase 2.5 — De-serialize the dispatch ladder *(NEW; prerequisite for any fleet)*

`render_app` (`render/mod.rs:96`) is a 399-line ladder with **35 `if app.X.visible` branches**. Every
surface agent would edit that one file → merge-conflict storm, and 35 hand-edited rows is exactly the
kind of table that rots.

Convert to a **z-ordered dispatch table owned by one file**, `render/dispatch.rs`:

```rust
// render/dispatch.rs — FROZEN after Phase 2.5. Surface agents never edit this.
pub const ENTRIES: &[Entry] = &[ /* 43 pre-seeded rows, z-order high→low */ ];
```

Each surface module exports `pub const ENTRY: Entry`. Phase 2.5 **pre-creates all 43 rows as stubs**
returning an empty render, so a surface agent's entire footprint is: its own file, its scenario, its
baseline. Zero shared edits.

**Justification** (ch.1 §1.8): 35 branches is 3+, and overlay z-order is genuinely one piece of shared
knowledge. The stronger justification is not DRY — it is that it makes the work parallelisable.

### Phase 3 — Surfaces *(massively parallel, one surface per agent)*

Conversation → chrome → management panels → pickers → overlays. Each agent owns exactly one surface
file + one scenario + one baseline, reusing the frozen Phase 1 foundation.

### Phase 4 — Affordances *(deferred, parallelisable, lowest risk)*

Narrow-terminal `MIN_*` floors + graceful collapse · empty states · focus indication · overflow
labelling (`↑N` / `+N more`) · one hint line · truncation primitives · actionable errors · resize
preserves reading position. All absent from operant today; take the *design* from jcode, write the
operant implementation.

---

## 4. Fleet deployment topology

**The phases are not equally parallelisable, and that is the whole scheduling problem.**

| Phase | Parallelism | Why |
|---|---|---|
| 0 | **1 agent** | touches `tui_app.rs`, `cmd_tui_debug.rs`, event bus — all shared |
| 1 | **1 agent** (or 2: crate-vendor ∥ wiring) | all 43 surfaces depend on it; a half-lifted foundation is worse than none |
| 2 | **1 agent** | the hot loop |
| 2.5 | **1 agent** | the dispatch table must be frozen before anyone branches |
| **3** | **N agents, one per surface** | 43 surfaces, disjoint file ownership, zero shared edits *by construction* |
| **4** | **N agents, one per affordance** | same |

So: **four sequential agents, then fan out to ~43.** Deploying a fleet at Phase 0 would be the
mistake — there is nothing independent to work on.

### Per-agent contract (Phase 3/4)

Each agent receives:
1. its surface file path — **the only production file it may edit**
2. the frozen foundation API (`Role::*`, `modal_frame()`, `space::*`, `render_rounded_box`)
3. its scenario + baseline paths
4. the verification block below

Explicitly forbidden: editing `render/dispatch.rs`, editing another surface, editing
`theme_colors.rs`/`operant-tui-style`, or running `cargo fmt --all` (see §6).

---

## 5. Verification gate (every iteration)

Per `AGENTS.md`: scoped, local. GitHub Actions runs are banned; read-only `gh run` inspection only.

```bash
source scripts/dev-env.sh
rustfmt --check --edition 2024 <changed files>          # NOT cargo fmt --all — see §6
./scripts/check.sh check -p operant-cli --bin operant
./scripts/check.sh test  -p operant-cli --bin operant -- tui
./scripts/check.sh test  -p operant-cli --test tui_scenarios
./scripts/tui-capture.sh --all                          # tmux real-terminal spot check
bash scripts/clippy-warning-gate.sh                     # no NEW warnings vs allowlist
```

Phase 1 additionally: `./scripts/check.sh test -p operant-tui-style` — the lifted crate's own tests
must pass.

Baselines: `./scripts/tui-baselines.sh accept` then **read the diff**. A regeneration touching more
files than the iteration changed means the change was too broad — split it.

### Working-tree constraint (live, blocking)

Peer WIP is uncommitted and **must not be staged, reformatted, or stashed**:
`crates/operant-core/tests/stream_interrupt.rs`, `crates/operant-core/src/agent/stream_retry_budget.rs`,
`crates/operant-cli/src/gateway_commands.rs`, `gateway_runner.rs`, `BUGS.md`, `Cargo.toml`,
`Cargo.lock`, `docs/plan-2026-09-29-telegram-loop.md`, `plans/telegram-ux-fixes.md`,
`docs/ORGANISM-OS-UPGRADE-OUTLINE.md`, plus dirty `crates/operant-core/src/agent/*`.

Consequences for this work:
- `cargo fmt --all` will reformat `gateway_commands.rs:517` and dirty the tree → use targeted `rustfmt`.
- `git pull --ff-only` is blocked → work from current `main`, do not stash.
- Phase 1 adds a **new crate to the workspace**, requiring a root `Cargo.toml` edit — a file *already
  dirty from peer work*. **Resolution needed before Phase 1:** either the peer lands first, or we
  stage the workspace-member line as an isolated hunk with explicit paths.

---

## 6. Honest sizing

| Iteration | Content | Risk |
|---|---|---|
| 1 | Phase 0 | Low — additive, no visual change. Unblocks everything. |
| 2 | Phase 1.1–1.5 (vendor crates) | Low — copy + small edits, licence-compatible |
| 3 | Phase 1.6–1.9 (wire + unify) + benchmark | Medium — touches every render site; baselines churn by design |
| 4 | Phase 2 | Medium — behaviour-preserving, hot loop |
| 5 | Phase 2.5 | Low — mechanical, but **blocks all parallelism** |
| 6..48 | Phase 3, one surface per iteration | Low each, cumulative |
| 49+ | Phase 4 | Low — additive |

**Not doing unless asked:** Oklab harmony scoring (2,357 LOC upstream) · jcode's 3D idle donut · the
15-widget HUD · inline interactive pickers (6,280 LOC, welded) · copy-mode edge autoscroll ·
`SoftRepaint` sentinel · mermaid diagram rendering (pinned git dep).

**The parity claim is not honest until baselines from Phase 1 + Phase 3.1–3.2 are reviewed
side-by-side against jcode at matched terminal sizes.**
