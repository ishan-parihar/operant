# Family 06 — terminal diagram rendering (mermaid)

**Question this file answers:** codex draws mermaid diagrams inside the terminal with box-drawing
characters. operant prints the source. Is the gap real, is the import small, and why was operant's
equivalent cut?

**Ground state:** operant `main` @ `f53907e8` (2026-10-31). Reference
`../external-repos/codex/codex-rs` @ `588a277b`.

---

## CODEX-SIDE

### What the mechanism is

codex ships a **standalone, bounded, dependency-free Mermaid renderer** (`mermaid/`) that emits
plain Unicode text — no terminal graphics protocol, no image, no Mermaid runtime. It parses five
diagram families (flowchart/graph, sequence, state, class, ER), lays them out with orthogonal
routing and one lane per edge, and returns either the drawn lines or an error. Unsupported syntax
returns `Err` and the caller keeps the source.

The crate declares **one production dependency: `unicode-width`** (`mermaid/Cargo.toml`), which is
already in operant's dependency set (`crates/operant-cli/Cargo.toml:47`).

### The load-bearing units (file:line + `wc -l`)

| unit | file:line | LOC | what it owns |
|---|---|---|---|
| `render` / `render_spans` entry + `RenderError` | `mermaid/src/lib.rs:59-79` | 172 (whole file) | the 5-way family dispatch, `MAX_SOURCE`/`MAX_NODES`/`MAX_EDGES`/`MAX_LABEL`/`MAX_CELLS` caps (`lib.rs:22-26`) |
| `syntax::statements` | `mermaid/src/syntax.rs:1` | 101 | statement splitting, comments, `;` separators |
| `parse::parse` (flowchart) | `mermaid/src/parse.rs:1` | 227 | node/edge grammar, `@@`-free context tokens |
| `relations::parse` | `mermaid/src/relations.rs:1` | 241 | class + ER grammars |
| `sequence::render` | `mermaid/src/sequence.rs:1` | 306 | lifelines, messages, nested loop/alt/opt |
| `state::render` | `mermaid/src/state.rs:1` | 119 | flat state machines, `[*]` initial/final |
| `draw::render` | `mermaid/src/draw.rs:1` | 263 | orthogonal routing, `┌┐└┘│─◄►` box drawing |
| `output` (`Cell`, `finish`) | `mermaid/src/output.rs:1` | 69 | the canvas primitive |
| **non-test total** | | **1,498** | |
| `tests.rs` + `families_tests.rs` | `mermaid/src/tests.rs`, `families_tests.rs` | 930 | 21 tests |

### The failure contract (the part that matters most)

`RenderError` is **three variants** (`lib.rs:32-42`):

```rust
pub enum RenderError {
    Unsupported,   // syntax or text outside the supported subset
    Limit,         // a source/diagram/label/canvas limit exceeded
    TooWide,       // exceeds the caller's display width
}
```

and `lib.rs:58-79` documents: **"no partial result is returned"** — every error leaves the caller
holding only the source. Caps are absolute and cheap: `MAX_SOURCE` 16 KiB, `MAX_NODES` 16,
`MAX_EDGES` 24, `MAX_LABEL` 40 chars, `MAX_CELLS` 64 Ki (`lib.rs:22-26`).

### The TUI integration is 91 lines and degrades to source

`tui/src/markdown_render/mermaid.rs` (91 LOC, quoted whole below because it is the template
operant would copy):

```rust
pub(super) fn render(source: &str, width: Option<usize>, theme: &Theme) -> Vec<Line<'static>> {
    let width = width.unwrap_or(120);
    let diagram = match codex_mermaid::render_spans(source, width) {
        Ok(diagram) => diagram,
        Err(error) => {
            let notice = match error {
                RenderError::Unsupported => "This Mermaid diagram uses features the terminal renderer doesn't support.",
                RenderError::TooWide    => "This Mermaid diagram doesn't fit the current terminal width.",
                RenderError::Limit      => "This Mermaid diagram exceeds the terminal renderer's size limits.",
            };
            let mut lines = textwrap::wrap(notice, width.max(1)).into_iter()
                .map(|line| Line::from(line.into_owned().dim())).collect::<Vec<_>>();
            lines.extend(highlight_code_to_lines(source, "mermaid"));
            return lines;
        }
    };
    // …theme-aware Node/Edge/Text spans…
}
```

Three properties of that integration, each deliberate:

1. **Source is never lost.** Every failure path emits the original, syntax-highlighted block. The
   diagram is an *enhancement*, never a gate on the content.
2. **The reason is specific and visible.** `Unsupported` vs `TooWide` vs `Limit` produce three
   different operator-facing sentences — this is the difference between "your diagram is too big"
   and "this grammar is not implemented".
3. **Theme-aware, not monochrome.** `Role::{Node, Edge, Text}` is a 3-variant span role
   (`lib.rs` re-exports `output::Role`), mapped to the live theme.

### What codex does NOT build

No Mermaid runtime, no headless browser, no rasteriser, no SVG parser, no image protocol. There is
no `mmdc` subprocess. The whole subsystem is text in, text out (`lib.rs:14`: "This crate performs
no I/O and does not depend on a Mermaid implementation").

---

## OPERANT-SIDE

### What exists today

**1. The integration point exists and is already wired — it just always says no.**

`crates/operant-cli/src/tui/operant_markdown/mod.rs:167-171`:

```rust
fn should_render_mermaid_block(lang: Option<&str>) -> bool {
    mermaid_rendering_enabled()
        && lang.map(mermaid::is_mermaid_lang).unwrap_or(false)
        && mermaid::native_image_protocol_available()
}
```

and `markdown_mermaid_fallback.rs:41`:

```rust
pub fn native_image_protocol_available() -> bool { false }
```

So the third conjunct is **permanently false**. Both settled rendering
(`markdown_render_full.rs:423`) and streaming rendering (`:993`) branch on this predicate, so a
mermaid fence always takes the `else` arm: a **plain syntax-highlighted code block labelled
`mermaid`**. The dual-mode/deferred/registration apparatus (`render_mermaid_deferred_with_stream_scope`,
`render_mermaid_deferred_with_registration`, `render_mermaid_untracked`, `render_mermaid_sized`,
`markdown_mermaid_fallback.rs:54-78`) all return
`RenderResult::Error("Mermaid rendering is disabled")`, and `native_image_protocol_available()`
returning `false` means none of them is ever called.

**2. operant has the richer image plumbing already built.**

`crates/operant-cli/src/tui/pinned_images.rs` (module doc, `:1-40`) is a whole persistence layer for
terminal graphics: a `RefCell<Vec<…>>` registry of placed graphics on `App`, `paint_strip` re-emitting
each frame, Kitty/Sixel/iTerm2 sequence handling. Its doc explicitly names mermaid rasters as one of
the two things it keeps alive. So operant's *display* stack is strictly more capable than codex's —
it can show an image where codex shows text. **It just has no producer.**

**3. Why it is disabled: a porting decision, recorded and escalated.**

`crates/operant-cli/src/tui/operant_markdown/markdown_mermaid_fallback.rs:1-6`:

```
// Vendored from jcode (…/markdown_mermaid_fallback.rs), MIT License …
// Ported verbatim @ 0a9dc7805, no changes; this module is unconditionally
// compiled in operant (mermaid engine cut per plan W9).
```

`docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md:132-142` is the decision:

> ### W9 — Mermaid engine — **second cut line**
> `jcode-tui-mermaid` = **10,215** — a diagram *compiler*, not chat chrome.
> `mermaid_fallback` (122, in W2) already covers the degraded path, and
> `ui_diagram_pane.rs` (973, in W6) is the pane that would consume it.
> Recommendation: ship W2's fallback now; port the engine only if
> diagrams-in-transcript is a real requirement.

And the test file carries the recovery marker:
`markdown_tests/cases/streaming_cache.rs:342` —
`// [port-decision] stays gated: … recover when W9 un-cuts`.

**4. The critical measurement: the cut was of a different, 6.8× larger thing.**

| | operant's cut target | codex's actual subsystem |
|---|---|---|
| size | **10,215** LOC | **1,498** non-test (2,428 total) |
| shape | a diagram *compiler* (rasterising engine) | a bounded text renderer |
| deps | needs a graphics/raster stack | `unicode-width` only |
| output | an image | Unicode lines |

The W9 decision was made about a **10,215-LOC compiler** that would have required the pending
batch-4 `ratatui_image` dependency. codex's subsystem is a different design with a different cost.
**The decision's premise ("a diagram compiler") does not describe the thing that is actually
available to import.** That is not an error in the original reasoning — it is a premise that a
later measurement invalidated, and the honest move is to re-derive, not to treat W9 as settled.

### The gap, stated precisely

- **Feature gap:** every mermaid fence in every transcript renders as source text. The agent can
  emit a perfectly valid `sequenceDiagram` and the operator reads the grammar, not the diagram.
- **Not a bug:** nothing is broken. The code block is legible and correct; `RenderResult::Error`
  is never even reached because the gate short-circuits first.
- **Asymmetry worth noting:** operant has *more* display capability than codex (real image pinning)
  and *less* rendering capability. The import converts codex's text-render path into a producer
  for a display stack operant already owns.

---

## VERDICT

| mechanism | codex unit + LOC | operant equivalent | gap severity | portable? | verdict |
|---|---|---|---|---|---|
| Bounded mermaid text renderer | `mermaid/` 1,498 non-test | `native_image_protocol_available() -> false` (`markdown_mermaid_fallback.rs:41`) | **high** (visible, daily) | yes, `unicode-width` already a dep | **IMPORT** |
| 91-LOC degrade-to-source integration | `tui/src/markdown_render/mermaid.rs:38-91` | the branch sites exist (`markdown_render_full.rs:423`, `:993`); the gate is the blocker | — | yes, drop-in pattern | **IMPORT** |
| Image/raster path | — | `pinned_images.rs` whole module | operant ahead | — | **ALREADY-HAVE** |
| `ratatui_image` dependency | — | never adopted (W9's blocker) | — | — | **DO-NOT** (still unnecessary for the text path) |

**The ponytail shape of this import:** it does not touch the agent loop, the prompt, the tools, or
the frozen prefix. It is a new leaf module in the TUI plus one changed boolean. The 930-LOC test
file ports with it and is almost entirely golden-line assertions, which is the cheapest possible
test surface for a renderer.

**The honest counter-argument, recorded rather than dismissed:** W9 said "port the engine only if
diagrams-in-transcript is a real requirement." That condition has **not** been measured. The
measurement that would settle it is a count of mermaid fences in real operant transcripts, and no
such count exists in this repo. So this row is **IMPORT — pending the operator's confirmation that
diagrams in transcript are actually wanted**, and it is ranked below the rows whose need is already
proven by a defect.
