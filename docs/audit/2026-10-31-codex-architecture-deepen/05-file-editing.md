## CODEX-SIDE

### What the mechanism is

Codex applies edits with a purpose-built, model-facing patch grammar — not unified diff, not JSON find/replace.
A `parse_patch` turns the raw text into a `Vec<Hunk>`, each hunk is Add/Delete/Update, and
`derive_new_contents_from_chunks` computes replacements in memory, then writes whole-file. The grammar is
deliberately LLM-shaped: it does not carry line numbers, so a patch can be authored without knowing the
current file state.

The critical architectural fact: `parse_patch` only parses. The Lark comment at `apply-patch/src/parser.rs:5-23`
states the module "does not attempt to actually check that the patch can be applied to the filesystem."
Validation and application are separate stages (`apply_hunks_to_files`, `apply-patch/src/lib.rs:438`).

### The patch grammar (quote the markers)

From `apply-patch/src/parser.rs:37-45`, the marker constants are the grammar's whole surface:

```rust
pub(crate) const BEGIN_PATCH_MARKER: &str = "*** Begin Patch";
pub(crate) const END_PATCH_MARKER: &str = "*** End Patch";
pub(crate) const ADD_FILE_MARKER: &str = "*** Add File: ";
pub(crate) const DELETE_FILE_MARKER: &str = "*** Delete File: ";
pub(crate) const UPDATE_FILE_MARKER: &str = "*** Update File: ";
pub(crate) const MOVE_TO_MARKER: &str = "*** Move to: ";
pub(crate) const EOF_MARKER: &str = "*** End of File";
pub(crate) const CHANGE_CONTEXT_MARKER: &str = "@@ ";
pub(crate) const EMPTY_CHANGE_CONTEXT_MARKER: &str = "@@";
```

The formal grammar, quoted from `parser.rs:5-23`:

```
! start: begin_patch environment_id? hunk+ end_patch
! begin_patch: "*** Begin Patch" LF
! environment_id: "*** Environment ID: " filename LF
! end_patch: "*** End Patch" LF?
! hunk: add_hunk | delete_hunk | update_hunk
! add_hunk: "*** Add File: " filename LF add_line+
! delete_hunk: "*** Delete File: " filename LF
! update_hunk: "*** Update File: " filename LF change_move? change?
! filename: /(.+)/
! add_line: "+" /(.+)/ LF -> line
! change_move: "*** Move to: " filename LF
! change: (change_context | change_line)+ eof_line?
! change_context: ("@@" | "@@ " /(.+)/) LF
! change_line: ("+" | "-" | " ") /(.+)/ LF
! eof_line: "*** End of File" LF
```

Note `@@` is a **bare context anchor**, never a line range — the immediately-following doc at `parser.rs:24-25`
says the parser is "a little more lenient than the explicit spec and allows for leading/trailing whitespace
around patch markers."

### The load-bearing units (with file:line and wc -l)

| unit | file:line | LOC | what it owns |
|---|---|---|---|
| `seek_sequence` | `apply-patch/src/seek_sequence.rs:12-111` | 164 (whole file) | 4-tier fuzzy context matcher. Zero crate imports — pure function over `&[String]` |
| `text_file.rs` (`SourceFile`) | `apply-patch/src/text_file.rs:1-121` | 121 (whole file) | Line-splitting that retains each line's ending, `LineEnding::{Lf,CrLf,Cr}` (`:4-8`), `apply_replacements` (`:84`), `into_contents` (`:111`) |
| `file_update.rs` | `apply-patch/src/file_update.rs:60-160` | 204 (whole file) | Turns chunks into `(start, old_len, new_lines)` replacements; drives seek_sequence |
| `parser.rs` (grammar + `Hunk`) | `apply-patch/src/parser.rs:37-682` | 682 (whole file) | Markers, `Hunk` enum, `UpdateFileChunk`, `ParseError` |
| `invocation.rs` | `apply-patch/src/invocation.rs:1-942` | 942 (whole file) | tree-sitter-bash heredoc extraction so `apply_patch <<'EOF'` works |
| `lib.rs` (`apply_hunks_to_files`) | `apply-patch/src/lib.rs:438-691` | 1408 (whole file) | Multi-hunk apply loop, `AppliedPatchDelta`, `ApplyPatchError` |
| `streaming_parser.rs` | `apply-patch/src/streaming_parser.rs:1-924` | 924 (whole file) | Incremental parse emitting UI events. Not on the apply path |

### What streaming buys

**Nothing on the apply path.** Verified precisely:

`StreamingPatchParser` has exactly **one** consumer outside its own crate — `core/src/tools/handlers/apply_patch.rs:49`
(import) and `:81` (field). Its driver at `apply_patch.rs:88-115`:

```rust
fn consume_diff(&mut self, turn: &TurnContext, call_id: String, diff: &str) -> Option<EventMsg> {
    if !turn.config.features.enabled(Feature::ApplyPatchStreamingEvents) { return None; }
    self.push_delta(call_id, diff).map(EventMsg::PatchApplyUpdated)
}
```

Gated behind `Feature::ApplyPatchStreamingEvents` (`features/src/lib.rs:177`), enabled only in tests. Its output is
`PatchApplyUpdatedEvent` — a UI notification, debounced by `APPLY_PATCH_ARGUMENT_DIFF_BUFFER_INTERVAL`
(`apply_patch.rs:117-131`). It never writes a byte. Actual application is a separate non-streaming call through
`maybe_parse_apply_patch_verified` (`invocation.rs:163`) → `parse_patch` → `apply_hunks_to_files`.

So "can the model emit a patch as it is still being generated": the parser tolerates it, but what it buys is a
**live progress render**, not earlier application. A non-streaming parser is strictly sufficient for correctness.

### Smallest portable unit for each row

- `seek_sequence` — **fully portable as-is.** No `use` statements at all; only `&[String]`, `&str`, `usize`. Drop-in.
  Its 4th tolerance tier is `fn normalise(s: &str)` at `seek_sequence.rs:77-110`; the EOF-preference block is `:28-33`.
- `text_file.rs` — **fully portable as-is.** No `use` statements; `Replacement` is a local type alias (`:1`). Drop-in.
  This is the unit that fixes mixed line endings, the most under-appreciated correctness property.
- `file_update.rs` — **portable after removing 5 imports** (`file_update.rs:1-10`): `codex_exec_server::*`,
  `codex_utils_path_uri::PathUri`, `similar::TextDiff`. The core, `compute_replacements` (`:60-160`), needs only
  `seek_sequence` + `text_file` + `UpdateFileChunk`. `similar` is used *only* in
  `unified_diff_from_chunks_with_context` (`:186-197`) for display.
- `parser.rs` — **portable after removing 3 imports** (`parser.rs:26-33`): drops `codex_utils_absolute_path`,
  `codex_utils_path_uri`, `streaming_parser`. `thiserror` is needed for `ParseError`.
- `streaming_parser.rs` — **not portable, and should not be.** Dead behind a test-only feature flag.
- `invocation.rs` — **not portable.** Coupled to codex's shell-execution model.

**Direct answer:** yes, `seek_sequence` + `text_file` + `compute_replacements` are portable **without** the
streaming parser and invocation. That combination is ~350 LOC with zero non-stdlib dependencies once `similar`
and the exec-server types are stripped.

Error taxonomy: `ParseError::{InvalidPatchError, InvalidHunkError}` (`parser.rs:60-66`) and
`ApplyPatchError::{ParseError, IoError, ComputeReplacements, PathUri, ImplicitInvocation}` (`lib.rs:75-92`).
A failed parse or compute leaves the file untouched (both happen before any write; `derive_new_contents_from_chunks`
is fully in memory at `file_update.rs:25-57` and returns before `write_file_with_missing_parent_retry` is reached).
A failed *write* may have already modified the target. And there is **no cross-file rollback**: a grep for
`rollback|revert|undo|restore` in `apply-patch/src/lib.rs` returns zero matches, so a multi-file patch failing on
hunk 3 leaves hunks 1 and 2 applied.

## OPERANT-SIDE

### What exists today

**1. The grammar exists, but as a shell over an external binary, not a native parser.**

`AftApplyPatchTool` (`crates/operant-core/src/tools/aft_tools.rs:615-668`) accepts a full `*** Begin Patch` body
and does two things: path-scan every `*** Update/Add/Delete File: ` header to run `guard_aft_write`
(`aft_tools.rs:645-655`), then forward the raw text to the subprocess (`aft_tools.rs:657-664`).

Its description (`aft_tools.rs:627-630`) explicitly teaches the grammar, including the `@@`-is-a-bare-anchor rule
and "Multiple files in one call are allowed." So **the model already has multi-file, multi-hunk editing** — through
the aft binary. Registered at `aft_tools.rs:46`, gated at `crates/operant-cli/src/main.rs:1263`
`if config.tools.aft_enabled`, default `true` (`config.rs:1857`).

**2. The critical gap: when aft dies, the capability silently collapses to one string replace.**

`main.rs:1306-1317` — native `patch` is disabled only "if the bridge is proven live":

```rust
if live {
    aft_replaced_tools.extend([
        "file_read".to_string(), "file_write".to_string(),
        "patch".to_string(), "terminal".to_string(),
        "file_search".to_string(), "file_list".to_string(),
    ]);
}
```

If the bridge is not live (`main.rs:1322-1332`), native tools remain — including `patch` (`builtin.rs:146`).
And that native `patch` is **single find/replace over the whole file**:

```rust
// crates/operant-core/src/tools/patch_tool.rs:20-29
struct PatchArgs { path: String, find: String, replace: String }
```

Apply logic at `patch_tool.rs:172-186`: `let exact_count = content.matches(&args.find).count();` at `:172`,
then if `exact_count > 0` a whole-file `content.replace()`, else `fuzzy_replace`, else error. So "edit 5 places in
this file" becomes 5 sequential full-file rewrites, and `matches().count()` will happily rewrite **every**
occurrence when the model intended one (`:172` has no occurrence index or uniqueness requirement).

**3. operant's fuzzy fallback is weaker than seek_sequence, and has dead code.**

`normalize_whitespace` at `patch_tool.rs:34-39` collapses all runs of whitespace to a single space per line.
`fuzzy_replace` at `patch_tool.rs:45-119` then does an O(n·m) line-block scan. Three concrete deficiencies:

- No 4-tier degrading match. It is one tier: full whitespace collapse.
- No EOF preference. `seek_sequence.rs:28-33` tries `lines.len() - pattern.len()` first when `eof` is set.
- No Unicode normalization. `seek_sequence.rs:77-110` maps typographic dashes/quotes/NBSP to ASCII.

The dead branch at `patch_tool.rs:95-99`:

```rust
if replace.is_empty() {
    // Don't add anything
    if !result_lines.is_empty() || i > 0 {
        // already handled
    }
}
```

A no-op branch that does nothing.

**4. The one thing operant does better: per-file atomic write with mode preservation.**

`atomic_write` at `crates/operant-core/src/tools/file_tools.rs:65-108`. Same-directory `NamedTempFile`,
`sync_all()`, drop-on-failure so a crash mid-write never corrupts the target, and Unix mode preservation including
the 0666 & ~umask computation for new files. Codex's `write_file_with_missing_parent_retry` (`lib.rs:765-818`) is a
plain `fs.write_file` through a sandbox abstraction with a NotFound retry — no temp file, no rename, no mode
preservation. **operant wins this row outright.**

**5. The diff-viewing side already parses real unified diffs, but display-only.**

`crates/operant-cli/src/tui/diff_viewer/parse.rs:38-160` — `parse_unified_diff` turns `git diff HEAD` text into
`FileDiffStats`, with a real `@@ -old,count +new,count @@` header parser at `parse.rs:146-161`. 166 LOC. There is
**no write, apply, or replace call anywhere in that file**. Total `diff_viewer/` = 1195 LOC.

`crates/operant-core/src/tools/working_diff_tool.rs` (296 LOC) similarly only *runs* `git diff` and returns text.

**6. Grammar-marker handling elsewhere is display-only too.**

`crates/operant-cli/src/tui/operant_ui/ui_tools.rs:170-265` (`summarize_apply_patch_input`,
`extract_apply_patch_primary_file`) and `info_widget_git.rs:38-39` scan the same `*** Update File: ` prefixes —
but only to produce summaries and tab titles.

**7. There is no hunk/unified-diff APPLIER in operant.** A grep for `@@|hunk|unified.diff|apply_patch` across
`crates/` returned only: test fixtures, the display-only parser, the aft delegation, and the UI summarizers.
The one place with a real hunk *model* (`DiffHunk`/`DiffLine`, `diff_viewer.rs:29-45`) is TUI rendering.

## VERDICT ROWS

| mechanism | codex unit + LOC | operant equivalent + LOC | gap severity | portable? | ponytail verdict |
|---|---|---|---|---|---|
| 4-tier fuzzy context matching | `seek_sequence.rs` 164 | `patch_tool.rs::normalize_whitespace` (`:34-39`) + `fuzzy_replace` (`:45-119`) ~90, weaker | medium | yes, pure fn, zero deps | **IMPORT-SMALL** |
| Line-ending-preserving file model | `text_file.rs` 121 | none — `.lines()`/`replace`, drops CRLF fidelity | medium | yes, zero deps | **IMPORT-SMALL** |
| Chunk → replacements computation | `file_update.rs::compute_replacements` 204 | none | high | yes after stripping `similar` + exec-server (~120 core LOC) | **IMPORT** |
| Multi-hunk patch grammar parser | `parser.rs` 682 | `aft_tools.rs:645-655` path-scan only; execution is the external aft binary | medium (native path only) | yes after 3 imports | **IMPORT-SMALL** |
| Streaming incremental parse | `streaming_parser.rs` 924 | none | none | n/a | **DO-NOT** |
| Heredoc / shell invocation seam | `invocation.rs` 942 | none, not needed | none | no | **DO-NOT** |
| Atomic per-file write | `lib.rs::write_file_with_missing_parent_retry` `:765-818` (no temp/rename) | `file_tools.rs::atomic_write` `:65-108` (temp + rename + mode) | operant is ahead | n/a | **ALREADY-HAVE** |
| Unified-diff render | `file_update.rs::unified_diff_from_chunks` `:186-197` | `diff_viewer/parse.rs` 166 | none | n/a | **ALREADY-HAVE** |
| Multi-file single-call edit | yes (`hunk+` in grammar) | `aft_apply_patch` when bridge live; absent when dead | medium | n/a | **ALREADY-HAVE** (conditional) |

**On whether the multi-hunk grammar is worth importing:** import the *engine*, not the surface. The grammar is
already reachable through `aft_apply_patch` (`aft_tools.rs:615`), so a second agent-facing grammar would be two
ways to say the same thing. The defect is narrower: when the aft bridge is dead, `patch` (`patch_tool.rs:20-29`)
degrades to whole-file single-string replace. Fixing that means importing `seek_sequence` + `text_file` +
`compute_replacements` (~350 LOC combined, no non-stdlib deps) as the native engine behind an internal multi-chunk
path, leaving the `*** Begin Patch` surface as the optional fast path. operant should keep applying hunks in order
and stop at the first failure with the target file untouched, which `atomic_write` already guarantees per file.

**On streaming in a non-streaming TUI:** none. Gated behind a test-only feature flag; emits UI pings. operant
streams *tool results* into a panel, not tool *arguments* into a half-applied file.

**UNVERIFIED in execution:** the aft binary resolves to a mise shim on this box, but an executed `apply_patch`
call was not performed, so "AFT gives multi-hunk today" holds at the code-path and registration level
(`aft_tools.rs:46`, `main.rs:1263`), not as a live test.
