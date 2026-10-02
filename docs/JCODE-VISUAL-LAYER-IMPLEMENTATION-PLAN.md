# jcode Visual Layer — Full Port Implementation Plan

**Date**: 2026-10-03
**Baseline**: `origin/main` @ `edcead68` (iter-570)
**jcode source**: `parent-projects/jcode` @ `0a9dc7805` — verified on disk
**Supersedes**: the scope framing of [`JCODE-VISUAL-PARITY-PLAN.md`](JCODE-VISUAL-PARITY-PLAN.md)
(iter-570) and the sequencing of [`TUI-PORT-NEXT-OUTLINE.md`](TUI-PORT-NEXT-OUTLINE.md)
(iter-567). Decision taken: **full port**, not partial.
**Parent authority**: `.omo/plans/reaudit-tui-faithful-jcode-port.md` (gitignored, local only)

All figures below were measured at the stated baseline. Three parent-plan claims
were re-tested for this document; two failed (§0).

---

## 0. What is actually already ported — smaller than any plan claims

| Landed | What | State |
|---|---|---|
| iter-517 | `vendor/style/` — jcode-tui-style, 22 roles | **wired**: 44 consumer files, all roles read |
| iter-566 | 58% accent-blend selection (`selection_highlight.rs` port) | wired, 15 tests |
| iter-566 | `copy_targets.rs` — 744 LOC from `markdown_render_support.rs` + `markdown_types.rs` + `ui_prepare.rs` | wired for **detection**; badge consumer not yet built |
| iter-549 | modal erase → `Borders::ALL` → content (`ui_overlays.rs:15-21` port) | wired, 26 surfaces rebaselined |
| (earlier) | `clear_area` (`chrome.rs:4-10` port) in `render/utils.rs` | wired |

**That is the entire ported visual layer: ~3.5k LOC.** Everything that renders a
message, a tool row, the composer, the chrome, or a pane is still operant's own
code.

Two parent-plan claims re-tested and **failed**:

1. *"jcode-tui-messages (1,559) — already ported."* **False.** `DisplayMessage`
   does not exist anywhere in operant outside `vendor/`. `messages/` (3,501 LOC)
   is operant's own renderer ("Mirrors src/components/messages/ ... Messages.tsx")
   — a TypeScript mirror, not a jcode port. `app/scroll_anchor.rs` is likewise
   operant's own bookmark design, not jcode's `anchor.rs`.
2. *"jcode-tui-tool-display / jcode-tui-permissions — already ported."*
   **False.** Only fragments of tool-display live inside `copy_targets.rs`.

**Consequence: the seam does not exist.** Every jcode renderer is a pure
`(&DisplayMessage, width) -> Vec<Line<'static>>`. Without the data model, none of
the renderers can be ported. This is Wave 1, before anything else.

---

## 1. What is left to be ported — the complete inventory

Scope rule (parent D1, now the user's decision): **every file that decides a
pixel.** Measured by counting files that construct `Line`/`Span`/`Style`/`Color`
or write into a buffer, excluding `app/tests/`, `ui_tests/`, `*tests.rs`, and
`*loading.rs` (2 pixel hits — flow logic).

### W1 — The seam (blocks everything)
| Source | LOC | Contents |
|---|---|---|
| `jcode-tui-messages` (− `swarm_collapse.rs`) | **1,464** | `message.rs` `DisplayMessage`, `prepared.rs` pre-wrapped lines, `anchor.rs`, `cache.rs`, `wrapped_line_map.rs` |
| `jcode-tui-tool-display` | **257** | name canonicalization, middle truncation, failure detection |
| **W1 total** | **1,721** | plus the one operant→`DisplayMessage` adapter (D1a's single crossing point) |

### W2 — Primitives (blocks all rendering)
| Source | LOC | Contents |
|---|---|---|
| `jcode-tui-render` generic (`chrome.rs` 91, `layout.rs` 64, `lib.rs` 202) | **357** | `clear_area` (have), `render_rounded_box`, both truncate fns, `line_plain_text` (have, in copy_targets) |
| `jcode-tui-markdown` in-scope (`lib` 1,161, `render_full` 1,069, `render_lazy` 962, `wrap` 557, `support` 516, `text_preprocess` 197, `context` 162, `render_core_adapter` 211, `types` 74, `mermaid_fallback` 122) | **5,031** | code fence `┌─ lang`/`│ `/`└─`, tables, blockquote gutters, `Compact` spacing |
| **W2 total** | **5,388** | |

### W3 — Content renderers (sequential; the transcript)
| Source | LOC | Delivers |
|---|---|---|
| `ui_tools.rs` | 1,687 | tool rows, summary budget, 3-state icons, hide-by-default output |
| `ui_prepare.rs` | 2,721 | role dispatch, user bubble + rainbow ordinal, no-blank-separator rules |
| `ui_messages.rs` | 4,480 | `render_assistant_message`, tool assembly |
| **W3 total** | **8,888** | |

### W4 — Composer and viewport
| Source | LOC | Delivers |
|---|---|---|
| `ui_input.rs` | 3,052 | 4-state composer glyph, cursor by display width, notice taxonomy, status line compaction, zero-height palette overlay |
| `ui_viewport.rs` | 1,692 | scroll math, copy badges (**consumer of `copy_targets`**), truncation |
| `ui_inline.rs` + `ui_inline_interactive.rs` | 1,577 | inline UI band |
| **W4 total** | **6,321** | |

### W5 — The frame
| Source | LOC | Delivers |
|---|---|---|
| `ui.rs` | 3,713 | full-frame clear every tick, 10-band chrome, packed/scrolling, overlay early-return |
| `ui_overlays.rs` | 796 | erase → `Borders::ALL` → content (finishes iter-549's slice) |
| `ui_header.rs` | 1,767 | header — **operant has no header renderer at all** |
| `ui_animations.rs` | 714 | spinner phase-lock, donut band |
| **W5 total** | **6,990** | |

### W6 — Media and panes
| Source | LOC |
|---|---|
| `ui_inline_image.rs` 1,797, `ui_pinned.rs` 1,269, `ui_diagram_pane.rs` 973, `ui_panel_image_preview.rs` 105, `ui_file_diff.rs` 615, `ui_diff.rs` 489, `ui_todo_changes.rs` 392 | **5,640** |

### W7 — Surface replacement (converge operant's parallel designs)
| Source | LOC | Replaces |
|---|---|---|
| `info_widget*` (19 production files) | 8,549 | — (no operant counterpart) |
| `session_picker.rs` + `session_picker/render.rs` | 3,212 | `session_browser.rs` |
| `jcode-tui-usage-overlay` | 928 | operant's usage overlay |
| `jcode-tui-permissions` | 866 | inline permission dialog (jcode's is a full-screen `PermissionsApp`) |
| **W7 total** | **13,555** | |

### W8 — Auth/onboarding — **recommended cut line**
`login_picker.rs` 1,046 + `jcode-tui-account-picker` 1,607 + `ui_onboarding.rs` 745
= **3,398**. Operant's connect/device-auth dialogs work and already went through
the Wave-3 `modal_frame` migration. Full fidelity here costs 3.4k LOC on
rare-path modals. Recommendation: **cut**; port only if literal completeness is
the requirement. Escalated, not silently decided.

### W9 — Mermaid engine — **second cut line**
`jcode-tui-mermaid` = **10,215** — a diagram *compiler*, not chat chrome.
`mermaid_fallback` (122, in W2) already covers the degraded path, and
`ui_diagram_pane.rs` (973, in W6) is the pane that would consume it.
Recommendation: ship W2's fallback now; port the engine only if
diagrams-in-transcript is a real requirement.

### Totals
| | LOC |
|---|---|
| **W1–W7 (the full port)** | **48,503** |
| + W8 (auth) | 51,901 |
| + W9 (mermaid engine) | 62,116 |

Against ~3.5k already landed.

### Excluded, with cause (do not re-litigate)
| Excluded | LOC | Cause |
|---|---|---|
| `swarm_gallery`/`swarm_tiles`/`memory_tiles` | 4,297 | swarm visualizer; operant is single-agent. The chrome keeps a 0-height swarm band slot. |
| `ui_frame_metrics.rs` | 1,434 | perf telemetry, 4 pixel hits; operant has `debug_hub` |
| `markdown_latex_image.rs` | 1,294 | operant has `latex.rs` |
| `markdown_incremental.rs` | 311 | streaming-cache optimization; port if profiling demands |
| `session_picker/loading.rs` | 3,084 | 2 pixel hits — flow logic |
| `jcode-tui-visual-debug` | 857 | operant has `debug/overlay.rs` |
| `jcode-tui-core` (3,241) | on demand | shared types only; port the modules the renderers import (`copy_selection` is needed by W4), never wholesale |
| `app/**` in jcode-tui | — | D1a hard constraint: operant's loop core stays ours |

---

## 2. Wave 0 — Purge (no jcode source needed; start today)

1. Delete `vendor/render_core/` (4,608) and `vendor/workspace/` (1,246).
   **Zero consumers** outside `vendor/` — the sole mention is a doc comment at
   `copy_targets.rs:60`. 92 tests covering dead code go with them. The real
   markdown renderer arrives in W2 and replaces what render_core *was for*.
2. Delete `vendor/anim/` (940) — zero consumers. Re-port from source at W5 if the
   donut band ships. Cheaper than carrying dead code through five waves.
3. Remove the bridge that exists only because we adapted instead of ported:
   `pin_truecolor_for_tests` (7 sites), `color_depth::detect` ↔
   `color::color_capability` (18 sites). jcode has one palette, no depth
   negotiation, no Surface model.
4. Palette conformance: the **84** production literals across 17 files behind
   `theme_colors` roles (re-measured; the ~300 figure counted definitions and
   tests). `messages/` and `stats_dialog/` first; `debug/` last or never.

---

## 3. Dependency order and parallelism

```
W0 purge ──┐ (independent, today)
           ├──────────────────────────────► final rebaseline (§6)
W1 seam ─► W2 primitives ─► W3 tools ─► W3 prepare ─► W3 messages
                        │        └───────────► W4 input ∥ W4 viewport ─┐
                        └──► W5 frame (needs W2) ────────────────────────┤
                                  └► W6 panes (parallel, one per family) ┤
                                  └► W7 surfaces (parallel) ────────────┤
W8/W9 cut lines — only on explicit user say-so ──────────────────────────┘
```

- **W1→W2→W3 is strictly sequential.** Nothing renders without the seam.
- **W3 internal order is forced**: tools → prepare → messages.
- **One ordering constraint is a trap, not a preference**: `markdown_render_full`
  (W2) **must** land before `ui_messages` (W3). The `│ ` fence gutter *is* the
  copy-target detection mechanism, and `copy_targets.rs` already depends on that
  shape. Fence first, copy silently breaks.
- W4's two files are disjoint — parallelisable after W2.
- W6/W7 shard cleanly: one agent per pane family, one scenario + one baseline each,
  zero shared edits.

---

## 4. Per-wave acceptance (no wave is "done" without all of these)

1. **Ported verbatim**, structure preserved, jcode doc comments stripped, MIT
   attribution header added. Adaptation is a bug, except at the single seam
   (operant message → `DisplayMessage`).
2. **Mutation-proven.** Neuter each ported renderer; *exactly* the expected tests
   fail. iter-566 shipped a test deriving its expectation from the constant under
   test — `0.58 → 0.30` stayed green. Pin expectations to literals.
3. **Golden scenario per surface**, added to the corpus the same iteration.
   `_validate.py` must stay 0-missing / 0-extra.
4. **`prove` then `verify`**: double-render, byte-diff, regenerate, third-render
   re-verify. A build or `cargo test` executes no screen assertions.
5. **Cutover**: when a wave replaces an operant renderer, the old one is deleted
   in the same iteration. No parallel render paths survive past W7 — enforced
   finally by a grep gate (no `Line::`/`Span::` construction outside the ported
   layer and `vendor/style`).
6. Scoped `check.sh` clean, 0 warnings `--all-targets`, fmt, clippy gate no-new.

---

## 5. Trap register (carried from the parent plan; each is a real defect if ignored)

| Trap | Where |
|---|---|
| 96-col cap is **centered-mode only**; default is `centered: false` — full width. Porting it unconditionally crams a 200-col terminal | `cache.rs:88-95` |
| `use_packed` is a bare `<=` with **no dead-band** while the scrollbar 40 lines up has hysteresis — port packed **with** hysteresis, a deliberate divergence, called out in the commit body | `ui.rs:3113-3178` |
| Notification band is **1 row** for every source except one | `ui_input.rs:1928-1937` |
| Copy-badge pool already excludes `h`/`j`/`k`/`l` | `ui.rs:557-559` |
| `mouse_capture = true` **disables native terminal selection**; the custom machinery is the substitute. The 58% blend is already live — users lose copy entirely if W4 regresses it | `display.rs:23-24` |
| Palette collision rule: any new role **must** re-run the collision test in `theme_colors.rs` — two real collisions exist (`dark.success` ≡ `Ai`, `muted` ≡ `Tool`) | `theme_colors.rs` |
| Overscroll band sits **below** input; the conditional 1-row gap is easy to omit | `ui.rs:3182-3215` |
| jcode **non-findings** — do not invent: no `NO_COLOR` in the TUI, no `warning` transcript role, no expand/collapse affordance, no composer placeholder, no streaming-prose cursor, no vim composer modes upstream (operant's `prompt_input/vim.rs` **survives** on our own authority) | parent §2c |

---

## 6. Final rebaseline (once, deliberate)

Current goldens encode operant's layout — that is correct today and wrong the
moment W3 lands. Protocol:

1. Land waves; let scenarios fail only where the port is mid-flight.
2. At the end (after W7): regenerate all goldens **once**, in a **single dedicated
   commit** with a rationale body, per the parent's Phase 5 rule.
3. Third-render re-verify — 0 unstable.
4. Close the `selection-highlight` park: it needs a mouse-injection seam on the
   simulated event pump plus a style baseline. Until then the 58% blend has no
   golden coverage — the one regression that is currently invisible.

---

## 7. Estimate

48.5k LOC across W1–W7 at the fleet's observed pace (iter-517 vendored 9.6k with
tests green in one iteration; Wave 3 of the old roadmap moved 38 files across 4
agents in two iterations). Rough read: **15–25 focused iterations**, sequential
through W3, highly parallel from W4 onward. W0 + W1 is the first deliverable that
changes what a user sees — everything before W3 is plumbing.
