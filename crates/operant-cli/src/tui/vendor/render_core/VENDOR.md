# Vendored: jcode-render-core

Backend-neutral document/render model — markdown parsing, the semantic block/span model,
LaTeX preprocessing and terminal-friendly math rendering, and width-parameterized wrapping.
Vendored from jcode as an internal operant module. Upstream is a standalone Cargo crate; here
it is a directory module under `crates/operant-cli/src/tui/vendor/render_core/`, wired in by
the integrator via `tui::vendor::mod.rs` (not owned by this vendoring pass).

This is the crate that backs the future rewritten message renderer: it produces a neutral
`Document` of styled spans, and each front-end supplies its own width measurer and colour-role
adapter.

## Upstream

| Field | Value |
|---|---|
| Repository | https://github.com/1jehuang/jcode |
| Path | `crates/jcode-render-core/` |
| Commit | `0a9dc7805db1d264bdaa96b6b8cea83c2c915a80` |
| Files | `Cargo.toml` + 7 `src/*.rs` + 2 `tests/*.rs` (4,532 LOC) |
| Licence | MIT, Copyright (c) 2025 Jeremy Huang |
| Upstream deps | `pulldown-cmark = "0.12"`, `serde = { version = "1", features = ["derive"] }`, `unicode-width = "0.2"` |

## External-reference audit (the brief asked this to be verified, not assumed)

Upstream's own `Cargo.toml` comment states the crate has **no** dependency on ratatui or any
GPU/glyph layer. Verified, not assumed:

- Every non-`std` path imported anywhere in the crate is exactly `pulldown_cmark`,
  `serde`, `unicode_width`, or `super::`/`crate::` (its own modules).
- Every symbol of those three crates used is present in operant-cli's existing dependency set
  (see *Dependency reconciliation* below).
- **No filesystem path, cache/config directory, environment variable, or `~/.`-style path
  appears anywhere in the crate.** The only string literals resembling paths are Windows drive
  letters inside *test fixtures* (`C:\work` in a reasoning round-trip case) and a
  `https://example.com` markdown link — both are test data, not paths the code touches.
- The only jcode identifiers were in doc comments and in the two integration-test
  `use jcode_render_core::…` lines. All removed.

## Files here

| File | Upstream counterpart | LOC (after header) |
|---|---|---|
| `mod.rs` | `src/lib.rs` | 53 |
| `markdown.rs` | `src/markdown.rs` | 733 |
| `math.rs` | `src/math.rs` | 1251 |
| `model.rs` | `src/model.rs` | 279 |
| `preprocess.rs` | `src/preprocess.rs` | 988 |
| `reasoning.rs` | `src/reasoning.rs` | 157 |
| `wrap.rs` | `src/wrap.rs` | 192 |
| `tests.rs` | `src/tests.rs` | 240 |
| `tests_latex_robustness.rs` | `tests/latex_robustness.rs` | 525 |
| `tests_latex_streaming_regressions.rs` | `tests/latex_streaming_regressions.rs` | 190 |
| **total** | | **4,608** |

`Cargo.toml` is intentionally **not** reproduced; see below.

## Dependency reconciliation — one real version difference

`pulldown-cmark` is the only dependency whose version differs between the two projects:

| | Upstream | operant-cli (workspace lock) |
|---|---|---|
| `pulldown-cmark` | `"0.12"` | **`0.13.4`** (`default-features = false`) |

This module therefore compiles against **0.13.4**, not 0.12. Every API the crate uses was
checked against the 0.13.4 source in the local registry and is unchanged in shape:

- `Options::{ENABLE_TABLES, ENABLE_FOOTNOTES, ENABLE_STRIKETHROUGH, ENABLE_TASKLISTS,
  ENABLE_SMART_PUNCTUATION, ENABLE_MATH, ENABLE_GFM, ENABLE_DEFINITION_LIST}` — all present.
- `Parser::new_ext(&str, Options) -> Parser` — unchanged.
- `Tag::{Paragraph, Heading{level,..}, BlockQuote(Option<BlockQuoteKind>), CodeBlock(CodeBlockKind),
  List(Option<u64>), Item, FootnoteDefinition(CowStr), DefinitionListTitle,
  DefinitionListDefinition, Table(Vec<pulldown_cmark::Alignment>), Emphasis, Strong,
  Strikethrough, Link{dest_url,..}, Image{dest_url,..}}` — all present with the same payload
  shapes. `Tag::Heading`'s `level` is still `HeadingLevel`, and `level as u8` still works.
- `TagEnd::{Paragraph, Heading(_), BlockQuote(_), List(_), Item, CodeBlock, FootnoteDefinition,
  DefinitionListTitle, DefinitionListDefinition, Table, TableHead, TableRow, TableCell,
  Emphasis, Strong, Strikethrough, Link, Image}` — all present.
- `Event::{Start, End, Text, Code, InlineMath, DisplayMath, Html, InlineHtml,
  FootnoteReference, SoftBreak, HardBreak, Rule, TaskListMarker}` — all present, same payloads.

`default-features = false` on operant-cli's dependency only drops the `getopts` and `html`
features (binary-CLI plumbing and `pulldown-cmark-escape`); neither is used here, and `Event`/
`Parser` are unconditional.

`serde` and `unicode-width` need no reconciliation — operant-cli already depends on both at
compatible versions and both resolve to the same single crate version in the workspace lock, so
the `Serialize`/`Deserialize` derives and `UnicodeWidthStr`/`UnicodeWidthChar` behave identically.

## Adaptations

1. **Crate root → module root.** `src/lib.rs` became `mod.rs`; its `pub mod` and `pub use`
   declarations are otherwise unchanged, so the public API is identical to upstream's crate root.
2. **Intra-crate paths `crate::` → `super::`.** Upstream's modules referred to each other as
   `crate::model`, `crate::math`, `crate::preprocess`, `crate::wrap`. Inside
   `tui::vendor::render_core`, `crate::` resolves against **operant-cli's** root, so every one
   of those had to become `super::`. 12 occurrences across `markdown.rs` (6 code + 2 intra-doc
   links), `wrap.rs` (1), and `tests.rs` (2). No logic touched.
3. **Integration tests became cfg(test) submodules.** Upstream's `tests/latex_robustness.rs` and
   `tests/latex_streaming_regressions.rs` are separate Cargo integration targets and cannot exist
   as files under a module directory. They became `#[cfg(test)] mod tests_latex_robustness;` /
   `mod tests_latex_streaming_regressions;` in `mod.rs`, with their `use jcode_render_core::{…}`
   rewritten to `use super::{…}`. Test *content* is otherwise byte-identical — same test names,
   same assertions, so the same counts are reported per crate.
4. **Fourteen `.unwrap()`/`.expect()` sites removed from production code** (operant's clippy gate
   denies `clippy::unwrap_used` and `clippy::expect_used`). All seven production sites were
   provably infallible upstream; each is now an equivalent non-panicking form with a comment
   recording why the branch was unreachable:

   | Site | Upstream | Here |
   |---|---|---|
   | `preprocess.rs` (promote closing `$`) | `.expect("matched closing marker")` | `let Some(close_start) = … else { copy line verbatim; continue }` |
   | `preprocess.rs` (main scan cursor) | `.expect("index stays on a char boundary")` | `let Some(ch) = … else { break }` |
   | `reasoning.rs` (escape unescape) | `chars.next().unwrap()` | `&& let Some(next) = chars.next()` in the guard |
   | `math.rs` `collapse_sequence` | `items.pop().unwrap()` under `len() == 1` | `if items.len() == 1 && let Some(only) = items.pop()` |
   | `math.rs` `split_matrix` ×3 | `rows.last_mut().unwrap()` | `if let Some(row) = rows.last_mut()` |

   Each preserves upstream behaviour on every reachable input and degrades to copying input
   verbatim instead of aborting the process on the unreachable one. **This is the only change in
   this crate that touches executable code.**
5. **Doc-comment de-jcode-ing.** `mod.rs`, `markdown.rs` and `reasoning.rs` named upstream
   sibling crates (`jcode-tui-markdown`, `jcode-tui-markdown::reasoning_summary_line_markup`,
   `jcode-render-core`, `jcode-tui-*`) and upstream's "front-end" topology. Rewritten to describe
   operant's situation (operant's existing renderer is authoritative; the backend-neutral
   adapter seam stays). `model.rs` and `wrap.rs` docs mentioned no upstream crate — `model.rs`'s
   said "the TUI markdown renderer", which in operant is accurate, so it was left alone;
   `wrap.rs`'s said "the desktop measures in pixels", which does not apply to operant, so that
   sentence was generalized to "other front-ends".
6. **Licence header.** Every `.rs` file carries the two-line MIT attribution header required by
   the vendoring brief.

## Verified against operant-cli

- Compiles as `operant_cli::tui::vendor::render_core` under the workspace toolchain
  (`rustc 1.98.0`, edition 2024).
- No `.unwrap()` / `.expect()` in production code; the only remaining ones are inside
  `#[cfg(test)]` modules, which operant exempts.
- No new dependency: `pulldown-cmark`, `serde`, `unicode-width` were already direct dependencies
  of `operant-cli`.

## Tests ported

| Module | Tests | Covers |
|---|---|---|
| `tests` (was `src/tests.rs`) | 16 | headings, paragraphs+emphasis, inline code, inline/display math, all latex containers, fenced code, ordered/unordered/nested lists, blockquote, thematic break, wrap styling/width, hard-split, alignment, no-op wrap |
| `math::tests` | 10 | inline notation, fraction display layout, matrix tall brackets, unknown-command preservation, malformed input, 20k-deep nesting on a 256 KiB stack, braced matrices + cases, matrix scanner, unmatched environment, 256-case deterministic fuzz |
| `preprocess::tests` | 11 | currency escaping, display-math passthrough, inline/fenced code skipping, `\(..\)`/`\[..\]` normalization, display environments + nesting, math fences, literal-code preservation, currency-vs-latex interaction |
| `reasoning::tests` | 2 | markup round-trip over escaped emphasis, sentinel-required decoder |
| `tests_latex_robustness` | 20 | full symbol vocabulary, script characters, formatting commands, accents/lines/delimiters, fraction-root-script geometry, every matrix environment family, ragged/nested matrices, ordinary display environments, unknown-command debuggability, 26 malformed constructs, 980-case generated grammar corpus, 20k-command depth + 40k-char flat input, every math-fence spelling, all literal code forms, fence-lookalike content, delimiter balance/escaping, all 25 display environments, nesting mismatch, markdown-pipeline structure, container parity |
| `tests_latex_streaming_regressions` | 9 | the exact multiline equation response, per-prefix determinism, markdown container survival, block-interruptor resistance, CRLF + adjacent blocks, newline/comment-preserving stabilization, nested quote+list containers, escaped/literal delimiters, standalone inline promotion |

All 68 pass under the module-path filter `tui::vendor::render_core`.